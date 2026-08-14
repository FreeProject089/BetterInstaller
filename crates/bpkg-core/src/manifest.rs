//! The package manifest: the JSON metadata block embedded at the head of every
//! `.bpkg`. It describes the app, its files (with per-file SHA-256), components,
//! and integrity info — everything the engine needs to install without trusting
//! the payload blindly.

use serde::{Deserialize, Serialize};

/// Top-level manifest stored (uncompressed) in the `.bpkg` header region.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Manifest schema version (independent of the container format version).
    pub schema: u16,
    pub app: AppMeta,
    /// All files shipped by the package, with integrity hashes.
    pub files: Vec<FileEntry>,
    /// Declared components (e.g. core, mcp-server, cli).
    #[serde(default)]
    pub components: Vec<Component>,
    /// RFC3339 build timestamp.
    pub created_at: String,
    /// Total uncompressed size of all files, in bytes.
    pub total_size: u64,
}

impl Manifest {
    pub const SCHEMA: u16 = 1;

    pub fn new(app: AppMeta) -> Self {
        Manifest {
            schema: Self::SCHEMA,
            app,
            files: Vec::new(),
            components: Vec::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
            total_size: 0,
        }
    }

    /// Files belonging to a given component id (or all files when `id` is None).
    pub fn files_for<'a>(
        &'a self,
        id: Option<&'a str>,
    ) -> impl Iterator<Item = &'a FileEntry> + 'a {
        self.files
            .iter()
            .filter(move |f| id.is_none_or(|c| f.component.as_deref() == Some(c)))
    }
}

/// Identity of the application being installed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppMeta {
    /// Reverse-DNS identifier, e.g. "com.bettermm.app".
    pub id: String,
    /// Display name, e.g. "Better Mods Manager".
    pub name: String,
    pub version: String,
    pub publisher: String,
    #[serde(default)]
    pub homepage: Option<String>,
    /// Target platforms this package supports.
    #[serde(default = "default_platforms")]
    pub platforms: Vec<String>,
}

fn default_platforms() -> Vec<String> {
    vec!["windows".to_string()]
}

/// One file in the payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Path relative to the install root (forward slashes), e.g. "bin/app.exe".
    pub path: String,
    pub size: u64,
    /// Lowercase hex SHA-256 of the file contents.
    pub sha256: String,
    /// Component this file belongs to (None = always installed / core).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    /// Whether the file should be marked executable (Unix).
    #[serde(default)]
    pub executable: bool,
}

/// A selectable install component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
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

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> AppMeta {
        AppMeta {
            id: "com.test.app".into(),
            name: "Test".into(),
            version: "1.0.0".into(),
            publisher: "Test".into(),
            homepage: None,
            platforms: vec!["windows".into()],
        }
    }

    fn entry(path: &str, comp: Option<&str>) -> FileEntry {
        FileEntry {
            path: path.into(),
            size: 1,
            sha256: "0".repeat(64),
            component: comp.map(String::from),
            executable: false,
        }
    }

    #[test]
    fn files_for_filters_by_component() {
        let mut m = Manifest::new(meta());
        m.files = vec![
            entry("core.exe", None),
            entry("mcp.exe", Some("mcp")),
            entry("extra.dat", Some("extra")),
        ];
        // None → every file (nothing filtered out).
        assert_eq!(m.files_for(None).count(), 3);
        // A component id → only that component's files (core/None is NOT included).
        let mcp: Vec<_> = m.files_for(Some("mcp")).map(|f| f.path.as_str()).collect();
        assert_eq!(mcp, vec!["mcp.exe"]);
        // An unknown component → nothing.
        assert_eq!(m.files_for(Some("nope")).count(), 0);
    }

    #[test]
    fn appmeta_platforms_default_and_component_flags() {
        // `platforms` defaults to ["windows"] when absent; optional fields have defaults.
        let am: AppMeta =
            serde_json::from_str(r#"{"id":"a","name":"A","version":"1","publisher":"P"}"#).unwrap();
        assert_eq!(am.platforms, vec!["windows".to_string()]);
        assert!(am.homepage.is_none());

        // A Component omits `default`/`required` → default=true, required=false.
        let c: Component = serde_json::from_str(r#"{"id":"core","name":"Core"}"#).unwrap();
        assert!(c.default, "component default should be true when omitted");
        assert!(!c.required);
        assert_eq!(c.size_mb, 0);
    }
}
