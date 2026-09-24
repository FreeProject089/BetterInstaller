# Signature

[🇬🇧 English](https://github.com/FreeProject089/BetterInstaller/blob/master/docs/SIGNING.md) · 🇫🇷 Français

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

La signature de 64 octets (ajoutée en dernier, avec `FLAG_SIGNED` posé dans le header
AVANT signature) couvre les **octets 0 .. 24+N+M** — tout le fichier jusqu'à la
signature elle-même. Altérer le manifest, le payload OU le header l'invalide.

Le header compte parce qu'il porte `manifest_len` et `payload_len`, que le vérificateur
lit pour décider combien hacher. Le format v1 ne signait que le manifest et le payload
— et cette doc le décrivait comme couvrant `header[6..]`, ce qui était faux — donc les
deux nombres choisissant la plage vérifiée étaient hors de celle-ci. Voir
[BPKG-FORMAT.md](BPKG-FORMAT.md).

## Badge de la page Bienvenue

| État | Badge |
|---|---|
| Signature valide contre `public_key` | **Signé & vérifié · `<éditeur>`** (vert) |
| Pas de signature | **Paquet non signé · `<éditeur>`** (rouge si `require_signature`) |
| Signature présente mais invalide | **Signature INVALIDE — ne pas faire confiance** (rouge, install bloquée) |
| Signé, mais aucun `public_key` configuré | **Signé · `<éditeur>`** (PAS vert : rien n'a été vérifié) |

`<éditeur>` vient de `installer.toml`, qui ne fait pas partie du paquet signé.

## Ce que la signature du paquet ne couvre pas

`installer.toml` — y compris `public_key` lui-même — est ajouté au setup **hors** du
paquet signé. Un setup re-tamponné peut porter n'importe quelle config et n'importe
quelle clé, et vérifie alors comme « Signé & vérifié » un paquet signé avec cette clé.
Ce qui authentifie la config, c'est une **signature Authenticode du `*-Setup.exe` final**
(signe-le après `bpkg build` ; le certificat couvre le moteur, la config et le paquet, et
le moteur retrouve sa charge utile devant la table de certificats). Ed25519 protège les
**mises à jour** : elles sont vérifiées contre la clé du build déjà installé.

## Rotation des clés

Signe une release avec la nouvelle clé, livre un build dont le `public_key` est la
nouvelle, et garde l'ancienne clé juste le temps de signer une mise à jour de
transition (la mise à jour appliquée doit se vérifier contre la clé du build
**installé**).
