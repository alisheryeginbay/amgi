//! Plain Markdown card format.
//!
//! ```markdown
//! Q: What does FSRS stand for?
//! A: Free Spaced Repetition Scheduler
//!
//! C: The capital of [France] is [Paris|city].
//! C: Indexing uses `v[0]`; write \[ and \] for literal brackets.
//! ```
//!
//! A card starts at a line beginning with `Q:` or `C:` and ends at the next blank
//! line or the next card marker. Continuation lines are appended to the current
//! field. Every other line (headings, notes) is ignored. Each `[...]` deletion
//! in a cloze produces its own card; `[answer|hint]` shows the hint when hidden.
//! Brackets escaped with a backslash, inside `code` spans or in an image
//! `![alt](path)` are literal. Escapes are kept in the output, where Markdown
//! renders them as plain brackets.
//!
//! A fenced code block (```` ``` ```` or `~~~`) keeps everything inside it, blank
//! lines and lines starting `Q:` included, and its brackets are never deletions.
//!
//! A card's last lines may hold tags: `#korean #verbs`. A file may start with TOML
//! front matter between `+++` lines, giving the deck a `name` and default `tags`.

use std::fmt::Write;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CardKind {
    Basic,
    Cloze,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCard {
    /// Stable identity derived from content, used as the key for review history.
    pub id: String,
    pub kind: CardKind,
    pub front: String,
    pub back: String,
    /// 1-based line where the card starts.
    pub line: u32,
    /// The deck's default tags, then the card's own, without `#`.
    pub tags: Vec<String>,
}

/// Cards with this tag, on the card or in the deck's front matter, are never studied.
pub const SUSPENDED_TAG: &str = "suspended";

impl ParsedCard {
    pub fn is_suspended(&self) -> bool {
        self.tags.iter().any(|t| t.eq_ignore_ascii_case(SUSPENDED_TAG))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ParseWarning {
    pub line: u32,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct ParseResult {
    /// Deck name from the front matter.
    pub name: Option<String>,
    pub cards: Vec<ParsedCard>,
    pub warnings: Vec<ParseWarning>,
    /// Default tags from the front matter.
    deck_tags: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct FrontMatter {
    name: Option<String>,
    tags: Vec<String>,
}

const FRONT_MATTER: &str = "+++";

enum Field {
    Question,
    Answer,
    Cloze,
}

struct Pending {
    line: u32,
    field: Field,
    question: Vec<String>,
    answer: Vec<String>,
    cloze: Vec<String>,
}

pub fn parse(source: &str) -> ParseResult {
    let mut out = ParseResult::default();
    let mut pending: Option<Pending> = None;
    let mut fence = Fences::default();
    let body_start = front_matter(source, &mut out);

    for (idx, raw) in source.lines().enumerate().skip(body_start) {
        let line_no = idx as u32 + 1;
        let line = raw.trim_end();

        if fence.inside() {
            fence.step(line);
            if let Some(p) = pending.as_mut() {
                p.current().push(line.to_string());
            }
            continue;
        }

        if line.trim().is_empty() {
            finish(pending.take(), &mut out);
            continue;
        }

        if let Some(rest) = marker(line, "Q:") {
            finish(pending.take(), &mut out);
            pending = Some(Pending::new(line_no, Field::Question, rest));
        } else if let Some(rest) = marker(line, "C:") {
            finish(pending.take(), &mut out);
            pending = Some(Pending::new(line_no, Field::Cloze, rest));
        } else if let Some(rest) = marker(line, "A:") {
            match pending.as_mut() {
                Some(p) if matches!(p.field, Field::Question) => {
                    p.field = Field::Answer;
                    p.answer.push(rest.to_string());
                }
                _ => out.warnings.push(ParseWarning {
                    line: line_no,
                    message: "`A:` without a preceding `Q:`".into(),
                }),
            }
        } else if let Some(p) = pending.as_mut() {
            p.current().push(line.to_string());
        }
        // A fence opens on a marker line too, as in `A: ```r`.
        let content = ["Q:", "A:", "C:"].iter().find_map(|m| marker(line, m)).unwrap_or(line);
        fence.step(content);
        if fence.inside() {
            fence.start = line_no;
        }
    }
    if fence.inside() {
        out.warnings.push(ParseWarning {
            line: fence.start,
            message: "code block has no closing fence".into(),
        });
    }
    finish(pending, &mut out);
    out
}

/// Tracks fenced code blocks line by line, as CommonMark does: a run of three or more
/// backticks or tildes, indented at most three spaces, closed by a longer or equal run
/// of the same character with nothing after it.
#[derive(Default)]
struct Fences {
    open: Option<(char, usize)>,
    /// 1-based line where the open block starts.
    start: u32,
}

impl Fences {
    fn inside(&self) -> bool {
        self.open.is_some()
    }

    fn step(&mut self, line: &str) {
        let indent = line.len() - line.trim_start_matches(' ').len();
        if indent > 3 {
            return;
        }
        let rest = &line[indent..];
        let Some(c) = rest.chars().next().filter(|c| matches!(c, '`' | '~')) else { return };
        let len = rest.len() - rest.trim_start_matches(c).len();
        let after = &rest[len..];
        match self.open {
            Some((open, open_len)) => {
                if c == open && len >= open_len && after.trim().is_empty() {
                    self.open = None;
                }
            }
            // A backtick fence's info string cannot hold backticks.
            None if len >= 3 && !(c == '`' && after.contains('`')) => self.open = Some((c, len)),
            None => {}
        }
    }
}

/// Read TOML front matter between `+++` lines at the top of the file. Returns the
/// number of lines it takes up.
fn front_matter(source: &str, out: &mut ParseResult) -> usize {
    let mut lines = source.lines();
    if lines.next().map(str::trim_end) != Some(FRONT_MATTER) {
        return 0;
    }
    let Some(len) = lines.position(|l| l.trim_end() == FRONT_MATTER) else {
        out.warnings.push(ParseWarning {
            line: 1,
            message: "front matter has no closing `+++`".into(),
        });
        return 0;
    };
    let toml: Vec<&str> = source.lines().skip(1).take(len).collect();
    match toml::from_str::<FrontMatter>(&toml.join("\n")) {
        Ok(fm) => {
            out.name = fm.name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
            out.deck_tags = fm.tags.iter().filter_map(|t| clean_tag(t)).collect();
        }
        Err(e) => out.warnings.push(ParseWarning {
            line: 1,
            message: format!("invalid front matter: {}", e.message()),
        }),
    }
    len + 2
}

/// A tag without its `#`, or `None` if empty or containing whitespace.
fn clean_tag(tag: &str) -> Option<String> {
    let tag = tag.trim().trim_start_matches('#');
    (!tag.is_empty() && !tag.contains(char::is_whitespace)).then(|| tag.to_string())
}

/// Tags on a line made only of `#tag` words, such as `#korean #verbs`.
pub fn tag_line(line: &str) -> Option<Vec<String>> {
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    words
        .iter()
        .map(|w| w.strip_prefix('#').filter(|t| !t.is_empty() && !t.contains('#')).map(String::from))
        .collect()
}

/// Remove trailing tag lines from a field, keeping its first (marker) line.
fn take_tags(lines: &mut Vec<String>) -> Vec<String> {
    let mut tags = Vec::new();
    while lines.len() > 1 {
        let Some(line_tags) = lines.last().and_then(|l| tag_line(l)) else { break };
        lines.pop();
        tags.splice(0..0, line_tags);
    }
    tags
}

/// Add or remove `#tag` on the card starting at 1-based `line`, in the card's own tag
/// lines; the rest of the file is kept byte for byte. `None` when no card starts
/// there or its own tags already say so (a front matter tag cannot be removed here).
pub fn set_card_tag(source: &str, line: u32, tag: &str, present: bool) -> Option<String> {
    let mut lines: Vec<String> = source.split_inclusive('\n').map(String::from).collect();
    let start = line.checked_sub(1)? as usize;
    let first = lines.get(start)?.trim_end();
    if marker(first, "Q:").is_none() && marker(first, "C:").is_none() {
        return None;
    }
    // The card runs to the next blank line or card marker outside a code block, as
    // when parsing.
    let mut fence = Fences::default();
    fence.step(marker(first, "Q:").or(marker(first, "C:")).unwrap_or(first));
    let mut end = start + 1;
    while let Some(l) = lines.get(end).map(|l| l.trim_end()) {
        if !fence.inside() && (l.trim().is_empty() || marker(l, "Q:").is_some() || marker(l, "C:").is_some()) {
            break;
        }
        let content = if fence.inside() { l } else { marker(l, "A:").unwrap_or(l) };
        fence.step(content);
        end += 1;
    }
    // Its own tag lines close it; the marker line never counts as one.
    let mut tags_start = end;
    while tags_start > start + 1 && tag_line(&lines[tags_start - 1]).is_some() {
        tags_start -= 1;
    }
    let matches = |t: &String| t.eq_ignore_ascii_case(tag);
    let has = (tags_start..end).any(|i| tag_line(&lines[i]).is_some_and(|tags| tags.iter().any(matches)));
    if has == present {
        return None;
    }

    let split = |l: &str| {
        let body = l.trim_end_matches(['\n', '\r']);
        (body.to_string(), l[body.len()..].to_string())
    };
    if present {
        let (body, newline) = split(&lines[end - 1]);
        if tags_start < end {
            lines[end - 1] = format!("{body} #{tag}{newline}");
        } else if newline.is_empty() {
            // The card ends the file without a final newline.
            lines[end - 1] = format!("{body}\n");
            lines.insert(end, format!("#{tag}"));
        } else {
            lines.insert(end, format!("#{tag}{newline}"));
        }
    } else {
        for i in (tags_start..end).rev() {
            let (body, newline) = split(&lines[i]);
            let kept: Vec<&str> = body.split_whitespace().filter(|w| !w.strip_prefix('#').is_some_and(|t| t.eq_ignore_ascii_case(tag))).collect();
            if kept.is_empty() {
                lines.remove(i);
            } else {
                lines[i] = format!("{}{newline}", kept.join(" "));
            }
        }
    }
    Some(lines.concat())
}

fn marker<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    line.strip_prefix(tag).map(str::trim_start)
}

impl Pending {
    fn new(line: u32, field: Field, first: &str) -> Self {
        let mut p = Pending {
            line,
            field,
            question: Vec::new(),
            answer: Vec::new(),
            cloze: Vec::new(),
        };
        p.current().push(first.to_string());
        p
    }

    fn current(&mut self) -> &mut Vec<String> {
        match self.field {
            Field::Question => &mut self.question,
            Field::Answer => &mut self.answer,
            Field::Cloze => &mut self.cloze,
        }
    }
}

fn join(lines: &[String]) -> String {
    lines.join("\n").trim().to_string()
}

fn finish(pending: Option<Pending>, out: &mut ParseResult) {
    let Some(mut p) = pending else { return };
    let mut tags = out.deck_tags.clone();
    for tag in take_tags(p.current()) {
        if !tags.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
            tags.push(tag);
        }
    }
    match p.field {
        Field::Question => out.warnings.push(ParseWarning {
            line: p.line,
            message: "`Q:` without an `A:`".into(),
        }),
        Field::Answer => {
            let front = join(&p.question);
            let back = join(&p.answer);
            if front.is_empty() || back.is_empty() {
                out.warnings.push(ParseWarning {
                    line: p.line,
                    message: "card has an empty question or answer".into(),
                });
                return;
            }
            out.cards.push(ParsedCard {
                id: card_id(&["basic", &normalize(&front)]),
                kind: CardKind::Basic,
                front,
                back,
                line: p.line,
                tags,
            });
        }
        Field::Cloze => parse_cloze(&join(&p.cloze), p.line, tags, out),
    }
}

enum Segment<'a> {
    Text(&'a str),
    Deletion { answer: &'a str, hint: Option<&'a str> },
}

/// Byte offset of the first `target` that is not backslash-escaped, inside a code
/// span, part of an image or inside math.
fn find_markup(s: &str, target: char) -> Option<usize> {
    let (mut escaped, mut skip_to) = (false, 0);
    // Length of the backtick run that opened the current code span or block, which
    // closes only on a run of the same length.
    let mut code: Option<usize> = None;
    for (i, c) in s.char_indices() {
        let in_code = code.is_some();
        match c {
            _ if i < skip_to => {}
            _ if escaped => escaped = false,
            '\\' if !in_code => escaped = true,
            '`' => {
                let run = s[i..].len() - s[i..].trim_start_matches('`').len();
                skip_to = i + run;
                code = match code {
                    None => Some(run),
                    Some(open) if open == run => None,
                    open => open,
                };
            }
            '!' if !in_code => skip_to = image_len(&s[i..]).map_or(0, |len| i + len),
            '$' if !in_code => skip_to = math_len(&s[i..]).map_or(0, |len| i + len),
            c if c == target && !in_code => return Some(i),
            _ => {}
        }
    }
    None
}

/// Byte length of `$…$` or `$$…$$` math at the start of `s`. As in Pandoc, a single `$`
/// opens only before a non-space and closes only after a non-space and before a
/// non-digit, so prices like `$5 and $10` stay text. Inside, `\` escapes the next
/// character. Kept in step with `mathLength` in `web/src/lib/markdown.ts`.
pub fn math_len(s: &str) -> Option<usize> {
    let display = s.starts_with("$$");
    let body = s.strip_prefix(if display { "$$" } else { "$" })?;
    if body.is_empty() || (!display && body.starts_with(char::is_whitespace)) {
        return None;
    }
    let mut escaped = false;
    for (i, c) in body.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '$' if display => {
                if body[i..].starts_with("$$") && i > 0 {
                    return Some(i + 4);
                }
            }
            '$' => {
                let before = body[..i].chars().next_back();
                let after = body[i + 1..].chars().next();
                if i > 0 && !before.is_some_and(char::is_whitespace) && !after.is_some_and(|c| c.is_ascii_digit()) {
                    return Some(i + 2);
                }
            }
            _ => {}
        }
    }
    None
}

/// Byte length of a Markdown image `![alt](path)` or `![alt](<path>)` at the start of `s`.
pub fn image_len(s: &str) -> Option<usize> {
    // Alt text may hold balanced brackets.
    let mut depth = 0;
    let alt_len = s.strip_prefix("![")?.char_indices().find_map(|(i, c)| {
        match c {
            '[' => depth += 1,
            ']' if depth == 0 => return Some(i),
            ']' => depth -= 1,
            _ => {}
        }
        None
    })?;
    let dest_start = 2 + alt_len + 2;
    let dest = s[dest_start - 1..].strip_prefix('(')?;
    let dest_len = match dest.strip_prefix('<') {
        Some(angled) => {
            let close = angled.find('>')?;
            angled[close + 1..].starts_with(')').then_some(close + 2)?
        }
        None => dest.find(')')?,
    };
    Some(dest_start + dest_len + 1)
}

fn segments(text: &str) -> Option<Vec<Segment<'_>>> {
    let mut segs = Vec::new();
    let mut rest = text;
    while let Some(start) = find_markup(rest, '[') {
        let end = find_markup(&rest[start + 1..], ']')? + start + 1;
        if start > 0 {
            segs.push(Segment::Text(&rest[..start]));
        }
        let inner = &rest[start + 1..end];
        let (answer, hint) = match find_markup(inner, '|') {
            Some(bar) => (inner[..bar].trim(), Some(inner[bar + 1..].trim())),
            None => (inner.trim(), None),
        };
        let hint = hint.filter(|h| !h.is_empty());
        segs.push(Segment::Deletion { answer, hint });
        rest = &rest[end + 1..];
    }
    if !rest.is_empty() {
        segs.push(Segment::Text(rest));
    }
    Some(segs)
}

fn parse_cloze(text: &str, line: u32, tags: Vec<String>, out: &mut ParseResult) {
    let Some(segs) = segments(text) else {
        out.warnings.push(ParseWarning {
            line,
            message: "unclosed `[` in cloze".into(),
        });
        return;
    };
    let deletions = segs
        .iter()
        .filter(|s| matches!(s, Segment::Deletion { .. }))
        .count();
    if deletions == 0 {
        out.warnings.push(ParseWarning {
            line,
            message: "cloze has no `[...]` deletions".into(),
        });
        return;
    }

    let normalized = normalize(text);
    for target in 0..deletions {
        let (mut front, mut back) = (String::new(), String::new());
        let mut n = 0;
        for seg in &segs {
            match seg {
                Segment::Text(t) => {
                    front.push_str(t);
                    back.push_str(t);
                }
                Segment::Deletion { answer, hint } => {
                    if n == target {
                        let _ = write!(front, "**[{}]**", hint.unwrap_or("…"));
                        let _ = write!(back, "**{answer}**");
                    } else {
                        front.push_str(answer);
                        back.push_str(answer);
                    }
                    n += 1;
                }
            }
        }
        out.cards.push(ParsedCard {
            id: card_id(&["cloze", &normalized, &target.to_string()]),
            kind: CardKind::Cloze,
            front,
            back,
            line,
            tags: tags.clone(),
        });
    }
}

/// Collapse whitespace so reflowing a card does not reset its history.
fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// FNV-1a 64-bit: stable across Rust versions and platforms, unlike `DefaultHasher`.
fn card_id(parts: &[&str]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            // 0xff never appears in UTF-8, so it cleanly separates parts.
            hash ^= 0xff;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        for b in part.bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_cards() {
        let r = parse("# Deck\n\nQ: one?\nA: 1\n\nnotes\nQ: two?\nmore\nA: 2\nline two\n");
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(r.cards.len(), 2);
        assert_eq!(r.cards[0].front, "one?");
        assert_eq!(r.cards[0].back, "1");
        assert_eq!(r.cards[0].line, 3);
        assert_eq!(r.cards[1].front, "two?\nmore");
        assert_eq!(r.cards[1].back, "2\nline two");
    }

    #[test]
    fn next_marker_ends_card_without_blank_line() {
        let r = parse("Q: a\nA: b\nQ: c\nA: d\nC: [e]");
        assert_eq!(r.cards.len(), 3);
    }

    #[test]
    fn cloze_expands_per_deletion() {
        let r = parse("C: The capital of [France] is [Paris|city].");
        assert_eq!(r.cards.len(), 2);
        assert_eq!(r.cards[0].front, "The capital of **[…]** is Paris.");
        assert_eq!(r.cards[0].back, "The capital of **France** is Paris.");
        assert_eq!(r.cards[1].front, "The capital of France is **[city]**.");
        assert_ne!(r.cards[0].id, r.cards[1].id);
    }

    #[test]
    fn literal_brackets() {
        let r = parse(r"C: \[1\] `v[0]` is [first|\[i\]] and [`m[k]`].");
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(r.cards.len(), 2);
        assert_eq!(r.cards[0].front, r"\[1\] `v[0]` is **[\[i\]]** and `m[k]`.");
        assert_eq!(r.cards[0].back, r"\[1\] `v[0]` is **first** and `m[k]`.");
        assert_eq!(r.cards[1].front, r"\[1\] `v[0]` is first and **[…]**.");
    }

    #[test]
    fn images_are_not_deletions() {
        let r = parse("C: ![a [b]](x.png) is [y] and [![](<z (1).png>)|pic]");
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(r.cards.len(), 2);
        assert_eq!(r.cards[0].front, "![a [b]](x.png) is **[…]** and ![](<z (1).png>)");
        assert_eq!(r.cards[1].back, "![a [b]](x.png) is y and **![](<z (1).png>)**");
        assert_eq!(image_len("![](a.png) rest"), Some(10));
        assert_eq!(image_len("![](<a b>) rest"), Some(10));
        assert_eq!(image_len("![x] (a)"), None);
        assert_eq!(image_len("!["), None);
    }

    #[test]
    fn tags() {
        let source = "+++\nname = \"Korean verbs\"\ntags = [\"korean\", \"#lang\"]\n+++\n\nQ: 가다\nA: to go\n#verbs #Korean\n\nC: [x]\n#a\n#b\n\nQ: q\nA: #1\n";
        let r = parse(source);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(r.name.as_deref(), Some("Korean verbs"));
        assert_eq!(r.cards[0].tags, ["korean", "lang", "verbs"]);
        assert_eq!((r.cards[0].back.as_str(), r.cards[0].line), ("to go", 6));
        assert_eq!(r.cards[1].tags, ["korean", "lang", "a", "b"]);
        // A marker line is never tags, and tags don't change a card's id.
        assert_eq!(r.cards[2].back, "#1");
        assert_eq!(r.cards[0].id, parse("Q: 가다\nA: to go").cards[0].id);
        assert_eq!(tag_line("# Heading"), None);
        assert_eq!(tag_line("#a b"), None);
    }

    #[test]
    fn front_matter_problems() {
        let r = parse("+++\nname = \n+++\nQ: a\nA: b");
        assert_eq!(r.warnings.len(), 1);
        assert!(r.warnings[0].message.starts_with("invalid front matter"));
        assert_eq!(r.cards.len(), 1);
        let r = parse("+++\nQ: a\nA: b");
        assert_eq!((r.warnings.len(), r.cards.len()), (1, 1));
    }

    #[test]
    fn only_escaped_brackets_is_not_a_cloze() {
        let r = parse(r"C: \[not a deletion\]");
        assert!(r.cards.is_empty());
        assert_eq!(r.warnings.len(), 1);
    }

    #[test]
    fn ids_are_stable_and_whitespace_insensitive() {
        let a = parse("Q: What  is\nthis?\nA: x");
        let b = parse("Q: What is this?\nA: something else");
        assert_eq!(a.cards[0].id, b.cards[0].id);
        assert_eq!(a.cards[0].id.len(), 16);
        assert_ne!(card_id(&["ab", "c"]), card_id(&["a", "bc"]));
    }

    #[test]
    fn warnings() {
        let r = parse("A: orphan\n\nQ: no answer\n\nC: nothing\n\nC: [open");
        assert!(r.cards.is_empty());
        let lines: Vec<u32> = r.warnings.iter().map(|w| w.line).collect();
        assert_eq!(lines, vec![1, 3, 5, 7]);
    }

    #[test]
    fn suspending_edits_only_the_card() {
        let source = "+++\ntags = [\"x\"]\n+++\n\nQ: one\nA: 1\n#geo\n\nC: The [capital] is here.\r\nmore text\r\n\nQ: last\nA: 3";
        // Joins an existing tag line.
        let a = set_card_tag(source, 5, SUSPENDED_TAG, true).unwrap();
        assert!(a.contains("A: 1\n#geo #suspended\n\nC:"));
        // Gets a tag line of its own, with the file's line ending.
        let b = set_card_tag(&a, 9, SUSPENDED_TAG, true).unwrap();
        assert!(b.contains("more text\r\n#suspended\r\n\nQ: last"));
        // The last card, without a final newline.
        let c = set_card_tag(&b, 13, SUSPENDED_TAG, true).unwrap();
        assert!(c.ends_with("A: 3\n#suspended"));
        let cards = parse(&c).cards;
        assert!(cards.iter().all(ParsedCard::is_suspended));
        // Tags are not part of a card's identity, so history is kept.
        assert_eq!(cards.iter().map(|c| &c.id).collect::<Vec<_>>(), parse(source).cards.iter().map(|c| &c.id).collect::<Vec<_>>());

        let mut back = c;
        for line in [13, 9, 5] {
            back = set_card_tag(&back, line, "Suspended", false).unwrap();
        }
        // Removing restores the file, now with a final newline.
        assert_eq!(back, format!("{source}\n"));
        // Nothing to do, or no card at that line.
        assert_eq!(set_card_tag(source, 5, SUSPENDED_TAG, false), None);
        assert_eq!(set_card_tag(source, 6, SUSPENDED_TAG, true), None);
        // A deck tag cannot be removed from one card.
        assert_eq!(set_card_tag(source, 5, "x", false), None);
    }

    #[test]
    fn code_blocks_keep_blank_lines_and_markers() {
        let source = "Q: Python example\nA:\n```python\ndef f(x):\n\n    return x\n# not a tag\nQ: not a card\n```\n#code\n\nQ: next\nA: ~~~\n````\n~~~";
        let r = parse(source);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(r.cards.len(), 2);
        assert_eq!(r.cards[0].back, "```python\ndef f(x):\n\n    return x\n# not a tag\nQ: not a card\n```");
        assert_eq!(r.cards[0].tags, ["code"]);
        assert_eq!(r.cards[1].back, "~~~\n````\n~~~");

        // Tags go after the closing fence, past the blank line inside the block.
        let tagged = set_card_tag(source, 1, SUSPENDED_TAG, true).unwrap();
        assert!(tagged.contains("```\n#code #suspended\n\nQ: next"));

        let r = parse("Q: a\nA: ```\nopen\n\nQ: b\nA: c");
        assert_eq!(r.warnings[0].message, "code block has no closing fence");
        assert_eq!(r.warnings[0].line, 2);
    }

    #[test]
    fn code_block_brackets_are_not_deletions() {
        let r = parse("C: In R, [`x[1]`|index] gets\n```r\nx[1]\ny <- x[[2]]\n```\nthe [first] item.");
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(r.cards.len(), 2);
        assert!(r.cards[1].front.contains("y <- x[[2]]\n```\nthe **[…]** item."));
    }

    #[test]
    fn math_is_not_a_deletion() {
        assert_eq!(math_len("$x$ rest"), Some(3));
        assert_eq!(math_len("$$\\frac{a}{b}$$ rest"), Some(15));
        assert_eq!(math_len(r"$a \$ b$"), Some(8));
        // Prices stay text.
        assert_eq!(math_len("$5 and $10"), None);
        assert_eq!(math_len("$ x$"), None);
        assert_eq!(math_len("$47.5 billion"), None);

        let r = parse("C: The interval $[0, 1]$ has [length $1$|size] and $|x|$ is [absolute].");
        assert_eq!(r.cards.len(), 2);
        assert_eq!(r.cards[0].front, "The interval $[0, 1]$ has **[size]** and $|x|$ is absolute.");
        assert_eq!(r.cards[0].back, "The interval $[0, 1]$ has **length $1$** and $|x|$ is absolute.");
    }
}
