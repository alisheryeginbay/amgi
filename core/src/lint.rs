//! Rules for how deck files are written, checked by `amgi lint` and set in `amgi.toml`
//! at the collection root:
//!
//! ```toml
//! [lint]
//! media-links = "absolute"  # media links start at the collection root, `@/media/…`
//! missing-media = true      # links to files that don't exist
//! ```
//!
//! Without the file, or a rule in it, each rule has its default. Links inside code
//! blocks and code spans are text, so they're left alone. A fix never changes a card's
//! id, which would lose its history: a file whose ids would change isn't written.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{media, parser, CARDS_DIR};

/// The collection's settings file, at its root.
pub const CONFIG_FILE: &str = "amgi.toml";

/// What `amgi.toml` says.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub lint: Rules,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Rules {
    pub media_links: MediaLinks,
    /// Report links to files that don't exist.
    pub missing_media: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Rules { media_links: MediaLinks::default(), missing_media: true }
    }
}

/// How media links are written.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaLinks {
    /// From the collection root: `@/media/photo.jpg`. Such a link keeps working when
    /// its deck moves.
    #[default]
    Absolute,
    /// Either way.
    Any,
}

impl Config {
    /// `amgi.toml` at `root`, or the defaults when there is none; or why it can't be read.
    pub fn load(root: &Path) -> Result<Self, String> {
        match fs::read_to_string(root.join(CONFIG_FILE)) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("{CONFIG_FILE}: {}", e.to_string().trim_end())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{CONFIG_FILE}: {e}")),
        }
    }
}

/// Something in a deck file that breaks a rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    /// Path from the collection root.
    pub path: String,
    /// 1-based; 0 for the whole file.
    pub line: u32,
    /// 1-based, in characters; 0 for the whole file.
    pub column: u32,
    /// The rule's name in `amgi.toml`, such as `media-links`.
    pub rule: &'static str,
    pub message: String,
    /// Whether `run` with `fix` fixes it.
    pub fixable: bool,
}

/// The files in the collection, to tell whether a link's file exists and suggest
/// another with its name.
struct Files {
    root: PathBuf,
    /// Paths from the root, by lowercased file stem.
    by_stem: HashMap<String, Vec<String>>,
}

impl Files {
    fn read(root: &Path) -> Self {
        let mut by_stem: HashMap<String, Vec<String>> = HashMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                if path.is_dir() {
                    stack.push(path);
                } else if let Some(stem) = path.file_stem() {
                    let rel = media::slashed(path.strip_prefix(root).unwrap_or(&path));
                    by_stem.entry(stem.to_string_lossy().to_lowercase()).or_default().push(rel);
                }
            }
        }
        by_stem.values_mut().for_each(|paths| paths.sort());
        Files { root: root.to_path_buf(), by_stem }
    }

    fn exists(&self, path: &Path) -> bool {
        self.root.join(path).is_file()
    }

    /// Files with the same name as `path` but another extension or folder.
    fn like(&self, path: &Path) -> &[String] {
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        self.by_stem.get(&stem).map_or(&[], Vec::as_slice)
    }
}

/// The problems in the decks of the collection at `root`, read with note types
/// `types`, by file, line and column. With `fix`, the fixable ones are fixed in the
/// files, unless that would change a card's id.
pub(crate) fn run(root: &Path, types: &parser::NoteTypes, config: &Config, fix: bool) -> std::io::Result<Vec<Problem>> {
    let mut decks = Vec::new();
    let cards = root.join(CARDS_DIR);
    if cards.is_dir() {
        crate::collect_markdown(&cards, &mut decks)?;
    }
    decks.sort();
    let files = Files::read(root);
    let mut problems = Vec::new();
    for file in decks {
        let path = crate::relative(root, &file);
        let text = fs::read_to_string(&file)?;
        let (fixed, mut found) = check(&text, &path, &config.lint, &files);
        if fix && fixed != text {
            if same_ids(&text, &fixed, types, &path) {
                fs::write(&file, fixed)?;
            } else {
                found.iter_mut().for_each(|p| p.fixable = false);
                found.insert(0, Problem {
                    path: path.clone(),
                    line: 0,
                    column: 0,
                    rule: "fix",
                    message: "not fixed, because fixing would change card ids and lose their review history".into(),
                    fixable: false,
                });
            }
        }
        problems.extend(found);
    }
    Ok(problems)
}

/// Whether the deck file at `path` makes cards with the same ids, in the same order,
/// as `before` and as `after`.
fn same_ids(before: &str, after: &str, types: &parser::NoteTypes, path: &str) -> bool {
    let ids = |text: &str| parser::parse_deck(text, types, path).cards.into_iter().map(|c| c.id).collect::<Vec<_>>();
    ids(before) == ids(after)
}

/// The problems in the deck file `text` at `path`, and the text with the fixable ones
/// fixed.
fn check(text: &str, path: &str, rules: &Rules, files: &Files) -> (String, Vec<Problem>) {
    let dir = Path::new(path).parent().unwrap_or(Path::new(""));
    let mut problems = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut fences = parser::Fences::default();
    for (i, line) in text.split_inclusive('\n').enumerate() {
        let in_code = fences.inside();
        fences.step(line.trim_end());
        if in_code || fences.inside() {
            out.push_str(line);
            continue;
        }
        let mut report = |column: usize, rule, message, fixable| {
            problems.push(Problem {
                path: path.to_string(),
                line: i as u32 + 1,
                column: line[..column].chars().count() as u32 + 1,
                rule,
                message,
                fixable,
            });
        };
        let fixed = each_link(line, |column, dest, angled| {
            let target = media::target(dest, angled);
            if dest.is_empty() || media::is_url(&target) {
                return None;
            }
            let Some(rooted) = media::resolve(dir, &target) else {
                if rules.missing_media {
                    let message = format!("`{dest}` leads out of the collection, so it doesn't show");
                    report(column, "missing-media", message, false);
                }
                return None;
            };
            if rules.missing_media && !files.exists(&rooted) {
                let message = match files.like(&rooted) {
                    [] => format!("`{dest}` doesn't exist"),
                    [one] => format!("`{dest}` doesn't exist; did you mean `@/{one}`?"),
                    more => format!("`{dest}` doesn't exist; {} files have its name, such as `@/{}`", more.len(), more[0]),
                };
                report(column, "missing-media", message, false);
            }
            if rules.media_links != MediaLinks::Absolute || target.starts_with("@/") {
                return None;
            }
            let new = format!("@/{}", media::slashed(&rooted));
            let written = match angled || new.contains([' ', '(', ')', '<', '>']) {
                true => format!("<{new}>"),
                false => new,
            };
            report(column, "media-links", format!("write `{dest}` from the collection root: `{written}`"), true);
            Some(written)
        });
        out.push_str(&fixed);
    }
    (out, problems)
}

/// `line` with each media link's destination passed to `visit`, with its byte column
/// and whether it's in angle brackets, and replaced by what `visit` returns, angle
/// brackets included. Code spans are skipped.
fn each_link(line: &str, mut visit: impl FnMut(usize, &str, bool) -> Option<String>) -> String {
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while let Some(c) = line[i..].chars().next() {
        let rest = &line[i..];
        if c == '`' {
            // A code span ends at the next run of as many backticks.
            let run = rest.len() - rest.trim_start_matches('`').len();
            let ticks = &rest[..run];
            let close = rest[run..]
                .match_indices(ticks)
                .map(|(at, _)| run + at)
                .find(|&at| !rest[at + run..].starts_with('`') && !rest[..at].ends_with('`'));
            let len = close.map_or(run, |at| at + run);
            out.push_str(&rest[..len]);
            i += len;
            continue;
        }
        let Some((len, dest, angled)) = rest.starts_with("![").then(|| parser::image(rest)).flatten() else {
            out.push(c);
            i += c.len_utf8();
            continue;
        };
        match visit(i, dest, angled) {
            Some(written) => {
                let open = dest.as_ptr() as usize - rest.as_ptr() as usize - usize::from(angled);
                let close = open + dest.len() + 2 * usize::from(angled);
                out.push_str(&rest[..open]);
                out.push_str(&written);
                out.push_str(&rest[close..len]);
            }
            None => out.push_str(&rest[..len]),
        }
        i += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A collection holding `files`, removed when dropped.
    struct Collection(PathBuf);

    impl Collection {
        fn new(files: &[(&str, &str)]) -> Self {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("amgi-lint-{}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            for (path, text) in files {
                let path = dir.join(path);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, text).unwrap();
            }
            Collection(dir)
        }
    }

    impl Drop for Collection {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const MEDIA: &[(&str, &str)] = &[
        ("media/eiffel.jpg", ""),
        ("media/a b.png", ""),
        ("media/c d.mp3", ""),
        ("media/c.png", ""),
        ("media/tower.jpeg", ""),
        ("media/old/bridge.png", ""),
    ];

    /// Line, column, rule, message and whether it's fixable.
    type Found = Vec<(u32, u32, &'static str, String, bool)>;

    fn lint_with(text: &str, path: &str, rules: &Rules) -> (String, Found) {
        let collection = Collection::new(MEDIA);
        let (fixed, problems) = check(text, path, rules, &Files::read(&collection.0));
        (fixed, problems.into_iter().map(|p| (p.line, p.column, p.rule, p.message, p.fixable)).collect())
    }

    fn lint(text: &str, path: &str) -> (String, Found) {
        lint_with(text, path, &Rules::default())
    }

    #[test]
    fn media_links_start_at_the_collection_root() {
        let text = "Q: Which city? ![x](../../media/eiffel.jpg)\nA: Paris ![](<../../media/a b.png>) ![](../../media/c%20d.mp3)\n";
        let (fixed, problems) = lint(text, "cards/places/europe.md");
        assert_eq!(fixed, "Q: Which city? ![x](@/media/eiffel.jpg)\nA: Paris ![](<@/media/a b.png>) ![](<@/media/c d.mp3>)\n");
        let rule = "media-links";
        assert_eq!(
            problems,
            [
                (1, 16, rule, "write `../../media/eiffel.jpg` from the collection root: `@/media/eiffel.jpg`".into(), true),
                (2, 10, rule, "write `../../media/a b.png` from the collection root: `<@/media/a b.png>`".into(), true),
                (2, 37, rule, "write `../../media/c%20d.mp3` from the collection root: `<@/media/c d.mp3>`".into(), true),
            ]
        );
        // Fixing again finds nothing.
        assert_eq!(lint(&fixed, "cards/places/europe.md"), (fixed.clone(), vec![]));
    }

    #[test]
    fn leaves_alone_what_it_cant_or_shouldnt_fix() {
        let text = "Q: ![](../../x.png) and ![](https://example.com/a.png) and `![](../a.png)`\n\
                    A: ``code ` ![](../b.png)`` ![](../media/c.png) ![](@/../y.png)\n\
                    ```md\n![](../media/d.png)\n```\n";
        let (fixed, problems) = lint(text, "cards/a.md");
        assert_eq!(fixed, text.replace("](../media/c.png)", "](@/media/c.png)"));
        let (out, links) = ("missing-media", "media-links");
        assert_eq!(
            problems,
            [
                (1, 4, out, "`../../x.png` leads out of the collection, so it doesn't show".into(), false),
                (2, 29, links, "write `../media/c.png` from the collection root: `@/media/c.png`".into(), true),
                (2, 49, out, "`@/../y.png` leads out of the collection, so it doesn't show".into(), false),
            ]
        );
    }

    #[test]
    fn missing_media_suggests_a_file_with_its_name() {
        let text = "Q: ![](@/media/tower.jpg) ![](@/media/bridge.png) ![](@/media/gone.png)\nA: ![](@/media/eiffel.jpg)\n";
        let (fixed, problems) = lint(text, "cards/a.md");
        assert_eq!(fixed, text);
        let rule = "missing-media";
        assert_eq!(
            problems,
            [
                (1, 4, rule, "`@/media/tower.jpg` doesn't exist; did you mean `@/media/tower.jpeg`?".into(), false),
                (1, 27, rule, "`@/media/bridge.png` doesn't exist; did you mean `@/media/old/bridge.png`?".into(), false),
                (1, 51, rule, "`@/media/gone.png` doesn't exist".into(), false),
            ]
        );
        // A relative link to a missing file is both.
        let (_, problems) = lint("Q: ![](../media/gone.png)\nA: x\n", "cards/a.md");
        assert_eq!(problems.iter().map(|p| p.2).collect::<Vec<_>>(), ["missing-media", "media-links"]);
    }

    #[test]
    fn rules_come_from_amgi_toml() {
        let config = |text: &str| toml::from_str::<Config>(text).map(|c| (c.lint.media_links, c.lint.missing_media)).map_err(|e| e.to_string());
        assert_eq!(config(""), Ok((MediaLinks::Absolute, true)));
        assert_eq!(config("[lint]\nmedia-links = \"any\"\nmissing-media = false"), Ok((MediaLinks::Any, false)));
        assert!(config("[lint]\nmedia-link = \"any\"").unwrap_err().contains("unknown field"));
        assert!(config("[lint]\nmedia-links = \"relative\"").is_err());
        let off = Rules { media_links: MediaLinks::Any, missing_media: false };
        let text = "Q: ![](../media/gone.png)\nA: b\n";
        assert_eq!(lint_with(text, "cards/a.md", &off), (text.to_string(), vec![]));
    }

    #[test]
    fn fix_writes_the_files() {
        let collection = Collection::new(&[("media/c.png", ""), ("cards/a.md", "Q: ![](../media/c.png)\nA: x\n")]);
        let problems = run(&collection.0, &parser::NoteTypes::default(), &Config::default(), true).unwrap();
        assert_eq!((problems.len(), problems[0].fixable), (1, true));
        assert_eq!(fs::read_to_string(collection.0.join("cards/a.md")).unwrap(), "Q: ![](@/media/c.png)\nA: x\n");
    }

    #[test]
    fn fixes_must_keep_ids() {
        let types = parser::NoteTypes::default();
        let same = |before: &str, after: &str| same_ids(before, after, &types, "cards/a/b.md");
        assert!(same("Q: ![](../../media/c.png) x\nA: y\n", "Q: ![](@/media/c.png)   x\nA: changed\n"));
        assert!(!same("Q: x\nA: y\n", "Q: x edited\nA: y\n"));
        assert!(!same("Q: ![](../../media/c.png)\nA: y\n", "Q: ![](@/media/other.png)\nA: y\n"));
    }
}
