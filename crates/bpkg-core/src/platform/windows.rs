//! Windows backend. Per-user (HKCU / user profile) so nothing needs admin —
//! matching the installer's `asInvoker` manifest.

use std::path::{Path, PathBuf};

use winreg::enums::*;
use winreg::RegKey;

use super::{env_dir, PlatformOps, ShortcutSpec, UninstallEntry};
use crate::error::{Error, Result};
use crate::manifest::AppMeta;

pub struct WindowsOps;

impl WindowsOps {
    fn start_menu_programs() -> PathBuf {
        env_dir("APPDATA", "C:\\Users\\Default\\AppData\\Roaming")
            .join("Microsoft\\Windows\\Start Menu\\Programs")
    }

    fn desktop() -> PathBuf {
        env_dir("USERPROFILE", "C:\\Users\\Default").join("Desktop")
    }

    fn make_lnk(target: &Path, icon: Option<&Path>, lnk: &Path) -> Result<()> {
        if let Some(parent) = lnk.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let mut link = mslnk::ShellLink::new(target)
            .map_err(|e| Error::Other(format!("shortcut target: {e}")))?;
        if let Some(icon) = icon {
            link.set_icon_location(Some(icon.to_string_lossy().into_owned()));
        }
        link.create_lnk(lnk)
            .map_err(|e| Error::Other(format!("create .lnk: {e}")))
    }
}

impl PlatformOps for WindowsOps {
    fn name(&self) -> &'static str {
        "Windows"
    }

    fn default_install_dir(&self, app: &AppMeta) -> PathBuf {
        // Per-user, writable WITHOUT admin (matches the asInvoker manifest):
        // %LOCALAPPDATA%\Programs\<name>. Installing into Program Files would need
        // an elevated (requireAdministrator) build.
        env_dir("LOCALAPPDATA", "C:\\Users\\Default\\AppData\\Local")
            .join("Programs")
            .join(&app.name)
    }

    fn app_data_dir(&self, app: &AppMeta) -> PathBuf {
        env_dir("APPDATA", "C:\\Users\\Default\\AppData\\Roaming").join(&app.id)
    }

    fn create_shortcuts(&self, spec: &ShortcutSpec) -> Result<()> {
        let icon = spec.icon.as_deref();
        if spec.start_menu {
            let lnk = Self::start_menu_programs().join(format!("{}.lnk", spec.name));
            Self::make_lnk(&spec.target, icon, &lnk)?;
        }
        if spec.desktop {
            let lnk = Self::desktop().join(format!("{}.lnk", spec.name));
            Self::make_lnk(&spec.target, icon, &lnk)?;
        }
        Ok(())
    }

    fn register_protocol(&self, scheme: &str, exe: &Path) -> Result<()> {
        // HKCU\Software\Classes\<scheme> (per-user protocol handler)
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let base = format!("Software\\Classes\\{scheme}");
        let (key, _) = hkcu.create_subkey(&base).map_err(Error::IoBare)?;
        key.set_value("", &format!("URL:{scheme} Protocol"))
            .map_err(Error::IoBare)?;
        key.set_value("URL Protocol", &"").map_err(Error::IoBare)?;
        let (cmd, _) = hkcu
            .create_subkey(format!("{base}\\shell\\open\\command"))
            .map_err(Error::IoBare)?;
        cmd.set_value("", &format!("\"{}\" \"%1\"", exe.display()))
            .map_err(Error::IoBare)?;
        Ok(())
    }

    fn register_uninstaller(&self, entry: &UninstallEntry) -> Result<()> {
        // HKCU "Add/Remove Programs" entry (Apps & Features).
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = format!(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{}",
            entry.app.id
        );
        let (key, _) = hkcu.create_subkey(&path).map_err(Error::IoBare)?;
        key.set_value("DisplayName", &entry.app.name)
            .map_err(Error::IoBare)?;
        key.set_value("DisplayVersion", &entry.app.version)
            .map_err(Error::IoBare)?;
        key.set_value("Publisher", &entry.app.publisher)
            .map_err(Error::IoBare)?;
        key.set_value(
            "InstallLocation",
            &entry.install_dir.to_string_lossy().to_string(),
        )
        .map_err(Error::IoBare)?;
        key.set_value(
            "UninstallString",
            &format!("\"{}\" --uninstall", entry.uninstaller.display()),
        )
        .map_err(Error::IoBare)?;
        key.set_value("NoModify", &1u32).map_err(Error::IoBare)?;
        key.set_value("NoRepair", &1u32).map_err(Error::IoBare)?;
        Ok(())
    }

    fn remove_shortcuts(&self, name: &str, desktop: bool, start_menu: bool) -> Result<()> {
        if start_menu {
            let _ = std::fs::remove_file(Self::start_menu_programs().join(format!("{name}.lnk")));
        }
        if desktop {
            let _ = std::fs::remove_file(Self::desktop().join(format!("{name}.lnk")));
        }
        Ok(())
    }

    fn unregister_protocol(&self, scheme: &str) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let _ = hkcu.delete_subkey_all(format!("Software\\Classes\\{scheme}"));
        Ok(())
    }

    fn unregister_uninstaller(&self, app_id: &str) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let _ = hkcu.delete_subkey_all(format!(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{app_id}"
        ));
        Ok(())
    }

    fn installed_dir(&self, app_id: &str) -> Option<PathBuf> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu
            .open_subkey(format!(
                "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{app_id}"
            ))
            .ok()?;
        let loc: String = key.get_value("InstallLocation").ok()?;
        let p = PathBuf::from(loc);
        if p.exists() {
            Some(p)
        } else {
            None
        }
    }

    fn installed_version(&self, app_id: &str) -> Option<String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu
            .open_subkey(format!(
                "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{app_id}"
            ))
            .ok()?;
        key.get_value("DisplayVersion").ok()
    }

    fn add_to_path(&self, dir: &Path) -> Result<()> {
        // Append to the per-user PATH (HKCU\Environment).
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (env, _) = hkcu.create_subkey("Environment").map_err(Error::IoBare)?;
        let current: String = env.get_value("Path").unwrap_or_default();
        let dir_s = dir.to_string_lossy();
        if current.split(';').any(|p| p.eq_ignore_ascii_case(&dir_s)) {
            return Ok(()); // already present
        }
        let next = if current.is_empty() {
            dir_s.into_owned()
        } else {
            format!("{};{}", current.trim_end_matches(';'), dir_s)
        };
        env.set_value("Path", &next).map_err(Error::IoBare)?;
        Ok(())
    }
}
