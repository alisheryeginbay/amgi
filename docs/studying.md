# Studying

Keys while studying: <kbd>Space</kbd> shows the answer, <kbd>1</kbd>–<kbd>4</kbd> rate
Again/Hard/Good/Easy, <kbd>⌘Z</kbd> takes back the last rating and shows that card again,
<kbd>R</kbd> replays the card's sounds, <kbd>⌘O</kbd> opens a folder, <kbd>⌘R</kbd> reloads,
and <kbd>⌘1</kbd>–<kbd>⌘3</kbd> go to Dashboard, Review and Decks. Files also reload
whenever the app comes to the front.

**Review** in the sidebar studies the whole collection. Pick a deck or tag to browse its
cards in a table: search (<kbd>⌘F</kbd>), sort by any column, and double-click a card to open its file.
The selected card shows beside the table as it looks when studied; drag the divider to
resize it, or hide it with **Hide Card** in the toolbar.

## Adding cards

**Add Card** (<kbd>⌥⌘N</kbd> from any app, or <kbd>⌘N</kbd> in Amgi) opens a small panel
over whatever you're doing: pick a deck, type a question and answer or a cloze, and press
<kbd>⌘↩</kbd>. The card is appended to the end of the deck's file, which is created if
needed. <kbd>Return</kbd> moves to the next field and <kbd>⌥↩</kbd> starts a new line; <kbd>Esc</kbd> puts the panel away and
keeps what you typed. Turn the shortcut off in **Settings**.

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

Every word in a search must match the card's question, answer or tags, ignoring case
and accents. Words can also be filters:

| Search | Finds cards |
|---|---|
| `"capital of"` | with that exact phrase |
| `-paris` | without `paris` (works before any filter, too) |
| `#verbs`, `#lang*` | tagged `verbs`; with a tag starting with `lang` |
| `deck:french` | in a deck named `french` or under one, such as `lang/french/words` |
| `q:paris`, `a:"to go"` | with that text in the question or answer only |
| `is:new` `is:due` `is:suspended` `is:leech` `is:cloze` `is:basic` | in that state or of that kind |
| `lapses>3`, `reviews<=5` | compared by count (`<`, `<=`, `>`, `>=`, `=`) |
| `due<7`, `due<0` | due within 7 days; overdue. New cards have no due date |

Anything else, such as `12:30`, is searched as text.
