<div align="center">

# BetterInstaller

**Un moteur, un `installer.toml` → un installeur auto-extractible signé + un auto-updater.**
Une alternative multiplateforme à NSIS / MSI pour n'importe quelle app desktop — UI native
Slint (pas de runtime WebView), handoff de config au 1er lancement, et updates avec rollback.

![Plateformes](https://img.shields.io/badge/plateformes-Windows%20%C2%B7%20Linux%20%C2%B7%20macOS-blue?style=flat-square)
![UI](https://img.shields.io/badge/UI-Slint%20(natif)-7c3aed?style=flat-square)
![Signature](https://img.shields.io/badge/signature-Ed25519-green?style=flat-square)
![CI](https://img.shields.io/badge/CI-fmt%20%C2%B7%20clippy%20%C2%B7%20test-success?style=flat-square)

🇫🇷 Français · [🇬🇧 English](README.MD)
[Démarrage rapide](#démarrage-rapide) · [Documentation](#-documentation) · [Développer](#développer) · [Contribuer](CONTRIBUTING_FR.md)

</div>

---

## Démarrage rapide

```sh
# 1. Build du moteur + CLI
cargo build --release -p bpkg-cli -p installer

# 2. Pack → sign → stamp ton app en un seul Setup.exe
bpkg keygen --out keys
bpkg pack  --root payload --config installer.toml --out app.bpkg
bpkg sign  --key keys/private.key app.bpkg
bpkg build --installer ./target/release/betterinstaller[.exe] \
           --config installer.toml --package app.bpkg --out App-Setup[.exe]
```

Un exemple réel complet (avec scripts build + release en une commande) est dans
[`examples/bmm/`](examples/bmm/) — voir [GUIDE.md](GUIDE.md) pour chaque champ de config.

## 📚 Documentation

- **[GUIDE.md](GUIDE.md)** — configurer `installer.toml` (tous les champs).
- **[docs/](docs/README.md)** — architecture · format `.bpkg` · CLI · handoff ·
  updates · **[config de l'updater (GitHub / serveur perso / multi-source)](docs/UPDATER-SETUP.md)** ·
  signature · **[multiplateforme](docs/PLATFORMS.md)**.
- **[CONTRIBUTING_FR.md](CONTRIBUTING_FR.md)** ([EN](CONTRIBUTING.md)) — setup, barrière CI, issues & PRs.

## Développer

```sh
# Rejouer la barrière CI exacte en local sous Linux (fmt + clippy + test + build), via Docker :
docker build -t betterinstaller-dev .
docker run --rm -e CARGO_TARGET_DIR=/tmp/t -v "${PWD}:/app" betterinstaller-dev
```

<sub>Historique de conception : `../.Assets/.md/PLAN_BETTER_INSTALLER.md`.</sub>

---

## Pourquoi

- **Réutilisable** — le moteur ne code jamais un projet en dur. Chaque app fournit son
  `installer.toml`. N'importe quelle app, dans n'importe quel langage, peut l'adopter
  (elle lit juste un fichier JSON au 1er lancement).
- **Natif & beau** — UI en [Slint](https://slint.dev) : un seul binaire statique, **pas de
  runtime WebView2**, rendu GPU, thémable.
- **Portable & léger** — un exe autonome ; profil release visé `<5 Mo` ; payload zstd.
- **Sécurisé** — SHA-256 par fichier, **signature Ed25519**, intégrité vérifiée à l'install.
- **Handoff au 1er lancement** — l'installeur pré-configure l'app (langue, confidentialité/CGU,
  tutoriel, télémétrie, composants) → **aucune modale au premier démarrage**.
- **Cycle de vie complet** — installer · configurer · mettre à jour (avec rollback) · réparer · désinstaller.

---

## Ce qui est implémenté

| Domaine | Statut |
|---|---|
| Format de paquet `.bpkg` (build / verify / extract, testé en aller-retour) | ✅ |
| CLI `bpkg` : `pack`, `info`, `verify`, `extract`, `install`, `build`, `keygen`, `sign`, `update` | ✅ |
| GUI Slint : Bienvenue → Conditions → Setup → Installation → Terminé | ✅ |
| **Mode maintenance** GUI (réparer / mettre à jour / désinstaller, avec confirmation) | ✅ |
| **Étape Conditions** : CGU + Confidentialité rendues en markdown, acceptation par doc | ✅ |
| **Lancement post-install** (option « lancer maintenant » sur la page Terminé) | ✅ |
| **Parcourir** (sélecteur de dossier) + vérif d'écriture préalable | ✅ |
| **Branding** : logo + nom de l'app dans le sidebar (`[branding].logo`) | ✅ |
| **Thémable** : re-skin complet de l'installeur via `[theme]` dans le TOML | ✅ |
| **Pré-import** : bundle langues/thèmes/preset, opt-in au 1er lancement | ✅ |
| **Config auto-update** (`[update]` : manifest distant, deltas, multi-source, auto-check) | ✅ |
| Écran Terminé adapté à l'action (installé / mis à jour / réparé / désinstallé) | ✅ |
| Handoff 1er lancement (`installer-handoff.json`) — l'installeur écrit, l'app lit une fois | ✅ |
| Installation réelle (vérif + extraction + progression) | ✅ |
| Intégration système (Windows) : raccourcis, protocole `app://`, désinstalleur ARP, PATH | ✅ |
| Désinstallation (`--uninstall`, annule tout) | ✅ |
| Manifeste `asInvoker` (pas d'UAC parasite) | ✅ |
| **Installeur auto-extractible unique** (`bpkg build` — config + payload dans un exe) | ✅ |
| **Signature Ed25519** + vérif à l'install (`[security]`) | ✅ |
| Vérif des prérequis + auto-téléchargement/-install des manquants | ✅ |
| **Update avec rollback atomique** (local + manifest distant) | ✅ |
| **Patches delta binaires** (bsdiff — patch ~1 % du nouveau fichier) | ✅ |
| i18n (en / fr) pour l'UI | ✅ |
| Packing par composants (`[[components]].paths` → mapping par fichier) | ✅ |
| Désinstalleur auto-suppressif | ✅ |
| Backend Linux (raccourcis `.desktop` + protocole `xdg-mime`) | ✅ |
| Backend macOS (raccourcis symlink + chemins ; protocole via Info.plist) | ✅ / partiel |
| CI (GitHub Actions : fmt + clippy + test + release, Windows & Linux) | ✅ |

**Pas encore** (honnête, mineur) : enregistrement runtime du protocole macOS (il est
déclaré dans l'Info.plist du `.app`, pas au runtime) ; changement de langue réactif en
cours de flux. SmartScreen requiert toujours Authenticode (Ed25519 ne couvre que
l'intégrité du paquet, pas la réputation Windows).

---

## Référence `installer.toml` (exemple)

```toml
[app]
id = "com.acme.editor"; name = "Acme Editor"; version = "1.0.0"
publisher = "BetterCommunity"; platforms = ["windows"]

[install]
default_dir = "{ProgramFiles}/Acme Editor"
main_exe = "acme-editor.exe"           # pour les raccourcis + le protocole
protocol = "acme"                      # enregistre les deep links acme://
create_shortcuts = true; desktop_shortcut = true; allow_portable = true

[[components]]                          # parties sélectionnables (ex. sidecar)
id = "mcp-server"; name = "MCP AI Server"; required = false; default = true; size_mb = 7

# Handoff au 1er lancement → pas de modale au démarrage
[handoff]
enabled = true; file = "installer-handoff.json"; location = "app_data"

[[setup_option]]
id = "language"; type = "select"; label_key = "setup.language"
choices = ["auto","en","fr"]; default = "auto"; maps_to = "settings.language"

[[setup_option]]
id = "legal"; type = "license"; label_key = "setup.acceptLegal"; required = true
documents = ["PRIVACY.md","TOS.md"]
maps_to = ["settings.privacy_accepted","settings.tos_accepted"]

# Imposer la signature du paquet à l'install (optionnel)
[security]
public_key = "<hex depuis keys/public.key>"
require_signature = true

# Mise à jour
[update]
manifest_url  = "https://…/update.json"   # source principale (URL stable)
manifest_urls = []                        # sources supplémentaires optionnelles (mirrors)
auto_check    = true
allow_delta   = true
```

Voir [GUIDE.md](GUIDE.md) pour la référence complète de chaque champ.

---

## Le contrat de handoff (1er lancement)

L'installeur écrit `installer-handoff.json` (réglages + composants) dans le dossier de
données de l'app. L'app le lit **une fois** au 1er lancement, l'applique, puis le renomme
en `.consumed.json`. Agnostique — le moteur ne touche jamais au format de config interne
d'une app.

```json
{ "schema": 1, "source": "betterinstaller", "app_version": "1.0.0",
  "components": ["core","mcp-server"],
  "settings": { "language": "fr", "privacy_accepted": true, "tos_accepted": true,
                "skip_tutorial": true, "telemetry": false } }
```

> ⚠️ Le handoff **n'est pas automatique** : tu dois ajouter quelques lignes dans ton app
> pour lire ce fichier au 1er lancement. Voir [docs/HANDOFF.md](docs/HANDOFF.md).

---

## Tester

```sh
cargo test --workspace      # tests unitaires (handoff, embed, sign, update+rollback)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Le cycle de vie CLI complet (pack → sign → verify → install → build → update) et le
lancement sans-args d'un installeur auto-extractible stampé sont vérifiés de bout en bout.
