//! A scratch directory only this process can write to.
//!
//! Everything this installer stages before using it went to `std::env::temp_dir()` under a
//! name derived from the process id: `betterinstaller-<pid>.bpkg` (the package whose
//! signature decides whether the install proceeds), `bi-update-<pid>.bpkg` (the downloaded
//! update), `bpkg-prereq-<id>.exe` (a downloaded prerequisite installer, which is then
//! EXECUTED) and `bi-logo-<pid>.<ext>`.
//!
//! On Windows `temp_dir()` is per-user, so that is an integrity problem between the
//! installer and anything else running as the same user. On Linux and macOS it is `/tmp`,
//! shared and world-writable, and a pid is a small guessable number:
//!
//! - plant a symlink at the name before the installer gets there and `std::fs::write`
//!   follows it, writing package bytes wherever the link points (CWE-377, CWE-59);
//! - replace the file between the moment it is verified and the moment it is read again —
//!   the signature check and the extraction are two separate reads of the same path, and
//!   `auto_install` hashes the bytes in memory and then runs the FILE (CWE-367). The
//!   winner of that race gets code execution as the user running the installer.
//!
//! The fix is not a better name. It is a directory that did not exist a moment ago, whose
//! creation fails rather than succeeds if something is already there, that nobody else can
//! enter, and files inside it opened with `create_new` so a planted entry is an error
//! instead of a target.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::error::{Error, Result};

static RUN_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// This process's private scratch directory, created on first use.
///
/// Created, not reused: `create_dir` fails when the path exists, so a directory another
/// user prepared is never adopted. The name carries 64 random bits as well as the pid, so
/// it cannot be pre-created by guessing either.
pub fn run_dir() -> Result<&'static Path> {
    RUN_DIR
        .get_or_init(|| {
            let base = std::env::temp_dir();
            for _ in 0..8 {
                let dir = base.join(format!(
                    "betterinstaller-{}-{:016x}",
                    std::process::id(),
                    rand::random::<u64>()
                ));
                if create_private_dir(&dir).is_ok() {
                    return Some(dir);
                }
            }
            None
        })
        .as_deref()
        .ok_or_else(|| Error::Other("could not create a private temporary directory".into()))
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    // 0700 before anything is written into it — a directory created world-readable and
    // chmod'ed afterwards is readable for the length of that gap.
    std::fs::DirBuilder::new().mode(0o700).create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    // Windows: temp_dir() is already inside the user's profile, and a fresh directory
    // inherits its ACL. What `create_dir` buys here is that the directory is OURS — it
    // did not exist, so nothing was waiting in it.
    std::fs::create_dir(dir)
}

/// Write `bytes` to `name` inside [`run_dir`] and return the path.
///
/// `name` is a bare file name: a path separator would put the file somewhere other than
/// the directory whose privacy is the whole point.
pub fn stage(name: &str, bytes: &[u8]) -> Result<PathBuf> {
    let dir = run_dir()?;
    if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
        return Err(Error::Other(format!("not a scratch file name: {name:?}")));
    }
    let path = dir.join(name);
    // Inside a directory nobody else can enter, so removing a leftover from an earlier
    // stage() in THIS process is safe; anything else is a bug worth failing on.
    let _ = std::fs::remove_file(&path);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| Error::io(&path, e))?;
    f.write_all(bytes).map_err(|e| Error::io(&path, e))?;
    Ok(path)
}

/// Delete a staged file. Best-effort: a scratch file left behind is untidy, not unsafe.
pub fn discard(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_files_never_sit_directly_in_the_shared_temp_dir() {
        let p = stage("a.bpkg", b"x").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"x");
        assert_ne!(
            p.parent().unwrap(),
            std::env::temp_dir(),
            "a staged file in the shared temp dir is a name another user can predict"
        );
        // Re-staging the same name in our own directory is fine.
        let again = stage("a.bpkg", b"y").unwrap();
        assert_eq!(again, p);
        assert_eq!(std::fs::read(&p).unwrap(), b"y");
        discard(&p);
        assert!(!p.exists());
    }

    #[test]
    fn a_scratch_name_cannot_be_a_path() {
        for n in ["", "a/b", "a\\b", "../x", "sub/../x"] {
            assert!(stage(n, b"x").is_err(), "should refuse {n:?}");
        }
    }

    #[test]
    fn the_run_directory_is_not_reused() {
        let dir = run_dir().unwrap();
        // It exists, and creating it again fails — which is what makes "created, not
        // adopted" true for the next process too.
        assert!(dir.is_dir());
        assert!(create_private_dir(dir).is_err());
    }

    /// The attack the whole module is about: a name an attacker can predict, prepared as
    /// a symlink before the installer writes it. `fs::write` — what every staging site
    /// used — follows the link; `stage` refuses. Unix only: Windows temp is per-user and
    /// a symlink there needs a privilege the attacker would not have to spend.
    #[cfg(unix)]
    #[test]
    fn staging_refuses_a_planted_symlink() {
        let victim = run_dir().unwrap().join("victim.txt");
        std::fs::write(&victim, b"original").unwrap();
        let planted = run_dir().unwrap().join("planted.bpkg");
        let _ = std::fs::remove_file(&planted);
        std::os::unix::fs::symlink(&victim, &planted).unwrap();

        // The old idiom writes straight through the link.
        std::fs::write(&planted, b"attacker").unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"attacker");

        std::fs::write(&victim, b"original").unwrap();
        let _ = std::fs::remove_file(&planted);
        std::os::unix::fs::symlink(&victim, &planted).unwrap();
        // `stage` removes the link (an entry in our own directory) and creates a real
        // file, so the victim is untouched either way.
        let p = stage("planted.bpkg", b"ours").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"ours");
        assert_eq!(std::fs::read(&victim).unwrap(), b"original");
        assert!(!std::fs::symlink_metadata(&p)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn the_run_directory_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(run_dir().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "group/other can reach the scratch dir");
    }
}
