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

## Update manifest (JSON you host)

```json
{
  "version": "1.2.0",
  "url": "https://…/App-1.2.0.bpkg",
  "urls": ["https://mirror.example/App-1.2.0.bpkg"],
  "notes": "optional changelog text",
  "deltas": [
    { "from": "1.1.0", "url": "https://…/1.1.0-to-1.2.0.patch",
      "urls": ["https://mirror.example/1.1.0-to-1.2.0.patch"] }
  ]
}
```

- `version` is compared numerically component-wise (`is_newer`).
- `urls` (optional, on the manifest and on each delta) are **mirrors for the same file**,
  tried in order after `url` when a download fails. One unreachable host then stops being
  the reason nobody can update. Before the install directory is touched, the download
  must (1) carry a valid Ed25519 signature for the pinned key and (2) be, according to
  its own **signed** manifest, the app being updated (`[app].id`) at exactly the
  `version` offered — which must be newer than the installed one. A mirror can therefore
  serve a bad file, an older release the same key once signed, or another app by the same
  publisher, and none of them is applied. Only the last error is reported — three
  identical "no network" failures are not three pieces of information.
- What the signature cannot stop: `update.json` itself is **not** signed. Whoever
  controls the host serving it (or a mirror listed in `manifest_urls`) can withhold
  updates, or offer a genuine release that is newer than what is installed but older than
  the latest. HTTPS protects it in transit; nothing protects it at rest on the host.
- If a `deltas` entry matches the installed version **and** the current `.bpkg` is
  available, a small bsdiff patch is downloaded and the new package is reconstructed
  locally; otherwise the full `url` is downloaded.

## Producing an update

```sh
# build + sign the new version
bpkg pack --root payload --config installer.toml --out App-1.2.0.bpkg
bpkg sign --key keys/private.key App-1.2.0.bpkg

# (optional) a delta from the previous release
bpkg delta App-1.1.0.bpkg App-1.2.0.bpkg 1.1.0-to-1.2.0.patch

# host App-1.2.0.bpkg, the patch, and update.json at stable URLs
```

The new package must be signed by the **same key** as the installed one. With a
`public_key` set the signature is always checked before applying; a `public_key` that
does not parse, or `require_signature = true` without a `public_key`, makes
`installer.toml` fail to load rather than falling back to an unverified update.

## CLI (manual / scripted)

```sh
bpkg fetch-update --url https://…/update.json --dir <install_dir> --current 1.1.0
bpkg update App-1.2.0.bpkg --dir <install_dir>     # apply a local pkg with rollback
```
