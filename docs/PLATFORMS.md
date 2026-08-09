# Cross-platform packaging

🇬🇧 English · [🇫🇷 Français](https://github.com/FreeProject089/BetterInstaller/blob/master/docs/PLATFORMS.fr.md)

**One `installer.toml`, one engine, three OSes.** The engine is written once against a
`PlatformOps` trait; each OS has a backend that does shortcuts / protocol / uninstall
the native way. You ship a **per-OS payload + a per-OS engine binary**, but the config
and the whole flow are identical.

| | Windows | Linux | macOS |
|---|---|---|---|
| Engine binary | `betterinstaller.exe` | `betterinstaller` | `betterinstaller` |
| Setup output | `App-Setup.exe` | `App-Setup` (or AppImage) | `App-Setup` (or `.app`/`.dmg`) |
| Default install dir | `%LOCALAPPDATA%\Programs\<name>` | `~/.local/opt/<id>` | `/Applications/<name>.app` |
| App data dir | `%APPDATA%\<id>` | `~/.config/<id>` | `~/Library/Application Support/<id>` |
| Shortcuts | `.lnk` (Start Menu / Desktop) | `.desktop` (`~/.local/share/applications`) | symlink in `~/Applications` |
| URL protocol | `HKCU\Software\Classes\<scheme>` | `.desktop` + `xdg-mime` | `.app` Info.plist `CFBundleURLTypes` |
| Uninstall entry | ARP (Apps & Features) | — (remove dir + `.desktop`) | — (remove `.app`) |
| Elevation | per-user, `asInvoker` (no UAC) | per-user (`~`) | per-user (`~`) |

Per-OS deep dives: **[Windows](platform-windows.md) · [Linux](platform-linux.md) ·
[macOS](platform-macos.md)**.

## What's the same everywhere

- The **`installer.toml`** (one file). `[app].platforms` lists what you target.
- The **`.bpkg`** format, signing, manifest, components, handoff, update logic.
- The **GUI flow** (Welcome → Terms → Setup → Install → Done) and maintenance mode.
- The first-run **handoff** contract (the app reads `installer-handoff.json` once).

## What differs (and is handled for you)

Only the OS-mutating steps — shortcuts, the URL protocol, the uninstall registration,
the default install/data dirs, and elevation. The `PlatformOps` backend for the OS the
binary was built for is selected automatically (`platform::current()`); your config and
payload don't branch on OS in code.

## Building for each OS

Build the engine **on each target OS** (or cross-compile), then pack + stamp there:

```sh
# on each OS:
cargo build --release -p bpkg-cli -p installer        # → bpkg + betterinstaller(.exe)

bpkg pack  --root payload-<os> --config installer.toml --out app-<os>.bpkg
bpkg sign  --key keys/private.key app-<os>.bpkg
bpkg build --installer ./target/release/betterinstaller[.exe] \
           --config installer.toml --package app-<os>.bpkg --out App-Setup-<os>[.exe]
```

- **Payload per OS** — the *contents* differ (a Windows `.exe` vs a Linux ELF vs a
  macOS `.app`), so keep a `payload-windows/`, `payload-linux/`, `payload-macos/`.
- **Same key** — sign all OS packages with the same Ed25519 key if you want one
  `[security].public_key` to verify them all.
- **`[[components]].paths`** are forward-slash, OS-agnostic.

> A single CI matrix (windows-latest / ubuntu-latest / macos-latest) can produce all
> three setups from the same repo — see [../.github/workflows/ci.yml](https://github.com/FreeProject089/BetterInstaller/blob/master/.github/workflows/ci.yml).

## Tips for an app that targets all three

1. Keep `[app].platforms = ["windows", "linux", "macos"]`.
2. `main_exe` / `protocol` are the same logical names; the backend maps them natively.
3. The app's first-run **handoff reader** must use that OS's data dir (the installer
   already writes to the right place per OS). Most toolkits (e.g. Tauri) give you the
   per-OS data dir for free.
4. Test the URL protocol on each OS — it's the most OS-specific piece (registry vs
   `xdg-mime` vs Info.plist).
