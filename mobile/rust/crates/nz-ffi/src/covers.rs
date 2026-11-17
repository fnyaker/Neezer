//! Covers: the encoded file, fetched on a lane and cached on disk, handed to
//! Android as a PATH (Android decodes it — copying RGBA across JNI costs more
//! than the platform decoder). Owned by **MB2**.
//!
//! The whole rule — the CDN allow-list, the 300 MB LRU, the zero-request cache
//! hit, `size_px` — lives in `nz_core::core::covers`; this block is only the
//! boundary. Kotlin's Coil `Fetcher` calls `cover_path` with the URL it was
//! asked for and the pixels its view will draw, opens the returned file, and
//! decodes it with the platform decoder (`docs/agents/mobile.md`, *Caches*).

use crate::core::Screens;
use crate::errors::NzError;

#[uniffi::export(async_runtime = "tokio")]
impl Screens {
    /// The cached file for `url` at `size_px`, fetched on the `User` lane when a
    /// visible item asks for it and on `Prefetch` right after.
    ///
    /// The CDN allow-list of *Covers* applies: the configured origin or
    /// `*.dzcdn.net` over HTTPS, never a private IP literal, and a file name
    /// that is a hash — a URL never names a path.
    pub async fn cover_path(&self, url: String, size_px: u32) -> Result<String, NzError> {
        self.core()
            .cover_path(&url, size_px)
            .await
            .map_err(NzError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{CoreConfig, NzCore};
    use nz_core::core::CoreError;

    struct Dir(std::path::PathBuf);

    impl Dir {
        fn new(tag: &str) -> Dir {
            let path =
                std::env::temp_dir().join(format!("nz-ffi-covers-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("data")).expect("data");
            std::fs::create_dir_all(path.join("cache")).expect("cache");
            Dir(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn core(dir: &Dir) -> std::sync::Arc<NzCore> {
        NzCore::new(CoreConfig {
            data_dir: dir.0.join("data").display().to_string(),
            cache_dir: dir.0.join("cache").display().to_string(),
            app_version: "0.0.0".into(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".into(),
        })
        .expect("a core")
    }

    /// A hostile cover URL is refused THROUGH the boundary, as the French cause
    /// a `LoadFailed` can show — never a request to the person's own network.
    #[tokio::test]
    async fn a_hostile_cover_is_refused_with_a_cause() {
        let dir = Dir::new("refused");
        let core = core(&dir);
        for url in [
            "http://10.0.0.1/x.jpg",
            "https://evil.example/x.jpg",
            "file:///etc/passwd",
        ] {
            let error = core
                .screens()
                .cover_path(url.to_owned(), 128)
                .await
                .expect_err("refused");
            assert!(
                matches!(error, NzError::Invalid { .. }),
                "{url} → {error:?}"
            );
            assert!(!crate::errors::nz_error_text(&error).is_empty());
        }
        assert!(core.stats().is_empty(), "nothing was asked for");
        core.shutdown();
    }

    /// The core's own refusal type is what the boundary maps, and a refused URL
    /// opens no socket: the same guarantee, at the FFI level.
    #[test]
    fn the_core_error_maps_to_the_ffi_error() {
        assert!(matches!(
            NzError::from(CoreError::Offline),
            NzError::Offline
        ));
    }
}
