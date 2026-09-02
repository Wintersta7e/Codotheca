# Codotheca

A library for the code you have written. Every repository you own — on disk or on a forge —
becomes a tile with generated cover art, real status and a launch button. It tracks time the
way a game launcher does, shows which projects are rotting, and adds a light, ignorable layer
that makes upkeep and consistency visible.

Local-first: an on-disk database, no account, no telemetry, and no network traffic except to
the forge you connect.

**Status: pre-alpha.** The design and the phase-1 specification are complete; implementation
has not started. Windows and Linux; macOS is deferred until it can be tested on real hardware.

## Layout

| Path | What |
|---|---|
| `core/` | Rust core — scanning, git, database, art rasterisation. Runs as a child process |
| `app/` | Electron shell and TypeScript/React renderer |
| `protocol/` | The single source of truth for the core↔shell contract, plus codegen |
| `scripts/` | Development and release helpers |

## Building

Requires Rust (stable), Node LTS, and `git` 2.22 or newer on `PATH`.

```sh
npm install           # workspace dependencies
npm run gen           # generate protocol bindings for both languages
npm run build         # core (release) + app bundles
npm run check:bundle  # guard the bundle after a build
npm run dev           # run the shell against the dev server
```

Windows builds carry a Linux worker for repositories that live inside WSL; see
[docs/wsl-worker.md](docs/wsl-worker.md).

## Licence

GPL-3.0-or-later. See `LICENSE`.
