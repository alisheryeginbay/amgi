# Studying

Keys while studying: <kbd>Space</kbd> shows the answer, <kbd>1</kbd>–<kbd>4</kbd> rate
Again/Hard/Good/Easy, <kbd>⌘Z</kbd> takes back the last rating and shows that card again,
<kbd>R</kbd> replays the card's sounds, <kbd>⌘O</kbd> opens a folder, <kbd>⌘R</kbd> reloads,
and <kbd>⌘1</kbd>–<kbd>⌘3</kbd> go to Dashboard, Review and Decks. Files also reload
whenever the app comes to the front.

A card that asks you to [type its answer](card-format.md#notes-with-fields) has a box
under the question: type, then press <kbd>Return</kbd> to see the answer with what you
typed checked against it. Rate it with <kbd>1</kbd>–<kbd>4</kbd> as usual.

**Review** in the sidebar studies the whole collection. Pick a deck or tag to browse its
cards in a table: search (<kbd>⌘F</kbd>), sort by any column, and double-click a card to open its file.
The selected card shows beside the table as it looks when studied; drag the divider to
resize it, or hide it with **Hide Card** in the toolbar.

For a card from a [note](card-format.md#notes-with-fields), the note's fields are listed
under it, by name, with the empty ones left out.

## Adding cards

**Add Card** (<kbd>⌥⌘N</kbd> from any app, or <kbd>⌘N</kbd> in Amgi) opens a small panel
over whatever you're doing: pick a deck, type a question and answer or a cloze, and press
<kbd>⌘↩</kbd>. The card is appended to the end of the deck's file, which is created if
needed. <kbd>Return</kbd> moves to the next field and <kbd>⌥↩</kbd> starts a new line; <kbd>Esc</kbd> puts the panel away and
keeps what you typed. Turn the shortcut off in **Settings**.

Next to Basic and Cloze, the menu at the top lists your
[note types](card-format.md#notes-with-fields); a deck with notes starts with the type
most of them have. For a note, the panel shows a box for each field and says how many
cards the note makes; fields left empty are left out. Drop an image or sound onto a
field to copy it into the collection's `media` folder and link it there.

## Menu bar

The menu bar shows how many cards are due. Its menu lists what's left today, the
reviews done so far and each deck with cards to study; pick one to open it, or **Study
Now** to start reviewing. While it's shown, Amgi keeps running when its window is closed.
Hide it in **Settings** (<kbd>⌘,</kbd>).

## Saved views

**Saved Views** are searches with a name. Type a search and click **Save Search**
(the magnifier with a plus, next to the search field) to add one; the deck, tag or view
you're browsing is part of it. Right-click a view to edit its search, give it an icon or
delete it. Views are kept in `.amgi.sqlite`. Every collection starts with **Leeches**
(`is:leech`): cards you have forgotten 8 or more times, which usually need rewording or
splitting.

## Searching

Every word in a search must match the card's question, answer, tags or note fields,
ignoring case and accents. Words can also be filters:

| Search | Finds cards |
|---|---|
| `"capital of"` | with that exact phrase |
| `-paris` | without `paris` (works before any filter, too) |
| `#verbs`, `#lang*` | tagged `verbs`; with a tag starting with `lang` |
| `deck:french` | in a deck named `french` or under one, such as `lang/french/words` |
| `q:paris`, `a:"to go"` | with that text in the question or answer only |
| `meaning:apple`, `wordmeaning:go` | from a note whose field of that name has the text; leave out spaces in the name |
| `is:new` `is:due` `is:suspended` `is:leech` `is:cloze` `is:basic` | in that state or of that kind |
| `lapses>3`, `reviews<=5` | compared by count (`<`, `<=`, `>`, `>=`, `=`) |
| `due<7`, `due<0` | due within 7 days; overdue. New cards have no due date |

Anything else, such as `12:30`, is searched as text.
