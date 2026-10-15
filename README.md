# amgi

Plain-text flash cards, scheduled with [FSRS](https://github.com/open-spaced-repetition/fsrs4anki/wiki/The-Algorithm).

A collection is a folder of Markdown files. Each file is a deck, and folders nest
decks: studying `lang` covers `lang.md` and everything in `lang/`. Review history is
stored in `.amgi.sqlite` at the root of the folder.

## Card format

```markdown
Q: What does **FSRS** stand for?
A: Free Spaced Repetition Scheduler

C: The capital of [France] is [Paris|city].

C: \[H⁺\] is the [concentration] of hydrogen ions; `v[0]` is the [first|which?] element.
```

- `Q:` / `A:`: basic card. Either field can continue onto later lines.
- `C:`: cloze. Each `[answer]` becomes its own card; `[answer|hint]` shows the hint.
  Write `\[` and `\]` for literal brackets. Brackets inside `` `code` `` are always literal.
- Images: `![](images/photo.jpg)`, with the path relative to the deck file. Use
  `![](<my photo (1).jpg>)` for paths with spaces or parentheses. An image can also be a
  cloze answer: `[![](flag.png)|flag]`.
- A card ends at a blank line or the next `Q:` / `C:`. Every other line is ignored.

A card's identity comes from its text (the question for basic cards, the whole text
for clozes). You can move or reorder cards, fix answers and change tags without losing
history. Editing a question starts the card over.

### Tags and deck settings

End a card with a line of tags. A file can start with TOML front matter between `+++`
lines: `name` is the deck's display name and `tags` are added to every card in it.

```markdown
+++
name = "Korean verbs"
tags = ["korean"]
+++

Q: 가다
A: to go
#verbs #motion
```

Tags go on the card's last lines, after its first one, and only a line made entirely
of `#tag` words counts, so a `# Heading` stays text. Pick a tag in the sidebar to study
it across decks.

## Importing from Anki

**File ▸ Import Anki Deck…** (<kbd>⇧⌘I</kbd>) converts an `.apkg` or `.colpkg` into Markdown
files in the open folder, one per deck (`Languages::Korean` becomes `Languages/Korean.md`).
Existing files are never overwritten. Images are copied to a `media` folder at the root,
and note tags become `#tag` lines.

- Basic note types become `Q:`/`A:` cards rendered from their templates. Cloze notes become
  `C:` cards, with `Back Extra` kept as a note under the card. Anki can test several
  deletions together (`c1` used twice); amgi gives each its own card.
- Review history is replayed through amgi's FSRS scheduler, and due dates follow Anki's.
- Suspended cards, sounds and image occlusion notes are left out.

## Layout

- `core/`: Rust library containing the parser, FSRS-6 scheduler, SQLite store and the
  `CardCollection` API, exported through [UniFFI](https://mozilla.github.io/uniffi-rs/)
- `app/`: SwiftUI macOS app (Swift package)
- `examples/`: a sample collection

## Build & run

```sh
scripts/build-core.sh        # build Rust core + generate Swift bindings
cd app && swift run          # or: open app/Package.swift in Xcode
cargo test                   # core tests
```

Re-run `scripts/build-core.sh` after any change to `core/`.

Keys while studying: <kbd>Space</kbd> shows the answer, <kbd>1</kbd>–<kbd>4</kbd> rate
Again/Hard/Good/Easy, <kbd>⌘O</kbd> opens a folder, <kbd>⌘R</kbd> reloads. Files also reload
whenever the app comes to the front.
