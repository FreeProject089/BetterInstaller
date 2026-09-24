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
    /// Write a SIGNED update.json for a signed .bpkg (card C-1).
    ///
    /// The signed part names the app, the version, the URLs, the package's SHA-256, the
    /// SHA-256 of the installer.toml stamped into the setup, and an expiry at most 7 days
    /// away: renew it with `resign-manifest` at least that often, or clients stop
    /// believing it.
    UpdateManifest {
        /// The signed .bpkg this manifest offers (app id, version and hash are read from it).
        #[arg(long)]
        package: PathBuf,
        /// The installer.toml `bpkg build` stamps into the setup (its hash is recorded).
        #[arg(long)]
        config: PathBuf,
        /// The publisher's private.key (the key that signed the package).
        #[arg(long)]
        key: PathBuf,
        /// Where clients download the .bpkg.
        #[arg(long)]
        url: String,
        /// A mirror of the same .bpkg (repeatable).
        #[arg(long = "mirror")]
        mirrors: Vec<String>,
        /// A delta patch, as `<from-version>=<url>` (repeatable).
        #[arg(long = "delta")]
        deltas: Vec<String>,
        /// Release notes.
        #[arg(long)]
        notes: Option<String>,
        /// Days until the manifest expires (1 to 7).
        #[arg(long, default_value_t = 7)]
        valid_days: i64,
        /// Output update.json.
        #[arg(long)]
        out: PathBuf,
    },
    /// Renew the expiry of a signed update.json (run it at least weekly between releases).
    ResignManifest {
        /// The signed update.json to renew (rewritten in place unless --out is given).
        manifest: PathBuf,
        /// The publisher's private.key; the manifest's current signature must be by it.
        #[arg(long)]
        key: PathBuf,
        #[arg(long, default_value_t = 7)]
        valid_days: i64,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Check a signed update.json: signature, expiry, and optionally that a setup carries
    /// exactly the installer.toml and .bpkg the manifest names.
    VerifyManifest {
        manifest: PathBuf,
        /// The publisher's public.key.
        #[arg(long)]
        key: PathBuf,
        /// The app id the manifest must name.
        #[arg(long)]
        app_id: Option<String>,
        /// A stamped setup to check against the manifest's config and package hashes.
        #[arg(long)]
        setup: Option<PathBuf>,
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
        Command::UpdateManifest {
            package,
            config,
            key,
            url,
            mirrors,
            deltas,
            notes,
            valid_days,
            out,
        } => cmd_update_manifest(ManifestArgs {
            package: &package,
            config: &config,
            key: &key,
            url,
            mirrors,
            deltas: &deltas,
            notes,
            valid_days,
            out: &out,
        }),
        Command::ResignManifest {
            manifest,
            key,
            valid_days,
            out,
        } => cmd_resign_manifest(&manifest, &key, valid_days, out.as_deref()),
        Command::VerifyManifest {
            manifest,
            key,
            app_id,
            setup,
        } => cmd_verify_manifest(&manifest, &key, app_id.as_deref(), setup.as_deref()),
    }
}

struct ManifestArgs<'a> {
    package: &'a Path,
    config: &'a Path,
    key: &'a Path,
    url: String,
    mirrors: Vec<String>,
    deltas: &'a [String],
    notes: Option<String>,
    valid_days: i64,
    out: &'a Path,
}

/// The signed update.json for `a.package`, as text. Split from the command so a test can
/// read what it produces.
fn build_update_manifest(a: &ManifestArgs) -> Result<String> {
    use bpkg_core::update::{sha256_hex, sign_manifest, DeltaEntry, UpdateManifest};
    let sk = bpkg_core::sign::load_private(a.key).context("loading private key")?;
    let bpkg =
        std::fs::read(a.package).with_context(|| format!("reading {}", a.package.display()))?;
    let mut pkg = Package::open(a.package).context("opening package")?;
    // The manifest vouches for this package, so it must be one this key signed.
    if !pkg
        .verify_signature(&sk.verifying_key())
        .context("verifying the package signature")?
    {
        anyhow::bail!(
            "{} is not signed by this key: sign it first (`bpkg sign`)",
            a.package.display()
        );
    }
    let app = pkg.manifest.app.clone();
    let cfg_bytes =
        std::fs::read(a.config).with_context(|| format!("reading {}", a.config.display()))?;
    let cfg = InstallerConfig::from_toml(&String::from_utf8_lossy(&cfg_bytes))
        .context("invalid installer.toml")?;
    if cfg.app.id != app.id || !bpkg_core::version::same_release(&cfg.app.version, &app.version) {
        anyhow::bail!(
            "installer.toml is {} {} but the package is {} {}",
            cfg.app.id,
            cfg.app.version,
            app.id,
            app.version
        );
    }
    let deltas = a
        .deltas
        .iter()
        .map(|d| {
            d.split_once('=')
                .map(|(from, url)| DeltaEntry {
                    from: from.trim().to_string(),
                    url: url.trim().to_string(),
                    urls: vec![],
                })
                .ok_or_else(|| anyhow::anyhow!("--delta {d:?}: expected <from-version>=<url>"))
        })
        .collect::<Result<Vec<_>>>()?;
    let body = UpdateManifest {
        app_id: Some(app.id.clone()),
        version: app.version.clone(),
        url: a.url.clone(),
        urls: a.mirrors.clone(),
        notes: a.notes.clone(),
        deltas,
        sha256: Some(sha256_hex(&bpkg)),
        config_sha256: Some(sha256_hex(&cfg_bytes)),
        ..Default::default()
    };
    sign_manifest(&body, &sk, chrono::Utc::now(), a.valid_days).context("signing manifest")
}

fn cmd_update_manifest(a: ManifestArgs) -> Result<()> {
    let text = build_update_manifest(&a)?;
    std::fs::write(a.out, format!("{text}\n"))
        .with_context(|| format!("writing {}", a.out.display()))?;
    println!(
        "Signed update manifest → {} (valid {} days; renew with `bpkg resign-manifest`)",
        a.out.display(),
        a.valid_days
    );
    Ok(())
}

fn cmd_resign_manifest(
    manifest: &Path,
    key: &Path,
    valid_days: i64,
    out: Option<&Path>,
) -> Result<()> {
    let sk = bpkg_core::sign::load_private(key).context("loading private key")?;
    let text = std::fs::read_to_string(manifest)
        .with_context(|| format!("reading {}", manifest.display()))?;
    let renewed = bpkg_core::update::resign_manifest(&text, &sk, chrono::Utc::now(), valid_days)
        .context("re-signing manifest")?;
    let out = out.unwrap_or(manifest);
    std::fs::write(out, format!("{renewed}\n"))
        .with_context(|| format!("writing {}", out.display()))?;
    println!("Re-signed {} (valid {valid_days} days).", out.display());
    Ok(())
}

/// Signature and expiry of `manifest`, and — with `setup` — that the setup carries exactly
/// the installer.toml and package the manifest names. The config hash is the part of the
/// setup no Ed25519 signature covers (`installer.toml` rides outside the package).
fn check_manifest(
    manifest: &Path,
    key: &Path,
    app_id: Option<&str>,
    setup: Option<&Path>,
) -> Result<bpkg_core::update::UpdateManifest> {
    use bpkg_core::update::{parse_manifest, sha256_hex, ManifestPolicy};
    let vk = bpkg_core::sign::load_public(key).context("loading public key")?;
    let text = std::fs::read_to_string(manifest)
        .with_context(|| format!("reading {}", manifest.display()))?;
    let m = parse_manifest(&text, &ManifestPolicy::new(Some(&vk), app_id))?;
    if let Some(setup) = setup {
        let emb = bpkg_core::embed::read_embedded(setup)
            .with_context(|| format!("reading {}", setup.display()))?
            .ok_or_else(|| anyhow::anyhow!("{} is not a stamped setup", setup.display()))?;
        let cfg = sha256_hex(&emb.config);
        if m.config_sha256.as_deref() != Some(cfg.as_str()) {
            anyhow::bail!(
                "the setup's installer.toml is not the one this manifest names (SHA-256 {cfg}, \
                 expected {})",
                m.config_sha256.as_deref().unwrap_or("none")
            );
        }
        let pkg = sha256_hex(&emb.bpkg);
        if m.sha256.as_deref() != Some(pkg.as_str()) {
            anyhow::bail!(
                "the setup's package is not the one this manifest names (SHA-256 {pkg}, \
                 expected {})",
                m.sha256.as_deref().unwrap_or("none")
            );
        }
    }
    Ok(m)
}

fn cmd_verify_manifest(
    manifest: &Path,
    key: &Path,
    app_id: Option<&str>,
    setup: Option<&Path>,
) -> Result<()> {
    let m = check_manifest(manifest, key, app_id, setup)?;
    println!(
        "OK — signed manifest for {} {} (expires {}).",
        m.app_id.as_deref().unwrap_or("?"),
        m.version,
        m.expires.as_deref().unwrap_or("?")
    );
    if setup.is_some() {
        println!("OK — the setup carries exactly this release's installer.toml and package.");
    }
    Ok(())
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
    // With a key, the manifest itself must be signed by it, unexpired, and newer than
    // `current` (card C-1). No migration grace here: the CLI has no install record to say
    // the install predates signed manifests.
    let policy = bpkg_core::update::ManifestPolicy::new(vk.as_ref(), None);
    match bpkg_core::update::check_remote(url, current, &policy).context("checking update")? {
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

    /// Card C-1 and the `installer.toml` row: the publisher tool writes a manifest that is
    /// signed, and that pins the exact config a setup of this release carries. A setup
    /// re-stamped with another config (same package, same key) no longer matches it.
    #[test]
    fn the_published_manifest_is_signed_and_pins_the_setups_config() {
        use super::{build_update_manifest, check_manifest, ManifestArgs};
        let (base, bpkg) = packed("manifest");
        let sk = bpkg_core::sign::generate();
        let (privf, pubf) = (base.join("private.key"), base.join("public.key"));
        bpkg_core::sign::save_private(&sk, &privf).unwrap();
        bpkg_core::sign::save_public(&sk.verifying_key(), &pubf).unwrap();
        let cfg = base.join("installer.toml");
        let cfg_text = "[app]\nid = \"t\"\nname = \"T\"\nversion = \"1\"\npublisher = \"p\"\n";
        std::fs::write(&cfg, cfg_text).unwrap();
        let out = base.join("update.json");
        let args = ManifestArgs {
            package: &bpkg,
            config: &cfg,
            key: &privf,
            url: "https://example.invalid/t.bpkg".into(),
            mirrors: vec![],
            deltas: &[],
            notes: None,
            valid_days: 7,
            out: &out,
        };

        // Only a package this key signed gets a manifest.
        assert!(build_update_manifest(&args).is_err());
        package::sign_package(&bpkg, &sk).unwrap();
        std::fs::write(&out, build_update_manifest(&args).unwrap()).unwrap();
        let m = check_manifest(&out, &pubf, Some("t"), None).unwrap();
        assert_eq!(m.trust, bpkg_core::update::ManifestTrust::Verified);

        let engine = base.join("engine.exe");
        std::fs::write(&engine, b"not really an engine").unwrap();
        let pkg_bytes = std::fs::read(&bpkg).unwrap();
        let setup = base.join("setup.exe");
        bpkg_core::embed::stamp(&engine, cfg_text.as_bytes(), &pkg_bytes, &setup).unwrap();
        check_manifest(&out, &pubf, Some("t"), Some(&setup))
            .expect("the setup built from this release's config and package");

        let restamped = base.join("restamped.exe");
        let other = format!("{cfg_text}[install]\nmain_exe = \"other.exe\"\n");
        bpkg_core::embed::stamp(&engine, other.as_bytes(), &pkg_bytes, &restamped).unwrap();
        let err = check_manifest(&out, &pubf, Some("t"), Some(&restamped)).unwrap_err();
        assert!(err.to_string().contains("installer.toml"), "{err}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
