# Packaging multiplateforme

[🇬🇧 English](PLATFORMS.md) · 🇫🇷 Français

**Un `installer.toml`, un moteur, trois OS.** Le moteur est écrit une fois contre un
trait `PlatformOps` ; chaque OS a un backend qui fait raccourcis / protocole /
désinstallation à la manière native. Tu livres un **payload par OS + un binaire moteur
par OS**, mais la config et tout le flux sont identiques.

| | Windows | Linux | macOS |
|---|---|---|---|
| Binaire moteur | `betterinstaller.exe` | `betterinstaller` | `betterinstaller` |
| Sortie Setup | `App-Setup.exe` | `App-Setup` (ou AppImage) | `App-Setup` (ou `.app`/`.dmg`) |
| Dossier d'install par défaut | `%LOCALAPPDATA%\Programs\<nom>` | `~/.local/opt/<id>` | `/Applications/<nom>.app` |
| Dossier de données | `%APPDATA%\<id>` | `~/.config/<id>` | `~/Library/Application Support/<id>` |
| Raccourcis | `.lnk` (Menu Démarrer / Bureau) | `.desktop` (`~/.local/share/applications`) | symlink dans `~/Applications` |
| Protocole URL | `HKCU\Software\Classes\<scheme>` | `.desktop` + `xdg-mime` | Info.plist `.app` `CFBundleURLTypes` |
| Entrée de désinstallation | ARP (Apps & Features) | — (suppr. dossier + `.desktop`) | — (suppr. `.app`) |
| Élévation | par-utilisateur, `asInvoker` (pas d'UAC) | par-utilisateur (`~`) | par-utilisateur (`~`) |

Détails par OS : **[Windows](platform-windows.md) · [Linux](platform-linux.md) ·
[macOS](platform-macos.md)**.

## Ce qui est identique partout

- L'**`installer.toml`** (un seul fichier). `[app].platforms` liste tes cibles.
- Le format **`.bpkg`**, la signature, le manifest, les composants, le handoff, la logique d'update.
- Le **flux GUI** (Bienvenue → Conditions → Setup → Install → Terminé) et le mode maintenance.
- Le contrat de **handoff** 1er lancement (l'app lit `installer-handoff.json` une fois).

## Ce qui diffère (et est géré pour toi)

Uniquement les étapes qui mutent l'OS — raccourcis, protocole URL, enregistrement de la
désinstallation, dossiers install/données par défaut, et élévation. Le backend
`PlatformOps` de l'OS pour lequel le binaire a été compilé est sélectionné
automatiquement (`platform::current()`) ; ta config et ton payload ne branchent pas sur
l'OS dans le code.

## Builder pour chaque OS

Build le moteur **sur chaque OS cible** (ou cross-compile), puis pack + stamp là :

```sh
# sur chaque OS :
cargo build --release -p bpkg-cli -p installer        # → bpkg + betterinstaller(.exe)

bpkg pack  --root payload-<os> --config installer.toml --out app-<os>.bpkg
bpkg sign  --key keys/private.key app-<os>.bpkg
bpkg build --installer ./target/release/betterinstaller[.exe] \
           --config installer.toml --package app-<os>.bpkg --out App-Setup-<os>[.exe]
```

- **Payload par OS** — le *contenu* diffère (un `.exe` Windows vs un ELF Linux vs un
  `.app` macOS), donc garde un `payload-windows/`, `payload-linux/`, `payload-macos/`.
- **Même clé** — signe tous les paquets OS avec la même clé Ed25519 si tu veux un seul
  `[security].public_key` pour tous les vérifier.
- **`[[components]].paths`** sont en slash avant, agnostiques de l'OS.

> Une seule matrice CI (windows-latest / ubuntu-latest / macos-latest) peut produire les
> trois setups depuis le même dépôt — voir [../.github/workflows/ci.yml](../.github/workflows/ci.yml).

## Conseils pour une app qui vise les trois

1. Garde `[app].platforms = ["windows", "linux", "macos"]`.
2. `main_exe` / `protocol` sont les mêmes noms logiques ; le backend les mappe nativement.
3. Le **lecteur de handoff** 1er lancement de l'app doit utiliser le dossier de données
   de cet OS (l'installeur écrit déjà au bon endroit par OS). La plupart des toolkits
   (ex. Tauri) te donnent le dossier de données par-OS gratuitement.
4. Teste le protocole URL sur chaque OS — c'est la partie la plus spécifique à l'OS
   (registre vs `xdg-mime` vs Info.plist).
