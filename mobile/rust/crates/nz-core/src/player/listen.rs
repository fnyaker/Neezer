//! What the player reports back: listens, and an episode's position.
//!
//! Owned by **MB3b**. Two reports, both ported, both fire-and-forget — a lost
//! report is never allowed to surface as an error on the person's screen
//! (`webapp/src/lib/api.js#reportListen`: *never let it break playback*).
//!
//! # The listen
//!
//! `POST /api/listen {id, listened, next_id, context, shuffle}` on every track
//! change, with **the seconds actually PLAYED** — pauses excluded, and the
//! accounting is `nz-player`'s: `now_ms` closes the mark, and only playing time
//! counts. The player never sends a listen for a row it did not play, and it
//! sends none while offline: a doomed POST per track change is just noise on a
//! bad link (the SPA's own rule, and `Api::report_listen` skips it too).
//!
//! # The episode's position
//!
//! `POST /api/podcast/progress {episode_id, position, duration, finished}` on
//! **pause, every 30 s while playing, and at quit** (`podcastProgress.js`), with
//! the SPA's two thresholds: below [`MIN_SAVE_S`] (8 s) a position is a mistaken
//! tap and is ignored; within [`DONE_TAIL_S`] (15 s) of the end the episode is
//! `finished`, so a completed episode never offers to resume from the credits.
//!
//! **Offline, the write goes to `pending_progress`** (`nz-cache`) and is sent on
//! reconnection, last write wins — the server keeps the authoritative copy, the
//! phone keeps the instant one. The local mirror is `kv` under
//! [`KEY_PROGRESS`], capped at [`MAX_REMEMBERED`] episodes, which is what the
//! player reads to RESUME an episode where it was left, days later.

use std::collections::BTreeMap;
use std::sync::Arc;

use nz_cache::Cache;
use serde::{Deserialize, Serialize};

use super::source::block_on;

/// Where the player keeps the last position it knows for each episode.
pub const KEY_PROGRESS: &str = "podcast.progress";
/// The SPA's cap on remembered episodes (`podcastProgress.js` `MAX`).
pub const MAX_REMEMBERED: usize = 400;
/// A position below this is an accidental tap, not a place to resume.
pub const MIN_SAVE_S: f64 = 8.0;
/// Within this many seconds of the end, an episode is finished.
pub const DONE_TAIL_S: f64 = 15.0;
/// How often a playing episode pushes its position (`SYNC_MIN_MS`, 30 s).
pub const SYNC_EVERY_MS: u64 = 30_000;

/// One remembered position, exactly as the SPA keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodeProgress {
    /// Seconds into the episode.
    pub t: f64,
    /// The episode's length, when it is known.
    pub d: f64,
    /// When it was written, in milliseconds — the merge's "newest wins".
    pub at: u64,
    /// The episode played to its end: never offer to resume.
    pub done: bool,
}

/// The reports, over the core's own client.
pub struct Reports {
    cache: Cache,
    core: Arc<crate::core::Core>,
    runtime: tokio::runtime::Handle,
    gone: tokio::sync::watch::Receiver<bool>,
}

impl Reports {
    pub fn new(
        core: Arc<crate::core::Core>,
        runtime: tokio::runtime::Handle,
        gone: tokio::sync::watch::Receiver<bool>,
    ) -> Reports {
        let cache = core.cache().clone();
        Reports {
            cache,
            core,
            runtime,
            gone,
        }
    }

    /// `POST /api/listen`, with the seconds the row was actually heard.
    pub fn listen(&mut self, payload: &nz_model::ListenPayload) {
        let Ok(api) = self.core.api() else {
            tracing::debug!(id = %payload.id, "offline: no listen sent");
            return;
        };
        let answer = block_on(&self.runtime, &mut self.gone, api.report_listen(payload));
        match answer {
            Some(Ok(())) => {
                tracing::debug!(id = %payload.id, listened = payload.listened, "listen sent")
            }
            Some(Err(error)) => tracing::debug!(%error, "listen refused"),
            None => {}
        }
    }

    /// One episode's position: the local mirror first (it is the instant one),
    /// then the server — or `pending_progress` when there is no link.
    pub fn podcast_position(&mut self, id: &str, t: f64, d: f64, now_ms: u64) {
        let Some(entry) = self.remember_and_store(id, t, d, now_ms) else {
            return;
        };
        self.push(id, &entry);
    }

    /// One episode finished by itself: `finished: true`, kept so the show page
    /// can say "Terminé" and no resume is offered into the credits.
    pub fn podcast_finished(&mut self, id: &str, d: f64, now_ms: u64) {
        let entry = EpisodeProgress {
            t: d,
            d,
            at: now_ms,
            done: true,
        };
        let mut map = self.map();
        map.insert(id.to_owned(), entry.clone());
        self.store(map);
        self.push(id, &entry);
    }

    /// Where an episode should resume, or `None` for "from the start": never
    /// inside the first 8 s, never inside the last 15, never a finished one.
    pub fn resume_point(&self, id: &str, duration_s: f64) -> Option<f64> {
        let map = self.map();
        let entry = map.get(id)?;
        if entry.done || entry.t < MIN_SAVE_S {
            return None;
        }
        if duration_s > 0.0 && entry.t >= duration_s - DONE_TAIL_S {
            return None;
        }
        Some(entry.t)
    }

    /// Sends every pending position, last write wins. Called when the link comes
    /// back and once at the start of a session; bounded, because it is a
    /// catch-up, not a bulk import.
    pub fn flush_pending(&mut self) {
        let Some(owner) = self.core.me().map(|me| me.user.id.to_string()) else {
            return;
        };
        let pending = self
            .cache
            .pending_progress()
            .pending(&owner)
            .unwrap_or_default();
        if pending.is_empty() || self.core.api().is_err() {
            return;
        }
        for (episode, body) in pending.iter().take(25) {
            let row: PendingBody = serde_json::from_str(body).unwrap_or_default();
            let Ok(api) = self.core.api() else {
                return;
            };
            let answer = block_on(
                &self.runtime,
                &mut self.gone,
                api.save_podcast_progress(episode, row.position, row.duration, row.finished),
            );
            match answer {
                Some(Ok(())) => {
                    // `put` with the same body marks it sent (`PendingProgress`
                    // keeps one row per episode: last write wins).
                    let _ = self.cache.pending_progress().put(&owner, episode, body);
                    tracing::debug!(episode = %episode, "pending position flushed");
                }
                // Still no link: the rest waits for the next flush.
                _ => return,
            }
        }
    }

    /// Remembers a position WITHOUT the network, and says whether it was
    /// rememberable at all (the two thresholds). The podcast screens read it
    /// back through [`Reports::resume_point`] and the mirror.
    pub fn remember_only(&mut self, id: &str, t: f64, d: f64, now_ms: u64) -> bool {
        self.remember_and_store(id, t, d, now_ms).is_some()
    }

    /// The whole mirror, for the show page's resume bars (a cheap read).
    pub fn mirror(&self) -> BTreeMap<String, EpisodeProgress> {
        self.map()
    }

    fn push(&mut self, id: &str, entry: &EpisodeProgress) {
        let owner = self.core.me().map(|me| me.user.id.to_string());
        let position = entry.t.round().max(0.0) as u32;
        let duration = entry.d.round().max(0.0) as u32;
        let body = PendingBody {
            position,
            duration,
            finished: entry.done,
        };
        let Ok(api) = self.core.api() else {
            self.remember_pending(owner, id, &body);
            return;
        };
        let answer = block_on(
            &self.runtime,
            &mut self.gone,
            api.save_podcast_progress(id, position, duration, entry.done),
        );
        match answer {
            Some(Ok(())) => {
                // Keep the local copy in the pending table too: it is the row
                // `sent()` reads, and the next flush must not resend it.
                self.remember_pending(owner, id, &body);
            }
            Some(Err(error)) => {
                tracing::debug!(%error, "position refused; kept for the next link");
                self.remember_pending(owner, id, &body);
            }
            None => {}
        }
    }

    fn remember_pending(&self, owner: Option<String>, id: &str, body: &PendingBody) {
        let Some(owner) = owner else {
            return;
        };
        let Ok(text) = serde_json::to_string(body) else {
            return;
        };
        let _ = self.cache.pending_progress().put(&owner, id, &text);
    }

    fn remember_and_store(
        &mut self,
        id: &str,
        t: f64,
        d: f64,
        now_ms: u64,
    ) -> Option<EpisodeProgress> {
        let mut map = self.map();
        let entry = Self::remember(&mut map, id, t, d, now_ms)?;
        self.store(map);
        Some(entry)
    }

    /// The SPA's two thresholds, in one place: `None` means "do not remember".
    fn remember(
        map: &mut BTreeMap<String, EpisodeProgress>,
        id: &str,
        t: f64,
        d: f64,
        now_ms: u64,
    ) -> Option<EpisodeProgress> {
        if !t.is_finite() || id.is_empty() {
            return None;
        }
        let d = if d.is_finite() && d > 0.0 { d } else { 0.0 };
        // A clip shorter than the done tail would be "finished" at its first
        // second: only a real episode length can be near its own end.
        if d > DONE_TAIL_S && t >= d - DONE_TAIL_S {
            let entry = EpisodeProgress {
                t: d,
                d,
                at: now_ms,
                done: true,
            };
            map.insert(id.to_owned(), entry.clone());
            return Some(entry);
        }
        if t < MIN_SAVE_S {
            return None;
        }
        let entry = EpisodeProgress {
            t,
            d,
            at: now_ms,
            done: false,
        };
        map.insert(id.to_owned(), entry.clone());
        Some(entry)
    }

    fn map(&self) -> BTreeMap<String, EpisodeProgress> {
        self.cache.kv().get(KEY_PROGRESS).unwrap_or_default()
    }

    /// Writes the mirror back, newest first when the cap is reached (the SPA
    /// drops the oldest-touched entries exactly this way).
    fn store(&self, map: BTreeMap<String, EpisodeProgress>) {
        let mut map = map;
        if map.len() > MAX_REMEMBERED {
            let mut rows: Vec<(String, EpisodeProgress)> = map.into_iter().collect();
            rows.sort_by_key(|(_, entry)| std::cmp::Reverse(entry.at));
            rows.truncate(MAX_REMEMBERED);
            map = rows.into_iter().collect();
        }
        if let Err(error) = self.cache.kv().set(KEY_PROGRESS, &map) {
            tracing::debug!(%error, "position mirror not written");
        }
    }
}

/// What `pending_progress` holds: the POST body, reduced to what `save_
/// podcast_progress` re-sends.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
struct PendingBody {
    #[serde(default)]
    position: u32,
    #[serde(default)]
    duration: u32,
    #[serde(default)]
    finished: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remember(id: &str, t: f64, d: f64) -> Option<EpisodeProgress> {
        let mut map = BTreeMap::new();
        Reports::remember(&mut map, id, t, d, 1_000)
    }

    /// The SPA's two thresholds: under 8 s is a tap, within 15 s of the end is
    /// finished, and the end is stored with its duration so a resume is never
    /// offered from the credits.
    #[test]
    fn a_position_is_remembered_between_the_two_thresholds() {
        assert!(remember("e", 3.0, 600.0).is_none(), "a mistaken tap");
        let entry = remember("e", 8.0, 600.0).expect("eight seconds is a position");
        assert_eq!(entry.t, 8.0);
        assert!(!entry.done);

        let near_end = remember("e", 590.0, 600.0).expect("finished");
        assert!(near_end.done);
        assert_eq!(near_end.t, 600.0, "a finished episode sits at its duration");

        assert!(remember("", 100.0, 600.0).is_none());
        assert!(remember("e", f64::NAN, 600.0).is_none());
    }

    /// The listen payload is the server's own shape (`nz-model`'s), and an
    /// absent `next_id` is ABSENT rather than `null`.
    #[test]
    fn the_listen_payload_is_the_servers_shape() {
        let payload = nz_model::ListenPayload {
            id: "abc".into(),
            listened: 42,
            next_id: None,
            context: None,
            shuffle: false,
        };
        let value = serde_json::to_value(&payload).expect("serialises");
        assert_eq!(value["id"], serde_json::json!("abc"));
        assert_eq!(value["listened"], serde_json::json!(42));
        assert!(
            value["next_id"].is_null(),
            "an absent next id is null, never a made-up row: {value}"
        );
    }
}
