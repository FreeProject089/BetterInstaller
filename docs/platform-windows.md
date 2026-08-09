# Windows

🇬🇧 English · [🇫🇷 Français](https://github.com/FreeProject089/BetterInstaller/blob/master/docs/platform-windows.fr.md)

The default + most complete backend. **Per-user** (HKCU + user profile), so nothing
needs administrator rights — matching the `asInvoker` manifest the engine ships with.

## Locations

| What | Path |
|---|---|
| Default install dir | `%LOCALAPPDATA%\Programs\<app.name>` |
| App data (handoff) | `%APPDATA%\<app.id>` |
| Start Menu shortcut | `%APPDATA%\Microsoft\Windows\Start Menu\Programs\<name>.lnk` |
| Desktop shortcut | `%USERPROFILE%\Desktop\<name>.lnk` |
| URL protocol | `HKCU\Software\Classes\<scheme>` |
| Uninstall entry (ARP) | `HKCU\…\CurrentVersion\Uninstall\<app.id>` |
| PATH entry | `HKCU\Environment` → `Path` |

## What the engine does

- **Shortcuts** — real `.lnk` files (via `mslnk`), Start Menu always, Desktop if
  `[install].desktop_shortcut = true`.
- **Protocol** — registers `<scheme>://` under `HKCU\Software\Classes` with
  `shell\open\command = "<exe>" "%1"`.
- **Uninstaller** — writes an Apps & Features entry whose `UninstallString` is
  `"<install>\uninstall.exe" --uninstall` (a copy of the setup), plus a
  `uninstall-info.json` so the uninstall can reverse exactly what it did.
- **Detect existing install** — reads the ARP `InstallLocation` / `DisplayVersion`
  (drives maintenance mode + the Update button).

## Elevation / Program Files

The manifest is `asInvoker` → **no UAC prompt**, but you can only write where the user
can. Installing into `C:\Program Files` needs an elevated (`requireAdministrator`)
build; the GUI shows a clear error if the chosen folder isn't writable, and the
**Browse…** button lets the user pick a writable location.

## Building

```powershell
cargo build --release -p bpkg-cli -p installer
./examples/bmm/build-installer.ps1        # pack → sign → stamp → BMM-Setup.exe
```

## SmartScreen / code signing

Ed25519 signing covers **package integrity** (the engine verifies it before
installing). It does **not** give Windows reputation — for that, Authenticode-sign the
`*-Setup.exe` with a code-signing certificate (EV cert clears SmartScreen fastest).
That's orthogonal to the `[security]` package signing.

## Gotchas

- The bundle identifier (`[app].id`) must equal the app's data-dir identifier — the
  handoff is written to `%APPDATA%\<id>`; a mismatch means the app never reads it.
- The uninstaller deletes itself (detached `cmd` + `rmdir /S /Q`) after removing the
  install dir, and kills the running app first so files aren't locked.
