# `bpkg` CLI reference

🇬🇧 English · [🇫🇷 Français](https://github.com/FreeProject089/BetterInstaller/blob/master/docs/CLI.fr.md)

Build it once: `cargo build --release -p bpkg-cli` → `target/release/bpkg`.

## Commands

### `pack` — build a `.bpkg` from a folder
```
bpkg pack --root <DIR> --config <installer.toml> --out <app.bpkg>
```
Hashes every file (SHA-256), assigns components by `[[components]].paths`,
zstd-compresses, writes the manifest + payload.

### `sign` — sign a package in place
```
bpkg sign --key <private.key> <app.bpkg>
```
Adds an Ed25519 signature (sets `FLAG_SIGNED`). Re-run after any `pack`.

### `keygen` — generate a signing keypair
```
bpkg keygen --out <dir>          # default: ./keys
```
Writes `private.key` (SECRET — never commit) + `public.key`. Put the public key in
`[security].public_key`.

### `verify` — integrity (+ signature)
```
bpkg verify <app.bpkg> [--key <public.key>]
```
Checks every file's SHA-256; with `--key`, also verifies the Ed25519 signature.

### `build` — stamp the self-extracting installer
```
bpkg build --installer <betterinstaller.exe> --config <installer.toml> \
           --package <app.bpkg> --out <App-Setup.exe>
```
Appends config + package + trailer to a copy of the engine exe → one `*-Setup.exe`.

### `info` / `extract` / `install`
```
bpkg info <app.bpkg>                       # print manifest metadata + components
bpkg extract <app.bpkg> --dest <dir>       # unpack (verifies on the way)
bpkg install <app.bpkg> --dest <dir>       # verify + extract with a progress bar
```
`install` uses the exact path the GUI's Install step uses.

### `update` / `fetch-update` — apply newer versions
```
bpkg update <new.bpkg> --dir <install_dir>            # apply a local newer pkg (rollback on fail)
bpkg fetch-update --url <manifest.json> --dir <install_dir> --current <version> [--key public.key]
```
With `--key`, the manifest must be signed by that key and unexpired, and the package
must match it. See [UPDATES.md](UPDATES.md).

### `update-manifest` / `resign-manifest` / `verify-manifest` — the signed `update.json`
```
bpkg update-manifest --package <app.bpkg> --config installer.toml --key private.key \
  --url <package url> [--mirror <url>]… [--delta <from>=<url>]… [--notes <text>] \
  [--valid-days 1..7] --out update.json
bpkg resign-manifest <update.json> --key private.key [--valid-days 1..7] [--out <file>]
bpkg verify-manifest <update.json> --key public.key [--app-id <id>] [--setup <App-Setup.exe>]
```
`update-manifest` reads the app id, version and SHA-256 from the (signed) package and the
SHA-256 of `installer.toml`, then signs; the manifest expires after `--valid-days`
(default and maximum 7). `resign-manifest` renews the expiry of a manifest the same key
signed — run it at least weekly between releases. `verify-manifest` checks signature and
expiry, and with `--setup` that the setup carries exactly the config and package named.

### `delta` / `apply-delta` — binary patches
```
bpkg delta --old <old.bpkg> --new <new.bpkg> --out <out.patch>     # create a bsdiff patch
bpkg apply-delta --old <old.bpkg> --patch <patch> --out <out.bpkg>   # reconstruct the new package
```

## Typical pipeline

```sh
cargo build --release -p bpkg-cli -p installer
bpkg keygen --out keys
# (paste keys/public.key into installer.toml [security].public_key)
bpkg pack  --root payload --config installer.toml --out app.bpkg
bpkg sign  --key keys/private.key app.bpkg
bpkg verify app.bpkg --key keys/public.key
bpkg build --installer ./target/release/betterinstaller.exe \
           --config installer.toml --package app.bpkg --out App-Setup.exe
```

The bundled example automates this: `./examples/<app>/build-installer.ps1`.
