# Security audit — September 24, 2026

Scope: the whole BetterInstaller workspace (`bpkg-core`, `bpkg-cli`, the Slint
`installer`) at `bcbd6af`, plus the docs, read against the code. It was run as an
adversarial review: several attack rounds, and each finding had to survive an attempt to
refute it before it was written down. Order of priority: security, reliability, simplicity,
performance, maintainability.

Every fix below ships with a test that was **born red**. It was checked by putting the bug
back and watching the test fail ("mutation-checked"). A test that has never failed on
demand proves nothing.

| # | Finding | Severity | CWE | Status |
|---|---|---|---|---|
| BI-01 | Uninstall deletes the whole folder the user picked, and everything in it | High | CWE-706 | Fixed |
| BI-02 | `protocol = ""` / `id = ""` make uninstall delete `HKCU\Software\Classes` / every per-user ARP entry | High | CWE-20 | Fixed |
| BI-03 | Update path fails open on a malformed `public_key`; `require_signature` without a key enforced nothing | High | CWE-636 | Fixed |
| BI-04 | Rollback deletes the only good copy of the install | Medium | CWE-460 | Fixed |
| BI-05 | Updates accept any package the key ever signed (rollback, replay, another app) | Medium | CWE-345 | Fixed |
| BI-06 | Code-signing the setup breaks it; the config (and the pinned key) is unauthenticated | Medium | CWE-345 | Fixed (+ docs) |
| BI-07 | DLL planting next to the setup (Downloads) | Medium | CWE-427 | Fixed |
| BI-08 | Verify-then-extract TOCTOU on the package file | Low | CWE-367 | Fixed |
| BI-09 | `taskkill /IM` for every `.exe` found in the chosen folder | Low | CWE-706 | Fixed |
| BI-10 | `bpkg keygen` silently replaces the publisher key; key file world-readable on Unix | Low | CWE-276 | Fixed |
| BI-11 | Green "Signed" badge for a signature nobody checked | Low | CWE-451 | Fixed |
| BI-12 | Allocation sized by unchecked header lengths; overflowing SFX trailer; hex-parser panic | Low | CWE-789, CWE-190 | Fixed |
| BI-13 | `quick-xml` 0.39.4 (RUSTSEC-2026-0194/0195) | Info | — | Fixed (lockfile) |

CVSS vectors are CVSS 3.1 base scores. Several of the findings need no attacker at all
(BI-01, BI-02, BI-04). For those the vector describes the accidental trigger, and the
severity column reflects how likely and how bad the outcome is.

---

## BI-01 — Uninstall deletes the whole folder the user picked

**CVSS 7.1** `AV:L/AC:L/PR:N/UI:R/S:U/C:N/I:H/A:H` · CWE-706 (the object to delete was
resolved as "the folder", not "what we installed")

**Trigger.** On the Setup page, "Browse…" sets the install directory to the picked folder
itself. Nothing appends `\<app name>`. A user picks `D:\Games` and installs, then
uninstalls from Apps & Features. `do_uninstall_full` ran `remove_dir_except(D:\Games)`
(everything but the running uninstaller), then `rmdir /S /Q "D:\Games"` from the
self-delete script. Every game in the folder was deleted. On Linux and macOS the same
path was a plain `remove_dir_all`. No attacker is needed, and nothing warns the user
before it happens.

**Fix.** `crates/installer/src/uninstall.rs` (new) and `run_real_install`:
- Before anything is written, the install decides whether it **owns** the folder: the
  folder did not exist, or existed empty. A repair or update keeps the first install's
  answer.
- `uninstall-info.json` now records `owns_dir` and `files` (the files actually written,
  merged across repairs).
- Uninstall removes the folder whole only when it is owned. Otherwise it removes the
  listed files and the directories they leave empty. The file list is read back from
  disk, so every entry is checked to stay inside the folder (same rule as archive entries,
  now exported as `bpkg_core::package::is_safe_entry_path`).
- A drive root, a folder directly below one, and the user's home are never removed whole,
  whatever the record says.
- For installs recorded before this change, a folder named exactly after the app counts
  as owned (that is the default layout). Any other folder falls back to the embedded
  package's file list.

**Why it holds.** Recursive deletion now requires a fact established before the first
file landed. In the `D:\Games` case the folder was not empty at install time, so
`owns_dir` is false and only package paths are deleted. `..` and absolute entries are
rejected, so a tampered record cannot name anything outside the folder.

**Tests** (`uninstall.rs`, all mutation-checked):
`installing_into_a_folder_that_held_files_does_not_own_it`,
`uninstalling_from_a_shared_folder_removes_only_what_was_installed` (user files and a
file outside the folder survive; a hostile `../outside.txt` entry is ignored),
`an_owned_folder_is_removed_whole_but_never_a_root_or_a_drive_folder`,
`a_legacy_record_is_owned_only_when_the_folder_is_named_after_the_app`,
`repairs_keep_every_file_an_earlier_run_installed`.

**Residual.** In a folder the install does not own, files added later by a *remote*
update, and zip prerequisites unpacked under the install folder, are not in the record
and are left behind on uninstall. That is leftover files, not data loss (card C-3).

## BI-02 — Empty or odd names become registry keys that uninstall deletes recursively

**CVSS 6.3** `AV:L/AC:H/PR:N/UI:R/S:U/C:N/I:H/A:H` · CWE-20

**Trigger.** Three `installer.toml` strings become OS names without any check:
`install.protocol` → `HKCU\Software\Classes\<scheme>`, `app.id` →
`HKCU\…\Uninstall\<id>` and the app-data folder, `app.name` → the shortcut file and the
default install folder.
- `protocol = ""` is a natural way to write "none". The handler was then registered on
  `HKCU\Software\Classes` itself. Uninstall ran
  `delete_subkey_all("Software\\Classes\\")`, which is `RegDeleteTreeW`. **Verified** on
  a scratch key (created, probed, removed): a trailing backslash deletes the key and its
  whole tree. The result is every per-user file association and COM registration gone.
- `id = ""` does the same to every per-user Apps & Features entry.
- `name = "..\..\Startup\x"` writes the Start Menu shortcut into the Startup folder.

The config sits outside the signed package (BI-06), so a publisher's typo and a
re-stamped setup reach these the same way. The uninstall side reads the same values back
from `uninstall-info.json`.

**Fix.** `InstallerConfig::validate_identity`, run at load time:
- `app.id`: `[A-Za-z0-9._-]`, at most 128 characters, no leading dot.
- `app.name`: a single Windows-valid file name.
- `install.protocol`: an RFC 3986 scheme.

The Windows, Linux and macOS `unregister_*` / `remove_shortcuts` functions now refuse an
invalid value before deleting anything, and `register_protocol` refuses an invalid scheme.

**Why it holds.** The empty string, separators and `..` cannot pass either gate, so no
registry path or file path built from these values can name a parent of the app's own
key.

**Tests.** `names_that_would_become_the_wrong_registry_key_or_path_do_not_load`
(mutation-checked) and `the_real_names_still_load` (BMM's own id, name and scheme).

## BI-03 — Update fails open on a malformed key; `require_signature` without a key

**CVSS 7.5** `AV:N/AC:H/PR:N/UI:R/S:U/C:H/I:H/A:H` (precondition: a misconfigured
`[security]` plus control of a mirror or the manifest host) · CWE-636

**Trigger.**
- The GUI's remote update built its key with
  `parse_public(pk).ok()`. A `public_key` with a typo or a truncated paste became
  `None`, and `download_and_apply(…, None)` applies an **unverified** download. The local
  install path failed closed on the same typo, so a test of the setup looked fine.
- `require_signature = true` with no `public_key`: every check sat inside
  `if let Some(key)`, so the setting refused nothing on install or update.

**Fix.**
- `InstallerConfig::validate_security`: a `public_key` that does not parse, or
  `require_signature` without a key, makes the config fail to load. Every consumer is
  covered, the CLI and `bpkg pack` included.
- The GUI update path now matches on the parse result and refuses on error, or on a
  missing key when signatures are required.
- `run_real_install` refuses `require_signature` without a key, and now verifies the
  package **before** prerequisites are run and before the running app is killed.

**Why it holds.** "No key" can now only mean that the config sets no key and does not
require one. Every other combination either loads with a valid key or does not load.

**Tests.** `requiring_signatures_without_a_key_does_not_load`,
`a_public_key_that_is_not_a_key_does_not_load` (both mutation-checked) and
`the_sound_combinations_still_load`.

## BI-04 — Rollback deletes the only good copy of the install

**CVSS 6.3** `AV:L/AC:H/PR:N/UI:R/S:U/C:N/I:H/A:H` · CWE-460

**Trigger.** The most common reason an update fails is a file held open: the running
app, or an antivirus scan. That same file also makes the restore fail. The restore
(`copy_dir`) stopped at its first error, and the snapshot was deleted whatever happened
(`let _ = copy_dir(…); let _ = remove_path(&backup)`). Every file after the held one in
directory order was then gone from the install **and** from the snapshot. Separately, a
`.bak` left by an interrupted update (power cut, killed process) was deleted at the start
of the next update, which then snapshotted the half-written folder in its place.

**Fix** (`update.rs`):
- The restore continues past failures and counts them. The snapshot is deleted only when
  nothing failed. Otherwise the error names the `.bak` folder, which is kept.
- An existing `.bak` makes the update refuse to start, with its path in the message.
- A partial snapshot from the current call is cleaned up if the snapshot itself fails.
- Snapshot and wipe no longer follow symlinks or junctions. `is_dir()` used to follow
  them and copy whatever they pointed at (a mods library, or a link back to an ancestor,
  which never finished), and a rollback then replaced the link with a copy.

**Tests** (`tests/install_rollback.rs`):
- `a_rollback_that_cannot_finish_keeps_the_snapshot_and_restores_the_rest` holds a file
  open with read-only sharing on Windows, and uses a read-only file in a read-only folder
  on Unix. It was red on the old code: `sub/`, `y.txt` and `z.txt` were lost.
- `an_unfinished_earlier_update_is_never_overwritten` (mutation-checked).
- `a_link_in_the_install_dir_is_neither_copied_nor_replaced` (Unix).

Linux runs were done as a non-root user, so none of these tests skipped.

## BI-05 — Updates accept any package the key ever signed

**CVSS 5.3** `AV:N/AC:H/PR:N/UI:R/S:U/C:N/I:H/A:N` · CWE-345 (rollback / freeze
attacks, in The Update Framework's terms)

**Trigger.** `update.json` names a version and URLs. Neither is signed, and neither is
compared with the package that arrives. A mirror listed in `urls`, or whoever controls
the manifest host, could serve the 1.0.0 release (genuinely signed, with the bugs it was
replaced for) as the 1.3.0 update. It could also replay the installed release forever, or
serve another app signed by the same publisher key. `docs/UPDATES.md` said a mirror
"can serve a bad file and still never get it applied", and SECURITY.md lists downgrade
attacks as in scope.

**Fix.** `update::check_offered`, run after the signature check on the package's own
**signed** manifest. The package must be `app_id`, at exactly the offered version, and
that version must be newer than the one installed. `download_and_apply` takes the app id:
the GUI passes `[app].id`, the CLI passes `None` (the version rules still apply).
`apply_downloaded` splits the download from the apply, so the path is testable offline.

**Tests.** `a_genuinely_signed_older_release_is_not_an_update` (mutation-checked) and
`the_offered_version_of_this_app_and_only_that_is_accepted`.

**Residual.** `update.json` is still unsigned. Its host can withhold updates, or offer a
genuine release that is newer than the installed one but older than the latest. Now
documented (card C-1).

## BI-06 — Code-signing the setup breaks it; the config is unauthenticated

**CVSS 7.5 for the unauthenticated config** `AV:N/AC:H/PR:N/UI:R/S:U/C:H/I:H/A:H` ·
CWE-345

**Trigger.** `installer.toml` is appended to the setup **outside** the Ed25519-signed
package, and `public_key` is part of it. A re-stamped setup can therefore carry the
attacker's key and a package signed with it, and the Welcome page shows "Signed &
verified · BetterCommunity". The only thing that can authenticate the config is an
Authenticode signature over the finished `*-Setup.exe`, which is exactly what the docs
recommend. But `signtool` appends the certificate table at the end of the file, and
`read_embedded` looked for its trailer in the last 24 bytes only. **A signed setup read as
"not stamped" and fell back to dev mode.** The docs also called Authenticode "orthogonal
to the `[security]` package signing".

**Fix.** `embed.rs` parses the PE security directory. When the certificate table ends
the file, the reader looks for the trailer just before it, allowing up to 7 zero bytes of
alignment. Real signed binaries were checked for the layout this relies on: `node.exe`
and `git.exe` both have an 8-aligned table ending exactly at EOF. The Authenticode hash
covers the overlay before the table, so signing after `bpkg build` authenticates the
engine, the config and the package together. The trailer length sum is now checked
arithmetic. SIGNING.md and platform-windows.md (EN + FR) now say what the package
signature does not cover and to sign **after** `bpkg build`.

**Tests.** `a_code_signed_setup_still_finds_its_payload` (three certificate sizes;
signed-but-not-stamped stays "not stamped"; bytes after the table are refused;
mutation-checked) and `trailer_lengths_that_overflow_are_corrupt`.

**Not verified:** signing with a real certificate (none available here).

## BI-07 — DLL planting next to the setup

**CVSS 7.0** `AV:L/AC:H/PR:N/UI:R/S:U/C:H/I:H/A:H` · CWE-427

**Trigger.** A setup is run from Downloads, where browsers drop files without asking.
Windows resolves a DLL from the executable's folder first. A `version.dll` or
`dwmapi.dll` that arrived there earlier runs inside the setup, under its name and
signature.

**Fix.** `build.rs` links the installer with `/DEPENDENTLOADFLAG:0x800`, which makes
static imports resolve from System32 only (MSVC only). `harden_dll_search()` calls
`SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)` first thing in `main`, for
everything loaded later (GPU and windowing stack, file dialog).

**Test.** `crates/installer/tests/pe_hardening.rs` reads `DependentLoadFlags` back out of
the built executable (`0x800`). It was mutation-checked by removing the link argument.
The GUI was smoke-launched after the change and stayed up (3 runs, killed after 6 s).

## BI-08 — Verify-then-extract TOCTOU

**CVSS 4.4** `AV:L/AC:H/PR:L/UI:R/S:U/C:N/I:H/A:N` · CWE-367

**Trigger.** One path was read three times: the manifest at `open`, the signed range at
`verify_signature`, the payload at extraction. An attacker who swaps the file between
reads can pass verification and still install an unsigned payload: hostile manifest at
open, genuine bytes while verifying, hostile payload at extraction. The scratch
directory (`tmp.rs`) limits this to the same user for the GUI. The CLI's `--key` path
takes any path, a shared folder included.

**Fix** (`reader.rs`). `open` keeps the raw header and manifest. `verify_signature`
refuses (`Ok(false)`) when the bytes it verified do not start with exactly those, and
keeps the verified bytes. Every later read of the payload is served from them.

**Test.** `a_package_swapped_between_reads_never_installs_as_verified` (both swap
orders, mutation-checked).

## BI-09 — `taskkill /IM` for every `.exe` in the chosen folder

**CVSS 3.3** `AV:L/AC:L/PR:N/UI:R/S:U/C:N/I:N/A:L`

**Trigger.** Install, repair and uninstall force-closed every running process whose name
matched **any** `.exe` found in the folder. `taskkill /IM` matches by name across the whole
system, so installing into a folder of other tools killed those tools' running copies.

**Fix.** Only the package's own top-level `.exe` entries are closed, taken from the
manifest or the recorded file list. The kill also moved after the signature check.

**Test.** `only_the_packages_own_top_level_executables_are_closed`.

**Residual.** Name-based: a second copy of the same app running from elsewhere is still
closed (card C-6).

## BI-10 — `bpkg keygen` replaces the publisher key

**Trigger.** Running `bpkg keygen` twice with the default `--out keys` overwrote
`private.key`. Every installed copy pins the old public key, so none of them could ever be
updated again. On Unix the key was also written 0644.

**Fix.** `save_private` uses `create_new` (refuses to overwrite, with a message about
rotation) and mode 0600 on Unix.

**Test.** `a_second_keygen_never_replaces_the_publisher_key` (mutation-checked).

## BI-11 — Green badge for an unchecked signature

With no `public_key` configured, `Trust::Signed` showed the same green shield as a
verified package. The flag bit plus 64 arbitrary bytes were enough, shown next to a
publisher name taken from the unsigned config. Only `Verified` is green now, and the
SIGNING.md badge table documents the state. Test:
`only_a_checked_signature_gets_the_green_badge`.

## BI-12 — Robustness of the parsers

- `Package::open` now bounds `24 + manifest_len + payload_len` by the file size before
  any allocation. A 40-byte file claiming a huge payload used to be an allocation
  failure, which is an abort with `panic = "abort"`. Test:
  `header_lengths_larger_than_the_file_are_refused_before_allocating`.
- `read_embedded`'s trailer sum is checked (BI-06).
- `hex_decode` sliced a `&str` two bytes at a time and panicked on a 3-byte character.
  It now refuses non-ASCII. Test: `a_key_with_non_ascii_text_is_an_error`
  (mutation-checked).

## BI-13 — Dependencies

`cargo audit`: 2 vulnerabilities, both `quick-xml` 0.39.4 (RUSTSEC-2026-0194/0195). The
crate is reached only through the `wayland-scanner` proc-macro, at build time and on Linux
only. `cargo update -p wayland-scanner` (0.31.10 → 0.31.11) takes it to 0.41.0.
`cargo audit` now reports **0 vulnerabilities**. The warnings that remain are
unmaintained or unsound transitive crates of the Slint stack (bincode, paste, rustybuzz,
ttf-parser, event-listener, a yanked chacha20), with no fixed version to move to.

---

## Code quality changes

- `platform/receipt.rs` hand-rolled its JSON writer and reader. They escaped `\` and `"`
  but not control characters (a Linux path may contain a newline), and the round-trip
  test re-implemented the reader inline, so it tested a copy. It now uses `serde_json`,
  already a dependency. The test goes through the real reader, and old receipts still
  read.
- Stale comments claiming the signature covers "manifest+payload" (`sign.rs`,
  `package/mod.rs`) corrected. The v2 format signs bytes `0 .. 24+N+M`.
- `apply_package_update` split into a public wrapper and `apply_checked`, so the update
  identity gate runs after the signature and before any write, without duplicating the
  rollback.

## Docs corrected (EN + FR)

The rule is that the docs must not promise more than the code does.
- UPDATES: what the rollback does and does not promise; what a mirror can no longer do
  (BI-05); that `update.json` is unsigned and what its host can still do; how
  `require_signature` and `public_key` are enforced.
- SIGNING: the badge for "signed, no key"; a section on what the package signature does
  **not** cover, and that Authenticode over the finished setup is what covers it.
- platform-windows: sign **after** `bpkg build`; DLL-search hardening; uninstall removes
  a folder only when the install created it.

## Remaining improvements (owner cards)

- **C-1 Signed update metadata.** Sign `update.json` with the publisher key and check it
  before trusting `version`/`url`, with an expiry to stop freeze attacks. This closes the
  BI-05 residual. It is a format decision.
- **C-2 Close the app before a remote update.** The local path closes it; the remote path
  does not, so an update started while the app runs fails (now with a clean rollback).
  Close it after the download and before the apply.
- **C-3 Record what updates and zip prerequisites add** to `uninstall-info.json`, so a
  non-owned folder is cleaned completely.
- **C-4 "Browse…" UX.** Offer `\<app name>` when the picked folder is not empty, like most
  installers. This is a product choice; BI-01 makes it safe either way.
- **C-5 One version comparator.** `update::is_newer`, the installer's `version_gt` and
  `crate::version` disagree on pre-release suffixes (`1.2.0-beta`).
- **C-6 Kill by path, not by name** (Toolhelp32 + `QueryFullProcessImageNameW`).
- **C-7** `UPDATES.fr.md` still lacks the `urls` mirror field in its JSON example (EN/FR
  drift that predates this audit).
- **C-8 Privacy.** `session_recorder` defaults to on under an opt-in `telemetry` parent.
  This is deliberately excepted in the defaults test and harmless while the parent is
  off. Worth one line in the privacy review.

## What was not reached

- A real Authenticode signature (no certificate). The layout was verified against signed
  binaries and a synthetic PE.
- A real install / uninstall / registry round trip. By rule, nothing was run against the
  system. The one registry behaviour the BI-02 finding depends on was proved on a scratch
  key that was created and removed.
- macOS (no machine). Linux was run in the repository's Docker image, as root for the CI
  gate and as a non-root user for the permission-based tests.
- A remote update against a live server. The apply half is tested offline through
  `apply_downloaded`.

## Gates run

Windows, the same commands as CI: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`. All
green; 103 tests before, 126 after. Linux (`betterinstaller-dev` image, `--locked`, run
as a non-root user so the permission-based tests do not skip): the same three, green, 130
tests.
