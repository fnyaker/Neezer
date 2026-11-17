//! Covers: the encoded file, fetched on a lane and cached on disk, handed to
//! Android as a PATH (Android decodes it — copying RGBA across JNI costs more
//! than the platform decoder). Owned by **MB2**.

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
        let _ = size_px;
        Err(NzError::Unsupported {
            feature: format!("mb2:cover_path:{url}"),
        })
    }
}
