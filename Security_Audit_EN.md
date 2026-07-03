# BetterInstaller — Security Audit

**Date:** 2026-07-03 · **Scope:** the `bpkg-core` engine, `bpkg-cli`, and the `installer`
GUI (Rust workspace). **Focus:** the trust boundaries that matter for an
installer/updater — package authenticity (signing), untrusted-input parsing
(`.bpkg`), archive extraction (path traversal / zip-slip), the network update path,
and OS privilege. This is a source review, not a released-binary pentest.

**Headline:** the cryptographic foundations are sound (Ed25519 signing, per-file
SHA-256, bounds-checked parsing, per-user no-admin install). Two findings deserve
attention before a public release: a **Windows drive-absolute path-traversal gap** in
extraction, and the **core auto-update API applies packages without verifying the
signature** (authenticity is only enforced in the GUI path). Neither is a remote-code
slam-dunk on its own, but both weaken the "signed updates" guarantee the framework
advertises.

---

## Reviewed and found SOLID

- **Signing (`sign.rs`)** — Ed25519 via `ed25519-dalek`; keypair from the OS CSPRNG
  (`OsRng`); keys length-/hex-validated on load; the signature covers manifest+payload.
- **Integrity (`package/reader.rs`)** — every file's SHA-256 is checked against the
  manifest **before** it is written (`install_with_progress`), and `verify()` checks the
  whole package.
- **Parser safety** — the inner-archive reader is fully bounds-checked (`slice` uses
  `checked_add` and rejects `end > buf.len()`), so a truncated/hostile `.bpkg` yields a
  clean `Corrupt` error, not an out-of-bounds read or panic-on-slice.
- **Privilege (`platform/windows.rs`)** — **per-user, no admin/UAC** (`asInvoker`):
  installs to `%LOCALAPPDATA%\Programs`, writes only `HKCU`. This removes the entire
  class of elevated-installer risks (writing to Program Files/HKLM, UAC-bypass abuse).
- **TLS (`net.rs`)** — `reqwest` with **rustls** (no native-tls), request timeout,
  versioned User-Agent.
- **Update rollback (`update.rs`)** — snapshot to `<name>.bak`, restore on ANY error;
  covered by a test that corrupts a payload byte and asserts the pre-update state is
  restored.
- **GUI signature enforcement (`installer/src/main.rs`)** — when `security.public_key`
  is set it calls `verify_signature`, and `security.require_signature` aborts the install
  on a missing/invalid signature.

---

## Findings

| # | Severity | Issue |
|---|---|---|
| 1 | **Medium** | Windows drive-absolute path traversal in extract/install |
| 2 | **Medium** | Core auto-update API applies packages without verifying the signature |
| 3 | Low | Plaintext HTTP is accepted for manifests/downloads (no HTTPS enforcement) |
| 4 | Low (DoS) | Unbounded decompression / download into memory |
| 5 | Info | Unsigned/unconfigured packages install with no authenticity check |

### 1. Windows drive-absolute path traversal — *Medium*
`extract()` and `install_with_progress()` (`package/reader.rs`) guard with:
```rust
if path.contains("..") || path.starts_with('/') || path.starts_with('\\') { …reject… }
```
This blocks `../` and root-relative paths, but **not a Windows drive-absolute path** such
as `C:\Windows\System32\evil.dll` (or `C:foo`). On Windows, `dest.join("C:\\Windows\\…")`
**discards `dest`** because the argument is absolute — so a malicious manifest path
escapes the install directory. Exploitability requires a package the installer accepts
(unsigned, or signed by a trusted key), but extraction is also reachable before/without
signature verification.
**Fix:** reject any entry where `Path::new(path).is_absolute()` or whose components
contain a `Prefix`/`RootDir`, and/or canonicalize the joined path and assert it stays
within `dest` (`starts_with(dest)`). Apply to BOTH `extract` and `install_with_progress`.

### 2. Auto-update applies packages without verifying the signature — *Medium*
`update.rs::download_and_apply` → `apply_package_update` do `Package::open` +
`install_with_progress`, which verify each file's SHA-256 **against the package's own
manifest** — i.e. *self-consistency*, not *authenticity*. They take no `VerifyingKey`
and never call `verify_signature`. A malicious mirror or a network MITM can serve a
self-consistent, unsigned `.bpkg` at the update URL and it will be installed. The
installer GUI verifies separately, but the reusable engine API makes the **unverified
path the default**, so any consumer using `download_and_apply` directly gets no
authenticity guarantee.
**Fix:** thread a pinned `VerifyingKey` (from `config.security.public_key`) through
`download_and_apply`/`apply_package_update` and **fail closed** (verify signature before
staging/extracting), so signed-update safety isn't opt-in.

### 3. Plaintext HTTP accepted — *Low*
`net.rs` fetches whatever URL it's given, including `http://`. Package authenticity
(once finding #2 is fixed) protects the payload, but the **update manifest** (which
dictates the version and download URL) is unauthenticated JSON — over plaintext it can
be tampered/downgraded.
**Fix:** require `https://` (scheme allow-list) for manifest + download URLs, or sign the
manifest too.

### 4. Unbounded decompression / download — *Low (DoS)*
`read_archive` calls `zstd::decode_all` and `net::download` calls `resp.bytes()`, both
reading fully into memory with no cap. A hostile package or response could exhaust memory.
**Fix:** cap the decompressed size (and/or stream) and enforce a `Content-Length`/read
limit on downloads.

### 5. Unsigned/unconfigured packages install with no authenticity — *Info*
If `security.public_key` is unset the GUI skips verification entirely (trust-on-first-use).
That's a legitimate design choice, but it means the *default* posture for an app author
who doesn't configure signing is "install whatever you were handed."
**Recommendation:** document that publishers SHOULD set `public_key` + `require_signature`;
consider a loud UI warning when installing an unsigned package.

---

## Remediation (applied 2026-07-03)

- **#1 Path traversal — FIXED.** `package/reader.rs` now routes every archive entry
  through `unsafe_entry_path()`, which rejects traversal (`..`), POSIX-absolute,
  Windows drive-absolute (`C:\…`/`C:foo`) and UNC (`\\server`) paths — host-OS-
  independent, applied to both `extract()` and `install_with_progress()`. Covered by
  `rejects_escaping_entry_paths`.
- **#2 Update signature verification — FIXED.** `update.rs::apply_package_update` /
  `download_and_apply` now take a `verify_key: Option<&VerifyingKey>`; when a key is
  passed they verify the package's Ed25519 signature **before** snapshotting/writing and
  **fail closed** on missing/invalid. The installer's update path pins
  `security.public_key`. Covered by `update_refuses_unsigned_package_when_key_pinned`.
- **#3 Plaintext HTTP — FIXED.** `net.rs` `require_https()` rejects any non-`https://`
  manifest/download URL.
- **#4 Unbounded decompress/download — FIXED.** `read_archive` stream-decodes with a
  4 GiB ceiling; `net.rs` caps response bodies at 1 GiB.
- **#5 Unsigned default — INFO.** Documented; publishers should set `public_key` +
  `require_signature` (unchanged behaviour by design).

`cargo test -p bpkg-core` green (11 tests), `cargo check` + `clippy` clean across the
workspace.

## Recommendation summary

| Item | Severity | Status |
|---|---|---|
| Drive-absolute path traversal | Medium | **Fixed** — reject absolute/prefixed paths |
| Update API skips signature verify | Medium | **Fixed** — pinned-key verify, fail closed |
| Plaintext HTTP | Low | **Fixed** — HTTPS required |
| Unbounded decompress/download | Low | **Fixed** — size caps |
| Unsigned default | Info | Documented; encourage `require_signature` |
| Ed25519 signing, SHA-256, parser bounds, no-admin install, TLS, rollback | — | Sound — kept |
