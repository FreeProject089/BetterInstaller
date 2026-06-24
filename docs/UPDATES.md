# Updates

The update engine (`bpkg-core/src/update.rs`) downloads a newer package and applies
it over the install dir with an **atomic-ish rollback**: it snapshots the dir to a
sibling `<name>.bak`, extracts the new package over it, and on **any** error wipes
and restores from the snapshot; on success it drops the snapshot.

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
  "notes": "optional changelog text",
  "deltas": [
    { "from": "1.1.0", "url": "https://…/1.1.0-to-1.2.0.patch" }
  ]
}
```

- `version` is compared numerically component-wise (`is_newer`).
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

The new package must be signed by the **same key** as the installed one
(`require_signature` is enforced before applying).

## CLI (manual / scripted)

```sh
bpkg fetch-update --url https://…/update.json --dir <install_dir> --current 1.1.0
bpkg update App-1.2.0.bpkg --dir <install_dir>     # apply a local pkg with rollback
```
