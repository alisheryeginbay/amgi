//! Anki's search syntax, as far as AnkiConnect clients use it: words, `field:value`,
//! `deck:`, `note:`, `tag:`, `nid:`, `cid:`, `is:` and `flag:`, combined with `or`,
//! `-` and brackets. Searches amgi can't answer, such as `added:1`, are errors rather
//! than quietly matching nothing.

use super::Entry;
use crate::search::fold;

pub enum Node {
    And(Vec<Node>),
    Or(Vec<Node>),
    Not(Box<Node>),
    Term(Term),
}

pub enum Term {
    /// Somewhere in any field.
    Text(Glob),
    /// A deck, or one inside it.
    Deck(Glob),
    Note(Glob),
    /// A tag, or one under it (`tag:a` matches `a::b`).
    Tag(Glob),
    /// No tags at all.
    Untagged,
    /// The whole of a field.
    Field { name: Glob, value: Glob },
    NoteIds(Vec<i64>),
    CardIds(Vec<i64>),
    Is(State),
    /// amgi has no flags, so only `flag:0` matches.
    Flag(u32),
}

#[derive(Clone, Copy)]
pub enum State {
    New,
    Learning,
    Review,
    Due,
    Suspended,
}

/// Parse a search; `None` when it's empty.
pub fn parse(query: &str) -> Result<Option<Node>, String> {
    let tokens = tokenize(query);
    let mut at = 0;
    if tokens.is_empty() {
        return Ok(None);
    }
    let node = parse_or(&tokens, &mut at)?;
    if at < tokens.len() {
        return Err("unbalanced brackets in search".into());
    }
    Ok(Some(node))
}

impl Node {
    pub fn matches(&self, entry: &Entry, day_start: i64) -> bool {
        match self {
            Node::And(nodes) => nodes.iter().all(|n| n.matches(entry, day_start)),
            Node::Or(nodes) => nodes.iter().any(|n| n.matches(entry, day_start)),
            Node::Not(node) => !node.matches(entry, day_start),
            Node::Term(term) => term.matches(entry, day_start),
        }
    }
}

impl Term {
    fn matches(&self, entry: &Entry, day_start: i64) -> bool {
        let card = entry.card;
        match self {
            Term::Text(glob) => entry.plain.iter().any(|value| glob.matches(value)),
            Term::Deck(glob) => ancestors(&entry.deck).any(|deck| glob.matches(&fold(deck))),
            Term::Note(glob) => glob.matches(&fold(&entry.model)),
            Term::Tag(glob) => card.tags.iter().any(|tag| ancestors(tag).any(|t| glob.matches(&fold(t)))),
            Term::Untagged => card.tags.is_empty(),
            Term::Field { name, value } => entry
                .fields
                .iter()
                .zip(&entry.plain)
                .any(|((field, _), plain)| name.matches(&fold(field)) && value.matches(plain)),
            Term::NoteIds(ids) => ids.contains(&entry.note_id),
            Term::CardIds(ids) => ids.contains(&entry.card_id),
            Term::Is(state) => match state {
                State::New => card.due.is_none(),
                State::Learning => card.interval.is_some_and(|i| i < crate::DAY_SECS),
                State::Review => card.interval.is_some_and(|i| i >= crate::DAY_SECS),
                // Due by the end of the study day, as amgi counts it.
                State::Due => !card.suspended && card.due.is_some_and(|due| due < day_start + crate::DAY_SECS),
                State::Suspended => card.suspended,
            },
            Term::Flag(flag) => *flag == 0,
        }
    }
}

/// `a::b::c`, then `a::b`, then `a`.
fn ancestors(name: &str) -> impl Iterator<Item = &str> {
    let mut end = Some(name.len());
    std::iter::from_fn(move || {
        let current = &name[..end?];
        end = current.rfind("::");
        Some(current)
    })
}

enum Token {
    Open,
    Close,
    Not,
    Or,
    /// With escapes kept, for [`Glob`] to read.
    Word(String),
}

fn tokenize(query: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = query.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '(' => {
                chars.next();
                tokens.push(Token::Open);
            }
            ')' => {
                chars.next();
                tokens.push(Token::Close);
            }
            '-' => {
                chars.next();
                match chars.peek() {
                    Some(c) if !c.is_whitespace() => tokens.push(Token::Not),
                    _ => tokens.push(Token::Word("-".into())),
                }
            }
            _ => {
                let mut word = String::new();
                let mut quoted = false;
                let mut was_quoted = false;
                while let Some(&c) = chars.peek() {
                    if !quoted && (c.is_whitespace() || c == '(' || c == ')') {
                        break;
                    }
                    chars.next();
                    match c {
                        '"' => {
                            quoted = !quoted;
                            was_quoted = true;
                        }
                        '\\' => {
                            word.push('\\');
                            if let Some(next) = chars.next() {
                                word.push(next);
                            }
                        }
                        c => word.push(c),
                    }
                }
                match word.to_lowercase().as_str() {
                    "or" if !was_quoted => tokens.push(Token::Or),
                    "and" if !was_quoted => {}
                    _ => tokens.push(Token::Word(word)),
                }
            }
        }
    }
    tokens
}

fn parse_or(tokens: &[Token], at: &mut usize) -> Result<Node, String> {
    let mut any = vec![parse_and(tokens, at)?];
    while matches!(tokens.get(*at), Some(Token::Or)) {
        *at += 1;
        any.push(parse_and(tokens, at)?);
    }
    Ok(if any.len() == 1 { any.pop().unwrap() } else { Node::Or(any) })
}

fn parse_and(tokens: &[Token], at: &mut usize) -> Result<Node, String> {
    let mut all = Vec::new();
    while let Some(token) = tokens.get(*at) {
        if matches!(token, Token::Or | Token::Close) {
            break;
        }
        all.push(parse_unary(tokens, at)?);
    }
    match all.len() {
        0 => Err("empty part of a search".into()),
        1 => Ok(all.pop().unwrap()),
        _ => Ok(Node::And(all)),
    }
}

fn parse_unary(tokens: &[Token], at: &mut usize) -> Result<Node, String> {
    let token = tokens.get(*at).ok_or("search ends too soon")?;
    *at += 1;
    match token {
        Token::Not => Ok(Node::Not(Box::new(parse_unary(tokens, at)?))),
        Token::Open => {
            let node = parse_or(tokens, at)?;
            match tokens.get(*at) {
                Some(Token::Close) => {
                    *at += 1;
                    Ok(node)
                }
                _ => Err("unbalanced brackets in search".into()),
            }
        }
        Token::Word(word) => term(word).map(Node::Term),
        Token::Close | Token::Or => Err("unexpected word in search".into()),
    }
}

/// Searches Anki has that amgi can't answer.
const UNSUPPORTED: &[&str] = &[
    "added", "rated", "edited", "introduced", "prop", "dupe", "resched", "re", "nc", "w", "card", "mid", "preset",
];

fn term(word: &str) -> Result<Term, String> {
    let Some((name, value)) = split_colon(word) else {
        return Ok(Term::Text(Glob::new(&format!("*{word}*"))));
    };
    let ids = |value: &str| {
        value
            .split(',')
            .map(|id| id.trim().parse::<i64>().map_err(|_| format!("invalid id in search: {id}")))
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(match name.to_lowercase().as_str() {
        "deck" => Term::Deck(Glob::new(value)),
        "note" => Term::Note(Glob::new(value)),
        "tag" if value.eq_ignore_ascii_case("none") => Term::Untagged,
        "tag" => Term::Tag(Glob::new(value)),
        "nid" => Term::NoteIds(ids(value)?),
        "cid" => Term::CardIds(ids(value)?),
        "flag" => Term::Flag(value.parse().map_err(|_| format!("invalid flag in search: {value}"))?),
        "is" => Term::Is(match value.to_lowercase().as_str() {
            "new" => State::New,
            "learn" => State::Learning,
            "review" => State::Review,
            "due" => State::Due,
            "suspended" => State::Suspended,
            _ => return Err(format!("amgi can't search is:{value}")),
        }),
        other if UNSUPPORTED.contains(&other) => return Err(format!("amgi can't search {other}:")),
        _ => Term::Field { name: Glob::new(name), value: Glob::new(value) },
    })
}

/// Split at the first `:` that isn't escaped.
fn split_colon(word: &str) -> Option<(&str, &str)> {
    let mut escaped = false;
    for (i, c) in word.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            ':' if i > 0 => return Some((&word[..i], &word[i + 1..])),
            _ => {}
        }
    }
    None
}

/// An Anki pattern, folded like the text it's matched against: `*` stands for any
/// run of characters and `_` for one, unless escaped with `\`.
pub struct Glob(Vec<Part>);

#[derive(Clone, Copy, PartialEq)]
enum Part {
    Any,
    One,
    Char(char),
}

impl Glob {
    pub fn new(pattern: &str) -> Self {
        let mut parts = Vec::new();
        let mut chars = pattern.chars();
        while let Some(c) = chars.next() {
            match c {
                '*' => parts.push(Part::Any),
                '_' => parts.push(Part::One),
                '\\' => {
                    if let Some(next) = chars.next() {
                        parts.extend(fold(&next.to_string()).chars().map(Part::Char));
                    }
                }
                c => parts.extend(fold(&c.to_string()).chars().map(Part::Char)),
            }
        }
        Glob(parts)
    }

    /// Whether `text`, already folded, matches the whole pattern.
    pub fn matches(&self, text: &str) -> bool {
        let text: Vec<char> = text.chars().collect();
        // matched[j]: the parts so far can match the first j characters.
        let mut matched = vec![false; text.len() + 1];
        matched[0] = true;
        for part in &self.0 {
            let mut next = vec![false; text.len() + 1];
            for j in 0..=text.len() {
                next[j] = match part {
                    Part::Any => matched[j] || (j > 0 && next[j - 1]),
                    Part::One => j > 0 && matched[j - 1],
                    Part::Char(c) => j > 0 && matched[j - 1] && text[j - 1] == *c,
                };
            }
            matched = next;
        }
        matched[text.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        let m = |pattern: &str, text: &str| Glob::new(pattern).matches(&fold(text));
        assert!(m("Hola", "hola"));
        assert!(m("cafe", "Café"));
        assert!(!m("hola", "hola mundo"));
        assert!(m("*mundo", "hola mundo"));
        assert!(m("h_la", "hola"));
        assert!(!m(r"h\_la", "hola"));
        assert!(m(r"a\*b", "a*b"));
        assert!(m("*", ""));
    }

    #[test]
    fn ancestors_first_to_root() {
        assert_eq!(ancestors("a::b::c").collect::<Vec<_>>(), ["a::b::c", "a::b", "a"]);
    }

    #[test]
    fn parsing() {
        let parsed = |q: &str| parse(q).map(|n| n.is_some());
        assert_eq!(parsed(""), Ok(false));
        assert_eq!(parsed(r#""deck:Lang::Spanish" "word:hola""#), Ok(true));
        assert_eq!(parsed(r#"nid:1,2 ((a) or (b)) -tag:x"#), Ok(true));
        assert_eq!(parsed("(a"), Err("unbalanced brackets in search".into()));
        assert_eq!(parsed("a)"), Err("unbalanced brackets in search".into()));
        assert_eq!(parsed("added:1"), Err("amgi can't search added:".into()));
        assert_eq!(parsed("nid:x"), Err("invalid id in search: x".into()));
        assert!(matches!(term(r#"word:a b"#), Ok(Term::Field { .. })));
        assert!(matches!(term(r"a\:b"), Ok(Term::Text(_))));
    }
}
