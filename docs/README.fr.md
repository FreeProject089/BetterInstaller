# BetterInstaller — Documentation

[🇬🇧 English](README.md) · 🇫🇷 Français

Un framework d'installeur/updater multiplateforme : **un `installer.toml` + un dossier
payload → un seul `*-Setup.exe` signé et auto-extractible**. GUI native Slint (pas de
runtime WebView), handoff de config au 1er lancement, auto-update avec rollback, et un
mode maintenance (réparer / mettre à jour / désinstaller).

## Par où commencer

| Tu veux… | Lire |
|---|---|
| Configurer un installeur pour ton app | [../GUIDE.md](../GUIDE.md) — chaque champ d'`installer.toml` |
| Comprendre le fonctionnement interne | [ARCHITECTURE.md](ARCHITECTURE.md) |
| Livrer une app sur Windows + Linux + macOS | [PLATFORMS.md](PLATFORMS.md) (+ par OS : [Windows](platform-windows.md) · [Linux](platform-linux.md) · [macOS](platform-macos.md)) |
| Connaître le layout octet du `.bpkg` / SFX | [BPKG-FORMAT.md](BPKG-FORMAT.md) |
| Utiliser la CLI `bpkg` | [CLI.md](CLI.md) |
| Implémenter le handoff 1er lancement dans ton app | [HANDOFF.md](HANDOFF.md) |
| Livrer des mises à jour (manifest, deltas) | [UPDATES.md](UPDATES.md) |
| Héberger l'updater (GitHub / serveur perso) | [UPDATER-SETUP.md](UPDATER-SETUP.md) |
| Signer les paquets | [SIGNING.md](SIGNING.md) |

## Le modèle mental en 60 secondes

```
payload/            installer.toml          keys/private.key
   │                     │                        │
   ▼                     ▼                        ▼
bpkg pack ─────────► app.bpkg ──► bpkg sign ──► app.bpkg (signé)
                                                   │
            betterinstaller.exe (moteur) ──► bpkg build ──► App-Setup.exe
                                                                  │
                                                   double-clic ──┘
                                                        │
                          ┌─────────────────────────────┴───────────────┐
                          ▼                                              ▼
                   install fraîche                              déjà installé
              (Bienvenue→Conditions→Setup→Install→Terminé)  (maintenance : réparer/màj/désinstaller)
                          │
                          ▼
            écrit installer-handoff.json  ──►  l'app le lit une fois au 1er lancement
```

## Structure du dépôt

```
BetterInstaller/
├── crates/
│   ├── bpkg-core/     # bibliothèque : format, signature, handoff, update, ops par OS
│   ├── bpkg-cli/      # l'outil en ligne de commande `bpkg`
│   └── installer/     # le moteur GUI Slint (betterinstaller.exe)
├── examples/bmm/      # une config réelle complète (Acme Editor)
├── GUIDE.md           # référence de configuration installer.toml
└── docs/              # ce dossier
```

## Décisions de conception (verrouillées)

- **UI = Slint** (GPU natif, binaire statique unique) — pas WebView2/wry.
- **Manifeste `asInvoker`** (pas d'UAC) → installe **par-utilisateur** par défaut
  (`%LOCALAPPDATA%\Programs\<nom>`) ; Program Files nécessite un build élevé.
- **Handoff agnostique** : l'installeur n'écrit jamais le format de config privé d'une
  app ; il écrit un `installer-handoff.json` standard que l'app consomme une fois.
- Signature de paquet **Ed25519** ; compression payload **zstd** ; deltas **bsdiff**.
