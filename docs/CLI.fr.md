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
bpkg fetch-update --url <manifest.json> --dir <install_dir> --current <version> [--key public.key]
```
Avec `--key`, le manifest doit être signé par cette clé et non expiré, et le paquet doit
lui correspondre. Voir [UPDATES.md](UPDATES.md).

### `update-manifest` / `resign-manifest` / `verify-manifest` — le `update.json` signé
```
bpkg update-manifest --package <app.bpkg> --config installer.toml --key private.key \
  --url <url du paquet> [--mirror <url>]… [--delta <from>=<url>]… [--notes <texte>] \
  [--valid-days 1..7] --out update.json
bpkg resign-manifest <update.json> --key private.key [--valid-days 1..7] [--out <fichier>]
bpkg verify-manifest <update.json> --key public.key [--app-id <id>] [--setup <App-Setup.exe>]
```
`update-manifest` lit l'id d'app, la version et le SHA-256 dans le paquet (signé) et le
SHA-256 de `installer.toml`, puis signe ; le manifest expire après `--valid-days` (7 par
défaut et au maximum). `resign-manifest` renouvelle l'expiration d'un manifest signé par la
même clé — à lancer au moins chaque semaine entre deux releases. `verify-manifest` vérifie
signature et expiration, et avec `--setup` que le setup porte exactement la config et le
paquet nommés.

### `delta` / `apply-delta` — patches binaires
```
bpkg delta --old <old.bpkg> --new <new.bpkg> --out <out.patch>     # crée un patch bsdiff
bpkg apply-delta --old <old.bpkg> --patch <patch> --out <out.bpkg>   # reconstruit le nouveau paquet
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
