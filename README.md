<p align="center">
  <img src="media/logo/amgi-symbol.svg" alt="" width="96" />
</p>
<h1 align="center">Amgi</h1>

<p align="center">
  <b>Flash cards in plain Markdown files, scheduled with <a href="https://github.com/open-spaced-repetition/fsrs4anki/wiki/The-Algorithm">FSRS</a>.</b><br />
  A macOS app to study them, and a command line for everything else.
</p>

<p align="center">
  <img src="media/screenshots/review.webp" alt="Amgi reviewing a card" />
</p>

```markdown
Q: What does **FSRS** stand for?
A: Free Spaced Repetition Scheduler

C: The capital of [France] is [Paris|city].
```

A collection is a folder of Markdown files, one deck each. Edit them in any editor and
keep them in git; review history lives beside them in `.amgi.sqlite`.

- Images, sounds, math, code, and notes with fields that make several cards each
- `amgi` to search, check and see your stats; `amgi --help` for more
- Anki decks import with their review history

## Install

Build from source; it needs macOS 26 on Apple Silicon, Xcode, Rust and [Bun](https://bun.sh).

```sh
make install                 # Amgi.app in /Applications
cargo install --path cli     # the amgi command
```

`examples/` is a sample collection, and [CONTRIBUTING.md](CONTRIBUTING.md) describes the
code.

## License

[MIT](LICENSE); the fonts are under the [SIL Open Font License](https://openfontlicense.org).
*Amgi* (암기) is Korean for memorization.
