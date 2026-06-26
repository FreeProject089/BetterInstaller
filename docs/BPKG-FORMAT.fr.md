# Le format de paquet `.bpkg`

[🇬🇧 English](BPKG-FORMAT.md) · 🇫🇷 Français

Un `.bpkg` est un fichier unique : un header fixe, un manifest JSON, un payload compressé,
et une signature optionnelle. Tous les entiers multi-octets sont en **little-endian**.

## Disposition

```
┌───────────────────────────────────────────────┐
│ Header            24 octets (fixe)             │
├───────────────────────────────────────────────┤
│ Manifest          manifest_len octets (JSON)   │
├───────────────────────────────────────────────┤
│ Payload           payload_len octets (zstd)    │
├───────────────────────────────────────────────┤
│ Signature         64 octets (seulement si FLAG_SIGNED) │
└───────────────────────────────────────────────┘
```

## Header (24 octets)

| Offset | Taille | Champ | Valeur |
|---|---|---|---|
| 0 | 6 | magic | `42 50 4B 47 1A 00` (`BPKG\x1a\x00`) |
| 6 | 2 | format_version | `1` |
| 8 | 2 | flags | bit 0 `FLAG_SIGNED` (0x0001) |
| 10 | 2 | réservé | 0 |
| 12 | 4 | manifest_len | longueur en octets du manifest JSON |
| 16 | 8 | payload_len | longueur en octets du payload compressé |

## Manifest (JSON)

JSON UTF-8, `manifest_len` octets, immédiatement après le header :

```json
{
  "schema": 1,
  "app": {
    "id": "com.acme.editor",
    "name": "Acme Editor",
    "version": "1.0.0",
    "publisher": "BetterCommunity",
    "homepage": "https://…",
    "platforms": ["windows"]
  },
  "files": [
    { "path": "acme-editor.exe", "size": 73964544,
      "sha256": "…", "component": null, "executable": true }
  ],
  "components": [
    { "id": "mcp-server", "name": "MCP AI Server", "description": "…", "required": false }
  ],
  "created_at": "2026-06-24T10:57:00Z",
  "total_size": 86310912
}
```

- Chaque fichier du payload a un `sha256` — vérifié avant d'être écrit sur le disque.
- `component` lie un fichier à un composant optionnel (`null` = toujours installé / core).

## Payload

Un flux **zstd** (niveau 19). Décompressé, c'est une séquence plate d'entrées :

```
répéter jusqu'à la fin :
  path_len   u32        (longueur en octets du chemin)
  path       path_len   (UTF-8, slashes avant)
  data_len   u64
  data       data_len   (octets bruts du fichier)
```

Le path-traversal est rejeté à l'extraction (`..`, `/` ou `\` en tête).

## Signature (optionnelle)

Si `FLAG_SIGNED` est posé, les **64 derniers octets** sont une signature Ed25519 sur
`header[6..] ⧺ manifest ⧺ payload` — c'est-à-dire tout depuis `format_version` jusqu'à la
fin du payload (le magic est exclu). Vérifiée contre la clé publique de
`[security].public_key`. Voir [SIGNING.md](SIGNING.md).

## Installeur auto-extractible (SFX)

`bpkg build` ajoute la config + le paquet + un trailer à une copie de
`betterinstaller.exe` :

```
[ betterinstaller.exe                 ]
[ octets installer.toml               ]
[ octets app.bpkg                     ]
[ trailer : config_len(u64) | bpkg_len(u64) | "BPKGSFX1" ]   ← 24 derniers octets
```

Au lancement, le moteur lit les 24 derniers octets ; si le magic `BPKGSFX1` est présent,
il extrait la config + le paquet embarqués. Un `betterinstaller.exe` nu (sans trailer)
retombe sur les args CLI (`<installer.toml> [package.bpkg]`) pour les runs de dev.

## Constantes (source de vérité : `crates/bpkg-core/src/package/format.rs`)

| Nom | Valeur |
|---|---|
| `MAGIC` | `BPKG\x1a\x00` |
| `FORMAT_VERSION` | 1 |
| `HEADER_LEN` | 24 |
| `FLAG_SIGNED` | 0x0001 |
| `SIGNATURE_LEN` | 64 |
| `ZSTD_LEVEL` | 19 |
| SFX `TRAILER_MAGIC` | `BPKGSFX1` |
| SFX `TRAILER_LEN` | 24 |
