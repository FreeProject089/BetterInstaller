# macOS

[🇬🇧 English](https://github.com/FreeProject089/BetterInstaller/blob/master/docs/platform-macos.md) · 🇫🇷 Français

Par-utilisateur, aucun root. Les apps macOS sont des bundles `.app`, et le protocole URL
est déclaré dans l'`Info.plist` du bundle plutôt qu'enregistré au runtime.

## Emplacements

| Quoi | Chemin |
|---|---|
| Dossier d'install par défaut | `/Applications/<app.name>.app` |
| Données app (handoff) | `~/Library/Application Support/<app.id>` |
| « Raccourci » | symlink dans `~/Applications` → le bundle de l'app |
| Protocole URL | déclaré dans `<app>.app/Contents/Info.plist` (`CFBundleURLTypes`) |

## Ce que fait le moteur

- **Raccourcis** — un symlink dans `~/Applications` est le lanceur par-utilisateur le
  plus simple.
- **Protocole** — *pas* un appel runtime. Ajoute à l'`Info.plist` de ton app :
  ```xml
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key>     <string>com.acme.editor</string>
      <key>CFBundleURLSchemes</key>  <array><string>acme</string></array>
    </dict>
  </array>
  ```
  Launch Services le prend en compte quand le `.app` est vu pour la première fois.
- **Désinstallation** — retire le `.app` (et le symlink `~/Applications`) ;
  `uninstall-info.json` note ce qu'il faut annuler.

## Conseils payload

- Le payload **est** le bundle `.app` (ou son contenu). Garde la structure standard :
  `<App>.app/Contents/{MacOS,Resources,Info.plist}`.
- Pointe `[install].main_exe` vers `Contents/MacOS/<binaire>`.
- `[branding].logo` peut être un PNG lu depuis le paquet pour le sidebar de l'installeur ;
  l'icône propre de l'app reste `Contents/Resources/<icon>.icns`.

## Build

```sh
cargo build --release -p bpkg-cli -p installer
bpkg pack  --root payload-macos --config installer.toml --out app-macos.bpkg
bpkg sign  --key keys/private.key app-macos.bpkg
bpkg build --installer ./target/release/betterinstaller \
           --config installer.toml --package app-macos.bpkg --out App-Setup
```

## Notarisation / Gatekeeper

La signature de paquet Ed25519 ≠ la notarisation Apple. Pour une distribution hors App
Store, **codesign + notarise** le `.app` (et idéalement livre-le dans un `.dmg` signé)
pour que Gatekeeper ne le bloque pas. C'est séparé de la signature de paquet de
BetterInstaller.

## Pièges

- Évite de terminer `[app].id` par `.app` si possible — ça entre en collision avec la
  convention d'extension de bundle (certains toolkits émettent un warning) ; renomme-le si tu peux.
- L'enregistrement runtime du protocole n'est **pas** effectué par le moteur sur macOS —
  il doit être dans l'`Info.plist` du bundle (déclaratif).
