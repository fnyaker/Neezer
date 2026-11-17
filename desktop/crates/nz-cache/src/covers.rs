//! Cover art on disk: the files under `$XDG_CACHE_HOME/neezer/covers/`, their
//! rows, and the LRU that keeps them inside 300 MB.
//!
//! The store keeps BYTES, never decoded images: decoding happens off the main
//! thread and the picture's in-memory LRU (64 MB) belongs to the app's image
//! path, not here (docs/agents/desktop.md, *Caches*).
//!
//! Two details the SPA paid for and this store repeats:
//!
//! - **A download's art is PINNED.** An album prepared for a plane keeps its
//!   pochette even when the 300 MB cap is reached; `lib/offline.js` keeps its
//!   own copy of every downloaded cover for exactly that reason.
//! - **A row whose file is gone heals.** The row is dropped on read, so the art
//!   is fetched again instead of being permanently artless.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::params;
use tracing::debug;

use crate::{Cache, Result, now_ms, url_digest};

/// The default cap of the cover directory (docs/agents/desktop.md, *Caches*).
pub const DEFAULT_CAP_BYTES: i64 = 300 * 1024 * 1024;

/// The cover-art store.
pub struct Covers<'a> {
    cache: &'a Cache,
}

impl<'a> Covers<'a> {
    pub(crate) fn new(cache: &'a Cache) -> Self {
        Self { cache }
    }

    /// The cached file for `url`, bumping its recency (a hit is a use). `None`
    /// when there is no copy, including when a row outlived its file.
    pub fn get(&self, url: &str) -> Option<PathBuf> {
        match self.lookup(url, true) {
            Ok(path) => path,
            Err(error) => {
                debug!(%error, "cover lookup failed");
                None
            }
        }
    }

    /// The cached file for `url` WITHOUT touching its recency: what MPRIS and a
    /// preload need (reading art is not the person choosing it).
    pub fn file(&self, url: &str) -> Option<PathBuf> {
        self.lookup(url, false).ok().flatten()
    }

    /// Stores `bytes` for `url` (replacing what was there, in any format).
    pub fn put(&self, url: &str, bytes: &[u8]) -> Result<PathBuf> {
        self.store(url, bytes, false)
    }

    /// Stores art a DOWNLOAD owns: the LRU never evicts it.
    pub fn put_pinned(&self, url: &str, bytes: &[u8]) -> Result<PathBuf> {
        self.store(url, bytes, true)
    }

    /// Marks an already-stored cover as a download's (pinned against eviction).
    pub fn pin(&self, url: &str) -> Result<()> {
        self.set_pinned(url, true)
    }

    /// Gives the cover back to the LRU.
    pub fn unpin(&self, url: &str) -> Result<()> {
        self.set_pinned(url, false)
    }

    /// Drops one cover (the person cleared it, or a download was removed).
    pub fn remove(&self, url: &str) -> Result<bool> {
        let file = self.cache.read(|conn| {
            self.cache.query_optional(
                conn,
                "SELECT file FROM covers WHERE url = ?1",
                params![url],
                |row| row.get::<_, String>(0),
            )
        })?;
        let Some(file) = file else {
            return Ok(false);
        };
        let _ = fs::remove_file(self.cache.covers_dir().join(file));
        self.cache.write(|cache, conn| {
            cache.exec(conn, "DELETE FROM covers WHERE url = ?1", params![url])?;
            Ok(())
        })?;
        Ok(true)
    }

    /// Bytes on disk (pinned art included: it occupies the directory too).
    pub fn total_bytes(&self) -> Result<i64> {
        self.cache.read(|conn| {
            self.cache.query(
                conn,
                "SELECT COALESCE(SUM(bytes), 0) FROM covers",
                [],
                |row| row.get(0),
            )
        })
    }

    /// How many covers are stored.
    pub fn len(&self) -> Result<i64> {
        self.cache.read(|conn| {
            self.cache
                .query(conn, "SELECT COUNT(*) FROM covers", [], |row| row.get(0))
        })
    }

    /// Nothing stored.
    pub fn is_empty(&self) -> bool {
        self.len().unwrap_or(1) == 0
    }

    /// Evicts least-recently-used covers until at most `cap_bytes` remain,
    /// never touching a pinned one.
    ///
    /// Returns the bytes freed. A row whose file has already disappeared counts
    /// as freed: the accounting is repaired instead of being believed.
    pub fn evict_to(&self, cap_bytes: i64) -> Result<u64> {
        let total = self.total_bytes()?;
        if total <= cap_bytes {
            return Ok(0);
        }
        let file = |dir: &Path, name: &str| dir.join(name);
        let rows = self.cache.read(|conn| {
            self.cache.rows(
                conn,
                "SELECT url, file, bytes FROM covers WHERE pinned = 0 ORDER BY used_at ASC",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
        })?;

        let mut freed: i64 = 0;
        let mut removed = 0usize;
        for (url, name, bytes) in rows {
            if total - freed <= cap_bytes {
                break;
            }
            let _ = fs::remove_file(file(&self.cache.covers_dir(), &name));
            self.cache.write(|cache, conn| {
                cache.exec(conn, "DELETE FROM covers WHERE url = ?1", params![url])?;
                Ok(())
            })?;
            freed += bytes.max(0);
            removed += 1;
        }
        debug!(removed, freed, "covers evicted");
        Ok(freed.max(0) as u64)
    }

    /// Wipes the cover directory (Réglages → Téléchargements), pinned art
    /// included: this is the person's own command, not the LRU.
    pub fn clear(&self) -> Result<u64> {
        let rows = self.cache.read(|conn| {
            self.cache
                .rows(conn, "SELECT file, bytes FROM covers", [], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
        })?;
        let mut freed = 0i64;
        for (name, bytes) in rows {
            let _ = fs::remove_file(self.cache.covers_dir().join(name));
            freed += bytes.max(0);
        }
        self.cache.write(|cache, conn| {
            cache.exec(conn, "DELETE FROM covers", [])?;
            Ok(())
        })?;
        Ok(freed as u64)
    }

    fn set_pinned(&self, url: &str, pinned: bool) -> Result<()> {
        self.cache.write(|cache, conn| {
            cache.exec(
                conn,
                "UPDATE covers SET pinned = ?2 WHERE url = ?1",
                params![url, i64::from(pinned)],
            )?;
            Ok(())
        })
    }

    fn lookup(&self, url: &str, touch: bool) -> Result<Option<PathBuf>> {
        let stored = self.cache.read(|conn| {
            self.cache.query_optional(
                conn,
                "SELECT file FROM covers WHERE url = ?1",
                params![url],
                |row| row.get::<_, String>(0),
            )
        })?;
        let Some(name) = stored else {
            return Ok(None);
        };
        let path = self.cache.covers_dir().join(&name);
        if !path.is_file() {
            // The row outlived its file: drop it, so the art is fetched again
            // rather than never showing.
            self.cache.write(|cache, conn| {
                cache.exec(conn, "DELETE FROM covers WHERE url = ?1", params![url])?;
                Ok(())
            })?;
            debug!(url, "cover row without its file dropped");
            return Ok(None);
        }
        if touch {
            self.cache.write(|cache, conn| {
                cache.exec(
                    conn,
                    "UPDATE covers SET used_at = ?2 WHERE url = ?1",
                    params![url, now_ms()],
                )?;
                Ok(())
            })?;
        }
        Ok(Some(path))
    }

    fn store(&self, url: &str, bytes: &[u8], pinned: bool) -> Result<PathBuf> {
        let name = format!("{}.{}", url_digest(url), sniff_extension(bytes));
        let dir = self.cache.covers_dir();
        let target = dir.join(&name);
        // Published only once whole, like every other file this crate writes.
        let temporary = dir.join(format!("{}.part", url_digest(url)));
        fs::write(&temporary, bytes)?;
        fs::rename(&temporary, &target)?;

        let previous = self.cache.read(|conn| {
            self.cache.query_optional(
                conn,
                "SELECT file FROM covers WHERE url = ?1",
                params![url],
                |row| row.get::<_, String>(0),
            )
        })?;

        self.cache.write(|cache, conn| {
            cache.exec(
                conn,
                "INSERT INTO covers(url, file, bytes, used_at, pinned) VALUES(?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT(url) DO UPDATE SET file = excluded.file, bytes = excluded.bytes, \
                 used_at = excluded.used_at, pinned = MAX(covers.pinned, excluded.pinned)",
                params![url, name, bytes.len() as i64, now_ms(), i64::from(pinned)],
            )?;
            Ok(())
        })?;

        // The same URL in another format (a re-encoded sleeve): the old file is
        // not reachable anymore, so it would sit in the directory forever.
        if let Some(previous) = previous
            && previous != name
        {
            let _ = fs::remove_file(dir.join(previous));
        }
        Ok(target)
    }
}

/// The extension a cover file gets, sniffed from its own first bytes.
///
/// The server sends what the album carried (JPEG, PNG, WebP, GIF); an
/// extensionless `file://` URI works for a decoder that sniffs, but not for
/// every caller (MPRIS' `mpris:artUrl` is handed to other programs), so the
/// name says what the bytes are. Anything unrecognised is `img`.
fn sniff_extension(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "gif"
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        "webp"
    } else {
        "img"
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::Result;
    use crate::testing::TempDir;

    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F'];
    // A real PNG signature; the body's content is irrelevant to the store.
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n000000000000";

    #[test]
    fn a_stored_cover_comes_back_and_its_extension_is_sniffed() -> Result<()> {
        let dir = TempDir::new("covers-roundtrip");
        let cache = dir.open();

        let file = cache.covers().put("/api/cover/abc", JPEG)?;
        assert_eq!(file.extension().and_then(|e| e.to_str()), Some("jpg"));
        assert!(file.is_file());
        assert_eq!(fs::read(&file)?, JPEG);
        assert_eq!(
            cache.covers().get("/api/cover/abc").as_deref(),
            Some(file.as_path())
        );

        // Same URL, another format: the old file must not be left behind.
        let png = cache.covers().put("/api/cover/abc", PNG)?;
        assert_eq!(png.extension().and_then(|e| e.to_str()), Some("png"));
        assert!(!file.exists(), "the JPEG file was replaced, not orphaned");
        assert_eq!(cache.covers().len()?, 1);
        Ok(())
    }

    #[test]
    fn the_sniffer_knows_the_four_the_server_sends() {
        assert_eq!(super::sniff_extension(JPEG), "jpg");
        assert_eq!(super::sniff_extension(PNG), "png");
        assert_eq!(super::sniff_extension(b"GIF89a...."), "gif");
        assert_eq!(super::sniff_extension(b"RIFF\0\0\0\0WEBPVP8 "), "webp");
        assert_eq!(super::sniff_extension(b"not an image"), "img");
        // Short bodies must not panic on the WebP slice.
        assert_eq!(super::sniff_extension(b"RI"), "img");
    }

    /// Criterion: "evict_to keeps the most recently used and never touches
    /// downloads".
    #[test]
    fn eviction_keeps_the_most_recently_used() -> Result<()> {
        let dir = TempDir::new("covers-lru");
        let cache = dir.open();
        let covers = cache.covers();

        let oldest = covers.put("/api/cover/old", JPEG)?;
        let middle = covers.put("/api/cover/mid", JPEG)?;
        let newest = covers.put("/api/cover/new", JPEG)?;
        let each = fs::metadata(&oldest)?.len() as i64;

        // Touch the newest two so the order is unambiguous, then cap at 2.
        assert!(covers.get("/api/cover/mid").is_some());
        assert!(covers.get("/api/cover/new").is_some());

        let freed = covers.evict_to(each * 2)?;
        assert_eq!(freed, each as u64);
        assert!(!oldest.exists(), "the least recently used went");
        assert!(middle.exists());
        assert!(newest.exists());
        assert_eq!(covers.len()?, 2);
        Ok(())
    }

    #[test]
    fn a_downloads_cover_is_pinned_against_eviction() -> Result<()> {
        let dir = TempDir::new("covers-pinned");
        let cache = dir.open();
        let covers = cache.covers();

        let pinned = covers.put_pinned("/api/cover/album-1", JPEG)?;
        let evictable = covers.put("/api/cover/album-2", JPEG)?;

        covers.evict_to(0)?;

        assert!(pinned.exists(), "a download's art survives the LRU");
        assert!(!evictable.exists());
        assert_eq!(covers.len()?, 1);

        // Unpinned, it goes like everything else.
        covers.unpin("/api/cover/album-1")?;
        covers.evict_to(0)?;
        assert!(!pinned.exists());
        Ok(())
    }

    #[test]
    fn a_row_without_its_file_is_dropped_on_read() -> Result<()> {
        let dir = TempDir::new("covers-heal");
        let cache = dir.open();
        let covers = cache.covers();

        let file = covers.put("/api/cover/gone", JPEG)?;
        fs::remove_file(&file)?;

        assert_eq!(covers.get("/api/cover/gone"), None);
        assert_eq!(covers.len()?, 0, "the row was healed away");
        Ok(())
    }

    #[test]
    fn clearing_frees_the_whole_directory_including_pinned_art() -> Result<()> {
        let dir = TempDir::new("covers-clear");
        let cache = dir.open();
        let covers = cache.covers();

        covers.put("/api/cover/a", JPEG)?;
        covers.put_pinned("/api/cover/b", PNG)?;
        let freed = covers.clear()?;

        assert_eq!(freed, (JPEG.len() + PNG.len()) as u64);
        assert_eq!(covers.len()?, 0);
        assert_eq!(covers.total_bytes()?, 0);
        Ok(())
    }
}
