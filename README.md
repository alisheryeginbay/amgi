# amgi

Plain-text flash cards, scheduled with [FSRS](https://github.com/open-spaced-repetition/fsrs4anki/wiki/The-Algorithm).

A collection is a folder of Markdown files. Each file is a deck. Review history is
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
- A card ends at a blank line or the next `Q:` / `C:`. Every other line is ignored.

A card's identity comes from its text (the question for basic cards, the whole text
for clozes). You can move or reorder cards and fix answers without losing history.
Editing a question starts the card over.

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
