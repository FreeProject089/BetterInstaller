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

The signed update manifest also carries the SHA-256 of the `installer.toml` stamped into
that release's setup (`config_sha256`). `bpkg verify-manifest update.json --key
public.key --setup App-Setup.exe` checks a setup against it, with the publisher key you
already hold rather than one the setup brings. That makes a re-stamped config
**detectable** by whoever runs the check (a release pipeline, a download page, a
support person); it does not stop a user from running a re-stamped setup. The setup does
not check itself at install time: a first install makes no network request, and a check
against the key in its own config would prove nothing. Only an Authenticode signature,
which needs a code-signing certificate, closes that.

## The update manifest (`update.json`)

The same key signs `update.json` (`bpkg update-manifest`), over the context line
`BetterInstaller update manifest v1` and a newline followed by the exact bytes of the
signed body. The body names the app, the version, the URLs, the package's SHA-256, the
config's SHA-256, and an expiry at most 7 days after signing; renew it with
`bpkg resign-manifest`. With a `public_key` set, an installer refuses a manifest that is
unsigned, signed by another key, edited, expired, for another app, or not newer than
what is installed. Format, migration rule and hosting: [UPDATES.md](UPDATES.md).

Keep the private key where the weekly re-signing runs (a CI secret, an offline machine
with a reminder): an expired manifest is refused, so a key nobody can reach for a week
stops updates until it is used again.

## Rotating keys

Sign a release with the new key, ship a build whose `public_key` is the new one, and
keep the old key only long enough to sign a transitional update (the update being
applied must verify against the **installed** build's key).
