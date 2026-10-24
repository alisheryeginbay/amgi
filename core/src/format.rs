//! `amgi fmt`: deck files in one layout. Lines lose trailing spaces and Windows line
//! endings, `Q:` and `C:` cards are a single blank line apart, a file ends in one newline and a card's
//! tags sit on their lines a space apart. Code blocks keep their blank lines. The cards a
//! file makes stay exactly the same, ids included, so no history is lost.

use std::fs;
use std::path::Path;

use crate::{parser, CARDS_DIR};

/// What `run` found.
#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Formatted {
    /// Deck files not in the layout, as paths from the collection root.
    pub changed: Vec<String>,
    /// Those of them left alone because formatting would change their cards.
    pub refused: Vec<String>,
}

/// The decks of the collection at `root` that aren't in the layout, read with note
/// types `types`. With `write`, they're rewritten, unless that would change a card.
pub(crate) fn run(root: &Path, types: &parser::NoteTypes, write: bool) -> std::io::Result<Formatted> {
    let mut decks = Vec::new();
    let cards = root.join(CARDS_DIR);
    if cards.is_dir() {
        crate::collect_markdown(&cards, &mut decks)?;
    }
    decks.sort();
    let mut out = Formatted::default();
    for file in decks {
        let path = crate::relative(root, &file);
        let text = fs::read_to_string(&file)?;
        let formatted = format(&text);
        if formatted == text {
            continue;
        }
        if !same_cards(&text, &formatted, types, &path) {
            out.refused.push(path.clone());
        } else if write {
            fs::write(&file, &formatted)?;
        }
        out.changed.push(path);
    }
    Ok(out)
}

/// Whether the deck file at `path` makes the same cards from `before` as from `after`:
/// the same ids, sides and tags, in the same order.
fn same_cards(before: &str, after: &str, types: &parser::NoteTypes, path: &str) -> bool {
    let cards = |text: &str| {
        let parsed = parser::parse_deck(text, types, path);
        parsed.cards.into_iter().map(|c| (c.id, c.front, c.back, c.tags)).collect::<Vec<_>>()
    };
    cards(before) == cards(after)
}

/// `text` in the standard layout.
pub(crate) fn format(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let mut out = String::with_capacity(text.len());
    let mut fences = parser::Fences::default();
    // Blank lines waiting for a line that isn't blank, outside code.
    let mut blank = false;
    for (i, line) in lines.iter().enumerate() {
        let in_code = fences.inside();
        fences.step(line);
        if in_code {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if line.is_empty() {
            blank = !out.is_empty();
            continue;
        }
        // A card starts after a blank line, as does the next one.
        let starts_card = line.starts_with("Q:") || line.starts_with("C:");
        if blank || (starts_card && !out.is_empty()) {
            out.push('\n');
            blank = false;
        }
        match ends_card_tags(&lines, i) {
            Some(tags) => out.push_str(&tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" ")),
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

/// The tags on line `i` of `lines` if it's one of the tag lines that end a card: lines
/// of only `#tag` words, followed by a blank line, the end of the file or the next card.
pub(crate) fn ends_card_tags(lines: &[&str], i: usize) -> Option<Vec<String>> {
    let tags = parser::tag_line(lines[i])?;
    let next = lines[i + 1..].iter().find(|l| parser::tag_line(l).is_none());
    let ends = match next {
        None => true,
        Some(l) => {
            let l = l.trim();
            l.is_empty() || l.starts_with("Q:") || l.starts_with("C:") || l.starts_with('@')
        }
    };
    ends.then_some(tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lays_out_deck_files() {
        let text = "\n\r\n+++\r\nname = \"Places\"  \r\n+++\r\nQ: Capital of France?   \nA: Paris\n  #geo   #europe\n\n\n\nQ: x\nA: y\n#a\n#b\t#c\nC: [z] w\n\n\n";
        assert_eq!(
            format(text),
            "+++\nname = \"Places\"\n+++\n\nQ: Capital of France?\nA: Paris\n#geo #europe\n\nQ: x\nA: y\n#a\n#b #c\n\nC: [z] w\n"
        );
    }

    #[test]
    fn formatting_keeps_cards() {
        let types = parser::NoteTypes::default();
        let text = "Q: a  \nA: b\n  #x   #y\n\n\nC: [c] d\n";
        assert!(same_cards(text, &format(text), &types, "cards/a.md"));
        assert!(!same_cards("Q: a\nA: b\n", "Q: a\nA: c\n", &types, "cards/a.md"));
    }

    #[test]
    fn keeps_code_and_text_that_isnt_tags() {
        let text = "Q: Show it\nA:\n```md\nline  \n\n\n#not tags\n```\n#tag\n\nQ: Heading?\nA: # Title\n#one  #two\nmore text\n";
        // In code, only trailing spaces go; `#one  #two` is followed by text, so it
        // isn't a tag line.
        assert_eq!(format(text), text.replace("line  \n", "line\n"));
        assert_eq!(format(""), "");
        assert_eq!(format("\n\n"), "");
    }
}
