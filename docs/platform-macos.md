# macOS

Per-user, no root. macOS apps are `.app` bundles, and the URL protocol is declared in
the bundle's `Info.plist` rather than registered at runtime.

## Locations

| What | Path |
|---|---|
| Default install dir | `/Applications/<app.name>.app` |
| App data (handoff) | `~/Library/Application Support/<app.id>` |
| "Shortcut" | symlink in `~/Applications` → the app bundle |
| URL protocol | declared in `<app>.app/Contents/Info.plist` (`CFBundleURLTypes`) |

## What the engine does

- **Shortcuts** — a symlink in `~/Applications` is the simplest per-user launcher.
- **Protocol** — *not* a runtime call. Add to your app's `Info.plist`:
  ```xml
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key>     <string>com.acme.editor</string>
      <key>CFBundleURLSchemes</key>  <array><string>acme</string></array>
    </dict>
  </array>
  ```
  Launch Services picks it up when the `.app` is first seen.
- **Uninstall** — remove the `.app` (and the `~/Applications` symlink);
  `uninstall-info.json` records what to undo.

## Payload tips

- The payload **is** the `.app` bundle (or its contents). Keep the standard structure:
  `<App>.app/Contents/{MacOS,Resources,Info.plist}`.
- Point `[install].main_exe` at `Contents/MacOS/<binary>`.
- `[branding].logo` can be a PNG read from the package for the installer sidebar; the
  app's own icon stays `Contents/Resources/<icon>.icns`.

## Building

```sh
cargo build --release -p bpkg-cli -p installer
bpkg pack  --root payload-macos --config installer.toml --out app-macos.bpkg
bpkg sign  --key keys/private.key app-macos.bpkg
bpkg build --installer ./target/release/betterinstaller \
           --config installer.toml --package app-macos.bpkg --out App-Setup
```

## Notarization / Gatekeeper

Ed25519 package signing ≠ Apple notarization. For distribution outside the App Store,
**codesign + notarize** the `.app` (and ideally ship it in a signed `.dmg`) so
Gatekeeper doesn't block it. That's separate from BetterInstaller's package signing.

## Gotchas

- Don't end `[app].id` with `.app` if you can avoid it — it collides with the bundle
  extension convention (you'll see a Tauri warning; it's cosmetic but rename when
  possible).
- Runtime protocol registration is **not** performed by the engine on macOS — it must
  be in the bundle's Info.plist (declarative).
