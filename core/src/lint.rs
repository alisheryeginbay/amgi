//! Rules for how deck files are written, checked by `amgi lint` and set in `amgi.toml`
//! at the collection root:
//!
//! ```toml
//! [lint]
//! media-links = "absolute"  # media links start at the collection root, `@/media/…`
//! ```
//!
//! Without the file, or a rule in it, each rule has its default. Links inside code
//! blocks and code spans are text, so they're left alone.

use std::fs;
use std::path::Path;

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

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Rules {
    pub media_links: MediaLinks,
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
    /// 1-based.
    pub line: u32,
    /// 1-based, in characters.
    pub column: u32,
    /// The rule's name in `amgi.toml`, such as `media-links`.
    pub rule: &'static str,
    pub message: String,
    /// Whether `fix` can fix it.
    pub fixable: bool,
}

/// The problems in the decks of the collection at `root`, by file, line and column.
/// With `fix`, those that are fixable are fixed in the files.
pub fn run(root: &Path, config: &Config, fix: bool) -> std::io::Result<Vec<Problem>> {
    let mut files = Vec::new();
    let cards = root.join(CARDS_DIR);
    if cards.is_dir() {
        crate::collect_markdown(&cards, &mut files)?;
    }
    files.sort();
    let mut problems = Vec::new();
    for file in files {
        let path = crate::relative(root, &file);
        let text = fs::read_to_string(&file)?;
        let (fixed, found) = check(&text, &path, &config.lint);
        if fix && fixed != text {
            fs::write(&file, fixed)?;
        }
        problems.extend(found);
    }
    Ok(problems)
}

/// The problems in the deck file `text` at `path`, and the text with the fixable ones
/// fixed.
fn check(text: &str, path: &str, rules: &Rules) -> (String, Vec<Problem>) {
    let mut problems = Vec::new();
    if rules.media_links == MediaLinks::Any {
        return (text.to_string(), problems);
    }
    let dir = Path::new(path).parent().unwrap_or(Path::new(""));
    let mut out = String::with_capacity(text.len());
    let mut fences = parser::Fences::default();
    for (i, line) in text.split_inclusive('\n').enumerate() {
        let in_code = fences.inside();
        fences.step(line.trim_end());
        if in_code || fences.inside() {
            out.push_str(line);
            continue;
        }
        let mut report = |column: usize, message: String, fixable: bool| {
            problems.push(Problem {
                path: path.to_string(),
                line: i as u32 + 1,
                column: line[..column].chars().count() as u32 + 1,
                rule: "media-links",
                message,
                fixable,
            });
        };
        out.push_str(&absolute_links(line, dir, &mut report));
    }
    (out, problems)
}

/// `line` with each media link that is relative to the folder `dir` written from the
/// collection root instead, and each such link passed to `report` with its byte column.
/// Links that leave the collection are reported but kept. Code spans are skipped.
fn absolute_links(line: &str, dir: &Path, report: &mut dyn FnMut(usize, String, bool)) -> String {
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while let Some(c) = line[i..].chars().next() {
        let rest = &line[i..];
        if c == '`' {
            // A code span ends at the next run of as many backticks.
            let run = rest.len() - rest.trim_start_matches('`').len();
            let ticks = &rest[..run];
            let close = rest[run..].match_indices(ticks).map(|(at, _)| run + at).find(|&at| {
                let after = &rest[at + run..];
                !after.starts_with('`') && !rest[..at].ends_with('`')
            });
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
        let target = media::target(dest, angled);
        if dest.is_empty() || target.starts_with("@/") || media::is_url(&target) {
            out.push_str(&rest[..len]);
        } else if let Some(rooted) = media::resolve(dir, &target) {
            let new = format!("@/{}", media::slashed(&rooted));
            let written = match angled || new.contains([' ', '(', ')', '<', '>']) {
                true => format!("<{new}>"),
                false => new,
            };
            let open = dest.as_ptr() as usize - rest.as_ptr() as usize - usize::from(angled);
            let close = open + dest.len() + 2 * usize::from(angled);
            report(i, format!("write `{dest}` from the collection root: `{written}`"), true);
            out.push_str(&rest[..open]);
            out.push_str(&written);
            out.push_str(&rest[close..len]);
        } else {
            report(i, format!("`{dest}` leads out of the collection, so it doesn't show"), false);
            out.push_str(&rest[..len]);
        }
        i += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lint(text: &str, path: &str) -> (String, Vec<(u32, u32, String, bool)>) {
        let (fixed, problems) = check(text, path, &Rules::default());
        (fixed, problems.into_iter().map(|p| (p.line, p.column, p.message, p.fixable)).collect())
    }

    #[test]
    fn media_links_start_at_the_collection_root() {
        let text = "Q: Which city? ![x](../../media/eiffel.jpg)\nA: Paris ![](<../../media/a b.png>) ![](../../media/c%20d.mp3)\n";
        let (fixed, problems) = lint(text, "cards/places/europe.md");
        assert_eq!(fixed, "Q: Which city? ![x](@/media/eiffel.jpg)\nA: Paris ![](<@/media/a b.png>) ![](<@/media/c d.mp3>)\n");
        assert_eq!(
            problems,
            [
                (1, 16, "write `../../media/eiffel.jpg` from the collection root: `@/media/eiffel.jpg`".into(), true),
                (2, 10, "write `../../media/a b.png` from the collection root: `<@/media/a b.png>`".into(), true),
                (2, 37, "write `../../media/c%20d.mp3` from the collection root: `<@/media/c d.mp3>`".into(), true),
            ]
        );
        // Fixing again finds nothing.
        assert_eq!(lint(&fixed, "cards/places/europe.md"), (fixed.clone(), vec![]));
    }

    #[test]
    fn leaves_alone_what_it_cant_or_shouldnt_fix() {
        let text = "Q: ![](../../x.png) and ![](https://example.com/a.png) and `![](../a.png)`\n\
                    A: ``code ` ![](../b.png)`` ![](../media/c.png)\n\
                    ```md\n![](../media/d.png)\n```\n";
        let (fixed, problems) = lint(text, "cards/a.md");
        assert_eq!(fixed, text.replace("](../media/c.png)", "](@/media/c.png)"));
        assert_eq!(
            problems,
            [
                (1, 4, "`../../x.png` leads out of the collection, so it doesn't show".into(), false),
                (2, 29, "write `../media/c.png` from the collection root: `@/media/c.png`".into(), true),
            ]
        );
    }

    #[test]
    fn rules_come_from_amgi_toml() {
        let config = |text: &str| toml::from_str::<Config>(text).map(|c| c.lint.media_links).map_err(|e| e.to_string());
        assert_eq!(config(""), Ok(MediaLinks::Absolute));
        assert_eq!(config("[lint]\nmedia-links = \"any\""), Ok(MediaLinks::Any));
        assert!(config("[lint]\nmedia-link = \"any\"").unwrap_err().contains("unknown field"));
        assert!(config("[lint]\nmedia-links = \"relative\"").is_err());
        let (text, problems) = check("Q: ![](../media/a.png)\nA: b\n", "cards/a.md", &Rules { media_links: MediaLinks::Any });
        assert_eq!((text.as_str(), problems.len()), ("Q: ![](../media/a.png)\nA: b\n", 0));
    }
}
