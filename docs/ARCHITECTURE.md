# Architecture

🇬🇧 English · [🇫🇷 Français](https://github.com/FreeProject089/BetterInstaller/blob/master/docs/ARCHITECTURE.fr.md)

BetterInstaller is a small Rust workspace. The engine is written once against a
platform-abstraction trait; everything project-specific lives in `installer.toml`.

## Crates

### `bpkg-core` (library)
The reusable core. Modules:

| Module | Responsibility |
|---|---|
| `package` (`format`, `reader`, `writer`) | The `.bpkg` format: pack, open, verify, extract, `install_with_progress`. |
| `manifest` | `Manifest` / `FileEntry` / `AppMeta` / `Component` types. |
| `config` | Parses `installer.toml` (`InstallerConfig` + all sections). |
| `sign` | Ed25519 keygen / sign / verify (ed25519-dalek). |
| `handoff` | Builds + writes `installer-handoff.json` (the first-run contract). |
| `update` | Remote manifest check, download, bsdiff delta apply, atomic rollback. |
| `delta` | Binary diff/patch (qbsdiff). |
| `embed` | Self-extracting trailer read/write (config + bpkg appended to the exe). |
| `prereq` | Detect/auto-install prerequisites (registry/file/command checks). |
| `platform` | `PlatformOps` trait + Windows/Linux/macOS backends. |
| `net`, `i18n`, `error` | HTTP (reqwest rustls), translations, error type. |

### `bpkg-cli` (`bpkg`)
A thin CLI over `bpkg-core`: `pack`, `sign`, `verify`, `keygen`, `build`,
`info`, `extract`, `install`, `update`, `fetch-update`, `delta`, `apply-delta`.
See [CLI.md](CLI.md).

### `installer` (`betterinstaller.exe`)
The Slint GUI engine. Frameless window (custom title bar via the winit backend for
drag / minimize / maximize). Resolves its config + package from the embedded SFX
trailer (or CLI args in dev), renders the flow, writes the handoff, runs the install
on a worker thread, and does the OS integration.

## `PlatformOps` (the abstraction)

The engine never branches on OS. Each backend implements:

```
default_install_dir, app_data_dir,
create_shortcuts, register_protocol, register_uninstaller, add_to_path,
remove_shortcuts, unregister_protocol, unregister_uninstaller,
installed_dir, installed_version
```

| Op | Windows | Linux | macOS |
|---|---|---|---|
| Shortcuts | `.lnk` (mslnk) in Start Menu / Desktop | `.desktop` files | `/Applications` symlink |
| Protocol | HKCU `Software\Classes\<scheme>` | `xdg-mime` | `Info.plist` |
| Uninstaller | HKCU ARP entry | — | — |
| Detect install | ARP `InstallLocation` / `DisplayVersion` | — | — |

Per-user everywhere (matches the `asInvoker` manifest — no admin needed).

## Install flow (GUI)

1. **resolve_sources** — read embedded SFX (config + staged `.bpkg`), else CLI args.
2. **detect** — `installed_dir(app.id)` → maintenance mode if already installed;
   `detect_signature` → the Welcome trust badge.
3. **Welcome** (dir + components) → **Terms** (license docs, per-doc accept) →
   **Setup** (options) → **Install** → **Done** (opt-in launch).
4. **On Install** (worker thread): write `installer-handoff.json` → prereq gate →
   writability preflight → verify signature → `install_with_progress` →
   `do_system_integration` (shortcuts, protocol, ARP entry, `uninstall-info.json`).

## Maintenance flow

Entered when `installed_dir` is found (or `--uninstall`). Reads the ARP
`InstallLocation`; offers **Repair** (re-verify+restore), **Update** (only if a
newer version is found — remote manifest or newer bundled package; download + apply
with rollback), **Uninstall** (kill running app → reverse integration → remove dir,
incl. the uninstaller via a detached self-delete). All confirm-gated with Cancel.

## First-run handoff

The headline feature: the installer pre-configures the app so it shows **no**
first-run modals. App-agnostic contract — see [HANDOFF.md](HANDOFF.md).
