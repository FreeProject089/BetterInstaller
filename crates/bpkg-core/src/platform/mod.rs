//! Platform abstraction (v3 Addendum §B). The engine is written once against
//! `PlatformOps`; each OS provides a backend. Phase 1 implements path resolution
//! for every OS; the system-mutating operations (shortcuts, protocol handlers,
//! uninstaller registration, PATH) are stubbed and land in Phase 3.

use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::manifest::AppMeta;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// A shortcut to create (Start Menu / desktop / `.desktop` / alias).
#[derive(Debug, Clone)]
pub struct ShortcutSpec {
    pub name: String,
    pub target: PathBuf,
    pub icon: Option<PathBuf>,
    pub desktop: bool,
    pub start_menu: bool,
}

/// Entry shown in "Add/Remove Programs" (Windows) or equivalent.
#[derive(Debug, Clone)]
pub struct UninstallEntry {
    pub app: AppMeta,
    pub install_dir: PathBuf,
    pub uninstaller: PathBuf,
}

/// OS-specific operations. Backends only need to implement the path methods for
/// Phase 1; the mutating methods have default stubs that error cleanly until
/// Phase 3 wires them per platform.
pub trait PlatformOps {
    /// Human label for logs/UI, e.g. "Windows".
    fn name(&self) -> &'static str;

    /// Default per-machine install directory for `app`.
    fn default_install_dir(&self, app: &AppMeta) -> PathBuf;

    /// Per-user application data directory (where the handoff file is written).
    fn app_data_dir(&self, app: &AppMeta) -> PathBuf;

    fn create_shortcuts(&self, _spec: &ShortcutSpec) -> Result<()> {
        Err(Error::Other(
            "create_shortcuts: implemented in Phase 3".into(),
        ))
    }
    fn register_protocol(&self, _scheme: &str, _exe: &std::path::Path) -> Result<()> {
        Err(Error::Other(
            "register_protocol: implemented in Phase 3".into(),
        ))
    }
    fn register_uninstaller(&self, _entry: &UninstallEntry) -> Result<()> {
        Err(Error::Other(
            "register_uninstaller: implemented in Phase 3".into(),
        ))
    }
    fn add_to_path(&self, _dir: &std::path::Path) -> Result<()> {
        Err(Error::Other("add_to_path: implemented in Phase 3".into()))
    }

    // ── Reverse ops (uninstall). Default no-ops so unsupported platforms don't
    //    fail an uninstall over a step they never performed. ──
    fn remove_shortcuts(&self, _name: &str, _desktop: bool, _start_menu: bool) -> Result<()> {
        Ok(())
    }
    fn unregister_protocol(&self, _scheme: &str) -> Result<()> {
        Ok(())
    }
    fn unregister_uninstaller(&self, _app_id: &str) -> Result<()> {
        Ok(())
    }

    /// The directory `app_id` is currently installed in, if any (for maintenance
    /// mode). Default: not detected.
    fn installed_dir(&self, _app_id: &str) -> Option<PathBuf> {
        None
    }

    /// The currently-installed version string of `app_id`, if recorded (to decide
    /// whether an update is available). Default: unknown.
    fn installed_version(&self, _app_id: &str) -> Option<String> {
        None
    }
}

/// The backend for the OS this binary was built for.
pub fn current() -> Box<dyn PlatformOps> {
    #[cfg(target_os = "windows")]
    {
        Box::new(windows::WindowsOps)
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::LinuxOps)
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacOps)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        compile_error!("BetterInstaller supports Windows, Linux and macOS only")
    }
}

/// Helper for backends: read an env var into a PathBuf, with a fallback.
#[allow(dead_code)]
pub(crate) fn env_dir(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(fallback))
}
