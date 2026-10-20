# Contributing

## Layout

- `core/`: Rust library containing the parser, FSRS-6 scheduler, SQLite store and the
  `CardCollection` API, exported through [UniFFI](https://mozilla.github.io/uniffi-rs/)
- `cli/`: the `amgi` command, built on the core
- `app/`: SwiftUI macOS app (Swift package): the window, sidebar, toolbar and settings
- `web/`: the study, dashboard and browse panes in [Svelte](https://svelte.dev), shown in
  one web view. The app sends each pane its data and the page sends back what you do;
  cards' images and sounds are served from the collection at `amgi://collection/`.
- `examples/`: a sample collection

## Build & run

```sh
make                         # rebuild whatever changed (Rust core, web panes), then run the app
make dev                     # same, with the panes served by Vite and reloaded live
make build                   # build without running
make test                    # core and web tests
make bundle                  # build target/bundle/Amgi.app (release, signed ad-hoc)
```

Building needs Rust and [Bun](https://bun.sh); `make bundle` also needs Xcode, whose `actool`
compiles the app icon, `app/Amgi.icon` (open it in Icon Composer to change it). `make` runs `scripts/build-core.sh` and
`scripts/build-web.sh` for you, only when `core/` or `web/` changed; `PROFILE=debug make`
builds the core without optimizations. If you open `app/Package.swift` in Xcode, run
`make build` first after changing `core/` or `web/`.

`make` and `make dev` run the app as **Amgi Dev**, with a violet flower in the Dock and
menu bar, so it isn't mistaken for an installed Amgi.app. Its settings, including the
folder it last opened, are kept apart from the app's, but review history lives in the
collection folder, so give it a copy of your decks rather than the ones you study.

Debug builds can be inspected with Safari's Web Inspector (Develop ▸ your Mac ▸ Amgi Dev).

## Generated files

- The emoji picker's names and keywords come from GitHub's
  [gemoji](https://github.com/github/gemoji) (MIT); `scripts/build-emoji.py` refreshes them.
- `app/Sources/Amgi/AmgiDev.icns`, Amgi Dev's violet icon, is built from `app/Amgi.icon`
  by `scripts/build-dev-icon.sh` (needs Xcode); run it again after changing the icon.
- `media/amgi.svg`, the handwritten "Amgi" and 암기, is built by `bun scripts/readme-svg.mjs`.
