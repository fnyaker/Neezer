//! The library: playlists and their folders, favourites, followed artists, kept
//! albums, uploads (and the quota), dislikes, unavailable titles, the listening
//! history and the person's own files.
//!
//! Owned by **MB8** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this file — its own `#[uniffi::export] impl` blocks, its own
//! records — and edits no other file of the crate.
//!
//! # The shape of this boundary
//!
//! - **`cached_x` then `x`** for every leaf, the pair the screen standard names:
//!   the copy is disk-only and synchronous (the ViewModel runs it off the main
//!   thread), the read is the SPA's route on the `User` lane, counted under the
//!   `library` tag. An 8 000-title favourites list therefore crosses as ONE
//!   `ListHandle`, never as 8 000 records.
//! - **A long list is a `ListHandle`** (`favorites`, `local`, `dislikes`,
//!   `unavailable`): the rows stay in Rust, the screen asks the page it is about
//!   to paint, and MB10's like/dislike patches every open list in the same frame
//!   through the registry.
//! - **A bounded list is a `Vec`**: artists, albums, the sections of the
//!   playlist tree and one page of the history are what their screens draw at
//!   once (the server itself caps `/me/local` and the history is 50 a page).
//! - **The folder writes answer the WHOLE tree**, after the change: the write is
//!   optimistic (the tree moves first, one request carries the complete order)
//!   and a refusal is an `NzError` the screen turns into a toast with
//!   « Réessayer » while it keeps the tree it drew.
//! - **No listener of its own.** Nothing here pushes: the favourites and the
//!   folders change because a screen asked, and a like/dislike is MB10's event.
//!
//! Every body converts to `nz_core::library` and back; no rule lives in this
//! file.

use std::sync::Arc;

use nz_core::library as core;

use crate::core::Screens;
use crate::errors::NzError;
use crate::lists::{ListHandle, TrackRow};

/// One folder of the playlist library.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct FolderRow {
    pub id: String,
    pub title: String,
    /// The server's stored position; a local reorder renumbers it 0..n-1.
    pub position: i64,
    /// How many playlists it holds — the section header's « 3 playlists ».
    pub count: u32,
}

impl From<core::FolderCard> for FolderRow {
    fn from(folder: core::FolderCard) -> Self {
        FolderRow {
            id: folder.id,
            title: folder.title,
            position: folder.position,
            count: folder.count,
        }
    }
}

/// One playlist card (`Card.svelte`, kind `playlist`).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistRow {
    pub id: String,
    /// The provider's id, when the list came from Deezer.
    pub deezer_id: Option<String>,
    pub title: String,
    pub cover: Option<String>,
    pub tracks: u32,
    /// Somebody else's role than `owner` (`isSharedPlaylist`).
    pub shared: bool,
    pub role: String,
    pub owner: String,
    pub editable: bool,
    pub can_manage: bool,
    /// `None` at « Sans dossier ».
    pub folder_id: Option<String>,
    pub position: Option<i64>,
}

impl From<core::PlaylistCard> for PlaylistRow {
    fn from(playlist: core::PlaylistCard) -> Self {
        PlaylistRow {
            id: playlist.id,
            deezer_id: playlist.deezer_id,
            title: playlist.title,
            cover: playlist.cover,
            tracks: playlist.tracks,
            shared: playlist.shared,
            role: playlist.role,
            owner: playlist.owner,
            editable: playlist.editable,
            can_manage: playlist.can_manage,
            folder_id: playlist.folder_id,
            position: playlist.position,
        }
    }
}

/// Which of the four kinds a section is (`Library.svelte#buildSections`).
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibrarySectionKind {
    /// A folder, in the person's own order.
    Folder,
    /// « Sans dossier ».
    Root,
    /// « Partagées avec moi ».
    Shared,
    /// « Depuis Deezer ».
    Imported,
}

impl From<core::SectionKind> for LibrarySectionKind {
    fn from(kind: core::SectionKind) -> Self {
        match kind {
            core::SectionKind::Folder => LibrarySectionKind::Folder,
            core::SectionKind::Root => LibrarySectionKind::Root,
            core::SectionKind::Shared => LibrarySectionKind::Shared,
            core::SectionKind::Imported => LibrarySectionKind::Imported,
        }
    }
}

/// One section of the « Playlists » leaf.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct LibrarySection {
    pub kind: LibrarySectionKind,
    /// `folder:<id>`, `root`, `shared`, `imported` — the UI's key.
    pub key: String,
    pub title: String,
    /// The number the header states (a folder's own count).
    pub count: u32,
    pub rows: Vec<PlaylistRow>,
}

/// Everything the « Playlists » leaf draws: the folders, the playlists, and the
/// sections both make.
///
/// `sections` is derived from the two lists on every change (`groupPlaylists` +
/// `buildSections`, ported in `nz-core`), so the screen never groups a tree
/// itself — and a section can never disagree with the rows it shows.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct LibraryTree {
    pub folders: Vec<FolderRow>,
    pub playlists: Vec<PlaylistRow>,
    pub sections: Vec<LibrarySection>,
}

impl From<core::LibraryTree> for LibraryTree {
    fn from(tree: core::LibraryTree) -> Self {
        LibraryTree {
            folders: tree.folders.into_iter().map(FolderRow::from).collect(),
            playlists: tree.playlists.into_iter().map(PlaylistRow::from).collect(),
            sections: tree
                .sections
                .into_iter()
                .map(|section| LibrarySection {
                    kind: LibrarySectionKind::from(section.kind),
                    key: section.key,
                    title: section.title,
                    count: section.count,
                    rows: section.rows.into_iter().map(PlaylistRow::from).collect(),
                })
                .collect(),
        }
    }
}

/// One followed artist, as the « Artistes » grid draws it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct LibraryArtist {
    pub id: String,
    pub name: String,
    pub picture: Option<String>,
    pub tracks: u32,
    pub plays: u32,
    /// When it was followed (ISO date-time) — « Ajoutés récemment » sorts by it.
    pub starred: String,
}

impl From<core::ArtistRow> for LibraryArtist {
    fn from(artist: core::ArtistRow) -> Self {
        LibraryArtist {
            id: artist.id,
            name: artist.name,
            picture: artist.picture,
            tracks: artist.tracks,
            plays: artist.plays,
            starred: artist.starred,
        }
    }
}

/// One kept album, as its grid card draws it.
///
/// Named `LibraryAlbum` (not `AlbumCardData`) because MB7's entity file already
/// owns that record name in this crate, and UniFFI's names are crate-wide.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct LibraryAlbum {
    pub id: String,
    pub title: String,
    pub cover: Option<String>,
    pub artist: String,
    pub year: String,
    pub tracks: u32,
}

impl From<core::AlbumCard> for LibraryAlbum {
    fn from(album: core::AlbumCard) -> Self {
        LibraryAlbum {
            id: album.id,
            title: album.title,
            cover: album.cover,
            artist: album.artist,
            year: album.year,
            tracks: album.tracks,
        }
    }
}

/// `/api/upload/usage` — the quota bar.
#[derive(uniffi::Record, Debug, Clone, Copy, PartialEq, Default)]
pub struct UploadUsageData {
    pub used: u64,
    pub quota: u64,
    pub unlimited: bool,
    /// How full the bar is, 0…100 (`usagePct` computes it in the SPA).
    pub percent: f64,
}

impl From<core::UploadUsageRow> for UploadUsageData {
    fn from(usage: core::UploadUsageRow) -> Self {
        UploadUsageData {
            used: usage.used,
            quota: usage.quota,
            unlimited: usage.unlimited,
            percent: usage.percent(),
        }
    }
}

/// `/api/me/favorite-ids` — the ids only: every heart, and the car's counter.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct FavoriteIdsData {
    pub tracks: Vec<String>,
    pub albums: Vec<String>,
    pub artists: Vec<String>,
    pub playlists: Vec<String>,
    /// A dislike is NOT a star.
    pub disliked: Vec<String>,
}

impl From<core::FavoriteIdSet> for FavoriteIdsData {
    fn from(ids: core::FavoriteIdSet) -> Self {
        FavoriteIdsData {
            tracks: ids.tracks,
            albums: ids.albums,
            artists: ids.artists,
            playlists: ids.playlists,
            disliked: ids.disliked,
        }
    }
}

/// One event of the listening history.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct HistoryEventData {
    pub id: i64,
    /// The server's local ISO time; the UI groups the days itself, on the
    /// DEVICE's calendar ("Aujourd'hui", "Hier").
    pub at: String,
    pub listened: u32,
    /// `1` a listen, `-1` a skip, `0` neither.
    pub outcome: i32,
    pub source: Option<String>,
    pub track: TrackRow,
}

/// The keyset cursor of the next history page: pass BOTH halves back.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct HistoryCursorData {
    pub before: String,
    pub before_id: i64,
}

/// One artist of the 30-day summary.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct HistoryArtistData {
    pub id: String,
    pub name: String,
    pub cover: Option<String>,
    pub count: u32,
}

/// One title of the 30-day summary.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct HistoryTrackData {
    pub track: TrackRow,
    pub count: u32,
}

/// The last 30 days, in figures.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct HistorySummaryData {
    pub days: u32,
    pub listens: u32,
    pub skips: u32,
    pub minutes: u32,
    pub top_artists: Vec<HistoryArtistData>,
    pub top_tracks: Vec<HistoryTrackData>,
}

/// One page of the history, with the cursor of the next one.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct HistoryPageData {
    pub events: Vec<HistoryEventData>,
    pub next: Option<HistoryCursorData>,
    pub summary: HistorySummaryData,
}

impl From<core::HistoryPage> for HistoryPageData {
    fn from(page: core::HistoryPage) -> Self {
        HistoryPageData {
            events: page
                .events
                .into_iter()
                .map(|event| HistoryEventData {
                    id: event.id,
                    at: event.at,
                    listened: event.listened,
                    outcome: event.outcome,
                    source: event.source,
                    track: TrackRow::from(event.track),
                })
                .collect(),
            next: page.next.map(|cursor| HistoryCursorData {
                before: cursor.before,
                before_id: cursor.before_id,
            }),
            summary: HistorySummaryData {
                days: page.summary.days,
                listens: page.summary.listens,
                skips: page.summary.skips,
                minutes: page.summary.minutes,
                top_artists: page
                    .summary
                    .top_artists
                    .into_iter()
                    .map(|artist| HistoryArtistData {
                        id: artist.id,
                        name: artist.name,
                        cover: artist.cover,
                        count: artist.count,
                    })
                    .collect(),
                top_tracks: page
                    .summary
                    .top_tracks
                    .into_iter()
                    .map(|entry| HistoryTrackData {
                        track: TrackRow::from(entry.row),
                        count: entry.count,
                    })
                    .collect(),
            },
        }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Screens {
    // -- playlists and folders ------------------------------------------------

    /// The playlist tree this device kept, or `None` the first time — disk only,
    /// no request. This is what a warm open of « Playlists » paints.
    pub fn cached_library_playlists(&self) -> Option<LibraryTree> {
        self.core()
            .cached_library_playlists()
            .map(LibraryTree::from)
    }

    /// `GET /api/me/playlists` — ONE request: the folders and the playlists in
    /// the same answer (the SPA reads both from this route).
    pub async fn library_playlists(&self) -> Result<LibraryTree, NzError> {
        self.core()
            .library_playlists()
            .await
            .map(LibraryTree::from)
            .map_err(NzError::from)
    }

    /// Whether this server has the playlist-folder routes at all (rule 7). The
    /// screen hides « Nouveau dossier » and « Organiser » on `Some(false)` and
    /// hides nothing on a doubt (`None`).
    pub async fn folders_supported(&self) -> Option<bool> {
        self.core().folders_supported().await
    }

    /// `POST /api/playlist-folders` — a new folder. The tree moves FIRST and the
    /// answer is the whole tree; a refusal is the cause to show beside
    /// « Réessayer ».
    pub async fn create_folder(&self, title: String) -> Result<LibraryTree, NzError> {
        self.core()
            .create_folder(&title)
            .await
            .map(LibraryTree::from)
            .map_err(NzError::from)
    }

    /// `PATCH /api/playlist-folders/<id>` — rename one of the person's folders.
    pub async fn rename_folder(&self, id: String, title: String) -> Result<LibraryTree, NzError> {
        self.core()
            .rename_folder(&id, &title)
            .await
            .map(LibraryTree::from)
            .map_err(NzError::from)
    }

    /// `DELETE /api/playlist-folders/<id>` — the folder goes, its playlists move
    /// to « Sans dossier » (never deleted).
    pub async fn delete_folder(&self, id: String) -> Result<LibraryTree, NzError> {
        self.core()
            .delete_folder(&id)
            .await
            .map(LibraryTree::from)
            .map_err(NzError::from)
    }

    /// `PUT /api/playlist-folders/order` — the complete desired order.
    pub async fn reorder_folders(&self, ids: Vec<String>) -> Result<LibraryTree, NzError> {
        self.core()
            .reorder_folders(&ids)
            .await
            .map(LibraryTree::from)
            .map_err(NzError::from)
    }

    /// `PUT /api/playlist-folders/{<id>|root}/order` — the complete desired order
    /// of ONE container (`folder: None` is « Sans dossier »).
    pub async fn reorder_container(
        &self,
        folder: Option<String>,
        ids: Vec<String>,
    ) -> Result<LibraryTree, NzError> {
        self.core()
            .reorder_container(folder.as_deref(), &ids)
            .await
            .map(LibraryTree::from)
            .map_err(NzError::from)
    }

    /// `actions.js#movePlaylistTo`: the playlist goes to the END of its new
    /// folder, and the ONE request is the destination's order. Offline it is
    /// refused BEFORE the move — the tree the screen drew stays, with
    /// « Réessayer ».
    pub async fn move_playlist_to(
        &self,
        playlist_id: String,
        folder: Option<String>,
    ) -> Result<LibraryTree, NzError> {
        self.core()
            .move_playlist_to(&playlist_id, folder.as_deref())
            .await
            .map(LibraryTree::from)
            .map_err(NzError::from)
    }

    // -- favourites and the light ids -----------------------------------------

    /// The favourites this device kept: the device's own copy, no request.
    pub fn cached_favorites(&self) -> Option<Arc<ListHandle>> {
        let rows = self.core().cached_favorites()?;
        Some(crate::lists::open_rows(self.core(), rows))
    }

    /// `GET /api/me/favorites` — the whole list in ONE request, as a windowed
    /// handle: 8 000 rows never cross the boundary whole (rule 4).
    pub async fn favorites(&self) -> Result<Arc<ListHandle>, NzError> {
        let rows = self.core().favorites().await.map_err(NzError::from)?;
        Ok(crate::lists::open_rows(self.core(), rows))
    }

    /// The starred ids from the device's copy, when there is one.
    pub fn cached_favorite_ids(&self) -> Option<FavoriteIdsData> {
        self.core().cached_favorite_ids().map(FavoriteIdsData::from)
    }

    /// `GET /api/me/favorite-ids` — four id arrays, no titles: the cheap route
    /// the hearts and the car'compteur read.
    pub async fn favorite_ids(&self) -> Result<FavoriteIdsData, NzError> {
        self.core()
            .favorite_ids()
            .await
            .map(FavoriteIdsData::from)
            .map_err(NzError::from)
    }

    // -- the leaves -----------------------------------------------------------

    /// The followed artists this device kept.
    pub fn cached_artists(&self) -> Option<Vec<LibraryArtist>> {
        self.core()
            .cached_artists()
            .map(|rows| rows.into_iter().map(LibraryArtist::from).collect())
    }

    /// `GET /api/me/artists` — the followed artists, ONE request.
    pub async fn artists(&self) -> Result<Vec<LibraryArtist>, NzError> {
        self.core()
            .artists()
            .await
            .map(|rows| rows.into_iter().map(LibraryArtist::from).collect())
            .map_err(NzError::from)
    }

    /// The kept albums this device kept.
    pub fn cached_albums(&self) -> Option<Vec<LibraryAlbum>> {
        self.core()
            .cached_albums()
            .map(|rows| rows.into_iter().map(LibraryAlbum::from).collect())
    }

    /// `GET /api/me/albums` — the kept albums, ONE request.
    pub async fn albums(&self) -> Result<Vec<LibraryAlbum>, NzError> {
        self.core()
            .albums()
            .await
            .map(|rows| rows.into_iter().map(LibraryAlbum::from).collect())
            .map_err(NzError::from)
    }

    /// The person's own files, from the device's copy.
    pub fn cached_local(&self) -> Option<Arc<ListHandle>> {
        let rows = self.core().cached_local()?;
        Some(crate::lists::open_rows(self.core(), rows))
    }

    /// `GET /api/me/local` — « Mes fichiers », ONE request, windowed.
    pub async fn local(&self) -> Result<Arc<ListHandle>, NzError> {
        let rows = self.core().local().await.map_err(NzError::from)?;
        Ok(crate::lists::open_rows(self.core(), rows))
    }

    /// The upload quota from the device's copy.
    pub fn cached_upload_usage(&self) -> Option<UploadUsageData> {
        self.core().cached_upload_usage().map(UploadUsageData::from)
    }

    /// `GET /api/upload/usage` — this person's uploads against their cap.
    pub async fn upload_usage(&self) -> Result<UploadUsageData, NzError> {
        self.core()
            .upload_usage()
            .await
            .map(UploadUsageData::from)
            .map_err(NzError::from)
    }

    /// The dislikes from the device's copy.
    pub fn cached_dislikes(&self) -> Option<Arc<ListHandle>> {
        let rows = self.core().cached_dislikes()?;
        Some(crate::lists::open_rows(self.core(), rows))
    }

    /// `GET /api/me/dislikes` — the titles the person said they do not like.
    pub async fn dislikes(&self) -> Result<Arc<ListHandle>, NzError> {
        let rows = self.core().dislikes().await.map_err(NzError::from)?;
        Ok(crate::lists::open_rows(self.core(), rows))
    }

    /// `POST /api/dislike` with `on: false` — « Retirer » on the dislikes list
    /// (`DislikedTracks.svelte#remove`). The screen drops the row first and puts
    /// it back with a toast when this fails.
    pub async fn remove_dislike(&self, reference: String) -> Result<(), NzError> {
        self.core()
            .remove_dislike(&reference)
            .await
            .map_err(NzError::from)
    }

    /// The unavailable rows from the device's copy.
    pub fn cached_unavailable(&self) -> Option<Arc<ListHandle>> {
        let rows = self.core().cached_unavailable()?;
        Some(crate::lists::open_rows(self.core(), rows))
    }

    /// `GET /api/unavailable` — the titles their provider no longer has, ONE
    /// request, asked when the leaf opens. A row's replace sheet is MB10's.
    pub async fn unavailable(&self) -> Result<Arc<ListHandle>, NzError> {
        let rows = self.core().unavailable().await.map_err(NzError::from)?;
        Ok(crate::lists::open_rows(self.core(), rows))
    }

    // -- history --------------------------------------------------------------

    /// The first page of the history for `outcome` (`""`: everything), from the
    /// device's copy.
    pub fn cached_history(&self, outcome: String) -> Option<HistoryPageData> {
        self.core()
            .cached_history(&outcome)
            .map(HistoryPageData::from)
    }

    /// `GET /api/history?…` — one page (50 by default at the call site), the
    /// 30-day summary included. `before` and `before_id` travel together, or not
    /// at all: a half cursor would restart the list.
    pub async fn history(
        &self,
        outcome: Option<String>,
        before: Option<String>,
        before_id: Option<i64>,
        limit: u32,
    ) -> Result<HistoryPageData, NzError> {
        self.core()
            .history(outcome.as_deref(), before.as_deref(), before_id, limit)
            .await
            .map(HistoryPageData::from)
            .map_err(NzError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_core::core::{Core, CoreConfig};

    fn core(tag: &str) -> (std::path::PathBuf, Arc<Core>) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-ffi-library-tests")
            .join(format!("{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let core = Arc::new(
            Core::new(CoreConfig {
                data_dir: path.join("data"),
                cache_dir: path.join("cache"),
                app_version: "0.0.0".into(),
                version_code: 2_000_001,
                sdk_int: 35,
                device_name: "test".into(),
            })
            .expect("a core"),
        );
        (path, core)
    }

    /// The tree crosses as the screen draws it: the folders with their badges,
    /// the playlists with the facts a card states, and the sections the grouping
    /// made — never the server's JSON.
    #[test]
    fn a_library_tree_crosses_as_screen_shaped_records() {
        let tree = core::LibraryTree::new(
            vec![core::FolderCard {
                id: "f1".to_owned(),
                title: "Voyage".to_owned(),
                position: 0,
                count: 1,
            }],
            vec![
                core::PlaylistCard {
                    id: "p1".to_owned(),
                    title: "Route".to_owned(),
                    folder_id: Some("f1".to_owned()),
                    tracks: 12,
                    role: "owner".to_owned(),
                    owner: "Nadia".to_owned(),
                    ..core::PlaylistCard::default()
                },
                core::PlaylistCard {
                    id: "p2".to_owned(),
                    deezer_id: Some("9080".to_owned()),
                    title: "Import".to_owned(),
                    role: "owner".to_owned(),
                    ..core::PlaylistCard::default()
                },
            ],
        );
        let crossed = LibraryTree::from(tree);
        assert_eq!(crossed.folders.len(), 1);
        assert_eq!(crossed.folders[0].count, 1);
        assert_eq!(crossed.playlists.len(), 2);
        assert_eq!(crossed.playlists[0].tracks, 12);
        assert_eq!(crossed.playlists[0].folder_id.as_deref(), Some("f1"));
        assert!(!crossed.playlists[0].shared, "the owner's own list");
        assert_eq!(
            crossed
                .sections
                .iter()
                .map(|section| section.key.as_str())
                .collect::<Vec<_>>(),
            ["folder:f1", "imported"]
        );
        assert_eq!(
            crossed.sections[1].kind,
            LibrarySectionKind::Imported,
            "a Deezer import gets its own shelf"
        );
        assert_eq!(
            crossed.sections[1].rows[0].deezer_id.as_deref(),
            Some("9080")
        );
    }

    /// A history page crosses with its cursor halves together and the track a
    /// tap plays.
    #[test]
    fn a_history_page_crosses_with_its_verdicts() {
        let page = core::HistoryPage {
            events: vec![core::HistoryEventRow {
                id: 42,
                at: "2026-11-17T21:04:00".to_owned(),
                listened: 187,
                outcome: 1,
                source: Some("album".to_owned()),
                track: nz_core::lists::Row::test_row(0, "Ouverture"),
            }],
            next: Some(core::HistoryCursorRow {
                before: "2026-11-17T21:04:00".to_owned(),
                before_id: 42,
            }),
            summary: core::HistorySummaryRow {
                days: 30,
                listens: 41,
                skips: 3,
                minutes: 220,
                top_artists: vec![core::TopArtist {
                    id: "42".to_owned(),
                    name: "Ascenseur".to_owned(),
                    cover: None,
                    count: 9,
                }],
                top_tracks: Vec::new(),
            },
        };
        let crossed = HistoryPageData::from(page);
        assert_eq!(crossed.events.len(), 1);
        assert_eq!(crossed.events[0].outcome, 1);
        assert_eq!(crossed.events[0].track.title, "Ouverture");
        assert_eq!(crossed.next.as_ref().map(|next| next.before_id), Some(42));
        assert_eq!(crossed.summary.listens, 41);
        assert_eq!(crossed.summary.top_artists[0].name, "Ascenseur");
    }

    /// The upload quota crosses with the percentage its bar draws — computed
    /// once, here, instead of by two screens.
    #[test]
    fn the_upload_quota_crosses_with_its_percentage() {
        let usage = UploadUsageData::from(core::UploadUsageRow {
            used: 1_073_741_824,
            quota: 5_368_709_120,
            unlimited: false,
        });
        assert_eq!(usage.used, 1_073_741_824);
        assert!((usage.percent - 20.0).abs() < 0.001);
        let unlimited = UploadUsageData::from(core::UploadUsageRow {
            used: 10,
            quota: 0,
            unlimited: true,
        });
        assert_eq!(
            unlimited.percent, 0.0,
            "no cap is an empty bar, not a full one"
        );
    }

    /// The favourites cross as ONE handle over 8 000 rows: the window hands out
    /// pages, and the list behind it is what MB10 patches.
    #[test]
    fn eight_thousand_favorites_cross_as_one_handle() {
        let (path, core) = core("favorites");
        let rows: Vec<nz_core::lists::Row> = (0..8_000)
            .map(|number| nz_core::lists::Row::test_row(number, &format!("Titre {number}")))
            .collect();
        let handle = crate::lists::open_rows(&core, rows);
        assert_eq!(handle.len(), 8_000);
        assert_eq!(handle.window(0, 64).len(), 64);
        assert_eq!(handle.window(7_990, 64).len(), 10);
        // The rows are registered, so a like/dislike redraws this list in place.
        let before = handle.version();
        core.lists().update_rows("test:7", |row| row.liked = false);
        assert!(handle.version() > before);
        let _ = std::fs::remove_dir_all(path);
    }
}
