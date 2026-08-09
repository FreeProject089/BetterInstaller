# Référence CLI `bpkg`

[🇬🇧 English](https://github.com/FreeProject089/BetterInstaller/blob/master/docs/CLI.md) · 🇫🇷 Français

Build unique : `cargo build --release -p bpkg-cli` → `target/release/bpkg`.

## Commandes

### `pack` — construire un `.bpkg` depuis un dossier
```
bpkg pack --root <DOSSIER> --config <installer.toml> --out <app.bpkg>
```
Hash chaque fichier (SHA-256), assigne les composants via `[[components]].paths`,
compresse en zstd, écrit le manifest + payload.

### `sign` — signer un paquet sur place
```
bpkg sign --key <private.key> <app.bpkg>
```
Ajoute une signature Ed25519 (pose `FLAG_SIGNED`). À relancer après chaque `pack`.

### `keygen` — générer une paire de clés de signature
```
bpkg keygen --out <dossier>          # défaut : ./keys
```
Écrit `private.key` (SECRÈTE — ne jamais commit) + `public.key`. Mets la clé publique
dans `[security].public_key`.

### `verify` — intégrité (+ signature)
```
bpkg verify <app.bpkg> [--key <public.key>]
```
Vérifie le SHA-256 de chaque fichier ; avec `--key`, vérifie aussi la signature Ed25519.

### `build` — stamper l'installeur auto-extractible
```
bpkg build --installer <betterinstaller.exe> --config <installer.toml> \
           --package <app.bpkg> --out <App-Setup.exe>
```
Ajoute config + paquet + trailer à une copie de l'exe moteur → un seul `*-Setup.exe`.

### `info` / `extract` / `install`
```
bpkg info <app.bpkg>                       # affiche les métadonnées du manifest + composants
bpkg extract <app.bpkg> --dest <dossier>   # décompresse (vérifie au passage)
bpkg install <app.bpkg> --dest <dossier>   # vérifie + extrait avec barre de progression
```
`install` utilise exactement le chemin de l'étape Install du GUI.

### `update` / `fetch-update` — appliquer des versions plus récentes
```
bpkg update <new.bpkg> --dir <install_dir>            # applique un pkg local plus récent (rollback si échec)
bpkg fetch-update --url <manifest.json> --dir <install_dir> --current <version>
```
Voir [UPDATES.md](UPDATES.md).

### `delta` / `apply-delta` — patches binaires
```
bpkg delta <old.bpkg> <new.bpkg> <out.patch>     # crée un patch bsdiff
bpkg apply-delta <old.bpkg> <patch> <out.bpkg>   # reconstruit le nouveau paquet
```

## Pipeline typique

```sh
cargo build --release -p bpkg-cli -p installer
bpkg keygen --out keys
# (colle keys/public.key dans installer.toml [security].public_key)
bpkg pack  --root payload --config installer.toml --out app.bpkg
bpkg sign  --key keys/private.key app.bpkg
bpkg verify app.bpkg --key keys/public.key
bpkg build --installer ./target/release/betterinstaller.exe \
           --config installer.toml --package app.bpkg --out App-Setup.exe
```

L'exemple bundlé automatise ça : `./examples/<app>/build-installer.ps1`.
