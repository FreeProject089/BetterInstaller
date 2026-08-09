# Mises à jour

[🇬🇧 English](https://github.com/FreeProject089/BetterInstaller/blob/master/docs/UPDATES.md) · 🇫🇷 Français

Le moteur d'update (`bpkg-core/src/update.rs`) télécharge un paquet plus récent et
l'applique sur le dossier d'install avec un **rollback quasi-atomique** : il snapshot le
dossier vers un voisin `<nom>.bak`, extrait le nouveau paquet par-dessus, et en cas
d'**erreur** quelconque efface et restaure depuis le snapshot ; en cas de succès il
supprime le snapshot.

## Configuration (`installer.toml`)

```toml
[update]
manifest_url = "https://…/update.json"   # une URL JSON stable que tu contrôles
auto_check   = true                        # vérifie à l'ouverture de la fenêtre maintenance
allow_delta  = true                        # préfère un petit patch binaire si proposé
```

En mode maintenance, le GUI vérifie le manifest en arrière-plan ; s'il annonce une
version plus récente, le bouton **Mettre à jour** apparaît (affichant `v<ancien> →
v<nouveau>`) et l'applique. Sans `[update]`, Update apparaît quand même si le setup
*embarqué* est plus récent que l'installé (il ré-extrait le paquet embarqué).

## Manifest de mise à jour (le JSON que tu héberges)

```json
{
  "version": "1.2.0",
  "url": "https://…/App-1.2.0.bpkg",
  "notes": "texte de changelog optionnel",
  "deltas": [
    { "from": "1.1.0", "url": "https://…/1.1.0-to-1.2.0.patch" }
  ]
}
```

- `version` est comparée numériquement composante par composante (`is_newer`).
- Si une entrée `deltas` correspond à la version installée **et** que le `.bpkg` actuel
  est disponible, un petit patch bsdiff est téléchargé et le nouveau paquet est
  reconstruit localement ; sinon le `url` complet est téléchargé.

## Produire une mise à jour

```sh
# build + sign de la nouvelle version
bpkg pack --root payload --config installer.toml --out App-1.2.0.bpkg
bpkg sign --key keys/private.key App-1.2.0.bpkg

# (optionnel) un delta depuis la release précédente
bpkg delta App-1.1.0.bpkg App-1.2.0.bpkg 1.1.0-to-1.2.0.patch

# héberge App-1.2.0.bpkg, le patch, et update.json à des URLs stables
```

Le nouveau paquet doit être signé par la **même clé** que l'installé
(`require_signature` est imposé avant d'appliquer).

## CLI (manuel / scripté)

```sh
bpkg fetch-update --url https://…/update.json --dir <install_dir> --current 1.1.0
bpkg update App-1.2.0.bpkg --dir <install_dir>     # applique un pkg local avec rollback
```
