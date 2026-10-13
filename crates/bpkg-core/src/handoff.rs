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
    /// Where the app was installed (so the app can import bundled presets from
    /// `<install_dir>/...`). Empty when unknown.
    #[serde(default)]
    pub install_dir: String,
    /// Flat settings map (key → value) built from the project's setup options.
    pub settings: BTreeMap<String, serde_json::Value>,
    /// Proof of what was accepted: one receipt per legal document the user accepted, naming
    /// the exact text (SHA-256 of the file shown, see [`text_hash`]), its language and file,
    /// and when. The app records these in its own consent ledger; an app that does not know
    /// the field ignores it. Absent (not `[]`) when nothing was accepted, so older readers
    /// see the same document as before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consents: Vec<ConsentReceipt>,
}

/// One consent the installer obtained, as handed to the app.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConsentReceipt {
    /// `tos` / `privacy` (legal documents), `telemetry` (left to the app, which derives it
    /// from the settings it applies).
    pub doc: String,
    /// SHA-256 (64 lowercase hex) of the text shown, normalised by [`text_hash`].
    pub text_sha256: String,
    /// Language of the text shown (`en`, `fr`, ...).
    pub lang: String,
    /// The package path of the file shown (`TOS_FR.md`).
    pub file: String,
    /// `accepted` (the license step cannot be passed otherwise).
    pub choice: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub categories: BTreeMap<String, bool>,
    /// RFC 3339, UTC: when the install was confirmed.
    pub at: String,
    /// How the acceptance was given: `ui` (the installer window: each document read and
    /// ticked) or `cli` (an unattended install run with `--accept-terms` /
    /// `--accept-privacy`). Absent from receipts written before the field existed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

/// The SHA-256 of a document as BMM's consent ledger hashes it (`consent_ledger::text_hash`):
/// CRLF → LF, trailing spaces of every line trimmed, the whole trimmed. Two checkouts of the
/// same text with different line endings are the same text.
pub fn text_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let normalized: String = text
        .replace("\r\n", "\n")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    let digest = Sha256::digest(normalized.trim().as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// The document a legal file name stands for: `TOS*.md` → `tos`, `PRIVACY*.md` → `privacy`,
/// anything else → `None` (a project's own licence is not a consent the app tracks).
pub fn legal_doc_id(file: &str) -> Option<&'static str> {
    let name = file
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(file)
        .to_ascii_uppercase();
    if name.starts_with("TOS") || name.starts_with("EULA") || name.starts_with("TERMS") {
        Some("tos")
    } else if name.starts_with("PRIVACY") {
        Some("privacy")
    } else {
        None
    }
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
            install_dir: String::new(),
            settings: BTreeMap::new(),
            consents: Vec::new(),
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
        // Resolve the value: explicit choice, else the declared default; a child of a
        // parent that is off is written `false`.
        let value = crate::config::effective_value(options, opt, chosen);
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
    fn text_hash_matches_bmm_and_ignores_line_endings() {
        // BMM's consent ledger (consent_ledger.rs, text_hash) normalises the same way.
        let lf = text_hash("a  \nb\n");
        assert_eq!(lf, text_hash("a\r\nb\r\n"));
        use sha2::{Digest, Sha256};
        let want: String = Sha256::digest(b"a\nb")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(lf, want);
    }

    #[test]
    fn legal_files_name_their_document() {
        assert_eq!(legal_doc_id("TOS.md"), Some("tos"));
        assert_eq!(legal_doc_id("docs/TOS_FR.md"), Some("tos"));
        assert_eq!(legal_doc_id("PRIVACY_PT-BR.md"), Some("privacy"));
        assert_eq!(legal_doc_id("LICENSE.md"), None);
    }

    #[test]
    fn consents_are_omitted_when_empty_and_round_trip_when_present() {
        let mut d = HandoffDoc::new("1.0.2", "x");
        assert!(!serde_json::to_string(&d).unwrap().contains("consents"));
        d.consents.push(ConsentReceipt {
            doc: "tos".into(),
            text_sha256: "ab".repeat(32),
            lang: "fr".into(),
            file: "TOS_FR.md".into(),
            choice: "accepted".into(),
            at: "2026-10-13T10:00:00Z".into(),
            ..Default::default()
        });
        let back: HandoffDoc = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(back.consents, d.consents);
    }

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
