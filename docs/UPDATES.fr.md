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

## Manifest de mise à jour (le JSON que tu héberges, signé)

`update.json` décide quelle version est proposée et d'où elle est téléchargée : il est
donc **signé avec la clé de l'éditeur** — la même clé Ed25519 que celle qui signe les
paquets. Produis-le avec `bpkg update-manifest`, jamais à la main :

```json
{
  "version": "1.2.0",
  "url": "https://…/App-1.2.0.bpkg",
  "urls": ["https://mirror.example/App-1.2.0.bpkg"],
  "notes": "texte de changelog optionnel",
  "deltas": [
    { "from": "1.1.0", "url": "https://…/1.1.0-to-1.2.0.patch" }
  ],
  "signed": "{\"app_id\":\"com.example.app\",\"version\":\"1.2.0\",\"url\":\"https://…/App-1.2.0.bpkg\",…,\"sha256\":\"…\",\"config_sha256\":\"…\",\"issued\":\"2026-09-24T12:00:00Z\",\"expires\":\"2026-10-01T12:00:00Z\"}",
  "signature": "<128 caractères hexadécimaux>"
}
```

- **`signed`** est un document JSON porté sous forme de chaîne ; la signature couvre ses
  octets exacts, précédés de la ligne de contexte `BetterInstaller update manifest v1` et
  d'un saut de ligne (une signature de manifest ne peut donc jamais passer pour une
  signature de paquet, ni l'inverse). Il contient `app_id`, `version`, `url`, `urls`,
  `notes`, `deltas`, le SHA-256 du `.bpkg` complet (`sha256`), le SHA-256 du
  `installer.toml` estampillé dans le setup de cette release (`config_sha256`), et
  `issued` / `expires` (RFC 3339, au plus **7 jours** d'écart).
- Les champs `version`, `url`, `urls`, `notes` et `deltas` **au premier niveau** sont une
  copie pour les installeurs antérieurs aux manifests signés, qui ignorent les champs
  qu'ils ne connaissent pas. Un installeur qui vérifie ne lit **que** la partie signée et
  ne retombe jamais sur la copie.

Avec un `public_key` dans `installer.toml`, l'installeur refuse :

- un manifest **non signé** (sauf sous la règle de migration ci-dessous) ;
- un manifest dont la signature ne se vérifie pas avec la clé épinglée, ou modifié après
  signature ;
- un manifest **expiré** (`expires` passé), ou signé pour plus de 7 jours ;
- un manifest pour une autre app (`app_id` ≠ `[app].id`) ;
- un manifest dont la version n'est **pas plus récente** que l'installée : il n'est jamais
  proposé, et son paquet est refusé si quelque chose l'applique quand même (protection
  contre le retour en arrière).

Une source qui échoue sur l'un de ces points compte comme une source en échec : avec
plusieurs `manifest_urls`, les autres sont quand même lues, et la plus récente valide
l'emporte. Sans `public_key`, il n'y a rien contre quoi vérifier : le manifest est lu tel
quel (la partie signée si elle est là, dont l'expiration s'applique toujours), comme un
paquet non signé.

Avant de toucher au dossier d'install, le téléchargement doit ensuite (1) correspondre au
`sha256` du manifest signé, (2) porter une signature Ed25519 valide pour la clé épinglée et
(3) être, d'après son **propre** manifeste de paquet signé, l'app mise à jour (`[app].id`)
exactement dans la `version` proposée — plus récente que l'installée. Un miroir peut donc
servir un fichier corrompu, une ancienne release, ou un autre paquet signé par la même
clé : aucun n'est appliqué. Seule la dernière erreur de téléchargement est rapportée.

- `version` est comparée par la règle de version unique du moteur (`bpkg_core::version`) :
  la première suite de nombres séparés par des points, composante par composante, une
  composante absente valant zéro (`1.2` = `1.2.0`, `v1.3.0` > `1.2.0`). Un suffixe ne fait
  pas partie de la version : `1.2.0-beta.2`, `1.2.0+5` et `1.2.0` sont la **même**
  release. Donne à une pré-version ses propres numéros (`1.2.90`) si elle doit être
  proposée comme mise à jour.
- Si une entrée `deltas` correspond à la version installée **et** que le `.bpkg` actuel
  est disponible, un petit patch bsdiff est téléchargé et le nouveau paquet est
  reconstruit localement ; le résultat doit toujours correspondre à `sha256`. Sinon le
  `url` complet est téléchargé.

Ce que le manifest signé ne peut toujours pas empêcher : qui contrôle l'hôte peut
**retenir** le manifest, mais pas longtemps — au bout de 7 jours au plus, la dernière copie
vue par les clients a expiré : la mise à jour distante n'est plus proposée et
`--check-update` rapporte une erreur (code 2) au lieu de « à jour ». Le prix est côté
éditeur : **re-signe `update.json` au moins une fois par semaine** entre deux releases
(`bpkg resign-manifest`), sinon aucune copie installée ne voit plus de mise à jour.

### Migration (une release du moteur)

Les installeurs construits avant les manifests signés ne lisent que la copie du premier
niveau ; ils ne peuvent rien vérifier, quoi que dise le fichier. Pour les installeurs qui
vérifient, la règle est :

> Un `update.json` **non signé** est accepté — signalé, et l'utilisateur en est averti sur
> la page de résultat (« les informations de mise à jour n'étaient pas signées… ») —
> seulement si les **deux** conditions tiennent : le moteur porte encore
> `MIGRATION_ACCEPTS_UNSIGNED = true` (cette release seulement ; il passe à `false` dans
> la suivante), **et** l'install mise à jour a été enregistrée par un moteur antérieur aux
> manifests signés (son `uninstall-info.json` existe et n'a pas de clé
> `signed_update_manifests`). La mise à jour qui suit écrit cette clé : chaque install n'en
> profite qu'une fois. Sans `uninstall-info.json` du tout, la règle stricte s'applique.

`bpkg fetch-update` n'a pas d'enregistrement d'install à lire et n'applique jamais la règle
de migration.

### Ce que la mise à jour fait à l'app en cours et à l'enregistrement de désinstallation

- **Fermeture de l'app.** Une fois que le téléchargement a passé toutes les vérifications
  ci-dessus, et avant le snapshot du dossier d'install, l'installeur ferme les processus
  de l'app elle-même (les `.exe` du premier niveau du paquet), comme le chemin de mise à
  jour local l'a toujours fait. Un processus n'est fermé que si son exécutable **est** ce
  fichier dans le dossier d'install (chemin complet de l'image, comparé après
  canonicalisation) : une seconde copie du même programme lancée depuis un autre dossier
  n'est pas touchée.
- **Enregistrement de désinstallation.** Les fichiers écrits par une mise à jour sont
  ajoutés à `uninstall-info.json`. Désinstaller depuis un dossier que l'install ne possède
  pas retire alors aussi les fichiers qu'une version ultérieure a ajoutés.

## Produire une mise à jour

```sh
# build + sign de la nouvelle version
bpkg pack --root payload --config installer.toml --out App-1.2.0.bpkg
bpkg sign --key keys/private.key App-1.2.0.bpkg

# (optionnel) un delta depuis la release précédente
bpkg delta --old App-1.1.0.bpkg --new App-1.2.0.bpkg --out 1.1.0-to-1.2.0.patch

# le manifest signé (id d'app, version et hash lus dans le paquet)
bpkg update-manifest --package App-1.2.0.bpkg --config installer.toml \
  --key keys/private.key --url https://…/App-1.2.0.bpkg \
  --mirror https://mirror.example/App-1.2.0.bpkg \
  --delta "1.1.0=https://…/1.1.0-to-1.2.0.patch" \
  --notes "Nouveautés" --out update.json

# héberge App-1.2.0.bpkg, le patch, et update.json à des URLs stables

# au moins chaque semaine jusqu'à la release suivante (un job CI planifié qui détient la clé) :
bpkg resign-manifest update.json --key keys/private.key
# … puis re-publie update.json partout où il est servi
```

`update-manifest` refuse un paquet que cette clé n'a pas signé, et une config dont
`[app].id` / `version` diffèrent du paquet. `resign-manifest` ne renouvelle qu'un manifest
que cette clé a déjà signé (son expiration peut être passée).

Le nouveau paquet doit être signé par la **même clé** que l'installé. Avec un
`public_key` défini, la signature est toujours vérifiée avant d'appliquer ; un
`public_key` illisible, ou `require_signature = true` sans `public_key`, empêche
`installer.toml` de se charger au lieu de retomber sur une mise à jour non vérifiée.

## CLI (manuel / scripté)

```sh
bpkg fetch-update --url https://…/update.json --dir <install_dir> --current 1.1.0 \
  --key keys/public.key                              # manifest ET paquet vérifiés
bpkg update App-1.2.0.bpkg --dir <install_dir>     # applique un pkg local avec rollback
bpkg verify-manifest update.json --key keys/public.key [--app-id <id>] [--setup App-Setup.exe]
```

`verify-manifest --setup` vérifie aussi qu'un setup porte exactement l'`installer.toml` et
le paquet que nomme le manifest de cette release. C'est la seule vérification de la config
qui ne dépend pas d'une clé fournie par la config elle-même (voir
[SIGNING.fr.md](SIGNING.fr.md)).
