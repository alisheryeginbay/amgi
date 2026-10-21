# Importing from Anki

**File ▸ Import Anki Deck…** (<kbd>⇧⌘I</kbd>) converts an `.apkg` or `.colpkg` into Markdown
files in the open folder, one per deck (`Languages::Korean` becomes `Languages/Korean.md`).
Existing files are never overwritten. Images and sounds are copied to a `media` folder at
the root, and note tags become `#tag` lines.

- Note types with more than two fields, such as vocabulary with word, meaning, audio,
  image and sentence, keep their fields: each note becomes an `@` line and a block of
  `Field: value` lines, and the note type a file in `note-types`, with a rule per card
  type made from the fields its template shows on each side (see
  [Notes with fields](card-format.md#notes-with-fields)). A note type file already there
  with the same definition is shared; one that differs is kept, and the imported one is
  named `Vocabulary 2` and so on.
- Other note types become `Q:`/`A:` cards rendered from their templates: those with two
  fields, like Basic, and those whose questions a list of fields can't describe, with
  text of their own (`Plural of {{Word}}?`), a script, or fields shown only when another
  field is filled. So does any note whose cards amgi's rules wouldn't make the same way,
  or whose cards are in different decks or only partly suspended.
- Cloze notes become `C:` cards, with `Back Extra` kept as a note under the card. Anki
  can test several deletions together (`c1` used twice); amgi gives each its own card.
- Review history is replayed through amgi's FSRS scheduler, and due dates follow Anki's.
- MathJax `\(…\)` and `\[…\]` become `$…$` and `$$…$$`. A cloze inside math is
  taken out of it, so the gap holds its own math.
- `[sound:…]` references become sound links. Text-to-speech (`{{tts:…}}`) is left out.
- Suspended cards are imported with the `#suspended` tag and keep their history. amgi
  suspends a cloze as a whole, so when only some of a cloze's deletions were suspended in
  Anki, those get a tagged copy of the cloze of their own.
- Image occlusion notes are left out.
