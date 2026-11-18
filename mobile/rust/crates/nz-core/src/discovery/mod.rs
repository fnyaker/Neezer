//! Discovery: Nouveautés, the release shelves, the Récap, moods, Explorer and
//! the parent styles, the discovery slider's home, the genre stations and
//! `/recommend`. Owned by **MB14** (docs/agents/mobile.md, "Module and file
//! ownership").
//!
//! The behaviour is the SPA's, PORTED, never re-invented. What that means,
//! screen by screen:
//!
//! | This module | The SPA |
//! |---|---|
//! | [`ReleasesData`], [`Core::releases`] | `routes/Releases.svelte#loadReleasesFeed` |
//! | [`Core::cached_releases`] | the module-scoped feed a back navigation paints |
//! | [`build_shelves`], [`Core::release_shelf`] | `buildReleaseShelves`, `routes/ReleaseShelf.svelte` |
//! | [`group_releases`] | `Releases.svelte#groupReleases` |
//! | [`recap`], [`Core::recap`] | `routes/Recap.svelte`, `lib/recap.js` |
//! | [`recap::year_chips`], [`Core::recap_years`] | `lib/recapyears.js` |
//! | [`moods`], [`Core::mood_tracks`] | `Home.svelte#playMood`, `/api/moods` |
//! | [`explorer`], [`Core::explorer_styles`] | `ExplorerStyles.svelte#parentStyles` |
//! | [`Core::explorer_discover`] | `ExplorerStyles.svelte#discover`, `#schedule` |
//! | [`Core::genre_station`] | `lib/genrestation.js`, `/api/radio/genre/<style>` |
//! | [`Core::mute_artist`] | `lib/actions.js#muteArtist` |
//! | [`Core::recommendations`] | `/api/recommend`, the discovery level |
//!
//! # The request budget (rule 3)
//!
//! A cold Nouveautés is `GET /releases` and — only when the answer carries an
//! unseen badge — `POST /releases/seen`: the SPA's own two calls, never more.
//! A shelf page is **zero** requests (`ReleaseShelf.svelte` reads the feed the
//! first screen already holds). A Récap is one `/recap/years` per ten minutes
//! then one `/recap/<year>`, and the nine cards cost nothing. Explorer's styles
//! are one `/explorer/styles`; a discovery is one POST and one GET every two
//! seconds (the screen's timer, not the core's), and the all-stations sheet is
//! one `/flow/styles` shared with MB5's chips.
//!
//! # What is kept, and where
//!
//! The release feed, the Explorer styles and the all-stations list are JSON
//! snapshots under the CACHE directory (evictable, one file per account, named
//! by a hash), so a warm open paints before any request and a dead link keeps
//! what the person already saw. The Récap is **not** written to disk: the SPA
//! refuses to persist personal listening history (`lib/recapyears.js`,
//! `lib/apicache.js`), so the years live ten minutes and a year until the
//! process dies — the offline answer is the screen's own sentence and
//! « Réessayer », exactly as `Recap.svelte` says it.

#[cfg(test)]
mod testing;

pub mod explorer;
pub mod recap;
mod wire;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use nz_api::Api;
use nz_model::{Release, Track};
use serde::{Deserialize, Serialize};

use crate::core::{Core, CoreError};
use crate::home::TrackCard;

pub use explorer::{
    LEVELS, MAX_POLLS, POLL_MS, STATION_KIND, StationContext, StationRow, StyleTile, TILES_MAX,
    clamp_level, count_label, filter_styles, level, max_tracks, parent_styles, station_context,
    station_rows, station_tracks, verified_label,
};
pub use recap::{
    EmptyReason, MonthBar, YearChip, format_minutes, format_number, format_span, poster_lines,
    share_text, slide_kicker, slide_title,
};
pub use wire::{DiscoverPlaylist, ExplorerStyleRow, ExplorerStyles, StationAnswer};

/// The tag every request of this lot is counted under (`NzCore::stats`).
pub const TAG_DISCOVERY: &str = "discovery";
/// `Releases.svelte#FEED_FRESH_MS`: five minutes.
pub const FEED_FRESH_SECS: u64 = 5 * 60;
/// `recapyears.js#TTL`: ten minutes.
pub const RECAP_YEARS_TTL_SECS: u64 = 10 * 60;
/// `/api/recommend`'s own ceiling: the screen draws six of them.
pub const RECO_ROWS: usize = 20;
/// The per-release list's ceiling, the server's own (`RELEASE_TRACKS_MAX`).
pub const RELEASE_TRACKS_MAX: usize = 400;
/// `ReleaseCard.svelte#limit`: six rows say what a shelf sounds like.
pub const SHELF_PREVIEW: usize = 6;

/// `Releases.svelte#SHELF_COPY`: the two shelves' own words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShelfCopy {
    pub title: &'static str,
    pub short: &'static str,
    pub description: &'static str,
}

pub const SHELF_ARTISTS: ShelfCopy = ShelfCopy {
    title: "Tes artistes",
    short: "Les sorties de tes favoris et des artistes que tu écoutes.",
    description: "Les sorties récentes des artistes que tu suis et de ceux que tu écoutes le plus.",
};

pub const SHELF_DISCOVERY: ShelfCopy = ShelfCopy {
    title: "Découverte",
    short: "Des artistes proches de tes favoris, et d'autres à découvrir.",
    description: "Des artistes proches de tes favoris, et des sorties que tu n'as pas encore écoutées.",
};

/// The copy of one shelf, or `None` for a kind that is not one
/// (`ReleaseShelf.svelte#key`'s fallback to the first shelf is the caller's).
pub fn shelf_copy(kind: &str) -> Option<ShelfCopy> {
    match kind {
        "artists" => Some(SHELF_ARTISTS),
        "discovery" => Some(SHELF_DISCOVERY),
        _ => None,
    }
}

/// One release card, shaped the way Nouveautés and an "Autres sorties" cell
/// draw it (`_release_card` in `webapp/src/webui/discover.py`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReleaseRow {
    pub key: String,
    pub title: String,
    pub cover: Option<String>,
    pub artist: String,
    pub artist_id: Option<String>,
    /// The server's own kind (`album`, `single`, `ep`, …).
    pub kind: String,
    /// Its French label, `Sortie` when the kind is not one this app knows.
    pub kind_label: String,
    pub released: Option<String>,
    /// "12 nov." — the card's date, or `""`.
    pub date_label: String,
    pub nb_tracks: u32,
    /// The favourite this release is close to, when it is a neighbour.
    pub close_to: Option<String>,
    /// "Ascenseur · Single · 12 nov." — one line, already joined.
    pub subtitle: String,
    /// Arrived with the latest check: the "Nouveau" badge.
    pub new: bool,
    /// How many of its titles the feed carries.
    pub track_count: u32,
}

impl ReleaseRow {
    fn memory_bytes(&self) -> u64 {
        let mut total = std::mem::size_of::<ReleaseRow>() as u64;
        for text in [
            &self.key,
            &self.title,
            &self.artist,
            &self.kind,
            &self.subtitle,
        ] {
            total += text.len() as u64;
        }
        total += self.cover.as_deref().map(str::len).unwrap_or(0) as u64;
        total += self.released.as_deref().map(str::len).unwrap_or(0) as u64;
        total += self.close_to.as_deref().map(str::len).unwrap_or(0) as u64;
        total
    }
}

/// One shelf of Nouveautés (`buildReleaseShelves`'s answer).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReleaseShelfRow {
    /// `artists` or `discovery`.
    pub key: String,
    pub title: String,
    pub subtitle: String,
    pub releases: Vec<ReleaseRow>,
    pub track_count: u32,
    pub duration_s: u64,
    /// Up to four sleeves for the banner's mosaic.
    pub mosaic: Vec<String>,
    /// The favourites the "Découverte" shelf is close to (its own sentence).
    pub close_names: Vec<String>,
    /// The six rows the card previews (`ReleaseCard.svelte#limit`): a shelf is
    /// an aperçu, not a tracklist.
    pub preview: Vec<TrackCard>,
}

/// One release's own titles, for "Toutes les sorties" (`groupReleases`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReleaseGroup {
    pub release: ReleaseRow,
    /// The performer line of its first title (`t.display_artist || artist.name`).
    pub artist: String,
    /// Its titles, in the release's own order.
    pub tracks: Vec<TrackCard>,
    /// "Album · 3 titres · Nouveau" — the card's label.
    pub label: String,
}

/// Everything Nouveautés draws, in one answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReleasesData {
    /// Every new title the feed carries.
    pub count: u32,
    /// Releases the person has not opened yet ("2 nouvelles sorties").
    pub unseen: u32,
    /// Releases the server is still reading (its own sentence under the list).
    pub pending: u32,
    /// The two shelves that hold at least one title, in the SPA's order.
    pub shelves: Vec<ReleaseShelfRow>,
    /// "Toutes les sorties": the releases that DO have titles, newest first.
    pub groups: Vec<ReleaseGroup>,
    /// The releases with no title yet (`albumsOnly`), as plain cards.
    pub others: Vec<ReleaseRow>,
    /// `groups.len() + others.len()` — the section's count.
    pub release_total: u32,
    /// The whole flat feed: what "Tout lire" queues, shelves and groups are
    /// built from it, and it is what the device keeps.
    pub tracks: Vec<TrackCard>,
    /// Every release card the feed carried, by id — the authority on `close_to`.
    pub releases: Vec<ReleaseRow>,
}

impl ReleasesData {
    /// "Rien de neuf pour l'instant": nothing to draw at all.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty() && self.releases.is_empty()
    }

    /// `Releases.svelte#freshLabel`: "3 nouvelles sorties".
    pub fn fresh_label(&self) -> String {
        if self.unseen == 0 {
            return String::new();
        }
        let plural = self.unseen > 1;
        format!(
            "{} nouvelle{} sortie{}",
            self.unseen,
            if plural { "s" } else { "" },
            if plural { "s" } else { "" }
        )
    }

    /// The sentence under "Toutes les sorties" when the server is still reading
    /// some releases (`Releases.svelte`'s `pending` line).
    pub fn pending_label(&self) -> String {
        if self.pending == 0 {
            return String::new();
        }
        let plural = self.pending > 1;
        format!(
            "{} sortie{} n'{} pas encore {} titre{} : le serveur {} lit en arrière-plan.",
            self.pending,
            if plural { "s" } else { "" },
            if plural { "ont" } else { "a" },
            if plural { "leurs" } else { "son" },
            if plural { "s" } else { "" },
            if plural { "les" } else { "le" },
        )
    }

    /// The sleeve of each title's release, for a group's own cover (`None` when
    /// the release has none).
    pub fn group(&self, release_id: &str) -> Option<&ReleaseGroup> {
        self.groups
            .iter()
            .find(|group| group.release.key == release_id)
    }

    /// Counted bytes of what this answer holds (worker rule 4).
    pub fn memory_bytes(&self) -> u64 {
        let mut total = std::mem::size_of::<ReleasesData>() as u64;
        for release in &self.releases {
            total += release.memory_bytes();
        }
        for shelf in &self.shelves {
            total += std::mem::size_of::<ReleaseShelfRow>() as u64;
            total += (shelf.title.len() + shelf.subtitle.len() + shelf.key.len()) as u64;
            for name in &shelf.close_names {
                total += name.len() as u64;
            }
            for cover in &shelf.mosaic {
                total += cover.len() as u64;
            }
        }
        for group in &self.groups {
            total += std::mem::size_of::<ReleaseGroup>() as u64 + group.artist.len() as u64;
            for track in &group.tracks {
                total += track.memory_bytes();
            }
        }
        for track in &self.tracks {
            total += track.memory_bytes();
        }
        total
    }
}

/// One shelf's page, ready to draw: its copy, its mosaic, and the titles a
/// `ListHandle` windows (`routes/ReleaseShelf.svelte`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReleaseShelfDetail {
    pub key: String,
    pub title: String,
    pub subtitle: String,
    /// `shelfDescription`: the "Découverte" shelf names the favourites.
    pub description: String,
    /// "12 titres · 3 sorties" — the banner's facts line.
    pub facts: String,
    pub total_duration_s: u64,
    pub releases: Vec<ReleaseRow>,
    pub mosaic: Vec<String>,
    pub tracks: Vec<TrackCard>,
}

/// What `/api/releases/settings` says (`neezer/releases.py#status`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseSettingsData {
    pub on: bool,
    /// Hours between two checks.
    pub every: u32,
    /// Days a release stays "new".
    pub window: u32,
    pub close: u32,
    pub listened: u32,
    pub watched: u32,
    pub favourites: u32,
    pub links: u32,
    pub asked: u32,
    pub releases: u32,
    pub running: bool,
    pub last_run: String,
    pub last_error: String,
}

/// The years a Récap can open (`lib/recapyears.js`'s state).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecapYearsData {
    pub years: Vec<nz_model::RecapYearRow>,
    /// One chip per year that holds an event, newest first.
    pub chips: Vec<YearChip>,
    pub latest: Option<i32>,
    pub now_year: i32,
    pub min_year: i32,
    /// The season's home card: this year or the one before, when it has a story.
    pub advertised: Option<nz_model::RecapYearRow>,
}

/// One row of a slide's top (`RecapSlide.svelte`'s `ranks`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecapSlideRow {
    pub key: String,
    /// 1-based rank.
    pub rank: u32,
    pub title: String,
    /// The second line: listens, the artist, the style's share.
    pub subtitle: String,
    /// The right-hand mono line ("41 écoutes", "41 %"), already worded.
    pub value: String,
    pub cover: Option<String>,
    /// The style's share, 0…1, for the genres card's bar.
    pub share: f64,
}

/// One card of the story: the ID selects the layout, every word is
/// `lib/recap.js`'s.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecapSlide {
    pub id: String,
    pub kicker: String,
    pub title: String,
    /// The glyph's name (`RecapSlide.svelte#ICONS`, through `Icon`).
    pub glyph: String,
    /// The sentence under the title ("2 180 écoutes · 8 540 minutes").
    pub lede: String,
    /// A second line ("17 styles explorés", the streak's dates).
    pub text: String,
    /// The small line at the bottom ("du 1ᵉʳ janvier au 13 novembre 2026").
    pub footnote: String,
    pub rows: Vec<RecapSlideRow>,
    /// The twelve month bars, on the minutes card only.
    pub bars: Vec<MonthBar>,
    /// Up to four sleeves, on the opening card.
    pub mosaic: Vec<String>,
    /// The six poster lines, on the share card.
    pub poster: Vec<String>,
}

/// One year in review, assembled (`routes/Recap.svelte`'s state).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecapData {
    pub year: i32,
    pub partial: bool,
    pub enough: bool,
    /// "du 1ᵉʳ janvier au 13 novembre 2026".
    pub period_label: String,
    pub minutes: u32,
    pub listens: u32,
    /// The same two figures with the French separator ("8 540"), decided here so
    /// the glance panel and the poster cannot word one year two ways.
    pub minutes_label: String,
    pub listens_label: String,
    pub slides: Vec<RecapSlide>,
    /// The share card's poster and the sentence a text-only share carries.
    pub poster_lines: Vec<String>,
    pub share_text: String,
    pub mosaic: Vec<String>,
    /// "En un coup d'œil": the artist and the style, `""` when there is none.
    pub top_artist: String,
    pub top_genre: String,
    /// "61 écoutes" on the biggest day, `""` when the year has none.
    pub biggest_day: String,
    /// Why the year has no story; `None` means the deck has cards.
    pub empty_reason: Option<EmptyReason>,
    pub empty_title: String,
    pub empty_text: String,
    /// The nearest other year the empty panel offers to open.
    pub nearby_year: Option<i32>,
    /// The top titles, what "Lire ces 5 titres" queues.
    pub top_tracks: Vec<TrackCard>,
}

/// One "Mix d'ambiance" card (`/api/moods`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoodRow {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    /// A COUNT in the catalog, the list in [`MoodTracks`].
    pub tracks: u32,
    pub ready: bool,
    pub cover: Option<String>,
}

/// One mix, when the card is tapped (`/api/moods/<id>`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MoodTracks {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub ready: bool,
    /// `not-enough` or `no-tracks`, when the mix is empty.
    pub reason: String,
    pub tracks: Vec<TrackCard>,
}

/// Explorer's shelf: the ten parent styles, and whose taste they are.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExplorerStylesData {
    pub tiles: Vec<StyleTile>,
    /// The biggest tile's count, for the bar under each name.
    pub max_tracks: u32,
    /// `history`, `unanalysed` or `none`: an empty shelf says which.
    pub source: String,
    pub tracks: u32,
    /// The RAW rows, so a later build never loses a leaf to the merge.
    pub rows: Vec<ExplorerStyleRow>,
}

/// One playlist of a finished discovery, shaped for the panel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExplorerPlaylist {
    pub id: String,
    pub title: String,
    pub cover: Option<String>,
    /// "12 titres vérifiés sur 60" — the panel's own words.
    pub count_label: String,
    pub tracks: Vec<TrackCard>,
}

/// Where one style's discovery stands (`ExplorerStyles.svelte`'s five phases).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoverPhase {
    /// Nothing was ever asked: the panel's hint, and the « Découvrir » button.
    Idle,
    /// A sweep is running server-side: the poll continues.
    Pending,
    /// The playlists arrived.
    Ready,
    /// A finished sweep with nothing in it: one notch wider may find something.
    Empty,
    /// The server asked for a wait (`429`/`503`, its own `Retry-After`).
    Busy,
}

impl DiscoverPhase {
    pub fn code(self) -> &'static str {
        match self {
            DiscoverPhase::Idle => "idle",
            DiscoverPhase::Pending => "pending",
            DiscoverPhase::Ready => "ready",
            DiscoverPhase::Empty => "empty",
            DiscoverPhase::Busy => "busy",
        }
    }
}

/// One answer of `/api/explorer/discover`, as the panel draws it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiscoverData {
    pub phase: Option<DiscoverPhase>,
    pub style: String,
    pub level: u32,
    pub playlists: Vec<ExplorerPlaylist>,
    /// The status that made the server busy (`429` or `503`), when it did.
    pub busy_status: u16,
    /// The server's own wait, in seconds, when it said one.
    pub retry_after_s: u32,
}

impl DiscoverData {
    pub fn phase_code(&self) -> &'static str {
        self.phase.map(DiscoverPhase::code).unwrap_or("idle")
    }

    /// The panel's cause sentence for a busy server
    /// (`ExplorerStyles.svelte#fail`).
    pub fn busy_cause(&self) -> String {
        match self.busy_status {
            429 => "Trop de découvertes demandées pour l'instant.".to_owned(),
            503 => "Le serveur est occupé.".to_owned(),
            _ => String::new(),
        }
    }

    /// "Réessaie dans 5 s." / "Tu peux réessayer." / "Réessaie dans un moment."
    pub fn busy_wait(&self, left: u32) -> String {
        if left > 0 {
            format!("Réessaie dans {left} s.")
        } else if self.retry_after_s > 0 {
            "Tu peux réessayer.".to_owned()
        } else {
            "Réessaie dans un moment.".to_owned()
        }
    }
}

/// A started station: its playback context and the titles to play NOW — the
/// tracks ARE the answer, so nothing is asked again before the first note
/// (`ExplorerStyles.svelte#startStation`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StationData {
    /// Always [`STATION_KIND`].
    pub kind: String,
    /// The server's canonical node (`stationContext`'s `id`).
    pub id: String,
    /// "Station Techno".
    pub title: String,
    pub name: String,
    pub tracks: Vec<TrackCard>,
}

/// Everything the device keeps in memory for this lot, keyed by the core's data
/// directory and the account, so a test's throwaway core never reads another's.
#[derive(Default)]
struct Session {
    /// The years, with the moment they arrived (`recapyears.js`'s ten minutes).
    years: Option<(Instant, RecapYearsData)>,
    /// The years a person opened, until the process dies: the SPA refuses to
    /// write personal listening history to a device.
    recaps: HashMap<i32, RecapData>,
}

fn sessions() -> &'static Mutex<HashMap<String, Session>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, Session>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn session_key(core: &Core) -> String {
    format!("{}|{}", core.config().data_dir.display(), owner(core))
}

/// Test hook: makes the kept `/recap/years` look older than its ten minutes, so
/// the TTL is proven without a `sleep` (worker rule 6: events, never a clock).
#[cfg(test)]
pub(crate) fn age_recap_years(core: &Core) {
    let key = session_key(core);
    let mut sessions = sessions().lock().unwrap_or_else(|err| err.into_inner());
    if let Some(session) = sessions.get_mut(&key)
        && let Some((at, _)) = &mut session.years
    {
        *at = Instant::now() - Duration::from_secs(RECAP_YEARS_TTL_SECS + 1);
    }
}

impl Core {
    // -- Nouveautés ---------------------------------------------------------

    /// The feed as this device last saw it — disk only, no request. `None` the
    /// first time, which is when the screen shows its skeleton.
    pub fn cached_releases(&self) -> Option<ReleasesData> {
        wire::read_snapshot(self, "releases")
    }

    /// The feed, refreshed: `GET /api/releases`, then `POST /api/releases/seen`
    /// when the answer carried an unseen badge — the SPA's own two calls
    /// (`Releases.svelte#loadReleasesFeed`).
    ///
    /// The snapshot is written with the badge CLEARED once it has been marked
    /// seen, so a later back navigation does not announce the same releases
    /// twice, while the answer returned here still carries this visit's count.
    pub async fn releases(&self) -> Result<ReleasesData, CoreError> {
        let api = wire::tagged(self)?;
        let before = wire::totals(&api);
        let answer = async {
            let feed = wire::fetch_releases(&api).await?;
            let mut data = ReleasesData::from_feed(&feed);
            let seen = data.unseen > 0;
            if seen {
                // The SPA's `.catch(() => {})`: a badge that cannot be cleared
                // never costs the screen, and the next visit asks again.
                if let Err(cause) = wire::mark_seen(&api).await {
                    tracing::debug!(%cause, "the releases badge was not cleared");
                }
            }
            let mut kept = data.clone();
            kept.unseen = 0;
            if let Err(cause) = wire::write_snapshot(self, "releases", &kept) {
                tracing::debug!(%cause, "the releases feed was not kept");
            }
            data.unseen = if seen { data.unseen } else { 0 };
            Ok::<ReleasesData, CoreError>(data)
        }
        .await;
        self.note_traffic(&api, before);
        answer
    }

    /// `POST /api/releases/seen` — the badge goes away now, on the person's own
    /// action (the shelf page being opened).
    pub async fn releases_seen(&self) -> Result<(), CoreError> {
        let api = wire::tagged(self)?;
        let before = wire::totals(&api);
        let answer = async {
            wire::mark_seen(&api).await?;
            if let Some(mut kept) = self.cached_releases() {
                kept.unseen = 0;
                if let Err(cause) = wire::write_snapshot(self, "releases", &kept) {
                    tracing::debug!(%cause, "the kept badge was not cleared");
                }
            }
            Ok::<(), CoreError>(())
        }
        .await;
        self.note_traffic(&api, before);
        answer
    }

    /// The last known `/api/releases/settings`, from the same snapshot.
    pub fn cached_release_settings(&self) -> Option<ReleaseSettingsData> {
        wire::read_snapshot(self, "release-settings")
    }

    /// `GET /api/releases/settings` — admin only; a member's answer is a `403`
    /// the card is hidden behind (MB16's rule, `user.admin`).
    pub async fn release_settings(&self) -> Result<ReleaseSettingsData, CoreError> {
        let api = wire::tagged(self)?;
        let before = wire::totals(&api);
        let answer = async {
            let status = wire::fetch_release_settings(&api).await?;
            let data = ReleaseSettingsData::from_status(&status);
            if let Err(cause) = wire::write_snapshot(self, "release-settings", &data) {
                tracing::debug!(%cause, "the releases settings were not kept");
            }
            Ok::<ReleaseSettingsData, CoreError>(data)
        }
        .await;
        self.note_traffic(&api, before);
        answer
    }

    /// `PUT /api/releases/settings` — the server clamps and answers the whole
    /// status, which is what the screen then draws (rule 6).
    ///
    /// The three values are typed here rather than handed a JSON document:
    /// `nz-ffi` carries no `serde_json` (MB16's rule), so the body is built in
    /// this crate, where the wire already lives.
    pub async fn save_release_settings(
        &self,
        on: bool,
        every: u32,
        window: u32,
    ) -> Result<ReleaseSettingsData, CoreError> {
        let api = wire::tagged(self)?;
        let before = wire::totals(&api);
        let change = serde_json::json!({ "on": on, "every": every, "window": window });
        let answer = async {
            let status = wire::save_release_settings(&api, &change).await?;
            let data = ReleaseSettingsData::from_status(&status);
            if let Err(cause) = wire::write_snapshot(self, "release-settings", &data) {
                tracing::debug!(%cause, "the releases settings were not kept");
            }
            Ok::<ReleaseSettingsData, CoreError>(data)
        }
        .await;
        self.note_traffic(&api, before);
        answer
    }

    /// `POST /api/releases/check` — one request starts the server's own pass
    /// (a worker, rule 2); the answer is its status, never a wait.
    pub async fn check_releases(&self) -> Result<ReleaseSettingsData, CoreError> {
        let api = wire::tagged(self)?;
        let before = wire::totals(&api);
        let answer = async {
            let status = wire::check_releases(&api).await?;
            let data = ReleaseSettingsData::from_status(&status);
            if let Err(cause) = wire::write_snapshot(self, "release-settings", &data) {
                tracing::debug!(%cause, "the releases settings were not kept");
            }
            Ok::<ReleaseSettingsData, CoreError>(data)
        }
        .await;
        self.note_traffic(&api, before);
        answer
    }

    /// One shelf's page, built from the feed the first screen already holds: no
    /// request at all, which is what makes a card click instant
    /// (`ReleaseShelf.svelte` reads the module-scoped feed).
    pub fn release_shelf(&self, kind: &str) -> Option<ReleaseShelfDetail> {
        let feed = self.cached_releases()?;
        let copy = shelf_copy(kind)?;
        Some(build_shelf_detail(&feed, kind, copy))
    }

    /// One release's titles, from the same feed — the group a person expands,
    /// windowed by the caller (`groupReleases`).
    pub fn release_group_tracks(&self, release_id: &str) -> Vec<TrackCard> {
        self.cached_releases()
            .map(|feed| {
                feed.tracks
                    .into_iter()
                    .filter(|track| track.release.as_deref() == Some(release_id))
                    .collect::<Vec<TrackCard>>()
            })
            .unwrap_or_default()
    }

    // -- Récap ---------------------------------------------------------------

    /// The years this device already knows, without a request (the screen's
    /// first paint). Stale is fine: the fresh read replaces it.
    pub fn cached_recap_years(&self) -> Option<RecapYearsData> {
        let key = session_key(self);
        let sessions = sessions().lock().unwrap_or_else(|err| err.into_inner());
        sessions
            .get(&key)
            .and_then(|session| session.years.as_ref().map(|(_, data)| data.clone()))
    }

    /// `GET /api/recap/years` — asked ONCE per ten minutes (`recapyears.js`'s
    /// TTL), so opening the home then the Récap costs one `/years` whatever the
    /// order.
    pub async fn recap_years(&self) -> Result<RecapYearsData, CoreError> {
        let key = session_key(self);
        {
            let sessions = sessions().lock().unwrap_or_else(|err| err.into_inner());
            if let Some((at, data)) = sessions
                .get(&key)
                .and_then(|session| session.years.as_ref())
                && at.elapsed() < Duration::from_secs(RECAP_YEARS_TTL_SECS)
            {
                return Ok(data.clone());
            }
        }
        let api = wire::tagged(self)?;
        let before = wire::totals(&api);
        let answer = async {
            let years = wire::fetch_recap_years(&api).await?;
            let data = RecapYearsData::from_answer(&years);
            let mut sessions = sessions().lock().unwrap_or_else(|err| err.into_inner());
            sessions.entry(key).or_default().years = Some((Instant::now(), data.clone()));
            Ok::<RecapYearsData, CoreError>(data)
        }
        .await;
        self.note_traffic(&api, before);
        answer
    }

    /// The year as this device already assembled it (memory only: the SPA never
    /// writes a person's listening history to disk).
    pub fn cached_recap(&self, year: i32) -> Option<RecapData> {
        let key = session_key(self);
        let sessions = sessions().lock().unwrap_or_else(|err| err.into_inner());
        sessions
            .get(&key)
            .and_then(|session| session.recaps.get(&year).cloned())
    }

    /// `GET /api/recap/<year>` — one bounded payload; the nine cards then cost
    /// nothing. A malformed year never reaches the server (`YEAR_RE`).
    pub async fn recap(&self, year: i32) -> Result<RecapData, CoreError> {
        if !(1000..=9999).contains(&year) {
            return Err(CoreError::Invalid {
                detail: "Cette année n'existe pas dans ton récap".to_owned(),
            });
        }
        // The chips are a side question: their failure never condemns the year
        // (`Recap.svelte#open`'s `.catch(() => {})`).
        let years = self.recap_years().await.ok();
        let api = wire::tagged(self)?;
        let before = wire::totals(&api);
        let answer = async {
            let payload = wire::fetch_recap(&api, year).await?;
            let data = RecapData::assemble(&payload, years.as_ref());
            let key = session_key(self);
            let mut sessions = sessions().lock().unwrap_or_else(|err| err.into_inner());
            sessions
                .entry(key)
                .or_default()
                .recaps
                .insert(data.year, data.clone());
            Ok::<RecapData, CoreError>(data)
        }
        .await;
        self.note_traffic(&api, before);
        answer
    }

    // -- moods ---------------------------------------------------------------

    /// The mixes this device already knows (disk, from the last visit).
    pub fn cached_moods(&self) -> Option<Vec<MoodRow>> {
        wire::read_snapshot(self, "moods")
    }

    /// `GET /api/moods` — the catalog, with a COUNT per card.
    pub async fn moods(&self) -> Result<Vec<MoodRow>, CoreError> {
        let api = wire::tagged(self)?;
        let before = wire::totals(&api);
        let answer = async {
            let answer = wire::fetch_moods(&api).await?;
            let rows: Vec<MoodRow> = answer
                .moods
                .into_iter()
                .map(|mood| MoodRow {
                    id: mood.id,
                    name: mood.name,
                    subtitle: mood.subtitle,
                    tracks: mood.tracks,
                    ready: mood.ready,
                    cover: mood.cover,
                })
                .collect();
            if let Err(cause) = wire::write_snapshot(self, "moods", &rows) {
                tracing::debug!(%cause, "the mood catalog was not kept");
            }
            Ok::<Vec<MoodRow>, CoreError>(rows)
        }
        .await;
        self.note_traffic(&api, before);
        answer
    }

    /// `GET /api/moods/<id>` — one request per tap (`Home.svelte#playMood`).
    /// Named `mood_mix` because MB5's `home` owns `mood_tracks` on `Core`; the
    /// two return different shapes (a module's own mix record here).
    pub async fn mood_mix(&self, id: &str) -> Result<MoodTracks, CoreError> {
        if id.trim().is_empty() {
            return Err(CoreError::Invalid {
                detail: "Ce mix n'existe pas".to_owned(),
            });
        }
        let api = wire::tagged(self)?;
        let before = wire::totals(&api);
        let answer = async {
            let mood = wire::fetch_mood(&api, id).await?;
            Ok::<MoodTracks, CoreError>(MoodTracks {
                id: mood.id,
                name: mood.name,
                subtitle: mood.subtitle,
                ready: mood.ready,
                reason: mood.reason.unwrap_or_default(),
                tracks: mood.tracks.iter().map(track_card).collect(),
            })
        }
        .await;
        self.note_traffic(&api, before);
        answer
    }

    // -- Explorer ------------------------------------------------------------

    /// The styles this device already has (the SPA's `localStorage` copy, kept
    /// in the CACHE directory instead).
    pub fn cached_explorer_styles(&self) -> Option<ExplorerStylesData> {
        wire::read_snapshot(self, "explorer-styles")
    }

    /// `GET /api/explorer/styles` — the ten parent styles this person listens
    /// to. No provider is asked. A server without the route is older than the
    /// feature (rule 7): `Unsupported`, and Explorer's entry hides.
    pub async fn explorer_styles(&self) -> Result<ExplorerStylesData, CoreError> {
        let api = wire::tagged(self)?;
        let before = wire::totals(&api);
        let answer = async {
            let answer = match wire::fetch_explorer_styles(&api).await {
                Ok(answer) => answer,
                Err(CoreError::Server { status: 404, .. }) => {
                    return Err(CoreError::Unsupported {
                        feature: "la découverte par style".to_owned(),
                    });
                }
                Err(cause) => return Err(cause),
            };
            let tiles = parent_styles(&answer.styles);
            let data = ExplorerStylesData {
                max_tracks: max_tracks(&tiles),
                tiles,
                source: answer.source,
                tracks: answer.tracks,
                rows: answer.styles,
            };
            if let Err(cause) = wire::write_snapshot(self, "explorer-styles", &data) {
                tracing::debug!(%cause, "the Explorer styles were not kept");
            }
            Ok::<ExplorerStylesData, CoreError>(data)
        }
        .await;
        self.note_traffic(&api, before);
        answer
    }

    /// `POST /api/explorer/discover` — ONE sweep, started by the person's own
    /// « Découvrir ». A busy server (`429`/`503`) answers its own state instead
    /// of an error, exactly as the panel waits instead of failing.
    pub async fn explorer_discover(
        &self,
        style: &str,
        level: u32,
    ) -> Result<DiscoverData, CoreError> {
        let style = style.trim().to_owned();
        if style.is_empty() {
            return Err(CoreError::Invalid {
                detail: "Choisissez un style".to_owned(),
            });
        }
        let level = clamp_level(level);
        let api = wire::tagged(self)?;
        let before = wire::totals(&api);
        let answer = async {
            match wire::start_explorer_discover(&api, &style, level).await {
                Ok(answer) => Ok(DiscoverData::from_answer(&answer, &style, level)),
                Err(cause) => Ok(busy_or_error(cause, &style, level)?),
            }
        }
        .await;
        self.note_traffic(&api, before);
        answer
    }

    /// `GET /api/explorer/discover?style=&level=` — the poll. It NEVER starts
    /// work: `idle` sends the panel back to its button, never a spinner waiting
    /// on a job nobody launched.
    pub async fn explorer_poll(&self, style: &str, level: u32) -> Result<DiscoverData, CoreError> {
        let style = style.trim().to_owned();
        if style.is_empty() {
            return Err(CoreError::Invalid {
                detail: "Choisissez un style".to_owned(),
            });
        }
        let level = clamp_level(level);
        let api = wire::tagged(self)?;
        let before = wire::totals(&api);
        let answer = async {
            match wire::poll_explorer_discover(&api, &style, level).await {
                Ok(answer) => Ok(DiscoverData::from_answer(&answer, &style, level)),
                Err(cause) => Ok(busy_or_error(cause, &style, level)?),
            }
        }
        .await;
        self.note_traffic(&api, before);
        answer
    }

    /// The all-stations list this device already has.
    pub fn cached_station_styles(&self) -> Option<Vec<StationRow>> {
        wire::read_snapshot(self, "station-styles")
    }

    /// `GET /api/flow/styles` — every analysed genre, which is the all-stations
    /// sheet's list (`stylesOf`) and MB5's Flow vocabulary in one answer.
    pub async fn station_styles(&self) -> Result<Vec<StationRow>, CoreError> {
        let api = wire::tagged(self)?;
        let before = wire::totals(&api);
        let answer = async {
            let answer = match wire::fetch_station_styles(&api).await {
                Ok(answer) => answer,
                Err(CoreError::Server { status: 404, .. }) => {
                    return Err(CoreError::Unsupported {
                        feature: "les stations".to_owned(),
                    });
                }
                Err(cause) => return Err(cause),
            };
            let rows = station_rows(&answer);
            if let Err(cause) = wire::write_snapshot(self, "station-styles", &rows) {
                tracing::debug!(%cause, "the station list was not kept");
            }
            Ok::<Vec<StationRow>, CoreError>(rows)
        }
        .await;
        self.note_traffic(&api, before);
        answer
    }

    /// `GET /api/radio/genre/<style>` — one station: its canonical node and the
    /// titles to play now (one request, no second before the first note).
    pub async fn genre_station(&self, style: &str) -> Result<StationData, CoreError> {
        let asked = style.trim().to_owned();
        if asked.is_empty() {
            return Err(CoreError::Invalid {
                detail: "Choisissez un style".to_owned(),
            });
        }
        let api = wire::tagged(self)?;
        let before = wire::totals(&api);
        let answer = async {
            let payload = wire::fetch_genre_station(&api, &asked).await?;
            let context = station_context(&asked, &payload).ok_or_else(|| CoreError::Invalid {
                detail: "Cette station n'a pas de style".to_owned(),
            })?;
            let tracks: Vec<TrackCard> = station_tracks(&payload).iter().map(track_card).collect();
            Ok::<StationData, CoreError>(StationData {
                kind: context.kind,
                id: context.id,
                title: context.title,
                name: payload.name,
                tracks,
            })
        }
        .await;
        self.note_traffic(&api, before);
        answer
    }

    // -- what the person can also do here ------------------------------------

    /// `POST /api/discover/mute` — never propose this artist again.
    pub async fn mute_artist(&self, provider: &str, id: &str) -> Result<(), CoreError> {
        let provider = provider.trim();
        let id = id.trim();
        if provider.is_empty() || id.is_empty() {
            return Err(CoreError::Invalid {
                detail: "Cet artiste ne peut pas être masqué".to_owned(),
            });
        }
        let api = wire::tagged(self)?;
        let before = wire::totals(&api);
        let answer = wire::mute_artist(&api, provider, id).await;
        self.note_traffic(&api, before);
        answer
    }

    /// `GET /api/recommend?d=` — what the discovery level proposes, at the
    /// person's own level (`discovery.js#settle`). The level itself is MB5's
    /// (`home::Core::discovery_level`): this lot does not own a second copy.
    pub async fn recommendations(&self, level: f64) -> Result<Vec<TrackCard>, CoreError> {
        let level = clamp_level_f(level);
        let api = wire::tagged(self)?;
        let before = wire::totals(&api);
        let answer = async {
            let answer = wire::fetch_recommend(&api, level).await?;
            Ok::<Vec<TrackCard>, CoreError>(
                answer
                    .tracks
                    .iter()
                    .take(RECO_ROWS)
                    .map(track_card)
                    .collect(),
            )
        }
        .await;
        self.note_traffic(&api, before);
        answer
    }

    // -- internals -----------------------------------------------------------

    /// Books one screen-open's traffic in the core's meter, so `NzCore::stats()`
    /// answers what this screen cost without the UI counting anything itself.
    fn note_traffic(&self, api: &Api, before: wire::Totals) {
        let after = wire::totals(api);
        let requests = after.requests.saturating_sub(before.requests);
        if requests == 0 {
            return;
        }
        // `Meter::record` counts ONE request; the run's bytes go with the first.
        self.meter().record(
            TAG_DISCOVERY,
            after.bytes_in.saturating_sub(before.bytes_in),
            after.bytes_out.saturating_sub(before.bytes_out),
        );
        for _ in 1..requests {
            self.meter().record(TAG_DISCOVERY, 0, 0);
        }
    }
}

/// `discovery.js#clamp`: the level is a 0…1 value.
fn clamp_level_f(level: f64) -> f64 {
    if level.is_finite() {
        level.clamp(0.0, 1.0)
    } else {
        crate::home::DEFAULT_LEVEL
    }
}

/// A refused discovery: a busy server is a WAIT (`429`/`503`, its own
/// `Retry-After` when the lane carries it), anything else is the cause.
fn busy_or_error(cause: CoreError, style: &str, level: u32) -> Result<DiscoverData, CoreError> {
    match &cause {
        CoreError::NotReady { retry_after_s } => Ok(DiscoverData {
            phase: Some(DiscoverPhase::Busy),
            style: style.to_owned(),
            level,
            busy_status: 503,
            retry_after_s: *retry_after_s,
            ..DiscoverData::default()
        }),
        CoreError::Server { status, .. } if matches!(status, 429 | 503) => Ok(DiscoverData {
            phase: Some(DiscoverPhase::Busy),
            style: style.to_owned(),
            level,
            busy_status: *status,
            retry_after_s: 0,
            ..DiscoverData::default()
        }),
        _ => Err(cause),
    }
}

impl ReleasesData {
    /// `_user_releases`'s answer, assembled: the cards, the flat tracks, the
    /// two shelves and the per-release groups.
    pub fn from_feed(feed: &nz_model::Releases) -> ReleasesData {
        let releases: Vec<ReleaseRow> = feed
            .releases
            .iter()
            .map(|release| release_row(release, 0))
            .collect();
        let tracks: Vec<TrackCard> = feed.tracks.iter().map(track_card).collect();
        let mut data = ReleasesData {
            count: tracks.len() as u32,
            unseen: feed.unseen,
            pending: feed.pending,
            tracks,
            releases,
            ..ReleasesData::default()
        };
        for track in &data.tracks {
            if let Some(key) = track.release.as_deref()
                && let Some(row) = data.releases.iter_mut().find(|row| row.key == key)
            {
                row.track_count = row.track_count.saturating_add(1);
            }
        }
        // A title that arrived with the latest check badges its release too:
        // the server sets `new` on both, and the group's label reads the
        // release (`Releases.svelte#groupReleases`).
        for (index, track) in feed.tracks.iter().enumerate() {
            if track.new != Some(true) {
                continue;
            }
            let Some(key) = data.tracks[index].release.clone() else {
                continue;
            };
            if let Some(row) = data.releases.iter_mut().find(|row| row.key == key) {
                row.new = true;
            }
        }
        data.shelves = build_shelves(&data);
        let (groups, others) = group_releases(&data);
        data.groups = groups;
        data.others = others;
        data.release_total = (data.groups.len() + data.others.len()) as u32;
        data
    }
}

/// One `/api/releases` card, as the screen draws it.
fn release_row(release: &Release, track_count: u32) -> ReleaseRow {
    let key = release.deezer_id.to_string();
    let kind = release.kind.clone().unwrap_or_default();
    let kind_label = kind_label(&kind).to_owned();
    let artist = {
        let name = release.artist.name.trim();
        if name.is_empty() {
            release.subtitle.clone().unwrap_or_default()
        } else {
            name.to_owned()
        }
    };
    let date_label = release
        .released
        .as_deref()
        .map(short_date)
        .unwrap_or_default();
    let close_to = release.close_to.clone();
    let who = match close_to.as_deref() {
        Some(name) if !name.is_empty() => format!("{artist} · proche de {name}"),
        _ => artist.clone(),
    };
    // `Releases.svelte#card`: [who, kind, date].filter(Boolean).join(" · ").
    let subtitle = [
        who,
        if kind == "single" {
            "Single".to_owned()
        } else if kind == "ep" {
            "EP".to_owned()
        } else {
            String::new()
        },
        date_label.clone(),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    ReleaseRow {
        key,
        title: release.title.clone(),
        cover: release.cover.clone(),
        artist,
        artist_id: {
            let id = release.artist.deezer_id.to_string();
            (!id.is_empty()).then_some(id)
        },
        kind,
        kind_label,
        released: release.released.clone(),
        date_label,
        nb_tracks: release.nb_tracks.unwrap_or(0),
        close_to,
        subtitle,
        new: release.new,
        track_count,
    }
}

/// `Releases.svelte#KIND`: the French label of a release kind.
fn kind_label(kind: &str) -> &'static str {
    match kind {
        "album" => "Album",
        "single" => "Single",
        "ep" => "EP",
        "compile" | "compilation" => "Compilation",
        _ => "Sortie",
    }
}

/// `toLocaleDateString("fr-FR", {day: "numeric", month: "short"})`: "12 nov.".
fn short_date(released: &str) -> String {
    let bytes = released.as_bytes();
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return String::new();
    }
    let (Ok(day), Ok(month)) = (
        released[8..10].parse::<u32>(),
        released[5..7].parse::<usize>(),
    ) else {
        return String::new();
    };
    if !(1..=12).contains(&month) {
        return String::new();
    }
    format!("{day} {}", recap::MONTHS_SHORT[month - 1])
}

/// `Releases.svelte#buildReleaseShelves`, ported: the flat track list split on
/// each release card's `close_to`, the cards being the authority on it.
pub fn build_shelves(data: &ReleasesData) -> Vec<ReleaseShelfRow> {
    let mut shelves: Vec<ReleaseShelfRow> = ["artists", "discovery"]
        .into_iter()
        .map(|key| {
            let copy = shelf_copy(key).expect("both kinds exist");
            ReleaseShelfRow {
                key: key.to_owned(),
                title: copy.title.to_owned(),
                subtitle: copy.short.to_owned(),
                ..ReleaseShelfRow::default()
            }
        })
        .collect();

    for track in &data.tracks {
        let Some(release_key) = track.release.as_deref().filter(|key| !key.is_empty()) else {
            continue;
        };
        let card = data.releases.iter().find(|row| row.key == release_key);
        let discovery = card
            .and_then(|row| row.close_to.as_deref())
            .is_some_and(|name| !name.is_empty());
        let shelf = &mut shelves[if discovery { 1 } else { 0 }];
        shelf.track_count = shelf.track_count.saturating_add(1);
        shelf.duration_s += u64::from(track.duration_s);
        if shelf.preview.len() < SHELF_PREVIEW {
            shelf.preview.push(track.clone());
        }
        if !shelf.releases.iter().any(|row| row.key == release_key) {
            let row = match card {
                Some(card) => card.clone(),
                None => ReleaseRow {
                    key: release_key.to_owned(),
                    title: track.album.clone(),
                    cover: track.cover.clone(),
                    ..ReleaseRow::default()
                },
            };
            shelf.mosaic.push(row.cover.clone().unwrap_or_default());
            shelf.releases.push(row);
        }
    }

    for shelf in &mut shelves {
        shelf.mosaic.retain(|cover| !cover.is_empty());
        shelf.mosaic.truncate(4);
        if shelf.key == "discovery" {
            let mut names: Vec<String> = Vec::new();
            for row in &shelf.releases {
                if let Some(name) = row.close_to.as_deref()
                    && !name.is_empty()
                    && !names.iter().any(|kept| kept == name)
                {
                    names.push(name.to_owned());
                }
            }
            shelf.close_names = names;
        }
    }
    // The SPA drops a shelf with no title (`cards` filters on `tracks.length`).
    shelves.retain(|shelf| shelf.track_count > 0);
    shelves
}

/// `Releases.svelte#groupReleases`: the releases that hold titles, newest
/// first (the feed's own order), and the ones that hold none.
pub fn group_releases(data: &ReleasesData) -> (Vec<ReleaseGroup>, Vec<ReleaseRow>) {
    let mut groups: Vec<ReleaseGroup> = Vec::new();
    for track in &data.tracks {
        let Some(key) = track.release.as_deref().filter(|key| !key.is_empty()) else {
            continue;
        };
        let release = data
            .releases
            .iter()
            .find(|row| row.key == key)
            .cloned()
            .unwrap_or_else(|| ReleaseRow {
                key: key.to_owned(),
                title: track.album.clone(),
                cover: track.cover.clone(),
                ..ReleaseRow::default()
            });
        match groups.iter_mut().find(|group| group.release.key == key) {
            Some(group) => group.tracks.push(track.clone()),
            None => groups.push(ReleaseGroup {
                artist: track.artists.clone(),
                label: String::new(),
                release,
                tracks: vec![track.clone()],
            }),
        }
    }
    for group in &mut groups {
        let count = group.tracks.len();
        let plural = count > 1;
        let mut label = format!(
            "{} · {} titre{}",
            group.release.kind_label,
            count,
            if plural { "s" } else { "" }
        );
        if group.release.new {
            label.push_str(" · Nouveau");
        }
        group.label = label;
    }
    let listed: Vec<&str> = groups
        .iter()
        .map(|group| group.release.key.as_str())
        .collect();
    let others: Vec<ReleaseRow> = data
        .releases
        .iter()
        .filter(|row| !listed.contains(&row.key.as_str()))
        .cloned()
        .collect();
    (groups, others)
}

/// `ReleaseShelf.svelte#shelfDescription`: the "Découverte" shelf names the
/// favourites it is close to, three at most.
pub fn shelf_description(shelf: &ReleaseShelfRow) -> String {
    let fallback = shelf_copy(&shelf.key)
        .map(|copy| copy.description.to_owned())
        .unwrap_or_default();
    if shelf.key != "discovery" || shelf.close_names.is_empty() {
        return fallback;
    }
    let shown: Vec<&str> = shelf
        .close_names
        .iter()
        .take(3)
        .map(String::as_str)
        .collect();
    let more = shelf.close_names.len() - shown.len();
    if more == 0 {
        return format!("Des artistes proches de {}.", list_fr(&shown));
    }
    format!(
        "Des artistes proches de {} et {more} autre{}.",
        shown.join(", "),
        if more > 1 { "s" } else { "" }
    )
}

/// `ReleaseShelf.svelte#listFr`: "A, B et C".
fn list_fr(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_owned(),
        _ => format!(
            "{} et {}",
            names[..names.len() - 1].join(", "),
            names[names.len() - 1]
        ),
    }
}

/// One shelf's page from the kept feed: the SPA's banner facts, its mosaic and
/// its description, with the titles the caller windows.
fn build_shelf_detail(data: &ReleasesData, kind: &str, copy: ShelfCopy) -> ReleaseShelfDetail {
    let shelf = data
        .shelves
        .iter()
        .find(|shelf| shelf.key == kind)
        .cloned()
        .unwrap_or_else(|| ReleaseShelfRow {
            key: kind.to_owned(),
            title: copy.title.to_owned(),
            subtitle: copy.short.to_owned(),
            ..ReleaseShelfRow::default()
        });
    let tracks: Vec<TrackCard> = data
        .tracks
        .iter()
        .filter(|track| {
            let Some(key) = track.release.as_deref() else {
                return false;
            };
            shelf.releases.iter().any(|row| row.key == key)
        })
        .cloned()
        .collect();
    let total_duration_s = tracks.iter().map(|track| u64::from(track.duration_s)).sum();
    let releases_word = format!(
        "{} sortie{}",
        shelf.releases.len(),
        if shelf.releases.len() > 1 { "s" } else { "" }
    );
    ReleaseShelfDetail {
        key: shelf.key.clone(),
        title: shelf.title.clone(),
        subtitle: shelf.subtitle.clone(),
        description: shelf_description(&shelf),
        facts: format!(
            "{} titre{} · {releases_word}",
            tracks.len(),
            if tracks.len() > 1 { "s" } else { "" }
        ),
        total_duration_s,
        mosaic: shelf.mosaic.clone(),
        releases: shelf.releases.clone(),
        tracks,
    }
}

impl ReleaseSettingsData {
    fn from_status(status: &nz_model::ReleasesSettings) -> ReleaseSettingsData {
        ReleaseSettingsData {
            on: status.on,
            every: status.every,
            window: status.window,
            close: status.close,
            listened: status.listened,
            watched: status.watched,
            favourites: status.favourites,
            links: status.links,
            asked: status.asked,
            releases: status.releases,
            running: status.running,
            last_run: status.last_run.clone().unwrap_or_default(),
            last_error: status.last_error.clone().unwrap_or_default(),
        }
    }
}

impl RecapYearsData {
    fn from_answer(answer: &nz_model::RecapYears) -> RecapYearsData {
        let now = Some(answer.now_year).filter(|year| *year != 0);
        RecapYearsData {
            chips: recap::year_chips(&answer.years, now),
            advertised: recap::advertised_year(&answer.years, now),
            years: answer.years.clone(),
            latest: answer.latest,
            now_year: answer.now_year,
            min_year: answer.min_year,
        }
    }
}

impl RecapData {
    /// Assembles one `/api/recap/<year>` payload: the deck, the glance panel and
    /// the share text, every word from [`recap`].
    pub fn assemble(payload: &nz_model::Recap, years: Option<&RecapYearsData>) -> RecapData {
        let slides: Vec<RecapSlide> = recap::card_ids(payload)
            .iter()
            .map(|id| slide(payload, id))
            .collect();
        let reason = recap::empty_reason(payload);
        let empty = reason.is_some() || slides.is_empty();
        let year = payload.year;
        let empty_reason = if empty { reason } else { None };
        let (empty_title, empty_text) = match empty_reason {
            Some(EmptyReason::SkipsOnly) => (
                format!("Aucune écoute en {year}"),
                "Seulement des titres passés cette année-là.".to_owned(),
            ),
            Some(EmptyReason::Thin) => (
                format!("Trop peu d'écoutes en {year}"),
                format!(
                    "{}, {} — pas assez pour raconter une année.",
                    count(payload.totals.listens, "écoute", "écoutes"),
                    recap::format_minutes(f64::from(payload.totals.minutes))
                ),
            ),
            Some(EmptyReason::None) => (
                format!("Rien à raconter pour {year}"),
                "Aucune écoute enregistrée cette année-là.".to_owned(),
            ),
            None if empty => (
                format!("Rien à raconter pour {year}"),
                "Aucune écoute enregistrée cette année-là.".to_owned(),
            ),
            None => (String::new(), String::new()),
        };
        let nearby_year = years.and_then(|state| nearest_year(state, year));
        RecapData {
            year,
            partial: payload.partial,
            enough: payload.enough,
            period_label: recap::period_label(payload),
            minutes: payload.totals.minutes,
            listens: payload.totals.listens,
            minutes_label: recap::format_number(f64::from(payload.totals.minutes)),
            listens_label: recap::format_number(f64::from(payload.totals.listens)),
            poster_lines: recap::poster_lines(payload),
            share_text: recap::share_text(payload),
            mosaic: recap::mosaic_covers(payload),
            top_artist: payload
                .tops
                .artists
                .first()
                .map(|row| row.name.clone())
                .unwrap_or_default(),
            top_genre: recap::genre_rows(payload)
                .first()
                .map(|row| row.name.clone())
                .unwrap_or_default(),
            biggest_day: payload
                .biggest_day
                .as_ref()
                .filter(|day| day.listens > 0)
                .map(|day| count(day.listens, "écoute", "écoutes"))
                .unwrap_or_default(),
            empty_reason,
            empty_title,
            empty_text,
            nearby_year,
            top_tracks: recap::rank_tracks(payload)
                .iter()
                .map(|row| track_card(&row.track))
                .collect(),
            slides,
        }
    }
}

/// `Recap.svelte#nearestYear`: the closest other year that holds an event.
fn nearest_year(state: &RecapYearsData, current: i32) -> Option<i32> {
    let rows: Vec<&nz_model::RecapYearRow> = state
        .years
        .iter()
        .filter(|row| row.year != current && row.events > 0)
        .collect();
    let full: Vec<&&nz_model::RecapYearRow> = rows.iter().filter(|row| row.enough).collect();
    let pool: Vec<i32> = if full.is_empty() {
        rows.iter().map(|row| row.year).collect()
    } else {
        full.iter().map(|row| row.year).collect()
    };
    pool.into_iter()
        .min_by_key(|year| ((year - current).abs(), *year))
}

impl RecapSlide {
    fn new(payload: &nz_model::Recap, id: &str) -> RecapSlide {
        RecapSlide {
            id: id.to_owned(),
            kicker: recap::slide_kicker(payload, id),
            title: recap::slide_title(payload, id),
            glyph: glyph_of(id).to_owned(),
            ..RecapSlide::default()
        }
    }
}

/// `RecapSlide.svelte#ICONS`.
fn glyph_of(id: &str) -> &'static str {
    match id {
        "year" => "sparkles",
        "minutes" => "timer",
        "artists" => "user",
        "tracks" => "music",
        "albums" => "disc",
        "genres" => "tag",
        "streak" => "activity",
        "share" => "share",
        _ => "music",
    }
}

/// One card, filled the way `RecapSlide.svelte` lays it out.
fn slide(payload: &nz_model::Recap, id: &str) -> RecapSlide {
    let mut slide = RecapSlide::new(payload, id);
    match id {
        "year" => {
            slide.lede = format!(
                "{} · {}",
                count(payload.totals.listens, "écoute", "écoutes"),
                recap::format_minutes(f64::from(payload.totals.minutes))
            );
            slide.mosaic = recap::mosaic_covers(payload);
            slide.footnote = recap::period_label(payload);
        }
        "minutes" => {
            slide.lede = format!(
                "soit {} de musique",
                recap::format_span(payload.totals.seconds as f64)
            );
            slide.bars = recap::month_bars(payload);
            slide.text = "écoutes par mois".to_owned();
            if let Some((label, _)) = recap::busiest_month(payload) {
                slide.footnote = format!("Ton mois le plus fort : {label}");
            }
        }
        "artists" => {
            slide.rows = recap::rank_artists(payload)
                .iter()
                .enumerate()
                .map(|(index, row)| RecapSlideRow {
                    key: if row.id.is_empty() {
                        row.name.clone()
                    } else {
                        row.id.clone()
                    },
                    rank: index as u32 + 1,
                    title: row.name.clone(),
                    subtitle: count(row.listens, "écoute", "écoutes"),
                    value: String::new(),
                    cover: row.cover.clone(),
                    share: 0.0,
                })
                .collect();
        }
        "tracks" => {
            slide.rows = recap::rank_tracks(payload)
                .iter()
                .enumerate()
                .map(|(index, row)| RecapSlideRow {
                    key: track_key(&row.track),
                    rank: index as u32 + 1,
                    title: row.track.title.clone(),
                    subtitle: artist_line(&row.track),
                    value: count(row.listens, "écoute", "écoutes"),
                    cover: row.track.album.cover.clone(),
                    share: 0.0,
                })
                .collect();
        }
        "albums" => {
            slide.rows = recap::rank_albums(payload)
                .iter()
                .enumerate()
                .map(|(index, row)| RecapSlideRow {
                    key: if row.id.is_empty() {
                        row.title.clone()
                    } else {
                        row.id.clone()
                    },
                    rank: index as u32 + 1,
                    title: row.title.clone(),
                    subtitle: row.artist.clone().unwrap_or_default(),
                    value: count(row.listens, "écoute", "écoutes"),
                    cover: row.cover.clone(),
                    share: 0.0,
                })
                .collect();
        }
        "genres" => {
            let genres = &payload.tops.genres;
            slide.lede = format!(
                "sur {} % de tes écoutes analysées",
                percent(genres.coverage)
            );
            slide.text = count(genres.explored, "style exploré", "styles explorés");
            slide.rows = recap::genre_rows(payload)
                .iter()
                .enumerate()
                .map(|(index, row)| RecapSlideRow {
                    key: row.name.clone(),
                    rank: index as u32 + 1,
                    title: row.name.clone(),
                    subtitle: String::new(),
                    value: format!("{} %", percent(row.share)),
                    cover: None,
                    share: row.share.clamp(0.0, 1.0),
                })
                .collect();
        }
        "streak" => {
            if let Some(streak) = &payload.streak
                && !streak.from_date.is_empty()
                && !streak.to.is_empty()
            {
                slide.text = streak_range(&streak.from_date, &streak.to);
            }
            if let Some(day) = &payload.biggest_day
                && !day.date.is_empty()
            {
                slide.footnote = format!(
                    "Ta plus grosse journée : le {}, {}",
                    day_month_iso(&day.date),
                    count(day.listens, "écoute", "écoutes")
                );
            }
        }
        "firstlast" => {
            if let Some(first) = &payload.first
                && !first.track.title.is_empty()
            {
                slide.rows.push(RecapSlideRow {
                    key: track_key(&first.track),
                    rank: 1,
                    title: first.track.title.clone(),
                    subtitle: artist_line(&first.track),
                    value: first
                        .at
                        .as_deref()
                        .map(|at| format!("le {}", day_month_iso(at)))
                        .unwrap_or_default(),
                    cover: first.track.album.cover.clone(),
                    share: 0.0,
                });
            }
            if let Some(last) = &payload.last
                && !last.track.title.is_empty()
            {
                slide.rows.push(RecapSlideRow {
                    key: track_key(&last.track),
                    rank: 2,
                    title: last.track.title.clone(),
                    // The card labels the second line "Ton dernier titre".
                    subtitle: artist_line(&last.track),
                    value: last
                        .at
                        .as_deref()
                        .map(|at| format!("le {}", day_month_iso(at)))
                        .unwrap_or_default(),
                    cover: last.track.album.cover.clone(),
                    share: 0.0,
                });
            }
        }
        "share" => {
            slide.poster = recap::poster_lines(payload);
        }
        _ => {}
    }
    slide
}

/// `RecapSlide.svelte#count`: "1 écoute", "41 écoutes".
fn count(n: u32, one: &str, many: &str) -> String {
    format!(
        "{} {}",
        recap::format_number(f64::from(n)),
        if n > 1 { many } else { one }
    )
}

/// `RecapSlide.svelte#percent`: 0…100, rounded.
fn percent(share: f64) -> u32 {
    if !share.is_finite() {
        return 0;
    }
    (share.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// `RecapSlide.svelte#streakRange`: "du 1ᵉʳ au 23 mars", or both months.
fn streak_range(from: &str, to: &str) -> String {
    let (Some(from), Some(to)) = (iso_parts(from), iso_parts(to)) else {
        return String::new();
    };
    let (from_year, from_month, from_day) = from;
    let (to_year, to_month, to_day) = to;
    if from_year == to_year && from_month == to_month {
        return format!(
            "du {} au {} {}",
            if from_day == 1 {
                "1ᵉʳ".to_owned()
            } else {
                from_day.to_string()
            },
            to_day,
            recap::MONTHS[(to_month as usize - 1).min(11)]
        )
        .trim()
        .to_owned();
    }
    format!(
        "du {} au {}",
        day_month(from_day, from_month),
        day_month(to_day, to_month)
    )
}

fn iso_parts(value: &str) -> Option<(i32, u32, u32)> {
    let bytes = value.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    Some((
        value.get(0..4)?.parse().ok()?,
        value.get(5..7)?.parse().ok()?,
        value.get(8..10)?.parse().ok()?,
    ))
}

fn day_month(day: u32, month: u32) -> String {
    let name = recap::MONTHS[(month as usize - 1).min(11)];
    if day == 1 {
        format!("1ᵉʳ {name}")
    } else {
        format!("{day} {name}")
    }
}

/// `RecapSlide.svelte#dayMonth`: the day and month of an ISO moment.
fn day_month_iso(value: &str) -> String {
    match iso_parts(value) {
        Some((_, month, day)) => day_month(day, month),
        None => String::new(),
    }
}

impl DiscoverData {
    fn from_answer(answer: &wire::DiscoverAnswer, style: &str, level: u32) -> DiscoverData {
        if let Some(error) = &answer.error {
            // A sweep that reported an error is OVER: the panel says the
            // provider is the cause and offers « Réessayer » (a POST again).
            tracing::debug!(error = %error, "a discovery reported an error");
            return DiscoverData {
                phase: Some(DiscoverPhase::Idle),
                style: style.to_owned(),
                level,
                ..DiscoverData::default()
            };
        }
        let playlists: Vec<ExplorerPlaylist> = answer
            .playlists
            .iter()
            .map(|playlist| ExplorerPlaylist {
                id: playlist.id.clone(),
                title: if playlist.title.trim().is_empty() {
                    "Playlist sans titre".to_owned()
                } else {
                    playlist.title.clone()
                },
                cover: playlist.cover.clone(),
                count_label: verified_label(playlist),
                tracks: playlist.tracks.iter().map(track_card).collect(),
            })
            .collect();
        let phase = match answer.state.as_str() {
            "ready" => {
                if playlists.is_empty() {
                    DiscoverPhase::Empty
                } else {
                    DiscoverPhase::Ready
                }
            }
            "pending" => DiscoverPhase::Pending,
            _ => DiscoverPhase::Idle,
        };
        DiscoverData {
            phase: Some(phase),
            style: if answer.style.is_empty() {
                style.to_owned()
            } else {
                answer.style.clone()
            },
            level: if answer.level == 0 {
                level
            } else {
                answer.level
            },
            playlists,
            busy_status: 0,
            retry_after_s: 0,
        }
    }
}

/// One catalogue title, as this lot's screens draw and queue it
/// (`home::wire#track_card`, the same rule: a library row by its UUID, a
/// catalogue row by the provider's id).
pub fn track_card(track: &Track) -> TrackCard {
    let row_id = track.id.as_ref().map(|id| id.to_string());
    let deezer = track.deezer_id.to_string();
    let source = match track.source.as_deref() {
        Some("youtube") => "youtube",
        _ if track.local => "upload",
        _ => "deezer",
    };
    TrackCard {
        key: row_id.clone().unwrap_or_else(|| deezer.clone()),
        id: row_id.unwrap_or(deezer),
        title: track.title.clone(),
        artists: artist_line(track),
        album: track.album.title.clone(),
        album_id: {
            let id = track.album.deezer_id.to_string();
            (!id.is_empty()).then_some(id)
        },
        duration_s: track.duration,
        cover: track.album.cover.clone(),
        explicit: track.explicit,
        unavailable: track.unavailable,
        source: source.to_owned(),
        release: track
            .release
            .as_ref()
            .map(|release| release.deezer_id.to_string()),
    }
}

/// The queue's own key (`trackkey.js#trackRef`): the row's UUID, else the
/// provider's id.
fn track_key(track: &Track) -> String {
    track
        .id
        .as_ref()
        .map(|id| id.to_string())
        .unwrap_or_else(|| track.deezer_id.to_string())
}

/// `format.js#artistLine`: who performed it, joined for one line.
fn artist_line(track: &Track) -> String {
    if let Some(line) = track
        .display_artist
        .as_deref()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        return line.to_owned();
    }
    let names: Vec<&str> = track
        .artists
        .iter()
        .map(|artist| artist.name.trim())
        .filter(|name| !name.is_empty())
        .collect();
    if !names.is_empty() {
        return names.join(", ");
    }
    track.artist.name.clone()
}

/// The account the device's copies belong to: `/me`'s id, `""` before a session.
fn owner(core: &Core) -> String {
    core.me()
        .map(|me| me.user.id.to_string())
        .filter(|id| !id.is_empty())
        .unwrap_or_default()
}

/// FNV-1a, 64 bits — the same rule `nz-cache` uses for a file name: stable
/// across runs and platforms, so a snapshot stays readable after an upgrade.
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

    /// The two shelves split on each release card's `close_to`, and a shelf with
    /// no title is not drawn (`buildReleaseShelves` + `cards`).
    #[test]
    fn the_shelves_split_on_close_to_and_drop_empty_ones() {
        let feed: nz_model::Releases = serde_json::from_str(
            r#"{"releases":[
                {"deezer_id":"501","title":"Le matin","artist":{"deezer_id":"42","name":"Ascenseur"},
                 "kind":"album","released":"2026-11-12","nb_tracks":2,"new":true},
                {"deezer_id":"502","title":"Voisin","artist":{"deezer_id":"43","name":"June"},
                 "kind":"single","released":"2026-11-10","nb_tracks":1,"close_to":"Ascenseur"}],
               "tracks":[
                {"deezer_id":"9001","title":"Ouverture","duration":180,
                 "artist":{"deezer_id":"42","name":"Ascenseur"},
                 "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"},
                 "release":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"},
                 "new":true},
                {"deezer_id":"9002","title":"Suite","duration":210,
                 "artist":{"deezer_id":"42","name":"Ascenseur"},
                 "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"},
                 "release":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}},
                {"deezer_id":"9003","title":"Voisin","duration":200,
                 "artist":{"deezer_id":"43","name":"June"},
                 "album":{"deezer_id":"502","title":"Voisin","cover":"/api/cover/502"},
                 "release":{"deezer_id":"502","title":"Voisin","cover":"/api/cover/502"}}],
               "unseen":2,"pending":1}"#,
        )
        .expect("the SPA's own /releases answer decodes");
        let data = ReleasesData::from_feed(&feed);
        assert_eq!(data.count, 3);
        assert_eq!(data.unseen, 2);
        assert_eq!(data.fresh_label(), "2 nouvelles sorties");
        assert_eq!(data.shelves.len(), 2);
        assert_eq!(data.shelves[0].key, "artists");
        assert_eq!(data.shelves[0].track_count, 2);
        assert_eq!(data.shelves[0].releases[0].key, "501");
        assert_eq!(data.shelves[1].key, "discovery");
        assert_eq!(data.shelves[1].track_count, 1);
        assert_eq!(data.shelves[1].close_names, vec!["Ascenseur"]);
        assert_eq!(
            data.shelves[1].releases[0].subtitle,
            "June · proche de Ascenseur · Single · 10 nov."
        );
        assert_eq!(data.release_total, 2, "two releases, both with titles");
        assert!(data.others.is_empty());
    }

    /// "Toutes les sorties" groups by release, in the feed's order, and a release
    /// with no title becomes a plain card.
    #[test]
    fn the_per_release_list_groups_titles_and_keeps_the_empty_releases() {
        let feed: nz_model::Releases = serde_json::from_str(
            r#"{"releases":[
                {"deezer_id":"501","title":"Le matin","artist":{"deezer_id":"42","name":"Ascenseur"},
                 "kind":"album","released":"2026-11-12","nb_tracks":2},
                {"deezer_id":"503","title":"Pas encore lue","artist":{"deezer_id":"43","name":"June"},
                 "kind":"ep","released":"2026-11-01","nb_tracks":3}],
               "tracks":[
                {"deezer_id":"9001","title":"Ouverture","duration":180,
                 "artist":{"deezer_id":"42","name":"Ascenseur"},
                 "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"},
                 "release":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}},
                {"deezer_id":"9002","title":"Suite","duration":210,
                 "artist":{"deezer_id":"42","name":"Ascenseur"},
                 "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"},
                 "release":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"},"new":true}],
               "unseen":0,"pending":1}"#,
        )
        .expect("feed");
        let data = ReleasesData::from_feed(&feed);
        assert_eq!(data.groups.len(), 1);
        assert_eq!(data.groups[0].tracks.len(), 2);
        assert_eq!(data.groups[0].label, "Album · 2 titres · Nouveau");
        assert_eq!(data.others.len(), 1);
        assert_eq!(data.others[0].key, "503");
        assert_eq!(data.release_total, 2);
        assert_eq!(
            data.pending_label(),
            "1 sortie n'a pas encore son titre : le serveur le lit en arrière-plan."
        );
        assert_eq!(data.groups[0].tracks[0].release.as_deref(), Some("501"));
    }

    /// A shelf's page is built from the kept feed with the SPA's own banner
    /// facts and its description.
    #[test]
    fn a_shelf_page_has_the_banners_facts_and_description() {
        let feed: nz_model::Releases = serde_json::from_str(
            r#"{"releases":[
                {"deezer_id":"502","title":"Voisin","artist":{"deezer_id":"43","name":"June"},
                 "kind":"single","released":"2026-11-10","nb_tracks":1,"close_to":"Ascenseur"}],
               "tracks":[
                {"deezer_id":"9003","title":"Voisin","duration":200,
                 "artist":{"deezer_id":"43","name":"June"},
                 "album":{"deezer_id":"502","title":"Voisin","cover":"/api/cover/502"},
                 "release":{"deezer_id":"502","title":"Voisin","cover":"/api/cover/502"}}],
               "unseen":0,"pending":0}"#,
        )
        .expect("feed");
        let data = ReleasesData::from_feed(&feed);
        let shelf = build_shelf_detail(&data, "discovery", SHELF_DISCOVERY);
        assert_eq!(shelf.title, "Découverte");
        assert_eq!(shelf.facts, "1 titre · 1 sortie");
        assert_eq!(shelf.total_duration_s, 200);
        assert_eq!(shelf.tracks.len(), 1);
        assert_eq!(shelf.description, "Des artistes proches de Ascenseur.");
    }

    /// A row is a hash of the account, stable, and one account never reads
    /// another's copy.
    #[test]
    fn a_snapshot_is_named_after_a_hash_of_the_account() {
        assert_eq!(fnv1a(b"u-1"), fnv1a(b"u-1"));
        assert_ne!(fnv1a(b"u-1"), fnv1a(b"u-2"));
    }

    /// The card's date is the SPA's French short form, or nothing.
    #[test]
    fn a_release_date_reads_like_the_cards_own() {
        assert_eq!(short_date("2026-11-12"), "12 nov.");
        assert_eq!(short_date("2026-01-02"), "2 janv.");
        assert_eq!(short_date("2026-13-02"), "");
        assert_eq!(short_date("nope"), "");
    }

    /// Counted bytes: the measure of what a feed holds is a sum of real string
    /// lengths, never a process RSS (worker rule 4).
    #[test]
    fn the_feed_memory_is_counted_from_what_it_holds() {
        let feed: nz_model::Releases = serde_json::from_str(
            r#"{"releases":[
                {"deezer_id":"501","title":"Le matin","artist":{"deezer_id":"42","name":"Ascenseur"},
                 "kind":"album","released":"2026-11-12","nb_tracks":2}],
               "tracks":[
                {"deezer_id":"9001","title":"Ouverture","duration":180,
                 "artist":{"deezer_id":"42","name":"Ascenseur"},
                 "album":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"},
                 "release":{"deezer_id":"501","title":"Le matin","cover":"/api/cover/501"}}],
               "unseen":0,"pending":0}"#,
        )
        .expect("feed");
        let data = ReleasesData::from_feed(&feed);
        let bytes = data.memory_bytes();
        assert!(
            bytes > 200,
            "one release and one title count real bytes: {bytes}"
        );
        assert!(bytes < 8_192, "and not a megabyte: {bytes}");
        println!("MEASURE releases snapshot: 1 release + 1 title = {bytes} counted bytes");
    }

    /// The recap's stack keeps its own words: a thin year says why, and a year
    /// with no card is not a deck.
    #[test]
    fn a_thin_year_says_why_and_an_empty_deck_is_not_a_story() {
        let payload: nz_model::Recap = serde_json::from_str(
            r#"{"year":2026,"partial":true,"enough":false,"timezone":"server-local",
                "period":{"from":"2026-01-01","to":"2026-02-01"},
                "totals":{"events":5,"listens":5,"minutes":12},
                "months":[],"tops":{"tracks":[],"artists":[],"albums":[],
                                    "genres":{"available":false,"rows":[]}},
                "cards":[],"generated":1}"#,
        )
        .expect("recap");
        let data = RecapData::assemble(&payload, None);
        assert_eq!(data.empty_reason, Some(EmptyReason::Thin));
        assert_eq!(data.empty_title, "Trop peu d'écoutes en 2026");
        assert_eq!(
            data.empty_text,
            "5 écoutes, 12 minutes — pas assez pour raconter une année."
        );
        assert!(data.slides.is_empty());
    }

    /// The eight cards are laid out with the data each one draws, and the top
    /// titles are what "Lire ces 5 titres" queues.
    #[test]
    fn the_recap_deck_carries_every_cards_own_data() {
        let payload: nz_model::Recap = serde_json::from_str(
            r#"{"year":2026,"partial":true,"enough":true,"timezone":"server-local",
                "period":{"from":"2026-01-01","to":"2026-11-13"},
                "totals":{"events":2412,"listens":2180,"minutes":8540,"seconds":512400},
                "months":[{"month":7,"listens":300}],
                "streak":{"days":23,"from":"2026-03-01","to":"2026-03-23"},
                "biggest_day":{"date":"2026-07-04","listens":61},
                "tops":{"tracks":[{"track":{"deezer_id":"1","title":"Titre un",
                                           "album":{"deezer_id":"7","cover":"cover-a"}},"listens":41}],
                        "artists":[{"id":"a1","name":"June","listens":210}],
                        "albums":[{"id":"al1","title":"Premier","cover":"cover-a","listens":96}],
                        "genres":{"available":true,"coverage":0.72,"explored":17,
                                  "rows":[{"name":"Techno","listens":640,"share":0.41}]}},
                "first":{"at":"2026-01-02T08:11:00","track":{"deezer_id":"1","title":"Titre un"}},
                "last":{"at":"2026-11-13T20:58:00","track":{"deezer_id":"3","title":"Titre trois"}},
                "cards":["year","minutes","artists","tracks","albums","genres","streak","firstlast"],
                "generated":1}"#,
        )
        .expect("recap");
        let data = RecapData::assemble(&payload, None);
        assert_eq!(data.slides.len(), 9, "eight cards and the share card");
        let year = &data.slides[0];
        assert_eq!(year.glyph, "sparkles");
        assert_eq!(year.lede, "2\u{202f}180 écoutes · 8\u{202f}540 minutes");
        let minutes = &data.slides[1];
        assert_eq!(minutes.lede, "soit 5 j 22 h de musique");
        assert_eq!(minutes.bars.len(), 12);
        assert_eq!(minutes.footnote, "Ton mois le plus fort : juillet");
        assert_eq!(data.slides[2].rows[0].title, "June");
        assert_eq!(data.slides[2].rows[0].subtitle, "210 écoutes");
        assert_eq!(data.slides[3].rows[0].value, "41 écoutes");
        assert_eq!(data.slides[4].rows[0].subtitle, "");
        assert_eq!(data.slides[5].lede, "sur 72 % de tes écoutes analysées");
        assert_eq!(data.slides[5].text, "17 styles explorés");
        assert_eq!(data.slides[5].rows[0].value, "41 %");
        assert_eq!(data.slides[6].text, "du 1ᵉʳ au 23 mars");
        assert_eq!(
            data.slides[6].footnote,
            "Ta plus grosse journée : le 4 juillet, 61 écoutes"
        );
        assert_eq!(data.slides[7].rows.len(), 2);
        assert_eq!(data.slides[7].rows[1].title, "Titre trois");
        assert_eq!(data.slides[8].poster.len(), 6);
        assert_eq!(data.top_tracks.len(), 1);
        assert_eq!(data.top_artist, "June");
        assert_eq!(data.top_genre, "Techno");
        assert_eq!(data.biggest_day, "61 écoutes");
        assert_eq!(
            data.share_text,
            "Mon année 2026 (en cours) en musique : 2\u{202f}180 écoutes, 8\u{202f}540 minutes, avec June en tête."
        );
    }
}

/// The lot's own tests: this lot's request budget, its failure shapes and its
/// kept copies — against a server that COUNTS, never against a duration.
#[cfg(test)]
mod lot_tests {
    use super::*;
    use crate::core::NetworkState;

    /// A cold Nouveautés is the SPA's TWO requests (`GET /releases`, then the
    /// unseen badge's `POST /releases/seen`), and a warm open costs ZERO: the
    /// feed is on the device with its badge cleared.
    ///
    /// Numbers this test holds: 2 requests cold, 3 titles, 2 shelves; 0 warm;
    /// one `discovery` row in `NzCore::stats()` with 2 requests.
    #[tokio::test]
    async fn a_cold_releases_open_asks_the_spas_two_requests_and_a_warm_one_asks_none() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();

        let data = core.releases().await.expect("the feed");
        let asked = server.routes();
        assert_eq!(
            asked,
            vec![
                "GET /api/releases".to_owned(),
                "POST /api/releases/seen".to_owned()
            ],
            "exactly the SPA's two calls"
        );
        assert_eq!(data.count, 3);
        assert_eq!(data.unseen, 2, "this visit announces two new releases");
        assert_eq!(data.fresh_label(), "2 nouvelles sorties");
        assert_eq!(data.release_total, 2);
        assert_eq!(data.shelves.len(), 2);
        assert_eq!(
            data.shelves[1].subtitle,
            "Des artistes proches de tes favoris, et d'autres à découvrir."
        );

        let row = core
            .stats()
            .into_iter()
            .find(|row| row.tag == TAG_DISCOVERY)
            .expect("one discovery row");
        assert_eq!(row.requests, 2, "NzCore::stats() counts the same two");
        assert!(
            row.bytes_in > 0,
            "the bodies are counted: {} bytes",
            row.bytes_in
        );
        println!(
            "MEASURE releases cold: {} requests, {} bytes in, {} bytes out",
            row.requests, row.bytes_in, row.bytes_out
        );

        // A warm open paints from the device: no request at all, and the badge
        // is not announced a second time.
        server.forget();
        let kept = core.cached_releases().expect("the feed the visit kept");
        assert_eq!(server.count(), 0, "a warm Nouveautés asks nothing");
        assert_eq!(kept.count, 3);
        assert_eq!(kept.unseen, 0, "the seen releases are not new again");
        println!(
            "MEASURE releases snapshot kept: {} counted bytes",
            kept.memory_bytes()
        );
    }

    /// A dead link fails fast and keeps what the person already saw: no socket
    /// is opened to learn there is no link.
    #[tokio::test]
    async fn a_dead_link_opens_no_socket_and_the_feed_still_paints() {
        let (server, _dir, core) = testing::configured().await;
        core.releases().await.expect("one good visit");
        server.forget();
        let before = server.accepted();
        core.set_network(NetworkState::None);
        assert_eq!(
            core.releases().await.expect_err("offline"),
            CoreError::Offline,
            "the cause is the link, not a timeout"
        );
        assert_eq!(server.accepted(), before, "not one socket was opened");
        assert_eq!(server.count(), 0);
        assert!(core.cached_releases().is_some(), "the copy is what is kept");
    }

    /// A shelf page and a group's titles come from the feed the first screen
    /// already holds: the SPA's `ReleaseShelf.svelte` costs no request at all.
    #[tokio::test]
    async fn a_shelf_page_and_a_group_cost_zero_requests() {
        let (server, _dir, core) = testing::configured().await;
        core.releases().await.expect("one good visit");
        server.forget();

        let shelf = core.release_shelf("discovery").expect("the shelf");
        assert_eq!(shelf.key, "discovery");
        assert_eq!(shelf.title, "Découverte");
        assert_eq!(shelf.facts, "1 titre · 1 sortie");
        assert_eq!(shelf.total_duration_s, 200);
        assert_eq!(shelf.tracks.len(), 1);
        assert_eq!(shelf.tracks[0].title, "Voisin");
        assert_eq!(shelf.description, "Des artistes proches de Ascenseur.");

        let artists = core.release_shelf("artists").expect("the other shelf");
        assert_eq!(artists.tracks.len(), 2);
        assert_eq!(artists.facts, "2 titres · 1 sortie");

        assert_eq!(core.release_group_tracks("501").len(), 2);
        assert!(core.release_shelf("inconnu").is_none(), "not a shelf");
        assert_eq!(server.count(), 0, "zero requests, the feed is in memory");
    }

    /// A refused feed is a cause with a status, not a spinner: the screen shows
    /// `nz_error_text` and « Réessayer » (rule 3).
    #[tokio::test]
    async fn a_refused_feed_is_a_cause_with_its_status() {
        let (server, _dir, core) = testing::configured().await;
        server.break_route("/api/releases", 503);
        // `nz-api` retries a User GET three times on a transient 5xx: the cause
        // is the status, and the screen says it once.
        let cause = core.releases().await.expect_err("a refusal");
        match cause {
            CoreError::Server { status, .. } => assert_eq!(status, 503),
            other => panic!("expected a server refusal, got {other:?}"),
        }
        assert!(core.cached_releases().is_none(), "nothing was kept");
    }

    /// The years are asked ONCE per ten minutes (`recapyears.js#TTL`), so the
    /// home then the Récap cost one `/years` whatever the order.
    ///
    /// Numbers: 1 request, then 0, then 1 again once the copy is ten minutes old.
    #[tokio::test]
    async fn the_recap_years_are_asked_once_per_ten_minutes() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let years = core.recap_years().await.expect("the years");
        assert_eq!(server.routes(), vec!["GET /api/recap/years".to_owned()]);
        assert_eq!(years.chips.len(), 2, "the empty 2024 is not offered");
        assert_eq!(years.chips[0].label, "2026 · en cours");
        assert_eq!(years.advertised.map(|row| row.year), Some(2026));

        server.forget();
        let again = core.recap_years().await.expect("the kept years");
        assert_eq!(server.count(), 0, "a warm read asks nothing");
        assert_eq!(again.chips.len(), 2);

        // Ten minutes later the copy is stale: the next open asks again.
        age_recap_years(&core);
        server.forget();
        core.recap_years().await.expect("the years again");
        assert_eq!(server.routes(), vec!["GET /api/recap/years".to_owned()]);
    }

    /// One year is ONE payload; the nine cards, the chips and the poster then
    /// cost nothing, and a malformed year never reaches the server.
    #[tokio::test]
    async fn a_recap_year_is_one_request_and_its_cards_cost_nothing() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let recap = core.recap(2026).await.expect("the year");
        assert_eq!(
            server.routes(),
            vec![
                "GET /api/recap/years".to_owned(),
                "GET /api/recap/2026".to_owned()
            ],
            "the year plus the chips' side question"
        );
        assert_eq!(recap.year, 2026);
        assert_eq!(recap.slides.len(), 9);
        assert_eq!(recap.minutes, 8_540);
        assert_eq!(recap.top_artist, "Ascenseur");
        assert_eq!(recap.top_genre, "Techno");
        assert_eq!(recap.slides[6].text, "du 1ᵉʳ au 23 mars");
        assert!(recap.empty_reason.is_none());

        // A second open of the same year repaints from memory first, then asks
        // ONE request: the years are still fresh.
        assert!(core.cached_recap(2026).is_some());
        server.forget();
        core.recap(2026).await.expect("the year again");
        assert_eq!(server.routes(), vec!["GET /api/recap/2026".to_owned()]);

        server.forget();
        assert!(matches!(
            core.recap(26).await.expect_err("not a year"),
            CoreError::Invalid { .. }
        ));
        assert_eq!(
            server.count(),
            0,
            "a malformed year never reaches the server"
        );
    }

    /// The mixes are one request each, and the catalog is kept.
    #[tokio::test]
    async fn the_mixes_are_one_request_each_and_the_catalog_is_kept() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let moods = core.moods().await.expect("the catalog");
        assert_eq!(server.routes(), vec!["GET /api/moods".to_owned()]);
        assert_eq!(moods.len(), 2);
        assert!(!moods[1].ready, "an unready mix is still listed");

        server.forget();
        assert_eq!(core.cached_moods().expect("kept").len(), 2);
        assert_eq!(server.count(), 0, "the catalog is on the device");

        server.forget();
        let mix = core.mood_mix("calme").await.expect("the mix");
        assert_eq!(server.routes(), vec!["GET /api/moods/calme".to_owned()]);
        assert_eq!(mix.tracks.len(), 1);
        assert_eq!(mix.tracks[0].title, "Mix 1");
        assert_eq!(mix.tracks[0].to_row(0).source, "deezer");
    }

    /// Explorer's styles are ONE request, kept on the device; a server without
    /// the route is older than the feature (rule 7) and says `Unsupported`.
    #[tokio::test]
    async fn explorer_styles_are_one_request_kept_on_the_device() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let styles = core.explorer_styles().await.expect("the styles");
        assert_eq!(server.routes(), vec!["GET /api/explorer/styles".to_owned()]);
        assert_eq!(
            styles.tiles.len(),
            2,
            "Techno merges its leaf, House stands"
        );
        assert_eq!(styles.tiles[0].name, "Techno");
        assert_eq!(styles.tiles[0].tracks, 12);
        assert_eq!(styles.max_tracks, 12);
        assert_eq!(styles.source, "history");

        server.forget();
        assert_eq!(core.cached_explorer_styles().expect("kept").tiles.len(), 2);
        assert_eq!(server.count(), 0, "the styles are on the device");

        let (server, _dir, core) = testing::configured().await;
        server.break_route("/api/explorer/styles", 404);
        assert!(matches!(
            core.explorer_styles().await.expect_err("too old"),
            CoreError::Unsupported { .. }
        ));
    }

    /// One style's discovery: ONE POST starts it, and each poll is one GET that
    /// never starts a second sweep. A busy server is a WAIT, not an error.
    #[tokio::test]
    async fn a_discovery_is_one_post_then_polls() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let started = core.explorer_discover("Techno", 3).await.expect("started");
        assert_eq!(
            server.routes(),
            vec!["POST /api/explorer/discover".to_owned()]
        );
        assert_eq!(started.phase, Some(DiscoverPhase::Pending));
        let body: serde_json::Value =
            serde_json::from_str(&server.last_body("/api/explorer/discover").expect("body"))
                .expect("JSON");
        assert_eq!(body["style"], "Techno");
        assert_eq!(body["level"], 3);

        server.forget();
        let polled = core.explorer_poll("Techno", 3).await.expect("polled");
        assert_eq!(
            server.routes(),
            vec!["GET /api/explorer/discover".to_owned()]
        );
        let hit = server.hits().pop().expect("the poll");
        assert_eq!(hit.query("style").as_deref(), Some("Techno"));
        assert_eq!(hit.query("level").as_deref(), Some("3"));
        assert_eq!(polled.phase, Some(DiscoverPhase::Ready));
        assert_eq!(polled.playlists.len(), 2);
        assert_eq!(
            polled.playlists[0].count_label, "12 titres vérifiés sur 60",
            "the panel says what the count is worth"
        );
        assert_eq!(polled.playlists[0].tracks.len(), 1);

        // A busy server: 503 + its own wait, never a failure with a status.
        let (server, _dir, core) = testing::configured().await;
        server.break_route("/api/explorer/discover", 503);
        let busy = core.explorer_discover("Techno", 3).await.expect("a wait");
        assert_eq!(busy.phase, Some(DiscoverPhase::Busy));
        assert_eq!(busy.busy_status, 503);
        assert_eq!(busy.busy_cause(), "Le serveur est occupé.");
        assert_eq!(busy.busy_wait(5), "Réessaie dans 5 s.");
    }

    /// A station is ONE request and plays at once; the all-stations list is the
    /// ONE `/flow/styles` MB5's chips already share.
    #[tokio::test]
    async fn a_station_is_one_request_and_the_list_is_one_more() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let station = core.genre_station("Techno").await.expect("the station");
        assert_eq!(
            server.routes(),
            vec!["GET /api/radio/genre/Techno".to_owned()]
        );
        assert_eq!(station.kind, STATION_KIND);
        assert_eq!(station.id, "Techno");
        assert_eq!(station.title, "Station Techno");
        assert_eq!(station.tracks.len(), 1);

        server.forget();
        let rows = core.station_styles().await.expect("the stations");
        assert_eq!(server.routes(), vec!["GET /api/flow/styles".to_owned()]);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            filter_styles(&rows, "electro").len(),
            0,
            "the fold works on the real names too"
        );
        assert_eq!(filter_styles(&rows, "techno")[0].name, "Techno");

        server.forget();
        assert_eq!(core.cached_station_styles().expect("kept").len(), 3);
        assert_eq!(server.count(), 0);
    }

    /// A genre with a slash reaches `<path:style>` as one encoded segment, and
    /// muting an artist is one POST carrying who.
    #[tokio::test]
    async fn a_slash_genre_and_a_mute_are_one_request_each() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        core.genre_station("Disco / funk")
            .await
            .expect("the station");
        let hit = server.hits().pop().expect("the hit");
        assert_eq!(hit.route(), "/api/radio/genre/Disco%20%2F%20funk");

        server.forget();
        core.mute_artist("deezer", "42").await.expect("muted");
        assert_eq!(server.routes(), vec!["POST /api/discover/mute".to_owned()]);
        let body = server.last_body("/api/discover/mute").expect("a body");
        assert!(body.contains("\"provider\":\"deezer\""));
        assert!(body.contains("\"id\":\"42\""));
    }

    /// `/recommend` is ONE request carrying the person's level, as
    /// `discovery.js#settle` sends it.
    #[tokio::test]
    async fn the_recommendations_carry_the_level() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let rows = core.recommendations(0.4).await.expect("rows");
        assert_eq!(server.routes(), vec!["GET /api/recommend".to_owned()]);
        let hit = server.hits().pop().expect("the hit");
        assert_eq!(hit.query("d").as_deref(), Some("0.40"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Recommandé");
    }

    /// The settings and the check are admin routes: a member's `403` is the
    /// cause the card is hidden behind, never a lost screen.
    #[tokio::test]
    async fn the_release_settings_are_read_and_written_one_request_each() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let settings = core.release_settings().await.expect("the settings");
        assert_eq!(
            server.routes(),
            vec!["GET /api/releases/settings".to_owned()]
        );
        assert!(settings.on);
        assert_eq!(settings.every, 6);
        assert_eq!(settings.watched, 120);

        server.forget();
        core.save_release_settings(true, 12, 14)
            .await
            .expect("saved");
        assert_eq!(
            server.routes(),
            vec![
                "PUT /api/releases/settings".to_owned(),
                "GET /api/releases/settings".to_owned()
            ],
            "the write, then the server's own clamped answer"
        );

        server.forget();
        core.check_releases().await.expect("checked");
        assert_eq!(
            server.routes(),
            vec![
                "POST /api/releases/check".to_owned(),
                "GET /api/releases/settings".to_owned()
            ]
        );
    }
}
