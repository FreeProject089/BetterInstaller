# BetterInstaller — Documentation

🇬🇧 English · [🇫🇷 Français](README.fr.md)

A proprietary, cross-platform installer/updater framework: **one `installer.toml` +
one payload folder → a single signed, self-extracting `*-Setup.exe`**. Native Slint
GUI (no WebView runtime), first-run config handoff, auto-update with rollback, and a
maintenance mode (repair / update / uninstall).

## Where to start

| You want to… | Read |
|---|---|
| Configure an installer for your app | [../GUIDE.md](../GUIDE.md) — every `installer.toml` field |
| Understand how it works internally | [ARCHITECTURE.md](ARCHITECTURE.md) |
| Ship one app on Windows + Linux + macOS | [PLATFORMS.md](PLATFORMS.md) (+ per-OS: [Windows](platform-windows.md) · [Linux](platform-linux.md) · [macOS](platform-macos.md)) |
| Know the `.bpkg` / SFX byte layout | [BPKG-FORMAT.md](BPKG-FORMAT.md) |
| Use the `bpkg` CLI | [CLI.md](CLI.md) |
| Implement the first-run handoff in your app | [HANDOFF.md](HANDOFF.md) |
| Ship updates (manifest, deltas) | [UPDATES.md](UPDATES.md) |
| Host the updater (GitHub / own server) | [UPDATER-SETUP.md](UPDATER-SETUP.md) |
| Sign packages | [SIGNING.md](SIGNING.md) |

## The 60-second mental model

```
payload/            installer.toml          keys/private.key
   │                     │                        │
   ▼                     ▼                        ▼
bpkg pack ─────────► app.bpkg ──► bpkg sign ──► app.bpkg (signed)
                                                   │
            betterinstaller.exe (engine) ──► bpkg build ──► App-Setup.exe
                                                                  │
                                                   double-click ──┘
                                                        │
                          ┌─────────────────────────────┴───────────────┐
                          ▼                                              ▼
                   fresh install                                 already installed
              (Welcome→Terms→Setup→Install→Done)            (maintenance: repair/update/uninstall)
                          │
                          ▼
            writes installer-handoff.json  ──►  app reads it once on first launch
```

## Repository layout

```
BetterInstaller/
├── crates/
│   ├── bpkg-core/     # library: format, signing, handoff, update, platform ops
│   ├── bpkg-cli/      # the `bpkg` command-line tool
│   └── installer/     # the Slint GUI engine (betterinstaller.exe)
├── examples/bmm/      # a complete real config (Acme Editor)
├── GUIDE.md           # installer.toml configuration reference
└── docs/              # this folder
```

## Design decisions (locked)

- **UI = Slint** (native GPU, single static binary) — not WebView2/wry.
- **`asInvoker` manifest** (no UAC) → installs **per-user** by default
  (`%LOCALAPPDATA%\Programs\<name>`); Program Files needs an elevated build.
- **App-agnostic handoff**: the installer never writes an app's private config
  format; it writes a standard `installer-handoff.json` the app consumes once.
- **Ed25519** package signing; **zstd** payload compression; **bsdiff** deltas.
