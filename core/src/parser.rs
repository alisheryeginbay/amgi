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
//! lines and lines starting `Q:` included. Brackets in code are literal; a deletion
//! there is written `{{answer}}` or `{{answer|hint}}`.
//!
//! A card's last lines may hold tags: `#korean #verbs`. A file may start with TOML
//! front matter between `+++` lines, giving the deck a `name` and default `tags`.
//!
//! A note starts at a line `@Vocabulary` naming its note type, defined in
//! `note-types/Vocabulary.toml` by its `fields` and `cards`, the rules for which cards
//! each note makes, such as `"Word -> Meaning"`. `Field: value` lines follow; each rule
//! makes one basic card from the note, its sides the rule's fields in order.

use std::fmt::Write;

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum, serde::Serialize)]
#[serde(rename_all = "lowercase")]
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
    /// The note the card comes from, shared by its cards; `None` for `Q:` and `C:` cards.
    pub note: Option<Arc<Note>>,
    /// The answer to type, for a card of a `=>` rule.
    pub typed: Option<Typed>,
}

/// The answer a card asks to have typed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Typed {
    /// The field it comes from, which the typing box names.
    pub field: String,
    /// Markdown.
    pub answer: String,
}

/// A note's type and the Markdown of each of its fields, in the type's order; empty
/// for a field the note doesn't fill.
#[derive(Debug, PartialEq, Eq)]
pub struct Note {
    pub note_type: String,
    pub values: Vec<String>,
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

/// Folder at the collection root holding note types, one `Name.toml` file each.
pub const NOTE_TYPES_DIR: &str = "note-types";

/// The collection's note types by name. `None` for a file with errors, so notes of
/// that type say so instead of that it is missing.
#[derive(Debug, Default, Clone)]
pub struct NoteTypes {
    types: HashMap<String, Option<NoteType>>,
}

impl NoteTypes {
    pub(crate) fn get(&self, name: &str) -> Option<&NoteType> {
        self.types.get(name)?.as_ref()
    }

    /// Add note type `name` from its TOML file, returning the file's problems.
    pub fn insert(&mut self, name: &str, toml: &str) -> Vec<ParseWarning> {
        let mut warnings = Vec::new();
        let note_type = NoteType::from_toml(toml, &mut warnings);
        self.types.insert(name.to_string(), note_type);
        warnings
    }
}

/// A note type's name and fields, for adding and browsing notes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, uniffi::Record)]
pub struct NoteTypeInfo {
    pub name: String,
    pub fields: Vec<String>,
}

impl NoteTypes {
    /// The note types without errors, by name.
    pub fn list(&self) -> Vec<NoteTypeInfo> {
        let mut list: Vec<NoteTypeInfo> = self
            .types
            .iter()
            .filter_map(|(name, t)| Some(NoteTypeInfo { name: name.clone(), fields: t.as_ref()?.fields.clone() }))
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct NoteTypeFile {
    fields: Vec<String>,
    cards: Vec<String>,
}

/// The fields of a deck's notes and the cards each note makes.
#[derive(Debug, Clone)]
pub(crate) struct NoteType {
    fields: Vec<String>,
    rules: Vec<Rule>,
}

/// One card per note: these fields on the front, these on the back, as indexes into
/// `NoteType::fields`.
#[derive(Debug, Clone, PartialEq)]
struct Rule {
    front: Vec<usize>,
    back: Vec<usize>,
    /// Written with `=>`: the first back field filled is typed in.
    typed: bool,
}

impl NoteType {
    /// The field a line starts, as in `Word: hola`, and the rest of the line.
    fn marker<'a>(&self, line: &'a str) -> Option<(usize, &'a str)> {
        self.fields.iter().enumerate().find_map(|(i, name)| {
            let rest = line.strip_prefix(name.as_str())?.strip_prefix(':')?;
            Some((i, rest.trim_start()))
        })
    }
}

const FRONT_MATTER: &str = "+++";

#[derive(Clone, Copy)]
enum Field {
    Question,
    Answer,
    Cloze,
    /// The last of `Pending::values`.
    Note,
}

struct Pending {
    line: u32,
    field: Field,
    question: Vec<String>,
    answer: Vec<String>,
    cloze: Vec<String>,
    /// A note's type, as its `@` line names it.
    note_type: String,
    /// A note's fields as written: the field's index and its lines.
    values: Vec<(usize, Vec<String>)>,
}

/// Cards in a deck file, without notes: they need their note types.
pub fn parse(source: &str) -> ParseResult {
    parse_deck(source, &NoteTypes::default())
}

/// Cards in a deck file, with notes of `types`.
pub fn parse_deck(source: &str, types: &NoteTypes) -> ParseResult {
    let mut out = ParseResult::default();
    let body_start = front_matter(source, &mut out);
    parse_body(source, body_start, types, out)
}

/// The cards in `source` from line `body_start` on, into `out`, which holds what its
/// front matter says.
fn parse_body(source: &str, body_start: usize, types: &NoteTypes, mut out: ParseResult) -> ParseResult {
    let mut pending: Option<Pending> = None;
    let mut fence = Fences::default();

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
            finish(pending.take(), types, &mut out);
            continue;
        }

        // In a note, the field a line starts, if any.
        let field = pending.as_ref().and_then(|p| p.note_type(types)).and_then(|t| t.marker(line));
        if let Some(rest) = marker(line, "Q:") {
            finish(pending.take(), types, &mut out);
            pending = Some(Pending::new(line_no, Field::Question, rest));
        } else if let Some(rest) = marker(line, "C:") {
            finish(pending.take(), types, &mut out);
            pending = Some(Pending::new(line_no, Field::Cloze, rest));
        } else if let Some(name) = note_start(&pending, line) {
            finish(pending.take(), types, &mut out);
            match types.types.get(name) {
                Some(Some(_)) => {
                    let mut p = Pending::new(line_no, Field::Note, "");
                    p.note_type = name.to_string();
                    pending = Some(p);
                }
                found => out.warnings.push(ParseWarning {
                    line: line_no,
                    message: match found {
                        Some(None) => format!("note type `{name}` has errors, see `{NOTE_TYPES_DIR}/{name}.toml`"),
                        _ => format!("there's no note type `{name}` in `{NOTE_TYPES_DIR}/`"),
                    },
                }),
            }
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
        } else if let Some(p) = pending.as_mut().filter(|p| matches!(p.field, Field::Note)) {
            match field {
                Some((f, rest)) if p.values.iter().all(|(given, _)| *given != f) => p.values.push((f, vec![rest.to_string()])),
                Some(_) => out.warnings.push(ParseWarning {
                    line: line_no,
                    message: "field given twice in a note; start another with its `@` line".into(),
                }),
                None if p.values.is_empty() => out.warnings.push(ParseWarning {
                    line: line_no,
                    message: format!("`@{}` is followed by a line that isn't one of its fields", p.note_type),
                }),
                None => p.current().push(line.to_string()),
            }
        } else if let Some(p) = pending.as_mut() {
            p.current().push(line.to_string());
        }
        // A fence opens on a marker line too, as in `A: ```r`.
        let content = ["Q:", "A:", "C:"]
            .iter()
            .find_map(|m| marker(line, m))
            .or_else(|| field.map(|(_, rest)| rest))
            .unwrap_or(line);
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
    finish(pending, types, &mut out);
    out
}

/// The note type named by a line `@Name` that starts a note: outside a `Q:` or `C:`
/// card, where such a line is just text.
fn note_start<'a>(pending: &Option<Pending>, line: &'a str) -> Option<&'a str> {
    if pending.as_ref().is_some_and(|p| !matches!(p.field, Field::Note)) {
        return None;
    }
    line.strip_prefix('@').map(str::trim).filter(|name| !name.is_empty())
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

impl NoteType {
    /// A note type from its file's `fields` and `cards`. Without `cards`, a note's
    /// first field is the front and the rest the back. `None` if it has no usable
    /// fields; rules with problems are left out.
    fn from_toml(toml: &str, warnings: &mut Vec<ParseWarning>) -> Option<NoteType> {
        match toml::from_str::<NoteTypeFile>(toml) {
            Ok(file) => {
                // A problem's line is the first that mentions it, from `cards` on for a rule.
                let lines: Vec<&str> = toml.lines().collect();
                let cards = lines.iter().position(|l| l.trim_start().starts_with("cards")).unwrap_or(0);
                let line_of = |needle: &str, rule: bool| {
                    let (from, needle) = if rule { (cards, toml_string(needle)) } else { (0, needle.to_string()) };
                    lines[from..].iter().position(|l| l.contains(&needle)).map_or(1, |i| (from + i) as u32 + 1)
                };
                note_type(&file.fields, &file.cards, &mut |message, needle, rule| {
                    warnings.push(ParseWarning { line: line_of(needle, rule), message })
                })
            }
            Err(e) => {
                let line = e.span().map_or(1, |span| toml[..span.start].lines().count().max(1) as u32);
                warnings.push(ParseWarning { line, message: format!("invalid note type: {}", e.message()) });
                None
            }
        }
    }
}

/// `warn` takes a problem, the text it is about and whether that is a card rule.
fn note_type(fields: &[String], cards: &[String], warn: &mut dyn FnMut(String, &str, bool)) -> Option<NoteType> {
    let mut names: Vec<String> = Vec::new();
    for name in fields.iter().map(|f| f.trim()) {
        if name.is_empty() || name.contains(':') || ["Q", "A", "C"].contains(&name) {
            warn(format!("`{name}` can't be a field name"), name, false);
        } else if names.iter().any(|n| n == name) {
            warn(format!("field `{name}` is listed twice"), name, false);
        } else {
            names.push(name.to_string());
        }
    }
    if names.is_empty() {
        if !cards.is_empty() {
            warn("`cards` needs `fields`".into(), "cards", false);
        }
        return None;
    }
    let mut rules: Vec<Rule> = Vec::new();
    if cards.is_empty() {
        if names.len() < 2 {
            warn("notes need two fields, or `cards` to say which make a card".into(), "fields", false);
            return None;
        }
        rules.push(Rule { front: vec![0], back: (1..names.len()).collect(), typed: false });
    }
    'rules: for text in cards {
        let arrow = [text.find("->"), text.find("=>")].into_iter().flatten().min();
        let Some(at) = arrow else {
            warn(format!("card rule `{text}` has no `->` or `=>`"), text, true);
            continue;
        };
        let typed = text[at..].starts_with("=>");
        let (front, back) = (&text[..at], &text[at + 2..]);
        let mut side = |part: &str| -> Option<Vec<usize>> {
            let mut out = Vec::new();
            for name in part.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                match names.iter().position(|n| n == name) {
                    Some(i) => out.push(i),
                    None => {
                        warn(format!("card rule `{text}` names `{name}`, which isn't in `fields`"), text, true);
                        return None;
                    }
                }
            }
            if out.is_empty() {
                warn(format!("card rule `{text}` has an empty side"), text, true);
                return None;
            }
            Some(out)
        };
        let (Some(front), Some(back)) = (side(front), side(back)) else { continue 'rules };
        if rules.iter().any(|r| r.front == front) {
            warn(format!("card rule `{text}` has the same front as another"), text, true);
            continue;
        }
        rules.push(Rule { front, back, typed });
    }
    Some(NoteType { fields: names, rules })
}

/// A TOML basic string.
pub(crate) fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
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

/// A new note of type `note_type` as Markdown ending in a newline, starting with its
/// `@` line, with the ids of the cards it makes; or why it can't be written. `values`
/// follow the note type's fields; empty ones are left out. Blank lines outside code
/// blocks are dropped, and lines that would start a field, a card, a note or a tag
/// line are escaped.
pub fn write_note(note_type: &str, types: &NoteTypes, values: &[String], tags: &[String]) -> Result<(String, Vec<String>), String> {
    let Some(Some(t)) = types.types.get(note_type) else {
        return Err(format!("There's no note type “{note_type}”."));
    };
    let mut out = format!("@{note_type}\n");
    for (name, value) in t.fields.iter().zip(values) {
        let mut fence = Fences::default();
        let mut lines: Vec<String> = Vec::new();
        for line in value.lines().map(str::trim_end) {
            if fence.inside() || !line.trim().is_empty() {
                lines.push(line.to_string());
            }
            fence.step(line);
        }
        let Some((first, rest)) = lines.split_first_mut() else { continue };
        let mut fence = Fences::default();
        fence.step(first);
        for line in rest.iter_mut() {
            if !fence.inside() {
                escape_line(line, t);
            }
            fence.step(line);
        }
        writeln!(out, "{name}: {}", first.trim_start()).unwrap();
        for line in rest.iter() {
            writeln!(out, "{line}").unwrap();
        }
    }
    let mut own: Vec<String> = Vec::new();
    for tag in tags.iter().flat_map(|t| t.split_whitespace()).filter_map(clean_tag) {
        if !own.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
            own.push(tag);
        }
    }
    if !own.is_empty() {
        let line: Vec<String> = own.iter().map(|t| format!("#{t}")).collect();
        writeln!(out, "{}", line.join(" ")).unwrap();
    }
    let parsed = parse_deck(&out, types);
    if let Some(warning) = parsed.warnings.iter().find(|w| !w.message.starts_with("note makes no cards")) {
        return Err(sentence(&warning.message));
    }
    if parsed.cards.is_empty() {
        return Err("Fill in a field for the front of a card and one for its back.".into());
    }
    Ok((out, parsed.cards.into_iter().map(|c| c.id).collect()))
}

/// Escape a line inside a note's field that would otherwise start a field of `t`, a
/// card, a note or a tag line. The backslash doesn't show: Markdown drops it.
pub(crate) fn escape_line(line: &mut String, t: &NoteType) {
    let colon = ["Q:", "A:", "C:"]
        .into_iter()
        .find(|m| line.starts_with(m))
        .map(|m| m.len() - 1)
        .or_else(|| t.marker(line).map(|(f, _)| t.fields[f].len()));
    if let Some(at) = colon {
        line.insert(at, '\\');
    } else if line.starts_with('@') || tag_line(line).is_some() {
        line.insert(0, '\\');
    }
}

/// A new card as Markdown, ending in a newline, or why it can't be written. A basic
/// card takes `front` and `back`; a cloze only `front`. Blank lines outside code blocks
/// are dropped, as they would end the card. The result reads back as this one card.
pub fn write_card(kind: CardKind, front: &str, back: &str, tags: &[String]) -> Result<String, String> {
    let mut out = String::new();
    match kind {
        CardKind::Basic => {
            write_field(&mut out, "Q:", front, "question")?;
            write_field(&mut out, "A:", back, "answer")?;
        }
        CardKind::Cloze => write_field(&mut out, "C:", front, "text")?,
    }
    let mut own: Vec<String> = Vec::new();
    for tag in tags.iter().flat_map(|t| t.split_whitespace()).filter_map(clean_tag) {
        if !own.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
            own.push(tag);
        }
    }
    if !own.is_empty() {
        let line: Vec<String> = own.iter().map(|t| format!("#{t}")).collect();
        writeln!(out, "{}", line.join(" ")).unwrap();
    }

    let parsed = parse(&out);
    if let Some(warning) = parsed.warnings.first() {
        return Err(sentence(&warning.message));
    }
    match parsed.cards.first() {
        None => Err("A cloze needs at least one [gap].".into()),
        Some(card) if card.tags != own => Err("Tags must be words without spaces.".into()),
        Some(_) => Ok(out),
    }
}

/// One field's lines after its marker, such as `Q:`.
fn write_field(out: &mut String, marker_tag: &str, text: &str, name: &str) -> Result<(), String> {
    let mut fence = Fences::default();
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim_end) {
        if fence.inside() || !line.trim().is_empty() {
            lines.push(line);
        }
        fence.step(line);
    }
    let Some(first) = lines.first() else {
        return Err(format!("The {name} is empty."));
    };
    for line in &lines[1..] {
        if let Some(m) = ["Q:", "A:", "C:"].into_iter().find(|m| line.starts_with(m)) {
            return Err(format!("A line of the {name} starts with `{m}`, which would start a new card."));
        }
    }
    if lines.len() > 1 && !fence.inside() && lines.last().is_some_and(|l| tag_line(l).is_some()) {
        return Err(format!("The {name}'s last line would be read as tags."));
    }
    writeln!(out, "{marker_tag} {}", first.trim_start()).unwrap();
    for line in &lines[1..] {
        writeln!(out, "{line}").unwrap();
    }
    Ok(())
}

/// A parser warning as a sentence: `card has…` → `Card has….`
fn sentence(message: &str) -> String {
    let mut chars = message.chars();
    let first = chars.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
    format!("{first}{}.", chars.as_str())
}

/// Add or remove `#tag` on the card starting at 1-based `line`, in the card's own tag
/// lines; the rest of the file is kept byte for byte. `None` when no card starts
/// there or its own tags already say so (a front matter tag cannot be removed here).
pub fn set_card_tag(source: &str, types: &NoteTypes, line: u32, tag: &str, present: bool) -> Option<String> {
    let mut lines: Vec<String> = source.split_inclusive('\n').map(String::from).collect();
    let start = line.checked_sub(1)? as usize;
    let first = lines.get(start)?.trim_end();
    fn card_marker(l: &str) -> Option<&str> {
        marker(l, "Q:").or(marker(l, "C:"))
    }
    // The card runs to the next blank line or card marker outside a code block, and a
    // note also to the next note, as when parsing.
    let mut fence = Fences::default();
    let note_type = match card_marker(first) {
        Some(rest) => {
            fence.step(rest);
            None
        }
        None => match types.types.get(note_start(&None, first)?) {
            Some(Some(t)) => Some(t),
            _ => return None,
        },
    };
    let mut end = start + 1;
    while let Some(l) = lines.get(end).map(|l| l.trim_end()) {
        let ends_note = note_type.is_some() && note_start(&None, l).is_some();
        if !fence.inside() && (l.trim().is_empty() || card_marker(l).is_some() || ends_note) {
            break;
        }
        let content = match note_type {
            _ if fence.inside() => l,
            Some(t) => t.marker(l).map_or(l, |(_, rest)| rest),
            None => marker(l, "A:").unwrap_or(l),
        };
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
            note_type: String::new(),
            values: Vec::new(),
        };
        if !matches!(field, Field::Note) {
            p.current().push(first.to_string());
        }
        p
    }

    /// A note's type, if this is a note of a known one.
    fn note_type<'a>(&self, types: &'a NoteTypes) -> Option<&'a NoteType> {
        if !matches!(self.field, Field::Note) {
            return None;
        }
        types.types.get(&self.note_type)?.as_ref()
    }

    fn current(&mut self) -> &mut Vec<String> {
        match self.field {
            Field::Question => &mut self.question,
            Field::Answer => &mut self.answer,
            Field::Cloze => &mut self.cloze,
            Field::Note => match self.values.last_mut() {
                Some((_, lines)) => lines,
                // Before its first field: tags only, kept for `take_tags`.
                None => &mut self.question,
            },
        }
    }
}

fn join(lines: &[String]) -> String {
    lines.join("\n").trim().to_string()
}

fn finish(pending: Option<Pending>, types: &NoteTypes, out: &mut ParseResult) {
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
                note: None,
                typed: None,
            });
        }
        Field::Cloze => parse_cloze(&join(&p.cloze), p.line, tags, out),
        Field::Note => note_cards(&p, types, tags, out),
    }
}

/// One basic card for each of the note type's rules with a front field and a back
/// field the note fills. Each side is its filled fields in the rule's order, a blank
/// line apart. A card's id is that of a `Q:` card with the same front, so it's the
/// same whether its answer is typed or not.
fn note_cards(p: &Pending, types: &NoteTypes, tags: Vec<String>, out: &mut ParseResult) {
    let Some(note_type) = p.note_type(types) else { return };
    let value = |field: usize| p.values.iter().find(|(f, _)| *f == field).map(|(_, lines)| join(lines)).filter(|v| !v.is_empty());
    let side = |fields: &[usize]| fields.iter().filter_map(|&f| value(f)).collect::<Vec<_>>().join("\n\n");
    let note = Arc::new(Note {
        note_type: p.note_type.clone(),
        values: (0..note_type.fields.len()).map(|f| value(f).unwrap_or_default()).collect(),
    });
    let mut cards = Vec::new();
    for rule in &note_type.rules {
        let back = side(&rule.back);
        let front = side(&rule.front);
        if front.is_empty() || back.is_empty() {
            continue;
        }
        cards.push(ParsedCard {
            id: card_id(&["basic", &normalize(&front)]),
            kind: CardKind::Basic,
            front,
            back,
            line: p.line,
            tags: tags.clone(),
            note: Some(note.clone()),
            typed: rule
                .typed
                .then(|| rule.back.iter().find_map(|&f| Some(Typed { field: note_type.fields[f].clone(), answer: value(f)? })))
                .flatten(),
        });
    }
    if cards.is_empty() {
        out.warnings.push(ParseWarning {
            line: p.line,
            message: "note makes no cards: each card needs a front field and a back field filled in".into(),
        });
    }
    out.cards.extend(cards);
}

enum Segment<'a> {
    Text(&'a str),
    /// `code` for a `{{…}}` deletion inside code.
    Deletion { answer: &'a str, hint: Option<&'a str>, code: bool },
}

/// Open and close a deletion inside code in a cloze card's sides, where `**` could be
/// code. The app strips them and marks the text between.
pub const CODE_GAP_OPEN: char = '\u{E001}';
pub const CODE_GAP_CLOSE: char = '\u{E002}';

/// A cloze's text and deletions: `[…]` outside code, `{{…}}` inside code spans and
/// fenced blocks. Outside code, brackets escaped with a backslash, in an image or in
/// math are literal. `Err` names a deletion left open.
fn segments(text: &str) -> Result<Vec<Segment<'_>>, &'static str> {
    let mut segs = Vec::new();
    let (mut text_start, mut skip_to, mut escaped) = (0, 0, false);
    // The open deletion: whether it is in code, where it starts, its first `|`.
    let mut open: Option<(bool, usize, Option<usize>)> = None;
    // Length of the backtick run that opened the current code span, which closes only
    // on a run of the same length.
    let mut span: Option<usize> = None;
    let mut fence = Fences::default();

    for (i, c) in text.char_indices() {
        if i < skip_to {
            continue;
        }
        // A fence line opens or closes a block and is otherwise literal.
        if span.is_none() && (i == 0 || text[..i].ends_with('\n')) {
            let line_end = text[i..].find('\n').map_or(text.len(), |n| i + n);
            let was = fence.inside();
            fence.step(text[i..line_end].trim_end());
            if fence.inside() != was {
                skip_to = line_end;
                continue;
            }
        }
        let in_code = span.is_some() || fence.inside();
        let rest = &text[i..];
        match c {
            _ if escaped => escaped = false,
            '`' if !fence.inside() => {
                let run = rest.len() - rest.trim_start_matches('`').len();
                skip_to = i + run;
                span = match span {
                    None => Some(run),
                    Some(len) if len == run => None,
                    len => len,
                };
            }
            '\\' if !in_code => escaped = true,
            '!' if !in_code => skip_to = image_len(rest).map_or(0, |len| i + len),
            '$' if !in_code => skip_to = math_len(rest).map_or(0, |len| i + len),
            '|' => {
                if let Some((code, _, bar @ None)) = &mut open
                    && *code == in_code
                {
                    *bar = Some(i);
                }
            }
            '[' if !in_code && open.is_none() => open = Some((false, i + 1, None)),
            '{' if in_code && open.is_none() && rest.starts_with("{{") => {
                open = Some((true, i + 2, None));
                skip_to = i + 2;
            }
            ']' | '}' => {
                let Some((code, start, bar)) = open else { continue };
                let closes = if code { in_code && rest.starts_with("}}") } else { !in_code && c == ']' };
                if !closes {
                    continue;
                }
                let delimiter = if code { 2 } else { 1 };
                if start - delimiter > text_start {
                    segs.push(Segment::Text(&text[text_start..start - delimiter]));
                }
                let (answer, hint) = match bar {
                    Some(bar) => (text[start..bar].trim(), Some(text[bar + 1..i].trim()).filter(|h| !h.is_empty())),
                    None => (text[start..i].trim(), None),
                };
                segs.push(Segment::Deletion { answer, hint, code });
                open = None;
                text_start = i + delimiter;
                skip_to = text_start;
            }
            _ => {}
        }
    }
    match open {
        Some((true, ..)) => Err("unclosed `{{` in cloze"),
        Some((false, ..)) => Err("unclosed `[` in cloze"),
        None => {
            if text_start < text.len() {
                segs.push(Segment::Text(&text[text_start..]));
            }
            Ok(segs)
        }
    }
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

fn parse_cloze(text: &str, line: u32, tags: Vec<String>, out: &mut ParseResult) {
    let segs = match segments(text) {
        Ok(segs) => segs,
        Err(message) => {
            out.warnings.push(ParseWarning { line, message: message.into() });
            return;
        }
    };
    let deletions = segs
        .iter()
        .filter(|s| matches!(s, Segment::Deletion { .. }))
        .count();
    if deletions == 0 {
        out.warnings.push(ParseWarning {
            line,
            message: "cloze has no `[...]` deletions (`{{...}}` in code)".into(),
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
                Segment::Deletion { answer, hint, code } => {
                    let hint = hint.unwrap_or("…");
                    if n == target && *code {
                        let _ = write!(front, "{CODE_GAP_OPEN}[{hint}]{CODE_GAP_CLOSE}");
                        let _ = write!(back, "{CODE_GAP_OPEN}{answer}{CODE_GAP_CLOSE}");
                    } else if n == target {
                        let _ = write!(front, "**[{hint}]**");
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
            note: None,
            typed: None,
        });
    }
}

/// Collapse whitespace so reflowing a card does not reset its history.
fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn card_id(parts: &[&str]) -> String {
    format!("{:016x}", stable_hash(parts))
}

/// FNV-1a 64-bit: stable across Rust versions and platforms, unlike `DefaultHasher`.
pub(crate) fn stable_hash(parts: &[&str]) -> u64 {
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
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_card_round_trips() {
        let tags = vec!["#verbs".to_string(), "motion verbs".to_string()];
        let md = write_card(CardKind::Basic, "What is **2 + 2**?\n\n", "4\n\nbecause 2 + 2 = 4", &tags).unwrap();
        assert_eq!(md, "Q: What is **2 + 2**?\nA: 4\nbecause 2 + 2 = 4\n#verbs #motion\n");
        let card = &parse(&md).cards[0];
        assert_eq!((card.front.as_str(), card.back.as_str()), ("What is **2 + 2**?", "4\nbecause 2 + 2 = 4"));
        assert_eq!(card.tags, ["verbs", "motion"]);

        // Blank lines stay inside code blocks.
        let md = write_card(CardKind::Cloze, "Print it:\n```py\nx = 1\n\n{{print}}(x)\n```", "", &[]).unwrap();
        assert!(md.contains("x = 1\n\n"));
        assert_eq!(parse(&md).cards.len(), 1);

        let md = write_card(CardKind::Cloze, "The capital of [France] is [Paris].", "ignored", &[]).unwrap();
        assert_eq!(md, "C: The capital of [France] is [Paris].\n");
        assert_eq!(parse(&md).cards.len(), 2);
    }

    #[test]
    fn write_card_rejects() {
        let err = |kind, front, back| write_card(kind, front, back, &[]).unwrap_err();
        assert_eq!(err(CardKind::Basic, " \n", "x"), "The question is empty.");
        assert_eq!(err(CardKind::Basic, "x", ""), "The answer is empty.");
        assert!(err(CardKind::Cloze, "No gaps here.", "").starts_with("Cloze has no"));
        assert!(err(CardKind::Basic, "x\nQ: y", "z").contains("`Q:`"));
        assert!(err(CardKind::Basic, "x", "y\n#oops").contains("tags"));
        assert!(err(CardKind::Basic, "```\ncode", "y").contains("closing fence"));
    }

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
        let a = set_card_tag(source, &NoteTypes::default(), 5, SUSPENDED_TAG, true).unwrap();
        assert!(a.contains("A: 1\n#geo #suspended\n\nC:"));
        // Gets a tag line of its own, with the file's line ending.
        let b = set_card_tag(&a, &NoteTypes::default(), 9, SUSPENDED_TAG, true).unwrap();
        assert!(b.contains("more text\r\n#suspended\r\n\nQ: last"));
        // The last card, without a final newline.
        let c = set_card_tag(&b, &NoteTypes::default(), 13, SUSPENDED_TAG, true).unwrap();
        assert!(c.ends_with("A: 3\n#suspended"));
        let cards = parse(&c).cards;
        assert!(cards.iter().all(ParsedCard::is_suspended));
        // Tags are not part of a card's identity, so history is kept.
        assert_eq!(cards.iter().map(|c| &c.id).collect::<Vec<_>>(), parse(source).cards.iter().map(|c| &c.id).collect::<Vec<_>>());

        let mut back = c;
        for line in [13, 9, 5] {
            back = set_card_tag(&back, &NoteTypes::default(), line, "Suspended", false).unwrap();
        }
        // Removing restores the file, now with a final newline.
        assert_eq!(back, format!("{source}\n"));
        // Nothing to do, or no card at that line.
        assert_eq!(set_card_tag(source, &NoteTypes::default(), 5, SUSPENDED_TAG, false), None);
        assert_eq!(set_card_tag(source, &NoteTypes::default(), 6, SUSPENDED_TAG, true), None);
        // A deck tag cannot be removed from one card.
        assert_eq!(set_card_tag(source, &NoteTypes::default(), 5, "x", false), None);
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
        let tagged = set_card_tag(source, &NoteTypes::default(), 1, SUSPENDED_TAG, true).unwrap();
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
    fn deletions_in_code_use_braces() {
        let source = "C: Load [expm|package]:\n```r\nlibrary(\"{{expm}}\")\nx[[2]] <- {{ expm | function }}(m)\n```\nor `{{require}}(x[1])`.";
        let r = parse(source);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(r.cards.len(), 4);
        let (open, close) = (CODE_GAP_OPEN, CODE_GAP_CLOSE);
        assert_eq!(r.cards[0].front, "Load **[package]**:\n```r\nlibrary(\"expm\")\nx[[2]] <- expm(m)\n```\nor `require(x[1])`.");
        assert_eq!(r.cards[1].front, format!("Load expm:\n```r\nlibrary(\"{open}[…]{close}\")\nx[[2]] <- expm(m)\n```\nor `require(x[1])`."));
        assert_eq!(r.cards[2].front, format!("Load expm:\n```r\nlibrary(\"expm\")\nx[[2]] <- {open}[function]{close}(m)\n```\nor `require(x[1])`."));
        assert_eq!(r.cards[2].back, format!("Load expm:\n```r\nlibrary(\"expm\")\nx[[2]] <- {open}expm{close}(m)\n```\nor `require(x[1])`."));
        assert!(r.cards[3].back.ends_with(&format!("`{open}require{close}(x[1])`.")));

        // Braces outside code are text; an open one in code is a mistake.
        assert_eq!(parse("C: {{x}} and [y]").cards[0].front, "{{x}} and **[…]**");
        assert_eq!(parse("C: `{{x`").warnings[0].message, "unclosed `{{` in cloze");
        // A `~~~` block's brackets are literal too.
        let r = parse("C: ~~~\nv[0] = {{1}}\n~~~");
        assert_eq!(r.cards.len(), 1);
        assert_eq!(r.cards[0].back, format!("~~~\nv[0] = {open}1{close}\n~~~"));
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

    const FOOD: &str = "+++\nname = \"Spanish\"\n+++\n";

    fn types() -> NoteTypes {
        let mut types = NoteTypes::default();
        let food = "fields = [\"Word\", \"Meaning\", \"Audio\", \"Sentence\"]\ncards = [\"Word, Audio -> Meaning, Sentence\", \"Meaning -> Word\"]\n";
        assert!(types.insert("Food", food).is_empty());
        assert!(types.insert("Pair", "fields = [\"Word\", \"Word Audio\"]").is_empty());
        assert!(types.insert("Code", "fields = [\"Task\", \"Code\"]").is_empty());
        types.insert("Broken", "fields = 3");
        types
    }

    fn food(source: &str) -> ParseResult {
        parse_deck(source, &types())
    }

    #[test]
    fn notes_make_a_card_per_rule() {
        let source = format!("{FOOD}\n@Food\nWord: manzana\nMeaning: apple\nAudio: ![](media/manzana.mp3)\nSentence: Me como\nuna manzana.\n#food\n\nQ: plain?\nA: still works\n");
        let r = food(&source);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(r.cards.len(), 3);
        let (a, b) = (&r.cards[0], &r.cards[1]);
        assert_eq!(a.front, "manzana\n\n![](media/manzana.mp3)");
        assert_eq!(a.back, "apple\n\nMe como\nuna manzana.");
        assert_eq!((b.front.as_str(), b.back.as_str()), ("apple", "manzana"));
        assert_eq!((a.line, a.tags.clone()), (5, vec!["food".to_string()]));
        assert_eq!(a.kind, CardKind::Basic);
        let note = a.note.as_ref().unwrap();
        assert_eq!((note.note_type.as_str(), note.values[1].as_str()), ("Food", "apple"));
        // A note's card is identified like a `Q:` card with its front.
        assert_eq!(b.id, parse("Q: apple\nA: something").cards[0].id);
        assert_eq!(r.cards[2].front, "plain?");
    }

    #[test]
    fn typed_rules_ask_for_their_first_back_field() {
        let mut types = NoteTypes::default();
        let spell = "fields = [\"Word\", \"Audio\", \"Meaning\"]\ncards = [\"Meaning => Audio, Word\", \"Word -> Meaning\"]";
        assert!(types.insert("Spell", spell).is_empty());
        let r = parse_deck("@Spell\nWord: hola\nMeaning: hello\n", &types);
        // The audio is missing, so the word is what's typed.
        let typed = r.cards[0].typed.as_ref().unwrap();
        assert_eq!((r.cards[0].front.as_str(), typed.field.as_str(), typed.answer.as_str()), ("hello", "Word", "hola"));
        assert_eq!(r.cards[1].typed, None);
        // Typed or not, a card is the same card.
        let plain = "fields = [\"Word\", \"Audio\", \"Meaning\"]\ncards = [\"Meaning -> Audio, Word\"]";
        let mut other = NoteTypes::default();
        other.insert("Spell", plain);
        assert_eq!(parse_deck("@Spell\nWord: hola\nMeaning: hello\n", &other).cards[0].id, r.cards[0].id);
    }

    #[test]
    fn note_ids_follow_the_front() {
        let ids = |note: &str| food(&format!("@Food\n{note}")).cards.into_iter().map(|c| c.id).collect::<Vec<_>>();
        let before = ids("Word: pan\nMeaning: bread\nAudio: ![](pan.mp3)");
        // Back fields, field order and a new field elsewhere keep history.
        assert_eq!(before, ids("Meaning: bread\nAudio: ![](pan.mp3)\nWord: pan\nSentence: Compro pan."));
        assert_ne!(before[0], ids("Word: pan!\nMeaning: bread\nAudio: ![](pan.mp3)")[0]);
    }

    #[test]
    fn notes_skip_cards_with_missing_fields() {
        // No audio: the first card shows only the word.
        let r = food("@Food\nWord: queso\nMeaning: cheese\n");
        assert_eq!(r.cards.iter().map(|c| c.front.as_str()).collect::<Vec<_>>(), ["queso", "cheese"]);
        // Nothing on a card's back.
        let r = food("@Food\nWord: queso\nAudio: ![](q.mp3)\n");
        assert!(r.cards.is_empty());
        let r = food("Q: a\nA: b\n\n@Food\nSentence: only this\n");
        assert_eq!(r.cards.len(), 1);
        assert!(r.warnings[0].message.starts_with("note makes no cards"));
        assert_eq!(r.warnings[0].line, 4);
    }

    #[test]
    fn notes_start_at_their_type() {
        let r = food("@Food\nWord: uno\nMeaning: one\n@ Food \nWord: dos\nMeaning: two\nnotes in prose\nare part of it\nQ: next\nA: card\n");
        let cards: Vec<_> = r.cards.iter().map(|c| (c.front.as_str(), c.back.as_str(), c.line)).collect();
        let two = "two\nnotes in prose\nare part of it";
        assert_eq!(cards, [("uno", "one", 1), ("one", "uno", 1), ("dos", two, 4), (two, "dos", 4), ("next", "card", 9)]);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        // A field given twice, and a line before the first field.
        let r = food("@Food\nnot a field\nWord: uno\nMeaning: one\nWord: dos\n");
        let warnings: Vec<_> = r.warnings.iter().map(|w| (w.line, w.message.as_str())).collect();
        assert_eq!(
            warnings,
            [
                (2, "`@Food` is followed by a line that isn't one of its fields"),
                (5, "field given twice in a note; start another with its `@` line"),
            ]
        );
        assert_eq!(r.cards[0].front, "uno");
        // Unknown and broken note types; their fields are left alone.
        let messages = |source: &str| food(source).warnings.into_iter().map(|w| w.message).collect::<Vec<_>>();
        assert_eq!(messages("@Nope\nWord: x\nMeaning: y\n"), ["there's no note type `Nope` in `note-types/`"]);
        assert_eq!(messages("@Broken\nWord: x\n"), ["note type `Broken` has errors, see `note-types/Broken.toml`"]);
        assert!(food("@Nope\nWord: x\nMeaning: y\n").cards.is_empty());
        // Without note types, there are no notes.
        assert!(parse("@Food\nWord: x\nMeaning: y\n").cards.is_empty());
    }

    #[test]
    fn note_lines_inside_cards_are_text() {
        let r = food("Q: What is it?\nA: A fruit.\n@Food\nMeaning: apple\n");
        assert_eq!(r.cards.len(), 1);
        assert_eq!(r.cards[0].back, "A fruit.\n@Food\nMeaning: apple");
        // Field lines outside a note are ignored, as other lines between cards are.
        assert!(food("Word: hola\nMeaning: hello").cards.is_empty());
        // `Word Audio:` is not `Word:`.
        let r = food("@Pair\nWord Audio: ![](a.mp3)\nWord: hola\n");
        assert_eq!((r.cards[0].front.as_str(), r.cards[0].back.as_str()), ("hola", "![](a.mp3)"));
    }

    #[test]
    fn fields_hold_code_blocks() {
        let r = food("@Code\nTask: Print\nCode: ```python\nTask: not a field\n\n@Food\nprint(1)\n```\n");
        assert_eq!(r.cards.len(), 1);
        assert_eq!(r.cards[0].back, "```python\nTask: not a field\n\n@Food\nprint(1)\n```");
    }

    #[test]
    fn note_type_problems() {
        let warnings = |toml: &str| NoteTypes::default().insert("X", toml).into_iter().map(|w| (w.line, w.message)).collect::<Vec<_>>();
        assert_eq!(warnings("cards = [\"a -> b\"]"), [(1, "`cards` needs `fields`".to_string())]);
        assert_eq!(warnings("fields = [\"Only\"]")[0].1, "notes need two fields, or `cards` to say which make a card");
        assert!(warnings("fields = 3")[0].1.starts_with("invalid note type"));
        assert!(warnings("feilds = []")[0].1.contains("unknown field"));
        let messages: Vec<_> = warnings("fields = [\"A\", \"x\", \"x\", \"y\"]\ncards = [\n  \"x\",\n  \"x -> z\",\n  \"x -> \",\n  \"x -> y\",\n  \"x -> y, x\",\n]");
        assert_eq!(
            messages,
            [
                (1, "`A` can't be a field name".to_string()),
                (1, "field `x` is listed twice".to_string()),
                (3, "card rule `x` has no `->` or `=>`".to_string()),
                (4, "card rule `x -> z` names `z`, which isn't in `fields`".to_string()),
                (5, "card rule `x -> ` has an empty side".to_string()),
                (7, "card rule `x -> y, x` has the same front as another".to_string()),
            ]
        );
    }

    #[test]
    fn write_note_reads_back() {
        let values = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (block, ids) = write_note(
            "Food",
            &types(),
            &values(&["manzana", " apple \n\nMeaning: not a field\n@Food\nA: no\n#red", "", "Q: a fruit\n```\n\nMeaning: code\n```"]),
            &["#food".into(), "fruit food".into()],
        )
        .unwrap();
        assert_eq!(
            block,
            "@Food\nWord: manzana\nMeaning: apple\nMeaning\\: not a field\n\\@Food\nA\\: no\n\\#red\nSentence: Q: a fruit\n```\n\nMeaning: code\n```\n#food #fruit\n"
        );
        let r = food(&format!("{FOOD}{block}"));
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(r.cards.iter().map(|c| c.id.clone()).collect::<Vec<_>>(), ids);
        assert_eq!(r.cards[0].back, "apple\nMeaning\\: not a field\n\\@Food\nA\\: no\n\\#red\n\nQ: a fruit\n```\n\nMeaning: code\n```");
        assert_eq!(r.cards[0].tags, ["food", "fruit"]);

        let err = |note_type: &str, v: &[&str]| write_note(note_type, &types(), &values(v), &[]).unwrap_err();
        assert_eq!(err("Food", &["", "", "", "only a sentence"]), "Fill in a field for the front of a card and one for its back.");
        assert_eq!(err("Nope", &["a", "b"]), "There's no note type “Nope”.");
    }

    #[test]
    fn suspending_a_note_tags_its_last_line() {
        let source = format!("{FOOD}\n@Food\nWord: uno\nMeaning: one\n@Food\nWord: dos\nMeaning: two\n```\n\n```\n#n\n\nQ: x\nA: y\n");
        let once = set_card_tag(&source, &types(), 8, SUSPENDED_TAG, true).unwrap();
        assert!(once.contains("Word: uno\nMeaning: one\n@Food"));
        assert!(once.contains("```\n#n #suspended\n\nQ: x"));
        let first = set_card_tag(&source, &types(), 5, SUSPENDED_TAG, true).unwrap();
        assert!(first.contains("Meaning: one\n#suspended\n@Food"));
        let r = food(&first);
        assert_eq!(r.cards.iter().map(|c| c.is_suspended()).collect::<Vec<_>>(), [true, true, false, false, false]);
        assert_eq!(set_card_tag(&first, &types(), 5, SUSPENDED_TAG, false).unwrap(), source);
        // A line that starts no note or card.
        assert_eq!(set_card_tag(&source, &types(), 6, SUSPENDED_TAG, true), None);
    }
}
