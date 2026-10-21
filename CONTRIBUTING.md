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

## Versions

The version is set once, under `[workspace.package]` in the root `Cargo.toml`; the
crates, `amgi --version` and the app's bundle all read it. The app's build number is the
commit count. Until 1.0, bump the minor version for new features and the patch version
for fixes, then tag the release commit `vX.Y.Z`. A change to card ids loses review
history, so call it out in the release.

## Releases

Pushing a `vX.Y.Z` tag runs `.github/workflows/release.yml`: it builds Amgi.app, signs
it with the Developer ID, notarizes it and attaches it to a draft release on GitHub as
`Amgi-X.Y.Z.dmg` and `Amgi-X.Y.Z.zip`. Write the notes there, then publish it. The tag
has to match the version in `Cargo.toml`.

The release also carries `appcast.xml`, the feed that installed copies check through
[Sparkle](https://sparkle-project.org) (Amgi ▸ Check for Updates…). It lists the zip,
signed with an EdDSA key (`scripts/appcast.sh`). The app reads the feed from the latest
published release, so a draft isn't offered until it's published. Keep the private
key safe: without it, installed copies can't be updated.

The disk image is made by [dmgbuild](https://dmgbuild.readthedocs.io), run through
[uv](https://docs.astral.sh/uv/); `scripts/dmg-settings.py` places the icons to match
the drawn arrow in `media/dmg-background.tiff`. That background comes from the separate
amgi-install repo as one TIFF with a 1x and a 2x image, a dpi for each and a color
profile: replace it as a whole and don't convert, resize or strip it, or Finder loses
the Retina image. `scripts/dmg.sh path/to/Amgi.app` builds an unsigned image to check
the layout.

The workflow needs these repository secrets:

- `DEVELOPER_ID_P12`: the Developer ID Application certificate with its private key,
  exported from Keychain Access as a .p12 and base64-encoded (`base64 -i cert.p12 | pbcopy`)
- `DEVELOPER_ID_P12_PASSWORD`: the password chosen when exporting it
- `NOTARY_KEY`, `NOTARY_KEY_ID`, `NOTARY_ISSUER_ID`: an App Store Connect API key
  (Users and Access ▸ Integrations), the .p8 file's contents and its two ids
- `SPARKLE_ED_PRIVATE_KEY`: the private key from Sparkle's `generate_keys`, exported
  with `generate_keys -x key.txt`; its public half is `SUPublicEDKey` in
  `scripts/bundle.sh`

To release from this Mac instead:

```sh
AMGI_SIGN_IDENTITY="Developer ID Application" scripts/bundle.sh
NOTARY_PROFILE=amgi scripts/notarize.sh   # after `xcrun notarytool store-credentials amgi`;
                                          # writes the .zip and the .dmg
scripts/appcast.sh                        # signs with the key in the login keychain
```

The Chinese, Japanese and Korean fonts aren't in the app: it downloads them from the
`fonts-1` GitHub release when a collection first needs them (`FontPacks.swift`).
`scripts/build-fonts.sh` builds those zips from `web/node_modules`; its header says how
to publish new ones.

Every push to `main` runs the tests, type checks and a debug build
(`.github/workflows/ci.yml`).
