//! Search: every kind, the top result, transcript hits, the YouTube fallback, the
//! query-radio rule.
//!
//! Owned by **MB6** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this file — its own `#[uniffi::export] impl` blocks, its own
//! records, its own listener — and edits no other file of the crate.
//!
//! The broad-search-becomes-a-radio rule is `lib/queryradio.js`'s; the ranking fold is `appcore`'s.
//!
//! # The shape of this boundary
//!
//! - **`cached_search`** is `Option<SearchResults>`, synchronous and disk-only:
//!   the answer a warm visit paints before a socket opens.
//! - **`search`** is ONE request (`GET /api/search`, or the transcripts, or the
//!   shows for the admin), on the `User` lane, counted under the `search` tag.
//! - **`youtube_hits`** is the fallback's own beat: ONE request, its rows woven
//!   into the SAME list (the ranking runs in Rust), and a failure is a STATE in
//!   the record — never an error — so the catalogue's list stays on screen.
//! - **The track list crosses as a `ListHandle`**, never whole: a search answer is
//!   small today, but the rule is the lists' (rule 4) and the screen draws it
//!   windowed like every other list.
//! - **`search_as_radio`** answers the plan (the opening titles and the context
//!   the player keeps); it asks nothing when the screen already has its answer.
//!
//! Every body converts to `nz_core::search` and back; no rule lives in this file,
//! and no French sentence either (the screen writes them, from `kind`/`failure`).

use std::sync::Arc;

use nz_core::core as core_type;
use nz_core::search as core;

use crate::core::Screens;
use crate::errors::NzError;
use crate::lists::{ListHandle, TrackRow};

/// Which branch of the screen a read is for (`Search.svelte`'s two scopes and the
/// podcasts scope's two tabs).
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    /// `GET /api/search`: the library and the main source, plus the shows for an
    /// admin.
    Library,
    /// `GET /api/transcripts/search`: the spoken passages.
    Podcasts,
    /// `GET /api/search/podcasts`: the shows only.
    Shows,
}

impl From<SearchScope> for core::Scope {
    fn from(scope: SearchScope) -> Self {
        match scope {
            SearchScope::Library => core::Scope::Library,
            SearchScope::Podcasts => core::Scope::Podcasts,
            SearchScope::Shows => core::Scope::Shows,
        }
    }
}

impl From<core::Scope> for SearchScope {
    fn from(scope: core::Scope) -> Self {
        match scope {
            core::Scope::Library => SearchScope::Library,
            core::Scope::Podcasts => SearchScope::Podcasts,
            core::Scope::Shows => SearchScope::Shows,
        }
    }
}

/// What a result card is: an artist, an album, a playlist or a show. The screen
/// writes the label ("Artiste", "Podcast"), never the core.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Artist,
    Album,
    Playlist,
    Show,
}

impl From<core::EntityKind> for EntityKind {
    fn from(kind: core::EntityKind) -> Self {
        match kind {
            core::EntityKind::Artist => EntityKind::Artist,
            core::EntityKind::Album => EntityKind::Album,
            core::EntityKind::Playlist => EntityKind::Playlist,
            core::EntityKind::Show => EntityKind::Show,
        }
    }
}

/// One card of the results, as `Card.svelte`/`PodcastCard.svelte` draws it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct SearchEntity {
    /// The id its route is opened by: the row's UUID when the library holds it,
    /// else the provider's id.
    pub id: String,
    pub deezer_id: String,
    pub title: String,
    /// The one line under the title; empty when the card's label says it.
    pub subtitle: String,
    pub cover: Option<String>,
    pub kind: EntityKind,
}

impl From<core::EntityRow> for SearchEntity {
    fn from(row: core::EntityRow) -> Self {
        SearchEntity {
            id: row.id,
            deezer_id: row.deezer_id,
            title: row.title,
            subtitle: row.subtitle,
            cover: row.cover,
            kind: row.kind.into(),
        }
    }
}

/// The best result (`TopResult.svelte`): the one artist, or the one show, the
/// query plainly names. Its label is the screen's (`kind`).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct TopResult {
    pub kind: EntityKind,
    pub row: SearchEntity,
}

/// One piece of a transcript line, marked when it is what was typed
/// (`TranscriptHits.svelte`'s `<mark>`).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct TranscriptPart {
    pub text: String,
    pub mark: bool,
}

/// One spoken passage of a transcript search.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct TranscriptHit {
    /// Stable across two answers of the same term: the list's `key`.
    pub key: String,
    pub episode_id: String,
    pub title: String,
    /// The show, when the answer named one.
    pub channel: String,
    pub start_s: u32,
    pub text: Vec<TranscriptPart>,
    pub cover: Option<String>,
}

/// Why the fallback added nothing. The screen writes the sentence and offers
/// "Réessayer"; it never condemns the list the catalogue gave.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchFallback {
    /// `403`: this account may not use the fallback's search here.
    Denied,
    /// `503`: the fallback is disabled, resting or unreachable on this server.
    Unavailable,
    /// No link at all: nothing was sent.
    Offline,
    /// Anything else the server answered.
    Failed,
}

impl From<core::YoutubeFailure> for SearchFallback {
    fn from(failure: core::YoutubeFailure) -> Self {
        match failure {
            core::YoutubeFailure::Denied => SearchFallback::Denied,
            core::YoutubeFailure::Unavailable => SearchFallback::Unavailable,
            core::YoutubeFailure::Offline => SearchFallback::Offline,
            core::YoutubeFailure::Failed => SearchFallback::Failed,
        }
    }
}

/// Everything the search screen draws, in one answer.
///
/// No `Debug`: the track list is a `ListHandle`, whose own state is behind a
/// lock the boundary has no reason to print (and UniFFI does not need it).
#[derive(uniffi::Record, Clone)]
pub struct SearchResults {
    /// The term this answer is FOR: the screen never draws a stale answer under a
    /// new query.
    pub term: String,
    pub scope: SearchScope,
    pub top: Option<TopResult>,
    /// The list the screen draws, the fallback's hits woven in: the "Tout" tab
    /// shows the first [`super::search::TOP_TRACKS`] rows, "Titres" the whole
    /// list.
    pub tracks: Arc<ListHandle>,
    pub artists: Vec<SearchEntity>,
    /// The albums with the shows mixed in after the first two.
    pub albums: Vec<SearchEntity>,
    pub playlists: Vec<SearchEntity>,
    /// The shows (the admin's "Émissions" tab).
    pub podcasts: Vec<SearchEntity>,
    pub transcripts: Vec<TranscriptHit>,
    pub transcript_total: u32,
    pub transcript_more: bool,
    pub transcript_episodes: u32,
    pub transcript_segments: Option<u32>,
    pub transcript_truncated: bool,
    /// How many of the tracks came from the fallback ("YT" rows).
    pub youtube: u32,
    /// Whether the fallback answered for this term at all.
    pub youtube_answered: bool,
    /// Whether the account can ask the fallback at all (the chip is offered).
    pub youtube_available: bool,
    /// Why the fallback answered nothing, when it failed.
    pub youtube_failure: Option<SearchFallback>,
    /// The distinct results across the tabs — the radio's own count.
    pub radio_total: u32,
    /// Whether that count is broad enough for « Lancer la radio ».
    pub radio_offered: bool,
    /// Anything to draw at all (`Search.svelte#hasResults`).
    pub has_results: bool,
}

/// A radio started from a search: the titles to play NOW, and the context the
/// player keeps for the top-ups (`queryradio.js`).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct RadioPlan {
    /// The opening titles, strongest first: the first one plays at once.
    pub tracks: Vec<TrackRow>,
    /// `"radio"` — the player's own context kind.
    pub context_kind: String,
    /// `track:<ref>` or `artist:<id>`: what every top-up asks for.
    pub context_id: String,
    /// « Radio de daft punk » — the queue's honest name.
    pub context_title: String,
    pub seed_kind: String,
    pub seed_artist: Option<String>,
    /// The distinct results that earned the button.
    pub total: u32,
}

impl RadioPlan {
    /// Whether the search had anything to start a station with: the screen toasts
    /// « La radio n'a rien à jouer pour cette recherche. » when it is false.
    pub fn playable(&self) -> bool {
        !self.tracks.is_empty() && !self.context_id.is_empty()
    }
}

/// A pasted playlist link, recognised by the box: an IMPORT offer, not a query
/// (`linkimport.js#playlistLink`).
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct ImportOffer {
    pub url: String,
    /// `deezer`, `spotify`, `youtube` or `apple`.
    pub source: String,
    /// The service's name, as the offer says it.
    pub label: String,
}

/// The rows the screen draws, numbered in the answer's order so a tap names the
/// same position the list window does.
fn rows(cards: Vec<nz_core::lists::Row>) -> Vec<TrackRow> {
    cards.into_iter().map(TrackRow::from).collect()
}

fn results(core: &Arc<core_type::Core>, data: core::SearchData) -> SearchResults {
    // Read before anything is moved out of the answer (`has_results` looks at
    // the five families the fields below take apart).
    let has_results = data.has_results();
    let tracks = rows(data.tracks);
    let handle = crate::lists::open_rows(core, tracks.into_iter().map(Into::into).collect());
    SearchResults {
        term: data.term,
        scope: data.scope.into(),
        top: data.top.map(|top| TopResult {
            kind: top.kind.into(),
            row: top.row.into(),
        }),
        tracks: handle,
        artists: data.artists.into_iter().map(Into::into).collect(),
        albums: data.albums.into_iter().map(Into::into).collect(),
        playlists: data.playlists.into_iter().map(Into::into).collect(),
        podcasts: data.podcasts.into_iter().map(Into::into).collect(),
        transcripts: data
            .transcripts
            .into_iter()
            .map(|line| TranscriptHit {
                key: line.key,
                episode_id: line.episode_id,
                title: line.title,
                channel: line.channel,
                start_s: line.start_s,
                text: line
                    .text
                    .into_iter()
                    .map(|part| TranscriptPart {
                        text: part.text,
                        mark: part.mark,
                    })
                    .collect(),
                cover: line.cover,
            })
            .collect(),
        transcript_total: data.transcript_total,
        transcript_more: data.transcript_more,
        transcript_episodes: data.transcript_episodes,
        transcript_segments: data.transcript_segments,
        transcript_truncated: data.transcript_truncated,
        youtube: data.youtube,
        youtube_answered: data.youtube_answered,
        youtube_available: data.youtube_available,
        youtube_failure: data.youtube_failure.map(Into::into),
        radio_total: data.radio_total,
        radio_offered: data.radio_offered,
        has_results,
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Screens {
    /// The answer this device kept for `term` — disk only, no request. `None` the
    /// first time, which is the one moment the screen has nothing to paint.
    ///
    /// `fallback` is the screen's own chip (`Search.svelte`'s `yt`): with it off
    /// the kept fallback rows are not woven in, and the list is the catalogue's.
    pub fn cached_search(
        &self,
        term: String,
        scope: SearchScope,
        fallback: bool,
    ) -> Option<SearchResults> {
        self.core()
            .cached_search(&term, scope.into(), fallback)
            .map(|data| results(self.core(), data))
    }

    /// One settled query: ONE request on the `User` lane, counted under `search`.
    /// A failure is a cause (`nz_error_text`) the screen prints with "Réessayer".
    pub async fn search(
        &self,
        term: String,
        scope: SearchScope,
        fallback: bool,
    ) -> Result<SearchResults, NzError> {
        let data = self
            .core()
            .search(&term, scope.into(), fallback)
            .await
            .map_err(NzError::from)?;
        Ok(results(self.core(), data))
    }

    /// The fallback, once per settled query: its rows are woven into the SAME
    /// list, and a refusal is `youtube_failure`, never an error — the catalogue's
    /// list stays on screen.
    pub async fn youtube_hits(&self, term: String) -> Result<SearchResults, NzError> {
        let data = self
            .core()
            .youtube_hits(&term)
            .await
            .map_err(NzError::from)?;
        Ok(results(self.core(), data))
    }

    /// « Lancer la radio »: the opening titles and the context the player keeps.
    /// Asks nothing when the screen already has its answer.
    pub async fn search_as_radio(&self, term: String) -> Result<RadioPlan, NzError> {
        let plan = self
            .core()
            .search_as_radio(&term)
            .await
            .map_err(NzError::from)?;
        Ok(RadioPlan {
            tracks: rows(plan.tracks),
            context_kind: plan.context_kind,
            context_id: plan.context_id,
            context_title: plan.context_title,
            seed_kind: plan.seed_kind,
            seed_artist: plan.seed_artist,
            total: plan.total,
        })
    }

    /// The terms this account searched, newest first.
    pub fn recent_searches(&self) -> Vec<String> {
        self.core().recent_searches()
    }

    /// Remembers a term the person really searched (the screen calls it when a
    /// search is launched or when the screen is left).
    pub fn note_search(&self, term: String) {
        self.core().note_search(&term);
    }

    /// « Effacer » — forgets them, one write.
    pub fn clear_recent_searches(&self) {
        self.core().clear_recent_searches();
    }

    /// Whether the account can ask the fallback at all: the chip is offered only
    /// then (a server without it shows no chip and sends no request).
    pub fn youtube_available(&self) -> bool {
        self.core().youtube_available()
    }

    /// Whether this account is the server's admin: the podcasts scope's
    /// "Émissions" tab and the shows among the albums are theirs alone
    /// (`Search.svelte`'s `$isAdmin`).
    pub fn search_is_admin(&self) -> bool {
        self.core().me().is_some_and(|me| me.user.admin)
    }

    /// The playlist link in `text`, if the box holds one: the screen offers the
    /// import instead of searching (0 requests).
    pub fn search_import_offer(&self, text: String) -> Option<ImportOffer> {
        core::playlist_link(&text).map(|link| ImportOffer {
            url: link.url,
            source: link.source,
            label: link.label,
        })
    }
}

/// The shortest term a search asks for (`Search.svelte`: two characters), so the
/// screen's debounce and the core's gate cannot drift apart.
#[uniffi::export]
pub fn nz_search_min_term() -> u32 {
    core::MIN_TERM as u32
}

/// How many titles the "Tout" tab shows before "Tout afficher" takes over.
#[uniffi::export]
pub fn nz_search_top_tracks() -> u32 {
    core::TOP_TRACKS as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every scope and every kind crosses both ways: a screen never has to map a
    /// string, and a new core variant is a compile error here.
    #[test]
    fn the_scopes_and_the_kinds_cross_both_ways() {
        for scope in [
            SearchScope::Library,
            SearchScope::Podcasts,
            SearchScope::Shows,
        ] {
            let inner: core::Scope = scope.into();
            assert_eq!(SearchScope::from(inner), scope);
        }
        for kind in [
            EntityKind::Artist,
            EntityKind::Album,
            EntityKind::Playlist,
            EntityKind::Show,
        ] {
            let inner: core::EntityKind = match kind {
                EntityKind::Artist => core::EntityKind::Artist,
                EntityKind::Album => core::EntityKind::Album,
                EntityKind::Playlist => core::EntityKind::Playlist,
                EntityKind::Show => core::EntityKind::Show,
            };
            assert_eq!(EntityKind::from(inner), kind);
        }
        for failure in [
            core::YoutubeFailure::Denied,
            core::YoutubeFailure::Unavailable,
            core::YoutubeFailure::Offline,
            core::YoutubeFailure::Failed,
        ] {
            let crossed = SearchFallback::from(failure);
            let back = match crossed {
                SearchFallback::Denied => core::YoutubeFailure::Denied,
                SearchFallback::Unavailable => core::YoutubeFailure::Unavailable,
                SearchFallback::Offline => core::YoutubeFailure::Offline,
                SearchFallback::Failed => core::YoutubeFailure::Failed,
            };
            assert_eq!(back, failure);
        }
    }

    /// A plan without a context is not playable: the screen's toast, not a queue.
    #[test]
    fn a_plan_without_a_context_is_not_playable() {
        let plan = RadioPlan {
            tracks: Vec::new(),
            context_kind: String::new(),
            context_id: String::new(),
            context_title: String::new(),
            seed_kind: String::new(),
            seed_artist: None,
            total: 0,
        };
        assert!(!plan.playable());
        let plan = RadioPlan {
            tracks: vec![TrackRow::test_row(0, "Titre")],
            context_kind: "radio".into(),
            context_id: "track:1".into(),
            ..plan
        };
        assert!(plan.playable());
    }

    /// The two numbers the screen's copy depends on are the core's own.
    #[test]
    fn the_screens_constants_are_the_cores() {
        assert_eq!(nz_search_min_term(), 2);
        assert_eq!(nz_search_top_tracks(), 10);
        assert_eq!(nz_search_min_term() as usize, core::MIN_TERM);
        assert_eq!(nz_search_top_tracks() as usize, core::TOP_TRACKS);
    }

    /// A pasted link crosses as the offer the screen draws.
    #[test]
    fn a_pasted_link_crosses_as_an_offer() {
        let link = core::playlist_link("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M")
            .expect("a link");
        let offer = ImportOffer {
            url: link.url,
            source: link.source,
            label: link.label,
        };
        assert_eq!(offer.source, "spotify");
        assert_eq!(offer.label, "Spotify");
        assert!(offer.url.starts_with("https://open.spotify.com/playlist/"));
    }
}
