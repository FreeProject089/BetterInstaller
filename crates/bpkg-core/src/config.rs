//! `installer.toml` — the per-project configuration that turns the shared engine
//! into a specific app's installer. One TOML → one installer.
//!
//! Only the sections needed for Phase 1 + the first-run handoff (v3 Addendum §C)
//! are modelled here; more sections (prerequisites, update, uninstall…) are added
//! in later phases.

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Parsed `installer.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallerConfig {
    pub app: AppSection,
    #[serde(default)]
    pub branding: Branding,
    #[serde(default)]
    pub install: InstallSection,
    #[serde(default, rename = "components")]
    pub components: Vec<ComponentSection>,
    /// First-run configuration handoff (the headline feature). Optional.
    #[serde(default)]
    pub handoff: Option<Handoff>,
    /// Options presented in the installer's "Configuration" page.
    #[serde(default, rename = "setup_option")]
    pub setup_options: Vec<SetupOption>,
}

impl InstallerConfig {
    /// Parse an `installer.toml` from a string.
    pub fn from_toml(s: &str) -> Result<Self> {
        Ok(toml::from_str(s)?)
    }

    /// Load and parse an `installer.toml` from disk.
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let p = path.as_ref();
        let s = std::fs::read_to_string(p).map_err(|e| crate::error::Error::io(p, e))?;
        Self::from_toml(&s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSection {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default = "default_platforms")]
    pub platforms: Vec<String>,
}

fn default_platforms() -> Vec<String> {
    vec!["windows".to_string()]
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Branding {
    #[serde(default)]
    pub accent: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallSection {
    /// e.g. "{ProgramFiles}/Better Mods Manager" — placeholders resolved at runtime.
    #[serde(default)]
    pub default_dir: Option<String>,
    /// Main executable, relative to the install root (for shortcuts + protocol).
    #[serde(default)]
    pub main_exe: Option<String>,
    /// Custom URL scheme to register, e.g. "bmm" (→ bmm:// deep links).
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default = "default_true")]
    pub create_shortcuts: bool,
    /// Also drop a desktop shortcut (Start Menu is implied by create_shortcuts).
    #[serde(default)]
    pub desktop_shortcut: bool,
    #[serde(default = "default_true")]
    pub allow_portable: bool,
}

impl Default for InstallSection {
    fn default() -> Self {
        InstallSection {
            default_dir: None,
            main_exe: None,
            protocol: None,
            create_shortcuts: true,
            desktop_shortcut: false,
            allow_portable: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentSection {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default = "default_true")]
    pub default: bool,
    #[serde(default)]
    pub size_mb: u32,
}

// ── First-run handoff (v3 Addendum §C) ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handoff {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// File name written for the app to read once on first launch.
    #[serde(default = "default_handoff_file")]
    pub file: String,
    /// Where to write it: "app_data" (per-user) or "install_dir" (portable).
    #[serde(default = "default_handoff_location")]
    pub location: HandoffLocation,
}

impl Default for Handoff {
    fn default() -> Self {
        Handoff {
            enabled: true,
            file: default_handoff_file(),
            location: default_handoff_location(),
        }
    }
}

fn default_handoff_file() -> String {
    "installer-handoff.json".to_string()
}
fn default_handoff_location() -> HandoffLocation {
    HandoffLocation::AppData
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffLocation {
    AppData,
    InstallDir,
}

/// One option presented in the installer's Configuration page, declared by the
/// project. Generic: the engine never hard-codes any app's settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupOption {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: SetupOptionKind,
    /// i18n key for the label.
    pub label_key: String,
    /// For `select`: allowed choices. "auto" may be used as a sentinel (e.g. OS language).
    #[serde(default)]
    pub choices: Vec<String>,
    /// Default value as a JSON value (bool, string, …).
    #[serde(default)]
    pub default: serde_json::Value,
    /// For `license`: documents to display (relative paths in the package).
    #[serde(default)]
    pub documents: Vec<String>,
    /// Whether the user must satisfy this option to proceed.
    #[serde(default)]
    pub required: bool,
    /// Which key(s) in handoff.settings this option writes to. A `license` option
    /// may map to several keys (e.g. privacy_accepted + tos_accepted).
    #[serde(default)]
    pub maps_to: MapsTo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupOptionKind {
    /// A toggle → bool.
    Bool,
    /// A dropdown → string.
    Select,
    /// A scrollable legal panel + "I accept" checkbox.
    License,
}

/// `maps_to` accepts either a single key or a list of keys.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MapsTo {
    #[default]
    None,
    One(String),
    Many(Vec<String>),
}

impl MapsTo {
    pub fn keys(&self) -> Vec<&str> {
        match self {
            MapsTo::None => vec![],
            MapsTo::One(k) => vec![k.as_str()],
            MapsTo::Many(v) => v.iter().map(String::as_str).collect(),
        }
    }
}

fn default_true() -> bool {
    true
}
