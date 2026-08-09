//! An install receipt, for the platforms that have nowhere to put one.
//!
//! Windows records an install under `HKCU\...\Uninstall\<app_id>`, which is what
//! `installed_dir` / `installed_version` read back. That is how the installer knows it is
//! looking at an existing install and offers Update / Repair / Uninstall instead of a
//! fresh install.
//!
//! Linux and macOS had no equivalent: `register_uninstaller` was a no-op and the two
//! lookups fell through to the trait's `None` default. So on those platforms every run
//! looked like a first install — maintenance mode was unreachable, and an "update" would
//! have been a blind re-extract with no version to compare against. The cross-platform
//! claim in the README was really "it can install once".
//!
//! A JSON file per app under the user's data dir. Per-user on purpose: it has to match
//! where the app is actually installed, and these installers do not elevate.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::manifest::AppMeta;

/// Where receipts live. `$XDG_DATA_HOME` (or `~/.local/share`) on Linux,
/// `~/Library/Application Support` on macOS.
fn receipts_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        super::env_dir("HOME", "/tmp").join("Library/Application Support/BetterInstaller/receipts")
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
            return PathBuf::from(xdg).join("betterinstaller/receipts");
        }
        super::env_dir("HOME", "/tmp").join(".local/share/betterinstaller/receipts")
    }
}

/// An app id reaches this from a config file, so it is not allowed to steer the path.
/// Anything outside `[A-Za-z0-9._-]` is replaced, and the leading dots that would make a
/// hidden file — or `..`, which would climb out of the directory — cannot survive.
fn receipt_path(app_id: &str) -> PathBuf {
    let safe: String = app_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = safe.trim_matches('.');
    let safe = if safe.is_empty() { "app" } else { safe };
    receipts_dir().join(format!("{safe}.json"))
}

/// Written on install, read on every later run, removed on uninstall.
pub fn write(app: &AppMeta, install_dir: &Path) -> Result<()> {
    let dir = receipts_dir();
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    // Hand-rolled rather than pulling serde_json in here: three fields, and the values are
    // escaped below, so there is nothing a path or a version string can do to the shape.
    let body = format!(
        "{{\n  \"id\": \"{}\",\n  \"version\": \"{}\",\n  \"install_dir\": \"{}\"\n}}\n",
        esc(&app.id),
        esc(&app.version),
        esc(&install_dir.to_string_lossy())
    );
    let p = receipt_path(&app.id);
    std::fs::write(&p, body).map_err(|e| Error::io(&p, e))
}

pub fn remove(app_id: &str) -> Result<()> {
    let p = receipt_path(app_id);
    match std::fs::remove_file(&p) {
        Ok(()) => Ok(()),
        // Uninstalling something that left no receipt is not an error.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(&p, e)),
    }
}

/// The recorded install directory — `None` when there is no receipt, and also when the
/// directory it names is gone. A receipt pointing at a folder the user deleted by hand
/// must not make the installer offer Repair on nothing.
pub fn installed_dir(app_id: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(field(app_id, "install_dir")?);
    dir.exists().then_some(dir)
}

pub fn installed_version(app_id: &str) -> Option<String> {
    field(app_id, "version")
}

/// Read one string field back. The file is ours and has a fixed shape, so this looks for
/// `"key": "` and takes up to the next unescaped quote rather than carrying a JSON parser.
fn field(app_id: &str, key: &str) -> Option<String> {
    let body = std::fs::read_to_string(receipt_path(app_id)).ok()?;
    let needle = format!("\"{key}\": \"");
    let start = body.find(&needle)? + needle.len();
    let rest = &body[start..];
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(chars.next()?),
            '"' => return Some(out),
            _ => out.push(c),
        }
    }
    None
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_id_cannot_steer_the_receipt_path() {
        // The id comes from a config file. None of these may escape the receipts dir.
        for id in ["../../etc/passwd", "..", ".", "a/b", "a\\b", "", "..hidden"] {
            let p = receipt_path(id);
            assert_eq!(
                p.parent(),
                Some(receipts_dir().as_path()),
                "id {id:?} escaped the receipts directory: {p:?}"
            );
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            assert!(
                !name.starts_with('.'),
                "id {id:?} produced a hidden file: {name}"
            );
        }
    }

    #[test]
    fn a_windows_path_survives_the_round_trip() {
        // Backslashes and quotes are escaped on the way in; the reader has to undo exactly
        // that, or a Windows-style path comes back mangled and installed_dir() misses.
        let raw = r#"C:\Users\a"b\Programs\App"#;
        let json = format!("{{\n  \"install_dir\": \"{}\"\n}}\n", esc(raw));
        let start = json.find("\"install_dir\": \"").unwrap() + "\"install_dir\": \"".len();
        let rest = &json[start..];
        let mut out = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => out.push(chars.next().unwrap()),
                '"' => break,
                _ => out.push(c),
            }
        }
        assert_eq!(out, raw);
    }
}
