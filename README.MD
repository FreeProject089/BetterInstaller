# BetterInstaller

A proprietary, cross-platform **installer / updater framework** — one engine, one
`installer.toml` per project. Built to replace NSIS / MSI for BMM and any other app.

> Full design: `../.Assets/.md/PLAN_BETTER_INSTALLER.md` (v2 + **v3 Addendum**).

## Why
- **Reusable** — the engine never hard-codes a project. Each app ships an `installer.toml`.
- **Beautiful & native** — UI in [Slint](https://slint.dev) (single static binary, no WebView runtime).
- **Portable & small** — one self-contained exe, `<5 MB` target, zstd-compressed payload.
- **Secure** — per-file SHA-256, Ed25519 signing (Phase 7), integrity verified at runtime.
- **First-run handoff** — the installer can pre-configure the app (language, privacy/ToS,
  tutorial, telemetry, components) so there are **no first-launch modals**. App-agnostic via a
  standard `installer-handoff.json` the app reads once.

## Status — Phase 1 (in progress)
Implemented & green (`cargo check` + tests):
- `bpkg-core` — the engine:
  - `.bpkg` container format (build / verify / extract, roundtrip-tested)
  - package `manifest` (app + files + components + SHA-256)
  - `installer.toml` parsing incl. the `[handoff]` + `[[setup_option]]` config
  - the `installer-handoff.json` contract + builder (unit-tested)
  - `PlatformOps` abstraction (Windows / Linux / macOS path backends)
- `bpkg` CLI — `pack`, `info`, `verify`, `extract`

Next: Phase 2 (Slint UI) → Phase 2.5 (handoff page + BMM reader) → Phase 3 (system integration).

## Workspace
```
crates/
  bpkg-core/   # engine library
  bpkg-cli/    # `bpkg` packaging tool
examples/
  bmm/         # Better Mods Manager installer.toml (reference config)
```

## Try it
```sh
cargo build -p bpkg-cli
./target/debug/bpkg pack  --root <files> --config examples/bmm/installer.toml --out app.bpkg
./target/debug/bpkg info  app.bpkg
./target/debug/bpkg verify app.bpkg
./target/debug/bpkg extract app.bpkg --dest out/
```

## License
MIT OR Apache-2.0.
