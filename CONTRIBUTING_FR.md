# Contribuer à BetterInstaller

[🇬🇧 English](CONTRIBUTING.md) · 🇫🇷 Français

Merci de ton intérêt ! BetterInstaller est un framework d'installeur/updater
multiplateforme (Rust + Slint). Ce guide couvre l'installation de l'environnement, la
barrière qualité, et comment ouvrir des issues et des pull requests.

## En bref

```sh
# build du moteur + CLI
cargo build --release -p bpkg-cli -p installer
# la barrière imposée par le CI (à lancer avant de push) :
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Structure du projet

```
crates/bpkg-core/   bibliothèque : format .bpkg, signature, handoff, update, ops par OS
crates/bpkg-cli/    l'outil en ligne de commande `bpkg`
crates/installer/   le moteur GUI Slint (betterinstaller.exe)
examples/bmm/       un exemple réel complet (config + scripts build/release)
docs/               architecture, format, CLI, handoff, updates, signature, plateformes
```

## Pré-requis

- **Rust** stable (comme le CI : dernier stable) avec `clippy` + `rustfmt`
  (`rustup component add clippy rustfmt`).
- **Dépendances Linux** (Slint + dialogue de fichiers rfd) :
  `sudo apt-get install -y libfontconfig-dev libxcb-shape0-dev libxcb-xfixes0-dev libgtk-3-dev pkg-config`
- **Windows / macOS** : aucune dépendance système supplémentaire pour le build par défaut.

### Tout builder sous Linux via Docker (sans galère de toolchain)

```sh
docker build -t betterinstaller-dev .
docker run --rm -e CARGO_TARGET_DIR=/tmp/t -v "${PWD}:/app" betterinstaller-dev   # lance toute la barrière
```

`CARGO_TARGET_DIR=/tmp/t` garde les artefacts Linux hors de ton `./target` Windows.

## La barrière qualité (doit être verte)

Le CI tourne sur **windows-latest + ubuntu-latest** :

1. `cargo fmt --all -- --check` — formatage.
2. `cargo clippy --workspace --all-targets -- -D warnings` — **chaque warning est une erreur**.
3. `cargo test --workspace`.
4. `cargo build --workspace --release`.

Lance la même chose en local avant d'ouvrir une PR. Clippy est l'échec le plus fréquent :
corrige le lint, ne le `#[allow]` pas (sauf raison claire et commentée).

## Style de code

- Formate avec `rustfmt` (aucune déviation manuelle).
- Fonctions courtes et documentées ; respecte la densité de commentaires et le nommage
  environnants.
- Le code multiplateforme passe par le trait `platform` — ne transforme pas le code
  commun en spaghetti de `#[cfg]`. Un nouveau comportement OS va dans le backend de cet OS.
- Les chemins sensibles (signature, extraction, chemins venant d'un manifest) doivent
  valider les entrées (pas de path traversal, vérifier la signature avant d'appliquer).
  Voir `docs/SIGNING.md`.

## Commits & pull requests

- **Branche** depuis `main` ; un changement ciblé par PR.
- **Messages de commit** : impératif présent (`Add …`, `Fix …`, `Refactor …`).
  Les préfixes conventional-commit (`feat:`, `fix:`, `docs:`, `chore:`) sont les bienvenus.
- **Description de PR** : quoi + pourquoi, comment testé, et les notes par plateforme
  (as-tu build/testé sous Linux + Windows ?).
- Garde la barrière verte et mets à jour la doc quand le comportement change.

## Ouvrir une issue

Cherche d'abord dans les issues existantes. Puis choisis le bon type :

### 🐛 Rapport de bug — inclure :
- Version de BetterInstaller / `bpkg` et **OS** (Windows / Linux / macOS + version).
- **Étapes pour reproduire** (la commande `bpkg`/installeur exacte, ou le parcours de clics).
- **Attendu vs obtenu**, plus la sortie d'erreur complète.
- Ton `installer.toml` (masque les secrets) et si le paquet était signé.

### ✨ Demande de fonctionnalité — inclure :
- Le problème que tu résous (pas juste la solution).
- Qui en bénéficie et un cas d'usage approximatif.
- Les alternatives envisagées.

### 🔒 Sécurité
N'ouvre **pas** d'issue publique pour une vulnérabilité. Signale-la en privé (voir
`SECURITY.md` s'il existe, sinon contacte le mainteneur). Les bugs de signature, de
sandbox et d'intégrité de paquet sont traités en priorité haute.

## Publier une release (mainteneurs)

Voir `examples/bmm/release.ps1` et `docs/UPDATER-SETUP.md` — bump des versions → build →
delta → `update.json` multi-source → `gh release create` avec les 3 (ou 4, avec un delta)
assets.
