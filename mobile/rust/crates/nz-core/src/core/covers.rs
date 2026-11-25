//! Cover art: the encoded file, fetched on a lane, cached on disk, handed to
//! Android as a PATH. Owned by **MB2**.
//!
//! Rust fetches and keeps the bytes; **Android decodes** (`docs/agents/mobile.md`,
//! *Caches*): copying RGBA across JNI costs more than the platform decoder, and
//! Coil already downsamples to the view's size with hardware bitmaps. So the
//! answer here is a file name, and its cache is `nz-cache`'s 300 MB LRU — the
//! same store the desktop paints from.
//!
//! # The allow-list (`CarCovers`' rule)
//!
//! A cover URL comes from a server payload, so it is hostile until checked: the
//! app sends a request to the **configured origin** (its `/api/cover/<id>`,
//! carrying the session) or to `https://*.dzcdn.net` (the provider's CDN, no
//! cookie), and to nothing else. A private IP literal is refused on the CDN
//! branch — an album row naming `http://10.0.0.1/x.jpg` must not turn the phone
//! into a probe of the person's own network. A refused URL opens **no socket**
//! and says so in French (`Invalid`), which is what the cover acceptance counts.

use std::sync::atomic::AtomicBool;

use nz_api::Lane;

use super::{Core, CoreError, TAG_COVER, Url};

/// The cover cache's cap, in bytes — `docs/agents/desktop.md`, *Caches*.
/// `nz-cache`'s own default; named here because the phone evicts inline (there
/// is no `Bulk` worker yet to do it).
pub const COVER_CAP_BYTES: i64 = nz_cache::covers::DEFAULT_CAP_BYTES;

/// The provider's CDN. The SPA's `CarCovers` allow-list, unchanged: the
/// configured origin or these hosts over HTTPS, nothing else.
const DZCDN: &str = "dzcdn.net";

/// Where one cover may be asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CoverRoute {
    /// Our own server: an API path without its `/api` prefix (`/cover/42?x=1`),
    /// asked with the session cookie.
    Api(String),
    /// The provider's CDN: an absolute HTTPS URL, asked with no cookie.
    External,
}

impl Core {
    /// The cached file for `url`, fetching it if needed.
    ///
    /// `size_px` is what the VIEW will draw the cover at: Rust does not resize
    /// (`nz-cache`'s `variant` would, and the note says Android decodes), so the
    /// value travels in the journal and in Coil's cache key. A cover scrolled
    /// away cancels the UniFFI future, which drops the `await` before anything is
    /// written — every step below is cancellation-safe.
    pub async fn cover_path(&self, url: &str, size_px: u32) -> Result<String, CoreError> {
        let url = url.trim();
        if url.is_empty() {
            return Err(CoreError::Invalid {
                detail: "Pochette sans adresse".to_owned(),
            });
        }
        // The server names its own sleeves by a ROOT-RELATIVE path
        // (`/api/cover/<album>`, the SPA's convention, in every payload a screen
        // draws). It is the configured origin's by definition, so it is made
        // absolute against that origin — which also keys the disk cache per
        // server. Measured on v1.6.6 against perf_api: every home card handed
        // `/api/cover/…` here, the allow-list refused it as "not an origin",
        // and no cover was ever requested. `//host/…` is protocol-relative,
        // another host, and stays refused below.
        let absolute;
        let url = if url.starts_with('/') && !url.starts_with("//") {
            let origin = self.origin().ok_or_else(refused)?;
            absolute = format!("{}{url}", origin.as_str());
            absolute.as_str()
        } else {
            url
        };
        // A hit costs no request at all, which is what a scrolling list needs.
        if let Some(path) = self.cache().covers().get(url) {
            tracing::trace!(size_px, path = %path.display(), "cover hit");
            return Ok(path.display().to_string());
        }

        let bytes = match self.cover_route(url)? {
            CoverRoute::Api(path) => {
                // Our own server: the session cookie goes with it, on the lane
                // the visible item asked for.
                let api = self.api()?;
                api.bytes(Lane::User, &path).await?
            }
            CoverRoute::External => self.fetch_external(url).await?,
        };
        self.meter().record(TAG_COVER, bytes.len() as u64, 0);

        let path = self.cache().covers().put(url, &bytes)?;
        self.evict_covers()?;
        tracing::debug!(
            size_px,
            bytes = bytes.len(),
            path = %path.display(),
            "cover stored"
        );
        Ok(path.display().to_string())
    }

    /// Where a cover URL may be asked for, or the French cause why not.
    fn cover_route(&self, url: &str) -> Result<CoverRoute, CoreError> {
        let parsed = Url::parse(url).ok_or_else(refused)?;
        if parsed.userinfo {
            return Err(refused());
        }
        if let Some(origin) = self.origin()
            && parsed.origin() == origin
        {
            // Our server: `/api/cover/<id>` is the only cover route, and the
            // client adds `/api` back itself.
            let Some(rest) = parsed.path.strip_prefix("/api") else {
                return Err(refused());
            };
            if !rest.starts_with('/') {
                return Err(refused());
            }
            return Ok(CoverRoute::Api(format!("{rest}{}", query_suffix(&parsed))));
        }
        // The provider's CDN, over HTTPS only, and never an IP literal.
        if parsed.scheme == "https"
            && parsed.host_is_or_under(DZCDN)
            && !parsed.origin().host_is_ip_literal()
        {
            return Ok(CoverRoute::External);
        }
        Err(refused())
    }

    /// One cover from a third-party HTTPS host: no cookie (a session never
    /// leaves the server it belongs to), redirects followed by `nz-api`, and the
    /// body written to a `.part` file it renames when whole.
    async fn fetch_external(&self, url: &str) -> Result<Vec<u8>, CoreError> {
        let api = self.api()?;
        let dir = self.cache().covers_dir();
        self.ensure_dir(&dir)?;
        // One name per request, so two grids asking at once never collide.
        let destination = dir.join(format!(
            "fetch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.subsec_nanos())
                .unwrap_or(0)
        ));
        let cancel = AtomicBool::new(false);
        let report = api
            .download_url_to(Lane::User, url, &destination, |_, _| {}, &cancel)
            .await?;
        let bytes = std::fs::read(&destination).map_err(|error| CoreError::Internal {
            detail: format!("pochette lue : {error}"),
        })?;
        let _ = std::fs::remove_file(&destination);
        if bytes.is_empty() {
            return Err(CoreError::Internal {
                detail: format!("pochette vide ({} octets annoncés)", report.bytes),
            });
        }
        Ok(bytes)
    }

    /// Keeps the cover directory inside its cap. Called after a store, so the
    /// list a person scrolls never pushes the disk past 300 MB.
    fn evict_covers(&self) -> Result<(), CoreError> {
        let covers = self.cache().covers();
        if covers.total_bytes()? > COVER_CAP_BYTES {
            let removed = covers.evict_to(COVER_CAP_BYTES)?;
            tracing::debug!(removed, "covers evicted");
        }
        Ok(())
    }
}

fn refused() -> CoreError {
    CoreError::Invalid {
        detail: "Adresse de pochette refusée".to_owned(),
    }
}

/// `?query` as it came, or an empty string.
fn query_suffix(url: &Url) -> String {
    match &url.query {
        Some(query) => format!("?{query}"),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::testing::{Bench, TempDir};

    /// The allow-list, byte for byte: our origin, `*.dzcdn.net` over HTTPS, and
    /// nothing else — including a private address that would probe the LAN.
    #[tokio::test]
    async fn the_allow_list_refuses_everything_else() {
        let bench = Bench::start(false);
        let dir = TempDir::new("covers-allow");
        let core = dir.core("test");
        core.set_server(&bench.base()).await.expect("a server");
        // A LAN install is a valid origin too (HTTP to the configured host).
        core.set_server("http://192.168.1.10:5000")
            .await
            .expect_err("no server there, but the origin is stored");

        // Ours, with the session: an API path without its `/api`.
        assert_eq!(
            core.cover_route("http://192.168.1.10:5000/api/cover/42?size=big"),
            Ok(CoverRoute::Api("/cover/42?size=big".to_owned()))
        );

        // The CDN, no cookie, HTTPS only.
        assert_eq!(
            core.cover_route("https://e-cdns-images.dzcdn.net/images/cover/ab/264x264.jpg"),
            Ok(CoverRoute::External)
        );
        assert!(
            core.cover_route("http://e-cdns-images.dzcdn.net/x.jpg")
                .is_err(),
            "cleartext never leaves for a CDN"
        );

        // Everything hostile, refused before a socket exists.
        for bad in [
            "http://10.0.0.1/x.jpg",
            "https://evil.example/x.jpg",
            "https://dzcdn.net.evil.example/x.jpg",
            "https://evil-dzcdn.net/x.jpg",
            "file:///etc/passwd",
            "/api/cover/1",
            "https://user:pw@e-cdns-images.dzcdn.net/x.jpg",
            "",
        ] {
            let error = core.cover_route(bad).expect_err(bad);
            assert!(matches!(error, CoreError::Invalid { .. }), "{bad}");
        }
    }

    /// A refused cover opens NO socket and records NO request: the acceptance's
    /// "0 sockets" for the cover path.
    #[tokio::test]
    async fn a_refused_cover_costs_nothing_at_all() {
        let bench = Bench::start(false);
        let dir = TempDir::new("covers-refused");
        let core = dir.core("test");
        core.set_server(&bench.base()).await.expect("server");
        let before = core.stats().iter().map(|row| row.requests).sum::<u64>();
        let error = core
            .cover_path("http://10.0.0.1/x.jpg", 128)
            .await
            .expect_err("refused");
        assert!(matches!(error, CoreError::Invalid { .. }), "{error:?}");
        assert!(
            core.stats().iter().all(|row| row.tag != TAG_COVER),
            "a refused cover records no request"
        );
        assert_eq!(
            core.stats().iter().map(|row| row.requests).sum::<u64>(),
            before,
            "nothing at all was counted"
        );
    }

    /// A cover is fetched once, then read from the disk with ZERO requests —
    /// what a scrolling list costs, counted.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cover_is_fetched_once_then_read_from_disk() {
        let bench = Bench::start(true);
        let dir = TempDir::new("covers-cache");
        let core = dir.core("test");
        core.set_server(&bench.base()).await.expect("server");
        core.login("bench", "Bench1").await.expect("login");

        let (url, sent_bytes) = bench.cover().await;
        let first = core.cover_path(&url, 128).await.expect("a cover");
        let bytes = std::fs::read(&first).expect("the file exists");
        assert_eq!(
            bytes.len(),
            sent_bytes,
            "the stored file is exactly what the server sent"
        );
        let cover = core
            .stats()
            .into_iter()
            .find(|stats| stats.tag == TAG_COVER)
            .expect("a cover row");
        assert_eq!(cover.requests, 1);
        assert_eq!(cover.bytes_in, sent_bytes as u64);

        // A second ask, same URL: the same path, no new request.
        let second = core.cover_path(&url, 128).await.expect("the same cover");
        assert_eq!(first, second);
        let cover = core
            .stats()
            .into_iter()
            .find(|stats| stats.tag == TAG_COVER)
            .expect("a cover row");
        assert_eq!(cover.requests, 1, "a hit is not a request");

        // And it survives a restart of the core (the file and its row).
        let expected = again_path(&dir, &url);
        assert_eq!(expected, Some(second));
    }

    /// The path a SCREEN hands over is the server's own root-relative one
    /// (`/api/cover/<album>`): it is asked of the configured origin, with the
    /// session, exactly once — and a protocol-relative `//host/…` is another
    /// host and costs nothing. Before the fix the relative path was refused and
    /// the home drew grooves only.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_root_relative_cover_is_asked_of_the_configured_origin() {
        let bench = Bench::start(true);
        let dir = TempDir::new("covers-relative");
        let core = dir.core("test");
        core.set_server(&bench.base()).await.expect("server");
        core.login("bench", "Bench1").await.expect("login");

        let (absolute, sent_bytes) = bench.cover().await;
        let relative = &absolute[absolute.find("/api/").expect("an API cover")..];
        let path = core.cover_path(relative, 128).await.expect("a cover");
        assert_eq!(
            std::fs::read(&path).expect("the file").len(),
            sent_bytes,
            "the configured origin's bytes"
        );
        let requests = |core: &crate::core::Core| {
            core.stats()
                .into_iter()
                .find(|stats| stats.tag == TAG_COVER)
                .map_or(0, |row| row.requests)
        };
        assert_eq!(requests(&core), 1);

        let error = core
            .cover_path("//evil.example/api/cover/1", 128)
            .await
            .expect_err("protocol-relative is another host");
        assert!(matches!(error, CoreError::Invalid { .. }), "{error:?}");
        assert_eq!(requests(&core), 1, "a refused cover is not a request");
    }

    fn again_path(dir: &TempDir, url: &str) -> Option<String> {
        let again = dir.core("test");
        again
            .cache()
            .covers()
            .get(url)
            .map(|path| path.display().to_string())
    }
}
