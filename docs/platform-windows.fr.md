# Windows

[🇬🇧 English](https://github.com/FreeProject089/BetterInstaller/blob/master/docs/platform-windows.md) · 🇫🇷 Français

Le backend par défaut + le plus complet. **Par-utilisateur** (HKCU + profil utilisateur),
donc rien ne nécessite de droits administrateur — en accord avec le manifeste `asInvoker`
livré par le moteur.

## Emplacements

| Quoi | Chemin |
|---|---|
| Dossier d'install par défaut | `%LOCALAPPDATA%\Programs\<app.name>` |
| Données app (handoff) | `%APPDATA%\<app.id>` |
| Raccourci Menu Démarrer | `%APPDATA%\Microsoft\Windows\Start Menu\Programs\<name>.lnk` |
| Raccourci Bureau | `%USERPROFILE%\Desktop\<name>.lnk` |
| Protocole URL | `HKCU\Software\Classes\<scheme>` |
| Entrée de désinstallation (ARP) | `HKCU\…\CurrentVersion\Uninstall\<app.id>` |
| Entrée PATH | `HKCU\Environment` → `Path` |

## Ce que fait le moteur

- **Raccourcis** — vrais fichiers `.lnk` (via `mslnk`), Menu Démarrer toujours, Bureau
  si `[install].desktop_shortcut = true`.
- **Protocole** — enregistre `<scheme>://` sous `HKCU\Software\Classes` avec
  `shell\open\command = "<exe>" "%1"`.
- **Désinstalleur** — écrit une entrée Apps & Features dont l'`UninstallString` est
  `"<install>\uninstall.exe" --uninstall` (une copie du setup), plus un
  `uninstall-info.json` pour que la désinstallation annule exactement ce qu'elle a fait.
- **Détection d'install existante** — lit l'`InstallLocation` / `DisplayVersion` ARP
  (pilote le mode maintenance + le bouton Update).

## Élévation / Program Files

Le manifeste est `asInvoker` → **pas de prompt UAC**, mais tu ne peux écrire que là où
l'utilisateur peut. Installer dans `C:\Program Files` nécessite un build élevé
(`requireAdministrator`) ; le GUI affiche une erreur claire si le dossier choisi n'est
pas accessible en écriture, et le bouton **Parcourir…** permet de choisir un emplacement
accessible.

## Build

```powershell
cargo build --release -p bpkg-cli -p installer
./examples/bmm/build-installer.ps1        # pack → sign → stamp → BMM-Setup.exe
```

## SmartScreen / signature de code

La signature Ed25519 couvre l'**intégrité du paquet** (le moteur la vérifie avant
d'installer). Elle ne donne **pas** de réputation Windows — pour ça, signe le
`*-Setup.exe` en Authenticode avec un certificat de signature de code (un cert EV passe
SmartScreen le plus vite). Signe le setup **tamponné**, après `bpkg build` : le
certificat est ajouté après la config et le paquet embarqués, le moteur les retrouve
devant lui, et la signature authentifie alors aussi `installer.toml` — que la signature
Ed25519 du paquet ne couvre pas (voir [SIGNING.md](SIGNING.md)).

Le moteur ne charge les DLL que depuis System32 (`/DEPENDENTLOADFLAG:0x800` à l'édition
de liens, `SetDefaultDllDirectories` au démarrage) : une DLL déposée par un navigateur à
côté du setup dans Téléchargements n'y est pas chargée.

## Pièges

- L'identifiant de bundle (`[app].id`) doit être égal à l'identifiant du dossier de
  données de l'app — le handoff est écrit dans `%APPDATA%\<id>` ; un décalage = l'app ne
  le lit jamais.
- La désinstallation supprime le dossier d'install entier **seulement si l'installation
  l'a créé** (il n'existait pas, ou était vide). Installé dans un dossier qui contenait
  déjà des fichiers — « Parcourir… » prend le dossier choisi tel quel, par ex.
  `D:\Games` — elle supprime les fichiers installés et les dossiers qu'ils laissent vides,
  rien d'autre. Les deux faits sont notés dans `uninstall-info.json` à l'installation.
  Une racine de lecteur, un dossier juste en dessous, ou le dossier personnel ne sont
  jamais supprimés entiers.
- Le désinstalleur se supprime lui-même (`cmd` détaché) après avoir retiré les fichiers,
  et ferme d'abord les exécutables de l'app (les `.exe` de premier niveau du paquet) pour
  que les fichiers ne soient pas verrouillés.
