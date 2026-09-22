# BetterInstaller — Complete Configuration Guide

🇬🇧 English · [🇫🇷 Français](GUIDE_FR.md)

BetterInstaller turns **one `installer.toml` + one payload folder** into a single,
signed, self-extracting `*-Setup.exe` (no NSIS/MSI, no WebView runtime — a native
Slint GUI in one binary). It also handles **first-run configuration handoff**,
**auto-update with rollback**, and a **maintenance mode** (repair / update /
uninstall).

> *"Un TOML pour les gouverner tous."* One config drives the whole installer.

---

## 1. Concepts in 30 seconds

| Piece | What it is |
|---|---|
| **`bpkg`** | The CLI: `pack`, `sign`, `build`, `update`, `keygen`, … |
| **`.bpkg`** | The package: a signed, zstd-compressed archive + JSON manifest. |
| **`betterinstaller.exe`** | The GUI engine (one per platform). |
| **`*-Setup.exe`** | `betterinstaller.exe` + your config + your `.bpkg` stamped into one file. |
| **`installer.toml`** | Everything project-specific (this guide). |
| **`installer-handoff.json`** | Written at install; the app reads it once on first launch. |

**Pipeline:** `payload/` → `bpkg pack` → `.bpkg` → `bpkg sign` → `bpkg build` → `*-Setup.exe`.

---

## 2. Quick start

```sh
# 0. Build the engine once
cargo build --release -p bpkg-cli -p installer

# 1. Generate a signing keypair (keep private.key SECRET, never commit it)
./target/release/bpkg keygen --out keys
#   → copy keys/public.key into [security].public_key in installer.toml

# 2. Assemble payload/  (your app exe + sidecars + TOS.md/PRIVACY.md + bundle/)
# 3. Pack + sign + stamp
./target/release/bpkg pack  --root payload --config installer.toml --out app.bpkg
./target/release/bpkg sign  --key keys/private.key app.bpkg
./target/release/bpkg build --installer ./target/release/betterinstaller.exe \
                            --config installer.toml --package app.bpkg --out App-Setup.exe
```

A complete worked example lives in `examples/` — its build script automates every
step above (assemble payload → pack → sign → stamp → emit `update.json`):

```powershell
./examples/<app>/build-installer.ps1     # from the BetterInstaller root
```

---

## 3. `installer.toml` reference

### `[app]` — identity (required)

```toml
[app]
id        = "com.acme.editor"   # MUST equal the app's data-dir identifier *
name      = "Acme Editor"
version   = "1.0.0"
publisher = "BetterCommunity"
homepage  = "https://…"          # optional
platforms = ["windows"]          # windows | linux | macos
```

\* **Critical:** `id` must match what the app uses for its per-user data dir
(the identifier your app uses for that dir). The handoff file is written to `%APPDATA%/<id>/`; if `id`
is wrong, the app never finds it and **none of the first-run settings apply**.

### `[branding]`

```toml
[branding]
accent     = "#3b82f6"   # installer accent color
logo       = "assets/logo.png"
background = "assets/installer-bg.png"
```

### `[install]`

```toml
[install]
main_exe         = "myapp.exe"  # for shortcuts + protocol
protocol         = "acme"                       # registers acme:// deep links
create_shortcuts = true
desktop_shortcut = true
```

> **Install location:** there is no `default_dir` setting. The installer ships an
> `asInvoker` manifest (no UAC prompt) and always proposes the per-user root
> `%LOCALAPPDATA%\Programs\<name>`, which it can write to without elevation. The
> **Browse…** button lets the user pick anywhere else; `C:\Program Files` needs an
> elevated build, and the GUI shows a clear error if the chosen folder isn't writable.

### `[security]` — package signing (recommended)

```toml
[security]
public_key        = "8e0647…168b"   # hex Ed25519 public key (from keygen)
require_signature = true             # refuse to install an unsigned/invalid pkg
```

The Welcome page shows a trust badge:
- **`Signed & verified · <publisher>`** — signature valid against `public_key`.
- **`Unsigned package · …`** — no signature (badge is red if `require_signature`).
- **`Signature INVALID`** — tampered package; install is blocked.

Generate + use a key:
```sh
bpkg keygen --out keys           # → keys/private.key (SECRET), keys/public.key
bpkg sign   --key keys/private.key app.bpkg
bpkg verify app.bpkg --key keys/public.key   # sanity check
```

### `[update]` — auto-update (optional)

```toml
[update]
manifest_url = "https://…/update.json"   # a stable JSON URL you control
auto_check   = true                        # check when maintenance opens
allow_delta  = true                        # prefer a small binary patch
```

The **manifest** is JSON:
```json
{
  "version": "1.2.0",
  "url": "https://…/App-1.2.0.bpkg",
  "deltas": [{ "from": "1.1.0", "url": "https://…/1.1.0-to-1.2.0.patch" }]
}
```

When the app is already installed and you re-run the setup (or it's opened via the
ARP entry), BetterInstaller checks the manifest in the background. If it advertises
a newer version, the **Update** button appears and downloads + applies it (using a
delta from the installed version when offered), with **automatic rollback** on any
failure. Create deltas with `bpkg delta old.bpkg new.bpkg patch`.

If `[update]` is omitted, the **Update** button still appears when the *bundled*
setup is newer than what's installed (it re-extracts the embedded package).

### `[[components]]` — optional install pieces

```toml
[[components]]
id          = "core"
name        = "Acme Editor"
description = "Main application — required."
required    = true       # always installed, checkbox disabled
default     = true        # pre-checked
size_mb     = 43

[[components]]
id          = "mcp-server"
name        = "MCP AI Server (sidecar)"
required    = false
default     = true
size_mb     = 7
paths       = ["acme-helper.exe", "mcp/"]   # payload paths owned by this component
```

`paths` are forward-slash prefixes; files matching none belong to `core` and are
always installed. Unchecked optional components are skipped at extraction.

### `[handoff]` — first-run configuration (the headline feature)

```toml
[handoff]
enabled  = true
file     = "installer-handoff.json"
location = "app_data"    # app_data (per-user) | install_dir (portable)
```

Writes a flat settings file the app reads **once** on first launch, then renames to
`*.consumed.json`. This is what removes first-run privacy/ToS/language/tutorial
modals. See §4 for the app side.

### `[[setup_option]]` — the Configuration page

Each option renders a control and maps to one or more handoff settings keys.

```toml
[[setup_option]]
id        = "language"
type      = "select"             # bool | select | license
label     = "Language"
description = "Interface language."
choices   = ["auto", "en", "fr"] # select only
default   = "auto"
maps_to   = "settings.language"  # one key, or ["k1","k2"]
```

- **`bool`** → a checkbox → JSON bool.
- **`select`** → a dropdown → JSON string. The dropdown shows translated labels and
  writes the value from `choices`. **`"auto"` is special:** the installer resolves it to
  the language the installer is shown in (the OS language unless the user picked another
  one), taking the first of its fallback chain found in `choices`, so leaving the default
  still gives the app a concrete value (this fixes "selects not applied").
- **`license`** → with `documents = ["TOS.md","PRIVACY.md"]` it becomes a dedicated
  **Terms** step: each document is rendered (markdown) on its **own page with its own
  Accept checkbox**, and all must be accepted to proceed. Maps its acceptance to
  every `maps_to` key (e.g. `tos_accepted` + `privacy_accepted`). Translated versions go
  in `localized_documents` (see [Languages](#languages-i18n-and-setup_group)).

Also on every option: `group = "<id>"` lists it under a `[[setup_group]]` heading, and
`sends_data = true` adds a "Sends data" mark next to its label. Every row shows its
default ("Default: On") and a "Changed" mark once the user moves away from it; a
"Restore defaults" button resets the page.

`required = true` blocks **Next/Install** until satisfied. `maps_to` keys are written
flat into `settings` after stripping a leading `settings.` prefix.

**Pre-import options** are just `bool` options whose key the app honors:
```toml
[[setup_option]]
id = "import_themes"
type = "bool"
label = "Import the starter theme pack"
default = true                          # checked, but the user can uncheck → no import
maps_to = "settings.import_starter_themes"
```

### Languages: `[i18n]` and `[[setup_group]]`

The installer opens in the OS language (Windows UI language; `LC_ALL`/`LC_MESSAGES`/`LANG`
elsewhere), or the one given with `--lang=<tag>`, and offers a language picker in the
sidebar until the install starts. Switching re-words every screen at once, including the
legal documents; a document whose text changed must be accepted again.

```toml
[i18n]
default     = "auto"               # or a tag such as "fr" / "pt-BR" to force it
locales_dir = "installer-locales"  # folder INSIDE the package: <code>.toml per language

[[setup_group]]                    # headings on the Configuration page, in order
id          = "privacy"
label       = "Privacy & data"
description = "What the app may send off this PC."

[[setup_option]]
id = "legal"
type = "license"
documents = ["TOS.md", "PRIVACY.md"]          # English, and the fallback
localized_documents = [
  { lang = "fr", documents = ["TOS_FR.md", "PRIVACY_FR.md"] },   # same order
]
```

**Adding a language is adding files, never code:**

- The installer's own strings (buttons, steps, messages) are in the engine catalogues,
  `crates/bpkg-core/locales/<code>.toml`, embedded at build time: a new file there is a
  new language after a rebuild.
- Your product's strings go in `<locales_dir>/<code>.toml` inside the package, loaded at
  startup with no engine rebuild. A product catalogue can also override engine strings or
  bring a language the engine does not have. Keys:
  `options.<id>.label` · `options.<id>.description` · `options.<id>.choices.<value>` ·
  `groups.<id>.label` · `groups.<id>.description` · `components.<id>.name` ·
  `components.<id>.description` · `launch.<id>.label` · `prereqs.<id>.name` ·
  `docs.<stem>` (title of a legal document). English lives in installer.toml itself; the
  one thing it has no field for is a `select` choice label, which goes in `en.toml`.
- Each catalogue starts with `[meta] name = "Français"` (shown in the picker) and may set
  `direction = "rtl"`.

**Fallback chain:** requested tag → its shorter forms → English (`pt-BR` → `pt` → `en`),
per string. For legal documents, at each step: the `localized_documents` entry, then a
`<NAME>_<TAG>.<ext>` sibling in the package (`TOS_FR.md`), then the English `documents`.
When a document falls back to another language than the one on screen, the Terms page
says so above it.

**Right-to-left:** a catalogue with `direction = "rtl"` right-aligns text. The layout is
not mirrored, and RTL glyph shaping depends on the Slint renderer; no RTL catalogue has
been tested yet.

**Signature:** installer.toml (so `[i18n]`, `group`, `sends_data`, `localized_documents`)
is appended to the installer OUTSIDE the signed package, like every other config field.
The catalogues and the documents themselves are files in the package and are covered by
its Ed25519 signature. The installer does not read them from a package whose signature is
known to be invalid, and the install is refused anyway before any file is written.

`bpkg-core`'s tests check the engine catalogues against English, and the BMM example
checks its product catalogues against every key its installer.toml declares
(`InstallerConfig::translatable_keys`).

### `[[launch]]` — post-install "Launch now" (Done page)

```toml
[[launch]]
id = "app"
label = "Launch Acme Editor"
exe = "myapp.exe"   # relative to the install dir
default = true                     # pre-checked

[[launch]]
id = "mcp"
label = "Start the MCP AI server now"
exe = "acme-helper.exe"
default = false                    # opt-in
component = "mcp-server"           # only offered if this component was installed
```

On the final page these appear as opt-in checkboxes. **Finish** launches whatever is
checked (detached) and closes; with nothing checked it just closes.

---

## 4. The first-run handoff (app side)

The installer writes (to `%APPDATA%/<app.id>/installer-handoff.json`):

```json
{
  "schema": 1,
  "source": "betterinstaller",
  "app_version": "1.0.0",
  "components": ["core", "mcp-server"],
  "install_dir": "C:\\Users\\me\\AppData\\Local\\Programs\\Acme Editor",
  "settings": {
    "language": "fr",
    "tos_accepted": true,
    "privacy_accepted": true,
    "skip_tutorial": false,
    "telemetry": false,
    "import_starter_themes": true
  }
}
```

The app must, **once** on first launch:
1. Read the file from its own data dir, validate `source == "betterinstaller"`.
2. Apply `settings` to its own config (clamp/validate every value).
3. Rename it to `installer-handoff.consumed.json` so it never re-applies.

A typical app reads it in its own startup code (validate → apply → rename). See the
worked example under `examples/` for one concrete implementation.

### Pre-import presets (ship ready-made content)

If your app can import its own export/backup file, you can ship one so a fresh install
starts pre-configured. Drop the file in your payload's `bundle/` folder; the build
bundles `bundle/*` into the install dir. Tie it to a `[[setup_option]]` of kind
`import_*`: when the user leaves that option checked, the handoff returns the bundled
file's path and your app imports it on first run; unchecked → nothing is imported.

> The `examples/` config wires this end-to-end (a checkbox that imports a bundled
> settings/theme/language preset) — copy its `bundle/` layout as a starting point.

---

## 5. Maintenance mode (repair / update / uninstall)

When the app is already installed (detected via the ARP registry entry), or the setup
is launched with `--uninstall` (the Windows "Uninstall" button does this), the GUI opens
in **maintenance mode** with three actions, each behind a confirmation with **Cancel**:

- **Repair** — re-verify (SHA-256) and restore the same version.
- **Update** — only shown when a newer version exists (remote manifest or newer bundled
  package); downloads/extracts with rollback.
- **Uninstall** — reverses shortcuts/protocol/ARP entry and removes the install dir.

---

## 6. CLI reference (`bpkg`)

| Command | Purpose |
|---|---|
| `pack --root <dir> --config <toml> --out <pkg>` | Build a `.bpkg` from a folder. |
| `sign --key <private.key> <pkg>` | Sign a package in place (Ed25519). |
| `verify <pkg> [--key <public.key>]` | Check hashes (+ signature). |
| `keygen --out <dir>` | Generate `private.key` + `public.key`. |
| `build --installer <exe> --config <toml> --package <pkg> --out <Setup.exe>` | Stamp the SFX. |
| `info <pkg>` / `extract <pkg> --dest <dir>` | Inspect / unpack. |
| `install <pkg> --dest <dir>` | Same path the GUI uses (verify + extract + progress). |
| `update / fetch-update` | Apply a newer package / check a remote manifest. |
| `delta old new patch` / `apply-delta` | Binary delta patches. |

---

## 7. Cross-platform notes

- **Windows:** HKCU shortcuts, protocol, ARP entry; per-user install (no UAC).
- **Linux:** `.desktop` files, `xdg-mime` protocol.
- **macOS:** `Info.plist` protocol, `/Applications` symlink.

The engine is written once against a `PlatformOps` trait; each OS provides a backend.

**How an existing install is recognised.** Windows reads its own ARP entry under
`HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\<app_id>`. Linux and macOS have no
registry, so they write an **install receipt** — one JSON per app under the user's data
directory (`$XDG_DATA_HOME` or `~/.local/share/betterinstaller/receipts` on Linux,
`~/Library/Application Support/BetterInstaller/receipts` on macOS) holding the app id, the
installed version and the install directory.

This is what makes maintenance mode reachable at all. Without it the installer cannot tell an
existing install from a fresh one, so it offers neither Update nor Repair nor Uninstall, and
an update has no version to compare against.

- Delete the receipt by hand and the next run treats the app as not installed.
- Delete the install *directory* but leave the receipt and the receipt is ignored — it is
  always checked against the disk, so Repair is never offered on nothing.

Receipts are per-user, matching where these installers actually put things: they ship an
`asInvoker` manifest and never elevate.

---

## 8. Checklist for a new app

1. Copy `examples/bmm/` as a template; edit `installer.toml` (`[app].id` first!).
2. `bpkg keygen` → paste `public.key` into `[security].public_key`.
3. Put your built binaries + `TOS.md`/`PRIVACY.md` (+ optional `bundle/`) in the payload.
4. Implement the handoff reader in your app (apply settings once, mark consumed).
5. `pack → sign → build` (or copy `build-installer.ps1`).
6. Host an `update.json` if you want auto-update; set `[update].manifest_url`.
