//! Cards added, removed and changed between two versions of a collection: a git
//! revision, the staged files or the files on disk that git would commit.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use amgi_core::parser::{self, NOTE_TYPES_DIR, NoteTypes, ParsedCard};
use amgi_core::{CARDS_DIR, CardKind};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Where one side of a diff reads its files from.
pub enum Source {
    /// A git revision, such as `HEAD` or `main~3`.
    Rev(String),
    /// The files staged for the next commit.
    Staged,
    /// The files on disk, leaving out those git ignores.
    Disk,
}

impl Source {
    pub fn label(&self) -> &str {
        match self {
            Source::Rev(rev) => rev,
            Source::Staged => "staged",
            Source::Disk => "disk",
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Card {
    pub id: String,
    pub deck: String,
    /// Relative to the collection root.
    pub path: String,
    pub line: u32,
    pub kind: CardKind,
    /// Markdown.
    pub front: String,
    /// Markdown.
    pub back: String,
    /// Without `#`, the deck's front matter tags included.
    pub tags: Vec<String>,
}

#[derive(Debug, PartialEq, serde::Serialize)]
pub struct Changed {
    pub card: Card,
    pub before: Card,
    /// What differs: `question`, `answer`, `tags` and `deck`. A card moved within its
    /// file isn't changed.
    pub changes: Vec<&'static str>,
}

#[derive(Debug, Default, PartialEq, serde::Serialize)]
pub struct Diff {
    pub added: Vec<Card>,
    /// Their review history stays in the database but no card uses it any more.
    pub removed: Vec<Card>,
    pub changed: Vec<Changed>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// Compare by id, which comes from a card's text: a card whose question is edited is
/// removed and added, as it starts over.
pub fn compare(before: Vec<Card>, after: Vec<Card>) -> Diff {
    let mut old: HashMap<String, Card> = before.iter().map(|c| (c.id.clone(), c.clone())).collect();
    let mut diff = Diff::default();
    for card in after {
        let Some(prev) = old.remove(&card.id) else {
            diff.added.push(card);
            continue;
        };
        let changes: Vec<_> = [
            ("question", prev.front != card.front),
            ("answer", prev.back != card.back),
            ("tags", prev.tags != card.tags),
            ("deck", prev.path != card.path),
        ]
        .into_iter()
        .filter_map(|(name, differs)| differs.then_some(name))
        .collect();
        if !changes.is_empty() {
            diff.changed.push(Changed { card, before: prev, changes });
        }
    }
    // In their old order.
    diff.removed = before.into_iter().filter(|c| old.contains_key(&c.id)).collect();
    diff
}

/// The cards in `files`, which are paths relative to the collection root and their text,
/// read as the app reads a collection: decks are the Markdown files under `cards`, note
/// types the TOML files in `note-types`, and a duplicate card is left out.
pub fn cards(files: &[(String, String)]) -> Vec<Card> {
    let mut note_types = NoteTypes::default();
    let mut decks = Vec::new();
    for (path, text) in files {
        if let Some(name) = note_type_name(path) {
            note_types.insert(name, text);
        } else if let Some(deck) = deck_name(path) {
            decks.push((deck, path, text));
        }
    }
    // By deck name, so a deck's own cards come before its subdecks', as in the app.
    decks.sort_by(|a, b| Path::new(a.0).cmp(Path::new(b.0)));

    let mut seen = std::collections::HashSet::new();
    let mut cards = Vec::new();
    for (deck, path, text) in decks {
        for parsed in parser::parse_deck(text, &note_types, path).cards {
            if seen.insert(parsed.id.clone()) {
                cards.push(card(parsed, deck, path));
            }
        }
    }
    cards
}

fn card(parsed: ParsedCard, deck: &str, path: &str) -> Card {
    Card {
        id: parsed.id,
        deck: deck.to_string(),
        path: path.to_string(),
        line: parsed.line,
        kind: parsed.kind,
        front: parsed.front,
        back: parsed.back,
        tags: parsed.tags,
    }
}

/// `french/verbs` for `cards/french/verbs.md`; `None` for a file that isn't a deck.
fn deck_name(path: &str) -> Option<&str> {
    let deck = path.strip_prefix(&format!("{CARDS_DIR}/"))?.strip_suffix(".md")?;
    let hidden = path.split('/').any(|part| part.starts_with('.'));
    (!hidden && !deck.is_empty()).then_some(deck)
}

/// `Vocab` for `note-types/Vocab.toml`.
fn note_type_name(path: &str) -> Option<&str> {
    let name = path.strip_prefix(&format!("{NOTE_TYPES_DIR}/"))?.strip_suffix(".toml")?;
    (!name.is_empty() && !name.contains('/') && !name.starts_with('.')).then_some(name)
}

/// The deck and note type files of the collection at `root`, as `source` has them.
pub fn files(root: &Path, source: &Source) -> Result<Vec<(String, String)>> {
    match source {
        Source::Disk => disk_files(root),
        Source::Staged => {
            // `<mode> <object> <stage>\t<path>`; a file with conflicts has no stage 0.
            let listing = git(root, &["ls-files", "-z", "--stage", "--", CARDS_DIR, NOTE_TYPES_DIR], None)?;
            let entries = records(&listing)
                .filter_map(|(meta, path)| {
                    let mut meta = meta.split(' ').skip(1);
                    let (object, stage) = (meta.next()?, meta.next()?);
                    (stage == "0").then(|| (path.to_string(), object.to_string()))
                })
                .collect();
            read_objects(root, entries)
        }
        // An empty repository's `HEAD` names no commit yet, so has no cards.
        Source::Rev(rev) if rev == "HEAD" && git(root, &["rev-parse", "--verify", "-q", "HEAD"], None).is_err() => {
            Ok(Vec::new())
        }
        Source::Rev(rev) => {
            // `<mode> <type> <object>\t<path>`, paths relative to `root`.
            let listing = git(root, &["ls-tree", "-r", "-z", rev, "--", CARDS_DIR, NOTE_TYPES_DIR], None)?;
            let entries = records(&listing)
                .filter_map(|(meta, path)| {
                    let mut meta = meta.split(' ').skip(1);
                    let (kind, object) = (meta.next()?, meta.next()?);
                    (kind == "blob").then(|| (path.to_string(), object.to_string()))
                })
                .collect();
            read_objects(root, entries)
        }
    }
}

/// `ls-files -z` and `ls-tree -z` records: metadata, a tab, then the path.
fn records(listing: &str) -> impl Iterator<Item = (&str, &str)> {
    listing.split('\0').filter_map(|record| record.split_once('\t'))
}

/// The files on disk that git would commit: tracked or new, but not ignored. A tracked
/// file deleted from disk is gone.
fn disk_files(root: &Path) -> Result<Vec<(String, String)>> {
    let args = ["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", CARDS_DIR, NOTE_TYPES_DIR];
    let listing = git(root, &args, None)?;
    let mut files = Vec::new();
    // A tracked file with unstaged changes is listed once; with conflicts, once per stage.
    let mut seen = std::collections::HashSet::new();
    for path in listing.split('\0').filter(|p| !p.is_empty()) {
        if !(deck_name(path).is_some() || note_type_name(path).is_some()) || !seen.insert(path) {
            continue;
        }
        match fs::read_to_string(root.join(path)) {
            Ok(text) => files.push((path.to_string(), text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{path}: {e}").into()),
        }
    }
    Ok(files)
}

/// Each `(path, object)`'s contents, read with one `git cat-file`. Files that aren't
/// decks or note types are skipped.
fn read_objects(root: &Path, entries: Vec<(String, String)>) -> Result<Vec<(String, String)>> {
    let entries: Vec<_> = entries
        .into_iter()
        .filter(|(path, _)| deck_name(path).is_some() || note_type_name(path).is_some())
        .collect();
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let input: String = entries.iter().map(|(_, object)| format!("{object}\n")).collect();
    let out = git_bytes(root, &["cat-file", "--batch"], Some(input))?;
    // Each object: `<object> <type> <size>\n`, its contents, then `\n`.
    let mut files = Vec::new();
    let mut rest = out.as_slice();
    for (path, _) in entries {
        let end = rest.iter().position(|&b| b == b'\n').ok_or("git cat-file stopped early")?;
        let header = std::str::from_utf8(&rest[..end])?;
        let size: usize = header.rsplit(' ').next().and_then(|s| s.parse().ok()).ok_or_else(|| format!("git cat-file: {header}"))?;
        let body = rest.get(end + 1..end + 1 + size).ok_or("git cat-file stopped early")?;
        files.push((path, String::from_utf8_lossy(body).into_owned()));
        rest = rest.get(end + 2 + size..).unwrap_or_default();
    }
    Ok(files)
}

fn git(root: &Path, args: &[&str], input: Option<String>) -> Result<String> {
    Ok(String::from_utf8(git_bytes(root, args, input)?)?)
}

/// Run git in `root`, failing with its message.
fn git_bytes(root: &Path, args: &[&str], input: Option<String>) -> Result<Vec<u8>> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't run git: {e}"))?;
    // Written from another thread, so a full stdout can't stall it.
    let writer = input.map(|input| {
        let mut stdin = child.stdin.take().expect("piped");
        std::thread::spawn(move || stdin.write_all(input.as_bytes()))
    });
    let mut stdout = Vec::new();
    child.stdout.take().expect("piped").read_to_end(&mut stdout)?;
    let output = child.wait_with_output()?;
    if let Some(writer) = writer {
        writer.join().map_err(|_| "git input thread panicked")??;
    }
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        let message = message.trim().trim_start_matches("fatal: ");
        return Err(format!("git {}: {message}", args[0]).into());
    }
    Ok(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(p, t)| (p.to_string(), t.to_string())).collect()
    }

    fn fronts(cards: &[Card]) -> Vec<&str> {
        cards.iter().map(|c| c.front.as_str()).collect()
    }

    #[test]
    fn reads_decks_and_note_types_only() {
        let cards = cards(&files(&[
            ("cards/b.md", "Q: b\nA: 1\n"),
            ("cards/a/sub.md", "Q: sub\nA: 1\n"),
            ("cards/a.md", "Q: a\nA: 1\n\nQ: b\nA: duplicate\n"),
            ("cards/.hidden/c.md", "Q: hidden\nA: 1\n"),
            ("notes.md", "Q: outside\nA: 1\n"),
            ("cards/readme.txt", "Q: text\nA: 1\n"),
        ]));
        assert_eq!(fronts(&cards), ["a", "b", "sub"]);
        assert_eq!(cards[1].path, "cards/a.md");
        assert_eq!(cards[2].deck, "a/sub");
    }

    #[test]
    fn compares_by_id() {
        let before = cards(&files(&[
            ("cards/a.md", "Q: kept\nA: 1\n\nQ: fixed\nA: wrong\n\nQ: reworded\nA: 1\n\nQ: gone\nA: 1\n"),
            ("cards/b.md", "Q: moved\nA: 1\n"),
        ]));
        let after = cards(&files(&[
            ("cards/a.md", "Q: new\nA: 1\n\nQ: fixed\nA: right\n#tagged\n\nQ: kept\nA: 1\n\nQ: reworded!\nA: 1\n"),
            ("cards/c.md", "Q: moved\nA: 1\n"),
        ]));
        let diff = compare(before, after);
        assert_eq!(fronts(&diff.added), ["new", "reworded!"]);
        assert_eq!(fronts(&diff.removed), ["reworded", "gone"]);
        let changed: Vec<_> = diff.changed.iter().map(|c| (c.card.front.as_str(), c.changes.clone())).collect();
        assert_eq!(changed, [("fixed", vec!["answer", "tags"]), ("moved", vec!["deck"])]);
        assert_eq!(diff.changed[1].before.path, "cards/b.md");
    }

    #[test]
    fn same_cards_make_no_diff() {
        let deck = files(&[("cards/a.md", "Q: one\nA: 1\n\nC: [two] and [three]\n")]);
        assert_eq!(cards(&deck).len(), 3);
        assert!(compare(cards(&deck), cards(&deck)).is_empty());
    }
}
