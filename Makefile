# Everyday tasks. Each one rebuilds only what changed since the last run.
#   make           build the core and the web panes if needed, then run the app
#   make dev       same, but serve the panes from Vite with live reload
#   make build     build everything without running
#   make test      core and web tests
#   make bundle    build target/bundle/Amgi.app
# PROFILE=debug builds the Rust core without optimizations (faster to compile).

PROFILE ?= release
WEB_OUT := app/Sources/Amgi/Web/index.html
WEB_SRC := $(shell find web/src -type f) web/index.html web/vite.config.ts \
	web/svelte.config.js web/tsconfig.json web/node_modules/.package-lock.json

.PHONY: run dev build core web test bundle

run: build
	swift run --package-path app

dev: core $(WEB_OUT)
	@(cd web && exec node_modules/.bin/vite --strictPort) & vite=$$!; \
	trap 'kill $$vite 2>/dev/null' EXIT; \
	AMGI_WEB_URL=http://localhost:5173 swift run --package-path app

build: core web
	swift build --package-path app

# Cargo tracks the core's sources itself, and a no-op build takes under a second.
core:
	@scripts/build-core.sh $(PROFILE)

web: $(WEB_OUT)

$(WEB_OUT): $(WEB_SRC)
	@scripts/build-web.sh

web/node_modules/.package-lock.json: web/package-lock.json
	cd web && npm ci

test:
	cargo test
	cd web && npm test

bundle:
	scripts/bundle.sh
