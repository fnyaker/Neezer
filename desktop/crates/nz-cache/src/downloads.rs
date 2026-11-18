//! The person's downloads ("hors ligne"): what was chosen per playlist, album
//! or show, the work left to do, and the files — under
//! `$XDG_DATA_HOME/neezer/offline/`, which the LRU never touches.
//!
//! Three rules from `webapp/src/lib/offline.js` and the note's *Caches*:
//!
//! - **A file is published only once complete**: bytes go to `<digest>.part`,
//!   `complete` renames it and reads the length back.
//! - **Nothing deletes a download but the person**: `remove_set` / `remove` are
//!   the only paths, and they are only reachable from a command the person gave.
//! - **Each failure keeps its CAUSE**, not a count: `mark(…, Failed, Some(cause))`
//!   is what a screen prints (rule 2).
//!
//! A track can be in several sets (its album, two playlists) and is downloaded
//! ONCE — `downloads` is keyed by `(owner, track)`, and `download_members` says
//! which sets want it, so removing one set takes back only what nothing else
//! needs. That link is why the table exists at all.

use std::fs;
use std::io::{Error as IoError, ErrorKind};
use std::path::PathBuf;

use rusqlite::params;
use tracing::debug;

use crate::{Cache, Quality, Result, TrackId, now_ms, parse_quality, quality_name, track_digest};

/// Where a download is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DownloadState {
    /// Chosen, nothing fetched yet.
    Pending,
    /// Bytes are being written (after a restart this goes back to the queue).
    Active,
    /// Published: the file is complete and playable.
    Done,
    /// Gave up, with a cause in `error`.
    Failed,
}

impl DownloadState {
    /// The TEXT spelling in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    /// Reads a row's state. An unknown word (a newer build) is treated as
    /// pending, so the work is retried rather than dropped.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "pending" => Some(Self::Pending),
            "active" => Some(Self::Active),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// One track chosen for offline, at the quality chosen when it was added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadTrack {
    /// The track's universal id.
    pub track: TrackId,
    /// The quality to transcode it at.
    pub quality: Quality,
}

/// One piece of work still to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadJob {
    /// Which track.
    pub track: TrackId,
    /// At which quality.
    pub quality: Quality,
    /// Pending, or already being written (a job interrupted by a restart).
    pub state: DownloadState,
}

/// A playlist, album or show chosen for offline, with its progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadSet {
    /// `playlist`, `album` or `show`.
    pub kind: String,
    /// The server's id of that entity.
    pub ident: String,
    /// The title, kept locally so the list opens offline.
    pub title: String,
    /// When it was added (ms since the epoch).
    pub added_at: i64,
    /// How many tracks it wants.
    pub tracks: usize,
    /// How many of those are published.
    pub done: usize,
}

/// A person's whole offline workload, for the badge and the settings screen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DownloadProgress {
    /// Chosen, not started.
    pub pending: usize,
    /// Being written right now (or interrupted by a restart).
    pub active: usize,
    /// Published and playable.
    pub done: usize,
    /// Gave up; each has its cause in the database.
    pub failed: usize,
    /// Bytes published.
    pub bytes: i64,
}

/// The downloads store.
pub struct Downloads<'a> {
    cache: &'a Cache,
}

impl<'a> Downloads<'a> {
    pub(crate) fn new(cache: &'a Cache) -> Self {
        Self { cache }
    }

    /// Chooses a playlist/album/show for offline: the set's row, its tracks as
    /// pending work, and their membership — one transaction, so a crash cannot
    /// leave a set that claims tracks nobody queued.
    ///
    /// A track that is already downloaded keeps its file (and its quality):
    /// asking for a playlist again must not discard what is on disk.
    /// Returns how many tracks were newly linked.
    pub fn add_set(
        &self,
        owner: &str,
        kind: &str,
        ident: &str,
        title: &str,
        tracks: &[DownloadTrack],
    ) -> Result<usize> {
        self.cache.write(|cache, conn| {
            cache.exec(
                conn,
                "INSERT INTO download_sets(owner, kind, ident, title, added_at) \
                 VALUES(?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT(owner, kind, ident) DO UPDATE SET title = excluded.title",
                params![owner, kind, ident, title, now_ms()],
            )?;
            let mut linked = 0usize;
            for DownloadTrack { track, quality } in tracks {
                cache.exec(
                    conn,
                    "INSERT INTO downloads(owner, track, quality, file, bytes, state, error, added_at) \
                     VALUES(?1, ?2, ?3, NULL, NULL, 'pending', NULL, ?4) \
                     ON CONFLICT(owner, track) DO NOTHING",
                    params![owner, track.as_str(), quality_name(*quality), now_ms()],
                )?;
                linked += cache.exec(
                    conn,
                    "INSERT INTO download_members(owner, kind, ident, track) VALUES(?1, ?2, ?3, ?4) \
                     ON CONFLICT(owner, kind, ident, track) DO NOTHING",
                    params![owner, kind, ident, track.as_str()],
                )?;
            }
            Ok(linked)
        })
    }

    /// Takes a set back: its membership goes, and the tracks no other set wants
    /// are deleted with their files. Returns how many downloads were removed.
    pub fn remove_set(&self, owner: &str, kind: &str, ident: &str) -> Result<usize> {
        let (removed, files) = self.cache.write(|cache, conn| {
            let tracks = cache.rows(
                conn,
                "SELECT track FROM download_members WHERE owner = ?1 AND kind = ?2 AND ident = ?3",
                params![owner, kind, ident],
                |row| row.get::<_, String>(0),
            )?;
            cache.exec(
                conn,
                "DELETE FROM download_members WHERE owner = ?1 AND kind = ?2 AND ident = ?3",
                params![owner, kind, ident],
            )?;

            let mut removed = 0usize;
            let mut files: Vec<String> = Vec::new();
            for track in tracks {
                let wanted: i64 = cache.query(
                    conn,
                    "SELECT COUNT(*) FROM download_members WHERE owner = ?1 AND track = ?2",
                    params![owner, &track],
                    |row| row.get(0),
                )?;
                if wanted > 0 {
                    continue; // another set still wants it
                }
                let file: Option<String> = cache
                    .query_optional(
                        conn,
                        "SELECT file FROM downloads WHERE owner = ?1 AND track = ?2",
                        params![owner, &track],
                        |row| row.get(0),
                    )?
                    .flatten();
                removed += cache.exec(
                    conn,
                    "DELETE FROM downloads WHERE owner = ?1 AND track = ?2",
                    params![owner, &track],
                )?;
                files.extend(file);
            }
            cache.exec(
                conn,
                "DELETE FROM download_sets WHERE owner = ?1 AND kind = ?2 AND ident = ?3",
                params![owner, kind, ident],
            )?;
            Ok((removed, files))
        })?;

        // Outside the transaction: a file that cannot be unlinked must not roll
        // back the decision the person made. The row is gone, so the leftover
        // is invisible and a later `clear` still takes it.
        for name in files {
            let _ = fs::remove_file(self.cache.offline_dir().join(name));
        }
        Ok(removed)
    }

    /// Removes one track's download (a context menu's "retirer"), files and all.
    /// Returns whether there was one.
    pub fn remove(&self, owner: &str, track: &TrackId) -> Result<bool> {
        let (removed, file) = self.cache.write(|cache, conn| {
            let file: Option<String> = cache
                .query_optional(
                    conn,
                    "SELECT file FROM downloads WHERE owner = ?1 AND track = ?2",
                    params![owner, track.as_str()],
                    |row| row.get(0),
                )?
                .flatten();
            cache.exec(
                conn,
                "DELETE FROM download_members WHERE owner = ?1 AND track = ?2",
                params![owner, track.as_str()],
            )?;
            let removed = cache.exec(
                conn,
                "DELETE FROM downloads WHERE owner = ?1 AND track = ?2",
                params![owner, track.as_str()],
            )?;
            let file = if removed > 0 { file } else { None };
            Ok((removed > 0, file))
        })?;
        if let Some(name) = file {
            let _ = fs::remove_file(self.cache.offline_dir().join(name));
        }
        Ok(removed)
    }

    /// The sets one account chose, newest first, with their progress.
    ///
    /// One statement, however many sets: the two sub-selects count the members
    /// and the published ones, so listing a library downloaded for a trip does
    /// not fan out per set (rule 3).
    pub fn sets(&self, owner: &str) -> Result<Vec<DownloadSet>> {
        let rows = self.cache.read(|conn| {
            self.cache.rows(
                conn,
                "SELECT s.kind, s.ident, s.title, s.added_at, \
                   (SELECT COUNT(*) FROM download_members m \
                     WHERE m.owner = s.owner AND m.kind = s.kind AND m.ident = s.ident), \
                   (SELECT COUNT(*) FROM download_members m \
                     JOIN downloads d ON d.owner = m.owner AND d.track = m.track \
                     WHERE m.owner = s.owner AND m.kind = s.kind AND m.ident = s.ident \
                       AND d.state = 'done') \
                 FROM download_sets s WHERE s.owner = ?1 \
                 ORDER BY s.added_at DESC, s.ident ASC",
                params![owner],
                |row| {
                    Ok(DownloadSet {
                        kind: row.get(0)?,
                        ident: row.get(1)?,
                        title: row.get(2)?,
                        added_at: row.get(3)?,
                        tracks: row.get::<_, i64>(4)?.max(0) as usize,
                        done: row.get::<_, i64>(5)?.max(0) as usize,
                    })
                },
            )
        })?;
        Ok(rows)
    }

    /// The work still to do, oldest first, at most `limit` entries — one page of
    /// the queue, never the whole library (rule 3: nothing fans out unbounded).
    ///
    /// `Active` rows are included on purpose: a restart leaves them there, and
    /// resuming is cheaper than starting over (rule 2).
    pub fn pending(&self, owner: &str, limit: usize) -> Result<Vec<DownloadJob>> {
        let rows = self.cache.read(|conn| {
            self.cache.rows(
                conn,
                "SELECT track, quality, state FROM downloads \
                 WHERE owner = ?1 AND state IN ('pending', 'active') \
                 ORDER BY added_at ASC, track ASC LIMIT ?2",
                params![owner, limit as i64],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
        })?;
        Ok(rows
            .into_iter()
            .filter_map(|(track, quality, state)| {
                Some(DownloadJob {
                    track: TrackId::new(track),
                    quality: parse_quality(&quality)?,
                    state: DownloadState::parse(&state).unwrap_or(DownloadState::Pending),
                })
            })
            .collect())
    }

    /// Records where a track stands, with the cause when it failed.
    ///
    /// Returns `false` when there is no such row anymore (the set was removed
    /// while the worker was fetching it): losing that race is not an error.
    pub fn mark(
        &self,
        owner: &str,
        track: &TrackId,
        state: DownloadState,
        error: Option<&str>,
    ) -> Result<bool> {
        let changed = self.cache.write(|cache, conn| {
            cache.exec(
                conn,
                "UPDATE downloads SET state = ?3, error = ?4 WHERE owner = ?1 AND track = ?2",
                params![owner, track.as_str(), state.as_str(), error],
            )
        })?;
        Ok(changed > 0)
    }

    /// Reserves the file a download's bytes are written to (a `.part`), and
    /// marks the track active.
    pub fn reserve(&self, owner: &str, track: &TrackId, quality: Quality) -> Result<PathBuf> {
        let digest = track_digest(track, quality);
        let partial = self.cache.offline_dir().join(format!("{digest}.part"));
        fs::write(&partial, b"")?;
        let file = format!("{digest}.{}", quality_name(quality));

        let previous = self.cache.write(|cache, conn| {
            let previous: Option<String> = cache
                .query_optional(
                    conn,
                    "SELECT file FROM downloads WHERE owner = ?1 AND track = ?2",
                    params![owner, track.as_str()],
                    |row| row.get(0),
                )?
                .flatten();
            cache.exec(
                conn,
                "INSERT INTO downloads(owner, track, quality, file, bytes, state, error, added_at) \
                 VALUES(?1, ?2, ?3, ?4, NULL, 'active', NULL, ?5) \
                 ON CONFLICT(owner, track) DO UPDATE SET quality = excluded.quality, \
                 file = excluded.file, bytes = NULL, state = 'active', error = NULL",
                params![owner, track.as_str(), quality_name(quality), file, now_ms()],
            )?;
            Ok(previous)
        })?;

        if let Some(previous) = previous
            && previous != file
        {
            // The same track at another quality: the old file would sit in the
            // offline directory, unplayable and unaccounted.
            let _ = fs::remove_file(self.cache.offline_dir().join(previous));
        }
        Ok(partial)
    }

    /// Publishes a reserved download: `.part` renamed into place, length read
    /// back, state done. The file is playable afterwards and only afterwards.
    pub fn complete(&self, owner: &str, track: &TrackId, bytes: u64) -> Result<PathBuf> {
        let (file, quality) = self
            .cache
            .read(|conn| {
                self.cache.query_optional(
                    conn,
                    "SELECT file, quality FROM downloads WHERE owner = ?1 AND track = ?2",
                    params![owner, track.as_str()],
                    |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
                )
            })?
            .ok_or_else(|| {
                IoError::new(
                    ErrorKind::NotFound,
                    format!("aucun téléchargement pour {track}"),
                )
            })?;

        let Some(file) = file else {
            return Err(
                IoError::new(ErrorKind::NotFound, format!("aucun fichier pour {track}")).into(),
            );
        };
        let container = self.cache.offline_dir();
        let target = container.join(&file);
        if let Some(quality) = parse_quality(&quality) {
            let partial = container.join(format!("{}.part", track_digest(track, quality)));
            if partial.is_file() {
                fs::rename(&partial, &target)?;
            }
        }
        if !target.is_file() {
            return Err(IoError::new(
                ErrorKind::NotFound,
                format!("réservation absente pour {track}"),
            )
            .into());
        }

        let length = fs::metadata(&target)?.len();
        if length != bytes {
            debug!(
                track = track.as_str(),
                counted = bytes,
                on_disk = length,
                "completed download: storing the file's own length"
            );
        }

        self.cache.write(|cache, conn| {
            cache.exec(
                conn,
                "UPDATE downloads SET bytes = ?3, state = 'done', error = NULL \
                 WHERE owner = ?1 AND track = ?2",
                params![owner, track.as_str(), length as i64],
            )?;
            Ok(())
        })?;
        Ok(target)
    }

    /// The published file of a track, at the quality it was downloaded at, or
    /// `None`.
    ///
    /// A row whose file disappeared is downgraded to `failed` with that cause
    /// instead of being handed to the decoder (`lib/offline.js` reconciles the
    /// same way at startup).
    pub fn local(&self, owner: &str, track: &TrackId) -> Result<Option<(PathBuf, Quality)>> {
        let row = self.cache.read(|conn| {
            self.cache.query_optional(
                conn,
                "SELECT file, quality FROM downloads \
                 WHERE owner = ?1 AND track = ?2 AND state = 'done'",
                params![owner, track.as_str()],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
            )
        })?;
        let Some((file, quality)) = row else {
            return Ok(None);
        };
        let (Some(file), Some(quality)) = (file, parse_quality(&quality)) else {
            return Ok(None);
        };
        let path = self.cache.offline_dir().join(&file);
        if !path.is_file() {
            self.mark(
                owner,
                track,
                DownloadState::Failed,
                Some("le fichier téléchargé a disparu du disque"),
            )?;
            return Ok(None);
        }
        Ok(Some((path, quality)))
    }

    /// Whether a track is downloaded and playable.
    pub fn is_downloaded(&self, owner: &str, track: &TrackId) -> bool {
        self.local(owner, track)
            .map(|found| found.is_some())
            .unwrap_or(false)
    }

    /// The whole workload of one account, for the badge and the settings screen.
    pub fn progress(&self, owner: &str) -> Result<DownloadProgress> {
        let rows = self.cache.read(|conn| {
            self.cache.rows(
                conn,
                "SELECT state, COUNT(*), COALESCE(SUM(bytes), 0) FROM downloads \
                 WHERE owner = ?1 GROUP BY state",
                params![owner],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
        })?;
        let mut progress = DownloadProgress::default();
        for (state, count, bytes) in rows {
            let count = count.max(0) as usize;
            match DownloadState::parse(&state).unwrap_or(DownloadState::Pending) {
                DownloadState::Pending => progress.pending = count,
                DownloadState::Active => progress.active = count,
                DownloadState::Done => {
                    progress.done = count;
                    progress.bytes += bytes;
                }
                DownloadState::Failed => progress.failed = count,
            }
        }
        Ok(progress)
    }

    /// Every track PUBLISHED on the device (`state = 'done'`), as ids.
    ///
    /// One `SELECT` on the `(owner, state)` index, for the snapshot a screen
    /// reads: a list needs to say « téléchargé » on a row without a second
    /// source of truth and without a query of its own (the GPUI thread never
    /// touches SQLite). The ids are the stream's (`deezer_id`, or the row uuid),
    /// which is what `/api/stream` and the engine's offline seam take.
    pub fn published(&self, owner: &str) -> Result<Vec<String>> {
        self.cache.read(|conn| {
            self.cache.rows(
                conn,
                "SELECT track FROM downloads WHERE owner = ?1 AND state = 'done'",
                params![owner],
                |row| row.get::<_, String>(0),
            )
        })
    }

    /// The cause of every failed download, ready to be printed (rule 2: what
    /// failed and why, never just a count).
    pub fn failures(&self, owner: &str) -> Result<Vec<(TrackId, String)>> {
        let rows = self.cache.read(|conn| {
            self.cache.rows(
                conn,
                "SELECT track, COALESCE(error, 'cause inconnue') FROM downloads \
                 WHERE owner = ?1 AND state = 'failed' ORDER BY added_at ASC",
                params![owner],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
        })?;
        Ok(rows
            .into_iter()
            .map(|(track, error)| (TrackId::new(track), error))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{DownloadState, DownloadTrack, Quality, Result, TrackId, testing::TempDir};

    fn one(track: &str) -> DownloadTrack {
        DownloadTrack {
            track: TrackId::new(track),
            quality: Quality::Opus320,
        }
    }

    #[test]
    fn a_chosen_set_queues_its_tracks_oldest_first() -> Result<()> {
        let dir = TempDir::new("dl-add");
        let cache = dir.open();
        let downloads = cache.downloads();

        let linked = downloads.add_set(
            "42",
            "playlist",
            "p1",
            "Soirée",
            &[one("a"), one("b"), one("c")],
        )?;
        assert_eq!(linked, 3);

        let jobs = downloads.pending("42", 2)?;
        assert_eq!(jobs.len(), 2, "the limit is a page, not the queue");
        assert_eq!(jobs[0].track.as_str(), "a");
        assert_eq!(jobs[0].quality, Quality::Opus320);
        assert_eq!(jobs[0].state, DownloadState::Pending);

        let sets = downloads.sets("42")?;
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].title, "Soirée");
        assert_eq!(sets[0].tracks, 3);
        assert_eq!(sets[0].done, 0);
        Ok(())
    }

    /// **What is ON the device, as ids**: the snapshot a list reads. A pending
    /// row is not published, a complete one is, a failed one is not — and the
    /// ids are the STREAM's, which is what a row's `t` and the engine's offline
    /// seam both look up.
    #[test]
    fn published_lists_exactly_the_complete_tracks() -> Result<()> {
        let dir = TempDir::new("dl-published");
        let cache = dir.open();
        let downloads = cache.downloads();
        downloads.add_set(
            "42",
            "album",
            "a1",
            "Un album",
            &[one("a"), one("b"), one("c")],
        )?;
        assert_eq!(downloads.published("42")?, Vec::<String>::new());

        for track in ["a", "b"] {
            let id = TrackId::new(track);
            let partial = downloads.reserve("42", &id, Quality::Opus320)?;
            fs::write(&partial, vec![1u8; 64])?;
            downloads.complete("42", &id, 64)?;
        }
        downloads.mark(
            "42",
            &TrackId::new("c"),
            DownloadState::Failed,
            Some("HTTP 404"),
        )?;
        let mut published = downloads.published("42")?;
        published.sort();
        assert_eq!(published, vec!["a".to_owned(), "b".to_owned()]);
        assert!(
            downloads.published("another-owner")?.is_empty(),
            "the set is per account, like every other row"
        );
        Ok(())
    }

    #[test]
    fn a_completed_download_is_local_at_its_quality() -> Result<()> {
        let dir = TempDir::new("dl-complete");
        let cache = dir.open();
        let track = TrackId::new("3135556");

        let partial = cache.downloads().reserve("42", &track, Quality::Opus320)?;
        assert_eq!(partial.extension().and_then(|e| e.to_str()), Some("part"));
        assert_eq!(cache.downloads().local("42", &track)?, None);

        fs::write(&partial, vec![9u8; 1234])?;
        let published = cache.downloads().complete("42", &track, 1234)?;
        assert!(published.is_file());
        assert!(!partial.exists());

        assert_eq!(
            cache.downloads().local("42", &track)?,
            Some((published, Quality::Opus320))
        );
        assert!(cache.downloads().is_downloaded("42", &track));
        assert_eq!(cache.downloads().progress("42")?.done, 1);
        assert_eq!(cache.downloads().progress("42")?.bytes, 1234);
        Ok(())
    }

    /// A track in two playlists is downloaded once; removing one playlist
    /// leaves it — and its file — for the other.
    #[test]
    fn removing_a_set_keeps_what_another_set_still_wants() -> Result<()> {
        let dir = TempDir::new("dl-shared");
        let cache = dir.open();
        let downloads = cache.downloads();
        let shared = TrackId::new("shared");

        downloads.add_set(
            "42",
            "playlist",
            "p1",
            "Un",
            std::slice::from_ref(&one("shared")),
        )?;
        downloads.add_set(
            "42",
            "playlist",
            "p2",
            "Deux",
            std::slice::from_ref(&one("shared")),
        )?;
        let partial = downloads.reserve("42", &shared, Quality::Opus320)?;
        fs::write(&partial, vec![1u8; 512])?;
        let file = downloads.complete("42", &shared, 512)?;

        assert_eq!(
            downloads.remove_set("42", "playlist", "p1")?,
            0,
            "still wanted by p2"
        );
        assert!(file.is_file());
        assert!(downloads.local("42", &shared)?.is_some());
        assert_eq!(downloads.sets("42")?.len(), 1);

        assert_eq!(downloads.remove_set("42", "playlist", "p2")?, 1);
        assert!(!file.exists(), "the last set's removal took the file");
        assert_eq!(downloads.local("42", &shared)?, None);
        assert!(downloads.sets("42")?.is_empty());
        Ok(())
    }

    #[test]
    fn a_failure_keeps_its_cause() -> Result<()> {
        let dir = TempDir::new("dl-failure");
        let cache = dir.open();
        let downloads = cache.downloads();
        let track = TrackId::new("x");

        downloads.add_set("42", "album", "a1", "Album", &[one("x")])?;
        assert!(downloads.mark(
            "42",
            &track,
            DownloadState::Failed,
            Some("le serveur n'a pas répondu (503, réessai dans 30 s)"),
        )?);

        let failures = downloads.failures("42")?;
        assert_eq!(failures.len(), 1);
        assert!(failures[0].1.contains("503"));
        assert_eq!(downloads.progress("42")?.failed, 1);
        Ok(())
    }

    #[test]
    fn a_missing_file_is_reported_not_handed_to_the_decoder() -> Result<()> {
        let dir = TempDir::new("dl-heal");
        let cache = dir.open();
        let downloads = cache.downloads();
        let track = TrackId::new("gone");

        let partial = downloads.reserve("42", &track, Quality::Opus64)?;
        fs::write(&partial, vec![1u8; 64])?;
        let file = downloads.complete("42", &track, 64)?;
        fs::remove_file(&file)?;

        assert_eq!(downloads.local("42", &track)?, None);
        assert_eq!(downloads.progress("42")?.failed, 1);
        assert!(downloads.failures("42")?[0].1.contains("disparu"));
        Ok(())
    }

    #[test]
    fn an_interrupted_job_is_still_pending_after_a_restart() -> Result<()> {
        let dir = TempDir::new("dl-resume");
        {
            let cache = dir.open();
            cache
                .downloads()
                .add_set("42", "show", "s1", "Podcast", &[one("e1"), one("e2")])?;
            cache
                .downloads()
                .mark("42", &TrackId::new("e1"), DownloadState::Active, None)?;
        }

        let cache = dir.open();
        let jobs = cache.downloads().pending("42", 10)?;
        assert_eq!(
            jobs.len(),
            2,
            "the active row resumes instead of restarting"
        );
        assert!(
            jobs.iter().any(|job| job.state == DownloadState::Active),
            "the interrupted state is visible"
        );
        Ok(())
    }

    #[test]
    fn accounts_never_see_each_others_downloads() -> Result<()> {
        let dir = TempDir::new("dl-owners");
        let cache = dir.open();

        cache
            .downloads()
            .add_set("42", "album", "a1", "Mien", &[one("t1")])?;
        cache
            .downloads()
            .add_set("7", "album", "a2", "Autre", &[one("t2")])?;

        assert_eq!(cache.downloads().pending("42", 10)?.len(), 1);
        assert_eq!(cache.downloads().pending("7", 10)?.len(), 1);
        assert_eq!(cache.downloads().sets("42")?[0].title, "Mien");
        Ok(())
    }

    /// Criterion: "each failure's cause" and "resume after restart" are stored,
    /// not recomputed — one mark is one statement.
    #[test]
    fn marking_progress_is_one_statement() -> Result<()> {
        let dir = TempDir::new("dl-statements");
        let cache = dir.open();
        cache
            .downloads()
            .add_set("42", "album", "a1", "Album", &[one("t1")])?;

        let before = cache.statements();
        cache
            .downloads()
            .mark("42", &TrackId::new("t1"), DownloadState::Active, None)?;
        assert_eq!(cache.statements() - before, 1);
        Ok(())
    }
}
