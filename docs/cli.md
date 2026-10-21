# Command line

`amgi` reads a collection from the terminal. Install it with `cargo install --path cli`.

```sh
amgi status                  # cards left today, by deck; --short for one line, empty when done
amgi check                   # problems in the deck files as path:line: message; exits 1 if any
amgi search deck:french is:due   # the search syntax; --view Leeches runs a saved view
amgi decks                   # every deck with due, new and total counts
amgi tags                    # every tag with its card count
amgi info cards/geography.md:12   # a card's schedule and history, by id, path:line or a search
amgi stats                   # streak, retention, recent reviews and the next two weeks
```

`search` takes the same syntax as the app; see [Searching](studying.md#searching).

Every command takes `--json`. `search` exits 1 when nothing matches, and `--ids` prints
only card ids. `--dir` (or `AMGI_DIR`) picks the collection; without it, `amgi` uses the
folder holding `.amgi.sqlite` at or above the current one, or else the folder last opened
in the app.
