//! Search: tracks, albums, artists, playlists, podcasts, the top result,
//! transcript hits, the YouTube fallback, the query-radio rule.
//!
//! Owned by **MB6** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this directory and edits no other file of the crate.
//!
//! The behaviour is `webapp/src/routes/Search.svelte` and the components it
//! draws, PORTED — not re-invented. What lives where:
//!
//! | This module | The SPA |
//! |---|---|
//! | [`rank`] | `lib/searchrank.js` (the top result, the woven YouTube list) |
//! | [`radio`] | `lib/queryradio.js` (the 30-result bar, the seed, the context) |
//! | [`wire`] | the three reads, the rows, the kept answer, the highlights |
//! | [`link`] | `lib/linkimport.js#playlistLink` (a pasted link is an import) |
//! | [`Core::search`], [`Core::cached_search`] | `Search.svelte#run`, the visit's copy |
//! | [`Core::youtube_hits`] | `Search.svelte#runYoutube` |
//! | [`Core::search_as_radio`] | `Search.svelte#launchRadio` |
//!
//! # The request budget (rule 3)
//!
//! One settled query costs **two** requests, the SPA's own two: `GET /api/search`
//! on the [`Core::search`] call, and `GET /api/youtube/search` once the answer is
//! on screen ([`Core::youtube_hits`]). They are two calls because the SPA has two
//! BEATS (280 ms for the list, 800 ms for the fallback): a person who pauses
//! mid-word pays for one search, not for one search plus one YouTube query per
//! pause. Every request is counted under [`TAG_SEARCH`], so `NzCore::stats()`
//! answers what this screen cost.
//!
//! The podcasts scope is `GET /api/transcripts/search` (one request), and the
//! admin's shows tab is `GET /api/search/podcasts` (one) — one request per tab,
//! never the whole library re-read to switch a tab.
//!
//! # What is kept, and where
//!
//! [`Core::cached_search`] answers from `search-<account>-<scope>-<term>.json` in
//! the CACHE directory (evictable, like every cache): the provider's own answer,
//! the fallback's hits once they arrived, a transcript page, or the shows. A warm
//! visit therefore paints before any request, and a dead link keeps what it
//! painted with « Hors ligne · version mémorisée ». The five most recent terms
//! are kept and pruned; the file's name is a hash of the account and of what was
//! typed, never a server string.
//!
//! The same file is what makes [`Core::search_as_radio`] and
//! [`Core::youtube_hits`] cost nothing extra: a launch reads the answer the
//! screen is already drawing instead of asking the catalogue again.

mod link;
mod radio;
mod rank;
mod wire;

#[cfg(test)]
mod testing;

use std::sync::Arc;

use nz_api::Api;
use serde::{Deserialize, Serialize};

pub use link::{PlaylistLink, playlist_link};
pub use radio::{RADIO_MIN_RESULTS, RadioContext, Seed, offer_radio};
pub use rank::{CATALOGUE_MIN, Top, YOUTUBE_MIN, match_score, recording_key, top_result};
pub use wire::{Highlight, highlight_parts};

use crate::core::{Core, CoreError};
use crate::lists::Row;

/// The tag every request of this screen is counted under (`NzCore::stats`).
pub const TAG_SEARCH: &str = "search";
/// How many terms the device remembers (`Search.svelte#RECENT_MAX`).
pub const RECENT_MAX: usize = 5;
/// `Search.svelte#TOP_TRACKS`: ten titles is the most that still reads as a
/// shortlist on a phone; the rest is one tap away on the Titres tab.
pub const TOP_TRACKS: usize = 10;
/// The shortest term either search asks for (`Search.svelte`'s transcript gate
/// and the phone's own debounce: "2 caractères minimum").
pub const MIN_TERM: usize = 2;

/// Which branch of the screen a read is for. `Search.svelte` has two scopes
/// ("Ma bibliothèque", "Podcasts") and the podcasts scope has two tabs
/// ("Passages", "Émissions" — the second for the admin only); one request per
/// branch, never a combined answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scope {
    /// `GET /api/search` — the library and the main source, plus the shows for
    /// an admin.
    #[default]
    Library,
    /// `GET /api/transcripts/search` — the spoken passages.
    Podcasts,
    /// `GET /api/search/podcasts` — the shows only.
    Shows,
}

impl Scope {
    /// The name a kept answer carries (`Search.svelte`'s `scope`).
    pub fn slug(self) -> &'static str {
        match self {
            Scope::Library => "library",
            Scope::Podcasts => "podcasts",
            Scope::Shows => "shows",
        }
    }
}

/// What an entity card shows: an artist, an album, a playlist or a show. The
/// screen writes the French label from it ("Artiste", "Podcast"), never the core.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityKind {
    #[default]
    Artist,
    Album,
    Playlist,
    Show,
}

/// One card of the results: the shape `Card.svelte`/`PodcastCard.svelte` draws.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EntityRow {
    /// The id its route is opened by: the row's UUID when the library holds it,
    /// else the provider's id (`entity.js#entityRef`).
    pub id: String,
    pub deezer_id: String,
    pub title: String,
    /// The one line under the title; empty when the card's own label says it.
    pub subtitle: String,
    pub cover: Option<String>,
    pub kind: EntityKind,
}

impl EntityRow {
    fn memory_bytes(&self) -> u64 {
        (self.id.len() + self.deezer_id.len() + self.title.len() + self.subtitle.len()) as u64
            + self.cover.as_ref().map_or(0, |cover| cover.len() as u64)
            + std::mem::size_of::<EntityRow>() as u64
    }
}

/// The best result (`TopResult.svelte`): the one artist, or the one show, the
/// query plainly names.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TopRow {
    pub kind: EntityKind,
    pub row: EntityRow,
}

/// One spoken passage of a transcript search, as `TranscriptHits.svelte` draws
/// it: the moment, the episode, the line with the words that were typed marked.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscriptLine {
    /// Stable across two answers of the same term (the list's `key`).
    pub key: String,
    pub episode_id: String,
    pub title: String,
    /// The show, when the answer named one (`showChannel`).
    pub channel: String,
    pub start_s: u32,
    pub text: Vec<Highlight>,
    pub cover: Option<String>,
}

/// Why the fallback has nothing to add. The screen writes the sentence (the
/// SPA's three) and offers "Réessayer"; a fallback that fails never condemns the
/// list the catalogue gave (CLAUDE.md: a provider's failure is never a verdict).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum YoutubeFailure {
    /// `403`: this account may not use YouTube's search here.
    Denied,
    /// `503`: YouTube is disabled, resting or unreachable on this server.
    Unavailable,
    /// No link at all: nothing was sent.
    Offline,
    /// Anything else the server answered.
    Failed,
}

/// Everything the search screen draws, in one answer.
///
/// Not `serde`: this shape is BUILT from the answer the device kept
/// ([`wire::Stored`]), never stored itself — and a row is `nz-core::lists`' own
/// type, which has no serde impl (and needs none).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchData {
    /// The term this answer is FOR (trimmed). The screen never draws a stale
    /// answer under a new query.
    pub term: String,
    pub scope: Scope,
    pub top: Option<TopRow>,
    /// The list the screen draws, the fallback's hits woven in
    /// ([`rank::rank_search_results`]): up to [`TOP_TRACKS`] are shown on the
    /// "Tout" tab, the whole list on "Titres".
    pub tracks: Vec<Row>,
    pub artists: Vec<EntityRow>,
    /// The albums with the shows mixed in after the first two.
    pub albums: Vec<EntityRow>,
    pub playlists: Vec<EntityRow>,
    /// The shows (the admin's "Émissions" tab, and what the albums carry).
    pub podcasts: Vec<EntityRow>,
    pub transcripts: Vec<TranscriptLine>,
    pub transcript_total: u32,
    pub transcript_more: bool,
    pub transcript_episodes: u32,
    pub transcript_segments: Option<u32>,
    pub transcript_truncated: bool,
    /// How many of `tracks` came from the fallback ("YT" rows).
    pub youtube: u32,
    /// Whether the fallback answered for this term at all — an empty answer and
    /// no answer are not the same thing.
    pub youtube_answered: bool,
    /// Whether the account can ask the fallback at all (the chip is offered).
    pub youtube_available: bool,
    /// Why the fallback answered nothing, when it failed.
    pub youtube_failure: Option<YoutubeFailure>,
    /// The distinct results across the tabs (`queryradio.js#countSearchResults`).
    pub radio_total: u32,
    /// Whether that count is broad enough for « Lancer la radio » (30).
    pub radio_offered: bool,
}

impl SearchData {
    /// `Search.svelte#hasResults`: is there anything to draw at all?
    pub fn has_results(&self) -> bool {
        match self.scope {
            Scope::Library => {
                !self.artists.is_empty()
                    || !self.albums.is_empty()
                    || !self.tracks.is_empty()
                    || !self.playlists.is_empty()
                    || !self.podcasts.is_empty()
            }
            Scope::Podcasts => !self.transcripts.is_empty(),
            Scope::Shows => !self.podcasts.is_empty(),
        }
    }

    /// The titles the "Tout" tab shows ([`TOP_TRACKS`]), in the list's order.
    pub fn top_tracks(&self) -> &[Row] {
        &self.tracks[..self.tracks.len().min(TOP_TRACKS)]
    }

    /// How many more titles the "Titres" tab holds ("Tout afficher 42").
    pub fn more_tracks(&self) -> u32 {
        self.tracks.len().saturating_sub(TOP_TRACKS) as u32
    }

    /// Counted bytes of what this answer holds in memory (worker rule 4): the
    /// sum of the strings' bytes and a fixed cost per record, never a process
    /// RSS.
    pub fn memory_bytes(&self) -> u64 {
        let mut total = std::mem::size_of::<SearchData>() as u64 + self.term.len() as u64;
        for row in &self.tracks {
            total += row.key.len() as u64
                + row.id.len() as u64
                + row.title.len() as u64
                + row.artists.len() as u64
                + row.album.len() as u64
                + row.cover.as_ref().map_or(0, |cover| cover.len() as u64)
                + 96;
        }
        for entity in self
            .artists
            .iter()
            .chain(&self.albums)
            .chain(&self.playlists)
            .chain(&self.podcasts)
        {
            total += entity.memory_bytes();
        }
        if let Some(top) = &self.top {
            total += top.row.memory_bytes();
        }
        for line in &self.transcripts {
            total += line.title.len() as u64
                + line.channel.len() as u64
                + line.cover.as_ref().map_or(0, |cover| cover.len() as u64)
                + line
                    .text
                    .iter()
                    .map(|part| part.text.len() as u64)
                    .sum::<u64>()
                + 64;
        }
        total
    }
}

/// A radio started from a search (`Search.svelte#launchRadio`): the titles to
/// play NOW, and the context the player keeps for the top-ups.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RadioPlan {
    /// The opening titles, strongest first. The first one plays at once, before
    /// any request (rule 1a).
    pub tracks: Vec<Row>,
    /// `"radio"` — the player's own context kind (`player/mod.rs#need_more`).
    pub context_kind: String,
    /// `track:<ref>` or `artist:<id>`: what every top-up asks for.
    pub context_id: String,
    /// « Radio de daft punk » — the queue's honest name.
    pub context_title: String,
    /// `"track"` or `"artist"`, for the journal and the tests.
    pub seed_kind: String,
    pub seed_artist: Option<String>,
    /// The distinct results that earned the button (for the toast's honesty).
    pub total: u32,
}

impl RadioPlan {
    /// Whether the search had anything to start a station with.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty() || self.context_id.is_empty()
    }
}

impl Core {
    /// The answer this device kept for `term` — disk only, no request.
    ///
    /// `None` the first time, which is the one moment the screen has nothing to
    /// paint while the network answers.
    ///
    /// `fallback` is the screen's own chip (`Search.svelte`'s `yt`): with it off
    /// the kept fallback rows are not woven in, and the list is the catalogue's —
    /// which is what makes turning the chip off cost NOTHING (a re-read of the
    /// file, never a request).
    pub fn cached_search(&self, term: &str, scope: Scope, fallback: bool) -> Option<SearchData> {
        let term = term.trim();
        if term.len() < MIN_TERM {
            return None;
        }
        wire::read_snapshot(self, term, scope).map(|stored| self.build(&stored, fallback))
    }

    /// One settled query, on the `User` lane, counted under [`TAG_SEARCH`].
    ///
    /// `Scope::Library` is `GET /api/search?q=` (the five families, library rows
    /// first); `Scope::Podcasts` is `GET /api/transcripts/search`; `Scope::Shows`
    /// is `GET /api/search/podcasts`. ONE request each, and the answer is kept
    /// for the next visit.
    pub async fn search(
        &self,
        term: &str,
        scope: Scope,
        fallback: bool,
    ) -> Result<SearchData, CoreError> {
        let term = term.trim();
        // A pasted playlist link is an IMPORT, not a query: the SPA answers
        // nothing at all for one (`Search.svelte#run`), so no request is spent on
        // a search that cannot answer.
        if term.len() < MIN_TERM || link::playlist_link(term).is_some() {
            return Ok(self.empty(term, scope));
        }
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_SEARCH));
        let before = wire::totals(&api);
        let answer = self.read(&api, term, scope).await;
        wire::note_traffic(self, &api, before);
        let mut stored = answer?;
        stored.term = term.to_owned();
        stored.scope = scope;
        // A fresh answer replaces what the device kept for this term, but keeps
        // the fallback's hits once they arrived: a Réessayer must not make the
        // woven rows disappear for a beat.
        if let Some(previous) = wire::read_snapshot(self, term, scope) {
            stored.youtube = previous.youtube;
            stored.youtube_answered = previous.youtube_answered;
            stored.youtube_failure = previous.youtube_failure;
        }
        if let Err(cause) = wire::write_snapshot(self, &stored) {
            // A cache that cannot be written never costs the screen: the fresh
            // answer is already on its way back.
            tracing::debug!(%cause, "the search answer was not kept");
        }
        wire::prune_snapshots(self);
        Ok(self.build(&stored, fallback))
    }

    /// `GET /api/youtube/search?q=` — the fallback, asked once per settled query
    /// and woven into the SAME list ([`rank::rank_search_results`]).
    ///
    /// A failure is a STATE, never an error: the catalogue's list stays on screen
    /// and the screen says why the fallback added nothing, with « Réessayer »
    /// (rule 3). Without a kept answer for this term the catalogue is asked once
    /// first, so the woven list is never half a list.
    pub async fn youtube_hits(&self, term: &str) -> Result<SearchData, CoreError> {
        let term = term.trim();
        if term.len() < MIN_TERM {
            return Ok(self.empty(term, Scope::Library));
        }
        if wire::read_snapshot(self, term, Scope::Library).is_none() {
            // The screen's own search did not land (a launch from a deep link, a
            // process that died): one request, so the fallback's rows join a
            // complete list instead of an empty one.
            self.search(term, Scope::Library, true).await?;
        }
        let Some(mut stored) = wire::read_snapshot(self, term, Scope::Library) else {
            return Ok(self.empty(term, Scope::Library));
        };
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_SEARCH));
        let before = wire::totals(&api);
        let answer = wire::fetch_youtube(&api, term).await;
        wire::note_traffic(self, &api, before);
        match answer {
            Ok(hits) => {
                stored.youtube = hits;
                stored.youtube_answered = true;
                stored.youtube_failure = None;
            }
            Err(cause) => {
                stored.youtube = Vec::new();
                stored.youtube_answered = true;
                stored.youtube_failure = Some(youtube_failure(&cause));
                tracing::debug!(%cause, "the fallback added nothing");
            }
        }
        if let Err(cause) = wire::write_snapshot(self, &stored) {
            tracing::debug!(%cause, "the fallback's hits were not kept");
        }
        Ok(self.build(&stored, true))
    }

    /// « Lancer la radio »: the titles to play NOW, and the context the player
    /// keeps (`queryradio.js#relevantTracks`, `#radioSeed`, `#radioContext`).
    ///
    /// **No request** when the screen already has its answer: the results that
    /// are on screen ARE the opening queue, and the player's own refill asks for
    /// the next title as it is needed (one request per title, on the `User`
    /// lane). A launch whose answer is not on the device asks the catalogue once,
    /// which is the SPA's own behaviour for a stale screen.
    pub async fn search_as_radio(&self, term: &str) -> Result<RadioPlan, CoreError> {
        let term = term.trim();
        if term.len() < MIN_TERM {
            return Ok(RadioPlan::default());
        }
        if wire::read_snapshot(self, term, Scope::Library).is_none() {
            self.search(term, Scope::Library, true).await?;
        }
        let Some(stored) = wire::read_snapshot(self, term, Scope::Library) else {
            return Ok(RadioPlan::default());
        };
        let youtube: &[nz_model::Track] = if stored.youtube_answered {
            &stored.youtube
        } else {
            &[]
        };
        let total = radio::count_search_results(
            term,
            &stored.catalogue,
            youtube,
            self.me().is_some_and(|me| me.user.admin),
        );
        let tracks = radio::relevant_tracks(term, &stored.catalogue, youtube);
        let Some(seed) = radio::radio_seed(term, &stored.catalogue, &tracks) else {
            return Ok(RadioPlan {
                total,
                ..RadioPlan::default()
            });
        };
        let context = radio::radio_context(term, &seed);
        Ok(RadioPlan {
            tracks: tracks
                .iter()
                .enumerate()
                .map(|(position, track)| wire::row(track, position as u32))
                .collect(),
            context_kind: context.kind,
            context_id: context.id,
            context_title: context.title,
            seed_kind: seed.kind().to_owned(),
            seed_artist: context.artist,
            total,
        })
    }

    /// The terms this account searched, newest first (`Search.svelte#readRecent`).
    pub fn recent_searches(&self) -> Vec<String> {
        self.cache()
            .kv()
            .get::<Vec<String>>(&self.recent_key())
            .map(sanitize_terms)
            .unwrap_or_default()
    }

    /// Remembers a term the person really searched. A half-typed word, a term
    /// under two characters and a pasted playlist link are not remembered
    /// (`Search.svelte#noteSearch`).
    pub fn note_search(&self, term: &str) {
        let term = term.trim();
        if term.chars().count() < MIN_TERM || link::playlist_link(term).is_some() {
            return;
        }
        let mut terms = self.recent_searches();
        terms.retain(|kept| !kept.eq_ignore_ascii_case(term));
        terms.insert(0, term.to_owned());
        terms.truncate(RECENT_MAX);
        if let Err(cause) = self.cache().kv().set(&self.recent_key(), &terms) {
            tracing::debug!(%cause, "the recent searches were not kept");
        }
    }

    /// « Effacer » — forgets them, one write.
    pub fn clear_recent_searches(&self) {
        if let Err(cause) = self
            .cache()
            .kv()
            .set(&self.recent_key(), &Vec::<String>::new())
        {
            tracing::debug!(%cause, "the recent searches were not cleared");
        }
    }

    /// Whether this account may search the fallback at all: the source is
    /// configured as a fallback AND the person holds `youtube:search`.
    ///
    /// A server that predates the grants (`access` empty) offers it, exactly as
    /// `lib/access.js#canUse` does — the server still decides.
    pub fn youtube_available(&self) -> bool {
        let Some(me) = self.me() else {
            return false;
        };
        let Some(source) = me.user.sources.iter().find(|source| source.id == "youtube") else {
            return false;
        };
        if source.place.as_deref() != Some("fallback") {
            return false;
        }
        me.user.access.is_empty() || me.allows("youtube", "search")
    }

    // -- internals ------------------------------------------------------------

    /// The one request of one scope.
    async fn read(&self, api: &Api, term: &str, scope: Scope) -> Result<wire::Stored, CoreError> {
        let mut stored = wire::Stored {
            term: term.to_owned(),
            scope,
            ..wire::Stored::default()
        };
        match scope {
            Scope::Library => stored.catalogue = wire::fetch_search(api, term).await?,
            Scope::Podcasts => {
                stored.transcript = Some(wire::fetch_transcripts(api, term, term).await?)
            }
            Scope::Shows => stored.shows = wire::fetch_shows(api, term).await?,
        }
        Ok(stored)
    }

    /// An answer with nothing in it, for a term too short to search.
    fn empty(&self, term: &str, scope: Scope) -> SearchData {
        SearchData {
            term: term.to_owned(),
            scope,
            youtube_available: self.youtube_available(),
            ..SearchData::default()
        }
    }

    /// The rows the screen draws, built from what the device kept. `fallback` is
    /// the screen's chip: off, the kept fallback rows are not woven in.
    fn build(&self, stored: &wire::Stored, fallback: bool) -> SearchData {
        let term = stored.term.clone();
        let mut data = SearchData {
            term: term.clone(),
            scope: stored.scope,
            youtube_available: self.youtube_available(),
            youtube_answered: stored.youtube_answered,
            youtube_failure: stored.youtube_failure,
            ..SearchData::default()
        };
        match stored.scope {
            Scope::Library => {
                let admin = self.me().is_some_and(|me| me.user.admin);
                let top = rank::top_result(
                    &term,
                    &stored.catalogue.artists,
                    &stored.catalogue.podcasts,
                    admin,
                );
                let youtube: &[nz_model::Track] = if stored.youtube_answered && fallback {
                    &stored.youtube
                } else {
                    &[]
                };
                let ranked = rank::rank_search_results(&term, &stored.catalogue.tracks, youtube);
                data.tracks = ranked
                    .tracks
                    .iter()
                    .enumerate()
                    .map(|(position, track)| wire::row(track, position as u32))
                    .collect();
                data.youtube = data
                    .tracks
                    .iter()
                    .filter(|row| row.source == "youtube")
                    .count() as u32;

                let top_artist = match &top {
                    Some(rank::Top::Artist(artist)) => Some(artist),
                    _ => None,
                };
                let top_show = match &top {
                    Some(rank::Top::Show(show)) => Some(show),
                    _ => None,
                };
                data.top = top.as_ref().map(|top| match top {
                    rank::Top::Artist(artist) => TopRow {
                        kind: EntityKind::Artist,
                        row: artist_row(artist),
                    },
                    rank::Top::Show(show) => TopRow {
                        kind: EntityKind::Show,
                        row: show_row(show),
                    },
                });
                data.artists = stored
                    .catalogue
                    .artists
                    .iter()
                    .filter(|artist| {
                        !top_artist.is_some_and(|top| top.deezer_id == artist.deezer_id)
                    })
                    .map(artist_row)
                    .collect();
                // The shows are mixed among the albums only for a screen that
                // SHOWS them (`Search.svelte`'s `podcasts = $isAdmin ? … : []`).
                let shows: Vec<nz_model::PodcastCard> = if admin {
                    stored
                        .catalogue
                        .podcasts
                        .iter()
                        .filter(|show| !top_show.is_some_and(|top| top.deezer_id == show.deezer_id))
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };
                data.albums =
                    rank::mix_podcasts(stored.catalogue.albums.clone(), shows, rank::SHOWS_AFTER)
                        .into_iter()
                        .map(|mixed| match mixed {
                            rank::Mixed::Album(album) => album_row(&album),
                            rank::Mixed::Show(show) => show_row(&show),
                        })
                        .collect();
                data.playlists = stored
                    .catalogue
                    .playlists
                    .iter()
                    .map(|playlist| {
                        wire::entity_row(
                            &playlist.deezer_id,
                            &playlist.deezer_id,
                            &playlist.title,
                            &playlist.owner,
                            playlist.cover.clone(),
                            EntityKind::Playlist,
                        )
                    })
                    .collect();
                if admin {
                    data.podcasts = stored.catalogue.podcasts.iter().map(show_row).collect();
                }
                data.radio_total =
                    radio::count_search_results(&term, &stored.catalogue, youtube, admin);
                data.radio_offered = radio::offer_radio(data.radio_total);
            }
            Scope::Podcasts => {
                if let Some(answer) = &stored.transcript {
                    data.transcripts = answer.lines.clone();
                    data.transcript_total = answer.total;
                    data.transcript_more = answer.more;
                    data.transcript_episodes = answer.episodes;
                    data.transcript_segments = answer.segments;
                    data.transcript_truncated = answer.truncated;
                }
            }
            Scope::Shows => {
                data.podcasts = stored.shows.iter().map(show_row).collect();
            }
        }
        data
    }

    /// The kv key of this account's recent searches.
    fn recent_key(&self) -> String {
        format!("search.recent.{}", owner(self))
    }
}

/// The cause of a failed fallback, as the screen's state.
fn youtube_failure(cause: &CoreError) -> YoutubeFailure {
    match cause {
        CoreError::Offline => YoutubeFailure::Offline,
        // The server's own `_denied()` is a `403` WITH `X-NS-Error` (a
        // `Provider`), and a bare `403` means the same thing to a person.
        CoreError::Provider { status, .. } | CoreError::Server { status, .. } if *status == 403 => {
            YoutubeFailure::Denied
        }
        // `503`: YouTube is switched off, resting, or unreachable there.
        CoreError::Provider { status, .. } | CoreError::Server { status, .. } if *status == 503 => {
            YoutubeFailure::Unavailable
        }
        CoreError::NotReady { .. } => YoutubeFailure::Unavailable,
        _ => YoutubeFailure::Failed,
    }
}

/// An artist card (`Card.svelte#kind="artist"`): round, and the label under it is
/// the screen's own word.
fn artist_row(artist: &nz_model::Artist) -> EntityRow {
    let deezer = artist.deezer_id.to_string();
    let id = artist
        .id
        .clone()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| deezer.clone());
    wire::entity_row(
        &id,
        &deezer,
        &artist.name,
        "",
        artist.picture.clone(),
        EntityKind::Artist,
    )
}

/// An album card: the artist's name under it, else the year (`Card.svelte`).
fn album_row(album: &nz_model::Album) -> EntityRow {
    let deezer = album.deezer_id.to_string();
    let id = album
        .id
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| deezer.clone());
    let subtitle = if album.artist.name.trim().is_empty() {
        album.year.clone().unwrap_or_default()
    } else {
        album.artist.name.clone()
    };
    wire::entity_row(
        &id,
        &deezer,
        &album.title,
        &subtitle,
        album.cover.clone(),
        EntityKind::Album,
    )
}

/// A show card (`PodcastCard.svelte`): the card's own label says "Podcast".
fn show_row(show: &nz_model::PodcastCard) -> EntityRow {
    wire::entity_row(
        &show.deezer_id,
        &show.deezer_id,
        &show.title,
        "",
        show.cover.clone(),
        EntityKind::Show,
    )
}

/// The account the device's answers belong to: `/me`'s id, `""` before a session.
fn owner(core: &Core) -> String {
    core.me()
        .map(|me| me.user.id.to_string())
        .filter(|id| !id.is_empty())
        .unwrap_or_default()
}

/// The terms as the screen may draw them: strings, trimmed, deduplicated
/// case-insensitively, capped — a hand-edited or older value cannot flood the
/// page (`Search.svelte#readRecent`).
fn sanitize_terms(terms: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for term in terms {
        let term = term.trim().to_owned();
        if term.is_empty() || out.iter().any(|kept| kept.eq_ignore_ascii_case(&term)) {
            continue;
        }
        out.push(term);
        if out.len() >= RECENT_MAX {
            break;
        }
    }
    out
}

/// FNV-1a, 64 bits — the rule `nz-cache` uses for a file name: stable across runs
/// and platforms, so a kept answer stays readable after an upgrade.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_answer_is_named_after_a_hash_of_the_account_and_the_term() {
        let name = |owner: &str, term: &str, scope: Scope| {
            format!(
                "search-{:016x}-{}-{:016x}.json",
                fnv1a(owner.as_bytes()),
                scope.slug(),
                fnv1a(term.as_bytes())
            )
        };
        assert_eq!(
            name("u1", "daft punk", Scope::Library),
            name("u1", "daft punk", Scope::Library),
            "stable"
        );
        assert_ne!(
            name("u1", "daft punk", Scope::Library),
            name("u2", "daft punk", Scope::Library),
            "one account never reads another's answer"
        );
        assert_ne!(
            name("u1", "daft punk", Scope::Library),
            name("u1", "daft punk", Scope::Podcasts),
            "each scope keeps its own"
        );
        assert!(name("u1", "x", Scope::Library).ends_with(".json"));
    }

    #[test]
    fn the_recent_terms_are_sanitized_the_way_the_screen_reads_them() {
        let terms = sanitize_terms(vec![
            "  Daft Punk ".to_owned(),
            String::new(),
            "daft punk".to_owned(),
            "Air".to_owned(),
            "  ".to_owned(),
            "Kavinsky".to_owned(),
        ]);
        assert_eq!(terms, ["Daft Punk", "Air", "Kavinsky"]);
        let many: Vec<String> = (0..9).map(|n| format!("term{n}")).collect();
        assert_eq!(sanitize_terms(many).len(), RECENT_MAX);
    }

    #[test]
    fn the_button_needs_thirty_distinct_results() {
        assert!(!offer_radio(29));
        assert!(offer_radio(30));
        assert_eq!(RADIO_MIN_RESULTS, 30);
    }

    #[test]
    fn the_tout_tab_shows_ten_titles_and_says_how_many_more_there_are() {
        let rows: Vec<Row> = (0..42)
            .map(|number| Row::test_row(number, &format!("Titre {number}")))
            .collect();
        let data = SearchData {
            tracks: rows,
            ..SearchData::default()
        };
        assert_eq!(data.top_tracks().len(), TOP_TRACKS);
        assert_eq!(data.more_tracks(), 32);
        assert!(data.has_results());
        let short = SearchData {
            tracks: (0..3).map(|n| Row::test_row(n, "T")).collect(),
            ..SearchData::default()
        };
        assert_eq!(short.more_tracks(), 0);
    }

    #[test]
    fn each_scope_decides_its_own_emptiness() {
        let transcripts = SearchData {
            scope: Scope::Podcasts,
            transcripts: vec![TranscriptLine::default()],
            ..SearchData::default()
        };
        assert!(transcripts.has_results());
        let shows = SearchData {
            scope: Scope::Shows,
            podcasts: vec![EntityRow::default()],
            ..SearchData::default()
        };
        assert!(shows.has_results());
        assert!(!SearchData::default().has_results());
    }

    #[test]
    fn memory_is_counted_from_what_the_answer_really_holds() {
        let empty = SearchData::default();
        let one = SearchData {
            tracks: vec![Row::test_row(0, "Titre")],
            ..SearchData::default()
        };
        let bytes = one.memory_bytes();
        assert!(bytes > empty.memory_bytes());
        assert!(
            bytes < 4_096,
            "one row is a few hundred counted bytes, not a megabyte: {bytes}"
        );
        let many = SearchData {
            tracks: (0..50).map(|n| Row::test_row(n, "Titre")).collect(),
            ..SearchData::default()
        };
        let many_bytes = many.memory_bytes();
        assert!(many_bytes > bytes);
        assert!(
            many_bytes < 64 * 1_024,
            "50 rows stay under 64 kB: {many_bytes}"
        );
    }

    #[test]
    fn a_radio_plan_without_a_context_is_empty() {
        assert!(RadioPlan::default().is_empty());
        let plan = RadioPlan {
            tracks: vec![Row::test_row(0, "T")],
            context_id: "track:1".to_owned(),
            ..RadioPlan::default()
        };
        assert!(!plan.is_empty());
    }
}

/// The lot's own tests: the screen's request budget, the woven fallback, the
/// radio's launch and the kept answer — against a server that COUNTS, never
/// against a duration.
#[cfg(test)]
mod lot_tests {
    use super::*;
    use crate::core::NetworkState;

    /// A settled query is the SPA's **two** requests, and not one more: the
    /// catalogue's answer (one) and the fallback's, once the list is on screen
    /// (one). The SPA's own endpoint list for `routes/Search.svelte`.
    ///
    /// Numbers this test holds: 1 + 1 = 2 requests, 0 from the kept answer, and
    /// one `search` row in `NzCore::stats()` with 2.
    #[tokio::test]
    async fn a_settled_query_costs_the_spas_two_requests_and_a_warm_one_asks_nothing() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();

        let data = core
            .search("daft punk", Scope::Library, true)
            .await
            .expect("a search");
        assert_eq!(
            server.routes(),
            vec!["GET /api/search"],
            "the catalogue first, alone"
        );
        assert_eq!(data.term, "daft punk");
        assert_eq!(data.tracks.len(), 4, "the library's row, then three");
        assert_eq!(
            data.tracks[0].id, "11111111-1111-4111-8111-111111111111",
            "a library row leads (it plays from disk)"
        );
        assert_eq!(data.tracks[0].source, "upload".to_owned(), "…and says so");
        assert_eq!(
            data.top.as_ref().map(|top| top.kind),
            Some(EntityKind::Artist)
        );
        assert_eq!(
            data.top.as_ref().map(|top| top.row.title.as_str()),
            Some("Daft Punk")
        );
        assert_eq!(
            data.artists
                .iter()
                .map(|a| a.title.as_str())
                .collect::<Vec<_>>(),
            ["Daftside"],
            "the top result is not repeated in the row below it"
        );
        assert_eq!(data.albums.len(), 2);
        assert_eq!(data.albums[0].subtitle, "Daft Punk");
        assert_eq!(data.playlists.len(), 1);
        assert_eq!(data.playlists[0].subtitle, "Nadia");
        assert!(
            data.podcasts.is_empty(),
            "the shows are the admin's, and this account is not"
        );
        assert!(
            !data.youtube_answered,
            "the fallback has not been asked yet"
        );
        assert_eq!(data.youtube, 0);
        assert_eq!(
            data.radio_total, 9,
            "4 titles + 2 artists + 2 albums + 1 playlist"
        );
        assert!(
            !data.radio_offered,
            "9 results are a lookup: the button needs 30"
        );

        assert!(
            core.cached_search("daft punk", Scope::Library, true)
                .is_some(),
            "the answer is kept before the fallback is asked"
        );
        let woven = core.youtube_hits("daft punk").await.expect("the fallback");
        assert_eq!(
            server.routes(),
            vec!["GET /api/search", "GET /api/youtube/search"],
            "2 requests for the settled query"
        );
        assert!(woven.youtube_answered);
        assert_eq!(woven.youtube_failure, None);
        assert_eq!(
            woven.youtube, 1,
            "one video survived: the Clip officiel is the same recording"
        );
        assert!(
            woven
                .tracks
                .iter()
                .all(|row| !row.title.contains("Clip officiel")),
            "an identical recording is never a second line"
        );
        assert!(
            woven
                .tracks
                .iter()
                .any(|row| row.title == "Daft Punk" && row.source == "youtube"),
            "the closer video joins the SAME list"
        );

        let row = core
            .stats()
            .into_iter()
            .find(|row| row.tag == TAG_SEARCH)
            .expect("one search row");
        assert_eq!(row.requests, 2, "NzCore::stats() counts the same two");
        assert!(
            row.bytes_in > 0,
            "the bodies are counted: {} bytes",
            row.bytes_in
        );
        println!(
            "MEASURE search cold: {} requests, {} bytes in, {} bytes out",
            row.requests, row.bytes_in, row.bytes_out
        );

        // A warm read paints from the device: no request at all, fallback included.
        server.forget();
        let kept = core
            .cached_search("daft punk", Scope::Library, true)
            .expect("the answer the visit kept");
        assert_eq!(server.count(), 0, "a warm search asks nothing");
        assert_eq!(kept.tracks.len(), woven.tracks.len());
        assert_eq!(kept.youtube, 1, "the fallback's row is kept too");
        assert_eq!(
            kept.top.as_ref().map(|top| top.row.title.as_str()),
            Some("Daft Punk")
        );
        println!(
            "MEASURE search warm: 0 requests, {} counted bytes",
            kept.memory_bytes()
        );
    }

    /// A dead link answers offline in ZERO sockets, and what the person already
    /// saw is still there (`screenFlow`'s "Hors ligne · version mémorisée").
    #[tokio::test]
    async fn a_dead_link_opens_no_socket_and_the_kept_answer_still_paints() {
        let (server, _dir, core) = testing::configured().await;
        core.search("daft punk", Scope::Library, true)
            .await
            .expect("one good visit");
        assert!(
            core.cached_search("daft punk", Scope::Library, true)
                .is_some()
        );

        server.forget();
        let before = server.accepted();
        core.set_network(NetworkState::None);
        assert_eq!(
            core.search("daft punk", Scope::Library, true)
                .await
                .expect_err("offline"),
            CoreError::Offline,
            "the cause is the link, not a timeout"
        );
        assert_eq!(
            core.youtube_hits("daft punk").await.expect_err("offline"),
            CoreError::Offline
        );
        assert_eq!(server.accepted(), before, "not one socket was opened");
        assert_eq!(server.count(), 0);
        assert!(
            core.cached_search("daft punk", Scope::Library, true)
                .is_some(),
            "the copy is what the screen keeps"
        );
    }

    /// The fallback is an accessory: a refusal costs its own sentence and never
    /// the list the catalogue gave (rule 3, and CLAUDE.md's provider rule).
    #[tokio::test]
    async fn a_refused_fallback_keeps_the_list_and_says_why() {
        let (server, _dir, core) = testing::configured().await;
        core.search("daft punk", Scope::Library, true)
            .await
            .expect("a search");

        server.break_route("/api/youtube/search", 503);
        let data = core
            .youtube_hits("daft punk")
            .await
            .expect("the answer, not an error");
        assert_eq!(data.youtube_failure, Some(YoutubeFailure::Unavailable));
        assert_eq!(
            data.tracks.len(),
            4,
            "the catalogue's four rows are untouched"
        );
        assert_eq!(data.youtube, 0);

        server.forget();
        server.break_route("/api/youtube/search", 403);
        let data = core
            .youtube_hits("daft punk")
            .await
            .expect("still an answer");
        assert_eq!(data.youtube_failure, Some(YoutubeFailure::Denied));
        assert_eq!(server.count(), 1, "Réessayer asks ONCE more, no storm");
    }

    /// The podcasts scope is ONE transcript read, and the words that were typed
    /// are marked in the passage the screen draws.
    #[tokio::test]
    async fn the_podcasts_scope_asks_the_transcripts_once() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let data = core
            .search("memoire", Scope::Podcasts, true)
            .await
            .expect("a transcript search");
        assert_eq!(server.routes(), vec!["GET /api/transcripts/search"]);
        assert_eq!(
            server.last_query("/api/transcripts/search", "q").as_deref(),
            Some("memoire")
        );
        assert_eq!(data.transcripts.len(), 1);
        assert_eq!(data.transcript_total, 1);
        assert_eq!(data.transcript_episodes, 1);
        assert_eq!(data.transcript_segments, Some(240));
        assert!(!data.transcript_more);
        let line = &data.transcripts[0];
        assert_eq!(line.title, "Épisode 1");
        assert_eq!(line.channel, "Les Histoires");
        assert_eq!(line.start_s, 600);
        assert!(
            line.text
                .iter()
                .any(|part| part.mark && part.text == "memoire"),
            "the typed word is marked"
        );
        assert!(data.has_results());
        // A second visit costs nothing.
        server.forget();
        let kept = core
            .cached_search("memoire", Scope::Podcasts, true)
            .expect("the kept page");
        assert_eq!(server.count(), 0);
        assert_eq!(kept.transcripts.len(), 1);
    }

    /// The shows tab is one request of its own, and a kept answer paints it.
    #[tokio::test]
    async fn the_shows_tab_asks_the_shows_once() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let data = core
            .search("daft", Scope::Shows, true)
            .await
            .expect("the shows");
        assert_eq!(server.routes(), vec!["GET /api/search/podcasts"]);
        assert_eq!(data.podcasts.len(), 1);
        assert_eq!(data.podcasts[0].title, "Daft Punk, une histoire");
        assert_eq!(data.podcasts[0].kind, EntityKind::Show);
        assert!(data.has_results());
        assert_eq!(
            core.cached_search("daft", Scope::Shows, true)
                .map(|kept| kept.podcasts.len()),
            Some(1)
        );
    }

    /// A server older than the transcript route is `Unsupported` (rule 7): the
    /// screen hides the list and says so, instead of failing every time.
    #[tokio::test]
    async fn a_server_without_the_transcript_route_says_unsupported() {
        let (server, _dir, core) = testing::configured().await;
        server.break_route("/api/transcripts/search", 404);
        assert_eq!(
            core.search("memoire", Scope::Podcasts, true)
                .await
                .expect_err("too old"),
            CoreError::Unsupported {
                feature: "transcription".to_owned()
            }
        );
    }

    /// « Lancer la radio » plays from the answer on screen: ZERO requests, and the
    /// context the player keeps is the one its own refill reads.
    #[tokio::test]
    async fn a_launch_plays_from_the_screens_answer_and_asks_nothing() {
        let (server, _dir, core) = testing::configured().await;
        // A BROAD search: 34 titles by one artist, so the button is really
        // offered (30 distinct results) and the launch is a real one.
        let data = core
            .search("fete", Scope::Library, true)
            .await
            .expect("the screen's search");
        assert!(
            data.radio_offered,
            "34 titles + 1 artist + 1 album is past the bar"
        );
        assert_eq!(data.radio_total, 36);
        server.forget();

        let plan = core.search_as_radio("fete").await.expect("a radio");
        assert_eq!(
            server.count(),
            0,
            "the launch asks nothing: the list is here"
        );
        assert!(!plan.is_empty());
        assert_eq!(plan.context_kind, "radio");
        assert_eq!(plan.seed_kind, "track");
        assert_eq!(plan.context_id, "track:3000", "the best title seeds it");
        assert_eq!(plan.context_title, "Radio de fete");
        assert_eq!(plan.seed_artist.as_deref(), Some("27"));
        assert_eq!(
            plan.tracks.len(),
            34,
            "every title about the query opens the queue"
        );
        assert_eq!(plan.tracks[0].id, "3000");
        println!(
            "MEASURE search radio: 0 requests, {} opening titles",
            plan.tracks.len()
        );

        // And the fallback, when it has answered, joins the SAME queue.
        let (server, _dir, core) = testing::configured().await;
        core.search("daft punk", Scope::Library, true)
            .await
            .expect("a search");
        core.youtube_hits("daft punk").await.expect("the fallback");
        server.forget();
        let plan = core.search_as_radio("daft punk").await.expect("a radio");
        assert_eq!(server.count(), 0);
        assert_eq!(
            plan.context_id, "track:11111111-1111-4111-8111-111111111111",
            "the strongest title, a library row, seeds it"
        );
        assert!(plan.tracks.iter().any(|row| row.source == "youtube"));
        assert_eq!(plan.total, 10, "9 results, plus the fallback's woven video");
    }

    /// A term the device never searched asks the catalogue once — the SPA's own
    /// behaviour for a stale screen — and then launches.
    #[tokio::test]
    async fn a_launch_without_a_kept_answer_asks_the_catalogue_once() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let plan = core.search_as_radio("daft punk").await.expect("a radio");
        assert_eq!(
            server.routes(),
            vec!["GET /api/search"],
            "one request, no fallback"
        );
        assert!(!plan.is_empty());
    }

    /// « Récents »: the last five terms this account searched, newest first, one
    /// casing per term, and never a playlist link or a half-typed word.
    #[tokio::test]
    async fn the_recent_searches_are_kept_newest_first_and_capped() {
        let (_server, _dir, core) = testing::configured().await;
        assert!(core.recent_searches().is_empty());
        for term in [
            "Daft Punk",
            "Air",
            "daft punk",
            "Kavinsky",
            "Justice",
            "Moderat",
            "a",
        ] {
            core.note_search(term);
        }
        core.note_search("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M");
        assert_eq!(
            core.recent_searches(),
            ["Moderat", "Justice", "Kavinsky", "daft punk", "Air"],
            "five, newest first, one casing per term, no link, no one-letter word"
        );
        core.clear_recent_searches();
        assert!(core.recent_searches().is_empty());
    }

    /// A pasted playlist link is an import: no search request is spent on it.
    #[tokio::test]
    async fn a_pasted_playlist_link_costs_no_request() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let link = "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M";
        let data = core
            .search(link, Scope::Library, true)
            .await
            .expect("an empty answer");
        assert_eq!(server.count(), 0, "a link is not a query");
        assert!(data.tracks.is_empty());
        assert!(playlist_link(link).is_some());
    }

    /// The fallback's chip is offered from what `/me` says: a fallback source AND
    /// the `youtube:search` grant.
    #[tokio::test]
    async fn the_fallback_is_offered_from_the_session_s_own_answer() {
        let (_server, _dir, core) = testing::configured().await;
        assert!(core.youtube_available(), "a fallback source and the grant");
        let data = core
            .search("daft punk", Scope::Library, true)
            .await
            .expect("a search");
        assert!(data.youtube_available);
    }
}
