# First-run handoff contract

🇬🇧 English · [🇫🇷 Français](https://github.com/FreeProject089/BetterInstaller/blob/master/docs/HANDOFF.fr.md)

The installer pre-configures the app at install time and drops a standard
`installer-handoff.json`. The app reads it **once** on first launch, applies it, and
marks it consumed — so there are no first-run privacy/ToS/language/tutorial modals.
The installer never touches the app's private config format; this contract is
app-agnostic.

> ## ⚠️ The handoff is NOT automatic
> BetterInstaller only **writes the JSON file**. Nothing happens to your app unless
> **you add a few lines to your app** that read that file on first launch and apply it.
> No reader = the file just sits there, ignored. The whole integration is:
> **1)** declare `[[setup_option]]`s in `installer.toml` (what the Setup step asks),
> **2)** write a small reader in your app (what to do with the answers).
> That's it — see [§ Make your own](#make-your-own-3-steps) below.

## What the installer writes

Location: `[handoff].location` → `app_data` (`%APPDATA%/<app.id>/`) or `install_dir`.
File name: `[handoff].file` (default `installer-handoff.json`).

```json
{
  "schema": 1,
  "source": "betterinstaller",
  "installer_version": "0.1.0",
  "app_version": "1.0.0",
  "installed_at": "2026-06-24T10:57:00Z",
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

`settings` is built from `[[setup_option]]` entries: each option's value is written
to its `maps_to` key(s) with the leading `settings.` stripped. A `select` left on
the `"auto"` sentinel is resolved to the detected OS value first (so the app always
gets a concrete choice).

## What the app must do (once, on first launch)

1. Read the file from its own data dir; ignore if `source != "betterinstaller"`.
2. Apply `settings` to its own config — **validate/clamp every value**, never trust
   blindly. Unknown keys are ignored.
3. Rename it to `installer-handoff.consumed.json` so it never re-applies.

### Reference shape (any Tauri / Electron / native app)
- A backend routine (e.g. `consume_installer_handoff`): reads the file, applies
  settings, copies bundled presets, marks consumed, returns a small result (legal
  accepted, language set, preset path, counts).
- A frontend hook called before your first-run modals: sets the "already onboarded"
  gates and imports any bundled preset via your normal import path.

> The bundled example under `examples/` implements exactly this end-to-end — use it
> as a template for your own app.

## Make your own (3 steps)

### 1. Decide what the Setup step asks — `[[setup_option]]`

Each block adds one control to the installer's Setup page and one entry to the handoff
`settings`. Add the ones you want, delete the rest — there are no required options.

Each block needs `id`, `type` (`bool` | `select` | `license`), a `label` (or
`label_key`), and `maps_to`. Full field reference is in [../GUIDE.md](https://github.com/FreeProject089/BetterInstaller/blob/master/GUIDE.md).

```toml
# A yes/no toggle → bool
[[setup_option]]
id          = "telemetry"
type        = "bool"
label       = "Send anonymous usage stats"
description = "Opt-in. No personal data."   # shown under the label (transparency)
default     = false
maps_to     = "settings.telemetry"          # → settings.telemetry in the JSON

# A dropdown → string
[[setup_option]]
id          = "language"
type        = "select"
label       = "Language"
choices     = ["auto", "en", "fr"]          # "auto" → resolved to the detected OS language
default     = "auto"
maps_to     = "settings.language"

# A legal gate (required = blocks Next until accepted); docs read from the package
[[setup_option]]
id          = "legal"
type        = "license"
label       = "Terms of Service & Privacy"
documents   = ["TOS.md", "PRIVACY.md"]
required    = true
maps_to     = ["settings.tos_accepted", "settings.privacy_accepted"]  # one option → several keys
```

> Want a fixed install with **no questions**? Declare zero `[[setup_option]]`s — the
> Setup step is skipped and the handoff still records version/components/install_dir.

### 2. Read the file in your app (the part that is *not* automatic)

Pseudo-code — adapt to your language:

```ts
const path = join(appDataDir(), "installer-handoff.json");
if (exists(path)) {
  const h = JSON.parse(read(path));
  if (h.source === "betterinstaller") {
    if (h.settings.language) setLanguage(validateLang(h.settings.language));
    if (h.settings.tos_accepted) markOnboarded();      // skip your first-run modals
    applyTelemetry(!!h.settings.telemetry);
    // …apply only the keys you defined; ignore the rest…
    rename(path, path.replace(".json", ".consumed.json")); // never re-apply
  }
}
```

Call this **before** your own first-run/onboarding UI so it can suppress it.

### 3. (Optional) bundle ready-made content

See [Pre-import](#pre-import-bundled-content) below — drop files in your payload's
`bundle/` and gate them behind an `import` option.

## Pre-import (bundled content)

The installer can bundle extra content under `<install_dir>/presets/` (the build
script copies `examples/<app>/bundle/presets/*`). When an `import_*` option stays
checked, the app copies those files in on first run:

- `presets/Lang/*.json` → the app's language dir (extra community languages).
- `presets/themes/*.json` → the app's themes dir.
- a full settings export (your app's backup file) → imported via the app's normal backup importer.

Built-in languages ship inside the app and are always present — the
`import_*` options only add content **beyond** the built-ins.
