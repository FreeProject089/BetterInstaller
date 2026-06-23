//! First-run configuration handoff (v3 Addendum §C).
//!
//! The installer never writes into an app's internal config format. Instead it
//! writes a standard `installer-handoff.json` that the app reads ONCE on first
//! launch, applies, then marks consumed. This keeps the engine fully app- and
//! language-agnostic.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::SetupOption;
use crate::error::{Error, Result};

/// The contract written for the target app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffDoc {
    pub schema: u16,
    /// Always "betterinstaller" — lets the app sanity-check the source.
    pub source: String,
    pub installer_version: String,
    pub app_version: String,
    /// RFC3339 timestamp.
    pub installed_at: String,
    /// Components the user chose to install.
    pub components: Vec<String>,
    /// Flat settings map (key → value) built from the project's setup options.
    pub settings: BTreeMap<String, serde_json::Value>,
}

impl HandoffDoc {
    pub const SCHEMA: u16 = 1;

    pub fn new(app_version: impl Into<String>, installer_version: impl Into<String>) -> Self {
        HandoffDoc {
            schema: Self::SCHEMA,
            source: "betterinstaller".to_string(),
            installer_version: installer_version.into(),
            app_version: app_version.into(),
            installed_at: chrono::Utc::now().to_rfc3339(),
            components: Vec::new(),
            settings: BTreeMap::new(),
        }
    }

    /// Set a flat setting key.
    pub fn set(&mut self, key: impl Into<String>, value: serde_json::Value) {
        self.settings.insert(key.into(), value);
    }

    /// Write the document atomically (temp file + rename) so the app never reads
    /// a half-written file.
    pub fn write_atomic(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let json = serde_json::to_vec_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &json).map_err(|e| Error::io(&tmp, e))?;
        std::fs::rename(&tmp, path).map_err(|e| Error::io(path, e))?;
        Ok(())
    }
}

/// Assembles a [`HandoffDoc`] from the project's setup-option definitions and the
/// values the user chose in the Configuration page.
///
/// `chosen` maps a [`SetupOption::id`] to the selected value:
///  - `bool`/`license` → JSON bool
///  - `select`        → JSON string
///
/// Each option's `maps_to` keys are stripped of an optional leading `settings.`
/// prefix and written flat into `settings`. A `license` option writes its bool to
/// every key it maps to (e.g. privacy_accepted + tos_accepted).
pub fn build(
    options: &[SetupOption],
    chosen: &BTreeMap<String, serde_json::Value>,
    components: Vec<String>,
    app_version: &str,
    installer_version: &str,
) -> HandoffDoc {
    let mut doc = HandoffDoc::new(app_version, installer_version);
    doc.components = components;

    for opt in options {
        // Resolve the value: explicit choice, else the declared default.
        let value = chosen
            .get(&opt.id)
            .cloned()
            .unwrap_or_else(|| opt.default.clone());
        for key in opt.maps_to.keys() {
            let flat = key.strip_prefix("settings.").unwrap_or(key);
            doc.set(flat, value.clone());
        }
    }
    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InstallerConfig;

    const BMM_TOML: &str = r#"
        [app]
        id = "com.bettermm.app"
        name = "Better Mods Manager"
        version = "1.0.0"
        publisher = "BetterCommunity"

        [[setup_option]]
        id = "language"
        type = "select"
        label_key = "setup.language"
        choices = ["auto","en","fr"]
        default = "auto"
        maps_to = "settings.language"

        [[setup_option]]
        id = "legal"
        type = "license"
        label_key = "setup.acceptLegal"
        required = true
        maps_to = ["settings.privacy_accepted","settings.tos_accepted"]

        [[setup_option]]
        id = "telemetry"
        type = "bool"
        label_key = "setup.telemetry"
        default = false
        maps_to = "settings.telemetry"
    "#;

    #[test]
    fn parses_handoff_config() {
        let cfg = InstallerConfig::from_toml(BMM_TOML).unwrap();
        assert_eq!(cfg.setup_options.len(), 3);
        // license maps to two keys
        assert_eq!(cfg.setup_options[1].maps_to.keys().len(), 2);
    }

    #[test]
    fn builds_handoff_doc_from_choices() {
        let cfg = InstallerConfig::from_toml(BMM_TOML).unwrap();

        let mut chosen = BTreeMap::new();
        chosen.insert("language".to_string(), serde_json::json!("fr"));
        chosen.insert("legal".to_string(), serde_json::json!(true)); // user accepted
        chosen.insert("telemetry".to_string(), serde_json::json!(true)); // opted in

        let doc = build(
            &cfg.setup_options,
            &chosen,
            vec!["core".into(), "mcp-server".into()],
            "1.0.0",
            "0.1.0",
        );

        assert_eq!(
            doc.settings.get("language").unwrap(),
            &serde_json::json!("fr")
        );
        // the single `license` accept fanned out to BOTH legal keys
        assert_eq!(
            doc.settings.get("privacy_accepted").unwrap(),
            &serde_json::json!(true)
        );
        assert_eq!(
            doc.settings.get("tos_accepted").unwrap(),
            &serde_json::json!(true)
        );
        assert_eq!(
            doc.settings.get("telemetry").unwrap(),
            &serde_json::json!(true)
        );
        assert_eq!(doc.components, vec!["core", "mcp-server"]);
        assert_eq!(doc.source, "betterinstaller");
    }

    #[test]
    fn defaults_apply_when_unchosen() {
        let cfg = InstallerConfig::from_toml(BMM_TOML).unwrap();
        let chosen = BTreeMap::new(); // user changed nothing
        let doc = build(&cfg.setup_options, &chosen, vec![], "1.0.0", "0.1.0");
        // telemetry default is false (opt-in)
        assert_eq!(
            doc.settings.get("telemetry").unwrap(),
            &serde_json::json!(false)
        );
        assert_eq!(
            doc.settings.get("language").unwrap(),
            &serde_json::json!("auto")
        );
    }
}
