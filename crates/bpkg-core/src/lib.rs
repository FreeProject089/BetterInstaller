//! # bpkg-core
//!
//! The reusable engine behind **BetterInstaller** — a proprietary, cross-platform
//! installer/updater framework. One engine + one `installer.toml` per project.
//!
//! Phase 1 surface:
//! - [`package`] — the `.bpkg` container (build / verify / extract).
//! - [`manifest`] — the package manifest (app + files + components + hashes).
//! - [`config`] — `installer.toml` parsing, including the first-run handoff config.
//! - [`handoff`] — the app-agnostic `installer-handoff.json` first-run contract.
//! - [`platform`] — the `PlatformOps` abstraction (Windows/Linux/macOS).
//!
//! See `.Assets/.md/PLAN_BETTER_INSTALLER.md` (v2 + v3 Addendum) for the full plan.

pub mod config;
pub mod delta;
pub mod embed;
pub mod error;
pub mod handoff;
pub mod i18n;
pub mod manifest;
pub mod net;
pub mod package;
pub mod platform;
pub mod prereq;
pub mod sign;
pub mod update;

pub use error::{Error, Result};

/// Engine version (matches the crate version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
