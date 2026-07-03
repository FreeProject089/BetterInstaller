# BetterInstaller — Technical Analysis

> A from-scratch engineering deep-dive into **BetterInstaller**: a proprietary,
> reusable installer/updater framework — a cross-platform NSIS/MSI alternative with a
> native Slint UI, Ed25519-signed self-extracting packages, first-run config handoff,
> and updates with rollback. Companion docs: **App_Features_EN.md** (what it does) and
> **Security_Audit_EN.md** (threat review).

---

## 1. Shape of the project

A Rust workspace (`Cargo.toml`, `resolver = "2"`), release profile tuned for a tiny
binary (`opt-level="z"`, `lto`, `codegen-units=1`, `strip`, `panic="abort"` — target
<5 MB). Three crates:

| Crate | Kind | Role |
|---|---|---|
| `bpkg-core` | library | the engine — the `.bpkg` package format, signing, delta, update/rollback, platform integration, config, i18n, handoff |
| `bpkg-cli` (`bpkg`) | binary | the packaging CLI — `keygen`, `pack`, `sign`, `stamp`, `verify` |
| `installer` | binary (Slint) | the GUI installer produced for an end app (self-extracting `Setup.exe`) |

Key dependencies: `serde`/`serde_json`/`toml` (config + manifest), `zstd`
(compression), `sha2` (integrity), `ed25519-dalek` + `rand` (signing), `qbsdiff`
(binary deltas), `reqwest` (rustls, blocking, for updates), `slint` (native UI),
`winreg`/`mslnk` (Windows integration), `embed-manifest` (Windows app manifest).

## 2. The `.bpkg` package format (`package/`)

A `.bpkg` is a single self-describing blob:

```
[ Header (fixed HEADER_LEN) ][ Manifest (JSON, manifest_len) ][ Payload (zstd, payload_len) ][ Signature (64B, if FLAG_SIGNED) ]
```

- **`format.rs`** — the binary `Header` (magic, version, flags incl. `FLAG_SIGNED`,
  `manifest_len`, `payload_len`) with `from_bytes`/`to_bytes`.
- **`manifest.rs`** — `Manifest { app: AppMeta, files: [{ path, sha256, component }] }`.
  `AppMeta` = id/name/version/publisher/homepage/platforms.
- **`writer.rs`** — builds the inner archive (a length-prefixed `path|data` stream),
  zstd-compresses it, computes per-file SHA-256, writes header+manifest+payload.
- **`reader.rs`** — `Package::open` reads header+manifest eagerly; the payload is read
  on demand. Provides `verify()` (every file's SHA-256 vs manifest),
  `verify_signature(vk)` (Ed25519 over the manifest+payload bytes), `read_files()`
  (peek license docs before install), and the two extract paths:
  `install_with_progress()` (verify each hash **then** write, progress callback) and
  `extract()`. Both apply a **path-traversal guard** (`..`, leading `/` or `\`).
  Archive parsing is fully bounds-checked (`slice`/`read_u32`/`read_u64` use
  `checked_add` and reject truncation) — no out-of-bounds reads on a corrupt package.

## 3. Signing (`sign.rs`)

Ed25519 via `ed25519-dalek`. `generate()` uses the OS CSPRNG (`OsRng`); keys are
stored as 32-byte hex (`private.key` seed / `public.key` verifying key), length- and
hex-validated on load. A signature covers the **manifest + payload** bytes.
`bpkg-cli` signs (`bpkg sign --key private.key app.bpkg`) and verifies; the installer
verifies against a `public_key` pinned in `installer.toml`.

## 4. Config & embedding (`config.rs`, `embed.rs`)

- **`config.rs`** — `installer.toml`: app metadata, UI/branding, components, shortcuts,
  prerequisites, `security { public_key, require_signature }`, update sources. This is
  the single source of truth an app author edits.
- **`embed.rs`** — the self-extracting trick: the built installer exe carries the
  config + `.bpkg` as an appended, magic-delimited blob; at runtime the installer
  locates and stages it to a temp file so `Package::open` can read it.

## 5. Updates & rollback (`update.rs`, `delta.rs`, `net.rs`)

- **`net.rs`** — minimal blocking HTTP (rustls, 60 s timeout, versioned UA):
  `fetch_text` (update manifest JSON) and `download` (bytes for a `.bpkg`/patch).
- **`delta.rs`** — `qbsdiff`/`qbspatch` binary diff/patch, so an update can ship a tiny
  patch (`old.bpkg → new.bpkg`) instead of the full package.
- **`update.rs`** — `UpdateManifest { version, url, deltas[] }`. `check_remote` /
  `check_remote_multi` fetch one or several manifest mirrors and return the single
  newest (dead mirrors are skipped; all-failed is an error, not a silent "up to date").
  `download_and_apply` prefers a delta from the current version, else full download,
  then `apply_package_update`, which is **atomic-ish**: snapshot the install dir to a
  sibling `<name>.bak`, install over it, and on **any** error wipe + restore the
  snapshot; on success drop it. (A test flips a payload byte and asserts the rollback.)

  > **Note (see Security_Audit):** `download_and_apply`/`apply_package_update` verify
  > each file's SHA-256 against the *package's own manifest* (self-consistency) but do
  > **not** call `verify_signature` — authenticity is only enforced where the installer
  > GUI verifies against the pinned `public_key`.

## 6. Platform integration (`platform/`)

A `PlatformOps` trait with `windows.rs` / `linux.rs` / `macos.rs` backends: default
install dir, app-data dir, shortcuts, protocol handler registration, uninstaller
(Add/Remove Programs) registration, PATH entry, installed dir/version lookup.

**Windows** is deliberately **per-user, no admin/UAC** (`asInvoker` manifest): installs
to `%LOCALAPPDATA%\Programs\<name>`, writes only `HKCU` (Classes protocol handler,
`CurrentVersion\Uninstall` entry, `Environment` PATH). Installing into Program Files
would require a separate elevated build.

## 7. Handoff & prerequisites (`handoff.rs`, `prereq.rs`)

- **`handoff.rs`** — the first-run contract: the installer writes a small handoff file
  the installed app reads on first launch (e.g. chosen options / initial config), so
  install-time choices flow into the app without a second setup wizard.
- **`prereq.rs`** — declared prerequisites verified before install (e.g. a runtime),
  surfaced in the UI.

## 8. UI & i18n (`installer` crate, `i18n.rs`)

The `installer` binary is a **Slint** native GUI (no WebView runtime), built via
`build.rs` + `slint-build`. `main.rs` wires the flow: stage the embedded package →
detect/verify signature → show license/components → verify + extract on a worker
thread with throttled progress → shortcuts/registry → done; plus Maintenance (Repair =
re-verify + restore same version; Update = check manifest, delta or full). `i18n.rs`
provides localized strings (EN/FR), matching the bilingual docs.

## 9. Build & use (CLI)

```sh
cargo build --release -p bpkg-cli -p installer
bpkg keygen --out keys
bpkg pack  --root payload --config installer.toml --out app.bpkg
bpkg sign  --key keys/private.key app.bpkg
bpkg stamp --installer target/release/installer.exe --package app.bpkg --out Setup.exe
```

CI runs `fmt` · `clippy` · `test`. See **GUIDE.md** for the full authoring walkthrough.
