//! Updates with atomic-ish rollback.
//!
//! Local apply: snapshot the install dir to a sibling `<name>.bak`, extract the
//! new package over it; on ANY error, wipe + restore from the snapshot; on
//! success, drop the snapshot. Remote: [`check_remote`] fetches an
//! [`UpdateManifest`] and [`download_and_apply`] downloads the new package —
//! preferring a small binary [delta](crate::delta) from the current version when
//! offered — then applies it with the same rollback safety net.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::package::Package;

/// A remote update manifest (JSON at a stable URL).
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateManifest {
    /// Latest available version, e.g. "1.2.0".
    pub version: String,
    /// URL of the full `.bpkg` for that version.
    pub url: String,
    /// Optional hex Ed25519 signature note (verification uses the package's own sig).
    #[serde(default)]
    pub notes: Option<String>,
    /// Binary deltas from older versions (download a small patch instead of the full pkg).
    #[serde(default)]
    pub deltas: Vec<DeltaEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeltaEntry {
    /// The version this patch upgrades *from*.
    pub from: String,
    /// URL of the bsdiff patch (old.bpkg → new.bpkg).
    pub url: String,
}

/// `a` is a strictly newer dotted version than `b` (numeric, component-wise).
pub fn is_newer(a: &str, b: &str) -> bool {
    let parse =
        |s: &str| -> Vec<u64> { s.split('.').filter_map(|x| x.trim().parse().ok()).collect() };
    let (pa, pb) = (parse(a), parse(b));
    for i in 0..pa.len().max(pb.len()) {
        let (x, y) = (
            pa.get(i).copied().unwrap_or(0),
            pb.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

/// Fetch the manifest at `manifest_url`; return it only if newer than `current_version`.
pub fn check_remote(manifest_url: &str, current_version: &str) -> Result<Option<UpdateManifest>> {
    let text = crate::net::fetch_text(manifest_url)?;
    let m: UpdateManifest = serde_json::from_str(&text)?;
    Ok(if is_newer(&m.version, current_version) {
        Some(m)
    } else {
        None
    })
}

/// Download the new package — using a binary delta from `current_version` when one
/// is offered and `current_bpkg` is available — then apply it with rollback.
pub fn download_and_apply(
    m: &UpdateManifest,
    current_version: &str,
    current_bpkg: Option<&Path>,
    install_dir: &Path,
) -> Result<u64> {
    let new_bytes = match (
        current_bpkg,
        m.deltas.iter().find(|d| d.from == current_version),
    ) {
        (Some(old_path), Some(delta)) => {
            // Delta path: download a small patch and reconstruct the new package.
            let old = std::fs::read(old_path).map_err(|e| Error::io(old_path, e))?;
            let patch = crate::net::download(&delta.url)?;
            crate::delta::apply_delta(&old, &patch)?
        }
        _ => crate::net::download(&m.url)?, // full download
    };

    let tmp = std::env::temp_dir().join(format!("bi-update-{}.bpkg", std::process::id()));
    std::fs::write(&tmp, &new_bytes).map_err(|e| Error::io(&tmp, e))?;
    let res = apply_package_update(&tmp, install_dir, None);
    let _ = std::fs::remove_file(&tmp);
    res
}

/// Apply a newer `.bpkg` over `install_dir`, rolling back on failure.
/// Returns the number of files written on success.
pub fn apply_package_update(
    new_bpkg: &Path,
    install_dir: &Path,
    components: Option<&[String]>,
) -> Result<u64> {
    let backup = backup_path(install_dir);
    let _ = remove_path(&backup);
    copy_dir(install_dir, &backup)?; // snapshot

    let outcome = (|| -> Result<u64> {
        let mut pkg = Package::open(new_bpkg)?;
        pkg.install_with_progress(install_dir, components, |_, _, _| {})
    })();

    match outcome {
        Ok(n) => {
            let _ = remove_path(&backup);
            Ok(n)
        }
        Err(e) => {
            // Roll back: discard the half-applied dir, restore the snapshot.
            let _ = wipe_dir_contents(install_dir);
            let _ = copy_dir(&backup, install_dir);
            let _ = remove_path(&backup);
            Err(e)
        }
    }
}

fn backup_path(dir: &Path) -> PathBuf {
    let name = dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "install".into());
    dir.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{name}.bak"))
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).map_err(|e| Error::io(dst, e))?;
    for entry in std::fs::read_dir(src).map_err(|e| Error::io(src, e))? {
        let entry = entry.map_err(|e| Error::io(src, e))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| Error::io(&from, e))?;
        }
    }
    Ok(())
}

fn wipe_dir_contents(dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))? {
        let p = entry.map_err(|e| Error::io(dir, e))?.path();
        if p.is_dir() {
            let _ = std::fs::remove_dir_all(&p);
        } else {
            let _ = std::fs::remove_file(&p);
        }
    }
    Ok(())
}

fn remove_path(p: &Path) -> Result<()> {
    if p.is_dir() {
        std::fs::remove_dir_all(p).map_err(|e| Error::io(p, e))
    } else if p.exists() {
        std::fs::remove_file(p).map_err(|e| Error::io(p, e))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::AppMeta;

    fn app() -> AppMeta {
        AppMeta {
            id: "test".into(),
            name: "Test".into(),
            version: "1".into(),
            publisher: "p".into(),
            homepage: None,
            platforms: vec!["windows".into()],
        }
    }

    #[test]
    fn version_compare() {
        assert!(is_newer("1.2.0", "1.1.9"));
        assert!(is_newer("2.0.0", "1.9.9"));
        assert!(is_newer("1.0.1", "1.0")); // missing components = 0
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.1"));
    }

    #[test]
    fn update_succeeds_then_rolls_back_on_corrupt() {
        let base = std::env::temp_dir().join(format!("bpkg-upd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let v1 = base.join("v1");
        let v2 = base.join("v2");
        std::fs::create_dir_all(&v1).unwrap();
        std::fs::create_dir_all(&v2).unwrap();
        std::fs::write(v1.join("f.txt"), b"VERSION ONE").unwrap();
        std::fs::write(v2.join("f.txt"), b"VERSION TWO!!").unwrap();

        let p1 = base.join("v1.bpkg");
        let p2 = base.join("v2.bpkg");
        crate::package::create_from_dir(&v1, app(), vec![], |_| None, &p1).unwrap();
        crate::package::create_from_dir(&v2, app(), vec![], |_| None, &p2).unwrap();

        // Install v1.
        let install = base.join("install");
        {
            let mut pkg = Package::open(&p1).unwrap();
            pkg.install_with_progress(&install, None, |_, _, _| {})
                .unwrap();
        }
        assert_eq!(
            std::fs::read(install.join("f.txt")).unwrap(),
            b"VERSION ONE"
        );

        // Update to v2 → success.
        apply_package_update(&p2, &install, None).unwrap();
        assert_eq!(
            std::fs::read(install.join("f.txt")).unwrap(),
            b"VERSION TWO!!"
        );

        // Corrupt v2 and update again → must fail AND leave v2 intact (rollback).
        let mut bytes = std::fs::read(&p2).unwrap();
        let n = bytes.len();
        bytes[n - 40] ^= 0xFF; // flip a byte inside the compressed payload
        let bad = base.join("bad.bpkg");
        std::fs::write(&bad, &bytes).unwrap();
        let err = apply_package_update(&bad, &install, None);
        assert!(err.is_err(), "corrupt update must fail");
        assert_eq!(
            std::fs::read(install.join("f.txt")).unwrap(),
            b"VERSION TWO!!",
            "rollback should restore the pre-update state"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
