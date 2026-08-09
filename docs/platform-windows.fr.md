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
SmartScreen le plus vite). C'est orthogonal à la signature de paquet `[security]`.

## Pièges

- L'identifiant de bundle (`[app].id`) doit être égal à l'identifiant du dossier de
  données de l'app — le handoff est écrit dans `%APPDATA%\<id>` ; un décalage = l'app ne
  le lit jamais.
- Le désinstalleur se supprime lui-même (`cmd` détaché + `rmdir /S /Q`) après avoir
  retiré le dossier d'install, et tue d'abord l'app en cours pour que les fichiers ne
  soient pas verrouillés.
