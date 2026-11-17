//! Test support for this lot's own files: a throwaway directory that can look
//! like the old app's data directory, and a core over it.
//!
//! MB2's `core::testing` is private to `core` (its benches are its own), so a
//! feature lot builds what it needs in its own directory instead of widening
//! another lot's file.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::core::{Core, CoreConfig};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// One test's two roots (`data/`, `cache/`) under a directory that also plays
/// the app's `filesDir`, removed on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let number = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("nz-setup-{}-{tag}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("data")).expect("data dir");
        std::fs::create_dir_all(path.join("cache")).expect("cache dir");
        TempDir { path }
    }

    /// The app's `filesDir` (the old app's `offline/` and `shared_prefs/` live
    /// here, beside this app's `neezer/`).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A core over this directory, built the way `NzBridge.install` builds it.
    pub fn core(&self, device: &str) -> Core {
        Core::new(CoreConfig {
            data_dir: self.path.join("data"),
            cache_dir: self.path.join("cache"),
            app_version: "0.0.0-test".to_owned(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: device.to_owned(),
        })
        .expect("a core over a throwaway directory")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
