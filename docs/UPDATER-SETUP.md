# Setting up the updater

The installer/updater pulls a small JSON **update manifest** from a URL you control,
compares its `version` to what's installed, and (if newer) downloads + applies the
new signed `.bpkg` with rollback. Two common hosting setups below.

```toml
# installer.toml
[update]
manifest_url = "https://…/update.json"   # the only thing that must be stable
auto_check   = true
allow_delta  = true
```

The manifest:
```json
{
  "version": "1.2.0",
  "url": "https://…/App-1.2.0.bpkg",
  "notes": "What's new in 1.2.0",
  "deltas": [ { "from": "1.1.0", "url": "https://…/1.1.0-to-1.2.0.patch" } ]
}
```

> The new `.bpkg` **must be signed by the same key** as the installed build
> (`require_signature` is enforced before applying). See [SIGNING.md](SIGNING.md).

---

## Option A — GitHub Releases (free, recommended)

Use a **stable "latest" download URL** so `manifest_url` never changes:

```
https://github.com/<owner>/<repo>/releases/latest/download/update.json
```

GitHub always redirects `…/releases/latest/download/<asset>` to the asset of the most
recent *non-prerelease* release. So:

1. Build + sign the new package:
   ```sh
   bpkg pack --root payload --config installer.toml --out App-1.2.0.bpkg
   bpkg sign --key keys/private.key App-1.2.0.bpkg
   ```
2. (Optional) a delta from the previous release:
   ```sh
   bpkg delta App-1.1.0.bpkg App-1.2.0.bpkg 1.1.0-to-1.2.0.patch
   ```
3. Write `update.json` pointing at the **release-asset** URLs of *this* tag:
   ```json
   {
     "version": "1.2.0",
     "url": "https://github.com/<owner>/<repo>/releases/download/v1.2.0/App-1.2.0.bpkg",
     "deltas": [
       { "from": "1.1.0",
         "url": "https://github.com/<owner>/<repo>/releases/download/v1.2.0/1.1.0-to-1.2.0.patch" }
     ]
   }
   ```
4. Create the GitHub Release `v1.2.0` and upload `update.json`, `App-1.2.0.bpkg`, and
   the patch as **assets**.

`manifest_url` stays `…/releases/latest/download/update.json` forever — each release
just publishes a fresh `update.json`.

> Automate it: a CI job (or your build script) can generate `update.json` from the
> built package's version and the asset URLs, then `gh release create … update.json
> App-*.bpkg`.

### Worked example (`examples/`)

The bundled example's build script **emits `update.json` automatically** (it reads
`[app].version` and derives the `.bpkg` URL from `[update].manifest_url`). So a release
is three uploads:

```
gh release create v1.0.0 \
  <App>-Setup.exe \
  app.bpkg \
  update.json
```

Its `[update]` block is already set (`manifest_url = …/releases/latest/download/update.json`,
`auto_check = true`, `allow_delta = true`), so an installed copy shows **Update** in
maintenance mode the moment a newer release is published. Bump `[app].version`,
rebuild, upload — done.

---

## Option B — Your own server / VPS / object storage

Host the files anywhere that serves plain HTTP(S) (nginx, S3/R2/B2, a static host):

```
https://downloads.example.com/myapp/update.json
https://downloads.example.com/myapp/App-1.2.0.bpkg
https://downloads.example.com/myapp/1.1.0-to-1.2.0.patch
```

1. `manifest_url = "https://downloads.example.com/myapp/update.json"`.
2. On each release, upload the signed `.bpkg` (+ optional patch) and overwrite
   `update.json` with the new `version` + URLs.
3. Serve with correct content types and **CORS not required** (the updater fetches
   server-side via the Rust HTTP client, not a browser).

Minimal nginx:
```nginx
location /myapp/ {
    root /var/www;
    autoindex off;
    add_header Cache-Control "no-cache" always;   # so update.json is re-fetched
}
```

> Keep `update.json` uncached (or short TTL) so clients see new releases promptly;
> the `.bpkg`/patch files are immutable and can be cached aggressively.

---

## Testing an update locally

```sh
# serve a folder with update.json + the .bpkg on localhost
python -m http.server 8000        # in the folder
bpkg fetch-update --url http://localhost:8000/update.json --dir <install_dir> --current 1.1.0
```

Or set `manifest_url` to the localhost URL, install an older build, then re-open the
installer (maintenance mode) — the **Update** button appears when the manifest is
newer.
