# BetterInstaller — Features

> What **BetterInstaller** does, feature by feature. Engineering detail in
> **Technical_Analysis_EN.md**; threat review in **Security_Audit_EN.md**.

## For app authors
- **One config → one installer.** Describe your app in a single `installer.toml`
  (name, version, publisher, branding, components, shortcuts, prerequisites, update
  sources) and produce a single self-extracting `Setup.exe`.
- **`.bpkg` packages** — your payload is packed into one signed, zstd-compressed,
  self-describing file with a per-file SHA-256 manifest.
- **Ed25519 signing** — `bpkg keygen` / `sign`; the installer verifies against a public
  key you pin in the config, and can be set to **abort on a missing/invalid signature**.
- **Tiny binary** — release profile tuned for a <5 MB installer, no WebView runtime.
- **CLI** — `bpkg keygen · pack · sign · stamp · verify` for scripted/CI packaging.
- **Cross-platform** — Windows, Linux, macOS backends behind one engine.

## For end users (the installer GUI)
- **Native Slint UI** — no browser/WebView, fast and small.
- **No admin needed (Windows)** — per-user install to `%LOCALAPPDATA%\Programs`, only
  `HKCU` touched (no UAC prompt, no system-wide changes).
- **License & components** — review license documents (read straight from the package
  before extracting) and pick optional components.
- **Verified install** — every file's hash is checked against the manifest **before**
  it's written; the package signature is verified when a public key is configured.
- **Shortcuts & integration** — Start-menu/desktop shortcuts, protocol-handler
  registration, Add/Remove Programs entry, optional PATH entry.
- **First-run handoff** — install-time choices are handed to the app so it starts
  already configured (no second setup wizard).
- **Progress** — throttled, per-file progress during extraction.

## Updates & maintenance
- **Auto-update** — checks one or several update-manifest mirrors and offers the newest
  version (a dead mirror never blocks the others).
- **Binary deltas** — downloads a tiny patch from your current version when offered,
  instead of the full package.
- **Rollback-safe** — an update snapshots the install dir first and **restores it on any
  failure**, so a broken update can't leave a half-installed app.
- **Repair** — re-verify and restore the current version.
- **Clean uninstall** — removes files, shortcuts, protocol handler and registry entries.
