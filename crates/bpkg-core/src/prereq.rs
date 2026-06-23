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
