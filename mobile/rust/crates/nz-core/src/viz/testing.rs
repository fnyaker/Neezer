//! What the visualiser's tests ask the core for: a throwaway data + cache
//! directory and a core over it, built the way `NzBridge.install` builds it.
//!
//! Its own and not `core::testing::TempDir` (MB2's, private to that module):
//! this area talks to no server at all — the calendar of this lot is "0 requests
//! for the bars and the scope, one for the sleeve and none once it is cached" —
//! so it needs no bench, only a store to read settings from.
//!
//! The directory lives under the crate's `target/`, never in `/tmp` (worker rule
//! 8: the machine's tmpfs is RAM), and it is removed on drop.

use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

/// One test's two roots, removed on drop.
pub struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-viz-tests")
            .join(format!("{}-{tag}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("data")).expect("data dir");
        std::fs::create_dir_all(path.join("cache")).expect("cache dir");
        TempDir { path }
    }

    /// A core over this directory, built the way the app builds it.
    pub fn core(&self, device: &str) -> crate::core::Core {
        crate::core::Core::new(crate::core::CoreConfig {
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
