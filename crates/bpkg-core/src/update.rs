//! Updates with atomic-ish rollback.
//!
//! Local apply: snapshot the install dir to a sibling `<name>.bak`, extract the
//! new package over it; on ANY error, wipe + restore from the snapshot; on
//! success, drop the snapshot. Remote: [`check_remote`] fetches an
//! [`UpdateManifest`] and [`download_and_apply`] downloads the new package —
//! preferring a small binary [delta](crate::delta) from the current version when
//! offered — then applies it with the same rollback safety net.

use std::path::{Path, PathBuf};

use ed25519_dalek::VerifyingKey;
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
    /// Additional sources for the SAME package, tried in order after `url` when a download
    /// fails. Safe by construction: `apply_package_update` verifies the Ed25519 signature
    /// before touching the install directory, so a mirror can serve a bad file but never get
    /// it applied. Without this, one unreachable host means nobody can update at all.
    #[serde(default)]
    pub urls: Vec<String>,
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
    /// Mirrors for that patch, same rule as [`UpdateManifest::urls`].
    #[serde(default)]
    pub urls: Vec<String>,
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
    let m = fetch_manifest(manifest_url)?;
    Ok(if is_newer(&m.version, current_version) {
        Some(m)
    } else {
        None
    })
}

/// Fetch + parse one manifest URL.
fn fetch_manifest(url: &str) -> Result<UpdateManifest> {
    let text = crate::net::fetch_text(url)?;
    let m: UpdateManifest = serde_json::from_str(&text)?;
    Ok(m)
}

/// Check **several** manifest sources and return the single newest update across all of
/// them. Sources that fail to fetch/parse are skipped (one dead mirror never blocks the
/// others). Returns:
/// - `Ok(Some(m))` — the highest-version manifest found, when it's newer than `current`;
/// - `Ok(None)`    — at least one source was reachable but none is newer;
/// - `Err(_)`      — **every** source failed (so the caller can report it instead of
///   silently claiming "up to date").
///
/// Backward-compatible: pass a single URL and it behaves like [`check_remote`], so
/// multi-source is always opt-in (just list more URLs).
pub fn check_remote_multi(
    urls: &[String],
    current_version: &str,
) -> Result<Option<UpdateManifest>> {
    let mut best: Option<UpdateManifest> = None;
    let mut ok_count = 0u32;
    let mut last_err: Option<Error> = None;

    for url in urls.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        match fetch_manifest(url) {
            Ok(m) => {
                ok_count += 1;
                // Keep the highest version seen so far across all reachable sources.
                let take = best
                    .as_ref()
                    .is_none_or(|b| is_newer(&m.version, &b.version));
                if take {
                    best = Some(m);
                }
            }
            Err(e) => last_err = Some(e),
        }
    }

    if ok_count == 0 {
        return Err(last_err.unwrap_or_else(|| Error::Other("no update source configured".into())));
    }
    // Only offer the best one if it's actually newer than what's installed.
    Ok(best.filter(|m| is_newer(&m.version, current_version)))
}

/// Download the new package — using a binary delta from `current_version` when one
/// is offered and `current_bpkg` is available — then apply it with rollback.
///
/// `app_id`: when `Some`, the package must be THAT app (see [`check_offered`]).
pub fn download_and_apply(
    m: &UpdateManifest,
    current_version: &str,
    current_bpkg: Option<&Path>,
    install_dir: &Path,
    verify_key: Option<&VerifyingKey>,
    app_id: Option<&str>,
) -> Result<u64> {
    let new_bytes = match (
        current_bpkg,
        m.deltas.iter().find(|d| d.from == current_version),
    ) {
        (Some(old_path), Some(delta)) => {
            // Delta path: download a small patch and reconstruct the new package.
            let old = std::fs::read(old_path).map_err(|e| Error::io(old_path, e))?;
            let patch = download_any(&delta.url, &delta.urls)?;
            crate::delta::apply_delta(&old, &patch)?
        }
        _ => download_any(&m.url, &m.urls)?, // full download
    };

    apply_downloaded(
        &new_bytes,
        m,
        current_version,
        install_dir,
        verify_key,
        app_id,
    )
}

/// The half of [`download_and_apply`] after the bytes have arrived: stage, verify, check
/// the package is the update that was offered, apply with rollback.
pub fn apply_downloaded(
    bytes: &[u8],
    m: &UpdateManifest,
    current_version: &str,
    install_dir: &Path,
    verify_key: Option<&VerifyingKey>,
    app_id: Option<&str>,
) -> Result<u64> {
    // Private scratch dir: `apply_package_update` verifies the signature of this path and
    // then reads it again to extract, so a path a local attacker can write is a package
    // that is verified and a package that is installed (crate::tmp).
    let tmp = crate::tmp::stage("update.bpkg", bytes)?;
    let res = apply_checked(&tmp, install_dir, None, verify_key, |app| {
        check_offered(app, &m.version, current_version, app_id)
    });
    crate::tmp::discard(&tmp);
    res
}

/// A signature says who MADE a package, not which package it is. Everything else that
/// picks the file — the manifest's version, `url`, every mirror in `urls` — is unsigned, so
/// "the signature verifies" alone let a mirror (or whoever controls the manifest's host)
/// hand over ANY package that key ever signed:
///
/// - an older release, with the bugs that release was replaced for — a rollback, which
///   SECURITY.md lists as in scope;
/// - the release already installed, replayed forever so nothing newer is ever applied;
/// - a different app by the same publisher, installed over this one.
///
/// So the package's own manifest — the part the signature covers — must name the app being
/// updated and exactly the version that was offered, and that version must be newer than
/// what is installed.
pub fn check_offered(
    app: &crate::manifest::AppMeta,
    offered: &str,
    current_version: &str,
    app_id: Option<&str>,
) -> Result<()> {
    if let Some(id) = app_id {
        if app.id != id {
            return Err(Error::Other(format!(
                "update rejected: the package is for {:?}, not {id:?}",
                app.id
            )));
        }
    }
    let same = |a: &str, b: &str| !is_newer(a, b) && !is_newer(b, a);
    if !same(&app.version, offered) {
        return Err(Error::Other(format!(
            "update rejected: version {} was offered but the package is {}",
            offered, app.version
        )));
    }
    if !is_newer(&app.version, current_version) {
        return Err(Error::Other(format!(
            "update rejected: the package ({}) is not newer than the installed version ({})",
            app.version, current_version
        )));
    }
    Ok(())
}

/// Download from the primary URL, falling back to each mirror in turn.
///
/// Only the LAST error is surfaced: a caller shown "mirror 3 failed" learns nothing useful when
/// mirrors 1 and 2 also failed for the same reason (usually: no network).
fn download_any(primary: &str, mirrors: &[String]) -> Result<Vec<u8>> {
    let mut last = match crate::net::download(primary) {
        Ok(b) => return Ok(b),
        Err(e) => e,
    };
    for url in mirrors.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        match crate::net::download(url) {
            Ok(b) => return Ok(b),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Apply a newer `.bpkg` over `install_dir`, rolling back on failure.
/// Returns the number of files written on success.
///
/// `verify_key`: when `Some`, the package MUST carry a valid Ed25519 signature for
/// that key or the update is refused *before* the install dir is touched (fail
/// closed). Pass the publisher key from `installer.toml`'s `security.public_key` so a
/// tampered/unsigned package from a hostile mirror can never be applied. `None`
/// preserves the legacy unverified behaviour for callers that don't pin a key.
pub fn apply_package_update(
    new_bpkg: &Path,
    install_dir: &Path,
    components: Option<&[String]>,
    verify_key: Option<&VerifyingKey>,
) -> Result<u64> {
    apply_checked(new_bpkg, install_dir, components, verify_key, |_| Ok(()))
}

/// [`apply_package_update`] with one more gate, `accept`, run on the package's own app
/// metadata AFTER the signature check (so it reads signed bytes when a key is pinned) and
/// before anything is snapshotted or written.
fn apply_checked(
    new_bpkg: &Path,
    install_dir: &Path,
    components: Option<&[String]>,
    verify_key: Option<&VerifyingKey>,
    accept: impl FnOnce(&crate::manifest::AppMeta) -> Result<()>,
) -> Result<u64> {
    // Authenticity gate FIRST — refuse an unsigned/invalid package before we snapshot
    // or write anything.
    let mut pkg = Package::open(new_bpkg)?;
    if let Some(vk) = verify_key {
        if !pkg.verify_signature(vk)? {
            return Err(Error::Other(
                "update rejected: package signature is missing or invalid".into(),
            ));
        }
    }
    accept(&pkg.manifest.app)?;

    // A snapshot already there is what an interrupted update leaves behind (power cut,
    // killed process, or a rollback that could not finish — see below), and it may be the
    // only intact copy of the install. This used to delete it and snapshot the half-written
    // directory in its place.
    let backup = backup_path(install_dir);
    if backup.exists() {
        return Err(Error::Other(format!(
            "an earlier update did not finish: {} holds the install as it was before it. \
             Restore it or delete it, then update again",
            backup.display()
        )));
    }
    if let Err(e) = copy_dir(install_dir, &backup) {
        // Our own partial snapshot, from this call: nothing else is in it.
        let _ = remove_path(&backup);
        return Err(e);
    }

    let outcome = pkg.install_with_progress(install_dir, components, |_, _, _| {});

    match outcome {
        Ok(n) => {
            let _ = remove_path(&backup);
            Ok(n)
        }
        Err(e) => {
            // Roll back: discard the half-applied dir, restore the snapshot.
            //
            // The snapshot is deleted ONLY when every file came back. The restore used to
            // stop at its first failure and the snapshot was deleted regardless — and the
            // commonest reason an update fails is the same thing that makes a restore fail:
            // a file held open (the running app, an antivirus scan). The files after it
            // were then gone from both places.
            let _ = wipe_dir_contents(install_dir);
            let failed = restore_dir(&backup, install_dir);
            if failed == 0 {
                let _ = remove_path(&backup);
                Err(e)
            } else {
                Err(Error::Other(format!(
                    "{e}; the rollback could not restore {failed} file(s) — the previous \
                     install is kept intact at {}",
                    backup.display()
                )))
            }
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

/// Snapshot `src` into `dst`.
///
/// Symbolic links and junctions are neither followed nor copied: `wipe_dir_contents`
/// leaves them in place, so they need no restoring. Following one (`is_dir()` does) copied
/// whatever it pointed at — a mods library, a whole drive — into the snapshot, and a link
/// back to an ancestor never finished.
fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).map_err(|e| Error::io(dst, e))?;
    for entry in std::fs::read_dir(src).map_err(|e| Error::io(src, e))? {
        let entry = entry.map_err(|e| Error::io(src, e))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        // DirEntry::file_type does not traverse links; a junction reports as one on Windows.
        let kind = entry.file_type().map_err(|e| Error::io(&from, e))?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| Error::io(&from, e))?;
        }
    }
    Ok(())
}

/// Copy the snapshot back, CONTINUING past a file that cannot be written, and return how
/// many could not be. A restore that stops at its first failure leaves every later file
/// missing from the install directory, and they are then only in the snapshot.
fn restore_dir(src: &Path, dst: &Path) -> usize {
    let entries = match std::fs::create_dir_all(dst).and_then(|()| std::fs::read_dir(src)) {
        Ok(rd) => rd,
        Err(_) => return 1,
    };
    let mut failed = 0;
    for entry in entries {
        let Ok(entry) = entry else {
            failed += 1;
            continue;
        };
        let (from, to) = (entry.path(), dst.join(entry.file_name()));
        match entry.file_type() {
            Ok(t) if t.is_symlink() => {}
            Ok(t) if t.is_dir() => failed += restore_dir(&from, &to),
            Ok(_) => {
                if std::fs::copy(&from, &to).is_err() {
                    failed += 1;
                }
            }
            Err(_) => failed += 1,
        }
    }
    failed
}

/// Empty `dir` before a restore. Links and junctions stay: the snapshot did not copy them,
/// and removing one would lose the link itself for good.
fn wipe_dir_contents(dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))? {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let p = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_symlink() => {}
            Ok(t) if t.is_dir() => {
                let _ = std::fs::remove_dir_all(&p);
            }
            _ => {
                let _ = std::fs::remove_file(&p);
            }
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
        // Numeric, not lexical: "1.10.0" > "1.9.0" (string compare would get this wrong).
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(!is_newer("1.9.0", "1.10.0"));
        assert!(is_newer("1.0.10", "1.0.9"));
        // Trailing-zero components are equal, not newer ("1.2" == "1.2.0").
        assert!(!is_newer("1.2", "1.2.0"));
        assert!(!is_newer("1.2.0", "1.2"));
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

        // Update to v2 → success (no key pinned).
        apply_package_update(&p2, &install, None, None).unwrap();
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
        let err = apply_package_update(&bad, &install, None, None);
        assert!(err.is_err(), "corrupt update must fail");
        assert_eq!(
            std::fs::read(install.join("f.txt")).unwrap(),
            b"VERSION TWO!!",
            "rollback should restore the pre-update state"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn update_refuses_unsigned_package_when_key_pinned() {
        let base = std::env::temp_dir().join(format!("bpkg-sig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let src = base.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("f.txt"), b"payload").unwrap();

        // An UNSIGNED package (the signer closure returns None).
        let pkg = base.join("v.bpkg");
        crate::package::create_from_dir(&src, app(), vec![], |_| None, &pkg).unwrap();

        let install = base.join("install");
        std::fs::create_dir_all(&install).unwrap();

        // With a pinned key, an unsigned package must be REFUSED (fail closed) and the
        // install dir left untouched.
        let vk = crate::sign::generate().verifying_key();
        let res = apply_package_update(&pkg, &install, None, Some(&vk));
        assert!(
            res.is_err(),
            "unsigned package must be rejected when a key is pinned"
        );
        assert!(
            !install.join("f.txt").exists(),
            "nothing should be written when the signature gate fails"
        );

        // Without a pinned key, the same package still applies (legacy behaviour).
        apply_package_update(&pkg, &install, None, None).unwrap();
        assert!(install.join("f.txt").exists());

        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod offered_tests {
    use super::*;
    use crate::manifest::AppMeta;

    fn meta(id: &str, version: &str) -> AppMeta {
        AppMeta {
            id: id.into(),
            name: "App".into(),
            version: version.into(),
            publisher: "p".into(),
            homepage: None,
            platforms: vec![],
        }
    }

    fn offer(version: &str) -> UpdateManifest {
        UpdateManifest {
            version: version.into(),
            url: "https://example.invalid/app.bpkg".into(),
            urls: vec![],
            notes: None,
            deltas: vec![],
        }
    }

    /// A signed package built for `app`, and the key that signed it.
    fn signed(base: &Path, app: AppMeta) -> (Vec<u8>, ed25519_dalek::SigningKey) {
        let src = base.join(format!("src-{}", app.version));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("app.exe"), format!("build {}", app.version)).unwrap();
        let out = base.join(format!("{}-{}.bpkg", app.id, app.version));
        crate::package::create_from_dir(&src, app, vec![], |_| None, &out).unwrap();
        let sk = crate::sign::generate();
        crate::package::sign_package(&out, &sk).unwrap();
        (std::fs::read(&out).unwrap(), sk)
    }

    /// The attack: the manifest offers 1.3.0, a mirror serves an OLD release the same
    /// publisher genuinely signed. The signature verifies. It must still not install.
    #[test]
    fn a_genuinely_signed_older_release_is_not_an_update() {
        let base = std::env::temp_dir().join(format!("bpkg-rollback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let install = base.join("install");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("app.exe"), b"build 1.2.0").unwrap();

        let (old, sk) = signed(&base, meta("app", "1.0.0"));
        let vk = sk.verifying_key();
        let err = apply_downloaded(
            &old,
            &offer("1.3.0"),
            "1.2.0",
            &install,
            Some(&vk),
            Some("app"),
        )
        .expect_err("a downgrade to a signed 1.0.0 must be refused");
        assert!(err.to_string().contains("1.3.0"), "{err}");
        assert_eq!(
            std::fs::read(install.join("app.exe")).unwrap(),
            b"build 1.2.0"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_offered_version_of_this_app_and_only_that_is_accepted() {
        assert!(check_offered(&meta("app", "1.3.0"), "1.3.0", "1.2.0", Some("app")).is_ok());
        assert!(check_offered(&meta("app", "1.3"), "1.3.0", "1.2.0", Some("app")).is_ok());
        // Offered 1.3.0, got 1.2.5: newer than installed, still not what was offered.
        assert!(check_offered(&meta("app", "1.2.5"), "1.3.0", "1.2.0", Some("app")).is_err());
        // The installed release replayed (manifest lying about its version too).
        assert!(check_offered(&meta("app", "1.2.0"), "1.2.0", "1.2.0", Some("app")).is_err());
        // Another app by the same publisher.
        assert!(check_offered(&meta("other", "1.3.0"), "1.3.0", "1.2.0", Some("app")).is_err());
        // No expected id (the CLI): the version rules still hold.
        assert!(check_offered(&meta("other", "1.3.0"), "1.3.0", "1.2.0", None).is_ok());
        assert!(check_offered(&meta("app", "1.0.0"), "1.0.0", "1.2.0", None).is_err());
    }
}

#[cfg(test)]
mod mirror_tests {
    use super::download_any;

    // The HTTPS gate rejects a plain-http URL before any socket is opened, which makes it a
    // deterministic, offline stand-in for "this source failed" — enough to prove the loop
    // actually walks the mirrors instead of stopping at the primary.
    #[test]
    fn tries_every_mirror_and_reports_the_last_failure() {
        let err = download_any(
            "http://primary.invalid/a.bpkg",
            &[
                "http://one.invalid/a.bpkg".into(),
                "http://two.invalid/a.bpkg".into(),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("two.invalid"),
            "expected the LAST mirror's error, got: {err}"
        );
    }

    #[test]
    fn blank_mirrors_are_skipped() {
        let err = download_any("http://primary.invalid/a.bpkg", &["".into(), "   ".into()])
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("primary.invalid"),
            "expected the primary's error, got: {err}"
        );
    }

    #[test]
    fn no_mirrors_behaves_exactly_as_before() {
        let err = download_any("http://primary.invalid/a.bpkg", &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("primary.invalid"));
    }
}
