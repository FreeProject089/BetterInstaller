# Contributing to BetterInstaller

🇬🇧 English · [🇫🇷 Français](CONTRIBUTING_FR.md)

Thanks for your interest! BetterInstaller is a cross-platform installer/updater
framework (Rust + Slint). This guide covers setup, the quality gate, and how to file
issues and pull requests.

## TL;DR

```sh
# build the engine + CLI
cargo build --release -p bpkg-cli -p installer
# the gate that CI enforces (run it before pushing):
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Project layout

```
crates/bpkg-core/   library: .bpkg format, signing, handoff, update, platform ops
crates/bpkg-cli/    the `bpkg` command-line tool
crates/installer/   the Slint GUI engine (betterinstaller.exe)
examples/bmm/       a complete, real worked example (config + build/release scripts)
docs/               architecture, format, CLI, handoff, updates, signing, platforms
```

## Prerequisites

- **Rust** stable (matches CI: latest stable) with `clippy` + `rustfmt`
  (`rustup component add clippy rustfmt`).
- **Linux build deps** (Slint + rfd file dialog):
  `sudo apt-get install -y libfontconfig-dev libxcb-shape0-dev libxcb-xfixes0-dev libgtk-3-dev pkg-config`
- **Windows / macOS**: no extra system deps for the default build.

### Build everything on Linux via Docker (no local toolchain fuss)

```sh
docker build -t betterinstaller-dev .
docker run --rm -e CARGO_TARGET_DIR=/tmp/t -v "${PWD}:/app" betterinstaller-dev   # runs the full gate
```

`CARGO_TARGET_DIR=/tmp/t` keeps Linux artifacts out of your host `./target`.

## The quality gate (must be green)

CI runs on **windows-latest + ubuntu-latest**:

1. `cargo fmt --all -- --check` — formatting.
2. `cargo clippy --workspace --all-targets -- -D warnings` — **every warning is an error**.
3. `cargo test --workspace`.
4. `cargo build --workspace --release`.

Run the same locally before opening a PR. Clippy is the most common failure — fix the
lint, don't `#[allow]` it (unless there's a clear, commented reason).

## Coding style

- Format with `rustfmt` (no manual deviations).
- Keep functions small and documented; match the surrounding comment density and naming.
- Cross-platform code lives behind the `platform` trait — don't `#[cfg]`-spaghetti the
  shared code. New OS behaviour goes in the per-OS backend.
- Security-sensitive paths (signing, extraction, paths from a manifest) must validate
  inputs (no path traversal, verify signatures before applying). See `docs/SIGNING.md`.

## Commits & pull requests

- **Branch** from `main`; one focused change per PR.
- **Commit messages**: imperative present tense (`Add …`, `Fix …`, `Refactor …`).
  Conventional-commit prefixes (`feat:`, `fix:`, `docs:`, `chore:`) are welcome.
- **PR description**: what + why, how you tested, and any platform notes
  (did you build/test on Linux + Windows?).
- Keep the gate green and update docs when behaviour changes.

## Filing issues

Search existing issues first. Then pick the right type:

### 🐛 Bug report — include:
- BetterInstaller / `bpkg` version and **OS** (Windows / Linux / macOS + version).
- **Steps to reproduce** (the exact `bpkg`/installer command, or the click path).
- **Expected vs actual**, plus the full error output.
- Your `installer.toml` (redact secrets) and whether the package was signed.

### ✨ Feature request — include:
- The problem you're solving (not just the solution).
- Who benefits and a rough use case.
- Any alternatives you considered.

### 🔒 Security
Do **not** open a public issue for vulnerabilities. Report privately (see
`SECURITY.md` if present, otherwise contact the maintainer). Signing, sandboxing and
package-integrity bugs are treated as high priority.

## Releasing (maintainers)

See `examples/bmm/release.ps1` and `docs/UPDATER-SETUP.md` — bump versions → build →
delta → multi-source `update.json` → `gh release create` with the 3 (or 4, with a delta)
assets.
