# BetterInstaller — Analyse technique

> Une plongée technique de zéro dans **BetterInstaller** : un framework
> d'installation/mise à jour propriétaire et réutilisable — une alternative
> multiplateforme à NSIS/MSI avec une UI native Slint, des paquets auto-extractibles
> signés Ed25519, un handoff de config au premier lancement, et des mises à jour avec
> rollback. Docs compagnons : **App_Features_FR.md** (ce qu'il fait) et
> **Security_Audit_FR.md** (revue de menaces).

---

## 1. Forme du projet

Un workspace Rust (`Cargo.toml`, `resolver = "2"`), profil release réglé pour un
binaire minuscule (`opt-level="z"`, `lto`, `codegen-units=1`, `strip`,
`panic="abort"` — cible <5 Mo). Trois crates :

| Crate | Type | Rôle |
|---|---|---|
| `bpkg-core` | bibliothèque | le moteur — le format de paquet `.bpkg`, la signature, le delta, la mise à jour/rollback, l'intégration plateforme, la config, l'i18n, le handoff |
| `bpkg-cli` (`bpkg`) | binaire | la CLI de packaging — `keygen`, `pack`, `sign`, `stamp`, `verify` |
| `installer` | binaire (Slint) | l'installeur GUI produit pour une app finale (`Setup.exe` auto-extractible) |

Dépendances clés : `serde`/`serde_json`/`toml` (config + manifeste), `zstd`
(compression), `sha2` (intégrité), `ed25519-dalek` + `rand` (signature), `qbsdiff`
(deltas binaires), `reqwest` (rustls, bloquant, pour les mises à jour), `slint` (UI
native), `winreg`/`mslnk` (intégration Windows), `embed-manifest` (manifeste app Windows).

## 2. Le format de paquet `.bpkg` (`package/`)

Un `.bpkg` est un seul blob auto-descriptif :

```
[ En-tête (HEADER_LEN fixe) ][ Manifeste (JSON, manifest_len) ][ Payload (zstd, payload_len) ][ Signature (64o, si FLAG_SIGNED) ]
```

- **`format.rs`** — l'`Header` binaire (magic, version, flags dont `FLAG_SIGNED`,
  `manifest_len`, `payload_len`) avec `from_bytes`/`to_bytes`.
- **`manifest.rs`** — `Manifest { app: AppMeta, files: [{ path, sha256, component }] }`.
  `AppMeta` = id/name/version/publisher/homepage/platforms.
- **`writer.rs`** — construit l'archive interne (un flux `path|data` préfixé en
  longueur), la compresse en zstd, calcule le SHA-256 par fichier, écrit
  en-tête+manifeste+payload.
- **`reader.rs`** — `Package::open` lit en-tête+manifeste immédiatement ; le payload est
  lu à la demande. Fournit `verify()` (SHA-256 de chaque fichier vs manifeste),
  `verify_signature(vk)` (Ed25519 sur les octets manifeste+payload), `read_files()`
  (aperçu des docs de licence avant install), et les deux chemins d'extraction :
  `install_with_progress()` (vérifie chaque hash **puis** écrit, callback de progression)
  et `extract()`. Les deux appliquent un **garde anti-traversée** (`..`, `/` ou `\` en
  tête). Le parsing d'archive est entièrement borné (`slice`/`read_u32`/`read_u64`
  utilisent `checked_add` et rejettent la troncature) — aucune lecture hors bornes sur
  un paquet corrompu.

## 3. Signature (`sign.rs`)

Ed25519 via `ed25519-dalek`. `generate()` utilise le CSPRNG de l'OS (`OsRng`) ; les
clés sont stockées en hex 32 octets (seed `private.key` / clé de vérif `public.key`),
validées en longueur et en hex au chargement. Une signature couvre les octets
**manifeste + payload**. `bpkg-cli` signe (`bpkg sign --key private.key app.bpkg`) et
vérifie ; l'installeur vérifie contre une `public_key` épinglée dans `installer.toml`.

## 4. Config & embarquement (`config.rs`, `embed.rs`)

- **`config.rs`** — `installer.toml` : métadonnées app, UI/branding, composants,
  raccourcis, prérequis, `security { public_key, require_signature }`, sources de mise à
  jour. C'est la source de vérité unique éditée par l'auteur de l'app.
- **`embed.rs`** — l'astuce auto-extractible : l'exe installeur construit porte la
  config + le `.bpkg` en blob appendu délimité par un magic ; à l'exécution l'installeur
  le localise et le stage dans un fichier temp pour que `Package::open` le lise.

## 5. Mises à jour & rollback (`update.rs`, `delta.rs`, `net.rs`)

- **`net.rs`** — HTTP bloquant minimal (rustls, timeout 60 s, UA versionné) :
  `fetch_text` (manifeste JSON) et `download` (octets d'un `.bpkg`/patch).
- **`delta.rs`** — diff/patch binaire `qbsdiff`/`qbspatch`, pour qu'une mise à jour
  livre un petit patch (`old.bpkg → new.bpkg`) au lieu du paquet complet.
- **`update.rs`** — `UpdateManifest { version, url, deltas[] }`. `check_remote` /
  `check_remote_multi` récupèrent un ou plusieurs miroirs de manifeste et renvoient le
  plus récent (les miroirs morts sont ignorés ; tout-échoué est une erreur, pas un
  silencieux « à jour »). `download_and_apply` préfère un delta depuis la version
  courante, sinon téléchargement complet, puis `apply_package_update`, qui est
  **atomique-ish** : snapshot du dossier d'install vers un voisin `<name>.bak`, install
  par-dessus, et sur **toute** erreur wipe + restaure le snapshot ; en cas de succès on
  le supprime. (Un test flippe un octet du payload et vérifie le rollback.)

  > **Note (voir Security_Audit) :** `download_and_apply`/`apply_package_update`
  > vérifient le SHA-256 de chaque fichier contre le *manifeste du paquet lui-même*
  > (auto-cohérence) mais n'appellent **pas** `verify_signature` — l'authenticité n'est
  > appliquée que là où l'installeur GUI vérifie contre la `public_key` épinglée.

## 6. Intégration plateforme (`platform/`)

Un trait `PlatformOps` avec des backends `windows.rs` / `linux.rs` / `macos.rs` :
dossier d'install par défaut, dossier app-data, raccourcis, enregistrement du gestion-
naire de protocole, enregistrement du désinstalleur (Ajout/Suppression de programmes),
entrée PATH, lookup dossier/version installés.

**Windows** est volontairement **par-utilisateur, sans admin/UAC** (manifeste
`asInvoker`) : installe dans `%LOCALAPPDATA%\Programs\<name>`, n'écrit que dans `HKCU`
(gestionnaire de protocole Classes, entrée `CurrentVersion\Uninstall`, PATH
`Environment`). Installer dans Program Files nécessiterait un build élevé séparé.

## 7. Handoff & prérequis (`handoff.rs`, `prereq.rs`)

- **`handoff.rs`** — le contrat de premier lancement : l'installeur écrit un petit
  fichier de handoff que l'app installée lit au premier démarrage (ex. options choisies
  / config initiale), pour que les choix d'install passent dans l'app sans second
  assistant.
- **`prereq.rs`** — prérequis déclarés vérifiés avant l'install (ex. un runtime),
  remontés dans l'UI.

## 8. UI & i18n (crate `installer`, `i18n.rs`)

Le binaire `installer` est une GUI native **Slint** (aucun runtime WebView), construite
via `build.rs` + `slint-build`. `main.rs` câble le flux : stage du paquet embarqué →
détection/vérif de signature → licence/composants → vérif + extraction sur un thread
worker avec progression throttlée → raccourcis/registre → fini ; plus la Maintenance
(Réparer = re-vérif + restaure la même version ; Mise à jour = vérifie le manifeste,
delta ou complet). `i18n.rs` fournit des chaînes localisées (EN/FR), en phase avec les
docs bilingues.

## 9. Build & utilisation (CLI)

```sh
cargo build --release -p bpkg-cli -p installer
bpkg keygen --out keys
bpkg pack  --root payload --config installer.toml --out app.bpkg
bpkg sign  --key keys/private.key app.bpkg
bpkg stamp --installer target/release/installer.exe --package app.bpkg --out Setup.exe
```

La CI lance `fmt` · `clippy` · `test`. Voir **GUIDE_FR.md** pour le tutoriel complet.
