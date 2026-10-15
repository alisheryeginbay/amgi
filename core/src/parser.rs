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
//! Brackets escaped with a backslash or inside `code` spans are literal. Escapes
//! are kept in the output, where Markdown renders them as plain brackets.

use std::fmt::Write;

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
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ParseWarning {
    pub line: u32,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct ParseResult {
    pub cards: Vec<ParsedCard>,
    pub warnings: Vec<ParseWarning>,
}

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

    for (idx, raw) in source.lines().enumerate() {
        let line_no = idx as u32 + 1;
        let line = raw.trim_end();

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
    }
    finish(pending, &mut out);
    out
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
    let Some(p) = pending else { return };
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
            });
        }
        Field::Cloze => parse_cloze(&join(&p.cloze), p.line, out),
    }
}

enum Segment<'a> {
    Text(&'a str),
    Deletion { answer: &'a str, hint: Option<&'a str> },
}

/// Byte offset of the first `target` that is neither backslash-escaped nor inside a code span.
fn find_markup(s: &str, target: char) -> Option<usize> {
    let (mut escaped, mut in_code) = (false, false);
    for (i, c) in s.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if !in_code => escaped = true,
            '`' => in_code = !in_code,
            c if c == target && !in_code => return Some(i),
            _ => {}
        }
    }
    None
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

fn parse_cloze(text: &str, line: u32, out: &mut ParseResult) {
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
}
