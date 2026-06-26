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
    /// Package-signature trust settings. Optional.
    #[serde(default)]
    pub security: Option<Security>,
    /// System prerequisites to check (and optionally auto-install).
    #[serde(default, rename = "prerequisite")]
    pub prerequisites: Vec<Prerequisite>,
    /// Things the user can optionally launch from the final "Done" page.
    #[serde(default, rename = "launch")]
    pub launch: Vec<LaunchItem>,
    /// Auto-update settings (remote manifest check + apply). Optional.
    #[serde(default)]
    pub update: Option<UpdateConfig>,
    /// Installer color theme — override any palette colour from the TOML. Optional.
    #[serde(default)]
    pub theme: ThemeConfig,
}

/// Hex colours (e.g. "#0d1117") overriding the installer's built-in palette. Any
/// field left out keeps the default. Lets a project re-skin the installer entirely.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThemeConfig {
    pub bg: Option<String>,
    pub panel: Option<String>,
    pub panel2: Option<String>,
    pub border: Option<String>,
    pub accent: Option<String>,
    pub accent_dark: Option<String>,
    pub accent_hover: Option<String>,
    pub text: Option<String>,
    pub dim: Option<String>,
    pub danger: Option<String>,
    pub shadow: Option<String>,
}

/// Configures the updater: where to look for newer versions and whether to check
/// automatically. The actual download/apply + rollback lives in `update.rs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateConfig {
    /// Primary URL of a JSON update manifest: `{ "version", "url", "deltas": [...] }`.
    /// Optional only if `manifest_urls` is set; normally this is your main source.
    #[serde(default)]
    pub manifest_url: String,
    /// OPTIONAL additional manifest sources (mirrors / a second host). When set, the
    /// updater fetches every source and uses the **newest** version found; dead sources
    /// are skipped. Leave empty (`[]`) for a single-source setup — multi is opt-in.
    #[serde(default)]
    pub manifest_urls: Vec<String>,
    /// Check the manifest automatically when the maintenance window opens.
    #[serde(default = "default_true")]
    pub auto_check: bool,
    /// Prefer a small binary delta patch over a full re-download when offered.
    #[serde(default = "default_true")]
    pub allow_delta: bool,
}

impl UpdateConfig {
    /// All configured manifest sources (primary first, then extras), trimmed and
    /// non-empty. A single `manifest_url` yields a 1-element list (back-compat).
    pub fn sources(&self) -> Vec<String> {
        std::iter::once(self.manifest_url.trim())
            .chain(self.manifest_urls.iter().map(|s| s.trim()))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }
}

/// A program offered as an opt-in "launch now" checkbox on the Done page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchItem {
    pub id: String,
    /// Checkbox label, e.g. "Launch Better Mods Manager".
    pub label: String,
    /// Executable to run, relative to the install root.
    pub exe: String,
    /// Pre-checked on the Done page. The main app is typically `true`; optional
    /// sidecars `false`.
    #[serde(default)]
    pub default: bool,
    /// Only offer this launch if the named component was installed (omit = always).
    #[serde(default)]
    pub component: Option<String>,
}

/// A prerequisite the target machine must satisfy (e.g. a runtime). Detection is
/// by registry key, file path, or a command on PATH — whichever is set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prerequisite {
    pub id: String,
    pub name: String,
    /// `HKLM\...` or `HKCU\...` key that exists when the prereq is installed.
    #[serde(default)]
    pub check_registry: Option<String>,
    /// A file that exists when the prereq is installed.
    #[serde(default)]
    pub check_file: Option<String>,
    /// A command that resolves on PATH when the prereq is installed.
    #[serde(default)]
    pub check_command: Option<String>,
    /// Where to download the installer if missing (auto-install support).
    #[serde(default)]
    pub download_url: Option<String>,
    /// Silent-install arguments for the downloaded installer.
    #[serde(default)]
    pub silent_args: Option<String>,
    #[serde(default = "default_true")]
    pub required: bool,
}

/// Trust anchor for verifying the `.bpkg` signature before installing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Security {
    /// Hex-encoded Ed25519 public key the package must be signed with.
    #[serde(default)]
    pub public_key: Option<String>,
    /// Refuse to install if the signature is missing or invalid.
    #[serde(default)]
    pub require_signature: bool,
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
    /// Payload path prefixes that belong to this component (forward slashes),
    /// e.g. ["bin/mcp-server.exe", "mcp/"]. Files matching none → core.
    #[serde(default)]
    pub paths: Vec<String>,
}

impl ComponentSection {
    /// Whether `rel` (a forward-slash payload path) belongs to this component.
    pub fn matches(&self, rel: &str) -> bool {
        self.paths.iter().any(|p| {
            let p = p.trim_end_matches("**").trim_end_matches('*');
            !p.is_empty() && (rel == p || rel.starts_with(p))
        })
    }
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
    /// Direct display label (overrides the humanized `label_key`).
    #[serde(default)]
    pub label: Option<String>,
    /// One-line explanation shown under the label (e.g. exactly what telemetry
    /// sends, or what accepting the terms means). Transparency.
    #[serde(default)]
    pub description: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_path_matching() {
        let c = ComponentSection {
            id: "mcp".into(),
            name: "MCP".into(),
            description: String::new(),
            required: false,
            default: true,
            size_mb: 0,
            paths: vec!["bmm-mcp-server.exe".into(), "mcp/".into()],
        };
        assert!(c.matches("bmm-mcp-server.exe"));
        assert!(c.matches("mcp/data.json")); // prefix
        assert!(!c.matches("better-mods-manager.exe")); // core
        assert!(!c.matches("mcpx.txt")); // not under mcp/
    }
}
