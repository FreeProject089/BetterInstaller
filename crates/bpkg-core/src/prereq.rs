//! Prerequisite detection (Phase: prerequisites). A prereq is "present" when its
//! configured registry key / file / command check passes. If none is configured,
//! it's assumed present. Auto-download/-install of missing prereqs is left to the
//! installer UI (it has the network + UAC story).

use crate::config::Prerequisite;

/// Result of checking one prerequisite.
#[derive(Debug, Clone)]
pub struct PrereqStatus {
    pub id: String,
    pub name: String,
    pub present: bool,
    pub required: bool,
}

/// Check a single prerequisite.
pub fn check(p: &Prerequisite) -> bool {
    if let Some(key) = &p.check_registry {
        return registry_key_exists(key);
    }
    if let Some(file) = &p.check_file {
        return std::path::Path::new(file).exists();
    }
    if let Some(cmd) = &p.check_command {
        return command_on_path(cmd);
    }
    true // nothing to check → consider satisfied
}

/// Check all prerequisites, returning their statuses.
pub fn check_all(prereqs: &[Prerequisite]) -> Vec<PrereqStatus> {
    prereqs
        .iter()
        .map(|p| PrereqStatus {
            id: p.id.clone(),
            name: p.name.clone(),
            present: check(p),
            required: p.required,
        })
        .collect()
}

/// Whether all *required* prerequisites are present.
pub fn all_required_present(prereqs: &[Prerequisite]) -> bool {
    prereqs.iter().all(|p| !p.required || check(p))
}

/// Download and silently run a prerequisite's installer (`download_url` +
/// `silent_args`). Blocks until it finishes. The downloaded installer may
/// trigger its own UAC prompt — that's the prerequisite's behaviour.
/// `install_dir` is where a `Zip` prerequisite is unpacked, relative to `install_to`. An
/// `Exe` one ignores it — it goes wherever its own installer decides, which is precisely
/// why Zip is the form to prefer.
pub fn auto_install(p: &Prerequisite, install_dir: &std::path::Path) -> crate::error::Result<()> {
    use crate::error::Error;
    let url = p
        .download_url
        .as_deref()
        .ok_or_else(|| Error::Other(format!("{}: no download_url", p.name)))?;

    // Validated BEFORE the request goes out. A prerequisite that cannot be verified must
    // not be downloaded at all, rather than downloaded and then rejected — and validate()
    // is what makes sha256 mandatory once download_url is set.
    p.validate().map_err(Error::Other)?;

    let bytes = crate::net::download(url)?;

    // The hash is the whole point of this function being trustworthy. Until now it wrote
    // whatever the server returned to temp and executed it: HTTPS proves who answered,
    // not what they sent, so a compromised or repurposed upstream URL meant arbitrary
    // code execution with the installer's rights. Compared case-insensitively because a
    // hash pasted from a vendor page is as often uppercase as not.
    let expected = p
        .sha256
        .as_deref()
        .ok_or_else(|| Error::Other(format!("{}: no sha256", p.name)))?;
    let actual = sha256_hex(&bytes);
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(Error::Other(format!(
            "{}: downloaded file does not match its sha256 (expected {}, got {}) — refusing to run it",
            p.name, expected, actual
        )));
    }

    // A zip is unpacked, never run. Handled inside this function rather than as a separate
    // entry point so there is exactly one place a prerequisite's bytes are verified — a
    // second install_zip() would be a second door somebody eventually opens without the
    // hash check above.
    if p.kind == crate::config::PrereqKind::Zip {
        let rel = p.install_to.as_deref().ok_or_else(|| {
            Error::Other(format!("{}: a zip prerequisite needs install_to", p.name))
        })?;
        extract_zip(&bytes, &install_dir.join(rel))?;
        return Ok(());
    }

    // The filename comes from the URL, so it is attacker-influenced if the URL ever is.
    // Only its extension is used, and the name is fixed — a path segment in the last
    // component would otherwise write outside the temp directory.
    let ext = if url.to_ascii_lowercase().ends_with(".msi") {
        "msi"
    } else {
        "exe"
    };
    let path = std::env::temp_dir().join(format!("bpkg-prereq-{}.{}", sanitise_id(&p.id), ext));
    std::fs::write(&path, &bytes).map_err(|e| Error::io(&path, e))?;

    let mut cmd = std::process::Command::new(&path);
    if let Some(args) = &p.silent_args {
        cmd.args(args.split_whitespace());
    }
    let status = cmd.status().map_err(|e| Error::io(&path, e));
    let _ = std::fs::remove_file(&path);
    let status = status?;
    if !status.success() {
        return Err(Error::Other(format!(
            "{}: installer exited with {}",
            p.name, status
        )));
    }
    Ok(())
}

/// Prerequisites the user may choose to install, as opposed to those that are simply
/// required. Only the MISSING ones — offering to install something already present is
/// noise, and ticking it would download 10 MB to no effect.
pub fn optional_missing(prereqs: &[Prerequisite]) -> Vec<&Prerequisite> {
    prereqs
        .iter()
        .filter(|p| !p.required && p.download_url.is_some() && !check(p))
        .collect()
}

/// Ensure prerequisites are satisfied: auto-install the missing ones that declare a
/// `download_url`, re-check, and error on any REQUIRED one still missing.
///
/// `opted_in` carries the ids of optional prerequisites the user ticked. They are
/// installed like required ones, with one deliberate difference: a failure is not fatal.
/// Something the user asked for as an extra should not abort an install of the app they
/// actually came for — they are told, and the install continues.
///
/// `on_step(name)` is called before each auto-install (for UI feedback).
pub fn ensure_required(
    prereqs: &[Prerequisite],
    install_dir: &std::path::Path,
    opted_in: &[String],
    mut on_step: impl FnMut(&str),
) -> crate::error::Result<()> {
    for p in prereqs {
        let wanted = p.required || opted_in.iter().any(|id| *id == p.id);
        if !wanted || check(p) {
            continue;
        }
        if p.download_url.is_none() {
            if p.required {
                return Err(crate::error::Error::Other(format!(
                    "missing prerequisite: {}",
                    p.name
                )));
            }
            continue;
        }
        on_step(&p.name);
        let outcome = auto_install(p, install_dir).and_then(|()| {
            if check(p) {
                Ok(())
            } else {
                Err(crate::error::Error::Other(format!(
                    "{}: still missing after install",
                    p.name
                )))
            }
        });
        if let Err(e) = outcome {
            // Required: the app cannot run without it, so stop. Optional: it is an extra,
            // and losing the whole install over an extra is the wrong trade.
            if p.required {
                return Err(e);
            }
        }
    }
    Ok(())
}

/// Is `cmd` resolvable on PATH? (`<cmd>` or `<cmd>.exe` on Windows.)
fn command_on_path(cmd: &str) -> bool {
    let path = match std::env::var_os("PATH") {
        Some(p) => p,
        None => return false,
    };
    let exts: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    std::env::split_paths(&path).any(|dir| {
        exts.iter()
            .any(|ext| dir.join(format!("{cmd}{ext}")).is_file())
    })
}

#[cfg(windows)]
fn registry_key_exists(key: &str) -> bool {
    use winreg::enums::*;
    use winreg::RegKey;
    let (root, sub) = match key.split_once('\\') {
        Some((r, s)) => (r, s),
        None => return false,
    };
    let hive = match root.to_ascii_uppercase().as_str() {
        "HKLM" | "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
        "HKCU" | "HKEY_CURRENT_USER" => HKEY_CURRENT_USER,
        "HKCR" | "HKEY_CLASSES_ROOT" => HKEY_CLASSES_ROOT,
        _ => return false,
    };
    RegKey::predef(hive).open_subkey(sub).is_ok()
}

#[cfg(not(windows))]
fn registry_key_exists(_key: &str) -> bool {
    false // no registry off Windows
}

/// SHA-256 of some bytes, lowercase hex.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// An id reduced to characters that cannot be a path. Used for the temp filename, so a
/// prerequisite id can never contribute a separator or a `..` to where the download lands.
fn sanitise_id(id: &str) -> String {
    let s: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if s.is_empty() {
        "prereq".into()
    } else {
        s
    }
}

/// Unpack a downloaded zip prerequisite into `<install_dir>/<install_to>`.
///
/// Zip-slip is checked PER ENTRY, which is a different question from the one
/// `Prerequisite::validate` answers. That one decides where the archive is allowed to go;
/// this one decides whether the archive is allowed to put its contents there. An archive
/// whose entries are named `../../../Windows/System32/x.dll` escapes a destination that
/// was itself perfectly legal (CWE-22), and the name is chosen by whoever built the zip.
///
/// Every path is rebuilt from its own components rather than trusted: absolute roots,
/// drive prefixes and `..` are dropped, and the result is required to still be under the
/// destination afterwards. Belt and braces on purpose — the component filter is what
/// stops the escape, and the final containment check is what catches a form of escape the
/// filter did not anticipate.
fn extract_zip(bytes: &[u8], dest: &std::path::Path) -> crate::error::Result<usize> {
    use crate::error::Error;
    use std::io::{Cursor, Read};

    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| Error::Other(format!("not a readable zip: {e}")))?;
    std::fs::create_dir_all(dest).map_err(|e| Error::io(dest, e))?;
    // Resolved once, so containment is compared between two real paths.
    let root = dest.canonicalize().map_err(|e| Error::io(dest, e))?;

    let mut written = 0usize;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| Error::Other(format!("zip entry {i}: {e}")))?;

        // `enclosed_name` is the zip crate's own refusal to hand back a path that escapes;
        // it returns None for absolute paths, `..` and Windows drive prefixes. Checked
        // rather than unwrapped, so a hostile entry is an error and not a panic.
        let rel = match entry.enclosed_name() {
            Some(p) => p,
            None => {
                return Err(Error::Other(format!(
                    "zip entry {:?} tries to escape the destination — refusing the archive",
                    entry.name()
                )))
            }
        };

        let out = root.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| Error::io(&out, e))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
            // The second belt: after creating the parent it can be resolved, so an escape
            // that survived enclosed_name — a symlink already on disk, say — still fails
            // before anything is written.
            let real = parent.canonicalize().map_err(|e| Error::io(parent, e))?;
            if !real.starts_with(&root) {
                return Err(Error::Other(format!(
                    "zip entry {:?} resolves outside the destination — refusing the archive",
                    entry.name()
                )));
            }
        }

        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut buf)
            .map_err(|e| Error::Other(format!("reading zip entry {:?}: {e}", entry.name())))?;
        std::fs::write(&out, &buf).map_err(|e| Error::io(&out, e))?;
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod zip_tests {
    use super::*;
    use std::io::Write;

    /// Build a zip in memory whose entries are named exactly as given — including names a
    /// well-behaved archiver would never produce. That is the point: the archive is built
    /// by whoever hosts the download, not by us.
    fn zip_with(names: &[&str]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default();
            for n in names {
                w.start_file(*n, opts).unwrap();
                w.write_all(b"payload").unwrap();
            }
            w.finish().unwrap();
        }
        buf
    }

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bpkg-ziptest-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_normal_archive_extracts() {
        let dir = tmpdir("ok");
        let dest = dir.join("runtime/python");
        let n = extract_zip(&zip_with(&["python.exe", "lib/os.py"]), &dest).unwrap();
        assert_eq!(n, 2);
        assert!(dest.join("python.exe").is_file());
        assert!(
            dest.join("lib/os.py").is_file(),
            "nested entries get their parent"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Zip-slip. This is a different question from where the archive is ALLOWED to go:
    /// the destination here is perfectly legal, and the escape is in the entry names,
    /// which are chosen by whoever built the archive (CWE-22).
    #[test]
    fn an_archive_that_escapes_its_destination_is_refused_whole() {
        for evil in [
            "../escaped.txt",
            "../../escaped.txt",
            "runtime/../../escaped.txt",
            "/abs/escaped.txt",
        ] {
            let dir = tmpdir("evil");
            let dest = dir.join("runtime/python");
            let res = extract_zip(&zip_with(&["good.txt", evil]), &dest);
            assert!(res.is_err(), "entry {evil:?} was accepted");
            // Nothing may be left outside the destination, whatever the archive claimed.
            assert!(
                !dir.join("escaped.txt").exists(),
                "{evil:?} wrote outside dest"
            );
            assert!(!dir.join("../escaped.txt").exists());
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_file_that_is_not_a_zip_is_an_error_not_a_panic() {
        let dir = tmpdir("bad");
        assert!(extract_zip(b"this is not a zip", &dir.join("x")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod opt_in_tests {
    use super::*;
    use crate::config::{PrereqKind, Prerequisite};

    fn p(id: &str, required: bool, url: Option<&str>) -> Prerequisite {
        Prerequisite {
            id: id.into(),
            name: id.into(),
            check_registry: None,
            check_file: None,
            // A command that certainly does not exist, so `check` reports it missing and
            // the selection logic is what decides, not the machine running the test.
            check_command: Some("definitely-not-a-real-binary-xyz".into()),
            download_url: url.map(String::from),
            sha256: Some("a".repeat(64)),
            kind: PrereqKind::Exe,
            install_to: None,
            silent_args: None,
            required,
        }
    }

    #[test]
    fn only_missing_optionals_with_a_download_are_offered() {
        let list = vec![
            p("python", false, Some("https://e.com/x.exe")),
            p("vcredist", true, Some("https://e.com/v.exe")), // required: not an offer
            p("manual", false, None),                         // nothing to download
        ];
        let offered: Vec<&str> = optional_missing(&list)
            .iter()
            .map(|p| p.id.as_str())
            .collect();
        assert_eq!(offered, vec!["python"]);
    }

    #[test]
    fn an_optional_nobody_ticked_is_not_installed() {
        // The whole point of the opt-in. If this regressed, every install would start
        // fetching megabytes nobody asked for — and the test would still pass if it only
        // checked that ensure_required returned Ok.
        let list = vec![p("python", false, Some("https://127.0.0.1:1/x.exe"))];
        let dir = std::env::temp_dir();
        // Assert on on_step, NOT on the return value. An earlier version of this test
        // checked only that it returned Ok — and that passes even when the opt-in is
        // ignored entirely, because the download then fails and an optional failure is
        // deliberately swallowed. on_step firing is the observable proof that an install
        // was ATTEMPTED, which is the thing that must not happen.
        let mut attempted = Vec::new();
        assert!(ensure_required(&list, &dir, &[], |n| attempted.push(n.to_string())).is_ok());
        assert!(
            attempted.is_empty(),
            "an unticked optional was installed anyway: {attempted:?}"
        );
    }

    #[test]
    fn a_failed_optional_does_not_abort_the_install() {
        // Port 1 on loopback: refused instantly rather than left to time out. An
        // unroutable address works too but costs ten seconds per test, every CI run.
        let list = vec![p("python", false, Some("https://127.0.0.1:1/x.exe"))];
        let mut announced = Vec::new();
        let r = ensure_required(&list, &std::env::temp_dir(), &["python".to_string()], |n| {
            announced.push(n.to_string())
        });
        assert!(
            r.is_ok(),
            "an optional extra failing must not lose the whole install"
        );
        assert_eq!(
            announced,
            vec!["python"],
            "the user should still be told it was tried"
        );
    }

    #[test]
    fn a_failed_required_still_aborts() {
        // The counterpart. Optional being forgiving must not have made required forgiving.
        let list = vec![p("vcredist", true, Some("https://127.0.0.1:1/v.exe"))];
        assert!(ensure_required(&list, &std::env::temp_dir(), &[], |_| {}).is_err());
    }
}
