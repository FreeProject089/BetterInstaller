# BetterInstaller — Guide de configuration complet

[🇬🇧 English](GUIDE.md) · 🇫🇷 Français

BetterInstaller transforme **un `installer.toml` + un dossier payload** en un seul
`*-Setup.exe` signé et auto-extractible (pas de NSIS/MSI, pas de runtime WebView — une GUI
native Slint en un binaire). Il gère aussi le **handoff de config au 1er lancement**,
l'**auto-update avec rollback**, et un **mode maintenance** (réparer / mettre à jour /
désinstaller).

> *« Un TOML pour les gouverner tous. »* Une config pilote tout l'installeur.

---

## 1. Les concepts en 30 secondes

| Pièce | Ce que c'est |
|---|---|
| **`bpkg`** | La CLI : `pack`, `sign`, `build`, `update`, `keygen`, … |
| **`.bpkg`** | Le paquet : une archive signée compressée zstd + un manifest JSON. |
| **`betterinstaller.exe`** | Le moteur GUI (un par plateforme). |
| **`*-Setup.exe`** | `betterinstaller.exe` + ta config + ton `.bpkg` stampés en un fichier. |
| **`installer.toml`** | Tout ce qui est spécifique au projet (ce guide). |
| **`installer-handoff.json`** | Écrit à l'install ; l'app le lit une fois au 1er lancement. |

**Pipeline :** `payload/` → `bpkg pack` → `.bpkg` → `bpkg sign` → `bpkg build` → `*-Setup.exe`.

---

## 2. Démarrage rapide

```sh
# 0. Build du moteur une fois
cargo build --release -p bpkg-cli -p installer

# 1. Génère une paire de clés de signature (garde private.key SECRÈTE, ne la commit jamais)
./target/release/bpkg keygen --out keys
#   → copie keys/public.key dans [security].public_key de installer.toml

# 2. Assemble payload/  (ton exe + sidecars + TOS.md/PRIVACY.md + bundle/)
# 3. Pack + sign + stamp
./target/release/bpkg pack  --root payload --config installer.toml --out app.bpkg
./target/release/bpkg sign  --key keys/private.key app.bpkg
./target/release/bpkg build --installer ./target/release/betterinstaller.exe \
                            --config installer.toml --package app.bpkg --out App-Setup.exe
```

Un exemple complet est dans `examples/` — son script de build automatise chaque étape
ci-dessus (assemble payload → pack → sign → stamp → émet `update.json`) :

```powershell
./examples/<app>/build-installer.ps1     # depuis la racine BetterInstaller
```

---

## 3. Référence `installer.toml`

### `[app]` — identité (requis)

```toml
[app]
id        = "com.acme.editor"   # DOIT être égal à l'identifiant du dossier de données *
name      = "Acme Editor"
version   = "1.0.0"
publisher = "BetterCommunity"
homepage  = "https://…"          # optionnel
platforms = ["windows"]          # windows | linux | macos
```

\* **Critique :** `id` doit correspondre à ce que l'app utilise pour son dossier de
données par-utilisateur (l'identifiant que ton app utilise pour ce dossier). Le fichier
handoff est écrit dans `%APPDATA%/<id>/` ; si `id` est faux, l'app ne le trouve jamais et
**aucun réglage de 1er lancement ne s'applique**.

### `[branding]`

```toml
[branding]
accent     = "#3b82f6"   # couleur d'accent de l'installeur
logo       = "assets/logo.png"
background = "assets/installer-bg.png"
```

### `[install]`

```toml
[install]
main_exe         = "myapp.exe"  # pour les raccourcis + le protocole
protocol         = "acme"                       # enregistre les deep links acme://
create_shortcuts = true
desktop_shortcut = true
```

> **Emplacement d'installation :** il n'y a pas de réglage `default_dir`. L'installeur
> livre un manifeste `asInvoker` (pas de prompt UAC) et propose toujours la racine
> par-utilisateur `%LOCALAPPDATA%\Programs\<name>`, où il peut écrire sans élévation.
> Le bouton **Parcourir…** permet d'en choisir une autre ; `C:\Program Files` nécessite
> un build élevé, et le GUI affiche une erreur claire si le dossier n'est pas accessible.

### `[security]` — signature de paquet (recommandé)

```toml
[security]
public_key        = "8e0647…168b"   # clé publique Ed25519 hex (depuis keygen)
require_signature = true             # refuse d'installer un pkg non signé/invalide
```

La page Bienvenue affiche un badge de confiance :
- **`Signé & vérifié · <publisher>`** — signature valide contre `public_key`.
- **`Paquet non signé · …`** — pas de signature (badge rouge si `require_signature`).
- **`Signature INVALIDE`** — paquet altéré ; l'install est bloquée.

Générer + utiliser une clé :
```sh
bpkg keygen --out keys           # → keys/private.key (SECRÈTE), keys/public.key
bpkg sign   --key keys/private.key app.bpkg
bpkg verify app.bpkg --key keys/public.key   # vérif de cohérence
```

### `[update]` — auto-update (optionnel)

```toml
[update]
manifest_url = "https://…/update.json"   # une URL JSON stable que tu contrôles
auto_check   = true                        # vérifie à l'ouverture de la maintenance
allow_delta  = true                        # préfère un petit patch binaire
```

Le **manifest** est du JSON :
```json
{
  "version": "1.2.0",
  "url": "https://…/App-1.2.0.bpkg",
  "deltas": [{ "from": "1.1.0", "url": "https://…/1.1.0-to-1.2.0.patch" }]
}
```

Quand l'app est déjà installée et que tu relances le setup (ou qu'il est ouvert via
l'entrée ARP), BetterInstaller vérifie le manifest en arrière-plan. S'il annonce une
version plus récente, le bouton **Mettre à jour** apparaît et télécharge + applique
(en utilisant un delta depuis la version installée si proposé), avec **rollback
automatique** en cas d'échec. Crée des deltas avec `bpkg delta old.bpkg new.bpkg patch`.

Pour plusieurs sources (mirrors / serveur perso), ajoute `manifest_urls = ["…"]` : l'updater
prend la version la plus récente parmi toutes les sources joignables. Voir
[docs/UPDATER-SETUP.md](docs/UPDATER-SETUP.md).

Si `[update]` est omis, le bouton **Mettre à jour** apparaît quand même si le setup
*embarqué* est plus récent que l'installé (il ré-extrait le paquet embarqué).

### `[[components]]` — parties d'install optionnelles

```toml
[[components]]
id          = "core"
name        = "Acme Editor"
description = "Application principale — requise."
required    = true       # toujours installé, case décochable désactivée
default     = true        # pré-coché
size_mb     = 43

[[components]]
id          = "mcp-server"
name        = "MCP AI Server (sidecar)"
required    = false
default     = true
size_mb     = 7
paths       = ["acme-helper.exe", "mcp/"]   # chemins payload appartenant à ce composant
```

`paths` sont des préfixes en slash avant ; les fichiers ne correspondant à aucun
appartiennent à `core` et sont toujours installés. Les composants optionnels décochés sont
sautés à l'extraction.

### `[handoff]` — config au 1er lancement (la fonctionnalité phare)

```toml
[handoff]
enabled  = true
file     = "installer-handoff.json"
location = "app_data"    # app_data (par-utilisateur) | install_dir (portable)
```

Écrit un fichier de réglages plat que l'app lit **une fois** au 1er lancement, puis
renomme en `*.consumed.json`. C'est ce qui retire les modales de
confidentialité/CGU/langue/tutoriel du 1er lancement. Voir §4 pour le côté app.

### `[[setup_option]]` — la page Configuration

Chaque option affiche un contrôle et mappe vers une ou plusieurs clés de réglages du handoff.

```toml
[[setup_option]]
id        = "language"
type      = "select"             # bool | select | license
label     = "Langue"
description = "Langue de l'interface."
choices   = ["auto", "en", "fr"] # select uniquement
default   = "auto"
maps_to   = "settings.language"  # une clé, ou ["k1","k2"]
```

- **`bool`** → une case → bool JSON.
- **`select`** → un menu déroulant → string JSON. **`"auto"` est spécial :** l'installeur
  le résout vers la langue OS détectée avant d'écrire le handoff, donc laisser le défaut
  donne quand même une valeur concrète à l'app (ça corrige « selects pas appliqués »).
- **`license`** → avec `documents = ["TOS.md","PRIVACY.md"]` ça devient une étape
  **Conditions** dédiée : chaque document est rendu (markdown) sur sa **propre page avec sa
  propre case Accepter**, et tous doivent être acceptés pour continuer. Mappe son
  acceptation vers chaque clé `maps_to` (ex. `tos_accepted` + `privacy_accepted`).

`required = true` bloque **Suivant/Installer** tant que non satisfait. Les clés `maps_to`
sont écrites à plat dans `settings` après retrait du préfixe `settings.`.

**Les options de pré-import** sont juste des options `bool` dont l'app honore la clé :
```toml
[[setup_option]]
id = "import_themes"
type = "bool"
label = "Importer le pack de thèmes de départ"
default = true                          # coché, mais l'utilisateur peut décocher → pas d'import
maps_to = "settings.import_starter_themes"
```

### `[[launch]]` — « Lancer maintenant » post-install (page Terminé)

```toml
[[launch]]
id = "app"
label = "Lancer Acme Editor"
exe = "myapp.exe"   # relatif au dossier d'install
default = true                     # pré-coché

[[launch]]
id = "mcp"
label = "Démarrer le serveur MCP AI maintenant"
exe = "acme-helper.exe"
default = false                    # opt-in
component = "mcp-server"           # proposé seulement si ce composant a été installé
```

Sur la page finale, elles apparaissent comme des cases opt-in. **Terminer** lance ce qui
est coché (détaché) et ferme ; rien de coché = ferme juste.

---

## 4. Le handoff 1er lancement (côté app)

L'installeur écrit (dans `%APPDATA%/<app.id>/installer-handoff.json`) :

```json
{
  "schema": 1,
  "source": "betterinstaller",
  "app_version": "1.0.0",
  "components": ["core", "mcp-server"],
  "install_dir": "C:\\Users\\me\\AppData\\Local\\Programs\\Acme Editor",
  "settings": {
    "language": "fr",
    "tos_accepted": true,
    "privacy_accepted": true,
    "skip_tutorial": false,
    "telemetry": false,
    "import_starter_themes": true
  }
}
```

L'app doit, **une fois** au 1er lancement :
1. Lire le fichier depuis son propre dossier de données, valider `source == "betterinstaller"`.
2. Appliquer `settings` à sa propre config (borner/valider chaque valeur).
3. Le renommer en `installer-handoff.consumed.json` pour qu'il ne se ré-applique jamais.

Une app typique le lit dans son code de démarrage (valider → appliquer → renommer). Voir
l'exemple bundlé sous `examples/` pour une implémentation concrète.

### Presets de pré-import (livrer du contenu prêt à l'emploi)

Si ton app peut importer son propre fichier d'export/backup, tu peux en livrer un pour
qu'une install fraîche démarre pré-configurée. Dépose le fichier dans le dossier `bundle/`
de ton payload ; le build bundle `bundle/*` dans le dossier d'install. Lie-le à un
`[[setup_option]]` de type `import_*` : quand l'utilisateur laisse cette option cochée, le
handoff renvoie le chemin du fichier bundlé et ton app l'importe au 1er lancement ;
décoché → rien n'est importé.

> La config `examples/` branche ça de bout en bout (une case qui importe un preset
> réglages/thème/langue bundlé) — copie sa disposition `bundle/` comme point de départ.

---

## 5. Mode maintenance (réparer / mettre à jour / désinstaller)

Quand l'app est déjà installée (détectée via l'entrée registre ARP), ou que le setup est
lancé avec `--uninstall` (le bouton « Désinstaller » de Windows fait ça), le GUI s'ouvre en
**mode maintenance** avec trois actions, chacune derrière une confirmation avec **Annuler** :

- **Réparer** — re-vérifie (SHA-256) et restaure la même version.
- **Mettre à jour** — affiché seulement quand une version plus récente existe (manifest
  distant ou paquet embarqué plus récent) ; télécharge/extrait avec rollback.
- **Désinstaller** — annule raccourcis/protocole/entrée ARP et retire le dossier d'install.

---

## 6. Référence CLI (`bpkg`)

| Commande | Rôle |
|---|---|
| `pack --root <dir> --config <toml> --out <pkg>` | Construire un `.bpkg` depuis un dossier. |
| `sign --key <private.key> <pkg>` | Signer un paquet sur place (Ed25519). |
| `verify <pkg> [--key <public.key>]` | Vérifier les hashes (+ signature). |
| `keygen --out <dir>` | Générer `private.key` + `public.key`. |
| `build --installer <exe> --config <toml> --package <pkg> --out <Setup.exe>` | Stamper le SFX. |
| `info <pkg>` / `extract <pkg> --dest <dir>` | Inspecter / décompresser. |
| `install <pkg> --dest <dir>` | Même chemin que le GUI (vérif + extract + progression). |
| `update / fetch-update` | Appliquer un paquet plus récent / vérifier un manifest distant. |
| `delta old new patch` / `apply-delta` | Patches delta binaires. |

---

## 7. Notes multiplateforme

- **Windows :** raccourcis HKCU, protocole, entrée ARP ; install par-utilisateur (pas d'UAC).
- **Linux :** fichiers `.desktop`, protocole `xdg-mime`.
- **macOS :** protocole `Info.plist`, symlink `/Applications`.

Le moteur est écrit une fois contre un trait `PlatformOps` ; chaque OS fournit un backend.

**Comment une installation existante est reconnue.** Windows relit sa propre entrée ARP sous
`HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\<app_id>`. Linux et macOS n'ont pas
de base de registre : ils écrivent un **reçu d'installation** — un JSON par application sous
le répertoire de données de l'utilisateur (`$XDG_DATA_HOME` ou
`~/.local/share/betterinstaller/receipts` sous Linux,
`~/Library/Application Support/BetterInstaller/receipts` sous macOS) contenant l'identifiant
de l'application, la version installée et le dossier d'installation.

C'est ce qui rend le mode maintenance atteignable. Sans lui, l'installeur ne peut pas
distinguer une installation existante d'une nouvelle : il ne propose ni Mettre à jour, ni
Réparer, ni Désinstaller, et une mise à jour n'a aucune version à comparer.

- Supprimez le reçu à la main et le lancement suivant considère l'application non installée.
- Supprimez le *dossier* d'installation en laissant le reçu : le reçu est ignoré — il est
  toujours confronté au disque, donc « Réparer » n'est jamais proposé sur du vide.

Les reçus sont par-utilisateur, comme les installations elles-mêmes : le manifeste est
`asInvoker` et n'élève jamais les privilèges.

---

## 8. Checklist pour une nouvelle app

1. Copie l'exemple sous `examples/` comme template ; édite `installer.toml` (`[app].id` d'abord !).
2. `bpkg keygen` → colle `public.key` dans `[security].public_key`.
3. Mets tes binaires buildés + `TOS.md`/`PRIVACY.md` (+ `bundle/` optionnel) dans le payload.
4. Implémente le lecteur de handoff dans ton app (applique les réglages une fois, marque consommé).
5. `pack → sign → build` (ou copie `build-installer.ps1`).
6. Héberge un `update.json` si tu veux l'auto-update ; pose `[update].manifest_url`.
