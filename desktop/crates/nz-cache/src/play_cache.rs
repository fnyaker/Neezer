//! The playback cache: the audio of played and prefetched tracks, on disk, LRU.
//!
//! This is the SPA's `lib/playcache.js`, minus IndexedDB: what was streamed
//! once is replayed from disk, so a re-buffer, a seek and the next track ride
//! out a network drop (rule 3). It is NOT the downloads store: everything here
//! is evictable, and nothing here is ever the person's only copy.
//!
//! The two rules the page states and this store keeps:
//!
//! - **Only a COMPLETE file is replayed.** `reserve` hands the caller the path
//!   its bytes go to; until `complete` publishes it (`.part` → renamed, length
//!   read back), `lookup` answers `None`. A half file played as a track is
//!   worse than a fetch.
//! - **Eviction is by recency, and it never reaches a download.** Downloads
//!   live in their own directory and their own table; a cap on the play cache
//!   cannot take one (the SPA's `enforce` does the same, with `isDownloaded`).

use std::fs;
use std::io::{Error as IoError, ErrorKind};
use std::path::PathBuf;
use std::time::Duration;

use rusqlite::params;
use tracing::debug;

use crate::{Cache, Quality, Result, TrackId, now_ms, parse_quality, quality_name, track_digest};

/// How long a reservation may sit unfinished before eviction may take it back.
///
/// A reservation is a file someone is writing RIGHT NOW (a stream spooling in
/// `nz-audio`); evicting it mid-write would fail that write for nothing. An
/// hour is longer than any track takes on a bad link, and short enough that a
/// cancelled prefetch cannot pin bytes forever.
pub const STALE_RESERVATION: Duration = Duration::from_secs(3_600);

/// The evictable audio cache.
pub struct PlayCache<'a> {
    cache: &'a Cache,
}

impl<'a> PlayCache<'a> {
    pub(crate) fn new(cache: &'a Cache) -> Self {
        Self { cache }
    }

    /// Reserves the file a track's bytes are written to, and returns the path.
    ///
    /// The file is created (empty) under its final digest with a `.part`
    /// suffix, and the row is marked incomplete: a crash between this and
    /// [`PlayCache::complete`] leaves a reservation, not a playable lie.
    pub fn reserve(&self, track: &TrackId, quality: Quality) -> Result<PathBuf> {
        let digest = track_digest(track, quality);
        let partial = self.cache.audio_dir().join(format!("{digest}.part"));
        fs::write(&partial, b"")?;
        let file = format!("{digest}.{}", quality_name(quality));

        self.cache.write(|cache, conn| {
            cache.exec(
                conn,
                "INSERT INTO play_cache(track, quality, file, bytes, complete, used_at) \
                 VALUES(?1, ?2, ?3, 0, 0, ?4) \
                 ON CONFLICT(track, quality) DO UPDATE SET file = excluded.file, bytes = 0, \
                 complete = 0, used_at = excluded.used_at",
                params![track.as_str(), quality_name(quality), file, now_ms()],
            )?;
            Ok(())
        })?;

        Ok(partial)
    }

    /// Publishes a reserved file: `.part` is renamed into place and the row
    /// becomes replayable.
    ///
    /// The size stored is the file's OWN length, not the caller's count: the
    /// cap is an accounting rule, and an accounting rule that trusts a number
    /// from outside is a cap that does not hold.
    pub fn complete(&self, track: &TrackId, quality: Quality, bytes: u64) -> Result<PathBuf> {
        let digest = track_digest(track, quality);
        let partial = self.cache.audio_dir().join(format!("{digest}.part"));
        let file = format!("{digest}.{}", quality_name(quality));
        let target = self.cache.audio_dir().join(&file);

        let published = if partial.is_file() {
            fs::rename(&partial, &target)?;
            true
        } else {
            // The reservation was evicted or never written by this handle. If a
            // finished file is already in place the row may still be published;
            // otherwise the caller is told, instead of a track appearing cached
            // with no bytes behind it.
            target.is_file()
        };
        if !published {
            return Err(IoError::new(
                ErrorKind::NotFound,
                format!(
                    "réservation absente pour {track} ({})",
                    quality_name(quality)
                ),
            )
            .into());
        }

        let length = fs::metadata(&target)?.len();
        if length != bytes {
            debug!(
                track = track.as_str(),
                quality = quality_name(quality),
                counted = bytes,
                on_disk = length,
                "completed play-cache entry: storing the file's own length"
            );
        }

        self.cache.write(|cache, conn| {
            cache.exec(
                conn,
                "INSERT INTO play_cache(track, quality, file, bytes, complete, used_at) \
                 VALUES(?1, ?2, ?3, ?4, 1, ?5) \
                 ON CONFLICT(track, quality) DO UPDATE SET file = excluded.file, \
                 bytes = excluded.bytes, complete = 1, used_at = excluded.used_at",
                params![
                    track.as_str(),
                    quality_name(quality),
                    file,
                    length as i64,
                    now_ms()
                ],
            )?;
            Ok(())
        })?;

        Ok(target)
    }

    /// The replayable file of a track at a quality, bumping its recency.
    pub fn lookup(&self, track: &TrackId, quality: Quality) -> Result<Option<PathBuf>> {
        let row = self.cache.read(|conn| {
            self.cache.query_optional(
                conn,
                "SELECT file FROM play_cache WHERE track = ?1 AND quality = ?2 AND complete = 1",
                params![track.as_str(), quality_name(quality)],
                |row| row.get::<_, String>(0),
            )
        })?;
        let Some(file) = row else {
            return Ok(None);
        };
        let path = self.cache.audio_dir().join(&file);
        if !path.is_file() {
            // Written by an older build and wiped since: forget it rather than
            // hand the decoder a path that does not open.
            self.forget(track, quality)?;
            return Ok(None);
        }
        self.cache.write(|cache, conn| {
            cache.exec(
                conn,
                "UPDATE play_cache SET used_at = ?3 WHERE track = ?1 AND quality = ?2",
                params![track.as_str(), quality_name(quality), now_ms()],
            )?;
            Ok(())
        })?;
        Ok(Some(path))
    }

    /// Whether a replayable copy exists (no recency change, no file read).
    pub fn contains(&self, track: &TrackId, quality: Quality) -> bool {
        self.cache
            .read(|conn| {
                self.cache.query(
                    conn,
                    "SELECT COUNT(*) FROM play_cache \
                     WHERE track = ?1 AND quality = ?2 AND complete = 1",
                    params![track.as_str(), quality_name(quality)],
                    |row| row.get::<_, i64>(0),
                )
            })
            .map(|count| count > 0)
            .unwrap_or(false)
    }

    /// Drops one entry and its files.
    pub fn forget(&self, track: &TrackId, quality: Quality) -> Result<bool> {
        let digest = track_digest(track, quality);
        let removed = self.cache.write(|cache, conn| {
            cache.exec(
                conn,
                "DELETE FROM play_cache WHERE track = ?1 AND quality = ?2",
                params![track.as_str(), quality_name(quality)],
            )
        })?;
        let _ = fs::remove_file(self.cache.audio_dir().join(&digest));
        let _ = fs::remove_file(self.cache.audio_dir().join(format!("{digest}.part")));
        Ok(removed > 0)
    }

    /// Every (track, quality) the cache can replay — the app's `cachedIds` set
    /// at startup, in one statement.
    pub fn entries(&self) -> Result<Vec<(TrackId, Quality)>> {
        let rows = self.cache.read(|conn| {
            self.cache.rows(
                conn,
                "SELECT track, quality FROM play_cache WHERE complete = 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
        })?;
        Ok(rows
            .into_iter()
            .filter_map(|(track, quality)| {
                parse_quality(&quality).map(|quality| (TrackId::new(track), quality))
            })
            .collect())
    }

    /// Bytes accounted for (the number the UI shows).
    pub fn total_bytes(&self) -> Result<i64> {
        self.cache.read(|conn| {
            self.cache.query(
                conn,
                "SELECT COALESCE(SUM(bytes), 0) FROM play_cache",
                [],
                |row| row.get(0),
            )
        })
    }

    /// Evicts least-recently-used audio until at most `cap_bytes` remain.
    ///
    /// Two phases, in this order:
    ///
    /// 1. **Abandoned reservations** (`complete = 0`, older than
    ///    [`STALE_RESERVATION`]) are taken back whatever the cap is: their
    ///    bytes belong to a fetch that will never finish.
    /// 2. **Complete entries**, least recently used first, until the cap holds.
    ///
    /// A reservation being written RIGHT NOW is never taken: someone is
    /// fetching that track, and pulling its file out from under the stream
    /// would break what the person is doing to save bytes for a cache (rule 1).
    /// Its bytes are not in `bytes` yet, so it does not distort the cap either.
    pub fn evict_to(&self, cap_bytes: i64) -> Result<u64> {
        let mut freed = self.evict_stale_reservations()?;
        let total = self.total_bytes()?;
        if total <= cap_bytes {
            return Ok(freed);
        }

        let rows = self.cache.read(|conn| {
            self.cache.rows(
                conn,
                "SELECT track, quality, file FROM play_cache WHERE complete = 1 \
                 ORDER BY used_at ASC",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
        })?;

        let mut remaining = total;
        let mut removed = 0usize;
        for (track, quality, file) in rows {
            if remaining <= cap_bytes {
                break;
            }
            let path = self.cache.audio_dir().join(&file);
            let took = fs::metadata(&path)
                .map(|meta| meta.len() as i64)
                .unwrap_or(0);
            let _ = fs::remove_file(&path);
            self.cache.write(|cache, conn| {
                cache.exec(
                    conn,
                    "DELETE FROM play_cache WHERE track = ?1 AND quality = ?2",
                    params![track, quality],
                )?;
                Ok(())
            })?;
            remaining -= took;
            freed += took.max(0) as u64;
            removed += 1;
        }
        debug!(removed, freed, "play cache evicted");
        Ok(freed)
    }

    /// Takes back reservations nobody is finishing: a cancelled prefetch, a
    /// track whose fetch died with the app. Returns the bytes freed.
    fn evict_stale_reservations(&self) -> Result<u64> {
        let stale_before = now_ms() - STALE_RESERVATION.as_millis() as i64;
        let rows = self.cache.read(|conn| {
            self.cache.rows(
                conn,
                "SELECT track, quality, file FROM play_cache \
                 WHERE complete = 0 AND used_at < ?1",
                params![stale_before],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
        })?;

        let mut freed = 0u64;
        for (track, quality, _file) in rows {
            let mut took = 0u64;
            if let Some(quality) = parse_quality(&quality) {
                let partial = self.cache.audio_dir().join(format!(
                    "{}.part",
                    track_digest(&TrackId::new(&track), quality)
                ));
                if let Ok(meta) = fs::metadata(&partial) {
                    took = meta.len();
                }
                let _ = fs::remove_file(&partial);
            }
            self.cache.write(|cache, conn| {
                cache.exec(
                    conn,
                    "DELETE FROM play_cache WHERE track = ?1 AND quality = ?2",
                    params![track, quality],
                )?;
                Ok(())
            })?;
            freed += took;
        }
        Ok(freed)
    }

    /// Wipes the audio directory (Réglages → Téléchargements).
    pub fn clear(&self) -> Result<u64> {
        let rows = self.cache.read(|conn| {
            self.cache
                .rows(conn, "SELECT file, bytes FROM play_cache", [], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
        })?;
        let mut freed = 0i64;
        for (file, bytes) in rows {
            let _ = fs::remove_file(self.cache.audio_dir().join(file));
            freed += bytes.max(0);
        }
        // Reservation files are named `<digest>.part` and are not rows' `file`.
        if let Ok(entries) = fs::read_dir(self.cache.audio_dir()) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("part") {
                    let _ = fs::remove_file(path);
                }
            }
        }
        self.cache.write(|cache, conn| {
            cache.exec(conn, "DELETE FROM play_cache", [])?;
            Ok(())
        })?;
        Ok(freed as u64)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{DownloadTrack, Quality, Result, TrackId, testing::TempDir};

    fn track() -> TrackId {
        TrackId::new("3135556")
    }

    /// Criterion: a reservation is not replayable, and completing it publishes
    /// the file under its digest.
    #[test]
    fn only_a_complete_entry_is_replayed() -> Result<()> {
        let dir = TempDir::new("play-reserve");
        let cache = dir.open();
        let play = cache.play_cache();

        let partial = play.reserve(&track(), Quality::Opus320)?;
        assert!(partial.is_file());
        assert_eq!(partial.extension().and_then(|e| e.to_str()), Some("part"));
        assert_eq!(play.lookup(&track(), Quality::Opus320)?, None);

        fs::write(&partial, vec![7u8; 4096])?;
        let published = play.complete(&track(), Quality::Opus320, 4096)?;
        assert!(published.is_file());
        assert!(!partial.exists(), "the .part was renamed, not copied");
        assert_eq!(fs::metadata(&published)?.len(), 4096);
        assert_eq!(
            play.lookup(&track(), Quality::Opus320)?,
            Some(published.clone())
        );
        assert_eq!(play.total_bytes()?, 4096);
        Ok(())
    }

    #[test]
    fn the_quality_is_part_of_the_key() -> Result<()> {
        let dir = TempDir::new("play-quality");
        let cache = dir.open();
        let play = cache.play_cache();

        for quality in [Quality::Flac, Quality::Opus320, Quality::Opus64] {
            let part = play.reserve(&track(), quality)?;
            fs::write(&part, vec![1u8; 100])?;
            play.complete(&track(), quality, 100)?;
        }

        assert_eq!(play.entries()?.len(), 3);
        assert_eq!(play.total_bytes()?, 300);
        // One quality dropped leaves the others in place.
        play.forget(&track(), Quality::Opus320)?;
        assert_eq!(play.lookup(&track(), Quality::Opus320)?, None);
        assert!(play.lookup(&track(), Quality::Opus64)?.is_some());
        Ok(())
    }

    /// Criterion: "evict_to keeps the most recently used".
    #[test]
    fn eviction_keeps_the_most_recently_used() -> Result<()> {
        let dir = TempDir::new("play-lru");
        let cache = dir.open();
        let play = cache.play_cache();

        let mut published = Vec::new();
        for id in ["old", "mid", "new"] {
            let track = TrackId::new(id);
            let part = play.reserve(&track, Quality::Opus128)?;
            fs::write(&part, vec![0u8; 1000])?;
            published.push(play.complete(&track, Quality::Opus128, 1000)?);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        // Touch the two newest, so recency is what the order reflects.
        assert!(
            play.lookup(&TrackId::new("mid"), Quality::Opus128)?
                .is_some()
        );
        assert!(
            play.lookup(&TrackId::new("new"), Quality::Opus128)?
                .is_some()
        );

        let freed = play.evict_to(2000)?;
        assert_eq!(freed, 1000);
        assert!(!published[0].exists(), "the oldest went");
        assert!(published[1].exists());
        assert!(published[2].exists());
        assert_eq!(play.total_bytes()?, 2000);
        Ok(())
    }

    /// Criterion: "never touches downloads". A download lives in the data
    /// directory, under a set the person chose; no play-cache cap reaches it.
    #[test]
    fn eviction_never_touches_a_download() -> Result<()> {
        let dir = TempDir::new("play-downloads");
        let cache = dir.open();

        cache.downloads().add_set(
            "42",
            "album",
            "99",
            "Songs from the Inside",
            &[DownloadTrack {
                track: track(),
                quality: Quality::Opus320,
            }],
        )?;
        let part = cache
            .downloads()
            .reserve("42", &track(), Quality::Opus320)?;
        fs::write(&part, vec![3u8; 2048])?;
        let offline = cache.downloads().complete("42", &track(), 2048)?;

        let play_part = cache
            .play_cache()
            .reserve(&TrackId::new("other"), Quality::Opus64)?;
        fs::write(&play_part, vec![0u8; 512])?;
        cache
            .play_cache()
            .complete(&TrackId::new("other"), Quality::Opus64, 512)?;

        cache.play_cache().evict_to(0)?;

        assert!(
            offline.is_file(),
            "the download survived the play cache's cap"
        );
        assert_eq!(cache.play_cache().total_bytes()?, 0);
        assert_eq!(
            cache.downloads().local("42", &track())?,
            Some((offline, Quality::Opus320))
        );
        Ok(())
    }

    #[test]
    fn an_abandoned_reservation_is_evicted_but_a_fresh_one_is_spared() -> Result<()> {
        let dir = TempDir::new("play-stale");
        let cache = dir.open();
        let play = cache.play_cache();

        let fresh = play.reserve(&TrackId::new("fresh"), Quality::Opus64)?;
        let old = play.reserve(&TrackId::new("old"), Quality::Opus64)?;
        fs::write(&old, vec![0u8; 64])?;
        // Age the abandoned reservation past the grace period, as a cancelled
        // prefetch would be after an hour.
        cache.write(|cache, conn| {
            cache.exec(
                conn,
                "UPDATE play_cache SET used_at = ?1 WHERE track = 'old'",
                rusqlite::params![crate::now_ms() - 7_200_000i64],
            )?;
            Ok(())
        })?;

        play.evict_to(0)?;

        assert!(fresh.exists(), "a reservation being written is spared");
        assert!(!old.exists(), "an hour-old reservation is taken back");
        Ok(())
    }
}
