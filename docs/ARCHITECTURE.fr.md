# Architecture

[🇬🇧 English](ARCHITECTURE.md) · 🇫🇷 Français

BetterInstaller est un petit workspace Rust. Le moteur est écrit une fois contre un trait
d'abstraction de plateforme ; tout ce qui est spécifique au projet vit dans
`installer.toml`.

## Crates

### `bpkg-core` (bibliothèque)
Le cœur réutilisable. Modules :

| Module | Responsabilité |
|---|---|
| `package` (`format`, `reader`, `writer`) | Le format `.bpkg` : pack, open, verify, extract, `install_with_progress`. |
| `manifest` | Types `Manifest` / `FileEntry` / `AppMeta` / `Component`. |
| `config` | Parse `installer.toml` (`InstallerConfig` + toutes les sections). |
| `sign` | Keygen / sign / verify Ed25519 (ed25519-dalek). |
| `handoff` | Construit + écrit `installer-handoff.json` (le contrat 1er lancement). |
| `update` | Vérif manifest distant, download, apply delta bsdiff, rollback atomique. |
| `delta` | Diff/patch binaire (qbsdiff). |
| `embed` | Lecture/écriture du trailer auto-extractible (config + bpkg ajoutés à l'exe). |
| `prereq` | Détecter/auto-installer les prérequis (vérifs registre/fichier/commande). |
| `platform` | Trait `PlatformOps` + backends Windows/Linux/macOS. |
| `net`, `i18n`, `error` | HTTP (reqwest rustls), traductions, type d'erreur. |

### `bpkg-cli` (`bpkg`)
Une CLI fine au-dessus de `bpkg-core` : `pack`, `sign`, `verify`, `keygen`, `build`,
`info`, `extract`, `install`, `update`, `fetch-update`, `delta`, `apply-delta`.
Voir [CLI.md](CLI.md).

### `installer` (`betterinstaller.exe`)
Le moteur GUI Slint. Fenêtre sans cadre (barre de titre custom via le backend winit pour
déplacer / minimiser / maximiser). Résout sa config + son paquet depuis le trailer SFX
embarqué (ou les args CLI en dev), affiche le flux, écrit le handoff, lance l'install sur
un thread worker, et fait l'intégration OS.

## `PlatformOps` (l'abstraction)

Le moteur ne branche jamais sur l'OS. Chaque backend implémente :

```
default_install_dir, app_data_dir,
create_shortcuts, register_protocol, register_uninstaller, add_to_path,
remove_shortcuts, unregister_protocol, unregister_uninstaller,
installed_dir, installed_version
```

| Op | Windows | Linux | macOS |
|---|---|---|---|
| Raccourcis | `.lnk` (mslnk) Menu Démarrer / Bureau | fichiers `.desktop` | symlink `/Applications` |
| Protocole | HKCU `Software\Classes\<scheme>` | `xdg-mime` | `Info.plist` |
| Désinstalleur | entrée ARP HKCU | — | — |
| Détection install | ARP `InstallLocation` / `DisplayVersion` | — | — |

Par-utilisateur partout (correspond au manifeste `asInvoker` — pas d'admin requis).

## Flux d'install (GUI)

1. **resolve_sources** — lit le SFX embarqué (config + `.bpkg` stagé), sinon les args CLI.
2. **detect** — `installed_dir(app.id)` → mode maintenance si déjà installé ;
   `detect_signature` → le badge de confiance de la page Bienvenue.
3. **Bienvenue** (dossier + composants) → **Conditions** (docs de licence, acceptation par
   doc) → **Setup** (options) → **Installation** → **Terminé** (lancement opt-in).
4. **À l'Install** (thread worker) : écrit `installer-handoff.json` → barrière prérequis →
   vérif d'écriture → vérif signature → `install_with_progress` →
   `do_system_integration` (raccourcis, protocole, entrée ARP, `uninstall-info.json`).

## Flux de maintenance

Activé quand `installed_dir` est trouvé (ou `--uninstall`). Lit l'`InstallLocation` ARP ;
propose **Réparer** (re-vérif+restaure), **Mettre à jour** (seulement si une version plus
récente est trouvée — manifest distant ou paquet embarqué plus récent ; download + apply
avec rollback), **Désinstaller** (tue l'app en cours → annule l'intégration → retire le
dossier, y compris le désinstalleur via une auto-suppression détachée). Tout avec
confirmation et Annuler.

## Handoff 1er lancement

La fonctionnalité phare : l'installeur pré-configure l'app pour qu'elle n'affiche
**aucune** modale au 1er lancement. Contrat agnostique — voir [HANDOFF.md](HANDOFF.md).
