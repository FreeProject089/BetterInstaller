//! Linux backend. Per-user XDG locations (no root). Shortcuts and protocol
//! handlers are `.desktop` files under `~/.local/share/applications`.

use std::path::{Path, PathBuf};

use super::{env_dir, PlatformOps, ShortcutSpec, UninstallEntry};
use crate::error::{Error, Result};
use crate::manifest::AppMeta;

pub struct LinuxOps;

impl LinuxOps {
    fn applications_dir() -> PathBuf {
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
            return PathBuf::from(xdg).join("applications");
        }
        env_dir("HOME", "/tmp").join(".local/share/applications")
    }

    fn desktop_dir() -> PathBuf {
        env_dir("HOME", "/tmp").join("Desktop")
    }

    fn desktop_entry(name: &str, exec: &Path, icon: Option<&Path>, scheme: Option<&str>) -> String {
        let mut s = String::from("[Desktop Entry]\nType=Application\nTerminal=false\n");
        s.push_str(&format!("Name={name}\n"));
        s.push_str(&format!("Exec=\"{}\" %u\n", exec.display()));
        if let Some(icon) = icon {
            s.push_str(&format!("Icon={}\n", icon.display()));
        }
        if let Some(scheme) = scheme {
            s.push_str(&format!("MimeType=x-scheme-handler/{scheme};\n"));
        }
        s
    }

    fn write_desktop(path: &Path, contents: &str) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        std::fs::write(path, contents).map_err(|e| Error::io(path, e))
    }
}

impl PlatformOps for LinuxOps {
    fn name(&self) -> &'static str {
        "Linux"
    }

    fn default_install_dir(&self, app: &AppMeta) -> PathBuf {
        env_dir("HOME", "/tmp").join(".local/opt").join(&app.id)
    }

    fn app_data_dir(&self, app: &AppMeta) -> PathBuf {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg).join(&app.id);
        }
        env_dir("HOME", "/tmp").join(".config").join(&app.id)
    }

    fn create_shortcuts(&self, spec: &ShortcutSpec) -> Result<()> {
        let entry = Self::desktop_entry(&spec.name, &spec.target, spec.icon.as_deref(), None);
        if spec.start_menu {
            Self::write_desktop(
                &Self::applications_dir().join(format!("{}.desktop", spec.name)),
                &entry,
            )?;
        }
        if spec.desktop {
            Self::write_desktop(
                &Self::desktop_dir().join(format!("{}.desktop", spec.name)),
                &entry,
            )?;
        }
        Ok(())
    }

    fn register_protocol(&self, scheme: &str, exe: &Path) -> Result<()> {
        // Drop a handler .desktop and register it as the default for the scheme.
        let id = format!("betterinstaller-{scheme}-handler");
        let path = Self::applications_dir().join(format!("{id}.desktop"));
        let entry = Self::desktop_entry(&id, exe, None, Some(scheme));
        Self::write_desktop(&path, &entry)?;
        // Best-effort: associate the scheme (ignore failure / missing xdg-mime).
        let _ = std::process::Command::new("xdg-mime")
            .args([
                "default",
                &format!("{id}.desktop"),
                &format!("x-scheme-handler/{scheme}"),
            ])
            .status();
        Ok(())
    }

    fn register_uninstaller(&self, entry: &UninstallEntry) -> Result<()> {
        // No ARP equivalent on Linux, so the install is recorded as a receipt instead —
        // without one, `installed_dir` and `installed_version` below have nothing to read
        // and every run looks like a first install.
        super::receipt::write(&entry.app, &entry.install_dir)
    }

    fn unregister_uninstaller(&self, app_id: &str) -> Result<()> {
        super::receipt::remove(app_id)
    }

    fn installed_dir(&self, app_id: &str) -> Option<PathBuf> {
        super::receipt::installed_dir(app_id)
    }

    fn installed_version(&self, app_id: &str) -> Option<String> {
        super::receipt::installed_version(app_id)
    }

    fn remove_shortcuts(&self, name: &str, desktop: bool, start_menu: bool) -> Result<()> {
        if start_menu {
            let _ = std::fs::remove_file(Self::applications_dir().join(format!("{name}.desktop")));
        }
        if desktop {
            let _ = std::fs::remove_file(Self::desktop_dir().join(format!("{name}.desktop")));
        }
        Ok(())
    }

    fn unregister_protocol(&self, scheme: &str) -> Result<()> {
        let id = format!("betterinstaller-{scheme}-handler");
        let _ = std::fs::remove_file(Self::applications_dir().join(format!("{id}.desktop")));
        Ok(())
    }
}
