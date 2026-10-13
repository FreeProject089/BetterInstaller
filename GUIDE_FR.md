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

[theme]
mode = "auto"            # auto (suit le mode clair/sombre des applis de l'OS) | dark | light
```

L'installeur a une palette sombre et une claire et, avec `mode = "auto"` (le défaut), s'ouvre
dans celle que Windows utilise pour les applis (Paramètres → Personnalisation → Couleurs). Toute
couleur de `[theme]` (`bg`, `panel`, `accent`, `text`, …) remplace celle de la palette, dans les
deux modes.

### `[install]`

```toml
[install]
main_exe         = "myapp.exe"  # pour les raccourcis + le protocole
protocol         = "acme"                       # enregistre les deep links acme://
create_shortcuts = true
desktop_shortcut = true
autostart        = false   # propose « Lancer avec Windows », décoché (absent = pas proposé)
```

Une installation personnalisée les montre à l'étape **Emplacement et composants** : le
dossier, les composants, puis « Raccourci dans le menu Démarrer », « Raccourci sur le Bureau » et
(avec `autostart`) « Lancer avec Windows », chacun avec une ligne sur ce que cocher change.
« Lancer avec Windows » écrit une valeur par-utilisateur `HKCU\…\CurrentVersion\Run` nommée
d'après `app.id` (sans élévation) ; le désinstalleur retire toujours cette valeur. Une
installation recommandée prend les valeurs ci-dessus.

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

Le **manifest** est du JSON **signé avec ta clé d'éditeur**, écrit par
`bpkg update-manifest` et valable 7 jours au plus (renouvelle-le avec
`bpkg resign-manifest`). Avec un `public_key` défini, un manifest non signé ou expiré est
refusé. Format et règles : [docs/UPDATES.fr.md](docs/UPDATES.fr.md).

Quand l'app est déjà installée et que tu relances le setup (ou qu'il est ouvert via
l'entrée ARP), BetterInstaller vérifie le manifest en arrière-plan. S'il annonce une
version plus récente, le bouton **Mettre à jour** apparaît et télécharge + applique
(en utilisant un delta depuis la version installée si proposé), avec **rollback
automatique** en cas d'échec. Crée des deltas avec `bpkg delta --old old.bpkg --new new.bpkg --out patch`.

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
- **`select`** → un menu déroulant → string JSON. Le menu affiche des libellés traduits
  et écrit la valeur tirée de `choices`. **`"auto"` est spécial :** l'installeur le résout
  vers la langue dans laquelle il est affiché (celle de l'OS sauf si l'utilisateur en a
  choisi une autre), en prenant la première de sa chaîne de repli présente dans `choices` ;
  laisser le défaut donne donc quand même une valeur concrète à l'app (ça corrige
  « selects pas appliqués »).
- **`license`** → avec `documents = ["TOS.md","PRIVACY.md"]` ça devient une étape
  **Conditions** dédiée : chaque document est rendu (markdown) sur sa **propre page avec sa
  propre case Accepter**, et tous doivent être acceptés pour continuer. Mappe son
  acceptation vers chaque clé `maps_to` (ex. `tos_accepted` + `privacy_accepted`). Les
  versions traduites vont dans `localized_documents` (voir [Langues](#langues-i18n-et-setup_group)).

Sur chaque option aussi : `group = "<id>"` la range sous un titre `[[setup_group]]`, et
`sends_data = true` ajoute une marque « Envoie des données » à côté du libellé. Chaque
ligne affiche sa valeur par défaut (« Par défaut : Activé ») et une marque « Modifié » dès
que l'utilisateur s'en écarte ; un bouton « Rétablir les valeurs par défaut » remet la
page à zéro.

Une option `bool` peut aussi indiquer `installs = "<id de prérequis>"` : si elle est cochée à
l'installation, ce `[[prerequisite]]` optionnel (qui doit avoir `download_url` et `sha256`) est
téléchargé aussi, et il n'est pas proposé comme ligne à part — la fonction et les fichiers
qu'elle demande sont un seul choix. Le `check_file` d'un prérequis zip peut être RELATIF
(`models/x/model.onnx`) : il est alors cherché sous le dossier d'installation, pour qu'une
réparation ne le retélécharge pas. Les prérequis zip sont écrits sur le disque au fil du
téléchargement et hachés au passage : un gros fichier ne tient jamais entier en mémoire.

`required = true` bloque **Suivant/Installer** tant que non satisfait. Les clés `maps_to`
sont écrites à plat dans `settings` après retrait du préfixe `settings.`.

#### Cartes de choix et préréglages (`cards`)

Une option `bool` ou `select` peut s'afficher en **cartes**, une par valeur, chacune avec un
titre et une ligne sur sa conséquence, au lieu d'un interrupteur ou d'un menu déroulant :

```toml
[[setup_option]]
id       = "privacy_choice"
type     = "select"
choices  = ["essential", "all", "custom"]
default  = ""            # aucune carte présélectionnée…
required = true          # …donc l'étape ne se passe pas tant qu'aucune n'est choisie
maps_to  = "settings.privacy_choice"
group    = "privacy"
[[setup_option.cards]]
value       = "essential"
label       = "Essential only"
description = "No statistics, no telemetry …"
sets        = [ { option = "telemetry", value = false } ]
[[setup_option.cards]]
value       = "all"
label       = "Accept all"
sets        = [ { option = "telemetry", value = true } ]
[[setup_option.cards]]
value       = "custom"
label       = "Customize"
custom      = true
```

- `recommended = true` ajoute une marque « Recommandé » à une carte.
- Une carte avec `sets` est un **préréglage** : tant qu'elle est choisie, elle décide des options
  qu'elle liste (et de leurs enfants `parent`), et ces options sont masquées. La carte `custom`
  (une au plus, sans `sets`) les affiche à la place, en partant des réponses de la carte choisie
  avant elle.
- Vérifié au chargement : chaque carte est une valeur de son option, chaque `sets` nomme une
  option réelle avec une valeur qu'elle peut prendre, une option n'est gouvernée que par un seul
  préréglage.
- Traductions : `options.<id>.choices.<valeur>` (titre) et `options.<id>.cards.<valeur>` (ligne).

`default_light = "<choix>"` sur un `select`/`swatch` remplace `default` quand l'OS est en mode
clair (un thème clair pour qui lit toutes ses applis en clair).

#### Installation recommandée ou personnalisée

La page Bienvenue demande **comment** installer. **Personnalisée** affiche une étape
Emplacement et composants, puis une étape par `[[setup_group]]`. **Recommandée** liste ce
qu'elle va faire (dossier, composants, téléchargements, raccourcis) et n'affiche que les groupes
marqués `always_ask = true`, ou contenant un choix en cartes obligatoire sans défaut (un choix de
confidentialité doit être celui de la personne) ; toutes les autres options prennent leur défaut
déclaré.

```toml
[[setup_group]]
id         = "privacy"
label      = "Privacy & data"
always_ask = true
```

**Les options de pré-import** sont juste des options `bool` dont l'app honore la clé :
```toml
[[setup_option]]
id = "import_themes"
type = "bool"
label = "Importer le pack de thèmes de départ"
default = true                          # coché, mais l'utilisateur peut décocher → pas d'import
maps_to = "settings.import_starter_themes"
```

### Langues : `[i18n]` et `[[setup_group]]`

L'installeur s'ouvre dans la langue de l'OS (langue d'interface de Windows ;
`LC_ALL`/`LC_MESSAGES`/`LANG` ailleurs), ou celle donnée par `--lang=<tag>`. Sous Windows,
s'il n'a pas de catalogue pour la langue d'affichage, il prend la langue suivante de la liste
Paramètres → Heure et langue qu'il connaît (un Windows en allemand avec le français en deuxième
s'ouvre en français), puis l'anglais. Il propose un
sélecteur de langue dans la barre latérale jusqu'au début de l'installation. Changer de
langue retraduit tous les écrans d'un coup, documents légaux compris ; un document dont le
texte a changé doit être accepté à nouveau.

```toml
[i18n]
default     = "auto"               # ou un tag comme "fr" / "pt-BR" pour l'imposer
locales_dir = "installer-locales"  # dossier DANS le paquet : un <code>.toml par langue

[[setup_group]]                    # titres de la page Configuration, dans l'ordre
id          = "privacy"
label       = "Privacy & data"
description = "What the app may send off this PC."

[[setup_option]]
id = "legal"
type = "license"
documents = ["TOS.md", "PRIVACY.md"]          # anglais, et le repli
localized_documents = [
  { lang = "fr", documents = ["TOS_FR.md", "PRIVACY_FR.md"] },   # même ordre
]
```

**Ajouter une langue, c'est ajouter des fichiers, jamais du code :**

- Les textes de l'installeur lui‑même (boutons, étapes, messages) sont dans les catalogues
  du moteur, `crates/bpkg-core/locales/<code>.toml`, intégrés à la compilation : un nouveau
  fichier est une nouvelle langue après un rebuild.
- Les textes de ton produit vont dans `<locales_dir>/<code>.toml` dans le paquet, chargés au
  démarrage sans rebuild du moteur. Un catalogue produit peut aussi remplacer des textes du
  moteur (sauf les clés réservées au moteur, plus bas) ou apporter une langue que le moteur
  n'a pas. Clés :
  `options.<id>.label` · `options.<id>.description` · `options.<id>.choices.<valeur>` ·
  `groups.<id>.label` · `groups.<id>.description` · `components.<id>.name` ·
  `components.<id>.description` · `launch.<id>.label` · `prereqs.<id>.name` ·
  `docs.<nom>` (titre d'un document légal). L'anglais est dans installer.toml lui‑même ; la
  seule chose qui n'y a pas de champ est le libellé d'un choix de `select`, qui va dans
  `en.toml`.
- Chaque catalogue commence par `[meta] name = "Français"` (affiché dans le sélecteur) et
  peut indiquer `direction = "rtl"`.

**Chaîne de repli :** tag demandé → ses formes plus courtes → anglais (`pt-BR` → `pt` →
`en`), texte par texte. Pour les documents légaux, à chaque étape : l'entrée de
`localized_documents`, puis un fichier voisin `<NOM>_<TAG>.<ext>` dans le paquet
(`TOS_FR.md`), puis les `documents` anglais. Quand un document retombe sur une autre langue
que celle affichée, la page Conditions le signale au‑dessus.

**Droite à gauche :** un catalogue avec `direction = "rtl"` aligne le texte à droite. La mise
en page n'est pas inversée, et le rendu des écritures RTL dépend du moteur de rendu de
Slint ; aucun catalogue RTL n'a encore été testé.

**Signature :** installer.toml (donc `[i18n]`, `group`, `sends_data`, `localized_documents`)
est ajouté à l'installeur EN DEHORS du paquet signé, comme tous les autres champs de la
config. Les catalogues et les documents eux‑mêmes sont des fichiers du paquet, couverts par sa
signature Ed25519. L'installeur ne les lit pas depuis un paquet dont la signature est connue
comme invalide, et l'installation est de toute façon refusée avant d'écrire un seul fichier.

Les catalogues d'un paquet NON signé sont lus — les refuser laisserait en anglais
précisément l'écran qui doit dire « non signé ». Un catalogue est donc du texte non fiable,
et une poignée de clés appartiennent au moteur seul : `signature.*`, `errors.signature_*`,
`setup.sends_data` et `legal.fallback_notice` — tout ce par quoi l'installeur parle DU
paquet au lieu de parler pour lui. Un catalogue produit qui en définit une est ignoré, avec
une ligne sur stderr. Les surcharges bidirectionnelles et les caractères de contrôle sont
retirés de toute chaîne de catalogue et de tout document légal avant affichage.

Les tests de `bpkg-core` vérifient les catalogues du moteur contre l'anglais, et l'exemple BMM
vérifie ses catalogues produit contre chaque clé déclarée par son installer.toml
(`InstallerConfig::translatable_keys`).

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
  },
  "consents": [
    { "doc": "tos", "text_sha256": "<64 hex>", "lang": "fr", "file": "TOS_FR.md",
      "choice": "accepted", "at": "2026-10-13T10:00:00+00:00" },
    { "doc": "privacy", "text_sha256": "<64 hex>", "lang": "fr", "file": "PRIVACY_FR.md",
      "choice": "accepted", "at": "2026-10-13T10:00:00+00:00" }
  ]
}
```

`consents` (absent si rien n'a été accepté) est la preuve de ce que l'étape `license` a fait
accepter : une preuve par document nommé `TOS*`/`EULA*`/`TERMS*` (`tos`) ou `PRIVACY*` (`privacy`),
avec le SHA-256 du texte exact affiché (`bpkg_core::handoff::text_hash` : CRLF → LF, espaces de fin
retirés), sa langue et son fichier, et l'heure de l'installation. Une app qui tient des preuves de
consentement vérifie l'empreinte contre les textes qu'elle livre avant d'y voir une preuve ; une
app qui ne le fait pas ignore le champ.

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

### Ligne de commande : installer, mettre à jour, réparer, modifier, désinstaller sans surveillance

Chaque action de la fenêtre se lance aussi en ligne de commande, sans aucune question. Le
même exécutable (`BMM-Setup.exe`, ou la copie `uninstall.exe` du dossier d'installation)
accepte :

| Option | Effet |
|---|---|
| *(aucune)* | L'installateur interactif, comme avant. |
| `--mode install\|update\|repair\|modify\|uninstall` | Lance cette action sans surveillance. |
| `--update` | Raccourci de `--mode update --passive`. |
| `--uninstall` | Seul : la fenêtre de désinstallation (le bouton d'Applications et fonctionnalités). Avec `--passive` ou `--quiet` : `--mode uninstall`. |
| `--passive` | Fenêtre de progression, aucune question, se ferme seule à la fin (défaut de tous les modes). Un échec reste affiché 15 s. |
| `--quiet`, `--silent` | Aucune fenêtre. Ne demande jamais les droits administrateur : échoue avec le code 4 à la place. |
| `--dir <chemin>` | Dossier d'une nouvelle installation (défaut : le dossier par utilisateur). Pour les autres modes, sert seulement à retrouver une installation absente du registre ; ne la déplace jamais. |
| `--components <a,b>` | Composants à installer (install, modify). Les composants requis sont toujours ajoutés ; un prérequis optionnel s'écrit `prereq:<id>`. |
| `--preset recommended\|minimal\|full` | Composants quand `--components` est absent : requis + par défaut (recommended, le défaut), requis seulement, ou tous. |
| `--shortcuts <liste>` | `desktop`, `startmenu`, `autostart` ou `none` (install, modify). Défaut d'une installation : ce que la page Emplacement coche ; update/repair gardent ce qui a été enregistré. |
| `--set <option>=<valeur>` | Répond à une option (`id` d'un `[[setup_option]]`) : un booléen prend `on`/`off`, une liste ou une palette une de ses `choices`. Jamais une licence. `--help` liste les options de cet installateur. |
| `--telemetry on\|off` | Raccourci de `--set telemetry=on\|off`. |
| `--accept-terms`, `--accept-privacy` | Accepte les Conditions d'utilisation / la Politique de confidentialité livrées dans le paquet. Obligatoires pour une nouvelle installation. |
| `--lang <code>` | Langue de l'installateur (aussi `--lang=<code>`). Décide aussi ce que devient une liste laissée sur `auto`, et la langue des textes juridiques acceptés. |
| `--wait-pid <pid>` | Attend d'abord la fin de ce processus (60 s par défaut, `--wait-timeout <s>`), puis ferme l'application si elle tourne encore. |
| `--launch` | Lance l'application à la fin (`[install] main_exe`). |
| `--log <fichier>` | Ajoute un journal horodaté de l'exécution (également écrit sur stderr). |
| `--no-restart` | Accepté pour les scripts : l'installateur ne redémarre jamais l'ordinateur. |
| `--offline` | `--update` seulement : ne pas interroger le réseau. |
| `--allow-downgrade` | repair / modify : autorise un setup plus ancien que la version installée. |
| `--check-update` | Affiche un rapport JSON de mise à jour et quitte (0 à jour, 10 disponible, 2 erreur). |
| `--version`, `--help` | Affiche la version / l'aide (avec les composants et options de cet installateur). |

Ce que fait chaque mode :

- **install** — une nouvelle installation, exactement comme la fenêtre : vérification du
  paquet, composants choisis, raccourcis, protocole, entrée d'Applications et fonctionnalités,
  puis écriture du transfert de premier lancement (handoff). Refusée (code 9) si
  l'application est déjà installée.
- **update** — retrouve l'installation existante (entrée d'Applications et fonctionnalités,
  sinon un `uninstall-info.json` de cette application dans `--dir` ou à côté de l'exécutable)
  et installe la version embarquée par-dessus, en gardant son dossier, ses composants et ses
  raccourcis. Le handoff n'est pas réécrit : les réponses et consentements donnés à
  l'installation restent tels quels. Si la version embarquée n'est pas plus récente, le
  manifeste signé `[update]` est interrogé (téléchargement, vérification, retour arrière,
  comme le bouton Mettre à jour) ; rien de plus récent nulle part est un succès (code 0).
  Jamais de retour à une version antérieure. Aucune installation : code 3, jamais une nouvelle
  installation (elle exige le consentement de la personne).
- **repair** — revérifie et réextrait la version embarquée avec les choix enregistrés.
- **modify** — change les composants (`--components` / `--preset` ; les fichiers d'un
  composant retiré sont supprimés) et les raccourcis (`--shortcuts`). Les réglages se changent
  dans l'application.
- **uninstall** — la même désinstallation que la fenêtre.

Règles valables dans tous les modes :

- **La vérification est obligatoire.** La signature du paquet (contre
  `[security] public_key`) et le SHA-256 de chaque fichier sont vérifiés comme pour un clic
  sur Installer ; un paquet qui ne se vérifie pas est refusé avant toute écriture (code 5).
  Aucune option ne la contourne.
- **Le consentement n'est jamais présumé.** Une nouvelle installation sans surveillance refuse
  de démarrer (code 11) sans `--accept-terms` pour les conditions et `--accept-privacy` pour la
  politique de confidentialité, et affiche chaque document avec le SHA-256 de son texte. Avec
  elles, le handoff contient un reçu par document — document, SHA-256 du texte exact, langue,
  fichier, heure et `"source": "cli"` (les reçus de la fenêtre portent `"ui"`).
- **Les données restent coupées sauf mention.** Toute option `sends_data = true` est désactivée
  sauf si elle est donnée par `--set` / `--telemetry` ; l'enfant d'un parent activé prend sa
  valeur déclarée, comme dans la fenêtre. Un choix dont la carte par défaut activerait l'envoi
  de données doit recevoir une réponse explicite : « Tout accepter » n'est jamais sous-entendu.
  Un choix requis sans défaut (le choix de confidentialité de BMM) reste sans réponse sauf
  `--set`, et l'application le pose au premier lancement.
- **L'application en cours d'exécution.** Avant de remplacer des fichiers, les exécutables de
  l'application dans le dossier d'installation sont fermés ; s'il en reste un : code 7.
  `--wait-pid` permet à l'application de passer son propre pid puis de quitter.
- **Les droits administrateur** sont demandés une seule fois (UAC) quand le dossier les exige ;
  la copie élevée fait le travail et son code de sortie est renvoyé. Invite refusée, ou
  `--quiet` : code 4.
- **L'espace disque** est vérifié d'abord, d'après les fichiers restant à écrire (code 6). Les
  téléchargements de prérequis ne sont pas comptés.
- **Fermer la fenêtre de progression** demande l'annulation : elle prend effet avant toute
  écriture (code 8) ; une fois l'écriture commencée, l'exécution va à son terme.

Codes de sortie (10 est le « mise à jour disponible » de `--check-update`, inutilisé ici) :

| Code | Signification |
|---|---|
| 0 | Succès (pour `--update`, aussi « déjà à jour ») |
| 1 | Échec (réseau, prérequis, écriture du handoff, autre) |
| 2 | Arguments invalides (option inconnue, valeur invalide, combinaison absurde) |
| 3 | Aucune installation existante (update, repair, modify, uninstall) |
| 4 | Droits administrateur nécessaires et refusés (ou `--quiet`) |
| 5 | Échec de la vérification de signature ou d'empreinte du paquet |
| 6 | Espace disque insuffisant / erreur d'écriture disque |
| 7 | L'application tourne et n'a pas pu être fermée |
| 8 | Annulé |
| 9 | Déjà installée (`--mode install`) |
| 11 | Nouvelle installation sans `--accept-terms` / `--accept-privacy` |
| 12 | Le setup est plus ancien que la version installée (repair, modify sans `--allow-downgrade`) |

Exemples :

```powershell
# Le programme de mise à jour intégré de BMM : BMM passe son pid, quitte, et est relancé à la fin.
BMM-Setup.exe --update --wait-pid 1234 --launch

# Réparation silencieuse, journalisée.
BMM-Setup.exe --mode repair --quiet --log "$env:TEMP\bmm-repair.log"

# Désinstallation silencieuse (depuis la copie installée).
& "$env:LOCALAPPDATA\Programs\Better Mods Manager\uninstall.exe" --uninstall --quiet

# Nouvelle installation sans surveillance : consentement donné explicitement, envoi de données
# coupé, réponse de confidentialité enregistrée, sans serveur MCP, en français, raccourcis
# Bureau + menu Démarrer.
BMM-Setup.exe --mode install --quiet --accept-terms --accept-privacy `
  --set privacy_choice=essential --components core,cli --shortcuts desktop,startmenu `
  --lang fr --log C:\Temp\bmm-install.log

# Ajouter le serveur MCP à une installation existante.
BMM-Setup.exe --mode modify --components core,cli,mcp-server
```

Remarques : une copie élevée lancée par UAC avec un autre compte administrateur écrit les
entrées par utilisateur de ce compte (raccourcis, Applications et fonctionnalités) ; installez
les dossiers par utilisateur sans élévation. En `--quiet`, la sortie va sur stderr (attaché à
la console appelante) et dans `--log`.

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
6. Pour l'auto-update : pose `[update].manifest_url`, héberge le `update.json` écrit
   par `bpkg update-manifest`, et renouvelle-le chaque semaine (`bpkg resign-manifest`).
