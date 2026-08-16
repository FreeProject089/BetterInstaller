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
    /// Minimum acceptable version, inclusive — e.g. "3.10". Only meaningful with
    /// `check_command`: a registry key or a file says something exists, not which build.
    ///
    /// "Installed" and "new enough" are different questions, and conflating them is why a
    /// machine with Python 3.8 counted as satisfying a prerequisite that needs 3.10 — the
    /// install then failed later, somewhere with no mention of Python.
    #[serde(default)]
    pub min_version: Option<String>,
    /// Maximum acceptable version, inclusive. For the case a newer release actually broke
    /// something — rarer than a minimum, and worth being able to say when it happens.
    #[serde(default)]
    pub max_version: Option<String>,
    /// How to ask for the version. Defaults to `--version`, which is what most tools
    /// answer; a few want something else (cmd wants `/c ver`, PowerShell 5.1 has no
    /// --version at all and errors on it).
    #[serde(default)]
    pub version_args: Option<Vec<String>>,
    /// Where to download the installer if missing (auto-install support).
    #[serde(default)]
    pub download_url: Option<String>,
    /// SHA-256 of the downloaded bytes, lowercase hex.
    ///
    /// Required whenever `download_url` is set — see `validate`. It is an Option only
    /// because a prerequisite that is merely CHECKED (a registry key for an already
    /// installed redistributable) downloads nothing and has nothing to hash.
    ///
    /// This was missing, and auto_install downloads to temp and EXECUTES the result.
    /// HTTPS protects the transport, but it says nothing about what the far end serves:
    /// an upstream that is compromised, hijacked, or simply repurposes a filename gets
    /// arbitrary code execution with the installer's rights. A hash is what makes the
    /// download the file that was reviewed rather than whatever answers today.
    #[serde(default)]
    pub sha256: Option<String>,
    /// What the download IS, and so what to do with it.
    #[serde(default)]
    pub kind: PrereqKind,
    /// For `PrereqKind::Zip`: where to unpack, relative to the install directory. Must
    /// stay inside it — see `check_relative`.
    #[serde(default)]
    pub install_to: Option<String>,
    /// Silent-install arguments for the downloaded installer.
    #[serde(default)]
    pub silent_args: Option<String>,
    #[serde(default = "default_true")]
    pub required: bool,
}

/// How a downloaded prerequisite becomes an installed one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrereqKind {
    /// An installer executable, run with `silent_args`. The default, because it is what
    /// every prerequisite did before `Zip` existed — changing the default would silently
    /// reinterpret existing installer.toml files.
    #[default]
    Exe,
    /// A zip unpacked into `install_to`. The form to prefer where there is a choice: no
    /// elevation, nothing touched outside the install directory, and uninstalling is
    /// deleting a folder. This is what makes a self-contained runtime — an embeddable
    /// Python, a portable toolchain — expressible at all.
    Zip,
}

impl Prerequisite {
    /// Reject a prerequisite that would download without an integrity check, or unpack
    /// outside the install directory.
    ///
    /// Runs when installer.toml is parsed, before any network work: the value of these
    /// checks is that a bad config fails while it is still text, not once a download is
    /// already on disk.
    pub fn validate(&self) -> std::result::Result<(), String> {
        let Some(url) = self.download_url.as_deref() else {
            // Check-only. Nothing is fetched, so there is nothing to verify.
            return Ok(());
        };
        if !url.starts_with("https://") {
            return Err(format!("{}: download_url must be https", self.id));
        }
        match self.sha256.as_deref() {
            None => return Err(format!("{}: download_url requires a sha256", self.id)),
            Some(h) if h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()) => {
                return Err(format!("{}: sha256 must be 64 hex characters", self.id));
            }
            Some(_) => {}
        }
        if self.kind == PrereqKind::Zip {
            let dest = self
                .install_to
                .as_deref()
                .ok_or_else(|| format!("{}: a zip prerequisite needs install_to", self.id))?;
            check_relative(&self.id, "install_to", dest)?;
        }
        Ok(())
    }
}

/// A path that must stay under the install directory.
///
/// Rejects absolute paths, drive letters, UNC prefixes and any `..` component. Checked on
/// the raw string rather than after canonicalising, because canonicalising resolves the
/// escape before anyone can object to it — and a path that does not exist yet cannot be
/// canonicalised at all.
///
/// The installer writes with the user's rights, so an unchecked relative path here is an
/// arbitrary file write (CWE-22), and every form of it looks harmless in a config file
/// nobody reads closely.
pub fn check_relative(id: &str, field: &str, value: &str) -> std::result::Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{id}: {field} is empty"));
    }
    let v = value.replace('\\', "/");
    if v.starts_with('/') {
        return Err(format!("{id}: {field} must be relative, got {value:?}"));
    }
    // "C:/x" and "C:x" are both absolute enough to escape.
    if v.len() >= 2 && v.as_bytes()[1] == b':' {
        return Err(format!(
            "{id}: {field} must not name a drive, got {value:?}"
        ));
    }
    if v.split('/').any(|seg| seg == "..") {
        return Err(format!(
            "{id}: {field} must not contain '..', got {value:?}"
        ));
    }
    Ok(())
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
        let cfg: Self = toml::from_str(s)?;
        cfg.validate_all()?;
        Ok(cfg)
    }

    /// Every per-section rule, run at LOAD time.
    ///
    /// `Prerequisite::validate` has existed and been tested since it was written, and nothing
    /// ever called it outside those tests — so the rules it enforces were true of the test
    /// fixtures and of nothing else. A recipe with `download_url = "http://…"` and no sha256
    /// parsed cleanly, and auto_install downloads to temp and EXECUTES the result: an
    /// upstream that is hijacked, compromised, or simply reusing a filename gets arbitrary
    /// code execution with the installer's rights.
    ///
    /// Called from from_toml so it cannot be skipped by loading a config another way — a
    /// check somebody has to remember to call is the check that was already here.
    pub fn validate_all(&self) -> Result<()> {
        for p in &self.prerequisites {
            // Other, not Config: Config wraps a toml parse failure, and this is a file
            // that parsed perfectly and says something unsafe. Reporting it as a parse
            // error would send somebody looking for a syntax mistake.
            p.validate().map_err(crate::error::Error::Other)?;
        }
        Ok(())
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

// ── The schema, as data ──────────────────────────────────────────────────────
//
// Every key `installer.toml` may set, derived from the types rather than listed. A list
// would be wrong the first time somebody adds a field, and wrong SILENTLY — which is the
// exact failure this whole area exists to catch, since no struct here uses
// `deny_unknown_fields` and serde therefore discards anything it does not recognise.
//
// Emitted by `bpkg schema` so a tool that is not this binary — a web recipe checker, an
// editor — can tell a typo from a real key without keeping its own copy of the schema. A
// second copy in another language is the same bug with a longer fuse.

/// Every key path in a JSON document, with array elements collapsed: an array of tables
/// contributes `components[].id` once, however many entries it has.
fn key_paths(v: &serde_json::Value, prefix: &str, out: &mut std::collections::BTreeSet<String>) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, val) in map {
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                out.insert(path.clone());
                key_paths(val, &path, out);
            }
        }
        serde_json::Value::Array(a) => {
            for item in a {
                key_paths(item, &format!("{prefix}[]"), out);
            }
        }
        _ => {}
    }
}

pub(crate) fn paths_of(v: &serde_json::Value) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    key_paths(v, "", &mut out);
    out
}

/// Keys the schema knows and NOTHING reads. They parse and do nothing, which is worse than
/// a typo because they look correct.
///
/// Listed rather than quietly deleted from the recipes that set them, because each is a
/// decision somebody has to make — add the field and wire it, or drop the line. Adding a
/// field that parses and still does nothing would be worse than the current state.
pub const KNOWN_DEAD: [&str; 3] = [
    "install.allow_portable",
    "install.default_dir",
    "update.package_urls",
];

/// A config with EVERY optional section present and every list non-empty.
///
/// This exists because the obvious derivation is wrong in a way that would be invisible: the
/// key set comes from serializing an INSTANCE, and `None` inside an `Option<Struct>` becomes
/// `null`, contributing the section's own name and none of its fields. Deriving the schema
/// from a real recipe would therefore omit `handoff.*`, `security.*` and `update.*` whenever
/// that recipe left them out — and a checker holding that schema would report a perfectly
/// valid `[handoff]` key as unknown. A schema that is confidently incomplete is worse than
/// none, and it is the same shape of mistake as the one it is built to catch.
///
/// Written as explicit struct literals with NO `..Default::default()`, deliberately: adding a
/// field to any of these structs stops this file compiling until somebody fills it in. That
/// is the ratchet. The alternative — a spread — would let a new field slip in, silently
/// missing from the emitted schema, which is precisely the failure above.
fn maximal() -> InstallerConfig {
    InstallerConfig {
        app: AppSection {
            id: String::new(),
            name: String::new(),
            version: String::new(),
            publisher: String::new(),
            homepage: None,
            platforms: vec![],
        },
        branding: Branding {
            accent: None,
            logo: None,
            background: None,
        },
        install: InstallSection {
            main_exe: None,
            protocol: None,
            create_shortcuts: true,
            desktop_shortcut: false,
        },
        components: vec![ComponentSection {
            id: String::new(),
            name: String::new(),
            description: String::new(),
            required: false,
            default: true,
            size_mb: 0,
            paths: vec![],
        }],
        handoff: Some(Handoff {
            enabled: true,
            file: String::new(),
            location: HandoffLocation::AppData,
        }),
        setup_options: vec![SetupOption {
            id: String::new(),
            kind: SetupOptionKind::Bool,
            label_key: String::new(),
            label: None,
            description: None,
            choices: vec![],
            previews: vec![SetupChoicePreview {
                value: String::new(),
                label: None,
                colors: vec![],
            }],
            show_at_end: false,
            // A SCALAR on purpose. `default` is a free-form serde_json::Value, so an object
            // here would invent key paths under it that no schema declares.
            default: serde_json::Value::Bool(false),
            documents: vec![],
            require_scroll: false,
            required: false,
            maps_to: MapsTo::None,
        }],
        security: Some(Security {
            public_key: None,
            require_signature: false,
        }),
        prerequisites: vec![Prerequisite {
            id: String::new(),
            name: String::new(),
            check_registry: None,
            check_file: None,
            check_command: None,
            min_version: None,
            max_version: None,
            version_args: None,
            download_url: None,
            sha256: None,
            kind: PrereqKind::default(),
            install_to: None,
            silent_args: None,
            required: true,
        }],
        launch: vec![LaunchItem {
            id: String::new(),
            label: String::new(),
            exe: String::new(),
            default: false,
            component: None,
        }],
        update: Some(UpdateConfig {
            manifest_url: String::new(),
            manifest_urls: vec![],
            auto_check: true,
            allow_delta: true,
        }),
        theme: ThemeConfig {
            bg: None,
            panel: None,
            panel2: None,
            border: None,
            accent: None,
            accent_dark: None,
            accent_hover: None,
            text: None,
            dim: None,
            danger: None,
            shadow: None,
        },
    }
}

/// Every key path `installer.toml` may set.
pub fn schema_key_paths() -> std::collections::BTreeSet<String> {
    let v = serde_json::to_value(maximal()).expect("the config must round-trip");
    paths_of(&v)
}

/// The schema as the artifact other tools read. Stable field names; `keys` is sorted, so a
/// regenerated file differs only when the schema does.
pub fn schema_json() -> Result<String> {
    let doc = serde_json::json!({
        "version": 1,
        "generator": "bpkg schema",
        "bpkgVersion": env!("CARGO_PKG_VERSION"),
        "keys": schema_key_paths().into_iter().collect::<Vec<_>>(),
        "knownDead": KNOWN_DEAD,
    });
    Ok(serde_json::to_string_pretty(&doc)?)
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

    fn prereq(url: Option<&str>) -> Prerequisite {
        Prerequisite {
            id: "python".into(),
            name: "Python 3.12 (embeddable)".into(),
            check_registry: None,
            check_file: None,
            check_command: Some("py".into()),
            min_version: None,
            max_version: None,
            version_args: None,
            download_url: url.map(String::from),
            sha256: Some("a".repeat(64)),
            kind: PrereqKind::Exe,
            install_to: None,
            silent_args: None,
            required: false,
        }
    }

    #[test]
    fn a_check_only_prerequisite_needs_no_hash() {
        // Nothing is fetched, so there is nothing to verify. Requiring a hash here would
        // force a meaningless one onto every "is the redistributable installed?" entry.
        let mut p = prereq(None);
        p.sha256 = None;
        assert!(p.validate().is_ok());
    }

    #[test]
    fn downloading_without_a_hash_is_refused() {
        // The gap this closes: auto_install writes the response to temp and RUNS it.
        // HTTPS proves who answered, not what they sent.
        let mut p = prereq(Some("https://example.com/x.exe"));
        p.sha256 = None;
        assert!(
            p.validate().is_err(),
            "a download with no hash was accepted"
        );

        for bad in ["", "abc", &"z".repeat(64), &"a".repeat(63)] {
            let mut p = prereq(Some("https://example.com/x.exe"));
            p.sha256 = Some(bad.into());
            assert!(p.validate().is_err(), "sha256 {bad:?} was accepted");
        }
    }

    #[test]
    fn plain_http_is_refused_for_a_download() {
        let p = prereq(Some("http://example.com/x.exe"));
        assert!(p.validate().is_err());
    }

    #[test]
    fn a_zip_prerequisite_cannot_unpack_outside_the_install_directory() {
        // The installer writes with the user's rights, so an unchecked install_to is an
        // arbitrary file write (CWE-22) — and each of these looks harmless in a TOML file.
        for bad in [
            "../evil",
            "runtime/../../evil",
            r"runtime\..\..\evil",
            "/etc/cron.d/evil",
            "C:/Windows/System32/evil",
            "C:evil",
            "",
            "   ",
        ] {
            let mut p = prereq(Some("https://example.com/x.zip"));
            p.kind = PrereqKind::Zip;
            p.install_to = Some(bad.into());
            assert!(p.validate().is_err(), "install_to {bad:?} was accepted");
        }
    }

    #[test]
    fn a_zip_prerequisite_needs_somewhere_to_go() {
        let mut p = prereq(Some("https://example.com/x.zip"));
        p.kind = PrereqKind::Zip;
        p.install_to = None;
        assert!(p.validate().is_err());

        p.install_to = Some("runtime/python".into());
        assert!(p.validate().is_ok());
    }

    #[test]
    fn an_installer_toml_written_before_these_fields_still_parses() {
        // All three are #[serde(default)], and kind defaults to Exe — what every
        // prerequisite did before Zip existed. A different default would silently
        // reinterpret configs already in use.
        let toml = r#"
            id = "vcredist"
            name = "VC++ Redistributable"
            required = true
        "#;
        let p: Prerequisite = toml::from_str(toml).expect("old config must still parse");
        assert_eq!(p.kind, PrereqKind::Exe);
        assert!(p.sha256.is_none() && p.install_to.is_none());
        assert!(p.validate().is_ok(), "a check-only entry must stay valid");
    }

    /// The shipped BMM config must satisfy the rules the engine enforces. A prerequisite
    /// that fails validate() would only be discovered when an installer tried to fetch it,
    /// on a user's machine.
    #[test]
    fn the_bmm_example_config_has_valid_prerequisites() {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/bmm/installer.toml"),
        )
        .expect("examples/bmm/installer.toml");
        let cfg: InstallerConfig = toml::from_str(&text).expect("installer.toml must parse");
        for p in &cfg.prerequisites {
            p.validate().unwrap_or_else(|e| panic!("{e}"));
        }
        // And the one that pairs with BMM's scheduler must keep pointing where it looks.
        let py = cfg
            .prerequisites
            .iter()
            .find(|p| p.id == "python")
            .expect("the python prerequisite");
        assert_eq!(
            py.install_to.as_deref(),
            Some("runtime/python"),
            "must match managed_engine() in src-tauri/src/commands/scheduler.rs"
        );
        assert_eq!(py.kind, PrereqKind::Zip);
        assert_eq!(
            py.check_command.as_deref(),
            Some("py"),
            "`python` would match the Microsoft Store alias and report a missing interpreter as present"
        );
    }

    // ── The recipe against the schema ────────────────────────────────────────────
    //
    // None of these structs uses `deny_unknown_fields`, so serde SILENTLY DROPS any key it
    // does not recognise. Write `[[componentss]]` in installer.toml and the installer builds
    // perfectly, with no components. Nothing errors, nothing warns, and the mistake surfaces
    // only as a missing feature in a shipped installer — the same failure family as the
    // `swatch` check above, where an unparseable table became an empty list.
    //
    // The set of keys the schema understands is derived by ROUND-TRIPPING the parsed config,
    // not by listing them here. A list would be wrong the first time somebody adds a field,
    // and a stale checker that reports nothing reads exactly like a clean one.
    //
    // Through JSON rather than TOML: `toml::Value::try_from` refuses an unset `Option` with
    // UnsupportedType("unit"), so the round-trip died on the first `None`. JSON represents it
    // as `null`, which is exactly what "a key the schema knows and the recipe never sets"
    // should look like.

    // key_paths / paths_of are no longer duplicated here: they live beside the schema they
    // derive, so `bpkg schema` and this test cannot disagree about what a key path is.

    const BMM_MANIFEST: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/bmm/installer.toml"
    );

    /// (keys the recipe sets, keys the schema understands)
    fn recipe_vs_schema(
        text: &str,
    ) -> (
        std::collections::BTreeSet<String>,
        std::collections::BTreeSet<String>,
    ) {
        let raw: toml::Value = toml::from_str(text).expect("the manifest must be valid TOML");
        let cfg: InstallerConfig = toml::from_str(text).expect("the manifest must parse");
        let raw_json = serde_json::to_value(raw).expect("TOML maps onto JSON");
        let cfg_json = serde_json::to_value(&cfg).expect("the config must round-trip");
        (paths_of(&raw_json), paths_of(&cfg_json))
    }

    /// The committed artifact must be what the CLI emits.
    ///
    /// A test rather than a CI step, deliberately: it runs on both platforms of the matrix,
    /// needs no built binary, and lives beside the derivation it guards. A stale schema is
    /// the one failure this whole feature cannot tolerate — a checker holding an out-of-date
    /// key set reports valid keys as unknown, which is worse than not checking at all,
    /// because it is confidently wrong.
    #[test]
    fn the_committed_schema_is_current() {
        const COMMITTED: &str = include_str!("../../../schema/installer-schema.json");
        let fresh = schema_json().expect("the schema must serialize");
        assert_eq!(
            COMMITTED.trim_end(),
            fresh.trim_end(),
            "schema/installer-schema.json is stale — run `bpkg schema --out schema/installer-schema.json`"
        );
    }

    /// Every key the real recipe sets is one the emitted schema knows — except the three
    /// already recorded as dead.
    ///
    /// This is the acceptance test for the artifact: if the schema were derived from an
    /// instance with its optional sections unset, `handoff.*` and `security.*` would be
    /// missing from it and a checker would call the BMM manifest's own keys unknown.
    #[test]
    fn the_emitted_schema_covers_the_real_recipe() {
        let text = std::fs::read_to_string(BMM_MANIFEST).unwrap();
        let (recipe, _) = recipe_vs_schema(&text);
        let emitted = schema_key_paths();
        let missing: Vec<_> = recipe
            .iter()
            .filter(|k| !emitted.contains(*k) && !KNOWN_DEAD.contains(&k.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "the emitted schema does not know {} key(s) the real recipe sets: {missing:?}",
            missing.len()
        );
    }

    #[test]
    fn the_bmm_manifest_sets_no_key_the_schema_ignores() {
        let text = std::fs::read_to_string(BMM_MANIFEST).unwrap();
        let (recipe, schema) = recipe_vs_schema(&text);

        // Known dead configuration, found by this check on its first run. Listed rather than
        // quietly deleted from the recipe, because each one is a decision somebody has to
        // make:
        //   install.default_dir    — the installer's default target directory is SPECIFIED
        //                            in the manifest and ignored; InstallSection has no such
        //                            field, so the value never reaches anything.
        //   install.allow_portable — same section, same fate.
        //   update.package_urls    — appears nowhere in the engine at all.
        //
        // A ratchet, not a suppression: a NEW ignored key still fails, which is the point.
        // Shrink this list when the fields land or the lines go.
        // The list lives in the module now, so the emitted schema and this ratchet name the
        // same three keys. Two copies would drift and the checker would stop agreeing with CI.

        let ignored: Vec<String> = recipe.difference(&schema).cloned().collect();
        let unexpected: Vec<_> = ignored
            .iter()
            .filter(|k| !KNOWN_DEAD.contains(&k.as_str()))
            .collect();
        assert!(
            unexpected.is_empty(),
            "installer.toml sets {} key(s) the schema does not know; serde drops them \
             silently: {unexpected:?}",
            unexpected.len()
        );

        // The debt must stay honest too: a name that stops being ignored (because the field
        // landed) belongs off this list, not sitting here forever pretending to be a finding.
        for k in KNOWN_DEAD {
            assert!(
                ignored.iter().any(|i| i == k),
                "{k} is no longer ignored — remove it from KNOWN_DEAD"
            );
        }
    }

    #[test]
    fn a_typo_in_the_recipe_is_invisible_without_this_check() {
        // The check above is only worth having if it FAILS on the mistake it exists for.
        // Proven on the real manifest, with one section name broken.
        let text = std::fs::read_to_string(BMM_MANIFEST).unwrap();
        let broken = text.replacen("[[components]]", "[[componentss]]", 1);
        assert_ne!(
            broken, text,
            "the manifest must contain a [[components]] table"
        );

        // Still valid TOML, and serde still accepts it happily — that is the whole problem.
        let (recipe, schema) = recipe_vs_schema(&broken);
        let ignored: Vec<_> = recipe.difference(&schema).cloned().collect();
        assert!(
            ignored.iter().any(|k| k.starts_with("componentss")),
            "a typo'd section must be reported as ignored, got {ignored:?}"
        );
    }

    #[test]
    fn the_schema_offers_more_than_the_only_recipe_uses() {
        // Informational, and deliberately not an assertion about WHICH keys: it prints the
        // options that exist and that nothing exercises, which is where a feature quietly
        // rots. Asserting a list here would only add a second place to update.
        let text = std::fs::read_to_string(BMM_MANIFEST).unwrap();
        let (recipe, schema) = recipe_vs_schema(&text);
        let unused: Vec<_> = schema.difference(&recipe).cloned().collect();
        println!(
            "schema keys the BMM recipe never sets ({}): {unused:?}",
            unused.len()
        );
    }

    // ── The prerequisite rules actually run ──────────────────────────────────────
    //
    // Prerequisite::validate has existed, and been tested, since it was written — and nothing
    // called it outside those tests. So its rules were true of the fixtures below and of
    // nothing else: a real installer.toml with an http:// download and no hash loaded
    // cleanly. auto_install downloads to temp and EXECUTES the result, so that is arbitrary
    // code execution with the installer's rights, gated on a check that never ran.
    //
    // These test from_toml, not validate, because "the rule is correct" and "the rule is
    // applied" are different claims and only the second one protects anybody.

    fn recipe_with(prereq: &str) -> String {
        format!(
            "[app]\nid = \"x\"\nname = \"X\"\nversion = \"1.0.0\"\npublisher = \"P\"\n\n{prereq}"
        )
    }

    #[test]
    fn a_download_without_a_hash_is_refused_at_LOAD() {
        let toml = recipe_with(
            "[[prerequisite]]\nid = \"p\"\nname = \"P\"\ndownload_url = \"https://example.com/p.zip\"\n",
        );
        let err = InstallerConfig::from_toml(&toml).unwrap_err().to_string();
        assert!(
            err.contains("sha256"),
            "expected a sha256 complaint, got: {err}"
        );
    }

    #[test]
    fn a_plain_http_download_is_refused_at_load() {
        let toml = recipe_with(
            "[[prerequisite]]\nid = \"p\"\nname = \"P\"\ndownload_url = \"http://example.com/p.zip\"\nsha256 = \"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"\n",
        );
        let err = InstallerConfig::from_toml(&toml).unwrap_err().to_string();
        assert!(
            err.contains("https"),
            "expected an https complaint, got: {err}"
        );
    }

    #[test]
    fn a_prerequisite_that_downloads_nothing_still_loads() {
        // Merely CHECKED — a registry key for something already installed. It has nothing to
        // hash, and demanding one would make the common case impossible.
        let toml =
            recipe_with("[[prerequisite]]\nid = \"p\"\nname = \"P\"\ncheck_command = \"py\"\n");
        assert!(InstallerConfig::from_toml(&toml).is_ok());
    }

    #[test]
    fn the_real_bmm_manifest_still_loads() {
        // The rule now runs on every load, so a manifest that was fine yesterday must still
        // be fine today — otherwise this change breaks the one installer that exists.
        assert!(InstallerConfig::load(BMM_MANIFEST).is_ok());
    }
}
