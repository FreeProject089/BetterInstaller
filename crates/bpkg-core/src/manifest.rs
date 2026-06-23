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
