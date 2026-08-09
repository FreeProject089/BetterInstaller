//! macOS backend. Per-user locations (no root). Shortcuts are symlinks under
//! `~/Applications`. Protocol registration on macOS is driven by the `.app`
//! bundle's Info.plist (`CFBundleURLTypes`) rather than a runtime call, so
//! `register_protocol` is a best-effort no-op here.

use std::path::{Path, PathBuf};

use super::{env_dir, PlatformOps, ShortcutSpec, UninstallEntry};
use crate::error::{Error, Result};
use crate::manifest::AppMeta;

pub struct MacOps;

impl MacOps {
    fn applications_dir() -> PathBuf {
        env_dir("HOME", "/tmp").join("Applications")
    }
}

impl PlatformOps for MacOps {
    fn name(&self) -> &'static str {
        "macOS"
    }

    fn default_install_dir(&self, app: &AppMeta) -> PathBuf {
        PathBuf::from("/Applications").join(format!("{}.app", app.name))
    }

    fn app_data_dir(&self, app: &AppMeta) -> PathBuf {
        env_dir("HOME", "/tmp")
            .join("Library/Application Support")
            .join(&app.id)
    }

    fn create_shortcuts(&self, spec: &ShortcutSpec) -> Result<()> {
        // A symlink in ~/Applications is the simplest per-user "shortcut".
        if spec.start_menu || spec.desktop {
            let dir = Self::applications_dir();
            std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
            let link = dir.join(&spec.name);
            let _ = std::fs::remove_file(&link); // replace if present
            #[cfg(unix)]
            std::os::unix::fs::symlink(&spec.target, &link).map_err(|e| Error::io(&link, e))?;
        }
        Ok(())
    }

    fn register_protocol(&self, _scheme: &str, _exe: &Path) -> Result<()> {
        // Handled declaratively via the .app Info.plist CFBundleURLTypes.
        Ok(())
    }

    fn register_uninstaller(&self, entry: &UninstallEntry) -> Result<()> {
        // Uninstalling on macOS is "drag the .app to the Trash" and the engine deletes the
        // directory — but the installer still has to RECOGNISE an existing install to offer
        // Update / Repair at all, and there is no registry to ask. Hence a receipt.
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

    fn remove_shortcuts(&self, name: &str, _desktop: bool, _start_menu: bool) -> Result<()> {
        let _ = std::fs::remove_file(Self::applications_dir().join(name));
        Ok(())
    }
}
