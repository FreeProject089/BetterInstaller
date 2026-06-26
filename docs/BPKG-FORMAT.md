# The `.bpkg` package format

🇬🇧 English · [🇫🇷 Français](BPKG-FORMAT.fr.md)

A `.bpkg` is a single file: a fixed header, a JSON manifest, a compressed payload,
and an optional signature. All multi-byte integers are **little-endian**.

## Layout

```
┌───────────────────────────────────────────────┐
│ Header            24 bytes (fixed)             │
├───────────────────────────────────────────────┤
│ Manifest          manifest_len bytes (JSON)    │
├───────────────────────────────────────────────┤
│ Payload           payload_len bytes (zstd)     │
├───────────────────────────────────────────────┤
│ Signature         64 bytes  (only if FLAG_SIGNED) │
└───────────────────────────────────────────────┘
```

## Header (24 bytes)

| Offset | Size | Field | Value |
|---|---|---|---|
| 0 | 6 | magic | `42 50 4B 47 1A 00` (`BPKG\x1a\x00`) |
| 6 | 2 | format_version | `1` |
| 8 | 2 | flags | bit 0 `FLAG_SIGNED` (0x0001) |
| 10 | 2 | reserved | 0 |
| 12 | 4 | manifest_len | byte length of the JSON manifest |
| 16 | 8 | payload_len | byte length of the compressed payload |

## Manifest (JSON)

UTF-8 JSON, `manifest_len` bytes, immediately after the header:

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
    { "path": "myapp.exe", "size": 73964544,
      "sha256": "…", "component": null, "executable": true }
  ],
  "components": [
    { "id": "mcp-server", "name": "MCP AI Server", "description": "…", "required": false }
  ],
  "created_at": "2026-06-24T10:57:00Z",
  "total_size": 86310912
}
```

- Every payload file has a `sha256` — verified before it is written to disk.
- `component` ties a file to an optional component (`null` = always installed / core).

## Payload

A **zstd** stream (level 19). Decompressed, it is a flat sequence of entries:

```
repeat until end:
  path_len   u32        (byte length of the path)
  path       path_len   (UTF-8, forward slashes)
  data_len   u64
  data       data_len   (raw file bytes)
```

Path-traversal is rejected on extraction (`..`, leading `/` or `\`).

## Signature (optional)

If `FLAG_SIGNED` is set, the **last 64 bytes** are an Ed25519 signature over
`header[6..] ⧺ manifest ⧺ payload` — i.e. everything from `format_version` to the
end of the payload (the magic is excluded). Verified against the public key in
`[security].public_key`. See [SIGNING.md](SIGNING.md).

## Self-extracting installer (SFX)

`bpkg build` appends the config + package + a trailer to a copy of
`betterinstaller.exe`:

```
[ betterinstaller.exe                 ]
[ installer.toml bytes                ]
[ app.bpkg bytes                      ]
[ trailer: config_len(u64) | bpkg_len(u64) | "BPKGSFX1" ]   ← last 24 bytes
```

At launch the engine reads the last 24 bytes; if the magic `BPKGSFX1` is present it
slices the embedded config + package back out. A bare `betterinstaller.exe` (no
trailer) falls back to CLI args (`<installer.toml> [package.bpkg]`) for dev runs.

## Constants (source of truth: `crates/bpkg-core/src/package/format.rs`)

| Name | Value |
|---|---|
| `MAGIC` | `BPKG\x1a\x00` |
| `FORMAT_VERSION` | 1 |
| `HEADER_LEN` | 24 |
| `FLAG_SIGNED` | 0x0001 |
| `SIGNATURE_LEN` | 64 |
| `ZSTD_LEVEL` | 19 |
| SFX `TRAILER_MAGIC` | `BPKGSFX1` |
| SFX `TRAILER_LEN` | 24 |
