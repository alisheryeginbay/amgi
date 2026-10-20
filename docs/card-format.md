# Card format

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
- Math: `$…$` inline and `$$…$$` for display, in TeX, turned into MathML by
  [Temml](https://temml.org) and drawn by WebKit in the system's math font. Brackets and `|` inside math are literal, so they never
  start a cloze. A `$` only starts math before a non-space (`$5 and $10` stays text);
  write `\$` for a literal one.
- Code: fenced blocks (```` ```python ```` … ```` ``` ```` or `~~~`) are highlighted by
  [Shiki](https://shiki.style) in any of its 200+ languages, with GitHub's light and dark
  themes. Each language's grammar loads the first time a card uses it. Blank lines and
  `Q:` lines inside a block stay part of the card. Brackets in code are literal, so a cloze
  gap in a code block or `` `span` `` is written `{{answer}}` or `{{answer|hint}}`:

  ````markdown
  C: Raise a matrix to a power with the [expm|package] package:
  ```r
  powmat <- {{expm|function}}(1.1 * logm(mat))
  ```
  ````
- A card ends at a blank line or the next `Q:` / `C:` outside a code block. Every other
  line is ignored.

A card's identity comes from its text (the question for basic cards, the whole text
for clozes). You can move or reorder cards, fix answers and change tags without losing
history. Editing a question starts the card over.

## Tags and deck settings

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
its cards across decks. Right-click a tag and choose **Choose Icon…** to give it an emoji:
type to search by name or keyword, use the arrow keys to move, and press <kbd>Return</kbd> to
choose. Icons are kept in `.amgi.sqlite`.

## Suspending cards

A card tagged `#suspended` never comes up for study, and isn't counted as due or new;
`tags = ["suspended"]` in a deck's front matter suspends the whole deck. **Suspend Card**
in the toolbar while studying, or right-clicking cards while browsing, adds or removes the
tag in the file for you. Tags aren't part of a card's identity, so it keeps its history.
Pick the `suspended` tag in the sidebar to see them all.
