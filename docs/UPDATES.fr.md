# Mises à jour

[🇬🇧 English](https://github.com/FreeProject089/BetterInstaller/blob/master/docs/UPDATES.md) · 🇫🇷 Français

Le moteur d'update (`bpkg-core/src/update.rs`) télécharge un paquet plus récent et
l'applique sur le dossier d'install avec un **rollback quasi-atomique** : il snapshot le
dossier vers un voisin `<nom>.bak`, extrait le nouveau paquet par-dessus, et en cas
d'**erreur** quelconque efface et restaure depuis le snapshot ; en cas de succès il
supprime le snapshot.

Ce que le rollback promet, et ce qu'il ne promet pas :

- Le snapshot n'est supprimé que si **tous** les fichiers sont revenus. Si un fichier ne
  peut pas être restauré (typiquement un fichier encore ouvert — l'app en cours, un scan
  antivirus), la restauration continue avec les autres et l'erreur indique le dossier
  `.bak`, conservé intact.
- Un `.bak` déjà présent au début d'une mise à jour est ce que laisse une mise à jour
  interrompue (coupure de courant, processus tué, rollback inachevé) et peut être la seule
  copie saine. La mise à jour **refuse alors de démarrer** et l'indique ; restaure-le ou
  supprime-le, puis relance.
- Les liens symboliques et jonctions dans le dossier d'install ne sont ni suivis ni
  copiés dans le snapshot, et un rollback les laisse en place.

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
- Avant de toucher au dossier d'install, le paquet téléchargé doit (1) porter une
  signature Ed25519 valide pour la clé épinglée et (2) être, d'après son **propre**
  manifeste signé, l'app mise à jour (`[app].id`) exactement dans la `version` proposée —
  qui doit être plus récente que l'installée. Un miroir peut donc servir un fichier
  corrompu, une ancienne release signée par la même clé, ou une autre app du même éditeur :
  aucun n'est appliqué.
- Ce que la signature ne peut pas empêcher : `update.json` lui-même n'est **pas** signé.
  Qui contrôle l'hôte qui le sert (ou un miroir de `manifest_urls`) peut retenir les mises
  à jour, ou proposer une vraie release plus récente que l'installée mais plus ancienne
  que la dernière. HTTPS le protège en transit ; rien ne le protège sur l'hôte.
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

Le nouveau paquet doit être signé par la **même clé** que l'installé. Avec un
`public_key` défini, la signature est toujours vérifiée avant d'appliquer ; un
`public_key` illisible, ou `require_signature = true` sans `public_key`, empêche
`installer.toml` de se charger au lieu de retomber sur une mise à jour non vérifiée.

## CLI (manuel / scripté)

```sh
bpkg fetch-update --url https://…/update.json --dir <install_dir> --current 1.1.0
bpkg update App-1.2.0.bpkg --dir <install_dir>     # applique un pkg local avec rollback
```
