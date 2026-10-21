//! Files in the `media` folder that no deck links to.
//!
//! Links are read from the text of every Markdown file in the collection, not only
//! the decks that load, so a file linked from a commented-out card, a code block or a
//! deck outside `cards` is kept. Paths are compared ignoring case and Unicode
//! normalization, as macOS finds files.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

use crate::{parser, MEDIA_DIR};

/// A media file no deck links to.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UnusedMedia {
    /// Path from the collection root, such as `media/photo.jpg`.
    pub path: String,
    pub bytes: u64,
}

/// Files under `root/media` that no Markdown file in `root` links to, sorted by path.
/// Hidden files are left out.
pub fn unused(root: &Path) -> std::io::Result<Vec<UnusedMedia>> {
    let mut used = HashSet::new();
    let mut decks = Vec::new();
    collect(root, root, &mut decks, &|p| p.extension().is_some_and(|e| e == "md"))?;
    for deck in decks {
        let text = fs::read_to_string(root.join(&deck))?;
        let dir = deck.parent().unwrap_or(Path::new(""));
        used.extend(links(&text).filter_map(|link| resolve(dir, &link)).map(|p| key(&p)));
    }

    let media = root.join(MEDIA_DIR);
    let mut files = Vec::new();
    if media.is_dir() {
        collect(root, &media, &mut files, &|_| true)?;
    }
    let mut out = Vec::new();
    for file in files {
        if !used.contains(&key(&file)) {
            let bytes = fs::metadata(root.join(&file))?.len();
            out.push(UnusedMedia { path: slashed(&file), bytes });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Files under `dir` passing `keep`, relative to `root`, leaving out hidden ones.
fn collect(root: &Path, dir: &Path, out: &mut Vec<PathBuf>, keep: &dyn Fn(&Path) -> bool) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect(root, &path, out, keep)?;
        } else if keep(&path) {
            out.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
        }
    }
    Ok(())
}

/// The destination of each image or sound `![alt](path)` in `text`, as the file names
/// it: `<path>` literally, a bare path percent-decoded.
fn links(text: &str) -> impl Iterator<Item = String> + '_ {
    text.match_indices("![").filter_map(|(i, _)| {
        let (_, path, angled) = parser::image(&text[i..])?;
        Some(match angled {
            true => path.to_string(),
            false => percent_decode(path).unwrap_or_else(|| path.to_string()),
        })
    })
}

/// `link`, relative to the folder `dir`, as a path from the collection root; `None` for
/// URLs and paths that leave the collection.
fn resolve(dir: &Path, link: &str) -> Option<PathBuf> {
    let scheme = link.split_once(':').is_some_and(|(s, _)| {
        s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && s.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
    });
    if scheme {
        return None;
    }
    let mut out = PathBuf::new();
    for part in dir.join(link).components() {
        match part {
            Component::Normal(p) => out.push(p),
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

/// `%20` and the like decoded, or `None` if that isn't valid UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1).and_then(|&b| hex(b)), bytes.get(i + 2).and_then(|&b| hex(b))) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (b, ..) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn slashed(path: &Path) -> String {
    path.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// A path as the file system tells files apart.
fn key(path: &Path) -> String {
    slashed(path).nfc().collect::<String>().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collection(files: &[(&str, &str)]) -> PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("amgi-media-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (path, text) in files {
            let path = dir.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        dir
    }

    fn unused_paths(root: &Path) -> Vec<String> {
        unused(root).unwrap().into_iter().map(|m| m.path).collect()
    }

    #[test]
    fn finds_files_no_deck_links_to() {
        let root = collection(&[
            ("cards/planets.md", "Q: ![](../media/mars.png)\nA: Mars\n\nQ: ![](<../media/red planet (1).jpg>)\nA: Mars\n"),
            ("cards/sub/sounds.md", "Q: ![](../../media/audio/hello%20world.mp3)\nA: hi\n"),
            ("cards/old.md", "<!-- Q: ![](../media/Comet.PNG) -->\n"),
            ("stray.md", "![a [b]](media/kept.gif)\n"),
            ("media/mars.png", "x"),
            ("media/red planet (1).jpg", "x"),
            ("media/audio/hello world.mp3", "x"),
            ("media/comet.png", "x"),
            ("media/kept.gif", "x"),
            ("media/unused.png", "12345"),
            ("media/audio/unused.mp3", "x"),
            ("media/.DS_Store", "x"),
        ]);
        assert_eq!(unused_paths(&root), ["media/audio/unused.mp3", "media/unused.png"]);
        assert_eq!(unused(&root).unwrap()[1].bytes, 5);
    }

    #[test]
    fn links_resolve_from_the_deck_folder() {
        let root = collection(&[
            // From `cards`, `media/x.png` is `cards/media/x.png`, not the media folder.
            ("cards/a.md", "Q: ![](media/x.png)\nA: ![](https://example.com/y.png)\n"),
            ("cards/b.md", "Q: ![](../../outside.png)\nA: ![](../media/caf%C3%A9.png)\n"),
            ("media/x.png", "x"),
            ("media/y.png", "x"),
            ("media/cafe\u{301}.png", "x"),
        ]);
        assert_eq!(unused_paths(&root), ["media/x.png", "media/y.png"]);
    }

    #[test]
    fn no_media_folder() {
        let root = collection(&[("cards/a.md", "Q: hi\nA: there\n")]);
        assert!(unused_paths(&root).is_empty());
    }
}
