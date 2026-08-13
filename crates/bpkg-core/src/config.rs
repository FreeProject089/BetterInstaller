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
/// NOTE: `default_dir` and `allow_portable` used to live here and were never read by
/// anything. The install root always comes from `Platform::default_install_dir` — on
/// Windows `%LOCALAPPDATA%\Programs\<name>`, which is what the `asInvoker` manifest can
/// actually write to; honouring a configured `{ProgramFiles}` path would have required an
/// elevated build and failed at runtime. There is no portable mode to gate. Old TOML files
/// keep parsing — serde ignores unknown keys here.
pub struct InstallSection {
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
}

impl Default for InstallSection {
    fn default() -> Self {
        InstallSection {
            main_exe: None,
            protocol: None,
            create_shortcuts: true,
            desktop_shortcut: false,
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
    /// For `swatch`: one entry per choice, giving a readable name and the handful
    /// of colors the preview tile is painted with. Declarative on purpose — the
    /// installer paints a generic little window out of them and never learns what
    /// the app means by them.
    #[serde(default)]
    pub previews: Vec<SetupChoicePreview>,
    /// Ask again on the final page, once the install has actually succeeded.
    ///
    /// For a choice like a theme, deciding on a picture is easier than deciding on
    /// a name in a dropdown, and the last page is the first moment the user has
    /// nothing else to do. Answering is optional: the value chosen during setup
    /// already stands, and skipping keeps it.
    #[serde(default)]
    pub show_at_end: bool,
    /// Default value as a JSON value (bool, string, …).
    #[serde(default)]
    pub default: serde_json::Value,
    /// For `license`: documents to display (relative paths in the package).
    #[serde(default)]
    pub documents: Vec<String>,
    /// For `license`: require the reader to reach the END of a document before its accept
    /// box becomes usable. Off by default, because forcing it on every project would be a
    /// behaviour change nobody asked for; BMM turns it on.
    ///
    /// It is a nudge, not a proof of reading — but a checkbox that cannot be ticked from
    /// the first screenful is the difference between "I clicked past it" and "I at least
    /// saw how long it was".
    #[serde(default)]
    pub require_scroll: bool,
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
    /// A grid of picture tiles → string. Same value shape as `select`; it differs
    /// only in being shown rather than named.
    Swatch,
}

/// One tile of a `swatch` option.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupChoicePreview {
    /// The value written to the handoff when this tile is picked.
    pub value: String,
    /// Human name under the tile. Falls back to `value`.
    #[serde(default)]
    pub label: Option<String>,
    /// `#rrggbb` colors, in order: background, surface, accent, text. Fewer is
    /// allowed — the missing ones fall back to the installer's own palette, so a
    /// half-filled entry renders as a dull tile rather than an invisible one.
    #[serde(default)]
    pub colors: Vec<String>,
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

    // The BMM manifest is the only real consumer of `swatch`, and its previews are
    // hand-copied from the app's theme tokens. A typo in the table is invisible at
    // runtime — serde's `default` turns an unparseable `previews` into an empty
    // list, and the picker then renders as an empty box with two buttons.
    #[test]
    fn bmm_manifest_declares_a_complete_swatch() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/bmm/installer.toml"
        );
        let cfg = InstallerConfig::load(path).expect("the BMM manifest must parse");
        let theme = cfg
            .setup_options
            .iter()
            .find(|o| o.id == "theme")
            .expect("BMM declares a theme option");

        assert!(matches!(theme.kind, SetupOptionKind::Swatch));
        assert!(
            theme.show_at_end,
            "the theme option asks for the final picker"
        );

        // Every choice must have a tile, and every tile a known choice: a preview
        // for a value that is not offered is dead, and a value with no preview
        // renders as a blank tile the user cannot tell apart.
        assert_eq!(theme.previews.len(), theme.choices.len());
        for c in &theme.choices {
            assert!(
                theme.previews.iter().any(|p| &p.value == c),
                "choice {c} has no preview tile"
            );
        }
        // Four usable colors each, or the tile silently falls back to the
        // installer's own palette and every theme looks identical.
        for p in &theme.previews {
            assert_eq!(
                p.colors.len(),
                4,
                "{} needs bg/surface/accent/text",
                p.value
            );
            for c in &p.colors {
                assert!(
                    c.len() == 7
                        && c.starts_with('#')
                        && c[1..].chars().all(|ch| ch.is_ascii_hexdigit()),
                    "{} has a malformed color {c}",
                    p.value
                );
            }
        }
        // The default must be one of the tiles, or the picker opens with nothing
        // highlighted.
        let default = theme.default.as_str().expect("a string default");
        assert!(theme.previews.iter().any(|p| p.value == default));
    }
}
