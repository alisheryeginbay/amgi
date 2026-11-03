# Contributing

## Layout

- `core/`: Rust library containing the parser, FSRS-6 scheduler, SQLite store and the
  `CardCollection` API, exported through [UniFFI](https://mozilla.github.io/uniffi-rs/)
- `cli/`: the `amgi` command, built on the core
- `app/`: SwiftUI macOS app (Swift package): the window, sidebar, toolbar and settings
- `web/`: the study, dashboard and browse panes in [Svelte](https://svelte.dev), shown in
  one web view. The app sends each pane its data and the page sends back what you do;
  cards' images and sounds are served from the collection at `amgi://collection/`.
- `examples/`: a sample collection, its decks in `examples/cards/`

## Build & run

```sh
make                         # rebuild whatever changed (Rust core, web panes), then run the app
make dev                     # same, with the panes served by Vite and reloaded live
make build                   # build without running
make test                    # core and web tests
make bundle                  # build target/bundle/Amgi.app (release, signed ad-hoc)
make install                 # same, then put it in /Applications
```

Building needs Rust and [Bun](https://bun.sh); `make bundle` and `make install` also need Xcode, whose `actool`
compiles the app icon, `app/Amgi.icon` (open it in Icon Composer to change it). `make` runs `scripts/build-core.sh` and
`scripts/build-web.sh` for you, only when `core/` or `web/` changed; `PROFILE=debug make`
builds the core without optimizations. If you open `app/Package.swift` in Xcode, run
`make build` first after changing `core/` or `web/`.

`make` and `make dev` run the app as **Amgi Dev**, with a violet flower in the Dock and
menu bar, so it isn't mistaken for an installed Amgi.app. Its settings, including the
folder it last opened, are kept apart from the app's, but review history lives in the
collection folder, so give it a copy of your decks rather than the ones you study.

Debug builds can be inspected with Safari's Web Inspector (Develop ▸ your Mac ▸ Amgi Dev).

## Docs

`docs/` is a [Mintlify](https://mintlify.com) site; `docs/docs.json` sets the navigation
and theme. Preview it with `cd docs && npx mint dev` and check its links with
`npx mint broken-links`. The logo, favicon and screenshot in `docs/` are copies of the
files in `media/`.

## Generated files

- The emoji picker's names and keywords come from GitHub's
  [gemoji](https://github.com/github/gemoji) (MIT); `scripts/build-emoji.py` refreshes them.
- `app/Sources/Amgi/AmgiDev.icns`, Amgi Dev's violet icon, is built from `app/Amgi.icon`
  by `scripts/build-dev-icon.sh` (needs Xcode); run it again after changing the icon.
- `media/amgi.svg`, the handwritten "Amgi" and 암기, is built by `bun scripts/readme-svg.mjs`.

## Versions

The version is set once, under `[workspace.package]` in the root `Cargo.toml`; the
crates, `amgi --version` and the app's bundle all read it. The app's build number is the
commit count.

## Installing

There are no releases for now, while things change quickly. `make install` builds
Amgi.app in release mode and puts it in `/Applications`, quitting and reopening Amgi if
it's open. It's signed ad-hoc, so macOS may ask again for access to the collection's
folder after an install; `AMGI_SIGN_IDENTITY="name" make install` signs with a
certificate from the keychain instead (a self-signed code-signing one will do), which
keeps the app's identity between builds. The old releases on GitHub stay up.

The Chinese, Japanese and Korean fonts aren't in the app: it downloads them from the
`fonts-1` GitHub release when a collection first needs them (`FontPacks.swift`).
`scripts/build-fonts.sh` builds those zips from `web/node_modules`; its header says how
to publish new ones.

Every push to `main` runs the tests, type checks and a debug build
(`.github/workflows/ci.yml`).
