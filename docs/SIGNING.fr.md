# Signature

[🇬🇧 English](SIGNING.md) · 🇫🇷 Français

Les paquets sont signés avec **Ed25519** (ed25519-dalek). L'installeur peut refuser
d'installer quoi que ce soit qui n'est pas signé par ta clé, et la page Bienvenue
affiche un badge de confiance.

## Clés

```sh
bpkg keygen --out keys      # → keys/private.key (SECRÈTE), keys/public.key
```

- **`private.key`** signe les releases. Garde-la hors-ligne/secrète. Elle est gitignorée
  (`private.key`, `**/keys/`) — ne la commit jamais.
- **`public.key`** est l'ancre de confiance — colle son hex dans `installer.toml` :

```toml
[security]
public_key        = "8e0647…168b"   # contenu de keys/public.key
require_signature = true             # refuse les paquets non signés / invalides
```

## Signer + vérifier

```sh
bpkg sign   --key keys/private.key app.bpkg     # à lancer après chaque `pack`
bpkg verify app.bpkg --key keys/public.key      # OK — signature Ed25519 valide
```

## Ce qui est signé

La signature de 64 octets (ajoutée en dernier, avec `FLAG_SIGNED` dans le header)
couvre `header[6..] ⧺ manifest ⧺ payload` — tout sauf les 6 octets de magic. Toute
altération du manifest ou du payload l'invalide. Voir [BPKG-FORMAT.md](BPKG-FORMAT.md).

## Badge de la page Bienvenue

| État | Badge |
|---|---|
| Signature valide contre `public_key` | **Signé & vérifié · `<éditeur>`** (vert) |
| Pas de signature | **Paquet non signé · `<éditeur>`** (rouge si `require_signature`) |
| Signature présente mais invalide | **Signature INVALIDE — ne pas faire confiance** (rouge, install bloquée) |

## Rotation des clés

Signe une release avec la nouvelle clé, livre un build dont le `public_key` est la
nouvelle, et garde l'ancienne clé juste le temps de signer une mise à jour de
transition (la mise à jour appliquée doit se vérifier contre la clé du build
**installé**).
