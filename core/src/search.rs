//! The browse search, as typed in the app's search field. Words must all match; see
//! the README for the syntax. The app's browse pane runs the same search in
//! `web/src/browse/query.ts`; keep the two in step.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use std::collections::HashMap;

use crate::parser::NoteTypeInfo;
use crate::{BrowseCard, CardKind};

/// Forgotten this many times or more, as in Anki.
pub const LEECH_LAPSES: u32 = 8;
const DAY_SECS: f64 = 86_400.0;

/// Case- and accent-insensitive, like Finder's search.
pub fn fold(s: &str) -> String {
    s.nfd().filter(|c| !is_combining_mark(*c)).collect::<String>().to_lowercase()
}

/// A parsed search. Never fails: anything that isn't a known filter is searched as
/// text, so `12:30` and links still work.
pub struct Query {
    tests: Vec<(bool, Test)>,
}

impl Query {
    /// `now` is in Unix seconds, for `is:due` and `due<7`.
    pub fn parse(query: &str, now: i64) -> Self {
        Self::parse_with(query, now, &[])
    }

    /// As `parse`, with `name:text` searching the field of that name in notes of
    /// these types.
    pub fn parse_with(query: &str, now: i64, note_types: &[NoteTypeInfo]) -> Self {
        let tests = tokenize(query)
            .into_iter()
            .filter_map(|token| compile(&token, now as f64, note_types).map(|test| (token.negated, test)))
            .collect();
        Query { tests }
    }

    pub fn matches(&self, card: &BrowseCard) -> bool {
        self.tests.iter().all(|(negated, test)| test.matches(card) != *negated)
    }
}

enum Test {
    /// Folded needle, searched in the folded fields.
    Text { needle: String, fields: Fields },
    /// Folded folders, matched as a run anywhere in the deck's path.
    Deck(Vec<String>),
    /// Folded tag; a prefix when it ended in `*`.
    Tag { want: String, prefix: bool },
    State(State, f64),
    Number { field: Number, op: Op, value: f64, now: f64 },
    /// Folded needle, searched in a field: its index in each note type that has it.
    Field { needle: String, index: HashMap<String, usize> },
}

#[derive(Clone, Copy)]
enum Fields {
    All,
    Front,
    Back,
}

#[derive(Clone, Copy)]
enum State {
    New,
    Due,
    Suspended,
    Leech,
    Cloze,
    Basic,
}

#[derive(Clone, Copy)]
enum Number {
    Reviews,
    Lapses,
    /// Days from now; new cards have none.
    Due,
}

#[derive(Clone, Copy)]
enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

impl Test {
    fn matches(&self, c: &BrowseCard) -> bool {
        match self {
            Test::Text { needle, fields } => {
                let found = |f: &str| fold(f).contains(needle.as_str());
                match fields {
                    Fields::Front => found(&c.front),
                    Fields::Back => found(&c.back),
                    // A note's fields count too, with those no card shows.
                    Fields::All => {
                        found(&c.front) || found(&c.back) || c.tags.iter().any(|t| found(t)) || c.fields.iter().any(|f| found(f))
                    }
                }
            }
            Test::Field { needle, index } => c
                .note_type
                .as_ref()
                .and_then(|t| index.get(t))
                .and_then(|&i| c.fields.get(i))
                .is_some_and(|f| fold(f).contains(needle.as_str())),
            Test::Deck(want) => {
                let deck = fold(&c.deck);
                let parts: Vec<&str> = deck.split('/').collect();
                parts.windows(want.len()).any(|w| w.iter().zip(want).all(|(a, b)| a == b))
            }
            Test::Tag { want, prefix } => c.tags.iter().any(|t| {
                let t = fold(t);
                if *prefix { t.starts_with(want.as_str()) } else { t == *want }
            }),
            Test::State(state, now) => match state {
                State::New => c.due.is_none(),
                State::Due => !c.suspended && c.due.is_some_and(|d| d as f64 <= *now),
                State::Suspended => c.suspended,
                State::Leech => c.lapses >= LEECH_LAPSES,
                State::Cloze => c.kind == CardKind::Cloze,
                State::Basic => c.kind == CardKind::Basic,
            },
            Test::Number { field, op, value, now } => {
                let v = match field {
                    Number::Reviews => f64::from(c.reps),
                    Number::Lapses => f64::from(c.lapses),
                    Number::Due => match c.due {
                        Some(d) => (d as f64 - now) / DAY_SECS,
                        None => return false,
                    },
                };
                match op {
                    Op::Lt => v < *value,
                    Op::Le => v <= *value,
                    Op::Gt => v > *value,
                    Op::Ge => v >= *value,
                    Op::Eq => v == *value,
                }
            }
        }
    }
}

struct Token {
    negated: bool,
    /// Text before the first `:`, `<`, `>` or `=`, if the token has one and it wasn't quoted.
    key: Option<String>,
    operator: &'static str,
    value: String,
    /// The token as typed, without the leading `-`, for when it isn't a known filter.
    raw: String,
}

/// The comparison at the start of `s`, longest first.
fn operator(s: &[char]) -> Option<&'static str> {
    match s {
        ['<', '=', ..] => Some("<="),
        ['>', '=', ..] => Some(">="),
        ['<', ..] => Some("<"),
        ['>', ..] => Some(">"),
        ['=', ..] => Some("="),
        [':', ..] => Some(":"),
        _ => None,
    }
}

/// Splits on spaces outside quotes. `a:"to go"` stays one token; an unclosed quote runs to the end.
fn tokenize(query: &str) -> Vec<Token> {
    let q: Vec<char> = query.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < q.len() {
        if q[i].is_whitespace() {
            i += 1;
            continue;
        }
        let mut negated = false;
        if q[i] == '-' && q.get(i + 1).is_some_and(|c| !c.is_whitespace()) {
            negated = true;
            i += 1;
        }
        let start = i;
        // A tag is never a filter, so tags like `lang::korean` keep their colons.
        let tag = q[i] == '#';
        let mut key = None;
        let mut op = "";
        let mut value = String::new();
        while i < q.len() && !q[i].is_whitespace() {
            if q[i] == '"' {
                let stop = q[i + 1..].iter().position(|&c| c == '"').map_or(q.len(), |p| i + 1 + p);
                value.extend(&q[i + 1..stop]);
                i = stop + 1;
                continue;
            }
            if key.is_none()
                && !tag
                && !value.is_empty()
                && let Some(found) = operator(&q[i..])
            {
                key = Some(std::mem::take(&mut value));
                op = found;
                i += found.len();
                // `lapses:>3` reads like `lapses>3`.
                if op == ":"
                    && let Some(after) = operator(&q[i..]).filter(|&o| o != ":")
                {
                    op = after;
                    i += after.len();
                }
                continue;
            }
            value.push(q[i]);
            i += 1;
        }
        let raw = q[start..i.min(q.len())].iter().collect();
        tokens.push(Token { negated, key, operator: op, value, raw });
    }
    tokens
}

fn text(word: &str, fields: Fields) -> Option<Test> {
    (!word.is_empty()).then(|| Test::Text { needle: fold(word), fields })
}

/// A field name as typed in a search: folded, without spaces, so `wordmeaning:` finds `Word Meaning`.
fn field_key(name: &str) -> String {
    fold(name).split_whitespace().collect()
}

/// What one token tests; `None` if it matches everything, like a filter still being typed (`deck:`).
fn compile(token: &Token, now: f64, note_types: &[NoteTypeInfo]) -> Option<Test> {
    let value = token.value.as_str();
    let Some(key) = &token.key else {
        if token.raw.starts_with('#') {
            let tag = fold(value.get(1..).unwrap_or_default());
            return match tag.strip_suffix('*') {
                _ if tag.is_empty() => None,
                Some(prefix) => Some(Test::Tag { want: prefix.to_string(), prefix: true }),
                None => Some(Test::Tag { want: tag, prefix: false }),
            };
        }
        // A plain `"quoted"` word has its quotes removed.
        return text(value, Fields::All);
    };
    let number = match key.to_lowercase().as_str() {
        "reviews" => Some(Number::Reviews),
        "lapses" => Some(Number::Lapses),
        "due" => Some(Number::Due),
        _ => None,
    };
    if let Some(field) = number {
        if value.is_empty() {
            return None;
        }
        if let Some(n) = value.parse::<f64>().ok().filter(|n| n.is_finite()) {
            let op = match token.operator {
                "<" => Op::Lt,
                "<=" => Op::Le,
                ">" => Op::Gt,
                ">=" => Op::Ge,
                _ => Op::Eq,
            };
            return Some(Test::Number { field, op, value: n, now });
        }
    } else if token.operator == ":" {
        match key.to_lowercase().as_str() {
            "deck" | "q" | "a" | "is" if value.is_empty() => return None,
            "deck" => {
                let folded = fold(value);
                let want: Vec<String> = folded.split('/').filter(|p| !p.is_empty()).map(String::from).collect();
                // `deck:/` names no folder, so it matches every deck.
                return (!want.is_empty()).then_some(Test::Deck(want));
            }
            "q" => return text(value, Fields::Front),
            "a" => return text(value, Fields::Back),
            "is" => {
                let state = match value.to_lowercase().as_str() {
                    "new" => Some(State::New),
                    "due" => Some(State::Due),
                    "suspended" => Some(State::Suspended),
                    "leech" => Some(State::Leech),
                    "cloze" => Some(State::Cloze),
                    "basic" => Some(State::Basic),
                    _ => None,
                };
                if let Some(state) = state {
                    return Some(Test::State(state, now));
                }
            }
            _ => {
                let key = field_key(key);
                let index: HashMap<String, usize> = note_types
                    .iter()
                    .filter_map(|t| Some((t.name.clone(), t.fields.iter().position(|f| field_key(f) == key)?)))
                    .collect();
                if !index.is_empty() {
                    return (!value.is_empty()).then(|| Test::Field { needle: fold(value), index });
                }
            }
        }
    }
    // Anything else is searched as typed.
    text(&token.raw, Fields::All)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    fn card(id: &str) -> BrowseCard {
        BrowseCard {
            id: id.into(),
            deck: "geography".into(),
            tags: vec![],
            path: "geography.md".into(),
            line: 1,
            kind: CardKind::Basic,
            front: String::new(),
            back: String::new(),
            due: None,
            note_type: None,
            fields: vec![],
            reps: 0,
            lapses: 0,
            suspended: false,
        }
    }

    fn tags(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| s.to_string()).collect()
    }

    fn cards() -> Vec<BrowseCard> {
        vec![
            BrowseCard {
                front: "Capital of France?".into(),
                back: "Paris".into(),
                tags: tags(&["europe", "capitals"]),
                due: Some(NOW - DAY),
                reps: 4,
                ..card("paris")
            },
            BrowseCard {
                front: "What does café mean?".into(),
                back: "coffee".into(),
                deck: "lang/french/words".into(),
                tags: tags(&["food"]),
                due: Some(NOW + 3 * DAY),
                reps: 2,
                ..card("cafe")
            },
            BrowseCard {
                front: "Formula for water".into(),
                back: "H2O".into(),
                deck: "science".into(),
                kind: CardKind::Cloze,
                tags: tags(&["chem::basics"]),
                ..card("water")
            },
            BrowseCard {
                front: "Boiling point at 12:30 altitude".into(),
                back: "less than 100".into(),
                deck: "science/physics".into(),
                lapses: 9,
                reps: 12,
                due: Some(NOW - 60),
                suspended: true,
                tags: tags(&["suspended"]),
                ..card("leech")
            },
        ]
    }

    fn search(query: &str) -> Vec<String> {
        let q = Query::parse(query, NOW);
        cards().into_iter().filter(|c| q.matches(c)).map(|c| c.id).collect()
    }

    fn check(query: &str, want: &[&str]) {
        assert_eq!(search(query), want, "{query}");
    }

    const ALL: &[&str] = &["paris", "cafe", "water", "leech"];

    #[test]
    fn empty_search_matches_everything() {
        check("", ALL);
        check("   ", ALL);
    }

    #[test]
    fn words_ignore_case_and_accents() {
        check("CAFE", &["cafe"]);
        check("paris", &["paris"]);
        check("capitals", &["paris"]);
    }

    #[test]
    fn every_word_must_match() {
        check("capital france", &["paris"]);
        check("capital water", &[]);
    }

    #[test]
    fn quotes_keep_a_phrase_together() {
        check("\"for water\"", &["water"]);
        check("\"water for\"", &[]);
        check("\"unclosed for wat", &[]);
        check("\"formula for wat", &["water"]);
    }

    #[test]
    fn minus_leaves_cards_out() {
        check("-paris", &["cafe", "water", "leech"]);
        check("-is:new -is:suspended", &["paris", "cafe"]);
        check("- paris", &[]);
    }

    #[test]
    fn tags_match_whole_or_by_prefix() {
        check("#capital", &[]);
        check("#Capitals", &["paris"]);
        check("#cap*", &["paris"]);
        check("-#suspended", &["paris", "cafe", "water"]);
        check("#", ALL);
        check("#chem::basics", &["water"]);
        check("#chem::*", &["water"]);
    }

    #[test]
    fn deck_matches_any_run_of_folders() {
        check("deck:science", &["water", "leech"]);
        check("deck:physics", &["leech"]);
        check("deck:french", &["cafe"]);
        check("deck:lang/french", &["cafe"]);
        check("deck:\"french/words\"", &["cafe"]);
        check("deck:fren", &[]);
        check("deck:lang/words", &[]);
    }

    #[test]
    fn q_and_a_search_one_side() {
        check("q:paris", &[]);
        check("a:paris", &["paris"]);
        check("Q:capital", &["paris"]);
        check("a:\"less than\"", &["leech"]);
    }

    #[test]
    fn is_filters_by_state_and_kind() {
        check("is:new", &["water"]);
        check("is:due", &["paris"]);
        check("is:suspended", &["leech"]);
        check("is:leech", &["leech"]);
        check("is:cloze", &["water"]);
        check("is:basic", &["paris", "cafe", "leech"]);
    }

    #[test]
    fn numbers_compare() {
        check("lapses>3", &["leech"]);
        check("lapses:>=9", &["leech"]);
        check("reviews:4", &["paris"]);
        check("reviews=2", &["cafe"]);
        check("reviews<=2", &["cafe", "water"]);
    }

    #[test]
    fn due_compares_days_and_skips_new_cards() {
        check("due<0", &["paris", "leech"]);
        check("due<7", &["paris", "cafe", "leech"]);
        check("due>1", &["cafe"]);
        check("due>-2", &["paris", "cafe", "leech"]);
    }

    #[test]
    fn filters_being_typed_match_everything() {
        for query in ["deck:", "is:", "q:", "lapses>", "due<"] {
            check(query, ALL);
        }
    }

    #[test]
    fn anything_else_is_text() {
        check("12:30", &["leech"]);
        check("is:nothing", &[]);
        check("lapses>many", &[]);
        check("H2O", &["water"]);
    }

    #[test]
    fn field_filters_search_notes() {
        let types = vec![NoteTypeInfo {
            name: "Vocab".into(),
            fields: vec!["Word".into(), "Word Meaning".into(), "Notes".into()],
        }];
        let note = |id: &str, values: [&str; 3]| BrowseCard {
            note_type: Some("Vocab".into()),
            fields: values.iter().map(|v| v.to_string()).collect(),
            ..card(id)
        };
        let all = [note("uno", ["uno", "one", "a number"]), note("dos", ["dos", "two", ""]), card("plain")];
        let find = |q: &str| {
            let query = Query::parse_with(q, NOW, &types);
            all.iter().filter(|c| query.matches(c)).map(|c| c.id.clone()).collect::<Vec<_>>()
        };
        assert_eq!(find("wordmeaning:one"), ["uno"]);
        assert_eq!(find("Word:DOS"), ["dos"]);
        assert_eq!(find("-word:uno"), ["dos", "plain"]);
        assert_eq!(find("number"), ["uno"]);
        assert_eq!(find("word:"), ["uno", "dos", "plain"]);
        // Without note types, it's text.
        assert!(!Query::parse("word:uno", NOW).matches(&all[0]));
    }
}
