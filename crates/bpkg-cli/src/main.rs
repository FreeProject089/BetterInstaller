//! `bpkg` — BetterInstaller packaging tool.
//!
//! Phase 1 subcommands: pack, info, verify, extract. Signing (`keygen`, `sign`)
//! and `build` (embed into a self-extracting installer exe) arrive in later phases.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use bpkg_core::config::InstallerConfig;
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
    },
    /// Install a package (verify + extract) with a progress bar — the same path
    /// the GUI Install step uses.
    Install {
        package: PathBuf,
        #[arg(long)]
        dest: PathBuf,
        #[arg(long, value_delimiter = ',')]
        components: Option<Vec<String>>,
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
    },
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
        } => cmd_extract(&package, &dest, components.as_deref()),
        Command::Install {
            package,
            dest,
            components,
        } => cmd_install(&package, &dest, components.as_deref()),
        Command::Build {
            installer,
            config,
            package,
            out,
        } => cmd_build(&installer, &config, &package, &out),
        Command::Update { package, dir } => cmd_update(&package, &dir),
    }
}

fn cmd_update(package: &Path, dir: &Path) -> Result<()> {
    let n = bpkg_core::update::apply_package_update(package, dir, None)
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

    // Phase 1: all files belong to core (None). Component-aware splitting comes later.
    let manifest = package::create_from_dir(root, app, components, |_| None, out)
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

fn cmd_extract(path: &Path, dest: &Path, components: Option<&[String]>) -> Result<()> {
    let mut pkg = Package::open(path).context("opening package")?;
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

fn cmd_install(path: &Path, dest: &Path, components: Option<&[String]>) -> Result<()> {
    let mut pkg = Package::open(path).context("opening package")?;
    let name = pkg.manifest.app.name.clone();
    let written = pkg
        .install_with_progress(dest, components, |done, total, file| {
            let pct = if total > 0 { done * 100 / total } else { 100 };
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
