<p align="center">
  <img src="media/logo/amgi-symbol.svg" alt="" width="96" />
</p>
<h1 align="center">Amgi</h1>

<p align="center">
  <b>Flash cards in plain Markdown files, scheduled with <a href="https://github.com/open-spaced-repetition/fsrs4anki/wiki/The-Algorithm">FSRS</a>.</b><br />
  A native macOS app for a folder of cards you can edit in any editor and keep in git.
</p>

<p align="center">
  <a href="https://docs.amgi.study">Docs</a>
  ·
  <a href="https://docs.amgi.study/anki-import">Import from Anki</a>
</p>

<p align="center">
  <img src="media/screenshots/review.webp" alt="Amgi reviewing a card: the deck sidebar on the left, the card in the middle and the Again, Hard, Good and Easy buttons below it" />
</p>

## Cards are text

```markdown
Q: What does **FSRS** stand for?
A: Free Spaced Repetition Scheduler

C: The capital of [France] is [Paris|city].
```

`Q:` / `A:` makes a basic card; in a `C:` cloze each `[answer]` becomes its own card, and
`[answer|hint]` shows the hint. There's no database of cards to export from: the files
are the cards, so you can write them in Vim, grep them, generate them with a script and
review the changes in a pull request.

<p align="center">
  <img src="media/screenshots/editor.webp" alt="A deck file open in Neovim: basic and cloze cards under the deck's front matter, beside a file tree with the cards and note-types folders and .amgi.sqlite" />
</p>

A collection is a folder with a `cards` folder of Markdown files inside. Each file is a
deck, and folders nest decks: the `lang` deck holds `cards/lang.md` and everything in
`cards/lang/`. Beside `cards` are `media` for images and sounds and `note-types` for
[notes with fields](https://docs.amgi.study/card-format#notes-with-fields); review
history is stored in `.amgi.sqlite`. Anything else in the folder is left alone.

```
README.md
cards/
  Spanish.md
  lang/Korean.md
media/
note-types/
```

## What it does

<img src="media/screenshots/math.webp" alt="A card with the chain rule typeset as math" />

- **Rich cards:** images, sounds, TeX math, code highlighted in 200+ languages, and
  [notes with fields](https://docs.amgi.study/card-format#notes-with-fields) that make
  several cards from one entry, such as word → meaning and meaning → word.
- **Add a card from any app** with <kbd>⌥⌘N</kbd>, and see what's due from the menu bar.
- **Bring your Anki decks:** `.apkg` and `.colpkg` files become Markdown with their media
  and review history, and an [AnkiConnect](https://docs.amgi.study/yomitan) server lets
  Yomitan and other Anki tools add cards.
- **A command line:** `amgi status`, `check`, `stats` with a year of activity, retention
  and the coming workload, `search` with `deck:french #verbs is:due` or `lapses>3`, and
  `diff` to list the cards added, changed and removed since a git commit.

## Install

Amgi is early and changing quickly, so there are no downloads for now: build it from
source. It's a Rust core with a SwiftUI app and a Svelte page in a web view, and needs
macOS 26 on Apple Silicon, Xcode, Rust and [Bun](https://bun.sh).

```sh
make install                 # build Amgi.app and put it in /Applications
cargo install --path cli     # the amgi command
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the project layout and tests.

## Docs

The full docs are at [docs.amgi.study](https://docs.amgi.study).

- [Card format](https://docs.amgi.study/card-format): cards, clozes, media, math, code, tags and suspending
- [Studying](https://docs.amgi.study/studying): shortcuts, scheduling, adding cards and the menu bar
- [Importing from Anki](https://docs.amgi.study/anki-import)
- [Adding cards from Yomitan](https://docs.amgi.study/yomitan): AnkiConnect for Yomitan and other tools made for Anki
- [Command line](https://docs.amgi.study/cli): search, stats and linting

## License

[MIT](LICENSE). The fonts (Source Serif 4 and Noto Serif CJK) are
under the [SIL Open Font License](https://openfontlicense.org).

*Amgi* (암기) is Korean for memorization.
