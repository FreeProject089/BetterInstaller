//! `bpkg` — BetterInstaller packaging tool.
//!
//! Phase 1 subcommands: pack, info, verify, extract. Signing (`keygen`, `sign`)
//! and `build` (embed into a self-extracting installer exe) arrive in later phases.

use std::path::PathBuf;

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
    Info {
        package: PathBuf,
    },
    /// Verify every file's SHA-256 against the manifest.
    Verify {
        package: PathBuf,
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Pack { root, config, out } => cmd_pack(&root, &config, &out),
        Command::Info { package } => cmd_info(&package),
        Command::Verify { package } => cmd_verify(&package),
        Command::Extract { package, dest, components } => {
            cmd_extract(&package, &dest, components.as_deref())
        }
        Command::Install { package, dest, components } => {
            cmd_install(&package, &dest, components.as_deref())
        }
    }
}

fn cmd_pack(root: &PathBuf, config: &PathBuf, out: &PathBuf) -> Result<()> {
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

fn cmd_info(path: &PathBuf) -> Result<()> {
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
            let tag = if c.required { "required" } else if c.default { "default" } else { "optional" };
            println!("  - {} ({}, {} MB) [{}]", c.id, c.name, c.size_mb, tag);
        }
    }
    Ok(())
}

fn cmd_verify(path: &PathBuf) -> Result<()> {
    let mut pkg = Package::open(path).context("opening package")?;
    let count = pkg.manifest.files.len();
    pkg.verify().context("integrity check failed")?;
    println!("OK — all {count} files verified (SHA-256).");
    Ok(())
}

fn cmd_extract(path: &PathBuf, dest: &PathBuf, components: Option<&[String]>) -> Result<()> {
    let mut pkg = Package::open(path).context("opening package")?;
    let written = pkg.extract(dest, components).context("extracting")?;
    println!("Extracted {written} files → {}", dest.display());
    Ok(())
}

fn cmd_install(path: &PathBuf, dest: &PathBuf, components: Option<&[String]>) -> Result<()> {
    let mut pkg = Package::open(path).context("opening package")?;
    let name = pkg.manifest.app.name.clone();
    let written = pkg
        .install_with_progress(dest, components, |done, total, file| {
            let pct = if total > 0 { done * 100 / total } else { 100 };
            print!("\r  [{pct:3}%] {done}/{total}  {file:<48}");
            let _ = std::io::Write::flush(&mut std::io::stdout());
        })
        .context("install failed")?;
    println!("\nInstalled {name}: {written} files (verified) → {}", dest.display());
    Ok(())
}
