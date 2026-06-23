# BetterInstaller — Integration Guide

How to ship **any** app with BetterInstaller, end to end. Works for Windows and
Linux apps (macOS partially). One `installer.toml`, one command, one `.exe`.

---

## 0. Build the engine once

```sh
git clone <BetterInstaller repo> && cd BetterInstaller
cargo build --release -p bpkg-cli -p installer
# → target/release/bpkg(.exe)             the packaging CLI
# → target/release/betterinstaller(.exe)  the reusable GUI shell
```

You reuse those two binaries for every project — no recompile per app.

---

## 1. Lay out your app's files

Put everything your app needs at runtime into one folder (the *payload root*):

```
myapp-payload/
  myapp.exe
  resources/...
  plugins/extra-tool.exe      # an optional component, say
```

Paths inside this folder become the install layout (forward slashes in config).

---

## 2. Write `installer.toml`

Minimal:

```toml
[app]
id = "com.example.myapp"
name = "My App"
version = "1.0.0"
publisher = "Example Inc."
platforms = ["windows"]          # or ["linux"], or both

[install]
default_dir = "{ProgramFiles}/My App"
main_exe    = "myapp.exe"         # used for shortcuts + protocol
create_shortcuts = true
desktop_shortcut = true
```

Add what you need:

```toml
# A custom URL scheme (deep links): myapp://...
[install]
protocol = "myapp"

# Optional components (the user can opt out). `paths` = which payload files
# belong to this component (prefixes). Unmatched files are always installed (core).
[[components]]
id = "extra"; name = "Extra Tool"; default = false; size_mb = 2
paths = ["plugins/extra-tool.exe"]

# Verify a runtime is present first (check_registry | check_file | check_command)
[[prerequisite]]
id = "vcredist"; name = "Visual C++ 2022"; required = true
check_registry = "HKLM\\SOFTWARE\\Microsoft\\VisualStudio\\14.0\\VC\\Runtimes\\x64"
download_url = "https://aka.ms/vs/17/release/vc_redist.x64.exe"
silent_args  = "/install /quiet /norestart"

# Enforce the package signature (see step 4)
[security]
public_key = "<hex from public.key>"
require_signature = true
```

### First-run handoff (the headline feature)

Let the installer pre-configure your app so it shows **no first-launch modals**:

```toml
[handoff]
enabled = true
file = "installer-handoff.json"
location = "app_data"            # or "install_dir" for portable apps

[[setup_option]]
id = "language"; type = "select"; label_key = "setup.language"
choices = ["auto","en","fr"]; default = "auto"
maps_to = "settings.language"

[[setup_option]]
id = "legal"; type = "license"; label_key = "setup.acceptLegal"; required = true
maps_to = ["settings.privacy_accepted","settings.tos_accepted"]

[[setup_option]]
id = "telemetry"; type = "bool"; label_key = "setup.telemetry"; default = false
maps_to = "settings.telemetry"
```

`type` is `select` (dropdown), `bool` (toggle), or `license` (an "I accept" gate that
blocks **Install** until checked when `required`). `maps_to` may list several keys.

---

## 3. Make your app consume the handoff

The installer writes `installer-handoff.json` into the app data dir (or install dir).
Your app reads it **once** on first launch and applies it. The contract:

```json
{ "schema": 1, "source": "betterinstaller", "app_version": "1.0.0",
  "components": ["core","extra"],
  "settings": { "language": "fr", "privacy_accepted": true, "tos_accepted": true,
                "telemetry": false } }
```

On startup, before any first-run UI:
1. Look for `installer-handoff.json` next to your config.
2. If present (and `source == "betterinstaller"`), apply each `settings.*` to your own
   config, **validating every value** (don't trust it blindly).
3. Rename it to `installer-handoff.consumed.json` so it never re-applies.

> BMM does exactly this in Rust (`consume_installer_handoff`), then the frontend skips
> its EULA/privacy/language modals. See `src-tauri/src/commands/installer_handoff.rs`.

---

## 4. Package → (sign) → build one installer

```sh
# Package the payload (component-aware via [[components]].paths)
bpkg pack --root myapp-payload --config installer.toml --out myapp.bpkg
bpkg verify myapp.bpkg

# Sign it (optional but recommended). Keep private.key secret — never commit it.
bpkg keygen --out keys/
bpkg sign   myapp.bpkg --key keys/private.key
#   then paste keys/public.key's hex into [security].public_key

# Stamp config + package into a single self-extracting installer
bpkg build --installer target/release/betterinstaller.exe \
           --config installer.toml --package myapp.bpkg --out MyAppSetup.exe
```

Ship `MyAppSetup.exe`. Double-clicking it:
1. checks prerequisites,
2. shows Welcome → Setup (your handoff options) → Progress → Done,
3. verifies the signature, extracts (verified) into the chosen folder,
4. creates shortcuts + protocol handler, registers the uninstaller,
5. writes `installer-handoff.json`.

No UAC prompt (the `asInvoker` manifest is embedded), no WebView runtime.

---

## 5. Update & uninstall

```sh
# Update from a local package (atomic rollback on failure)
bpkg update myapp-v2.bpkg --dir "C:\Program Files\My App"

# Update from a remote manifest (downloads the new .bpkg, or a tiny delta patch)
bpkg fetch-update --url https://example.com/myapp/update.json --dir "C:\Program Files\My App" --current 1.0.0

# Ship small delta patches between versions (≈1% of the full package)
bpkg delta --old myapp-v1.bpkg --new myapp-v2.bpkg --out v1-to-v2.patch
```

`update.json` looks like:
```json
{ "version": "1.1.0", "url": "https://example.com/myapp/myapp-1.1.0.bpkg",
  "deltas": [ { "from": "1.0.0", "url": "https://example.com/myapp/v1-to-v2.patch" } ] }
```

Uninstall is wired to the **Apps & Features** entry, or run it directly:

```sh
"C:\Program Files\My App\uninstall.exe" --uninstall
```

It removes shortcuts, unregisters the protocol + ARP entry, deletes the install dir,
and self-deletes the uninstaller.

---

## Notes & limits (honest)

- **SmartScreen**: Ed25519 guarantees package integrity but does *not* remove the Windows
  SmartScreen warning — that needs Authenticode (paid cert, or SignPath OSS / Azure
  Trusted Signing).
- **Linux**: shortcuts (`.desktop`) and protocol (`xdg-mime`) work; package via
  `platforms = ["linux"]`. **macOS**: paths work, shortcuts are symlinks, protocol is
  declared in the `.app` Info.plist (not at runtime).
- **Updates**: local + remote (manifest URL) apply with atomic rollback, and binary *delta*
  patches (bsdiff, ≈1% of the full package) are all implemented. You host `update.json` + the
  `.bpkg`/patches; the installer/app calls `fetch-update`.
- **Uninstall** removes what the install created/declared; files an app writes at runtime
  outside the install dir are the app's responsibility (same as NSIS/MSI).
- **Admin rights**: the installer is `asInvoker` (no UAC), so it installs **per-user** by
  default (`%LOCALAPPDATA%\Programs\<name>`, writable without admin). Installing into
  `C:\Program Files` needs an **elevated** installer — the GUI shows a clear error if you
  pick a folder you can't write to. (Per-machine/elevated builds are on the roadmap.)

---

## Appendix A — `bpkg` command reference

| Command | What it does |
|---|---|
| `bpkg pack --root <dir> --config <toml> --out <bpkg>` | Build a `.bpkg` from a folder (component-aware via `[[components]].paths`). |
| `bpkg info <bpkg>` | Print app metadata + components. |
| `bpkg verify <bpkg> [--key <public.key>]` | Verify every file's SHA-256 (and the Ed25519 signature with `--key`). |
| `bpkg extract <bpkg> --dest <dir> [--components a,b]` | Extract files (optionally only some components). |
| `bpkg install <bpkg> --dest <dir> [--components a,b]` | Verify + extract with a progress bar (the GUI's path). |
| `bpkg keygen --out <dir>` | Generate an Ed25519 keypair (`private.key` + `public.key`). |
| `bpkg sign <bpkg> --key <private.key>` | Sign a package in place. |
| `bpkg build --installer <exe> --config <toml> --package <bpkg> --out <Setup.exe>` | Stamp config + package into one self-extracting installer. |
| `bpkg update <bpkg> --dir <install>` | Update an install from a local package (atomic rollback). |
| `bpkg fetch-update --url <manifest> --dir <install> --current <ver>` | Check a remote manifest, download (delta if available) + apply. |
| `bpkg delta --old <bpkg> --new <bpkg> --out <patch>` | Create a binary delta patch (≈1% of the new file). |
| `bpkg apply-delta --old <bpkg> --patch <patch> --out <bpkg>` | Reconstruct the new file from old + patch. |

The GUI binary: `betterinstaller <installer.toml> [package.bpkg]` (dev), `betterinstaller --uninstall`
(reverse an install), or a stamped `Setup.exe` (no args — uses the embedded blob).

## Appendix B — `installer.toml` reference

**`[app]`** — `id` (reverse-DNS), `name`, `version`, `publisher`, `homepage?`, `platforms` (`["windows"|"linux"|"macos"]`).

**`[branding]`** — `accent?` (hex color), `logo?`, `background?`.

**`[install]`** — `default_dir?` (placeholders: `{ProgramFiles}`, …), `main_exe?` (relative, for shortcuts/protocol), `protocol?` (URL scheme), `create_shortcuts` (bool), `desktop_shortcut` (bool), `allow_portable` (bool).

**`[[components]]`** — `id`, `name`, `description?`, `required` (bool), `default` (bool), `size_mb`, `paths` (payload prefixes that belong to this component; unmatched files = core).

**`[handoff]`** — `enabled` (bool), `file` (default `installer-handoff.json`), `location` (`app_data`|`install_dir`).

**`[[setup_option]]`** — `id`, `type` (`select`|`bool`|`license`), `label?` (display text), `label_key` (i18n key), `description?` (one-line transparency text under the label), `choices` (for `select`), `default` (JSON value), `documents` (for `license`), `required` (bool), `maps_to` (one key or a list → `settings.*` in the handoff).

**`[security]`** — `public_key?` (hex Ed25519; enforce the package signature), `require_signature` (bool).

**`[[prerequisite]]`** — `id`, `name`, one of `check_registry` / `check_file` / `check_command`, `download_url?` + `silent_args?` (auto-install when missing), `required` (bool).

## Appendix C — `bpkg-core` modules

| Module | Responsibility |
|---|---|
| `package` | `.bpkg` format: `format` (header), `writer` (`create_from_dir`), `reader` (`Package`: verify / extract / `install_with_progress` / signature). |
| `manifest` | The in-package JSON manifest (app + files + components + SHA-256). |
| `config` | `installer.toml` model + parsing. |
| `handoff` | `installer-handoff.json` contract + `build()` from setup options. |
| `sign` | Ed25519 keygen / sign / verify (hex keys). |
| `embed` | Self-extracting stamp / read (trailer magic `BPKGSFX1`). |
| `update` | Local + remote apply with atomic rollback; `UpdateManifest`, `check_remote`, `download_and_apply`, `is_newer`. |
| `delta` | bsdiff `make_delta` / `apply_delta` (via `qbsdiff`). |
| `net` | Blocking HTTP (`fetch_text` / `download`, rustls). |
| `prereq` | `check` / `ensure_required` / `auto_install`. |
| `platform` | `PlatformOps` trait + `windows` / `linux` / `macos` backends. |
| `i18n` | Compiled-in en/fr UI strings (`t`, `detect_lang`). |
