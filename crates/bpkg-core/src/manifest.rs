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
    /// Prerequisites fetched from outside the package (a Python runtime, a
    /// redistributable). `#[serde(default)]` so every existing .bpkg still parses — this
    /// field arriving must not invalidate packages built before it existed.
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
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
            dependencies: Vec::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
            total_size: 0,
        }
    }

    /// Validate every declared dependency. Call this as soon as a manifest is parsed —
    /// the whole value of the checks is that they run before anything is fetched or
    /// written, while a bad manifest is still only text.
    ///
    /// Reports the FIRST failure with the offending id and field: an installer that says
    /// "invalid manifest" and stops leaves whoever built the package guessing.
    pub fn validate_dependencies(&self) -> std::result::Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for d in &self.dependencies {
            d.validate()?;
            // Two dependencies with one id would install over each other, and `detect`
            // would then report whichever won as proof that both are present.
            if !seen.insert(d.id.as_str()) {
                return Err(format!("duplicate dependency id: {}", d.id));
            }
        }
        Ok(())
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

/// How a fetched dependency becomes an installed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DepKind {
    /// A zip archive, unpacked into `install_to`. The form to prefer: it needs no
    /// elevation, changes nothing outside the install directory, and uninstalling is
    /// deleting a folder.
    Zip,
    /// An installer executable, run with `args`. Necessary for things that genuinely
    /// must touch the system (a redistributable, a driver) and unavoidable for those —
    /// but it runs with the installer's rights and can do anything, so it is the
    /// exception, not the default.
    Exe,
}

/// A prerequisite fetched from outside the package.
///
/// A `Component` is a group of files already inside the `.bpkg`; this is the other kind
/// of dependency — a Python runtime, a redistributable, a toolchain — which cannot be
/// shipped in the package, either because of its size or its licence.
///
/// Everything here is declared, never inferred at install time. The URL, the hash and the
/// destination are all fixed when the package is built, so an installer run has no
/// decisions left to make about what to fetch or where to put it — which is what keeps a
/// tampered manifest from turning into an arbitrary download-and-execute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    /// Stable id, e.g. "python". Also the folder name when `install_to` is defaulted.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// HTTPS source. Checked again at fetch time by net::download — declared here so a
    /// bad manifest is rejected when it is read, not after a download has started.
    pub url: String,
    /// SHA-256 of the downloaded bytes, lowercase hex. MANDATORY, and deliberately not an
    /// Option: a dependency is a file from another server, executed or unpacked on the
    /// user's machine. "No hash" is not a weaker guarantee, it is none at all, and an
    /// optional field is one somebody eventually leaves out.
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
    pub kind: DepKind,
    /// Where it lands, relative to the install directory. Must stay inside it.
    pub install_to: String,
    /// A path, relative to the install directory, whose existence means this is already
    /// installed. Lets a re-run skip a 10 MB download instead of redoing it.
    #[serde(default)]
    pub detect: Option<String>,
    /// Arguments for `DepKind::Exe`. Ignored for a zip.
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default = "default_true")]
    pub default: bool,
}

impl Dependency {
    /// Reject anything that could install outside the install directory, or fetch without
    /// a real integrity check.
    ///
    /// Called when a manifest is read, before any network work: the point is that a
    /// malformed or hostile manifest fails while it is still just JSON. A `..` segment in
    /// `install_to` is the whole game here — the installer writes with the user's rights,
    /// so an unchecked relative path is an arbitrary file write (CWE-22).
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("dependency has no id".into());
        }
        if !self.url.starts_with("https://") {
            return Err(format!("{}: url must be https", self.id));
        }
        if self.sha256.len() != 64 || !self.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("{}: sha256 must be 64 hex characters", self.id));
        }
        check_relative(&self.id, "install_to", &self.install_to)?;
        if let Some(d) = &self.detect {
            check_relative(&self.id, "detect", d)?;
        }
        Ok(())
    }
}

/// A path that must stay under the install directory.
///
/// Rejects absolute paths, drive letters, UNC prefixes and any `..` component. Checked on
/// the raw string rather than after canonicalising, because canonicalising resolves the
/// escape before anyone can object to it — and on a path that does not exist yet, it
/// cannot be canonicalised at all.
fn check_relative(id: &str, field: &str, value: &str) -> std::result::Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{id}: {field} is empty"));
    }
    let v = value.replace('\\', "/");
    if v.starts_with('/') || v.starts_with("//") {
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

    fn dep(install_to: &str) -> Dependency {
        Dependency {
            id: "python".into(),
            name: "Python 3.12 (embeddable)".into(),
            description: String::new(),
            url: "https://www.python.org/ftp/python/x.zip".into(),
            sha256: "a".repeat(64),
            size: 0,
            kind: DepKind::Zip,
            install_to: install_to.into(),
            detect: None,
            args: vec![],
            required: false,
            default: true,
        }
    }

    #[test]
    fn a_well_formed_dependency_passes() {
        assert!(dep("runtime/python").validate().is_ok());
        assert!(
            dep("runtime\\python").validate().is_ok(),
            "windows separators are fine"
        );
    }

    /// The one that matters. The installer writes with the user's rights, so an
    /// unchecked relative path is an arbitrary file write (CWE-22) — and every one of
    /// these looks harmless in a JSON file nobody reads closely.
    #[test]
    fn a_dependency_cannot_install_outside_the_install_directory() {
        for bad in [
            "../evil",
            "runtime/../../evil",
            "runtime\\..\\..\\evil",
            "/etc/cron.d/evil",
            "//server/share/evil",
            "C:/Windows/System32/evil",
            "C:evil",
            "",
            "   ",
        ] {
            assert!(
                dep(bad).validate().is_err(),
                "install_to {bad:?} was accepted"
            );
        }
    }

    #[test]
    fn detect_is_checked_the_same_way_as_install_to() {
        // It is only ever tested for existence, but a path that escapes still discloses
        // whether a file exists anywhere on the machine.
        let mut d = dep("runtime/python");
        d.detect = Some("../../../secret".into());
        assert!(d.validate().is_err());
    }

    #[test]
    fn integrity_and_transport_are_not_optional() {
        let mut d = dep("runtime/python");
        d.url = "http://python.org/x.zip".into();
        assert!(d.validate().is_err(), "plain http was accepted");

        for bad_hash in ["", "abc", &"z".repeat(64), &"a".repeat(63)] {
            let mut d = dep("runtime/python");
            d.sha256 = bad_hash.into();
            assert!(d.validate().is_err(), "sha256 {bad_hash:?} was accepted");
        }
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        // Two dependencies with one id install over each other, and `detect` then reports
        // whichever won as proof that both are present.
        let mut m = Manifest::new(meta());
        m.dependencies = vec![dep("runtime/a"), dep("runtime/b")];
        assert!(m.validate_dependencies().is_err());
    }

    #[test]
    fn a_manifest_written_before_dependencies_existed_still_parses() {
        // The field is #[serde(default)]; without that every already-published .bpkg
        // would fail to read the moment this shipped.
        let json = r#"{"schema":1,"app":{"id":"a","name":"a","version":"1","publisher":"p","platforms":["windows"]},"files":[],"created_at":"2026-01-01T00:00:00Z","total_size":0}"#;
        let m: Manifest = serde_json::from_str(json).expect("old manifest must still parse");
        assert!(m.dependencies.is_empty());
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
