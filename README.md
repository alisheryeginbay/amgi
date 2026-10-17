# amgi

Plain-text flash cards, scheduled with [FSRS](https://github.com/open-spaced-repetition/fsrs4anki/wiki/The-Algorithm).

A collection is a folder of Markdown files. Each file is a deck, and folders nest
decks: the `lang` deck holds `lang.md` and everything in `lang/`. Review history is
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
- Sounds use the same syntax: `![](audio/word.mp3)`. Links to audio files (`mp3`, `m4a`,
  `wav`, `flac` and other formats macOS can play) show as play buttons. A card's sounds
  play when it appears, and the answer's when it is shown; press <kbd>R</kbd> to replay.
  Turn autoplay off in **Settings** (<kbd>⌘,</kbd>).
- Math: `$…$` inline and `$$…$$` for display, in TeX, rendered with
  [KaTeX](https://katex.org). Brackets and `|` inside math are literal, so they never
  start a cloze. A `$` only starts math before a non-space (`$5 and $10` stays text);
  write `\$` for a literal one.
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
of `#tag` words counts, so a `# Heading` stays text. Pick a tag in the sidebar to browse
its cards across decks. Right-click a tag and choose **Choose Icon…** to give it an emoji;
icons are kept in `.amgi.sqlite`.

### Suspending cards

A card tagged `#suspended` never comes up for study, and isn't counted as due or new;
`tags = ["suspended"]` in a deck's front matter suspends the whole deck. **Suspend Card**
in the toolbar while studying, or right-clicking cards while browsing, adds or removes the
tag in the file for you. Tags aren't part of a card's identity, so it keeps its history.
Pick the `suspended` tag in the sidebar to see them all.

## Importing from Anki

**File ▸ Import Anki Deck…** (<kbd>⇧⌘I</kbd>) converts an `.apkg` or `.colpkg` into Markdown
files in the open folder, one per deck (`Languages::Korean` becomes `Languages/Korean.md`).
Existing files are never overwritten. Images and sounds are copied to a `media` folder at
the root, and note tags become `#tag` lines.

- Basic note types become `Q:`/`A:` cards rendered from their templates. Cloze notes become
  `C:` cards, with `Back Extra` kept as a note under the card. Anki can test several
  deletions together (`c1` used twice); amgi gives each its own card.
- Review history is replayed through amgi's FSRS scheduler, and due dates follow Anki's.
- MathJax `\(…\)` and `\[…\]` become `$…$` and `$$…$$`. A cloze inside math is
  taken out of it, so the gap holds its own math.
- `[sound:…]` references become sound links. Text-to-speech (`{{tts:…}}`) is left out.
- Suspended cards are imported with the `#suspended` tag and keep their history. amgi
  suspends a cloze as a whole, so when only some of a cloze's deletions were suspended in
  Anki, those get a tagged copy of the cloze of their own.
- Image occlusion notes are left out.

## Layout

- `core/`: Rust library containing the parser, FSRS-6 scheduler, SQLite store and the
  `CardCollection` API, exported through [UniFFI](https://mozilla.github.io/uniffi-rs/)
- `app/`: SwiftUI macOS app (Swift package): the window, sidebar, toolbar and settings
- `web/`: the study, dashboard and browse panes in [Svelte](https://svelte.dev), shown in
  one web view. The app sends each pane its data and the page sends back what you do;
  cards' images and sounds are served from the collection at `amgi://collection/`.
- `examples/`: a sample collection

## Build & run

```sh
scripts/build-core.sh        # build Rust core + generate Swift bindings
scripts/build-web.sh         # build the Svelte panes into the app (needs Node)
cd app && swift run          # or: open app/Package.swift in Xcode
cargo test                   # core tests
```

Re-run `scripts/build-core.sh` after any change to `core/` and `scripts/build-web.sh` after
any change to `web/`. To work on the panes with live reload, run `npm run dev` in `web/`
and start the app with `AMGI_WEB_URL=http://localhost:5173 swift run`. Debug builds can be
inspected with Safari's Web Inspector (Develop ▸ your Mac ▸ Amgi).

Keys while studying: <kbd>Space</kbd> shows the answer, <kbd>1</kbd>–<kbd>4</kbd> rate
Again/Hard/Good/Easy, <kbd>R</kbd> replays the card's sounds, <kbd>⌘O</kbd> opens a folder, <kbd>⌘R</kbd> reloads. Files also reload
whenever the app comes to the front.

**Review** in the sidebar studies the whole collection. Pick a deck or tag to browse its
cards in a table: search, sort by any column, and double-click a card to open its file.

**Saved Views ▸ Leeches** lists cards you have forgotten 8 or more times, across all decks:
they usually need rewording or splitting.
