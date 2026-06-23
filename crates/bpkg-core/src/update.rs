//! In-place updates with atomic-ish rollback.
//!
//! Strategy: snapshot the install dir to a sibling `<name>.bak`, extract the new
//! package over the install dir; on ANY error, wipe + restore from the snapshot.
//! On success, drop the snapshot. (Remote manifest fetch + binary delta patches
//! are a later increment; this is the local apply + safety net.)

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::package::Package;

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
