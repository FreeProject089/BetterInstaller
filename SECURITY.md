# Security Policy

🇬🇧 English · [🇫🇷 Français](SECURITY_FR.md)

BetterInstaller installs, updates and removes software, and verifies package integrity
and signatures. We take security seriously and appreciate responsible disclosure.

## Reporting a vulnerability

**Do not open a public issue for a security vulnerability.** Instead, report it
privately:

- Use GitHub's **[Private vulnerability reporting](https://github.com/FreeProject089/BetterInstaller/security/advisories/new)**
  (Security tab → *Report a vulnerability*), **or**
- Contact the maintainer directly (see the profile of
  [@FreeProject089](https://github.com/FreeProject089)).

Please include:

- A clear description and the **impact** (what an attacker can achieve).
- **Steps to reproduce** (a minimal `installer.toml` / package / command is ideal).
- Affected version(s) and OS, and any logs or a proof-of-concept.

We aim to acknowledge a report within **72 hours** and to provide a remediation
timeline after triage. Please give us a reasonable window to fix the issue before any
public disclosure; we're happy to credit you in the advisory.

## Scope — what we care about most

High-impact areas (treated as priority):

- **Signature / verification bypass** — installing or updating a package that should
  have been rejected (invalid/missing Ed25519 signature while `require_signature` is on).
- **Package integrity** — a crafted `.bpkg`/SFX that passes verification but writes
  different bytes than its manifest.
- **Path traversal / arbitrary write** — a manifest, component path, or update URL that
  escapes the install directory or writes outside it.
- **Update channel abuse** — downgrade attacks, swapping the package for an unsigned/
  malicious one, or a non-HTTPS update fetch.
- **Local privilege escalation** beyond the per-user (`asInvoker`) model, or executing
  attacker-controlled code during install/update/uninstall.

## Out of scope

- Issues requiring an already-compromised machine or a malicious *private signing key*
  (protect `private.key` — see [docs/SIGNING.md](docs/SIGNING.md)).
- SmartScreen/Gatekeeper reputation (those need Authenticode / Apple notarization,
  orthogonal to package signing).
- Social-engineering an end user into running an unrelated executable.

## Hardening notes for integrators

- Set `[security].require_signature = true` and ship the real `public_key`.
- Keep the **private key offline**; never commit it (it's gitignored).
- Host `update.json` and packages over **HTTPS**; the new package must be signed by the
  **same key** the installed build trusts.
- Validate anything your app reads from `installer-handoff.json` — treat it as input.

## Supported versions

Security fixes target the latest release (`main`). Older releases are fixed on a
best-effort basis.
