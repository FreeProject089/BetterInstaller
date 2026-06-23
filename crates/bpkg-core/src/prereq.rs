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
pub fn auto_install(p: &Prerequisite) -> crate::error::Result<()> {
    use crate::error::Error;
    let url = p
        .download_url
        .as_deref()
        .ok_or_else(|| Error::Other(format!("{}: no download_url", p.name)))?;

    let bytes = crate::net::download(url)?;
    let fname = url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("prereq-installer.exe");
    let path = std::env::temp_dir().join(fname);
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

/// Ensure every *required* prerequisite is present: auto-install the missing ones
/// that declare a `download_url`, re-check, and error on any still missing.
/// `on_step(name)` is called before each auto-install (for UI feedback).
pub fn ensure_required(
    prereqs: &[Prerequisite],
    mut on_step: impl FnMut(&str),
) -> crate::error::Result<()> {
    for p in prereqs.iter().filter(|p| p.required) {
        if check(p) {
            continue;
        }
        if p.download_url.is_some() {
            on_step(&p.name);
            auto_install(p)?;
            if !check(p) {
                return Err(crate::error::Error::Other(format!(
                    "{}: still missing after install",
                    p.name
                )));
            }
        } else {
            return Err(crate::error::Error::Other(format!(
                "missing prerequisite: {}",
                p.name
            )));
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
