//! Podcasts: the subscriptions, a show and its episodes, this person's positions
//! and markers, the chapters of an episode, its transcript, and the server's
//! transcription controls. Owned by **MB13**
//! (docs/agents/mobile.md, "Module and file ownership"): the lot fills this
//! directory and edits no other file of the crate.
//!
//! The behaviour is the SPA's podcast screens, PORTED — not re-invented. What
//! that means, file by file and section by section:
//!
//! | This module | The SPA |
//! |---|---|
//! | [`Core::podcasts`] | `routes/Podcasts.svelte#load` → `api.podcasts()` |
//! | [`Core::search_podcasts`] | `Podcasts.svelte#runSearch` (300 ms after the last key, 1 request) |
//! | [`Core::subscribe_podcast`] | `Podcasts.svelte#onSubmit` + `lib/actions.js#subscribeToPodcast` |
//! | [`Core::unsubscribe_podcast`] | `Show.svelte#unsubscribe` |
//! | [`Core::show`] / [`Core::show_more`] | `Show.svelte#load` / `#loadMoreEpisodes` (cursor, 50 a page) |
//! | [`Core::auto_archive`] | `Show.svelte#toggleAuto` |
//! | [`Core::set_show_transcribe`] | `Show.svelte#toggleShowTranscribe`/`#setShowLanguage` |
//! | [`Core::set_episode_transcription`] | `Show.svelte#startOne`/`#startBatch`/`#stopTranscript`, `TranscriptPanel`'s start/stop/delete |
//! | [`Core::position`] | `lib/podcastProgress.js#getEpisodeProgress`, merged with `/api/podcast/progress` |
//! | [`Core::show_markers`], [`Core::set_marker`], [`Core::remove_marker`] | `lib/markers.js` |
//! | [`Core::chapters`] | `lib/chapters.js#loadChapters` |
//! | [`Core::transcript`] | `TranscriptPanel.svelte#loadPage`/`#poll`/`#runSearch` |
//! | [`Core::show_transcript_search`] | `Show.svelte#runShowSearch` |
//! | [`Core::transcribe_settings`], [`Core::transcribe_status`] | `lib/transcript.js#loadTranscribeSettings`/`#loadTranscribeStatus` |
//!
//! # The request budget (rule 3)
//!
//! A show's cold open costs **2** requests — `/api/podcast/<id>` and
//! `/api/podcast/<id>/markers` — which is the SPA's own count (`Show.svelte`
//! fires both in `load`), and a warm open **0 blocking** ones: the device's copy
//! paints first (that is the SPA's `apicache` role) and the two reads replace it.
//! The next page of episodes is ONE request per 50 rows, and the panel reads a
//! transcript page only when it does not already hold it — a ready transcript
//! costs **0** while listening, which is the SPA's measured `0/1/0`.
//!
//! # The episode list is windowed AND paged (rule 4)
//!
//! 2 000 episodes never cross to Kotlin: the device keeps the loaded pages
//! (`EPISODE_PAGE` = 50 rows each, the server's own default) and the screen holds
//! a `ListHandle` over them, so it paints 64 rows at a time and asks for the next
//! cursor when the viewport reaches the end. A show of 2 000 episodes is 40
//! requests over its whole life, never 2 000 rows in one answer.
//!
//! # The position is the PLAYER's (MB3b), not a second store
//!
//! `podcastProgress.js` keeps one map: written by the player while it plays,
//! merged with the server's copy at the session's start. The phone keeps the
//! same split — the player writes its mirror (`player::listen`) and this module
//! only READS it, adding what `/api/podcast/progress` says — so an episode
//! resumes exactly where it stopped with the link down, and no write happens
//! twice. The mirror's key is `podcast.progress` (MB3b's `player::listen`);
//! `player/mod.rs` does not re-export it, and this lot may not edit that file,
//! so the literal lives in [`KEY_PROGRESS`] with a test that a stored position
//! round-trips through it.

mod chapters;
mod markers;
mod store;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;
mod text;
mod transcript_cache;
mod views;
mod wire;

use std::collections::BTreeMap;

use nz_api::features::Feature;
use serde::{Deserialize, Serialize};

use crate::core::{Core, CoreError};
use crate::lists::Row;
use crate::player::EpisodeProgress;

pub use text::{
    PageInfo, TextPart, batch_options, chapter_count_label, chapter_label, fold_text,
    highlight_parts, language_label, page_info, reason_text,
};
pub use views::{
    ChapterCard, Chapters, EpisodeCard, Marker, Position, Segment, ShowCard, ShowPage, ShowSearch,
    TranscriptHit, TranscriptInfo, TranscriptPage, TranscriptSearch,
};

/// The tag every request of this area is counted under (`NzCore::stats`).
pub const TAG_PODCASTS: &str = "podcasts";

/// `podcast.progress` — the key MB3b's player writes its position mirror under
/// (`player::listen::KEY_PROGRESS`, which `player/mod.rs` does not re-export; this
/// lot reads it and never writes it).
pub const KEY_PROGRESS: &str = "podcast.progress";

/// How many episodes one page holds (`api.js#podcast`'s `limit = 50`).
pub const EPISODE_PAGE: u32 = 50;

/// The server's own ceiling for one page (`PODCAST_EPISODES_MAX`).
pub const EPISODE_PAGE_MAX: u32 = 200;

/// One transcript page: the SPA's `TranscriptPanel` `PAGE`.
pub const TRANSCRIPT_PAGE: u32 = 200;

/// One page of a transcript SEARCH (`SEARCH_PAGE`).
pub const TRANSCRIPT_SEARCH_PAGE: u32 = 100;

/// The `limit=1` read that only asks for the STATUS of a transcript
/// (`TranscriptPanel#fetchMeta`, `#poll`).
pub const TRANSCRIPT_META: u32 = 1;

/// How many characters a show search needs before it is sent (`onSearchInput`).
pub const SHOW_SEARCH_MIN: usize = 2;

/// How many hits a show search asks for (`runShowSearch`'s `50`).
pub const SHOW_SEARCH_LIMIT: u32 = 50;

/// The SPA's debounce for a search field, in milliseconds: one request after the
/// last keystroke, never one per key (`onSearchInput`'s `300`).
pub const SEARCH_DEBOUNCE_MS: u64 = 300;

/// How often the panel polls a transcription in progress (`POLL_MS`).
pub const TRANSCRIPT_POLL_MS: u64 = 10_000;

impl Core {
    // -- the subscriptions ----------------------------------------------------

    /// The shows this device kept, for the first frame — disk only, no request.
    pub fn cached_podcasts(&self) -> Option<Vec<ShowCard>> {
        let body = store::read(self, store::SHOWS, "")?;
        let stored: StoredShows = serde_json::from_str(&body).ok()?;
        Some(stored.shows)
    }

    /// `GET /api/podcasts` — every show this server holds, the unsubscribed ones
    /// included (`webui`'s `podcasts`: the audio is on the server and must stay
    /// reachable).
    pub async fn podcasts(&self) -> Result<Vec<ShowCard>, CoreError> {
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_shows(&api).await;
        self.note_podcasts_traffic(&api, before);
        let shows: Vec<ShowCard> = answer?.podcasts.iter().map(ShowCard::from).collect();
        write_shows(self, &shows);
        Ok(shows)
    }

    /// `GET /api/search/podcasts?q=` — the provider's catalogue, one request.
    pub async fn search_podcasts(&self, query: &str) -> Result<Vec<ShowCard>, CoreError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::search_podcasts(&api, query).await;
        self.note_podcasts_traffic(&api, before);
        Ok(answer?.podcasts.iter().map(ShowCard::from_card).collect())
    }

    /// `POST /api/podcasts {url}` — subscribe by a Deezer show URL (a catalogue
    /// card builds it with [`ShowCard::subscribe_url`]). The answer IS the show's
    /// page, so the screen it opens needs no request of its own.
    ///
    /// Optimistic in the only sense that has a state to roll back: the show is
    /// added to the device's list BEFORE the request would land, and a refusal
    /// leaves the list exactly as it was.
    pub async fn subscribe_podcast(&self, url: &str) -> Result<ShowCard, CoreError> {
        let url = url.trim();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(CoreError::Invalid {
                detail: "Un lien de podcast doit commencer par http:// ou https://".to_owned(),
            });
        }
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::subscribe(&api, url).await;
        self.note_podcasts_traffic(&api, before);
        let page = build_page(self, &answer?);
        let card = page.show.clone();
        store_page(self, &page);
        let mut shows = self.cached_podcasts().unwrap_or_default();
        shows.retain(|show| show.id != card.id);
        shows.push(card.clone());
        shows.sort_by_key(|show| show.title.to_lowercase());
        write_shows(self, &shows);
        Ok(card)
    }

    /// `DELETE /api/podcast/<id>` — unsubscribe. The archive STAYS: a show with
    /// audio on disk is kept and simply stops syncing (`webui`'s
    /// `unsubscribe_podcast`), which is why the list is re-read afterwards
    /// instead of assuming the row is gone.
    pub async fn unsubscribe_podcast(&self, id: &str) -> Result<(), CoreError> {
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::unsubscribe(&api, id).await;
        self.note_podcasts_traffic(&api, before);
        answer?;
        store::forget(self, store::SHOW, id);
        // One re-read: the server decides whether the row survived (an archived
        // show stays listed with `subscribed: false`).
        self.podcasts().await?;
        Ok(())
    }

    // -- one show -------------------------------------------------------------

    /// The show this device kept: its header, the episodes loaded so far and the
    /// cursor of the next page — disk only, no request. This is what a warm open
    /// paints, including the pages read on an earlier visit.
    pub fn cached_show(&self, id: &str) -> Option<ShowPage> {
        let body = store::read(self, store::SHOW, id)?;
        let page: ShowPage = serde_json::from_str(&body).ok()?;
        Some(self.with_positions(page))
    }

    /// `GET /api/podcast/<id>?limit=50` — the show's first page, plus its markers
    /// in the same visit (two requests, the SPA's own count).
    ///
    /// The markers are a nicety: a refusal there never fails the page
    /// (`markers.js#loadShowMarkers` catches and says nothing).
    pub async fn show(&self, id: &str) -> Result<ShowPage, CoreError> {
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_show(&api, id, EPISODE_PAGE, None).await;
        let markers = wire::show_markers(&api, id).await;
        self.note_podcasts_traffic(&api, before);

        let page = build_page(self, &answer?);
        if let Ok(show_markers) = markers {
            let mut grouped: BTreeMap<String, Vec<Marker>> = BTreeMap::new();
            for (episode, list) in show_markers.markers {
                grouped.insert(episode, list.iter().map(Marker::from).collect());
            }
            markers::merge_show(self, grouped);
        }
        let page = self.with_positions(page);
        store_page(self, &page);
        Ok(page)
    }

    /// `GET /api/podcast/<id>?cursor=…` — the NEXT page, appended to what the
    /// device already holds (one request, and the list grows by 50 rows).
    ///
    /// A show with no cursor left answers the page it already has, so a screen
    /// that asks twice pays nothing extra.
    pub async fn show_more(&self, id: &str) -> Result<ShowPage, CoreError> {
        let Some(mut page) = self.cached_show(id) else {
            return self.show(id).await;
        };
        let Some(cursor) = page.next.clone() else {
            return Ok(page);
        };
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_show(&api, id, EPISODE_PAGE, Some(&cursor)).await;
        self.note_podcasts_traffic(&api, before);
        let next = build_page(self, &answer?);
        append_page(&mut page, next);
        let page = self.with_positions(page);
        store_page(self, &page);
        Ok(page)
    }

    /// The episodes `start..start+count` this device holds, for the rows a
    /// windowed list is about to draw. No request: the page(s) are already in.
    pub fn episode_cards(&self, id: &str, start: u32, count: u32) -> Vec<EpisodeCard> {
        let Some(page) = self.cached_show(id) else {
            return Vec::new();
        };
        let start = start as usize;
        page.episodes
            .into_iter()
            .skip(start)
            .take(count.min(EPISODE_PAGE_MAX) as usize)
            .collect()
    }

    /// `POST /api/podcast/<id>/auto_archive` — archive this show's new episodes
    /// (and queue the ones it lists). Optimistic: the switch moves first, and a
    /// refusal puts it back.
    pub async fn auto_archive(&self, id: &str, on: bool) -> Result<ShowArchiveAnswer, CoreError> {
        let previous = self.cached_show(id);
        if let Some(mut page) = previous.clone() {
            page.show.auto_archive = on;
            store_page(self, &page);
        }
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::auto_archive(&api, id, on).await;
        self.note_podcasts_traffic(&api, before);
        match answer {
            Ok(answer) => {
                // The answer is the channel's payload flattened beside `queued`,
                // so it decodes as the show it is.
                let card = serde_json::from_value::<nz_model::PodcastShow>(
                    serde_json::Value::Object(answer.show.clone()),
                )
                .map(|show| ShowCard::from(&show))
                .unwrap_or_else(|_| {
                    previous
                        .as_ref()
                        .map(|page| page.show.clone())
                        .unwrap_or_default()
                });
                let mut card = card;
                // The answer is the channel WITHOUT episodes: the loaded rows,
                // the cursor and the transcription counters stay this device's.
                if let Some(page) = previous {
                    card.transcribed_count = page.show.transcribed_count;
                    let mut page = page;
                    page.show = card.clone();
                    store_page(self, &page);
                }
                Ok(ShowArchiveAnswer {
                    show: card,
                    queued: answer.queued,
                })
            }
            Err(cause) => {
                if let Some(page) = previous {
                    store_page(self, &page);
                }
                Err(cause)
            }
        }
    }

    /// `POST /api/podcast/<id>/transcribe` — this show's transcription switch,
    /// its language and its backlog mode. The answer says what the server queued
    /// and what it skipped (rule 2: a job says what it did).
    pub async fn set_show_transcribe(
        &self,
        id: &str,
        auto: Option<bool>,
        backlog: Option<&str>,
        language: Option<&str>,
    ) -> Result<ShowTranscribeAnswer, CoreError> {
        if auto.is_none() && backlog.is_none() && language.is_none() {
            return Err(CoreError::Invalid {
                detail: "Aucun réglage de transcription à enregistrer".to_owned(),
            });
        }
        let mut payload = serde_json::Map::new();
        if let Some(auto) = auto {
            payload.insert("auto".to_owned(), serde_json::Value::Bool(auto));
        }
        if let Some(backlog) = backlog {
            if !["all", "recent", "none"].contains(&backlog) {
                return Err(CoreError::Invalid {
                    detail: "Le rattrapage doit être all, recent ou none".to_owned(),
                });
            }
            payload.insert(
                "backlog".to_owned(),
                serde_json::Value::String(backlog.to_owned()),
            );
        }
        if let Some(language) = language {
            payload.insert(
                "language".to_owned(),
                serde_json::Value::String(language.to_owned()),
            );
        }
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::set_show_transcribe(&api, id, &serde_json::Value::Object(payload)).await;
        self.note_podcasts_traffic(&api, before);
        let answer = answer?;
        let card = serde_json::from_value::<nz_model::PodcastShow>(answer.clone())
            .ok()
            .map(|show| ShowCard::from(&show));
        // The answer is the channel alone: what the DEVICE knew about the page
        // (the transcription counter, the loaded rows) stays.
        if let Some(cached) = self.cached_show(id) {
            let mut page = cached;
            if let Some(fresh) = card.clone() {
                let mut merged = fresh;
                merged.transcribed_count = page.show.transcribed_count;
                page.show = merged;
                store_page(self, &page);
            }
        }
        let skipped = answer.get("skipped").and_then(|value| match value {
            serde_json::Value::Number(number) => number.as_u64(),
            // The route sums a per-code map into a number for `auto`, but an
            // older server may send the map itself: add its entries.
            serde_json::Value::Object(map) => Some(
                map.values()
                    .filter_map(serde_json::Value::as_u64)
                    .sum::<u64>(),
            ),
            _ => None,
        });
        Ok(ShowTranscribeAnswer {
            show: card,
            queued: answer
                .get("queued")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as u32,
            skipped: skipped.unwrap_or(0) as u32,
            backlog: answer
                .get("backlog")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("all")
                .to_owned(),
        })
    }

    // -- the person's positions -----------------------------------------------

    /// Every position this person has, the device's mirror merged with the
    /// server's copy — the SPA's `initPodcastProgress`. One request; the merge is
    /// newest-wins on both sides, and a local entry the server has not seen is
    /// flagged `pending` until `pending_progress` is flushed.
    pub async fn progress(&self) -> Result<BTreeMap<String, Position>, CoreError> {
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_progress(&api).await;
        self.note_podcasts_traffic(&api, before);
        let remote = answer?;
        let mut stored = StoredProgress::default();
        for (episode, row) in remote.progress {
            stored.entries.insert(
                episode,
                Position {
                    seconds: row.position,
                    duration_s: row.duration,
                    finished: row.finished,
                    at_ms: (row.updated.max(0) as u64) * 1_000,
                    pending: false,
                },
            );
        }
        write_progress(self, &stored);
        Ok(self.positions())
    }

    /// The merged map, disk only: this is what the show page's resume bars and an
    /// episode's `position` read, with **0 requests** — an episode resumes at its
    /// position on a dead link (the acceptance's row).
    pub fn positions(&self) -> BTreeMap<String, Position> {
        merge_positions(
            self.cached_progress(),
            self.player_mirror(),
            self.pending_episodes(),
        )
    }

    /// One episode's position, merged the same way, or `None` when there is
    /// nothing worth resuming.
    pub fn position(&self, episode_id: &str) -> Option<Position> {
        let merged = self.positions().remove(episode_id)?;
        if merged.finished {
            return Some(merged);
        }
        if merged.seconds < 8 {
            // `podcastProgress.js#MIN_SAVE`: an accidental tap is not a place.
            return None;
        }
        Some(merged)
    }

    /// The server's copy this device kept, for the merge above.
    fn cached_progress(&self) -> BTreeMap<String, Position> {
        let Some(body) = store::read(self, store::PROGRESS, "") else {
            return BTreeMap::new();
        };
        serde_json::from_str::<StoredProgress>(&body)
            .map(|stored| stored.entries)
            .unwrap_or_default()
    }

    /// The player's mirror, read through MB3b's key. It is the INSTANT copy: the
    /// player writes it on pause, on a track change and every 30 s, with or
    /// without a link.
    fn player_mirror(&self) -> BTreeMap<String, EpisodeProgress> {
        self.cache().kv().get(KEY_PROGRESS).unwrap_or_default()
    }

    /// The episodes whose last write the server has not accepted yet
    /// (`nz-cache`'s `pending_progress`, which the player's flush drains).
    fn pending_episodes(&self) -> std::collections::HashSet<String> {
        let Some(owner) = self.me().map(|me| me.user.id.to_string()) else {
            return std::collections::HashSet::new();
        };
        self.cache()
            .pending_progress()
            .pending(&owner)
            .unwrap_or_default()
            .into_iter()
            .map(|(episode, _)| episode)
            .collect()
    }

    /// The same page with every episode's resume position filled in from the
    /// merged map — done on the way OUT, so a write to the copy never freezes a
    /// position that the player is still moving.
    fn with_positions(&self, mut page: ShowPage) -> ShowPage {
        let positions = self.positions();
        for episode in &mut page.episodes {
            episode.position = positions
                .get(&episode.id)
                .filter(|position| position.finished || position.seconds >= 8)
                .cloned();
        }
        page
    }

    // -- markers --------------------------------------------------------------

    /// `GET /api/podcast/<id>/markers` — every marker of a show, by episode.
    pub async fn show_markers(&self, id: &str) -> Result<(), CoreError> {
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::show_markers(&api, id).await;
        self.note_podcasts_traffic(&api, before);
        let mut grouped: BTreeMap<String, Vec<Marker>> = BTreeMap::new();
        for (episode, list) in answer?.markers {
            grouped.insert(episode, list.iter().map(Marker::from).collect());
        }
        markers::merge_show(self, grouped);
        Ok(())
    }

    /// One episode's markers, disk only — what the chips under a row read.
    pub fn episode_markers(&self, id: &str) -> Vec<Marker> {
        markers::for_episode(self, id)
    }

    /// `GET /api/podcast/episode/<id>/markers` — one episode's markers, for a
    /// panel opened from the player rather than from the show page (the show
    /// page's own answer already carried them).
    pub async fn load_episode_markers(&self, id: &str) -> Result<Vec<Marker>, CoreError> {
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::episode_markers(&api, id).await;
        self.note_podcasts_traffic(&api, before);
        let markers: Vec<Marker> = answer?.markers.iter().map(Marker::from).collect();
        markers::put_episode(self, id, markers.clone());
        Ok(markers)
    }

    /// `POST /api/podcast/episode/<id>/markers` — a marker at `position_ms`
    /// (MILLISECONDS). Optimistic: the chip appears before the answer, and a
    /// refusal removes it and reports the cause.
    pub async fn add_marker(
        &self,
        id: &str,
        position_ms: u32,
        label: Option<&str>,
    ) -> Result<Marker, CoreError> {
        let label = label.map(str::trim).filter(|label| !label.is_empty());
        let label = label.unwrap_or_default().to_owned();
        let local = Marker {
            id: format!("local-{position_ms}-{}", store::fnv1a(id.as_bytes())),
            episode_id: id.to_owned(),
            position_ms,
            label: label.clone(),
        };
        markers::insert(self, local.clone());
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let label_ref = (!label.is_empty()).then_some(label.as_str());
        let answer = wire::add_marker(&api, id, position_ms, label_ref).await;
        self.note_podcasts_traffic(&api, before);
        match answer {
            Ok(marker) => {
                let stored = Marker::from(&marker);
                markers::remove(self, &local.id);
                markers::insert(self, stored.clone());
                Ok(stored)
            }
            Err(cause) => {
                markers::remove(self, &local.id);
                Err(cause)
            }
        }
    }

    /// `DELETE /api/podcast/marker/<id>` — optimistic, with the previous marker
    /// put back exactly where it was on a refusal.
    pub async fn remove_marker(&self, marker_id: &str) -> Result<(), CoreError> {
        let removed = markers::remove(self, marker_id);
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::delete_marker(&api, marker_id).await;
        self.note_podcasts_traffic(&api, before);
        match answer {
            Ok(()) => Ok(()),
            Err(cause) => {
                if let Some(marker) = removed {
                    markers::restore(self, marker);
                }
                Err(cause)
            }
        }
    }

    // -- chapters -------------------------------------------------------------

    /// The chapter list this device kept, or `None` the first time — disk only.
    pub fn cached_chapters(&self, id: &str) -> Option<Chapters> {
        chapters::cached(self, id)
    }

    /// `GET /api/podcast/episode/<id>/chapters` — one request, and the copy is
    /// kept so a reopen paints at once.
    ///
    /// An unarchived episode answers `archived: false` with no chapters: that is
    /// the server saying « les chapitres seront lus à la fin de l'archivage »,
    /// not a failure, and it is kept as such.
    pub async fn chapters(&self, id: &str) -> Result<Chapters, CoreError> {
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::chapters(&api, id).await;
        self.note_podcasts_traffic(&api, before);
        let answer = answer?;
        // Sorted and whole-second, as `normalizeChapters` does — and a chapter
        // with no declared end keeps `None`, so the last one ends with the file.
        let normalized = normalize_chapter_rows(&answer.chapters);
        let chapters = Chapters {
            episode_id: answer.episode_id,
            chapters: normalized,
            source: answer.source,
            archived: answer.archived,
            scanned: answer.scanned,
            duration_s: answer.duration_s,
        };
        chapters::write(self, &chapters);
        Ok(chapters)
    }

    // -- transcripts ----------------------------------------------------------

    /// The transcript page this device kept, or `None` — disk only, 0 requests.
    /// This is what makes a ready transcript cost nothing while listening.
    pub fn cached_transcript(
        &self,
        id: &str,
        limit: u32,
        offset: u32,
        at: Option<f64>,
    ) -> Option<TranscriptPage> {
        match at {
            Some(seconds) => transcript_cache::read_at(self, id, limit, seconds),
            None => transcript_cache::read_page(self, id, limit, offset),
        }
    }

    /// `GET /api/podcast/episode/<id>/transcript` — one page: by `offset`, by
    /// `at` (the server picks the page that covers that moment), or the lines
    /// that match `q`. A ready page with no query is kept on the device; a
    /// search's excerpts are not (they are a view, not the text).
    pub async fn transcript(
        &self,
        id: &str,
        limit: u32,
        offset: Option<u32>,
        at: Option<f64>,
        q: Option<&str>,
    ) -> Result<TranscriptPage, CoreError> {
        let q = q.map(str::trim).filter(|query| !query.is_empty());
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::transcript(&api, id, offset, Some(limit), at, q).await;
        self.note_podcasts_traffic(&api, before);
        let page = TranscriptPage::from(&answer?);
        if q.is_none() && page.status == "ready" {
            transcript_cache::write_page(self, &page, now_ms(self));
        }
        Ok(page)
    }

    /// A local search over the pages this device holds — what the panel answers
    /// with when the link is dead (`localSearch` in `TranscriptPanel.svelte`).
    pub fn local_transcript_search(&self, id: &str, q: &str, limit: u32) -> Vec<Segment> {
        transcript_cache::search_episode(self, id, q, limit as usize)
    }

    /// `GET /api/podcast/<id>/transcript/search?q=` — inside one show.
    pub async fn show_transcript_search(
        &self,
        id: &str,
        q: &str,
        limit: u32,
    ) -> Result<ShowSearch, CoreError> {
        let q = q.trim();
        if q.chars().count() < SHOW_SEARCH_MIN {
            return Ok(ShowSearch {
                query: q.to_owned(),
                ..ShowSearch::default()
            });
        }
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::search_show_transcripts(&api, id, q, limit.max(1)).await;
        self.note_podcasts_traffic(&api, before);
        let answer = answer?;
        Ok(ShowSearch {
            query: answer.q,
            hits: answer.hits.iter().map(TranscriptHit::from).collect(),
            total: answer.total,
            truncated: answer.scope.truncated,
            episodes: answer.scope.episodes,
        })
    }

    /// Whether this server has the transcription routes at all (rule 7). `None`
    /// is a doubt, and a doubt hides nothing.
    pub async fn transcription_supported(&self) -> Option<bool> {
        let api = self.api().ok()?;
        api.has_route(Feature::Transcription.probe()).await
    }

    /// `GET /api/transcribe/settings` — availability, languages and the queue.
    /// `probe` asks the server to run its engine check, which is what the
    /// « Réessayer » of the transcription card does.
    pub async fn transcribe_settings(
        &self,
        probe: bool,
    ) -> Result<TranscribeSettingsCard, CoreError> {
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::transcribe_settings(&api, probe).await;
        self.note_podcasts_traffic(&api, before);
        Ok(TranscribeSettingsCard::from(&answer?))
    }

    /// `GET /api/transcribe/status` — the engine's queue and daily budget.
    pub async fn transcribe_status(&self) -> Result<TranscribeStatusCard, CoreError> {
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::transcribe_status(&api).await;
        self.note_podcasts_traffic(&api, before);
        Ok(TranscribeStatusCard::from(&answer?))
    }

    /// `POST /api/podcast/episode/<id>/transcribe` — a server JOB (rule 2): it
    /// answers at once, the panel shows « en attente » and polls the status.
    /// `force` re-transcribes an episode that already has a text; `language`
    /// overrides the show's, `"auto"` included.
    pub async fn start_transcript(
        &self,
        id: &str,
        language: Option<&str>,
        force: bool,
    ) -> Result<(), CoreError> {
        let mut payload = serde_json::Map::new();
        if let Some(language) = language {
            payload.insert(
                "language".to_owned(),
                serde_json::Value::String(language.to_owned()),
            );
        }
        if force {
            payload.insert("force".to_owned(), serde_json::Value::Bool(true));
        }
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::start_transcript(&api, id, &serde_json::Value::Object(payload)).await;
        self.note_podcasts_traffic(&api, before);
        answer?;
        // A forced restart describes a text that no longer exists.
        if force {
            transcript_cache::forget(self, id);
        }
        Ok(())
    }

    /// `POST /api/podcast/episode/<id>/transcribe/stop` — the server keeps what
    /// it already wrote (`done_ms`), so the panel says « arrêtée » and can resume.
    pub async fn stop_transcript(&self, id: &str) -> Result<(), CoreError> {
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::stop_transcript(&api, id).await;
        self.note_podcasts_traffic(&api, before);
        answer
    }

    /// `DELETE /api/podcast/episode/<id>/transcript` — the server's text goes,
    /// and so does this device's copy of its pages.
    pub async fn delete_transcript(&self, id: &str) -> Result<(), CoreError> {
        let api = self.podcasts_api()?;
        let before = wire::totals(&api);
        let answer = wire::delete_transcript(&api, id).await;
        self.note_podcasts_traffic(&api, before);
        answer?;
        transcript_cache::forget(self, id);
        Ok(())
    }

    /// What this device holds of this episode's transcript: pages and bytes
    /// (`transcriptcache.js#cacheStats`), so the panel can say it and a test can
    /// assert the caps.
    pub fn transcript_cache_stats(&self) -> TranscriptCacheStats {
        let (pages, bytes, episodes) = transcript_cache::stats(self);
        TranscriptCacheStats {
            pages: pages as u32,
            bytes,
            episodes: episodes as u32,
        }
    }
}

/// What `/api/podcast/<id>/auto_archive` answered: the show, and what it queued.
#[derive(Debug, Clone, PartialEq)]
pub struct ShowArchiveAnswer {
    pub show: ShowCard,
    pub queued: u32,
}

/// What `/api/podcast/<id>/transcribe` answered: the show as it now stands, what
/// the server queued, and what it skipped (too long, over budget).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShowTranscribeAnswer {
    pub show: Option<ShowCard>,
    pub queued: u32,
    pub skipped: u32,
    /// `all`, `recent` or `none`.
    pub backlog: String,
}

/// `GET /api/transcribe/settings`, as the show page's card and the panel read it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TranscribeSettingsCard {
    pub available: bool,
    pub enabled: bool,
    /// Why it is not available (`engine_missing`, …), through
    /// [`views`]/[`text::reason_text`]'s sentence.
    pub reason: Option<String>,
    pub language: String,
    pub languages: Vec<String>,
    pub daily_minutes: u32,
    pub max_minutes: u32,
    pub queued: u32,
    pub running: u32,
    pub ready: u32,
    pub error: u32,
    pub engine: Option<String>,
    pub model: Option<String>,
}

impl From<&nz_model::TranscribeSettings> for TranscribeSettingsCard {
    fn from(settings: &nz_model::TranscribeSettings) -> Self {
        TranscribeSettingsCard {
            available: settings.available,
            enabled: settings.enabled,
            reason: settings.reason.clone(),
            language: if settings.language.is_empty() {
                "auto".to_owned()
            } else {
                settings.language.clone()
            },
            languages: if settings.languages.is_empty() {
                text::LANGUAGES
                    .iter()
                    .map(|code| (*code).to_owned())
                    .collect()
            } else {
                settings.languages.clone()
            },
            daily_minutes: settings.daily_minutes,
            max_minutes: settings.max_minutes,
            queued: settings.queue.queued,
            running: settings.queue.running + settings.queue.leased,
            ready: settings.queue.ready,
            error: settings.queue.error,
            engine: settings.engine.as_ref().map(|engine| engine.name.clone()),
            model: settings.model.as_ref().and_then(|model| model.name.clone()),
        }
    }
}

/// `GET /api/transcribe/status`, as the panel's status card reads it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TranscribeStatusCard {
    pub available: bool,
    pub enabled: bool,
    pub reason: Option<String>,
    pub queued: u32,
    pub running: u32,
    pub ready: u32,
    pub error: u32,
    pub budget_limit_ms: u64,
    pub budget_used_ms: u64,
}

impl From<&nz_model::TranscribeStatus> for TranscribeStatusCard {
    fn from(status: &nz_model::TranscribeStatus) -> Self {
        TranscribeStatusCard {
            available: status.available,
            enabled: status.enabled,
            reason: status.reason.clone(),
            queued: status.queue.queued,
            running: status.queue.running + status.queue.leased,
            ready: status.queue.ready,
            error: status.queue.error,
            budget_limit_ms: status.queue.budget_limit_ms,
            budget_used_ms: status.queue.budget_used_ms,
        }
    }
}

/// What the transcript cache holds (`cacheStats`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TranscriptCacheStats {
    pub pages: u32,
    pub bytes: u64,
    pub episodes: u32,
}

// --- the device's copies -----------------------------------------------------

/// The stored subscription list, in a container so the file can grow.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoredShows {
    #[serde(default)]
    shows: Vec<ShowCard>,
}

/// The stored positions, keyed by episode id, in a container for the same reason.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoredProgress {
    #[serde(default)]
    entries: BTreeMap<String, Position>,
}

fn write_shows(core: &Core, shows: &[ShowCard]) {
    let stored = StoredShows {
        shows: shows.to_vec(),
    };
    if let Ok(body) = serde_json::to_string(&stored) {
        let _ = store::write(core, store::SHOWS, "", &body);
    }
}

fn store_page(core: &Core, page: &ShowPage) {
    if let Ok(body) = serde_json::to_string(page) {
        let _ = store::write(core, store::SHOW, &page.show.id, &body);
    }
}

fn write_progress(core: &Core, progress: &StoredProgress) {
    if let Ok(body) = serde_json::to_string(progress) {
        let _ = store::write(core, store::PROGRESS, "", &body);
    }
}

/// The merge of `initPodcastProgress`: newest wins, the server on a tie, and the
/// local write still flagged while `pending_progress` holds it.
fn merge_positions(
    remote: BTreeMap<String, Position>,
    mirror: BTreeMap<String, EpisodeProgress>,
    pending: std::collections::HashSet<String>,
) -> BTreeMap<String, Position> {
    let mut merged = remote;
    for (episode, local) in mirror {
        let at_ms = local.at;
        let entry = Position {
            seconds: local.t.round().max(0.0) as u32,
            duration_s: local.d.round().max(0.0) as u32,
            finished: local.done,
            at_ms,
            pending: pending.contains(&episode),
        };
        match merged.get(&episode) {
            Some(existing) if existing.at_ms >= at_ms && !existing.pending => {}
            _ => {
                merged.insert(episode, entry);
            }
        }
    }
    for episode in &pending {
        if let Some(entry) = merged.get_mut(episode) {
            entry.pending = true;
        }
    }
    merged
}

/// The route's chapter rows as the list and the ticks read them: whole seconds,
/// sorted by start, one per start, and `end` kept only when it is really there.
fn normalize_chapter_rows(rows: &[(u32, f64, Option<f64>, String)]) -> Vec<ChapterCard> {
    text::normalize_chapters(rows)
        .into_iter()
        .map(|chapter| ChapterCard {
            index: chapter.index,
            start: chapter.start,
            end: chapter.end,
            title: chapter.title,
        })
        .collect()
}

/// The server's page (or a fresh one) as this device keeps it.
fn build_page(core: &Core, answer: &nz_model::PodcastPage) -> ShowPage {
    let show = ShowCard::from(&answer.show);
    let show_title = show.title.clone();
    let channel_id = show.id.clone();
    let markers = markers::all(core);
    let episodes = answer
        .episodes
        .iter()
        .map(|episode| episode_card(episode, &show_title, &channel_id, &markers))
        .collect();
    ShowPage {
        show,
        episodes,
        next: answer.next.clone(),
    }
}

/// One server episode as the row the show page draws.
fn episode_card(
    episode: &nz_model::Episode,
    show_title: &str,
    channel_id: &str,
    markers: &BTreeMap<String, Vec<Marker>>,
) -> EpisodeCard {
    let id = episode.id.to_string();
    let channel = {
        let own = episode.channel_id.to_string();
        if own.is_empty() {
            channel_id.to_owned()
        } else {
            own
        }
    };
    EpisodeCard {
        deezer_id: episode
            .deezer_id
            .clone()
            .filter(|deezer| !deezer.is_empty())
            .unwrap_or_else(|| id.clone()),
        title: episode.title.clone(),
        description: episode.description.clone().unwrap_or_default(),
        duration_s: episode.duration,
        published: episode.published.unwrap_or(0),
        archived: episode.archived,
        unavailable: episode.unavailable,
        explicit: episode.explicit,
        chapter_count: episode.chapter_count,
        status: episode.status.clone().unwrap_or_default(),
        cover: episode.cover.clone(),
        channel_id: channel,
        channel_title: show_title.to_owned(),
        transcript: episode.transcript.as_ref().map(TranscriptInfo::from),
        position: None,
        markers: markers.get(&id).cloned().unwrap_or_default(),
        id,
    }
}

/// Appends one page to another: the rows it brings, the cursor it names, and the
/// header the server repeated (which may have changed — an archive finished).
fn append_page(page: &mut ShowPage, next: ShowPage) {
    let known: std::collections::HashSet<String> = page
        .episodes
        .iter()
        .map(|episode| episode.id.clone())
        .collect();
    for episode in next.episodes {
        if !known.contains(&episode.id) {
            page.episodes.push(episode);
        }
    }
    page.next = next.next;
    page.show = next.show;
}

/// Now, in milliseconds, for the caches' own age (`podcastProgress.js` uses
/// `Date.now()` for the same reason). The ONE clock read in this module, and it
/// never decides a test's outcome: the tests pass the value they mean.
fn now_ms(core: &Core) -> u64 {
    let _ = core;
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// A page's rows, for a caller that has a [`ShowPage`] in hand (the FFI opens the
/// windowed handle from them).
pub fn page_rows(page: &ShowPage) -> Vec<Row> {
    page.rows()
}
