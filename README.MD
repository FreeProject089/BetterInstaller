<div align="center">

# BetterInstaller

**One engine, one `installer.toml` → a signed, self-extracting installer + auto-updater.**
A cross-platform alternative to NSIS / MSI for any desktop app — native Slint UI (no
WebView runtime), first-run config handoff, and updates with rollback.

![Platforms](https://img.shields.io/badge/platforms-Windows%20%C2%B7%20Linux%20%C2%B7%20macOS-blue?style=flat-square)
![UI](https://img.shields.io/badge/UI-Slint%20(native)-7c3aed?style=flat-square)
![Signing](https://img.shields.io/badge/signing-Ed25519-green?style=flat-square)
![CI](https://img.shields.io/badge/CI-fmt%20%C2%B7%20clippy%20%C2%B7%20test-success?style=flat-square)

🇬🇧 English · [🇫🇷 Français](README_FR.MD)
[Quick start](#quick-start) · [Documentation](#-documentation) · [Develop](#develop) · [Contributing](CONTRIBUTING.md)

</div>

---

## Quick start

```sh
# 1. Build the engine + CLI
cargo build --release -p bpkg-cli -p installer

# 2. Pack → sign → stamp your app into a single Setup.exe
bpkg keygen --out keys
bpkg pack  --root payload --config installer.toml --out app.bpkg
bpkg sign  --key keys/private.key app.bpkg
bpkg build --installer ./target/release/betterinstaller[.exe] \
           --config installer.toml --package app.bpkg --out App-Setup[.exe]
```

A complete worked example (with one-command build + release scripts) lives in
[`examples/bmm/`](examples/bmm/) — see [GUIDE.md](GUIDE.md) for every config field.

## 📚 Documentation

- **[GUIDE.md](GUIDE.md)** — configure `installer.toml` (every field).
- **[docs/](docs/README.md)** — architecture · `.bpkg` format · CLI · handoff ·
  updates · **[updater setup (GitHub / own server / multi-source)](docs/UPDATER-SETUP.md)** ·
  signing · **[cross-platform](docs/PLATFORMS.md)**.
- **[CONTRIBUTING.md](CONTRIBUTING.md)** ([FR](CONTRIBUTING_FR.md)) — setup, the CI gate, issues & PRs.

## Develop

```sh
# Run the exact CI gate locally on Linux (fmt + clippy + test + build), via Docker:
docker compose run --rm ci            # cached registry + target → fast repeat runs
docker compose run --rm shell         # interactive shell with toolchain + deps
# (or without compose:)
docker build -t betterinstaller-dev . && docker run --rm -e CARGO_TARGET_DIR=/tmp/t -v "${PWD}:/app" betterinstaller-dev
```

<sub>Design history: `../.Assets/.md/PLAN_BETTER_INSTALLER.md`.</sub>

---

## Why

- **Reusable** — the engine never hard-codes a project. Each app ships an `installer.toml`.
  Any app, in any language, can opt in (it just reads one JSON file on first launch).
- **Native & beautiful** — UI in [Slint](https://slint.dev): a single static binary, **no
  WebView2 runtime**, GPU-rendered, themeable.
- **Portable & compact** — one self-contained exe, no runtime to install. The engine alone
  is ~11 MB; a shipped setup is that plus the zstd-compressed payload (BMM: 57 MB for a
  49 MB app).
- **Secure** — per-file SHA-256, **Ed25519 signing**, integrity verified at install time.
- **First-run handoff** — the installer pre-configures the app (language, privacy/ToS,
  tutorial, telemetry, components) so there are **no first-launch modals**.
- **Full lifecycle** — install · configure · update (with rollback) · repair · uninstall.

---

## What's implemented

| Area | Status |
|---|---|
| `.bpkg` package format (build / verify / extract, roundtrip-tested) | ✅ |
| `bpkg` CLI: `pack`, `info`, `verify`, `extract`, `install`, `build`, `keygen`, `sign`, `update` | ✅ |
| Slint GUI: Welcome → Terms → Setup → Install → Done | ✅ |
| **Maintenance mode** GUI (repair / update / uninstall, confirm-gated) | ✅ |
| **Terms step**: TOS + Privacy rendered as markdown, per-doc accept | ✅ |
| **Post-install launch** (opt-in "launch now" on the Done page) | ✅ |
| **Browse** folder picker + writability preflight | ✅ |
| **Branding**: app logo + name in the sidebar (`[branding].logo`) | ✅ |
| **Themeable**: re-skin the whole installer from `[theme]` in the TOML | ✅ |
| **Pre-import**: bundle languages/themes/preset, opt-in on first run | ✅ |
| **Auto-update config** (`[update]`: remote manifest, deltas, auto-check) | ✅ |
| Action-aware Done screen (installed / updated / repaired / uninstalled) | ✅ |
| First-run handoff (`installer-handoff.json`) — installer writes, app reads once | ✅ |
| Real install (verify + extract + progress) | ✅ |
| System integration (Windows): shortcuts, custom `app://` protocol, ARP uninstaller, PATH | ✅ |
| Uninstall (`--uninstall`, reverses everything) | ✅ |
| `asInvoker` manifest (no spurious UAC prompt) | ✅ |
| **Single self-extracting installer** (`bpkg build` — config + payload in one exe) | ✅ |
| **Ed25519 signing** + verify-on-install (`[security]`) | ✅ |
| Prerequisites check + auto-download/-install of missing ones | ✅ |
| **Update with atomic rollback** (local + remote manifest) | ✅ |
| **Binary delta patches** (bsdiff — patches ~1% of the new file) | ✅ |
| i18n (en / fr) for the UI chrome | ✅ |
| Component-aware packing (`[[components]].paths` → per-file mapping) | ✅ |
| Self-deleting uninstaller | ✅ |
| Linux backend (`.desktop` shortcuts + `xdg-mime` protocol) | ✅ |
| macOS backend (symlink shortcuts + paths; protocol via Info.plist) | ✅ / partial |
| CI (GitHub Actions: fmt + clippy + test + release, Windows & Linux) | ✅ |

**Not yet** (honest, minor): macOS runtime protocol registration (it's declared in the `.app`
Info.plist, not at runtime); reactive UI language switch mid-flow. SmartScreen still needs
Authenticode (Ed25519 only covers package integrity, not Windows reputation).

---

## Workspace

```
crates/
  bpkg-core/   # the engine (library)
    src/
      package/   # .bpkg format: format / writer / reader
      platform/  # PlatformOps: windows / linux / macos
      config.rs  # installer.toml model
      manifest.rs# package manifest (app + files + components + SHA-256)
      handoff.rs # installer-handoff.json contract + builder
      sign.rs    # Ed25519 keygen / sign / verify
      embed.rs   # self-extracting stamp / read
      update.rs  # update + rollback
      prereq.rs  # prerequisite checks
      i18n.rs    # en/fr UI strings
  bpkg-cli/    # the `bpkg` packaging tool
  installer/   # the Slint GUI binary (`betterinstaller`)
    ui/main.slint
examples/
  bmm/                 # a complete, real worked example (config + build script)
.github/workflows/ci.yml
```

---

## Quick start

```sh
cargo build -p bpkg-cli -p installer

# 1. Package an app's files (app meta + components come from installer.toml)
bpkg pack  --root <app-files> --config installer.toml --out app.bpkg
bpkg info  app.bpkg
bpkg install app.bpkg --dest out/          # verify + extract with progress

# 2. Sign it (optional, recommended)
bpkg keygen --out keys/                     # → keys/private.key (secret) + keys/public.key
bpkg sign   app.bpkg --key keys/private.key
bpkg verify app.bpkg --key keys/public.key  # SHA-256 + Ed25519

# 3. Build ONE self-extracting installer (config + payload baked in)
bpkg build --installer target/debug/betterinstaller.exe \
           --config installer.toml --package app.bpkg \
           --out MyAppSetup.exe
./MyAppSetup.exe                            # double-click installs; no args needed

# 4. Update an existing install (rolls back on failure)
bpkg update app-v2.bpkg --dir <install-dir>

# 5. Uninstall (also wired to the Apps & Features entry)
<install-dir>/uninstall.exe --uninstall
```

`betterinstaller.exe` can also run with explicit args during development:
`betterinstaller <installer.toml> [package.bpkg]`.

---

## `installer.toml` reference (example)

```toml
[app]
id = "com.acme.editor"; name = "Acme Editor"; version = "1.0.0"
publisher = "BetterCommunity"; platforms = ["windows"]

[install]
default_dir = "{ProgramFiles}/Acme Editor"
main_exe = "myapp.exe"   # for shortcuts + protocol
protocol = "acme"                      # registers acme:// deep links
create_shortcuts = true; desktop_shortcut = true; allow_portable = true

[[components]]                          # selectable parts (e.g. MCP sidecar)
id = "mcp-server"; name = "MCP AI Server"; required = false; default = true; size_mb = 7

# First-run handoff → no modals on first launch
[handoff]
enabled = true; file = "installer-handoff.json"; location = "app_data"

[[setup_option]]
id = "language"; type = "select"; label_key = "setup.language"
choices = ["auto","en","fr"]; default = "auto"; maps_to = "settings.language"

[[setup_option]]
id = "legal"; type = "license"; label_key = "setup.acceptLegal"; required = true
documents = ["PRIVACY.md","TOS.md"]
maps_to = ["settings.privacy_accepted","settings.tos_accepted"]

# Enforce the package signature at install (optional)
[security]
public_key = "<hex from keys/public.key>"
require_signature = true

# Verify a runtime is present before installing (optional)
[[prerequisite]]
id = "vcredist"; name = "Visual C++ 2022"
check_registry = "HKLM\\SOFTWARE\\Microsoft\\VisualStudio\\14.0\\VC\\Runtimes\\x64"
download_url = "https://aka.ms/vs/17/release/vc_redist.x64.exe"
silent_args = "/install /quiet /norestart"; required = true
```

---

## The first-run handoff contract

The installer writes `installer-handoff.json` (settings + components) into the app's
data dir. The app reads it **once** on first launch, applies it, and renames it to
`.consumed.json`. App-agnostic — the engine never touches an app's internal config format.

```json
{ "schema": 1, "source": "betterinstaller", "app_version": "1.0.0",
  "components": ["core","mcp-server"],
  "settings": { "language": "fr", "privacy_accepted": true, "tos_accepted": true,
                "skip_tutorial": true, "telemetry": false } }
```

The app consumes it at boot (e.g. a `consume_installer_handoff` routine) → no privacy/ToS/language/
tutorial modals.

---

## Testing

```sh
cargo test --workspace      # unit tests (handoff, embed, sign, update+rollback)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

The full CLI lifecycle (pack → sign → verify → install → build → update) and a no-args
launch of a stamped self-extracting installer are verified end-to-end.

## License

[GPL 3.0](https://github.com/FreeProject089/BetterInstaller/blob/master/LICENSE)
