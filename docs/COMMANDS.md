# Every command, and when you want it

Building, checking and shipping BetterInstaller itself. For what `bpkg` does once it is
built, see [Command line](CLI.md) — this page is about getting there.

Paths are relative to the BetterInstaller root.

## Building

```bash
cargo build --release -p bpkg-cli -p installer
```

Two binaries come out of `target/release/`:

| | |
|---|---|
| `bpkg` | the packaging tool — pack, sign, verify, build |
| `betterinstaller` | the SFX stub that gets stamped with a package |

Faster while working:

```bash
cargo check --workspace        # does it compile
cargo build -p bpkg-cli        # debug build of one crate
```

## The gate

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Or all of it in a container, which is what CI runs:

```bash
docker compose run --rm ci
docker compose run --rm shell    # interactive, with the toolchain and deps
```

Named volumes cache the Cargo registry and the Linux target directory, so repeat runs are
fast and Linux artifacts never touch your host `./target`.

## Making an installer

The full pipeline, by hand:

```bash
./target/release/bpkg keygen --out keys
./target/release/bpkg pack  --root payload --config installer.toml --out app.bpkg
./target/release/bpkg sign  --key keys/private.key app.bpkg
./target/release/bpkg build --installer ./target/release/betterinstaller.exe \
                            --config installer.toml --package app.bpkg --out App-Setup.exe
```

!!! danger "`keys/private.key` is secret"
    It signs packages. Anyone holding it can produce an installer your users' copies will
    accept as genuine. Only `public.key` goes into `installer.toml`.

### For BMM, scripted

```powershell
./examples/bmm/build-installer.ps1              # payload → .bpkg → sign → stamp the SFX
./examples/bmm/build-installer.ps1 -Logo my.png # with a custom sidebar logo
./examples/bmm/release.ps1                      # the whole release
```

`release.ps1` bumps versions, builds BMM, packs/signs/stamps, makes a delta against the
previous release, writes a multi-source `update.json`, and can publish the GitHub release
with every asset attached.

From the BMM repository the same thing is `npm run build:installer`, and `npm run release`
does the build first.

## Documentation

```bash
pip install -r requirements.txt
mkdocs serve                        # http://127.0.0.1:8000
mkdocs build --strict               # what CI checks: a broken internal link fails
mkdocs build -f mkdocs.pdf.yml      # the PDF — needs GTK, easiest in CI
python tools/check_pdf.py           # sanity-check the generated PDF
```

`mkdocs.yml` deliberately does not include the PDF plugin: mkdocs imports a plugin even when
it is disabled, so listing it there made `mkdocs serve` fail on a machine without the GTK
libraries. The PDF config inherits from it instead.

## The workspace

| Crate | What it is |
|---|---|
| `bpkg-core` | the format — reading, writing, verifying a `.bpkg` |
| `bpkg-cli` | the `bpkg` command |
| `installer` | the SFX stub, the window a user actually sees |

Target one with `-p`:

```bash
cargo test -p bpkg-core
cargo clippy -p installer -- -D warnings
```
