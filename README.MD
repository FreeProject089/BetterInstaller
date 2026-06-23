# BetterInstaller

A proprietary, cross-platform **installer / updater framework** — one engine, one
`installer.toml` per project. Built to replace NSIS / MSI for BMM and any other app,
and reusable by anyone for any app (Windows / Linux / macOS).

> Design doc: `../.Assets/.md/PLAN_BETTER_INSTALLER.md` (v2 body + **v3 Addendum**).

---

## Why

- **Reusable** — the engine never hard-codes a project. Each app ships an `installer.toml`.
  Any app, in any language, can opt in (it just reads one JSON file on first launch).
- **Native & beautiful** — UI in [Slint](https://slint.dev): a single static binary, **no
  WebView2 runtime**, GPU-rendered, themeable.
- **Portable & small** — one self-contained exe; release profile targets `<5 MB`; zstd payload.
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
| Slint GUI: Welcome → Setup → Progress → Done | ✅ |
| First-run handoff (`installer-handoff.json`) — installer writes, app reads once | ✅ |
| Real install (verify + extract + progress) | ✅ |
| System integration (Windows): shortcuts, `bmm://` protocol, ARP uninstaller, PATH | ✅ |
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
  bmm/installer.toml   # Better Mods Manager reference config
.github/workflows/ci.yml
```

---

## Quick start

```sh
cargo build -p bpkg-cli -p installer

# 1. Package an app's files (app meta + components come from installer.toml)
bpkg pack  --root <app-files> --config examples/bmm/installer.toml --out app.bpkg
bpkg info  app.bpkg
bpkg install app.bpkg --dest out/          # verify + extract with progress

# 2. Sign it (optional, recommended)
bpkg keygen --out keys/                     # → keys/private.key (secret) + keys/public.key
bpkg sign   app.bpkg --key keys/private.key
bpkg verify app.bpkg --key keys/public.key  # SHA-256 + Ed25519

# 3. Build ONE self-extracting installer (config + payload baked in)
bpkg build --installer target/debug/betterinstaller.exe \
           --config examples/bmm/installer.toml --package app.bpkg \
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

## `installer.toml` reference (BMM example)

```toml
[app]
id = "com.bettermm.app"; name = "Better Mods Manager"; version = "1.0.0"
publisher = "BetterCommunity"; platforms = ["windows"]

[install]
default_dir = "{ProgramFiles}/Better Mods Manager"
main_exe = "better-mods-manager.exe"   # for shortcuts + protocol
protocol = "bmm"                        # registers bmm:// deep links
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

BMM consumes it Rust-side at boot (`consume_installer_handoff`) → no privacy/ToS/language/
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

GPL 3.0
