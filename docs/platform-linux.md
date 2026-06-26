# Linux

🇬🇧 English · [🇫🇷 Français](platform-linux.fr.md)

Per-user under `$HOME`, freedesktop-compliant. No root needed.

## Locations

| What | Path |
|---|---|
| Default install dir | `~/.local/opt/<app.id>` |
| App data (handoff) | `$XDG_CONFIG_HOME/<app.id>` (else `~/.config/<app.id>`) |
| App shortcut | `$XDG_DATA_HOME/applications/<name>.desktop` (else `~/.local/share/applications`) |
| Desktop shortcut | `~/Desktop/<name>.desktop` |
| URL protocol | a handler `.desktop` + `xdg-mime default …` for `x-scheme-handler/<scheme>` |

## What the engine does

- **Shortcuts** — writes `.desktop` launchers (`Exec=`, `Name=`, optional `Icon=`) in
  the applications dir (always) and Desktop (if `desktop_shortcut = true`).
- **Protocol** — drops a handler `.desktop` declaring
  `MimeType=x-scheme-handler/<scheme>;` and runs `xdg-mime default <id>.desktop
  x-scheme-handler/<scheme>` (best-effort; ignored if `xdg-mime` is absent).
- **Uninstall** — removes the install dir + the `.desktop` files (no central registry
  on Linux). `uninstall-info.json` records what to undo.

## Payload tips

- Ship a normal ELF binary (and any sidecars/resources) in `payload-linux/`.
- For an **icon**, include a PNG and point `[branding].logo` + the `.desktop` `Icon=`
  at it; for menu integration, an icon under
  `~/.local/share/icons/hicolor/…` helps some DEs.
- Mark the main binary executable in your build before packing (the engine preserves
  the `executable` flag from the manifest).

## Distribution choices

- **Self-extracting `App-Setup`** (this engine) — double-click / `./App-Setup`,
  installs to `~/.local/opt`. Simplest, matches Windows/macOS UX.
- **AppImage / Flatpak / .deb** — if you also want store/package-manager distribution,
  build those separately; BetterInstaller is the cross-distro per-user path.

## Building

```sh
cargo build --release -p bpkg-cli -p installer
bpkg pack  --root payload-linux --config installer.toml --out app-linux.bpkg
bpkg sign  --key keys/private.key app-linux.bpkg
bpkg build --installer ./target/release/betterinstaller \
           --config installer.toml --package app-linux.bpkg --out App-Setup
chmod +x App-Setup
```

## Gotchas

- Desktop environments cache `.desktop` files — a new launcher may take a moment (or
  `update-desktop-database`) to appear in the menu.
- `xdg-mime` behaviour varies by DE; the protocol registration is best-effort.
