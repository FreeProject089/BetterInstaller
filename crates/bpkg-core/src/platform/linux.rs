//! Linux backend. Phase 1: XDG path resolution. `.desktop` files / xdg-mime /
//! AppImage integration land in Phase 3.

use std::path::PathBuf;

use super::{env_dir, PlatformOps};
use crate::manifest::AppMeta;

pub struct LinuxOps;

impl PlatformOps for LinuxOps {
    fn name(&self) -> &'static str {
        "Linux"
    }

    fn default_install_dir(&self, app: &AppMeta) -> PathBuf {
        // Per-user, no root needed: ~/.local/opt/<id>
        let home = env_dir("HOME", "/tmp");
        home.join(".local/opt").join(&app.id)
    }

    fn app_data_dir(&self, app: &AppMeta) -> PathBuf {
        // $XDG_CONFIG_HOME/<id>  (defaults to ~/.config/<id>)
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg).join(&app.id);
        }
        env_dir("HOME", "/tmp").join(".config").join(&app.id)
    }
}
