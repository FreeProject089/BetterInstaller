//! What an uninstall is allowed to delete.
//!
//! The install directory is whatever the user picked, and "Browse…" hands back the folder
//! itself — nothing appends `\<app name>`. Pick `D:\Games` and the app's files land among
//! the games; the Apps & Features entry then records `InstallLocation = D:\Games`.
//! Uninstall used to be `remove_dir_all` of that location (or `rmdir /S /Q` from the
//! self-delete script), so removing the app removed every game with it. Nothing about
//! that needed an attacker: one ordinary choice in a folder picker, then Uninstall.
//!
//! The rule now is the one the uninstaller's own docs already promised — "reverse exactly
//! what it did":
//!
//! - an install into a folder that did not exist, or existed empty, OWNS it, and uninstall
//!   removes the folder whole (what it always did — it also takes the logs and caches the
//!   app wrote next to itself);
//! - an install into a folder that already held something records the files it wrote, and
//!   uninstall deletes those files and the directories they leave empty. Nothing else.
//!
//! Both facts are written into `uninstall-info.json` at install time, because only then is
//! "was this folder empty?" still answerable.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use bpkg_core::package::is_safe_entry_path;

pub const INFO_FILE: &str = "uninstall-info.json";
pub const UNINSTALLER: &str = "uninstall.exe";

/// `<dir>/uninstall-info.json`, or `Null` when absent or unreadable.
pub fn read_info(dir: &Path) -> serde_json::Value {
    std::fs::read(dir.join(INFO_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// Decided BEFORE anything is written into `dest`: does this install own the folder?
///
/// A repair or update keeps what the first install decided — by then the folder is full of
/// our own files, and "not empty" would wrongly demote it.
pub fn owns_dir_before_install(dest: &Path, prior: &serde_json::Value) -> bool {
    if let Some(owned) = prior["owns_dir"].as_bool() {
        return owned;
    }
    if !prior.is_null() {
        // Installed by a build that did not record ownership.
        return legacy_owns(dest, prior);
    }
    match std::fs::read_dir(dest) {
        Ok(mut entries) => entries.next().is_none(),
        // Missing: we are about to create it. Present but unreadable: not provably ours.
        Err(_) => !dest.exists(),
    }
}

/// An install recorded before `owns_dir` existed. The default location is
/// `…\Programs\<app name>`, and a folder named exactly after the app is one somebody made
/// for it; any other folder may be `D:\Games`.
fn legacy_owns(dir: &Path, info: &serde_json::Value) -> bool {
    match (
        dir.file_name().and_then(|n| n.to_str()),
        info["app_name"].as_str(),
    ) {
        (Some(name), Some(app)) => !app.trim().is_empty() && name.eq_ignore_ascii_case(app.trim()),
        _ => false,
    }
}

/// The file list to record: what earlier runs installed plus what this one wrote (a repair
/// with a different component selection must not forget files the first run put there).
pub fn merged_files(prior: &serde_json::Value, written: &[String]) -> Vec<String> {
    let mut all: BTreeSet<String> = recorded_files(prior)
        .unwrap_or_default()
        .into_iter()
        .collect();
    all.extend(written.iter().cloned());
    all.into_iter().collect()
}

fn recorded_files(info: &serde_json::Value) -> Option<Vec<String>> {
    info["files"].as_array().map(|a| {
        a.iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    })
}

/// What one uninstall will do.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    /// Remove the whole folder (it is ours). Otherwise only `files`.
    pub recursive: bool,
    /// Relative paths, every one checked to stay inside the folder.
    pub files: Vec<String>,
}

/// `fallback_files` is the embedded package's manifest — the list for an install that
/// predates the recorded one.
pub fn plan(dir: &Path, info: &serde_json::Value, fallback_files: &[String]) -> Plan {
    let owns = info["owns_dir"]
        .as_bool()
        .unwrap_or_else(|| legacy_owns(dir, info));
    let mut files = recorded_files(info).unwrap_or_else(|| fallback_files.to_vec());
    files.push(UNINSTALLER.into());
    files.push(INFO_FILE.into());
    // The list is read from a file in the install folder. A `..` or an absolute path in it
    // would turn "delete what we installed" into "delete that".
    files.retain(|f| is_safe_entry_path(f));
    files.sort();
    files.dedup();
    Plan {
        recursive: owns && !too_broad_to_own(dir),
        files,
    }
}

/// Folders no install owns whatever its record says: a filesystem root, a folder one level
/// below one (`D:\Games`, `C:\Users`), and the user's home.
fn too_broad_to_own(dir: &Path) -> bool {
    let normal = dir
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count();
    if dir.parent().is_none() || normal < 2 {
        return true;
    }
    ["USERPROFILE", "HOME"]
        .iter()
        .filter_map(std::env::var_os)
        .any(|home| Path::new(&home) == dir)
}

/// Delete `files` (never `keep`, the running uninstaller) and then each directory they
/// leave empty, deepest first — never `dir` itself. Returns how many files were removed.
pub fn remove_listed(dir: &Path, files: &[String], keep: &Path) -> usize {
    let mut parents: BTreeSet<PathBuf> = BTreeSet::new();
    let mut removed = 0;
    for rel in files {
        if !is_safe_entry_path(rel) {
            continue;
        }
        let path = dir.join(rel);
        if path == keep {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        // A listed path was a file when we wrote it. A directory there now is not ours.
        if meta.is_dir() {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
        let mut cur = path.parent();
        while let Some(p) = cur {
            if p == dir || !p.starts_with(dir) {
                break;
            }
            parents.insert(p.to_path_buf());
            cur = p.parent();
        }
    }
    let mut parents: Vec<PathBuf> = parents.into_iter().collect();
    parents.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for p in parents {
        // Succeeds only when empty: a folder that still holds the user's files stays.
        let _ = std::fs::remove_dir(&p);
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bi-uninst-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(p: &Path, body: &[u8]) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// The trigger: the user browsed to a folder that already held their things.
    #[test]
    fn installing_into_a_folder_that_held_files_does_not_own_it() {
        let base = scratch("own");
        let games = base.join("Games");
        put(&games.join("SomeGame/save.dat"), b"100 hours");
        assert!(!owns_dir_before_install(&games, &serde_json::Value::Null));

        let fresh = base.join("Fresh");
        assert!(owns_dir_before_install(&fresh, &serde_json::Value::Null));
        std::fs::create_dir_all(base.join("Empty")).unwrap();
        assert!(owns_dir_before_install(
            &base.join("Empty"),
            &serde_json::Value::Null
        ));

        // A repair keeps the first install's answer, whatever the folder holds now.
        assert!(owns_dir_before_install(
            &games,
            &json!({ "owns_dir": true })
        ));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn uninstalling_from_a_shared_folder_removes_only_what_was_installed() {
        let base = scratch("shared");
        let dir = base.join("Games");
        put(&dir.join("SomeGame/save.dat"), b"100 hours");
        put(&dir.join("notes.txt"), b"mine");
        // What the install wrote.
        put(&dir.join("app.exe"), b"app");
        put(&dir.join("res/lang/en.json"), b"{}");
        put(&dir.join(UNINSTALLER), b"setup");
        put(&dir.join(INFO_FILE), b"{}");
        // Outside the folder entirely, named by a hostile record.
        put(&base.join("outside.txt"), b"keep");

        let info = json!({
            "owns_dir": false,
            "files": ["app.exe", "res/lang/en.json", "../outside.txt", "SomeGame"],
        });
        let p = plan(&dir, &info, &[]);
        assert!(
            !p.recursive,
            "a folder that held the user's files is never removed whole"
        );
        remove_listed(&dir, &p.files, Path::new("not-running"));

        assert_eq!(
            std::fs::read(dir.join("SomeGame/save.dat")).unwrap(),
            b"100 hours"
        );
        assert_eq!(std::fs::read(dir.join("notes.txt")).unwrap(), b"mine");
        assert_eq!(std::fs::read(base.join("outside.txt")).unwrap(), b"keep");
        assert!(!dir.join("app.exe").exists());
        assert!(!dir.join("res").exists(), "emptied directories are removed");
        assert!(!dir.join(UNINSTALLER).exists() && !dir.join(INFO_FILE).exists());
        assert!(dir.is_dir(), "the folder itself was the user's and stays");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn an_owned_folder_is_removed_whole_but_never_a_root_or_a_drive_folder() {
        let owned = json!({ "owns_dir": true, "files": [] });
        let deep = std::env::temp_dir()
            .join("bi-x")
            .join("Programs")
            .join("App");
        assert!(plan(&deep, &owned, &[]).recursive);
        for broad in [
            if cfg!(windows) { "D:\\" } else { "/" },
            if cfg!(windows) { "D:\\Games" } else { "/games" },
        ] {
            assert!(
                !plan(Path::new(broad), &owned, &[]).recursive,
                "{broad} must never be removed recursively"
            );
        }
    }

    /// Installs recorded before `owns_dir`: a folder named after the app is its own, anything
    /// else falls back to the package's file list.
    #[test]
    fn a_legacy_record_is_owned_only_when_the_folder_is_named_after_the_app() {
        let legacy = json!({ "app_name": "Better Mods Manager" });
        let own = std::env::temp_dir()
            .join("Programs")
            .join("Better Mods Manager");
        assert!(plan(&own, &legacy, &[]).recursive);

        let games = std::env::temp_dir().join("x").join("Games");
        let p = plan(&games, &legacy, &["app.exe".into()]);
        assert!(!p.recursive);
        assert!(p.files.contains(&"app.exe".to_string()));
    }

    #[test]
    fn repairs_keep_every_file_an_earlier_run_installed() {
        let prior = json!({ "files": ["a.exe", "mcp.exe"] });
        assert_eq!(
            merged_files(&prior, &["a.exe".into(), "b.dll".into()]),
            vec!["a.exe", "b.dll", "mcp.exe"]
        );
    }
}
