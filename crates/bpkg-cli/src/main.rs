//! `bpkg` — BetterInstaller packaging tool.
//!
//! Phase 1 subcommands: pack, info, verify, extract. Signing (`keygen`, `sign`)
//! and `build` (embed into a self-extracting installer exe) arrive in later phases.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use bpkg_core::config::{self, InstallerConfig};
use bpkg_core::manifest::{AppMeta, Component};
use bpkg_core::package::{self, Package};

#[derive(Parser)]
#[command(name = "bpkg", version, about = "BetterInstaller package tool")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build a .bpkg from a directory of files + an installer.toml.
    Pack {
        /// Directory whose contents become the package payload.
        #[arg(long)]
        root: PathBuf,
        /// Project config (installer.toml).
        #[arg(long)]
        config: PathBuf,
        /// Output .bpkg path.
        #[arg(long)]
        out: PathBuf,
    },
    /// Print a package's metadata.
    Info { package: PathBuf },
    /// Verify every file's SHA-256 against the manifest (and optionally the
    /// Ed25519 signature with --key).
    Verify {
        package: PathBuf,
        /// Public key to also verify the package signature.
        #[arg(long)]
        key: Option<PathBuf>,
    },
    /// Generate an Ed25519 signing keypair (writes private.key + public.key).
    Keygen {
        #[arg(long, default_value = "keys")]
        out: PathBuf,
    },
    /// Sign a .bpkg in place with a private key.
    Sign {
        package: PathBuf,
        #[arg(long)]
        key: PathBuf,
    },
    /// Extract a package into a directory.
    Extract {
        package: PathBuf,
        #[arg(long)]
        dest: PathBuf,
        /// Comma-separated component ids to extract (default: all).
        #[arg(long, value_delimiter = ',')]
        components: Option<Vec<String>>,
        /// Public key to verify the package's Ed25519 signature BEFORE writing anything.
        ///
        /// Without it the package is applied on its own say-so: every file is checked
        /// against the package's own manifest, which proves it is internally consistent
        /// and nothing whatever about who made it.
        #[arg(long)]
        key: Option<PathBuf>,
    },
    /// Install a package (verify + extract) with a progress bar — the same path
    /// the GUI Install step uses.
    Install {
        package: PathBuf,
        #[arg(long)]
        dest: PathBuf,
        #[arg(long, value_delimiter = ',')]
        components: Option<Vec<String>>,
        /// Public key to verify the package's Ed25519 signature BEFORE writing anything.
        ///
        /// Without it the package is applied on its own say-so: every file is checked
        /// against the package's own manifest, which proves it is internally consistent
        /// and nothing whatever about who made it.
        #[arg(long)]
        key: Option<PathBuf>,
    },
    /// Stamp a project's config + package into a copy of the installer exe,
    /// producing a single self-extracting installer.
    Build {
        /// The prebuilt BetterInstaller GUI exe to stamp.
        #[arg(long)]
        installer: PathBuf,
        /// The project's installer.toml.
        #[arg(long)]
        config: PathBuf,
        /// The project's .bpkg.
        #[arg(long)]
        package: PathBuf,
        /// Output exe (e.g. MyAppSetup.exe).
        #[arg(long)]
        out: PathBuf,
    },
    /// Update an existing install with a newer package (rolls back on failure).
    Update {
        package: PathBuf,
        #[arg(long)]
        dir: PathBuf,
        /// Public key to verify the package's Ed25519 signature BEFORE writing anything.
        ///
        /// Without it the package is applied on its own say-so: every file is checked
        /// against the package's own manifest, which proves it is internally consistent
        /// and nothing whatever about who made it.
        #[arg(long)]
        key: Option<PathBuf>,
    },
    /// Create a binary delta patch (old.bpkg → new.bpkg).
    Delta {
        #[arg(long)]
        old: PathBuf,
        #[arg(long)]
        new: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Apply a binary delta patch to reconstruct the new file.
    ApplyDelta {
        #[arg(long)]
        old: PathBuf,
        #[arg(long)]
        patch: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Check a remote update manifest and, if newer, download + apply it.
    /// Print the installer.toml schema as JSON — every key the engine understands.
    ///
    /// Derived from the types, never listed, so it cannot go stale while looking current.
    /// A checker that is not this binary reads it instead of keeping its own copy of the
    /// schema; a second copy in another language is the same bug with a longer fuse.
    Schema {
        /// Write to this file instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    FetchUpdate {
        /// URL of the update manifest JSON.
        #[arg(long)]
        url: String,
        /// Install directory to update.
        #[arg(long)]
        dir: PathBuf,
        /// The currently-installed version.
        #[arg(long)]
        current: String,
        /// Public key to verify the downloaded package's signature before applying it.
        ///
        /// Without it the download is applied on its own say-so: each file is checked
        /// against the package's own manifest, which proves the package is INTERNALLY
        /// CONSISTENT and nothing about who made it. `verify` has taken a key from the
        /// start; this is the same option on the path that fetches over a network.
        #[arg(long)]
        key: Option<PathBuf>,
    },
}

fn cmd_schema(out: Option<&Path>) -> Result<()> {
    let json = config::schema_json().context("serializing the schema")?;
    match out {
        // A trailing newline, because the committed artifact is diffed against this output
        // in CI and a file without one is a file every editor silently changes.
        Some(p) => {
            std::fs::write(
                p,
                format!(
                    "{json}
"
                ),
            )
            .with_context(|| format!("writing {}", p.display()))?;
            println!("wrote {}", p.display());
        }
        None => println!("{json}"),
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Pack { root, config, out } => cmd_pack(&root, &config, &out),
        Command::Info { package } => cmd_info(&package),
        Command::Verify { package, key } => cmd_verify(&package, key.as_deref()),
        Command::Keygen { out } => cmd_keygen(&out),
        Command::Sign { package, key } => cmd_sign(&package, &key),
        Command::Extract {
            package,
            dest,
            components,
            key,
        } => cmd_extract(&package, &dest, components.as_deref(), key.as_deref()),
        Command::Install {
            package,
            dest,
            components,
            key,
        } => cmd_install(&package, &dest, components.as_deref(), key.as_deref()),
        Command::Build {
            installer,
            config,
            package,
            out,
        } => cmd_build(&installer, &config, &package, &out),
        Command::Update { package, dir, key } => cmd_update(&package, &dir, key.as_deref()),
        Command::Delta { old, new, out } => cmd_delta(&old, &new, &out),
        Command::ApplyDelta { old, patch, out } => cmd_apply_delta(&old, &patch, &out),
        Command::Schema { out } => cmd_schema(out.as_deref()),
        Command::FetchUpdate {
            url,
            dir,
            current,
            key,
        } => cmd_fetch_update(&url, &dir, &current, key.as_deref()),
    }
}

fn cmd_delta(old: &Path, new: &Path, out: &Path) -> Result<()> {
    let o = std::fs::read(old).with_context(|| format!("reading {}", old.display()))?;
    let n = std::fs::read(new).with_context(|| format!("reading {}", new.display()))?;
    let patch = bpkg_core::delta::make_delta(&o, &n).context("creating delta")?;
    std::fs::write(out, &patch).with_context(|| format!("writing {}", out.display()))?;
    println!(
        "Delta {} → {}  ({:.1} KB, {:.0}% of new)",
        old.display(),
        out.display(),
        patch.len() as f64 / 1024.0,
        100.0 * patch.len() as f64 / n.len().max(1) as f64
    );
    Ok(())
}

fn cmd_apply_delta(old: &Path, patch: &Path, out: &Path) -> Result<()> {
    let o = std::fs::read(old)?;
    let p = std::fs::read(patch)?;
    let n = bpkg_core::delta::apply_delta(&o, &p).context("applying delta")?;
    std::fs::write(out, &n)?;
    println!("Reconstructed {} ({} bytes).", out.display(), n.len());
    Ok(())
}

fn cmd_fetch_update(url: &str, dir: &Path, current: &str, key: Option<&Path>) -> Result<()> {
    // Loaded BEFORE the download, so a bad key path fails immediately rather than after
    // pulling a package down and staging it.
    let vk = match key {
        Some(k) => Some(bpkg_core::sign::load_public(k).context("loading public key")?),
        None => None,
    };
    match bpkg_core::update::check_remote(url, current).context("checking update")? {
        None => println!("Up to date (current {current})."),
        Some(m) => {
            println!("Update available: {current} → {}. Downloading…", m.version);
            // The installer GUI pins a publisher key of its own; this is the CLI's way
            // to ask for the same guarantee. Said out loud when it is absent, because a
            // silent unverified update over a network is the case worth noticing.
            if vk.is_none() {
                eprintln!(
                    "Warning: applying an unverified update. Each file is checked against the \
                     package's own manifest, which proves it is internally consistent and \
                     nothing about who made it. Pass --key <public.key> to verify the publisher."
                );
            }
            let n =
                bpkg_core::update::download_and_apply(&m, current, None, dir, vk.as_ref(), None)
                    .context("update failed (rolled back)")?;
            println!("Updated to {} ({n} files).", m.version);
        }
    }
    Ok(())
}

fn cmd_update(package: &Path, dir: &Path, key: Option<&Path>) -> Result<()> {
    // Loaded BEFORE anything is touched, so a wrong key path fails here rather than
    // after the install directory has been snapshotted.
    let vk = match key {
        Some(k) => Some(bpkg_core::sign::load_public(k).context("loading public key")?),
        None => {
            eprintln!(
                "Warning: applying an unverified update. Each file is checked against the \
                 package's own manifest, which proves it is internally consistent and \
                 nothing about who made it. Pass --key <public.key> to verify the publisher."
            );
            None
        }
    };
    let n = bpkg_core::update::apply_package_update(package, dir, None, vk.as_ref())
        .context("update failed (rolled back)")?;
    println!("Updated {} ({n} files).", dir.display());
    Ok(())
}

fn cmd_pack(root: &Path, config: &Path, out: &Path) -> Result<()> {
    let cfg = InstallerConfig::load(config).context("reading installer.toml")?;
    let app = AppMeta {
        id: cfg.app.id.clone(),
        name: cfg.app.name.clone(),
        version: cfg.app.version.clone(),
        publisher: cfg.app.publisher.clone(),
        homepage: cfg.app.homepage.clone(),
        platforms: cfg.app.platforms.clone(),
    };
    let components: Vec<Component> = cfg
        .components
        .iter()
        .map(|c| Component {
            id: c.id.clone(),
            name: c.name.clone(),
            description: c.description.clone(),
            required: c.required,
            default: c.default,
            size_mb: c.size_mb,
        })
        .collect();

    // Assign each file to the first component whose `paths` prefix matches; files
    // matching none belong to core (None).
    let sections = cfg.components.clone();
    let component_of = move |rel: &str| -> Option<String> {
        sections
            .iter()
            .find(|c| c.matches(rel))
            .map(|c| c.id.clone())
    };
    let manifest = package::create_from_dir(root, app, components, component_of, out)
        .context("building package")?;

    println!(
        "Packed {} v{} → {}",
        manifest.app.name,
        manifest.app.version,
        out.display()
    );
    println!(
        "  {} files, {:.2} MB uncompressed",
        manifest.files.len(),
        manifest.total_size as f64 / 1_048_576.0
    );
    Ok(())
}

fn cmd_info(path: &Path) -> Result<()> {
    let pkg = Package::open(path).context("opening package")?;
    let m = &pkg.manifest;
    println!("App:        {} v{}", m.app.name, m.app.version);
    println!("Id:         {}", m.app.id);
    println!("Publisher:  {}", m.app.publisher);
    println!("Platforms:  {}", m.app.platforms.join(", "));
    println!("Created:    {}", m.created_at);
    println!(
        "Files:      {} ({:.2} MB uncompressed)",
        m.files.len(),
        m.total_size as f64 / 1_048_576.0
    );
    if !m.components.is_empty() {
        println!("Components:");
        for c in &m.components {
            let tag = if c.required {
                "required"
            } else if c.default {
                "default"
            } else {
                "optional"
            };
            println!("  - {} ({}, {} MB) [{}]", c.id, c.name, c.size_mb, tag);
        }
    }
    Ok(())
}

fn cmd_verify(path: &Path, key: Option<&std::path::Path>) -> Result<()> {
    let mut pkg = Package::open(path).context("opening package")?;
    let count = pkg.manifest.files.len();
    pkg.verify().context("integrity check failed")?;
    println!("OK — all {count} files verified (SHA-256).");

    if let Some(key) = key {
        let vk = bpkg_core::sign::load_public(key).context("loading public key")?;
        if pkg.verify_signature(&vk).context("verifying signature")? {
            println!("OK — Ed25519 signature valid.");
        } else if pkg.is_signed() {
            anyhow::bail!("signature INVALID (does not match this key)");
        } else {
            anyhow::bail!("package is not signed");
        }
    } else if pkg.is_signed() {
        println!("Note: package is signed (pass --key <public.key> to verify it).");
    }
    Ok(())
}

fn cmd_keygen(out: &Path) -> Result<()> {
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let sk = bpkg_core::sign::generate();
    let vk = sk.verifying_key();
    let priv_path = out.join("private.key");
    let pub_path = out.join("public.key");
    bpkg_core::sign::save_private(&sk, &priv_path).context("writing private.key")?;
    bpkg_core::sign::save_public(&vk, &pub_path).context("writing public.key")?;
    println!("Generated keypair:");
    println!(
        "  private: {}  (keep secret — never commit)",
        priv_path.display()
    );
    println!("  public:  {}", pub_path.display());
    Ok(())
}

fn cmd_sign(package: &Path, key: &Path) -> Result<()> {
    let sk = bpkg_core::sign::load_private(key).context("loading private key")?;
    bpkg_core::package::sign_package(package, &sk).context("signing package")?;
    println!("Signed {} (Ed25519).", package.display());
    Ok(())
}

/// Refuse a package that does not carry a valid signature for the key we were given,
/// and say so out loud when we were given none.
///
/// The warning is not decoration. Every file IS checked against the package's own
/// manifest before it is written, which is a real guarantee and an easy one to mistake
/// for the other one: it proves the package is internally consistent and says nothing
/// about who made it. Somebody handed a `.bpkg` and told to run `bpkg install` should
/// be told which of the two they are getting.
fn gate_signature(pkg: &mut Package, key: Option<&Path>, what: &str) -> Result<()> {
    match key {
        Some(k) => {
            let vk = bpkg_core::sign::load_public(k).context("loading public key")?;
            if !pkg.verify_signature(&vk).context("verifying signature")? {
                anyhow::bail!(
                    "{what} refused: the package is {} for this key",
                    if pkg.is_signed() {
                        "signed by somebody else"
                    } else {
                        "not signed"
                    }
                );
            }
            println!("OK — Ed25519 signature valid.");
        }
        None => eprintln!(
            "Warning: {what} without checking who made this package. Each file is verified \
             against the package's own manifest, which proves it is internally consistent and \
             nothing about its author. Pass --key <public.key> to verify the publisher."
        ),
    }
    Ok(())
}

fn cmd_extract(
    path: &Path,
    dest: &Path,
    components: Option<&[String]>,
    key: Option<&Path>,
) -> Result<()> {
    let mut pkg = Package::open(path).context("opening package")?;
    gate_signature(&mut pkg, key, "extracting")?;
    let written = pkg.extract(dest, components).context("extracting")?;
    println!("Extracted {written} files → {}", dest.display());
    Ok(())
}

fn cmd_build(installer: &Path, config: &Path, package: &Path, out: &Path) -> Result<()> {
    // Validate the config parses before stamping (fail early on a bad TOML).
    let cfg_bytes =
        std::fs::read(config).with_context(|| format!("reading {}", config.display()))?;
    InstallerConfig::from_toml(&String::from_utf8_lossy(&cfg_bytes))
        .context("invalid installer.toml")?;
    let bpkg = std::fs::read(package).with_context(|| format!("reading {}", package.display()))?;

    bpkg_core::embed::stamp(installer, &cfg_bytes, &bpkg, out).context("stamping installer")?;

    let size = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
    println!(
        "Built self-extracting installer → {} ({:.2} MB)",
        out.display(),
        size as f64 / 1_048_576.0
    );
    Ok(())
}

fn cmd_install(
    path: &Path,
    dest: &Path,
    components: Option<&[String]>,
    key: Option<&Path>,
) -> Result<()> {
    let mut pkg = Package::open(path).context("opening package")?;
    gate_signature(&mut pkg, key, "installing")?;
    let name = pkg.manifest.app.name.clone();
    let written = pkg
        .install_with_progress(dest, components, |done, total, file| {
            let pct = (done * 100).checked_div(total).unwrap_or(100);
            print!("\r  [{pct:3}%] {done}/{total}  {file:<48}");
            let _ = std::io::Write::flush(&mut std::io::stdout());
        })
        .context("install failed")?;
    println!(
        "\nInstalled {name}: {written} files (verified) → {}",
        dest.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::gate_signature;
    use bpkg_core::manifest::AppMeta;
    use bpkg_core::package::{self, Package};

    fn app() -> AppMeta {
        AppMeta {
            id: "t".into(),
            name: "T".into(),
            version: "1".into(),
            publisher: "p".into(),
            homepage: None,
            platforms: vec!["windows".into()],
        }
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bpkg-cli-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A scratch directory and an unsigned package in it. The signing key is NOT returned:
    /// `SigningKey` belongs to ed25519-dalek and this crate does not depend on it, so the
    /// type cannot be named in a signature. Each test generates its own and lets inference
    /// hold it.
    fn packed(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let base = scratch(tag);
        let src = base.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("app.exe"), b"payload").unwrap();
        let bpkg = base.join("p.bpkg");
        package::create_from_dir(&src, app(), vec![], |_| None, &bpkg).unwrap();
        (base, bpkg)
    }

    /// The point of the whole option: a package signed by SOMEBODY ELSE is refused, and
    /// refused before `extract`/`install` is reached rather than after some files land.
    #[test]
    fn refuses_a_package_signed_by_another_key() {
        let (base, bpkg) = packed("wrongkey");
        let publisher = bpkg_core::sign::generate();
        let attacker = bpkg_core::sign::generate();
        package::sign_package(&bpkg, &attacker).unwrap();

        let keyfile = base.join("public.key");
        bpkg_core::sign::save_public(&publisher.verifying_key(), &keyfile).unwrap();

        let mut pkg = Package::open(&bpkg).unwrap();
        let err = gate_signature(&mut pkg, Some(&keyfile), "installing")
            .expect_err("a package signed by an untrusted key must be refused");
        assert!(
            err.to_string().contains("signed by somebody else"),
            "unexpected: {err}"
        );
    }

    /// An UNSIGNED package must be refused too, and must say which of the two it is. The
    /// two failures want different responses — one is a package from the wrong author,
    /// the other is a publisher who never signed — and one message for both is how
    /// "it is unsigned" gets read as "your key is wrong".
    #[test]
    fn refuses_an_unsigned_package_and_says_so() {
        let (base, bpkg) = packed("unsigned");
        let publisher = bpkg_core::sign::generate();
        let keyfile = base.join("public.key");
        bpkg_core::sign::save_public(&publisher.verifying_key(), &keyfile).unwrap();

        let mut pkg = Package::open(&bpkg).unwrap();
        let err = gate_signature(&mut pkg, Some(&keyfile), "extracting")
            .expect_err("an unsigned package must be refused when a key is pinned");
        assert!(err.to_string().contains("not signed"), "unexpected: {err}");
    }

    /// The right key passes. Without this the two tests above would also pass on a gate
    /// that refused everything, which is a gate nobody can use.
    #[test]
    fn accepts_the_key_that_signed_it() {
        let (base, bpkg) = packed("right");
        let publisher = bpkg_core::sign::generate();
        package::sign_package(&bpkg, &publisher).unwrap();
        let keyfile = base.join("public.key");
        bpkg_core::sign::save_public(&publisher.verifying_key(), &keyfile).unwrap();

        let mut pkg = Package::open(&bpkg).unwrap();
        gate_signature(&mut pkg, Some(&keyfile), "installing")
            .expect("the publisher's own key must pass");
    }

    /// No key is a WARNING, not a refusal. That is the documented behaviour and the one
    /// worth pinning: the same call that used to be silent must still succeed, or every
    /// existing `bpkg install` invocation breaks.
    #[test]
    fn no_key_still_installs() {
        let (_base, bpkg) = packed("nokey");
        let mut pkg = Package::open(&bpkg).unwrap();
        gate_signature(&mut pkg, None, "installing").expect("no key must not be a refusal");
    }
}
