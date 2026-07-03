# BetterInstaller — Audit de sécurité

**Date :** 2026-07-03 · **Périmètre :** le moteur `bpkg-core`, `bpkg-cli`, et l'installeur
GUI `installer` (workspace Rust). **Focus :** les frontières de confiance qui comptent
pour un installeur/updater — authenticité des paquets (signature), parsing d'entrées non
fiables (`.bpkg`), extraction d'archive (traversée de chemin / zip-slip), le chemin
réseau de mise à jour, et les privilèges OS. C'est une revue de code source, pas un
pentest de binaire publié.

**En résumé :** les fondations cryptographiques sont saines (signature Ed25519, SHA-256
par fichier, parsing borné, install par-utilisateur sans admin). Deux constats méritent
attention avant une sortie publique : une **faille de traversée par chemin absolu à
lettre de lecteur Windows** à l'extraction, et le fait que l'**API de mise à jour auto du
moteur applique les paquets sans vérifier la signature** (l'authenticité n'est appliquée
que dans le chemin GUI). Aucun n'est un RCE évident à lui seul, mais les deux affaiblissent
la garantie de « mises à jour signées » que le framework annonce.

---

## Examiné et jugé SOLIDE

- **Signature (`sign.rs`)** — Ed25519 via `ed25519-dalek` ; paire de clés depuis le
  CSPRNG de l'OS (`OsRng`) ; clés validées en longueur/hex au chargement ; la signature
  couvre manifeste+payload.
- **Intégrité (`package/reader.rs`)** — le SHA-256 de chaque fichier est vérifié contre le
  manifeste **avant** écriture (`install_with_progress`), et `verify()` vérifie tout le
  paquet.
- **Sûreté du parseur** — le lecteur d'archive interne est entièrement borné (`slice`
  utilise `checked_add` et rejette `end > buf.len()`), donc un `.bpkg` tronqué/hostile
  produit une erreur `Corrupt` propre, pas une lecture hors bornes ou un panic.
- **Privilèges (`platform/windows.rs`)** — **par-utilisateur, sans admin/UAC**
  (`asInvoker`) : install dans `%LOCALAPPDATA%\Programs`, n'écrit que `HKCU`. Cela retire
  toute la classe de risques d'installeur élevé (écriture Program Files/HKLM, abus de
  contournement UAC).
- **TLS (`net.rs`)** — `reqwest` avec **rustls** (pas de native-tls), timeout de requête,
  User-Agent versionné.
- **Rollback de mise à jour (`update.rs`)** — snapshot vers `<name>.bak`, restauration sur
  TOUTE erreur ; couvert par un test qui corrompt un octet de payload et vérifie la
  restauration de l'état pré-mise à jour.
- **Application de la signature dans la GUI (`installer/src/main.rs`)** — quand
  `security.public_key` est défini, elle appelle `verify_signature`, et
  `security.require_signature` abandonne l'install sur une signature absente/invalide.

---

## Constats

| # | Sévérité | Problème |
|---|---|---|
| 1 | **Moyenne** | Traversée par chemin absolu à lettre de lecteur Windows à l'extraction/install |
| 2 | **Moyenne** | L'API de mise à jour auto du moteur applique les paquets sans vérifier la signature |
| 3 | Faible | HTTP en clair accepté pour manifestes/téléchargements (pas d'HTTPS forcé) |
| 4 | Faible (DoS) | Décompression / téléchargement non borné en mémoire |
| 5 | Info | Les paquets non signés/non configurés s'installent sans contrôle d'authenticité |

### 1. Traversée par chemin absolu Windows — *Moyenne*
`extract()` et `install_with_progress()` (`package/reader.rs`) gardent avec :
```rust
if path.contains("..") || path.starts_with('/') || path.starts_with('\\') { …rejeter… }
```
Cela bloque `../` et les chemins relatifs à la racine, mais **pas un chemin absolu à
lettre de lecteur Windows** comme `C:\Windows\System32\evil.dll` (ou `C:foo`). Sous
Windows, `dest.join("C:\\Windows\\…")` **écarte `dest`** car l'argument est absolu — donc
un chemin de manifeste malveillant s'échappe du dossier d'install. L'exploitation exige un
paquet accepté par l'installeur (non signé, ou signé par une clé de confiance), mais
l'extraction est aussi atteignable avant/sans vérification de signature.
**Correctif :** rejeter toute entrée où `Path::new(path).is_absolute()` ou dont les
composants contiennent un `Prefix`/`RootDir`, et/ou canonicaliser le chemin joint et
vérifier qu'il reste dans `dest` (`starts_with(dest)`). À appliquer aux DEUX chemins.

### 2. La mise à jour auto applique les paquets sans vérifier la signature — *Moyenne*
`update.rs::download_and_apply` → `apply_package_update` font `Package::open` +
`install_with_progress`, qui vérifient le SHA-256 de chaque fichier **contre le manifeste
du paquet lui-même** — c.-à-d. l'*auto-cohérence*, pas l'*authenticité*. Ils ne prennent
aucune `VerifyingKey` et n'appellent jamais `verify_signature`. Un miroir malveillant ou
un MITM réseau peut servir un `.bpkg` auto-cohérent, non signé, à l'URL de mise à jour, et
il sera installé. L'installeur GUI vérifie séparément, mais l'API réutilisable du moteur
fait du **chemin non vérifié le défaut**, donc tout consommateur utilisant directement
`download_and_apply` n'a aucune garantie d'authenticité.
**Correctif :** faire passer une `VerifyingKey` épinglée (depuis
`config.security.public_key`) à travers `download_and_apply`/`apply_package_update` et
**échouer fermé** (vérifier la signature avant staging/extraction), pour que la sûreté des
mises à jour signées ne soit pas optionnelle.

### 3. HTTP en clair accepté — *Faible*
`net.rs` récupère n'importe quelle URL fournie, y compris `http://`. L'authenticité du
paquet (une fois le constat #2 corrigé) protège le payload, mais le **manifeste de mise à
jour** (qui dicte la version et l'URL de téléchargement) est un JSON non authentifié — en
clair il peut être altéré/rétrogradé.
**Correctif :** exiger `https://` (liste blanche de schémas) pour les URLs de manifeste +
téléchargement, ou signer aussi le manifeste.

### 4. Décompression / téléchargement non borné — *Faible (DoS)*
`read_archive` appelle `zstd::decode_all` et `net::download` appelle `resp.bytes()`, tous
deux lisant entièrement en mémoire sans plafond. Un paquet ou une réponse hostile pourrait
épuiser la mémoire.
**Correctif :** plafonner la taille décompressée (et/ou streamer) et imposer une limite de
`Content-Length`/lecture sur les téléchargements.

### 5. Paquets non signés/non configurés installés sans authenticité — *Info*
Si `security.public_key` n'est pas défini, la GUI saute entièrement la vérification
(confiance au premier usage). C'est un choix de conception légitime, mais cela signifie
que la posture *par défaut* pour un auteur d'app qui ne configure pas la signature est
« installe ce qu'on t'a donné ».
**Recommandation :** documenter que les éditeurs DEVRAIENT définir `public_key` +
`require_signature` ; envisager un avertissement UI voyant à l'install d'un paquet non signé.

---

## Remédiation (appliquée le 2026-07-03)

- **#1 Traversée de chemin — CORRIGÉ.** `package/reader.rs` fait désormais passer chaque
  entrée d'archive par `unsafe_entry_path()`, qui rejette la traversée (`..`), les
  chemins POSIX-absolus, Windows drive-absolus (`C:\…`/`C:foo`) et UNC (`\\server`) —
  indépendant de l'OS hôte, appliqué à `extract()` ET `install_with_progress()`. Couvert
  par `rejects_escaping_entry_paths`.
- **#2 Vérification de signature à la mise à jour — CORRIGÉ.**
  `update.rs::apply_package_update` / `download_and_apply` prennent maintenant un
  `verify_key: Option<&VerifyingKey>` ; quand une clé est fournie, la signature Ed25519
  est vérifiée **avant** snapshot/écriture et **échoue fermé** si absente/invalide. Le
  chemin de mise à jour de l'installeur épingle `security.public_key`. Couvert par
  `update_refuses_unsigned_package_when_key_pinned`.
- **#3 HTTP en clair — CORRIGÉ.** `net.rs` `require_https()` rejette toute URL de
  manifeste/téléchargement non-`https://`.
- **#4 Décompression/téléchargement non borné — CORRIGÉ.** `read_archive` décode en flux
  avec un plafond de 4 Gio ; `net.rs` plafonne les corps de réponse à 1 Gio.
- **#5 Non signé par défaut — INFO.** Documenté ; les éditeurs devraient définir
  `public_key` + `require_signature` (comportement inchangé par conception).

`cargo test -p bpkg-core` au vert (11 tests), `cargo check` + `clippy` propres sur tout
le workspace.

## Résumé des recommandations

| Élément | Sévérité | Statut |
|---|---|---|
| Traversée par chemin absolu | Moyenne | **Corrigé** — chemins absolus/préfixés rejetés |
| L'API de mise à jour saute la vérif de signature | Moyenne | **Corrigé** — vérif par clé épinglée, échoue fermé |
| HTTP en clair | Faible | **Corrigé** — HTTPS exigé |
| Décompression/téléchargement non borné | Faible | **Corrigé** — plafonds de taille |
| Non signé par défaut | Info | Documenté ; encourager `require_signature` |
| Signature Ed25519, SHA-256, bornes du parseur, install sans admin, TLS, rollback | — | Sain — conservé |
