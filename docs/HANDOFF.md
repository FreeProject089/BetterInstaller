# First-run handoff contract

The installer pre-configures the app at install time and drops a standard
`installer-handoff.json`. The app reads it **once** on first launch, applies it, and
marks it consumed — so there are no first-run privacy/ToS/language/tutorial modals.
The installer never touches the app's private config format; this contract is
app-agnostic.

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
  "install_dir": "C:\\Users\\me\\AppData\\Local\\Programs\\Better Mods Manager",
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

### Reference implementation (BMM, Rust + TS)
- `src-tauri/src/commands/installer_handoff.rs` — `consume_installer_handoff`:
  reads the file, applies settings, copies bundled presets, marks consumed,
  returns a small result (legal accepted, language set, preset path, counts).
- `frontend/src/ui/app.ts` — calls the command before the first-run modals, sets the
  localStorage gates, and imports any bundled preset via `import_app_data`.

## Pre-import (bundled content)

The installer can bundle extra content under `<install_dir>/presets/` (the build
script copies `examples/<app>/bundle/presets/*`). When an `import_*` option stays
checked, the app copies those files in on first run:

- `presets/Lang/*.json` → the app's language dir (extra community languages).
- `presets/themes/*.json` → the app's themes dir.
- a full export `bmm-preset.json` → imported via the app's normal backup importer.

Built-in languages (en/fr for BMM) ship inside the app and are always present — the
`import_*` options only add content **beyond** the built-ins.
