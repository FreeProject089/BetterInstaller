# Signing

🇬🇧 English · [🇫🇷 Français](https://github.com/FreeProject089/BetterInstaller/blob/master/docs/SIGNING.fr.md)

Packages are signed with **Ed25519** (ed25519-dalek). The installer can refuse to
install anything not signed by your key, and the Welcome page shows a trust badge.

## Keys

```sh
bpkg keygen --out keys      # → keys/private.key (SECRET), keys/public.key
```

- **`private.key`** signs releases. Keep it offline/secret. It is gitignored
  (`private.key`, `**/keys/`) — never commit it.
- **`public.key`** is the trust anchor — paste its hex into `installer.toml`:

```toml
[security]
public_key        = "8e0647…168b"   # contents of keys/public.key
require_signature = true             # refuse unsigned / invalid packages
```

## Sign + verify

```sh
bpkg sign   --key keys/private.key app.bpkg     # run after every `pack`
bpkg verify app.bpkg --key keys/public.key      # OK — Ed25519 signature valid
```

## What is signed

The 64-byte signature (appended last, with `FLAG_SIGNED` set in the header before
signing) covers **bytes 0 .. 24+N+M** — the whole file up to the signature itself.
Tampering with the manifest, the payload OR the header invalidates it.

The header matters because it holds `manifest_len` and `payload_len`, which a verifier
reads to decide how much to hash. Format v1 signed only the manifest and payload — and
these docs described it as covering `header[6..]`, which it did not — so the two
numbers choosing the verified range sat outside it. See
[BPKG-FORMAT.md](BPKG-FORMAT.md).

## Welcome-page badge

| State | Badge |
|---|---|
| Signature valid against `public_key` | **Signed & verified · `<publisher>`** (green) |
| No signature | **Unsigned package · `<publisher>`** (red if `require_signature`) |
| Signature present but invalid | **Signature INVALID — do not trust** (red, install blocked) |
| Signed, but no `public_key` configured | **Signed · `<publisher>`** (NOT green: nothing was checked) |

`<publisher>` comes from `installer.toml`, which is not part of the signed package.

## What the package signature does not cover

`installer.toml` — including `public_key` itself — is appended to the setup **outside**
the signed package. A re-stamped setup can carry any config and any key, and then
verifies a package signed with that key as "Signed & verified". What authenticates the
config is an **Authenticode signature over the finished `*-Setup.exe`** (sign it after
`bpkg build`; the certificate covers the engine, the config and the package, and the
engine finds its payload in front of the certificate table). Ed25519 is what protects
**updates**: they are checked against the key of the build already installed.

## Rotating keys

Sign a release with the new key, ship a build whose `public_key` is the new one, and
keep the old key only long enough to sign a transitional update (the update being
applied must verify against the **installed** build's key).
