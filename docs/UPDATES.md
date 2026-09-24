# Updates

🇬🇧 English · [🇫🇷 Français](https://github.com/FreeProject089/BetterInstaller/blob/master/docs/UPDATES.fr.md)

The update engine (`bpkg-core/src/update.rs`) downloads a newer package and applies
it over the install dir with an **atomic-ish rollback**: it snapshots the dir to a
sibling `<name>.bak`, extracts the new package over it, and on **any** error wipes
and restores from the snapshot; on success it drops the snapshot.

What the rollback does and does not promise:

- The snapshot is deleted only when **every** file came back. If a file cannot be
  restored (typically one still held open — the running app, an antivirus scan), the
  restore carries on with the others and the error names the `.bak` folder, which is
  kept intact.
- A `.bak` already present when an update starts is what an interrupted update leaves
  (power cut, killed process, an unfinished rollback) and may be the only good copy. The
  update then **refuses to start** and names it; restore it or delete it, then retry.
- Symbolic links and junctions inside the install dir are neither followed nor copied
  into the snapshot, and a rollback leaves them in place.

## Configure it (`installer.toml`)

```toml
[update]
manifest_url = "https://…/update.json"   # a stable JSON URL you control
auto_check   = true                        # check when the maintenance window opens
allow_delta  = true                        # prefer a small binary patch when offered
```

In maintenance mode the GUI checks the manifest in the background; if it advertises a
newer version, the **Update** button appears (showing `v<old> → v<new>`) and applies
it. With no `[update]`, Update still appears when the *bundled* setup is newer than
what's installed (it re-extracts the embedded package).

## Update manifest (JSON you host, signed)

`update.json` decides which version is offered and where it is downloaded from, so it is
**signed with the publisher key** — the same Ed25519 key that signs the packages. Produce
it with `bpkg update-manifest`, never by hand:

```json
{
  "version": "1.2.0",
  "url": "https://…/App-1.2.0.bpkg",
  "urls": ["https://mirror.example/App-1.2.0.bpkg"],
  "notes": "optional changelog text",
  "deltas": [
    { "from": "1.1.0", "url": "https://…/1.1.0-to-1.2.0.patch" }
  ],
  "signed": "{\"app_id\":\"com.example.app\",\"version\":\"1.2.0\",\"url\":\"https://…/App-1.2.0.bpkg\",…,\"sha256\":\"…\",\"config_sha256\":\"…\",\"issued\":\"2026-09-24T12:00:00Z\",\"expires\":\"2026-10-01T12:00:00Z\"}",
  "signature": "<128 hex characters>"
}
```

- **`signed`** is a JSON document carried as a string; the signature covers its exact
  bytes, preceded by the context line `BetterInstaller update manifest v1` and a newline
  (so a manifest signature can never pass for a package signature, or the reverse). It
  holds `app_id`, `version`, `url`, `urls`, `notes`, `deltas`, the SHA-256 of the full
  `.bpkg` (`sha256`), the SHA-256 of the `installer.toml` stamped into that release's
  setup (`config_sha256`), and `issued` / `expires` (RFC 3339, at most **7 days** apart).
- The **top-level** `version`, `url`, `urls`, `notes` and `deltas` are a copy for
  installers built before signed manifests, which ignore the fields they do not know. An
  installer that verifies reads the **signed** part only and never falls back to the copy.

With a `public_key` in `installer.toml`, the installer refuses:

- an **unsigned** manifest (except under the migration rule below);
- a manifest whose signature does not verify with the pinned key, or that was edited
  after signing;
- an **expired** manifest (`expires` in the past), or one signed for more than 7 days;
- a manifest for another app (`app_id` ≠ `[app].id`);
- a manifest whose version is **not newer** than the installed one: it is never offered,
  and its package is refused if something applies it anyway (rollback protection).

A source that fails any of these counts as a failed source: with several
`manifest_urls`, the others are still read, and the newest valid one wins. With no
`public_key` there is nothing to verify against: the manifest is read as given (the
signed part when present, and its expiry still applies), like an unsigned package.

Before the install directory is touched, the download must then (1) match the `sha256`
of the signed manifest, (2) carry a valid Ed25519 signature for the pinned key and (3) be,
according to its own **signed** package manifest, the app being updated (`[app].id`) at
exactly the `version` offered — newer than the installed one. A mirror can therefore
serve a bad file, an older release, or another package the same key signed, and none of
them is applied. Only the last download error is reported.

- `version` is compared by the one version rule of the engine (`bpkg_core::version`): the
  first dotted number run, component by component, a missing component being zero
  (`1.2` = `1.2.0`, `v1.3.0` > `1.2.0`). A suffix is not part of the version:
  `1.2.0-beta.2`, `1.2.0+5` and `1.2.0` are the **same** release. Give a pre-release its
  own numbers (`1.2.90`) if it must be offered as an update.
- If a `deltas` entry matches the installed version **and** the current `.bpkg` is
  available, a small bsdiff patch is downloaded and the new package is rebuilt locally;
  the result must still match `sha256`. Otherwise the full `url` is downloaded.

What the signed manifest still cannot stop: whoever controls the host can **withhold** the
manifest, but not for long — after at most 7 days the last copy clients saw has expired:
the remote update is no longer offered and `--check-update` reports an error (exit 2)
rather than "up to date". The price is on the
publisher's side: **re-sign `update.json` at least once a week** between releases
(`bpkg resign-manifest`), or every installed copy stops seeing updates.

### Migration (one engine release)

Installers built before signed manifests read only the top-level copy; they cannot check
anything, whatever the file says. For installers that verify, the rule is:

> An **unsigned** `update.json` is accepted — flagged, and the user is told on the result
> page ("the update information was not signed…") — only when **both** hold: the engine
> still carries `MIGRATION_ACCEPTS_UNSIGNED = true` (this release only; it is set to
> `false` in the next one), **and** the install being updated was recorded by an engine
> from before signed manifests (its `uninstall-info.json` exists and has no
> `signed_update_manifests` key). The update that follows writes that key, so each install
> gets this once. With no `uninstall-info.json` at all, the strict rule applies.

`bpkg fetch-update` has no install record to read and never applies the migration rule.

### What the update does to the running app and to the uninstall record

- **Closing the app.** Once the download has passed every check above, and before the
  install directory is snapshotted, the installer closes the app's own running processes
  (the package's top-level `.exe` files), as the local update path always did. A process
  is closed only if its executable **is** that file in the install directory (full image
  path, compared after canonicalisation), so a second copy of the same program running
  from another folder is left alone.
- **Uninstall record.** The files an update writes are added to `uninstall-info.json`.
  Uninstalling from a folder the install does not own then removes the files a later
  version added, too.

## Producing an update

```sh
# build + sign the new version
bpkg pack --root payload --config installer.toml --out App-1.2.0.bpkg
bpkg sign --key keys/private.key App-1.2.0.bpkg

# (optional) a delta from the previous release
bpkg delta --old App-1.1.0.bpkg --new App-1.2.0.bpkg --out 1.1.0-to-1.2.0.patch

# the signed manifest (reads app id, version and hash from the package)
bpkg update-manifest --package App-1.2.0.bpkg --config installer.toml \
  --key keys/private.key --url https://…/App-1.2.0.bpkg \
  --mirror https://mirror.example/App-1.2.0.bpkg \
  --delta "1.1.0=https://…/1.1.0-to-1.2.0.patch" \
  --notes "What's new" --out update.json

# host App-1.2.0.bpkg, the patch, and update.json at stable URLs

# at least weekly until the next release (a scheduled CI job holding the key):
bpkg resign-manifest update.json --key keys/private.key
# … then re-upload update.json wherever it is served
```

`update-manifest` refuses a package this key did not sign, and a config whose
`[app].id` / `version` differ from the package. `resign-manifest` only renews a manifest
this key already signed (its expiry may be past).

The new package must be signed by the **same key** as the installed one. With a
`public_key` set, the signature is always checked before applying; a `public_key` that
does not parse, or `require_signature = true` without a `public_key`, makes
`installer.toml` fail to load rather than falling back to an unverified update.

## CLI (manual / scripted)

```sh
bpkg fetch-update --url https://…/update.json --dir <install_dir> --current 1.1.0 \
  --key keys/public.key                              # manifest AND package verified
bpkg update App-1.2.0.bpkg --dir <install_dir>     # apply a local pkg with rollback
bpkg verify-manifest update.json --key keys/public.key [--app-id <id>] [--setup App-Setup.exe]
```

`verify-manifest --setup` also checks that a setup carries exactly the `installer.toml`
and the package this release's manifest names. That is the one check on the config that
does not depend on a key the config itself provides (see [SIGNING.md](SIGNING.md)).
