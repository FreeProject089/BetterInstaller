# Contrat de handoff (1er lancement)

[🇬🇧 English](https://github.com/FreeProject089/BetterInstaller/blob/master/docs/HANDOFF.md) · 🇫🇷 Français

L'installeur pré-configure l'app au moment de l'install et dépose un
`installer-handoff.json` standard. L'app le lit **une fois** au 1er lancement, l'applique,
et le marque consommé — donc aucune modale de confidentialité/CGU/langue/tutoriel au
1er lancement. L'installeur ne touche jamais au format de config privé de l'app ; ce
contrat est agnostique.

> ## ⚠️ Le handoff n'est PAS automatique
> BetterInstaller ne fait qu'**écrire le fichier JSON**. Rien n'arrive à ton app à moins
> que **tu ajoutes quelques lignes dans ton app** qui lisent ce fichier au 1er lancement
> et l'appliquent. Pas de lecteur = le fichier reste là, ignoré. Toute l'intégration :
> **1)** déclarer des `[[setup_option]]` dans `installer.toml` (ce que l'étape Setup
> demande), **2)** écrire un petit lecteur dans ton app (quoi faire des réponses).
> C'est tout — voir [§ Faire le tien](#faire-le-tien-3-étapes) plus bas.

## Ce que l'installeur écrit

Emplacement : `[handoff].location` → `app_data` (`%APPDATA%/<app.id>/`) ou `install_dir`.
Nom de fichier : `[handoff].file` (défaut `installer-handoff.json`).

```json
{
  "schema": 1,
  "source": "betterinstaller",
  "installer_version": "0.1.0",
  "app_version": "1.0.0",
  "installed_at": "2026-06-24T10:57:00Z",
  "components": ["core", "mcp-server"],
  "install_dir": "C:\\Users\\me\\AppData\\Local\\Programs\\Acme Editor",
  "settings": {
    "language": "fr",
    "tos_accepted": true,
    "privacy_accepted": true,
    "skip_tutorial": false,
    "telemetry": false,
    "import_starter_themes": true
  }
}
```

`settings` est construit depuis les entrées `[[setup_option]]` : la valeur de chaque
option est écrite dans sa/ses clé(s) `maps_to` en retirant le préfixe `settings.`. Un
`select` resté sur le sentinel `"auto"` est d'abord résolu vers la valeur OS détectée
(pour que l'app reçoive toujours un choix concret).

## Ce que l'app doit faire (une fois, au 1er lancement)

1. Lire le fichier depuis son propre dossier de données ; ignorer si
   `source != "betterinstaller"`.
2. Appliquer `settings` à sa propre config — **valider/borner chaque valeur**, ne jamais
   faire confiance aveuglément. Les clés inconnues sont ignorées.
3. Le renommer en `installer-handoff.consumed.json` pour qu'il ne se ré-applique jamais.

### Forme de référence (n'importe quelle app Tauri / Electron / native)
- Une routine backend (ex. `consume_installer_handoff`) : lit le fichier, applique les
  réglages, copie les presets bundlés, marque consommé, renvoie un petit résultat (légal
  accepté, langue posée, chemin du preset, compteurs).
- Un hook frontend appelé avant tes modales de 1er lancement : pose les verrous « déjà
  onboardé » et importe tout preset bundlé via ton chemin d'import normal.

> L'exemple bundlé sous `examples/` implémente exactement ça de bout en bout — utilise-le
> comme template pour ta propre app.

## Faire le tien (3 étapes)

### 1. Décider ce que l'étape Setup demande — `[[setup_option]]`

Chaque bloc ajoute un contrôle à la page Setup de l'installeur et une entrée au `settings`
du handoff. Ajoute ceux que tu veux, supprime le reste — aucune option n'est requise.

Chaque bloc a besoin de `id`, `type` (`bool` | `select` | `license`), un `label` (ou
`label_key`), et `maps_to`. Référence complète des champs dans [../GUIDE.md](https://github.com/FreeProject089/BetterInstaller/blob/master/GUIDE.md).

```toml
# Un toggle oui/non → bool
[[setup_option]]
id          = "telemetry"
type        = "bool"
label       = "Envoyer des stats d'usage anonymes"
description = "Opt-in. Aucune donnée perso."   # affiché sous le label (transparence)
default     = false
maps_to     = "settings.telemetry"             # → settings.telemetry dans le JSON

# Un menu déroulant → string
[[setup_option]]
id          = "language"
type        = "select"
label       = "Langue"
choices     = ["auto", "en", "fr"]             # "auto" → résolu vers la langue OS détectée
default     = "auto"
maps_to     = "settings.language"

# Une barrière légale (required = bloque Suivant tant que pas accepté) ; docs lus du paquet
[[setup_option]]
id          = "legal"
type        = "license"
label       = "Conditions d'utilisation & Confidentialité"
documents   = ["TOS.md", "PRIVACY.md"]
required    = true
maps_to     = ["settings.tos_accepted", "settings.privacy_accepted"]  # une option → plusieurs clés
```

> Tu veux un install fixe **sans question** ? Déclare zéro `[[setup_option]]` — l'étape
> Setup est sautée et le handoff enregistre quand même version/composants/install_dir.

### 2. Lire le fichier dans ton app (la partie qui n'est *pas* automatique)

Pseudo-code — adapte à ton langage :

```ts
const path = join(appDataDir(), "installer-handoff.json");
if (exists(path)) {
  const h = JSON.parse(read(path));
  if (h.source === "betterinstaller") {
    if (h.settings.language) setLanguage(validateLang(h.settings.language));
    if (h.settings.tos_accepted) markOnboarded();      // saute tes modales de 1er lancement
    applyTelemetry(!!h.settings.telemetry);
    // …applique seulement les clés que tu as définies ; ignore le reste…
    rename(path, path.replace(".json", ".consumed.json")); // ne jamais ré-appliquer
  }
}
```

Appelle ça **avant** ton UI de 1er lancement/onboarding pour pouvoir la supprimer.

### 3. (Optionnel) bundler du contenu prêt à l'emploi

Voir [Pré-import](#pré-import-contenu-bundlé) plus bas — dépose des fichiers dans le
`bundle/` de ton payload et conditionne-les derrière une option `import`.

## Pré-import (contenu bundlé)

L'installeur peut bundler du contenu supplémentaire sous `<install_dir>/presets/` (le
script de build copie `examples/<app>/bundle/presets/*`). Quand une option `import_*`
reste cochée, l'app copie ces fichiers au 1er lancement :

- `presets/Lang/*.json` → le dossier de langues de l'app (langues communautaires en plus).
- `presets/themes/*.json` → le dossier de thèmes de l'app.
- un export de réglages complet (le fichier de backup de ton app) → importé via
  l'importeur de backup normal de l'app.

Les langues intégrées sont livrées dans l'app et toujours présentes — les options
`import_*` n'ajoutent que du contenu **au-delà** des intégrées.
