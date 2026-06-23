//! macOS backend. Phase 1: path resolution. `.app` bundle handling / Launch
//! Services registration land in Phase 3.

use std::path::PathBuf;

use super::{env_dir, PlatformOps};
use crate::manifest::AppMeta;

pub struct MacOps;

impl PlatformOps for MacOps {
    fn name(&self) -> &'static str {
        "macOS"
    }

    fn default_install_dir(&self, app: &AppMeta) -> PathBuf {
        // /Applications/<Name>.app
        PathBuf::from("/Applications").join(format!("{}.app", app.name))
    }

    fn app_data_dir(&self, app: &AppMeta) -> PathBuf {
        // ~/Library/Application Support/<id>
        env_dir("HOME", "/tmp")
            .join("Library/Application Support")
            .join(&app.id)
    }
}
