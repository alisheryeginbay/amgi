# Agent guide

Amgi is a macOS flash-card app: a Rust core, a SwiftUI shell and a Svelte page in a web
view. Read [CONTRIBUTING.md](CONTRIBUTING.md) for the layout and build. `examples/` is a
sample collection showing the card format, which `core/src/parser.rs` reads.

## Commands

```sh
make build                   # build the core, web page and app
make test                    # cargo test, then bun test in web/
cd web && bun run check      # svelte-check (types)
amgi check --dir examples    # validate deck files
```

Use `PROFILE=debug make build` for a faster core build while iterating.

## Rules

- Don't run `make bundle` / `make install` (`scripts/bundle.sh`, `scripts/install.sh`)
  or launch `Amgi.app`; the owner runs the app themselves.
- Don't edit generated code: `app/Sources/AmgiCore/`, `app/Sources/amgi_coreFFI/include/`
  (UniFFI, from `scripts/build-core.sh`) and `app/Sources/Amgi/Web/` (from
  `scripts/build-web.sh`). Change `core/` or `web/` and rebuild.
- A change to the core's public API needs `make build` so the Swift bindings regenerate.
- Card identity comes from the card's text, so parser changes must keep existing ids
  stable or review history is lost.
- Keep the README short. When a change affects what users type at the command line,
  update `amgi --help`.
- Use neutral content in new examples and tests.

## Commits

- Conventional commits (`feat:`, `fix:`, `perf:`, `docs:`), with the subject written
  as what the change does for the user, e.g. `feat: take back the last answer with ⌘Z`.
- Commit straight to `main`; no feature branches.
- Fold small follow-ups into the unpushed commit they fix instead of stacking
  tweak or revert commits.
