# CI/CD et scans de sécurité

Chaque workflow GitHub Actions de ce dépôt : ce qui le déclenche, ce qu'il vérifie, ce qui le fait
échouer, ce qu'il laisse derrière lui, et comment lancer la même chose sur ta machine. La seconde
moitié détaille les scans de sécurité : les lancer à la main, changer les seuils, exclure un chemin,
écarter un faux positif.

## Les trois workflows

| Workflow | Fichier | Se lance sur | Bloque sur |
|---|---|---|---|
| CI | `.github/workflows/ci.yml` | push sur `main` / `master`, chaque pull request | le formatage, un avertissement clippy, un test ou un build en échec, une alerte cargo non expliquée |
| Docs | `.github/workflows/docs.yml` | push sur `master` ou pull request qui touche `docs/`, `mkdocs.yml`, `requirements.txt` ou le workflow ; à la main | un lien cassé, une langue manquante, des ancres accentuées cassées, un PDF tronqué |
| Security | `.github/workflows/security.yml` | push sur `main` / `master`, chaque pull request, lundi 04:23 UTC, à la main | un secret dans l'historique, ou un résultat Semgrep / Trivy au seuil ou au-dessus |

Chaque action est épinglée par SHA de commit complet, avec son tag en commentaire. Chaque image
Docker des scans de sécurité est épinglée par digest. Chaque workflow part de `permissions: {}` ou de
`contents: read`, et un job n'obtient davantage que s'il écrit quelque chose. Ce dépôt est public :
la pull request d'un fork lance les mêmes workflows avec un token en lecture seule.

### CI (`ci.yml`)

| Job | Ce qu'il vérifie |
|---|---|
| build & test (Windows, Ubuntu) | `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --workspace --release` |
| cargo audit | `node .github/scripts/dep-audit.mjs cargo .` : toute vulnérabilité RustSec échoue, sauf si `.github/audit-ignore.json` l'explique (alerte, dossier, raison) |

- **Secrets / variables :** aucun. **Artefacts :** aucun.
- **En local :** `docker compose run --rm ci` lance tout le gate Linux dans l'image de dev (voir le
  `Dockerfile`) ; `cargo install cargo-audit --locked`, puis `node .github/scripts/dep-audit.mjs cargo .`.

### Docs (`docs.yml`)

Construit le site bilingue avec `mkdocs build --strict`, vérifie que les ancres françaises accentuées
survivent et que les deux langues sont construites, puis construit le PDF et le relit avec
`tools/check_pdf.py`. Sur `master`, le job `deploy` publie sur GitHub Pages.

- **Variables :** `PAGES_ENABLED=true` active le job de déploiement (après avoir activé Pages avec
  GitHub Actions comme source). Sans elle, le site est construit et vérifié, pas publié.
- **Secrets :** aucun.
- **Artefacts :** `betterinstaller-pdf`.
- **En local :** `pip install -r requirements.txt`, puis `mkdocs build --strict` et
  `mkdocs build -f mkdocs.pdf.yml`. Le PDF a besoin des bibliothèques pango et harfbuzz listées dans
  le workflow.

### Security (`security.yml`)

| Job | Outil | Ce qu'il lit | Échoue quand |
|---|---|---|---|
| Gate self-test | `node --test` | `.github/scripts/security-gate.test.mjs` | le gate lui-même est faux ; chaque scan l'attend |
| Secrets | Gitleaks 8.30.1 | tout l'historique git, toutes les branches | un résultat non revu dans `.gitleaksignore` / `.gitleaks.toml`, ou zéro commit lu |
| SAST | Semgrep 1.178.0 | tout le checkout : `crates/`, `tools/`, les workflows | un résultat au seuil Semgrep ou au-dessus |
| Dépendances + config conteneur | Trivy 0.74.0 | `requirements.txt` (la chaîne docs et PDF) et le `Dockerfile` | un résultat au seuil Trivy ou au-dessus |
| Envoi SARIF vers code scanning | `codeql-action/upload-sarif` | les trois fichiers SARIF | un envoi qui échoue sur un dépôt qui accepte le code scanning |
| Commentaire de PR | `gh api` | les trois verdicts | n'échoue jamais une pull request à lui seul |

Chaque scanner écrit son propre rapport et sort en 0. **Un seul script décide :**
`.github/scripts/security-gate.mjs`, le même fichier dans BetterInstaller, BMM, BMM Docs et BCW. Il
place chaque résultat sur une seule échelle (`critical > high > medium > low > info`), affiche un
tableau et ne fait échouer le job qu'au seuil ou au-dessus.

- **Secrets :** aucun à créer. Les jobs d'envoi et de commentaire utilisent le `GITHUB_TOKEN`
  automatique.
- **Variables :** voir [Seuils](#seuils).
- **Artefacts** (gardés 30 jours) : `security-gitleaks`, `security-semgrep` et `security-trivy`.
  Chacun contient le rapport JSON natif, le SARIF, un tableau `*-summary.md` et un verdict
  `*.result.json`.

**Pourquoi certains scans ne sont pas là :**

- **Pas de DAST (OWASP ZAP, Nuclei).** BetterInstaller est un installeur natif et un CLI. Il n'y a
  aucune application web à scanner, en staging ou ailleurs ; la doc est faite de pages statiques sur
  GitHub Pages. Aucun job ne vise un hôte du réseau.
- **Pas de scan Trivy de `Cargo.lock`.** Le job `cargo audit` de `ci.yml` le contrôle déjà, en
  bloquant, avec les raisons dans `.github/audit-ignore.json`. Un second scanner obligerait à écrire
  chaque alerte acceptée dans deux fichiers d'exclusion qui finiraient par diverger.
- **Pas de scan d'image.** Aucune image n'est construite ni publiée. Le `Dockerfile` est une image de
  développement locale, construite `FROM rust:latest`, un tag mouvant. Sa configuration est scannée ;
  scanner une image construite donnerait chaque semaine une réponse différente sur un outillage que
  personne ne livre.

## Lire les résultats

- **Log et résumé du job.** Le gate affiche un tableau par outil : sévérité, `BLOCK` ou `pass`, id de
  règle et emplacement. Il écrit aussi ce tableau dans le résumé du job, et chaque résultat bloquant
  est une annotation `::error`. Gitleaks tourne avec `--redact` : le log montre la règle, le fichier,
  la ligne, le commit et l'empreinte, jamais la valeur.
- **Commentaire de pull request.** Le job `PR comment` poste un commentaire, puis modifie ce même
  commentaire à chaque run suivant. Il le retrouve grâce à la première ligne cachée
  `<!-- betterinstaller-security-gate -->`. Le commentaire montre une ligne par outil (seuil, nombre
  par sévérité, nombre bloquant, pass ou **FAIL**) et un verdict global `PASSED`, `FAILED` ou
  `INCOMPLETE`. `INCOMPLETE` veut dire qu'un scan n'a pas produit de verdict ; ce n'est jamais
  présenté comme un succès. Il se termine par un lien vers le run et ses artefacts. La pull request
  d'un fork ou de Dependabot reçoit un token en lecture seule : le job affiche une notice et ne poste
  rien.
- **Code scanning (onglet Security).** Les SARIF de Gitleaks, Semgrep et Trivy sont envoyés avec une
  catégorie par outil (`gitleaks`, `semgrep`, `trivy`). GitHub suit alors chaque alerte dans le temps
  et annote les lignes modifiées d'une pull request. L'envoi a lieu même quand un gate a échoué. Le
  job demande d'abord à l'API si le code scanning est disponible. Ce dépôt est public, donc il l'est.
  Sur un dépôt privé sans GitHub Advanced Security, le job affiche une notice et le SARIF reste dans
  les artefacts.

Le code scanning est une vue. **C'est le gate qui fait échouer le build.** Une alerte écartée dans
l'onglet Security refait échouer le job au run suivant, tant que l'outil la signale.

## Seuils

Le seuil est la **plus basse sévérité qui fait échouer**. Définis-le comme variable de dépôt dans
**Settings > Secrets and variables > Actions > Variables** :

| Variable | Valeurs | Défaut |
|---|---|---|
| `SECURITY_GATE_SEVERITY` | `critical`, `high`, `medium`, `low` | `high` |
| `SECURITY_GATE_SEVERITY_SEMGREP` | les mêmes | la valeur globale |
| `SECURITY_GATE_SEVERITY_TRIVY` | les mêmes | la valeur globale |

- `critical` et `high` bloquent par défaut. `medium` ne bloque qu'à `medium` ou `low`. `low` ne bloque
  qu'à `low`. `info` ne bloque jamais.
- Gitleaks n'a pas de seuil : tout secret qui n'est pas une exclusion revue fait échouer.
- Une valeur que le gate ne connaît pas, une faute de frappe par exemple, **fait échouer le run**
  (code de sortie 2). Une faute de frappe ne doit jamais vouloir dire que rien ne bloque.
- Le `UNKNOWN` de Trivy compte comme `high`. Les `ERROR`, `WARNING` et `INFO` de Semgrep comptent comme
  `high`, `medium` et `low`.

## Lancer chaque scan à la main

Depuis la racine du dépôt, avec Docker et les mêmes images épinglées que la CI. Sous Windows,
utilise Git Bash avec `export MSYS_NO_PATHCONV=1`, ou un clone dans WSL, et lance depuis un clone
propre : Trivy parcourt tout le dossier, et un `target/` local suffit à atteindre son délai de
5 minutes.

```bash
GITLEAKS=zricethezav/gitleaks:v8.30.1@sha256:c00b6bd0aeb3071cbcb79009cb16a60dd9e0a7c60e2be9ab65d25e6bc8abbb7f
SEMGREP=semgrep/semgrep:1.178.0@sha256:32e459968daabe7ab86968184a29109b9564aa00392401156f9788452b42786b
TRIVY=aquasec/trivy:0.74.0@sha256:62b1e65e8869bc4b4c6aa4fa2b21595256c7c2f6018a9d9ad61caf87187c1969
mkdir -p reports

# Secrets : tout l'historique. Le log doit dire « N commits scanned » avec N supérieur à 0.
docker run --rm -v "$PWD:/repo" \
  -e GIT_CONFIG_COUNT=1 -e GIT_CONFIG_KEY_0=safe.directory -e GIT_CONFIG_VALUE_0='*' \
  "$GITLEAKS" git /repo --redact --verbose \
  --report-format json --report-path /repo/reports/gitleaks.json --exit-code 0
node .github/scripts/security-gate.mjs --tool gitleaks --report reports/gitleaks.json

# SAST, avec les règles au commit épinglé.
REF=$(grep -oE 'SEMGREP_RULES_REF: [0-9a-f]{40}' .github/workflows/security.yml | cut -d' ' -f2)
git init -q ../semgrep-rules && git -C ../semgrep-rules fetch -q --depth 1 https://github.com/semgrep/semgrep-rules "$REF" \
  && git -C ../semgrep-rules checkout -q FETCH_HEAD
CFG=$(grep -v '^#' .github/security/semgrep-rules.txt | grep . | sed 's#^#--config=/rules/#')
docker run --rm -v "$PWD:/src" -v "$PWD/../semgrep-rules:/rules:ro" -w /src \
  -e GIT_CONFIG_COUNT=1 -e GIT_CONFIG_KEY_0=safe.directory -e GIT_CONFIG_VALUE_0='*' \
  "$SEMGREP" semgrep scan --metrics=off $CFG --json-output=reports/semgrep.json .
node .github/scripts/security-gate.mjs --tool semgrep --report reports/semgrep.json

# Dépendances et Dockerfile.
docker run --rm -v "$PWD:/src" -w /src "$TRIVY" fs --scanners vuln,misconfig --show-suppressed \
  --skip-files Cargo.lock --format json --output reports/trivy.json --exit-code 0 .
node .github/scripts/security-gate.mjs --tool trivy --report reports/trivy.json

# Le gate lui-même.
node --test .github/scripts/security-gate.test.mjs
```

Pour rendre le commentaire de pull request en local, ajoute `--json reports/<outil>.result.json` à
chaque ligne de gate, puis lance `node .github/scripts/security-gate.mjs --markdown --results reports/gitleaks.result.json --results reports/semgrep.result.json --results reports/trivy.result.json`.

Pour essayer un autre seuil sur un artefact téléchargé :
`SECURITY_GATE_SEVERITY=medium node .github/scripts/security-gate.mjs --tool trivy --report trivy.json`.

## Ajouter ou exclure une cible

| Pour… | Modifier |
|---|---|
| Ajouter une règle Semgrep | une ligne dans `.github/security/semgrep-rules.txt` : le chemin d'un fichier de règle de `semgrep/semgrep-rules` au commit épinglé |
| Exclure des chemins de Semgrep | un `.semgrepignore` à la racine, un motif par ligne, avec un commentaire qui dit pourquoi |
| Confier un nouveau lockfile à Trivy | rien : `trivy fs` trouve tous les lockfiles sauf `Cargo.lock` |
| Exclure un dossier de Trivy | `--skip-dirs <dossier>` dans l'étape `trivy fs`, avec un commentaire qui dit pourquoi |
| Mettre à jour les règles Semgrep | changer `SEMGREP_RULES_REF`, régénérer la liste et lire le diff |

La liste ne garde que les fichiers de règles dont les règles ne sont pas marquées
`subcategory: audit`. Une règle d'audit signale chaque usage d'un sink pour qu'un humain le relise.
C'est une aide à la revue manuelle, pas une décision qu'un gate de CI peut prendre. Des règles prises
à un commit épinglé donnent le même résultat l'an prochain, alors qu'un ruleset du registre (`p/…`)
change sous tes pieds. La contrepartie : les nouvelles règles n'arrivent que quand quelqu'un met la
ref à jour.

## Écarter un faux positif proprement

**Préfère la config de l'outil** à un rejet dans l'onglet Security. La config est versionnée, relue
dans une pull request, porte sa raison, et vaut à la fois pour le gate, les artefacts et l'onglet
Security. Un rejet dans l'onglet Security ne fait que cacher l'alerte dans cette vue ; le gate échoue
encore au run suivant.

| Outil | Où | Format |
|---|---|---|
| Gitleaks, un résultat | `.gitleaksignore` | l'empreinte du log (`commit:fichier:règle:ligne`), sous un commentaire qui dit pourquoi |
| Gitleaks, une classe de faux positifs | `.gitleaks.toml` avec `[extend] useDefault = true` et `[[allowlists]]` | `targetRules` + une `regexes` étroite, avec un commentaire qui prouve qu'elle ne peut pas toucher un vrai secret |
| Semgrep | la ligne de source | `// nosemgrep: <id-de-règle>` (`#` en Python), avec la raison sur la ligne au-dessus |
| Trivy | `.trivyignore` | `CVE-XXXX-YYYY exp:2026-12-31` sous un commentaire avec la raison ; l'expiration refait passer l'exclusion en revue |
| cargo audit (`ci.yml`) | `.github/audit-ignore.json` | id, outil, dossier, raison, date |

- **Un vrai secret n'est jamais exclu.** Fais-le d'abord tourner (rotation). Seulement ensuite, s'il
  doit rester dans l'historique, ajoute son empreinte avec la date de rotation.
- Une exclusion nomme un résultat, ou une classe prouvée inoffensive. Jamais une règle entière ou un
  dossier entier « pour que la CI passe ».
- Une modification de l'un de ces fichiers est une modification de sécurité : relis-la comme telle.

Un rejet dans l'onglet Security (`False positive` / `Won't fix`) reste l'endroit pour consigner une
décision que la config de l'outil ne sait pas exprimer. Écris la justification dans le commentaire,
pas seulement le motif.

## Tester sans rien toucher en production

- Chaque scan ne lit que le checkout. Aucun job ne se connecte à un serveur de mise à jour, à la
  machine d'un utilisateur ou à un hôte de production. Le seul trafic réseau récupère les images
  épinglées, la base d'alertes de Trivy et les règles Semgrep épinglées.
- Lance les workflows sur une branche, une pull request, ou avec **Run workflow**
  (`workflow_dispatch`).
- Lance les commandes ci-dessus en local ; les rapports arrivent dans `reports/`. Ne commite pas ce
  dossier.
- `node --test .github/scripts/security-gate.test.mjs` prouve que le gate distingue toujours `high` de
  `low`, et qu'un rapport manquant ou un scan de rien est un échec.
