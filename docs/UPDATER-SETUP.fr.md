# Configurer l'updater

[🇬🇧 English](https://github.com/FreeProject089/BetterInstaller/blob/master/docs/UPDATER-SETUP.md) · 🇫🇷 Français

L'installeur/updater récupère un petit **manifest de mise à jour** JSON depuis une URL que
tu contrôles, compare son `version` à l'installé, et (si plus récent) télécharge + applique
le nouveau `.bpkg` signé avec rollback. Deux setups d'hébergement courants ci-dessous.

```toml
# installer.toml
[update]
manifest_url  = "https://…/update.json"   # source principale (URL stable)
manifest_urls = []                        # sources/mirrors supplémentaires OPTIONNELS (voir plus bas)
auto_check    = true
allow_delta   = true
```

> **Sources multiples (optionnel).** Mets dans `manifest_urls` des URLs de manifest
> supplémentaires (un mirror, ton propre serveur…). L'updater interroge **toutes** les
> sources et utilise la **plus récente** trouvée ; les sources injoignables sont ignorées.
> Laisse `[]` pour une seule source — le multi est opt-in et totalement rétro-compatible.

Le manifest :
```json
{
  "version": "1.2.0",
  "url": "https://…/App-1.2.0.bpkg",
  "notes": "Nouveautés de la 1.2.0",
  "deltas": [ { "from": "1.1.0", "url": "https://…/1.1.0-to-1.2.0.patch" } ]
}
```

> Le nouveau `.bpkg` **doit être signé par la même clé** que le build installé
> (`require_signature` est imposé avant d'appliquer). Voir [SIGNING.md](SIGNING.md).

---

## Option A — GitHub Releases (gratuit, recommandé)

Utilise une **URL de téléchargement « latest » stable** pour que `manifest_url` ne change
jamais :

```
https://github.com/<owner>/<repo>/releases/latest/download/update.json
```

GitHub redirige toujours `…/releases/latest/download/<asset>` vers l'asset de la release
*non-préversion* la plus récente. Donc :

1. Build + sign du nouveau paquet :
   ```sh
   bpkg pack --root payload --config installer.toml --out App-1.2.0.bpkg
   bpkg sign --key keys/private.key App-1.2.0.bpkg
   ```
2. (Optionnel) un delta depuis la release précédente :
   ```sh
   bpkg delta App-1.1.0.bpkg App-1.2.0.bpkg 1.1.0-to-1.2.0.patch
   ```
3. Écris `update.json` pointant vers les URLs **d'assets de release** de *ce* tag :
   ```json
   {
     "version": "1.2.0",
     "url": "https://github.com/<owner>/<repo>/releases/download/v1.2.0/App-1.2.0.bpkg",
     "deltas": [
       { "from": "1.1.0",
         "url": "https://github.com/<owner>/<repo>/releases/download/v1.2.0/1.1.0-to-1.2.0.patch" }
     ]
   }
   ```
4. Crée la GitHub Release `v1.2.0` et uploade `update.json`, `App-1.2.0.bpkg`, et le patch
   comme **assets**.

`manifest_url` reste `…/releases/latest/download/update.json` pour toujours — chaque
release publie juste un nouveau `update.json`.

> Automatise-le : un job CI (ou ton script de build) peut générer `update.json` depuis la
> version du paquet et les URLs d'assets, puis `gh release create … update.json App-*.bpkg`.

### Exemple bundlé (`examples/`)

Le script de build de l'exemple bundlé **émet `update.json` automatiquement** (il lit
`[app].version` et dérive l'URL du `.bpkg` depuis `[update].manifest_url`). Une release =
trois uploads :

```
gh release create v1.0.0 \
  <App>-Setup.exe \
  app.bpkg \
  update.json
```

Son bloc `[update]` est déjà posé (`manifest_url = …/releases/latest/download/update.json`,
`auto_check = true`, `allow_delta = true`), donc une copie installée affiche **Mettre à
jour** en mode maintenance dès qu'une release plus récente est publiée. Bump
`[app].version`, rebuild, upload — terminé.

---

## Option B — Ton propre serveur / VPS / stockage objet

Héberge les fichiers n'importe où qui sert du HTTP(S) simple (nginx, S3/R2/B2, un host
statique) :

```
https://downloads.example.com/myapp/update.json
https://downloads.example.com/myapp/App-1.2.0.bpkg
https://downloads.example.com/myapp/1.1.0-to-1.2.0.patch
```

1. `manifest_url = "https://downloads.example.com/myapp/update.json"`.
2. À chaque release, uploade le `.bpkg` signé (+ patch optionnel) et écrase `update.json`
   avec le nouveau `version` + les URLs.
3. Sers avec les bons content-types et **CORS non requis** (l'updater récupère côté
   serveur via le client HTTP Rust, pas un navigateur).

nginx minimal :
```nginx
location /myapp/ {
    root /var/www;
    autoindex off;
    add_header Cache-Control "no-cache" always;   # pour que update.json soit re-récupéré
}
```

> Garde `update.json` non-caché (ou TTL court) pour que les clients voient vite les
> nouvelles releases ; les `.bpkg`/patches sont immuables et peuvent être cachés
> agressivement.

---

## Tester une mise à jour en local

```sh
# sers un dossier avec update.json + le .bpkg en localhost
python -m http.server 8000        # dans le dossier
bpkg fetch-update --url http://localhost:8000/update.json --dir <install_dir> --current 1.1.0
```

Ou mets `manifest_url` sur l'URL localhost, installe un build plus ancien, puis rouvre
l'installeur (mode maintenance) — le bouton **Mettre à jour** apparaît quand le manifest
est plus récent.

---

## Vérifier / appliquer les updates depuis ton app (CLI headless)

L'installeur **est** l'updater. Après l'install, il laisse une copie complète de lui-même
à `<install_dir>/uninstall.exe` (il gère install / réparer / **mettre à jour** /
désinstaller). Ton app peut le piloter avec deux flags — pas de GUI, pas de paquet
embarqué nécessaire pour la vérif :

### `--check-update` → JSON + code de sortie

```sh
"<install_dir>/uninstall.exe" --check-update
```

Imprime un rapport JSON sur **stdout** et pose le **code de sortie** :

| Sortie | Signification |
|---|---|
| `10` | Une mise à jour est disponible |
| `0`  | Déjà à jour |
| `2`  | Erreur (pas de `manifest_url`, échec réseau/HTTP, manifest invalide) |

```jsonc
// exit 10
{
  "app": "Ton App",
  "current_version": "1.0.0",      // l'installé (lu depuis l'OS), sinon la version embarquée
  "update_available": true,
  "latest_version": "2.0.0",
  "notes": "Ajoute le mode sombre et un scan plus rapide.",   // depuis le manifest, si fourni
  "url": "https://…/app.bpkg",
  "has_delta": false
}
// exit 0  → { "app": …, "current_version": "2.0.0", "update_available": false }
// exit 2  → { …, "update_available": false, "error": "HTTP 404 …" }
```

> Il compare la version **installée** (lue depuis l'OS — l'entrée ARP sous Windows) au
> `version` du manifest. Il ne rapporte `update_available: true` que si le manifest est
> strictement plus récent.

### `--update` → l'appliquer

```sh
"<install_dir>/uninstall.exe" --update
```

Ouvre la fenêtre de maintenance et, dès que le manifest confirme une version plus récente,
**démarre la mise à jour automatiquement** (download → vérif signature → apply avec
rollback, delta si proposé). Sans `--update`, le lancer normalement affiche le bouton
**Mettre à jour** que l'utilisateur clique.

### Le brancher dans ton app (exemple)

```rust
// Dans ton app : "Check for updates" → spawn le binaire de maintenance bundlé, lis le JSON.
let exe = std::env::current_exe()?.parent().unwrap().join("uninstall.exe");
let out = std::process::Command::new(&exe).arg("--check-update").output()?;
let report: serde_json::Value = serde_json::from_slice(&out.stdout)?;
if report["update_available"] == true {
    // affiche : "Mise à jour dispo : {current_version} → {latest_version}\n{notes}"
    // sur confirmation de l'utilisateur :
    std::process::Command::new(&exe).arg("--update").spawn()?;
    // (optionnellement, quitte ton app pour que l'updater puisse remplacer ses fichiers)
}
```

Comme l'installeur est un binaire en sous-système GUI, quand ton app le spawn avec un pipe
stdout capturé, la sortie est délivrée normalement (le pipe est hérité) ; lancé depuis un
terminal, il s'attache à la console parente.

> C'est le chemin prévu pour le bouton *Check for updates* de ton app : spawn
> `<install>/uninstall.exe --check-update`, parse le JSON pour afficher *actuel → dernier*
> avec les notes de release, puis lance `--update` sur confirmation.
