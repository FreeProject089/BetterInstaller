# Linux

[🇬🇧 English](platform-linux.md) · 🇫🇷 Français

Par-utilisateur sous `$HOME`, conforme freedesktop. Aucun root nécessaire.

## Emplacements

| Quoi | Chemin |
|---|---|
| Dossier d'install par défaut | `~/.local/opt/<app.id>` |
| Données app (handoff) | `$XDG_CONFIG_HOME/<app.id>` (sinon `~/.config/<app.id>`) |
| Raccourci app | `$XDG_DATA_HOME/applications/<name>.desktop` (sinon `~/.local/share/applications`) |
| Raccourci Bureau | `~/Desktop/<name>.desktop` |
| Protocole URL | un `.desktop` handler + `xdg-mime default …` pour `x-scheme-handler/<scheme>` |

## Ce que fait le moteur

- **Raccourcis** — écrit des lanceurs `.desktop` (`Exec=`, `Name=`, `Icon=` optionnel)
  dans le dossier applications (toujours) et le Bureau (si `desktop_shortcut = true`).
- **Protocole** — dépose un `.desktop` handler déclarant
  `MimeType=x-scheme-handler/<scheme>;` et lance `xdg-mime default <id>.desktop
  x-scheme-handler/<scheme>` (au mieux ; ignoré si `xdg-mime` est absent).
- **Désinstallation** — retire le dossier d'install + les fichiers `.desktop` (pas de
  registre central sous Linux). `uninstall-info.json` note ce qu'il faut annuler.

## Conseils payload

- Livre un binaire ELF normal (et tout sidecar/ressource) dans `payload-linux/`.
- Pour une **icône**, inclus un PNG et pointe `[branding].logo` + le `Icon=` du
  `.desktop` dessus ; pour l'intégration au menu, une icône sous
  `~/.local/share/icons/hicolor/…` aide certains DE.
- Marque le binaire principal exécutable dans ton build avant le pack (le moteur
  préserve le flag `executable` du manifest).

## Choix de distribution

- **`App-Setup` auto-extractible** (ce moteur) — double-clic / `./App-Setup`, installe
  dans `~/.local/opt`. Le plus simple, même UX que Windows/macOS.
- **AppImage / Flatpak / .deb** — si tu veux aussi une distribution store/gestionnaire de
  paquets, construis-les séparément ; BetterInstaller est le chemin par-utilisateur
  inter-distros.

## Build

```sh
cargo build --release -p bpkg-cli -p installer
bpkg pack  --root payload-linux --config installer.toml --out app-linux.bpkg
bpkg sign  --key keys/private.key app-linux.bpkg
bpkg build --installer ./target/release/betterinstaller \
           --config installer.toml --package app-linux.bpkg --out App-Setup
chmod +x App-Setup
```

## Pièges

- Les environnements de bureau cachent les fichiers `.desktop` — un nouveau lanceur peut
  prendre un moment (ou `update-desktop-database`) pour apparaître au menu.
- Le comportement de `xdg-mime` varie selon le DE ; l'enregistrement du protocole est au mieux.
