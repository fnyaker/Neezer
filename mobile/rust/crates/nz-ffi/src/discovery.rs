//! Discovery: Nouveautés, the release shelves, the Récap, moods, Explorer, the
//! discovery slider's home, the genre stations and `/recommend`.
//!
//! Owned by **MB14** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this file — its own `#[uniffi::export] impl` blocks, its own
//! records, its own listener — and edits no other file of the crate.
//!
//! # The shape of this boundary
//!
//! - **`cached_x` is synchronous and disk-only**, `x` is the network read on the
//!   `User` lane: the screen paints the first, replaces it with the second, and
//!   keeps it with « Hors ligne · version mémorisée » when the second fails.
//! - **A long list never crosses whole.** A shelf's titles, a release group's
//!   titles, a station's tracks and a year's top are handed as a `ListHandle`
//!   the screen windows: at most 128 rows per call, keyed by the row's own key.
//! - **The Récap is not a file.** Its years carry the SPA's ten minutes, and a
//!   year lives until the process dies: personal listening history is never
//!   written to the device (`lib/recapyears.js`, `lib/apicache.js`), so a cold
//!   start says « Ton récap a besoin du serveur » with « Réessayer ».
//! - **No listener of its own.** Nothing here changes behind the person's back:
//!   the discovery level's changes are MB5's (`home`) and a station's queue is
//!   the player's `PlayerSnapshot` (MB3b).
//!
//! Every body converts to `nz_core::discovery` and back; no rule lives in this
//! file.

use std::sync::Arc;

use nz_core::discovery as core;

use crate::core::Screens;
use crate::errors::NzError;
use crate::lists::{ListHandle, TrackRow, open_rows};

/// One release card, as Nouveautés and the per-release list draw it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ReleaseRowData {
    pub key: String,
    pub title: String,
    pub cover: Option<String>,
    pub artist: String,
    pub artist_id: Option<String>,
    pub kind: String,
    /// "Album", "Single", "EP", "Compilation" or "Sortie".
    pub kind_label: String,
    /// "12 nov.", or `""`.
    pub date_label: String,
    pub nb_tracks: u32,
    pub close_to: Option<String>,
    /// "Ascenseur · Single · 12 nov." — one line, already joined.
    pub subtitle: String,
    /// Arrived with the latest check: the "Nouveau" badge.
    pub new: bool,
    /// How many of its titles the feed carries.
    pub track_count: u32,
}

impl From<core::ReleaseRow> for ReleaseRowData {
    fn from(row: core::ReleaseRow) -> Self {
        ReleaseRowData {
            key: row.key,
            title: row.title,
            cover: row.cover,
            artist: row.artist,
            artist_id: row.artist_id,
            kind: row.kind,
            kind_label: row.kind_label,
            date_label: row.date_label,
            nb_tracks: row.nb_tracks,
            close_to: row.close_to,
            subtitle: row.subtitle,
            new: row.new,
            track_count: row.track_count,
        }
    }
}

/// One shelf of Nouveautés, as its card draws it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ReleaseShelfData {
    /// `artists` or `discovery`.
    pub key: String,
    pub title: String,
    pub subtitle: String,
    pub track_count: u32,
    /// How many releases hold those titles ("2 titres · 1 sortie").
    pub release_count: u32,
    pub duration_s: u64,
    /// Up to four sleeves for the card's mosaic.
    pub mosaic: Vec<String>,
    pub close_names: Vec<String>,
    /// The six rows the card previews, already in the shelf's order.
    pub preview: Vec<TrackRow>,
}

impl From<&core::ReleaseShelfRow> for ReleaseShelfData {
    fn from(shelf: &core::ReleaseShelfRow) -> Self {
        ReleaseShelfData {
            key: shelf.key.clone(),
            title: shelf.title.clone(),
            subtitle: shelf.subtitle.clone(),
            track_count: shelf.track_count,
            release_count: shelf.releases.len() as u32,
            duration_s: shelf.duration_s,
            mosaic: shelf.mosaic.clone(),
            close_names: shelf.close_names.clone(),
            preview: shelf
                .preview
                .iter()
                .enumerate()
                .map(|(position, card)| TrackRow::from(card.to_row(position as u32)))
                .collect(),
        }
    }
}

/// One release of "Toutes les sorties": its card, its label and how many titles
/// it holds. The titles themselves are [`Screens::release_group_tracks`]'s
/// handle, so a group of forty never crosses as records.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ReleaseGroupData {
    pub release: ReleaseRowData,
    pub artist: String,
    /// "Album · 3 titres · Nouveau".
    pub label: String,
    pub track_count: u32,
}

/// Everything Nouveautés draws, in one answer.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ReleasesData {
    /// Every new title the feed carries.
    pub count: u32,
    /// The releases this visit can announce as new.
    pub unseen: u32,
    /// Releases the server is still reading.
    pub pending: u32,
    /// "2 nouvelles sorties", `""` when nothing is new.
    pub fresh_label: String,
    /// The sentence under "Toutes les sorties" while the server reads.
    pub pending_label: String,
    pub shelves: Vec<ReleaseShelfData>,
    pub groups: Vec<ReleaseGroupData>,
    /// The releases with no title yet, as plain cards.
    pub others: Vec<ReleaseRowData>,
    pub release_total: u32,
    /// How many titles "Tout lire" queues.
    pub track_count: u32,
    /// Nothing to draw at all: "Rien de neuf pour l'instant".
    pub empty: bool,
}

impl From<core::ReleasesData> for ReleasesData {
    fn from(data: core::ReleasesData) -> Self {
        // `empty` is read BEFORE any field moves: it is the screen's own
        // "Rien de neuf pour l'instant".
        let empty = data.is_empty();
        let fresh_label = data.fresh_label();
        let pending_label = data.pending_label();
        let track_count = data.tracks.len() as u32;
        let groups: Vec<ReleaseGroupData> = data
            .groups
            .iter()
            .map(|group| ReleaseGroupData {
                release: group.release.clone().into(),
                artist: group.artist.clone(),
                label: group.label.clone(),
                track_count: group.tracks.len() as u32,
            })
            .collect();
        ReleasesData {
            count: data.count,
            unseen: data.unseen,
            pending: data.pending,
            fresh_label,
            pending_label,
            shelves: data.shelves.iter().map(Into::into).collect(),
            groups,
            others: data.others.into_iter().map(Into::into).collect(),
            release_total: data.release_total,
            track_count,
            empty,
        }
    }
}

/// One shelf's page: its banner, its description and the handle over its
/// titles (`routes/ReleaseShelf.svelte`).
#[derive(uniffi::Record, Clone)]
pub struct ReleaseShelfPage {
    pub key: String,
    pub title: String,
    pub subtitle: String,
    pub description: String,
    /// "12 titres · 3 sorties".
    pub facts: String,
    pub total_duration_s: u64,
    pub mosaic: Vec<String>,
    pub releases: Vec<ReleaseRowData>,
    pub tracks: Arc<ListHandle>,
    /// "2 titres" — the empty state's own count.
    pub track_count: u32,
}

/// What `/api/releases/settings` says.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
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

impl From<core::ReleaseSettingsData> for ReleaseSettingsData {
    fn from(data: core::ReleaseSettingsData) -> Self {
        ReleaseSettingsData {
            on: data.on,
            every: data.every,
            window: data.window,
            close: data.close,
            listened: data.listened,
            watched: data.watched,
            favourites: data.favourites,
            links: data.links,
            asked: data.asked,
            releases: data.releases,
            running: data.running,
            last_run: data.last_run,
            last_error: data.last_error,
        }
    }
}

/// One year chip of the Récap's selector.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct RecapChipData {
    pub year: i32,
    /// "2026 · en cours" for the year the SERVER says is running.
    pub label: String,
}

/// The years a Récap can open (`lib/recapyears.js`).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct RecapYearsData {
    pub chips: Vec<RecapChipData>,
    pub latest: Option<i32>,
    pub now_year: i32,
    pub min_year: i32,
    pub has_any: bool,
}

impl From<core::RecapYearsData> for RecapYearsData {
    fn from(data: core::RecapYearsData) -> Self {
        RecapYearsData {
            has_any: !data.years.is_empty(),
            chips: data
                .chips
                .into_iter()
                .map(|chip| RecapChipData {
                    year: chip.year,
                    label: chip.label,
                })
                .collect(),
            latest: data.latest,
            now_year: data.now_year,
            min_year: data.min_year,
        }
    }
}

/// One row of a slide's top.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct RecapRowData {
    pub key: String,
    /// 1-based rank.
    pub rank: u32,
    pub title: String,
    /// The second line: listens, the artist, nothing when the card has none.
    pub subtitle: String,
    /// The right-hand mono line ("41 écoutes", "41 %").
    pub value: String,
    pub cover: Option<String>,
    /// The style's share, 0…1, for the genres card's bar.
    pub share: f64,
}

/// One bar of the minutes card.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct RecapBarData {
    pub month: u8,
    pub label: String,
    pub listens: u32,
    pub pct: u8,
}

/// One card of the story: the ID selects the layout, the words are the plan's.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct RecapSlideData {
    pub id: String,
    pub kicker: String,
    pub title: String,
    /// The glyph's name (`RecapSlide.svelte#ICONS`, through `Icon`): `sparkles`,
    /// `timer`, `user`, `music`, `disc`, `tag`, `activity`, `share`.
    pub glyph: String,
    pub lede: String,
    pub text: String,
    pub footnote: String,
    pub rows: Vec<RecapRowData>,
    pub bars: Vec<RecapBarData>,
    pub mosaic: Vec<String>,
    /// The six poster lines, on the share card.
    pub poster: Vec<String>,
}

impl From<core::RecapSlide> for RecapSlideData {
    fn from(slide: core::RecapSlide) -> Self {
        RecapSlideData {
            id: slide.id,
            kicker: slide.kicker,
            title: slide.title,
            glyph: slide.glyph,
            lede: slide.lede,
            text: slide.text,
            footnote: slide.footnote,
            rows: slide
                .rows
                .into_iter()
                .map(|row| RecapRowData {
                    key: row.key,
                    rank: row.rank,
                    title: row.title,
                    subtitle: row.subtitle,
                    value: row.value,
                    cover: row.cover,
                    share: row.share,
                })
                .collect(),
            bars: slide
                .bars
                .into_iter()
                .map(|bar| RecapBarData {
                    month: bar.month,
                    label: bar.label,
                    listens: bar.listens,
                    pct: bar.pct,
                })
                .collect(),
            mosaic: slide.mosaic,
            poster: slide.poster,
        }
    }
}

/// One year in review, assembled: the deck, the glance panel and the share.
#[derive(uniffi::Record, Clone)]
pub struct RecapData {
    pub year: i32,
    pub partial: bool,
    /// "du 1ᵉʳ janvier au 13 novembre 2026".
    pub period_label: String,
    pub minutes: u32,
    pub listens: u32,
    /// "8 540" / "2 180": the glance panel prints these, never the raw count.
    pub minutes_label: String,
    pub listens_label: String,
    pub slides: Vec<RecapSlideData>,
    pub poster_lines: Vec<String>,
    pub share_text: String,
    pub mosaic: Vec<String>,
    pub top_artist: String,
    pub top_genre: String,
    pub biggest_day: String,
    /// `none`, `skips-only`, `thin`, or `""` when the deck has cards.
    pub empty_reason: String,
    pub empty_title: String,
    pub empty_text: String,
    pub nearby_year: Option<i32>,
    /// The top titles: what "Lire ces 5 titres" queues.
    pub top_tracks: Arc<ListHandle>,
    /// `true` when the year has no story to tell.
    pub empty: bool,
}

impl RecapData {
    fn from_core(data: core::RecapData, core: &Arc<nz_core::core::Core>) -> RecapData {
        // A year with no story is the panel, never an empty deck: the reason
        // says which (the SPA's `isEmpty`, `Recap.svelte`).
        let empty = data.empty_reason.is_some() || data.slides.is_empty();
        let top_tracks: Vec<nz_core::lists::Row> = data
            .top_tracks
            .iter()
            .enumerate()
            .map(|(position, card)| card.to_row(position as u32))
            .collect();
        RecapData {
            year: data.year,
            partial: data.partial,
            period_label: data.period_label,
            minutes: data.minutes,
            listens: data.listens,
            minutes_label: data.minutes_label,
            listens_label: data.listens_label,
            slides: data.slides.into_iter().map(Into::into).collect(),
            poster_lines: data.poster_lines,
            share_text: data.share_text,
            mosaic: data.mosaic,
            top_artist: data.top_artist,
            top_genre: data.top_genre,
            biggest_day: data.biggest_day,
            empty_reason: data
                .empty_reason
                .map(|reason| reason.code().to_owned())
                .unwrap_or_default(),
            empty_title: data.empty_title,
            empty_text: data.empty_text,
            nearby_year: data.nearby_year,
            top_tracks: open_rows(core, top_tracks),
            empty,
        }
    }
}

/// One "Mix d'ambiance" card (`/api/moods`).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct MoodRowData {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    /// A COUNT here: the titles come from [`Screens::discovery_mood_tracks`].
    pub tracks: u32,
    pub ready: bool,
    pub cover: Option<String>,
}

impl From<core::MoodRow> for MoodRowData {
    fn from(row: core::MoodRow) -> Self {
        MoodRowData {
            id: row.id,
            name: row.name,
            subtitle: row.subtitle,
            tracks: row.tracks,
            ready: row.ready,
            cover: row.cover,
        }
    }
}

/// One mix, when its card is tapped.
#[derive(uniffi::Record, Clone)]
pub struct MoodTracksData {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub ready: bool,
    /// `not-enough` or `no-tracks`, when the mix is empty.
    pub reason: String,
    pub tracks: Arc<ListHandle>,
    pub track_count: u32,
}

/// One tile of "Parcourir par style".
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct StyleTileData {
    pub name: String,
    /// "12 titres" — the caption's own words.
    pub count_label: String,
    pub tracks: u32,
    /// Its bar's width in percent of the biggest tile.
    pub bar_pct: u32,
}

/// Explorer's shelf: the ten parent styles, and whose taste they are.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ExplorerStylesData {
    pub tiles: Vec<StyleTileData>,
    /// `history`, `unanalysed` or `none`: an empty shelf says which.
    pub source: String,
    pub tracks: u32,
    /// Nothing to draw: the empty sentence names the `source`.
    pub empty: bool,
}

impl From<core::ExplorerStylesData> for ExplorerStylesData {
    fn from(data: core::ExplorerStylesData) -> Self {
        let max = data.max_tracks.max(1);
        ExplorerStylesData {
            empty: data.tiles.is_empty(),
            tiles: data
                .tiles
                .into_iter()
                .map(|tile| StyleTileData {
                    count_label: core::count_label(tile.tracks),
                    bar_pct: ((u64::from(tile.tracks) * 100) / u64::from(max)) as u32,
                    name: tile.name,
                    tracks: tile.tracks,
                })
                .collect(),
            source: data.source,
            tracks: data.tracks,
        }
    }
}

/// One playlist of a finished discovery.
#[derive(uniffi::Record, Clone)]
pub struct ExplorerPlaylistData {
    pub id: String,
    pub title: String,
    pub cover: Option<String>,
    /// "12 titres vérifiés sur 60".
    pub count_label: String,
    pub tracks: Arc<ListHandle>,
    pub track_count: u32,
}

/// Where one style's discovery stands, and what it found.
#[derive(uniffi::Record, Clone)]
pub struct ExplorerDiscoverData {
    /// `idle`, `pending`, `ready`, `empty` or `busy`.
    pub phase: String,
    pub style: String,
    pub level: u32,
    pub playlists: Vec<ExplorerPlaylistData>,
    /// The status that made the server busy (`429`/`503`), `0` otherwise.
    pub busy_status: u16,
    pub retry_after_s: u32,
    /// "Trop de découvertes demandées pour l'instant." / "Le serveur est occupé."
    pub busy_cause: String,
}

/// One row of "Toutes les stations".
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct StationRowData {
    pub name: String,
    pub parent: Option<String>,
    /// Titles under it, this node included.
    pub branch: u32,
    /// "12 titres".
    pub count_label: String,
}

impl From<core::StationRow> for StationRowData {
    fn from(row: core::StationRow) -> Self {
        StationRowData {
            count_label: core::count_label(row.branch),
            name: row.name,
            parent: row.parent,
            branch: row.branch,
        }
    }
}

/// The five breadth notches, as the sheet names them.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ExplorerLevelData {
    pub level: u32,
    pub name: String,
    pub hint: String,
}

/// A started station: its context and the titles to play NOW.
#[derive(uniffi::Record, Clone)]
pub struct StationData {
    /// Always `genre-radio`.
    pub kind: String,
    /// The server's canonical node, which a refill takes back.
    pub id: String,
    /// "Station Techno".
    pub title: String,
    pub tracks: Arc<ListHandle>,
    pub track_count: u32,
}

/// What `Screens::discovery_settings` answers: the slider's five notches, and
/// nothing that would be a second copy of MB5's level.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ExplorerSettingsData {
    pub levels: Vec<ExplorerLevelData>,
    /// `ExplorerStyles.svelte#MAX_POLLS`: two minutes of polling.
    pub max_polls: u32,
    /// `ExplorerStyles.svelte#POLL_MS`.
    pub poll_ms: u64,
}

/// The rows a screen opens as a handle, written once here.
fn rows_of(
    cards: Vec<nz_core::home::TrackCard>,
    core: &Arc<nz_core::core::Core>,
) -> Arc<ListHandle> {
    let rows: Vec<nz_core::lists::Row> = cards
        .iter()
        .enumerate()
        .map(|(position, card)| card.to_row(position as u32))
        .collect();
    open_rows(core, rows)
}

#[uniffi::export(async_runtime = "tokio")]
impl Screens {
    // -- Nouveautés ----------------------------------------------------------

    /// The feed as this device last saw it: disk only, no request. `None` the
    /// first time, when the screen shows its skeleton.
    pub fn cached_releases(&self) -> Option<ReleasesData> {
        self.core().cached_releases().map(Into::into)
    }

    /// `GET /releases`, then `POST /releases/seen` when the badge was unseen —
    /// the SPA's own two calls. Fails fast offline (`NzError::Offline`), which
    /// is what « Hors ligne · version mémorisée » is read from.
    pub async fn releases(&self) -> Result<ReleasesData, NzError> {
        self.core()
            .releases()
            .await
            .map(Into::into)
            .map_err(NzError::from)
    }

    /// `POST /releases/seen` — the badge goes away on the person's own action.
    pub async fn releases_seen(&self) -> Result<(), NzError> {
        self.core().releases_seen().await.map_err(NzError::from)
    }

    /// The last known settings, from the device.
    pub fn cached_release_settings(&self) -> Option<ReleaseSettingsData> {
        self.core().cached_release_settings().map(Into::into)
    }

    /// `GET /releases/settings` — admin only; a member's `403` is the cause the
    /// card is hidden behind (`/api/me`'s `user.admin`).
    pub async fn release_settings(&self) -> Result<ReleaseSettingsData, NzError> {
        self.core()
            .release_settings()
            .await
            .map(Into::into)
            .map_err(NzError::from)
    }

    /// `PUT /releases/settings` — the server clamps and answers the whole
    /// status, which is what the screen draws.
    pub async fn save_release_settings(
        &self,
        on: bool,
        every: u32,
        window: u32,
    ) -> Result<ReleaseSettingsData, NzError> {
        self.core()
            .save_release_settings(on, every, window)
            .await
            .map(Into::into)
            .map_err(NzError::from)
    }

    /// `POST /releases/check` — one request starts the server's own pass.
    pub async fn check_releases(&self) -> Result<ReleaseSettingsData, NzError> {
        self.core()
            .check_releases()
            .await
            .map(Into::into)
            .map_err(NzError::from)
    }

    /// One shelf's page, from the kept feed: NO request (a card click is
    /// instant, `ReleaseShelf.svelte`'s module-scoped feed).
    pub fn release_shelf(&self, kind: String) -> Option<ReleaseShelfPage> {
        let detail = self.core().release_shelf(&kind)?;
        let handle = rows_of(detail.tracks.clone(), self.core());
        Some(ReleaseShelfPage {
            key: detail.key,
            title: detail.title,
            subtitle: detail.subtitle,
            description: detail.description,
            facts: detail.facts,
            total_duration_s: detail.total_duration_s,
            mosaic: detail.mosaic,
            releases: detail.releases.into_iter().map(Into::into).collect(),
            track_count: detail.tracks.len() as u32,
            tracks: handle,
        })
    }

    /// One release's titles, from the kept feed: the group a person expands.
    pub fn release_group_tracks(&self, release_id: String) -> Arc<ListHandle> {
        rows_of(self.core().release_group_tracks(&release_id), self.core())
    }

    /// Every new title, from the kept feed: what "Tout lire" queues. Windowed
    /// like every long list, so a 400-title feed is not one record copy.
    pub fn releases_tracks(&self) -> Arc<ListHandle> {
        let cards = self
            .core()
            .cached_releases()
            .map(|feed| feed.tracks)
            .unwrap_or_default();
        rows_of(cards, self.core())
    }

    // -- Récap ---------------------------------------------------------------

    /// The years this device already knows, without a request.
    pub fn cached_recap_years(&self) -> Option<RecapYearsData> {
        self.core().cached_recap_years().map(Into::into)
    }

    /// `GET /recap/years` — asked once per ten minutes (`recapyears.js`).
    pub async fn recap_years(&self) -> Result<RecapYearsData, NzError> {
        self.core()
            .recap_years()
            .await
            .map(Into::into)
            .map_err(NzError::from)
    }

    /// The year this device already assembled, in memory only.
    pub fn cached_recap(&self, year: i32) -> Option<RecapData> {
        self.core()
            .cached_recap(year)
            .map(|data| RecapData::from_core(data, self.core()))
    }

    /// The year's top titles, from the year already on the device: what
    /// « Lire ces 5 titres » queues, windowed like every long list.
    pub fn recap_top_tracks(&self, year: i32) -> Arc<ListHandle> {
        let cards = self
            .core()
            .cached_recap(year)
            .map(|data| data.top_tracks)
            .unwrap_or_default();
        rows_of(cards, self.core())
    }

    /// `GET /recap/<year>` — one bounded payload; the nine cards cost nothing.
    /// A year that is not four digits never reaches the server.
    pub async fn recap(&self, year: i32) -> Result<RecapData, NzError> {
        let data = self.core().recap(year).await.map_err(NzError::from)?;
        Ok(RecapData::from_core(data, self.core()))
    }

    // -- moods ---------------------------------------------------------------

    /// The mix catalog as this device last saw it.
    pub fn cached_moods(&self) -> Option<Vec<MoodRowData>> {
        self.core()
            .cached_moods()
            .map(|rows| rows.into_iter().map(Into::into).collect())
    }

    /// `GET /moods` — the catalog, a COUNT per card.
    pub async fn discovery_moods(&self) -> Result<Vec<MoodRowData>, NzError> {
        self.core()
            .moods()
            .await
            .map(|rows| rows.into_iter().map(Into::into).collect())
            .map_err(NzError::from)
    }

    /// `GET /moods/<id>` — ONE request per tap; the titles come windowed.
    pub async fn discovery_mood_tracks(&self, id: String) -> Result<MoodTracksData, NzError> {
        let mix = self.core().mood_mix(&id).await.map_err(NzError::from)?;
        let handle = rows_of(mix.tracks.clone(), self.core());
        Ok(MoodTracksData {
            id: mix.id,
            name: mix.name,
            subtitle: mix.subtitle,
            ready: mix.ready,
            reason: mix.reason,
            track_count: mix.tracks.len() as u32,
            tracks: handle,
        })
    }

    // -- Explorer ------------------------------------------------------------

    /// The Explorer styles this device already has.
    pub fn cached_explorer_styles(&self) -> Option<ExplorerStylesData> {
        self.core().cached_explorer_styles().map(Into::into)
    }

    /// `GET /explorer/styles` — the ten parent styles this person listens to; a
    /// server without the route is older than the feature
    /// (`NzError::Unsupported`, and the entry hides).
    pub async fn explorer_styles(&self) -> Result<ExplorerStylesData, NzError> {
        self.core()
            .explorer_styles()
            .await
            .map(Into::into)
            .map_err(NzError::from)
    }

    /// `POST /explorer/discover` — ONE sweep started by « Découvrir ». A busy
    /// server answers `busy` with its own wait, never an error.
    pub async fn explorer_discover(
        &self,
        style: String,
        level: u32,
    ) -> Result<ExplorerDiscoverData, NzError> {
        let data = self
            .core()
            .explorer_discover(&style, level)
            .await
            .map_err(NzError::from)?;
        let core = self.core();
        Ok(ExplorerDiscoverData {
            phase: data.phase_code().to_owned(),
            busy_cause: data.busy_cause(),
            playlists: data
                .playlists
                .iter()
                .map(|playlist| ExplorerPlaylistData {
                    id: playlist.id.clone(),
                    title: playlist.title.clone(),
                    cover: playlist.cover.clone(),
                    count_label: playlist.count_label.clone(),
                    track_count: playlist.tracks.len() as u32,
                    tracks: rows_of(playlist.tracks.clone(), core),
                })
                .collect(),
            style: data.style,
            level: data.level,
            busy_status: data.busy_status,
            retry_after_s: data.retry_after_s,
        })
    }

    /// `GET /explorer/discover?style=&level=` — the poll. It never starts work.
    pub async fn explorer_poll(
        &self,
        style: String,
        level: u32,
    ) -> Result<ExplorerDiscoverData, NzError> {
        let data = self
            .core()
            .explorer_poll(&style, level)
            .await
            .map_err(NzError::from)?;
        let core = self.core();
        Ok(ExplorerDiscoverData {
            phase: data.phase_code().to_owned(),
            busy_cause: data.busy_cause(),
            playlists: data
                .playlists
                .iter()
                .map(|playlist| ExplorerPlaylistData {
                    id: playlist.id.clone(),
                    title: playlist.title.clone(),
                    cover: playlist.cover.clone(),
                    count_label: playlist.count_label.clone(),
                    track_count: playlist.tracks.len() as u32,
                    tracks: rows_of(playlist.tracks.clone(), core),
                })
                .collect(),
            style: data.style,
            level: data.level,
            busy_status: data.busy_status,
            retry_after_s: data.retry_after_s,
        })
    }

    /// The five breadth notches, and the poll's two numbers.
    pub fn explorer_settings(&self) -> ExplorerSettingsData {
        ExplorerSettingsData {
            levels: (1..=core::LEVELS)
                .map(|level| {
                    let notch = core::level(level);
                    ExplorerLevelData {
                        level: notch.level,
                        name: notch.name.to_owned(),
                        hint: notch.hint.to_owned(),
                    }
                })
                .collect(),
            max_polls: core::MAX_POLLS,
            poll_ms: core::POLL_MS,
        }
    }

    /// The all-stations list this device already has.
    pub fn cached_station_styles(&self) -> Option<Vec<StationRowData>> {
        self.core()
            .cached_station_styles()
            .map(|rows| rows.into_iter().map(Into::into).collect())
    }

    /// `GET /flow/styles` — every analysed genre, the all-stations sheet's list
    /// (and MB5's Flow vocabulary in one answer).
    pub async fn station_styles(&self) -> Result<Vec<StationRowData>, NzError> {
        self.core()
            .station_styles()
            .await
            .map(|rows| rows.into_iter().map(Into::into).collect())
            .map_err(NzError::from)
    }

    /// `GET /radio/genre/<style>` — one station, playing at once.
    pub async fn genre_station(&self, style: String) -> Result<StationData, NzError> {
        let station = self
            .core()
            .genre_station(&style)
            .await
            .map_err(NzError::from)?;
        let handle = rows_of(station.tracks.clone(), self.core());
        Ok(StationData {
            kind: station.kind,
            id: station.id,
            title: station.title,
            track_count: station.tracks.len() as u32,
            tracks: handle,
        })
    }

    // -- what the person can do here -----------------------------------------

    /// `POST /discover/mute` — never propose this artist again.
    pub async fn mute_artist(&self, provider: String, id: String) -> Result<(), NzError> {
        self.core()
            .mute_artist(&provider, &id)
            .await
            .map_err(NzError::from)
    }

    /// `GET /recommend?d=` — what the discovery level proposes now, windowed.
    pub async fn recommendations(&self, level: f64) -> Result<Arc<ListHandle>, NzError> {
        let rows = self
            .core()
            .recommendations(level)
            .await
            .map_err(NzError::from)?;
        Ok(rows_of(rows, self.core()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A feed crosses as SCREEN-shaped records: no server JSON, the counts the
    /// labels need, and the badge this visit can announce.
    ///
    /// The fixture is built field by field on purpose: `nz-ffi` carries neither
    /// `serde_json` nor `nz-model` (MB16's rule), so a test here proves the
    /// CONVERSION, and the wire shapes are `nz-core`'s own tests' business.
    #[test]
    fn a_feed_crosses_as_screen_shaped_records() {
        let card = nz_core::home::TrackCard {
            key: "9001".to_owned(),
            id: "9001".to_owned(),
            title: "Ouverture".to_owned(),
            artists: "Ascenseur".to_owned(),
            album: "Le matin".to_owned(),
            album_id: Some("501".to_owned()),
            duration_s: 180,
            cover: Some("/api/cover/501".to_owned()),
            source: "deezer".to_owned(),
            release: Some("501".to_owned()),
            ..nz_core::home::TrackCard::default()
        };
        let release = core::ReleaseRow {
            key: "501".to_owned(),
            title: "Le matin".to_owned(),
            artist: "Ascenseur".to_owned(),
            kind: "album".to_owned(),
            kind_label: "Album".to_owned(),
            date_label: "12 nov.".to_owned(),
            new: true,
            track_count: 1,
            ..core::ReleaseRow::default()
        };
        let shelf = core::ReleaseShelfRow {
            key: "artists".to_owned(),
            title: "Tes artistes".to_owned(),
            subtitle: "Les sorties de tes favoris.".to_owned(),
            track_count: 1,
            releases: vec![release.clone()],
            ..core::ReleaseShelfRow::default()
        };
        let group = core::ReleaseGroup {
            release,
            artist: "Ascenseur".to_owned(),
            tracks: vec![card.clone()],
            label: "Album · 1 titre · Nouveau".to_owned(),
        };
        let data = core::ReleasesData {
            count: 1,
            unseen: 2,
            pending: 1,
            shelves: vec![shelf],
            groups: vec![group],
            release_total: 1,
            tracks: vec![card],
            ..core::ReleasesData::default()
        };
        let crossed = ReleasesData::from(data);
        assert_eq!(crossed.count, 1);
        assert_eq!(crossed.unseen, 2);
        assert_eq!(crossed.fresh_label, "2 nouvelles sorties");
        assert_eq!(crossed.shelves.len(), 1);
        assert_eq!(crossed.shelves[0].key, "artists");
        assert_eq!(crossed.groups.len(), 1);
        assert_eq!(crossed.groups[0].label, "Album · 1 titre · Nouveau");
        assert_eq!(crossed.track_count, 1);
        assert!(!crossed.empty);
    }

    /// Every variant of the recap's empty reason has a stable code, and `""` is
    /// "the deck has cards".
    #[test]
    fn an_empty_year_says_why_in_a_stable_code() {
        assert_eq!(core::EmptyReason::SkipsOnly.code(), "skips-only");
        assert_eq!(core::EmptyReason::Thin.code(), "thin");
        assert_eq!(core::EmptyReason::None.code(), "none");
    }

    /// The five notches cross with their own words, and the poll's numbers are
    /// the component's own.
    #[test]
    fn the_five_breadths_cross_with_their_words() {
        let levels: Vec<ExplorerLevelData> = (1..=core::LEVELS)
            .map(|level| {
                let notch = core::level(level);
                ExplorerLevelData {
                    level: notch.level,
                    name: notch.name.to_owned(),
                    hint: notch.hint.to_owned(),
                }
            })
            .collect();
        assert_eq!(levels.len(), 5);
        assert_eq!(levels[0].name, "Prudent");
        assert_eq!(levels[4].name, "Large");
        assert_eq!(core::MAX_POLLS, 60);
        assert_eq!(core::POLL_MS, 2_000);
    }

    /// A station row's caption is the sheet's own "12 titres".
    #[test]
    fn a_station_row_carries_its_caption() {
        let row = StationRowData::from(core::StationRow {
            name: "Techno".to_owned(),
            parent: None,
            branch: 12,
            tracks: 10,
        });
        assert_eq!(row.count_label, "12 titres");
    }
}
