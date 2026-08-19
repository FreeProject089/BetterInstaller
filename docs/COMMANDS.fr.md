# Toutes les commandes, et quand tu les veux

Construire, vérifier et livrer BetterInstaller lui-même. Pour ce que fait `bpkg` une fois
construit, voir [Ligne de commande](CLI.fr.md) — cette page parle d'y arriver.

Les chemins sont relatifs à la racine de BetterInstaller.

## Construire

```bash
cargo build --release -p bpkg-cli -p installer
```

Deux binaires sortent dans `target/release/` :

| | |
|---|---|
| `bpkg` | l'outil d'empaquetage — pack, sign, verify, build |
| `betterinstaller` | le stub auto-extractible qu'on estampille avec un paquet |

Plus rapide pendant le travail :

```bash
cargo check --workspace        # est-ce que ça compile
cargo build -p bpkg-cli        # build debug d'une seule crate
```

## La barrière de qualité

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Ou tout d'un coup dans un conteneur, ce que fait la CI :

```bash
docker compose run --rm ci
docker compose run --rm shell    # interactif, avec la chaîne d'outils et les dépendances
```

Des volumes nommés mettent en cache le registre Cargo et le répertoire cible Linux : les
relances sont rapides et les artefacts Linux ne touchent jamais ton `./target` local.

## Fabriquer un installeur

Le pipeline complet, à la main :

```bash
./target/release/bpkg keygen --out keys
./target/release/bpkg pack  --root payload --config installer.toml --out app.bpkg
./target/release/bpkg sign  --key keys/private.key app.bpkg
./target/release/bpkg build --installer ./target/release/betterinstaller.exe \
                            --config installer.toml --package app.bpkg --out App-Setup.exe
```

!!! danger "`keys/private.key` est secrète"
    Elle signe les paquets. Quiconque la détient peut produire un installeur que les copies
    de tes utilisateurs accepteront comme authentique. Seule `public.key` va dans
    `installer.toml`.

### Pour BMM, scripté

```powershell
./examples/bmm/build-installer.ps1              # payload → .bpkg → signature → estampillage du SFX
./examples/bmm/build-installer.ps1 -Logo mon.png # avec un logo de barre latérale personnalisé
./examples/bmm/release.ps1                      # la release entière
```

`release.ps1` incrémente les versions, construit BMM, empaquette/signe/estampille, fabrique
un delta par rapport à la release précédente, écrit un `update.json` multi-sources, et peut
publier la release GitHub avec tous ses assets.

Depuis le dépôt BMM, la même chose s'appelle `npm run build:installer`, et `npm run release`
fait le build avant.

## Documentation

```bash
pip install -r requirements.txt
mkdocs serve                        # http://127.0.0.1:8000
mkdocs build --strict               # ce que vérifie la CI : un lien interne cassé fait échouer
mkdocs build -f mkdocs.pdf.yml      # le PDF — demande GTK, plus simple en CI
python tools/check_pdf.py           # contrôle de cohérence du PDF généré
```

`mkdocs.yml` n'inclut délibérément pas le plugin PDF : mkdocs importe un plugin même
désactivé, donc l'y lister faisait échouer `mkdocs serve` sur une machine sans les
bibliothèques GTK. La configuration PDF en hérite à la place.

## L'espace de travail

| Crate | Ce que c'est |
|---|---|
| `bpkg-core` | le format — lire, écrire, vérifier un `.bpkg` |
| `bpkg-cli` | la commande `bpkg` |
| `installer` | le stub SFX, la fenêtre que voit réellement l'utilisateur |

En cibler une avec `-p` :

```bash
cargo test -p bpkg-core
cargo clippy -p installer -- -D warnings
```
