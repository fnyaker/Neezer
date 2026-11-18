//! The library: playlists and their folders, favourites, followed artists,
//! kept albums, uploads (and the quota), dislikes, unavailable titles, the
//! listening history and the person's own files. Owned by **MB8**
//! (docs/agents/mobile.md, "Module and file ownership"): the lot fills this
//! directory and edits no other file of the crate.
//!
//! The behaviour is the SPA's library, PORTED — not re-invented. What that
//! means, file by file and section by section:
//!
//! | This module | The SPA |
//! |---|---|
//! | [`Core::library_playlists`] | `routes/Library.svelte#onMount` → `userPlaylists()` |
//! | [`LibraryTree::sections`] | `lib/playlistfolders.js#groupPlaylists` + `Library.svelte#buildSections` |
//! | [`Core::create_folder`] … [`Core::move_playlist_to`] | `lib/actions.js#createFolder`/`#renameFolder`/`#deleteFolder`/`#reorderFolders`/`#reorderContainer`/`#movePlaylistTo` |
//! | [`Core::favorites`] | `#onMount` → `loadMyFavorites()` |
//! | [`Core::favorite_ids`] | the light route the hearts and the car's counter read |
//! | [`Core::artists`], [`Core::albums`] | the two lazily-opened leaves |
//! | [`Core::local`], [`Core::upload_usage`] | « Mes fichiers » and the quota bar |
//! | [`Core::dislikes`] | `components/DislikedTracks.svelte#load` |
//! | [`Core::unavailable`] | the « Indisponibles » leaf, `#loadGone` |
//! | [`Core::history`] | `routes/History.svelte#loadFirst`/`#loadMore` |
//!
//! # The request budget (rule 3)
//!
//! One leaf of the library costs **one** request, never the four the SPA fires
//! on mount: `Library.svelte` loads the favourites, the playlists, the local
//! files and the upload usage together whatever leaf is open (its `onMount`),
//! while a phone opens ONE leaf and pays only for it. Tags are the screen's own
//! ([`TAG_LIBRARY`]), so `NzCore::stats()` answers what the leaf cost: an 8 000
//! title favourites list is **1** request cold, and **0** before it can be drawn
//! on a warm open (the copy is painted, then revalidated in the background —
//! exactly what `loadMyFavorites()` does).
//!
//! # Optimism, and what is inside it
//!
//! The folder writes are the SPA's: the tree is rewritten **before** the request
//! (`actions.js#movePlaylistTo` moves locally then sends the destination's
//! order), the one request carries the whole desired order, and a refusal rolls
//! the answer back — the caller gets `Err` and keeps the tree it drew, which is
//! what "rolls back with Réessayer" means for a person.
//!
//! `Core` has no field this lot may add (`core/mod.rs` is MB2's file, worker rule
//! 3), so the tree's optimistic copy lives in the store under the CACHE
//! directory: a mutation rewrites that file, sends, and puts the previous body
//! back on failure. That is also what makes the tree survive a restart, which a
//! `static` in this module would not.
//!
//! # What is not here, and whose it is
//!
//! Play tracks (the queue), the heart on a row, "Télécharger" and the context
//! menu are other lots' (`PlayerApi`, MB10, MB15); this screen hands them what
//! they draw and asks them by their MB0 stubs. Uploading NEW files from the
//! phone is not ported (see the lot's section of the note for the cause).

mod store;
#[cfg(test)]
mod testing;
mod wire;

use std::sync::Arc;

use nz_api::Api;
use nz_api::features::Feature;
use nz_model::{FavoriteIds, History, PlaylistList};

use crate::core::{Core, CoreError};
use crate::lists::Row;

/// The tag every request of this area is counted under (`NzCore::stats`).
pub const TAG_LIBRARY: &str = "library";

/// How many history events one page asks for (`History.svelte#PAGE`).
pub const HISTORY_PAGE: u32 = 50;

/// The cap the server itself applies to « Mes fichiers » (`/me/local`).
pub const LOCAL_MAX: u32 = 5_000;

// -- the playlist tree --------------------------------------------------------

/// One folder of playlists, as the « Playlists » leaf's section header draws it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderCard {
    pub id: String,
    pub title: String,
    /// The server's stored position; local reorders renumber it 0..n-1.
    pub position: i64,
    /// How many of this person's playlists it holds (`nb_playlists`).
    pub count: u32,
}

/// One playlist card (`components/Card.svelte`, kind `playlist`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaylistCard {
    pub id: String,
    /// The provider's id when the list came from Deezer — what makes it
    /// « Depuis Deezer » rather than a local list.
    pub deezer_id: Option<String>,
    pub title: String,
    pub cover: Option<String>,
    pub tracks: u32,
    /// `lib/actions.js#isSharedPlaylist`: somebody else's role than `owner`.
    pub shared: bool,
    pub role: String,
    pub owner: String,
    pub editable: bool,
    pub can_manage: bool,
    /// `None` at the root — `folder_id` is null on the wire for both "unfiled"
    /// and "no placement row", exactly as `/me/playlists` reports it.
    pub folder_id: Option<String>,
    pub position: Option<i64>,
}

/// Which of the four kinds a section is (`Library.svelte#buildSections`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    /// A folder, in the person's own order.
    Folder,
    /// « Sans dossier » — the unfiled, unshared lists.
    Root,
    /// « Partagées avec moi » — any list somebody else owns.
    Shared,
    /// « Depuis Deezer » — imported and not filed.
    Imported,
}

/// One section of the « Playlists » leaf: a folder, the root, or one of the two
/// derived shelves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub kind: SectionKind,
    /// `folder:<id>`, `root`, `shared`, `imported` — the UI's key.
    pub key: String,
    pub title: String,
    /// The number the header states (a folder's own count, not the rows it shows).
    pub count: u32,
    pub rows: Vec<PlaylistCard>,
}

/// Everything `GET /api/me/playlists` answers, grouped the way the SPA groups
/// it: the folders in the person's order, the playlists in the server's render
/// order, and the sections both of them make.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryTree {
    pub folders: Vec<FolderCard>,
    pub playlists: Vec<PlaylistCard>,
    /// `sections` is what the screen draws; it is derived from the two lists
    /// above on every change, so it can never disagree with them.
    pub sections: Vec<Section>,
}

impl LibraryTree {
    /// Builds the tree from the server's answer.
    pub fn new(folders: Vec<FolderCard>, playlists: Vec<PlaylistCard>) -> LibraryTree {
        let mut tree = LibraryTree {
            folders,
            playlists,
            sections: Vec::new(),
        };
        tree.regroup();
        tree
    }

    /// Recomputes the sections (`groupPlaylists` + `buildSections`) and each
    /// folder's badge from the playlists it really holds: a local move must move
    /// the count with the row (`groupPlaylists`'s `group.count`).
    pub fn regroup(&mut self) {
        self.sections = sections(&self.folders, &self.playlists, false);
        // The folder sections come first, one per folder, in the folders' order.
        let counts: Vec<u32> = self
            .sections
            .iter()
            .filter(|section| section.kind == SectionKind::Folder)
            .map(|section| section.rows.len() as u32)
            .collect();
        for (folder, count) in self.folders.iter_mut().zip(counts) {
            folder.count = count;
        }
        // The sections were built with the badge the server sent: rebuild them
        // so a heading and its folder agree after a local move.
        self.sections = sections(&self.folders, &self.playlists, false);
    }

    /// The folder with `id`, when it is one of the person's.
    pub fn folder(&self, id: &str) -> Option<&FolderCard> {
        self.folders.iter().find(|folder| folder.id == id)
    }

    /// The playlists one container holds, in the tree's order (`root` for the
    /// `None` container).
    pub fn container(&self, folder: Option<&str>) -> Vec<&PlaylistCard> {
        self.playlists
            .iter()
            .filter(|playlist| container_of(playlist, &self.folders) == folder)
            .collect()
    }

    /// Counted bytes of what this answer holds in memory, for the lot's own
    /// measure (worker rule 4): real string lengths, never a process RSS.
    pub fn memory_bytes(&self) -> u64 {
        let mut total = std::mem::size_of::<LibraryTree>() as u64;
        for folder in &self.folders {
            total += 40 + (folder.id.len() + folder.title.len()) as u64;
        }
        for playlist in &self.playlists {
            total += 64
                + (playlist.id.len()
                    + playlist.title.len()
                    + playlist.role.len()
                    + playlist.owner.len()
                    + playlist.deezer_id.as_deref().map_or(0, str::len)
                    + playlist.cover.as_deref().map_or(0, str::len)
                    + playlist.folder_id.as_deref().map_or(0, str::len)) as u64;
        }
        total
    }
}

/// The container a playlist belongs to, as the SPA decides it
/// (`playlistfolders.js#containerOf`): only a folder this person owns counts, and
/// anything else is the root.
fn container_of<'tree>(
    playlist: &'tree PlaylistCard,
    folders: &'tree [FolderCard],
) -> Option<&'tree str> {
    let folder = playlist.folder_id.as_deref()?;
    folders
        .iter()
        .find(|candidate| candidate.id == folder)
        .map(|candidate| candidate.id.as_str())
}

/// `playlistfolders.js#groupPlaylists`, then `Library.svelte#buildSections`:
/// the folders in the person's order, « Sans dossier », « Partagées avec moi »,
/// « Depuis Deezer ». `filtering` keeps only the sections that have rows, which
/// is what a search does.
pub fn sections(
    folders: &[FolderCard],
    playlists: &[PlaylistCard],
    filtering: bool,
) -> Vec<Section> {
    use std::collections::HashMap;
    let mut groups: Vec<Section> = folders
        .iter()
        .map(|folder| Section {
            kind: SectionKind::Folder,
            key: format!("folder:{}", folder.id),
            title: folder.title.clone(),
            count: folder.count,
            rows: Vec::new(),
        })
        .collect();
    // `byId` in `groupPlaylists`: a folder id the client names must be one the
    // tree holds, or the row is unfiled.
    let index: HashMap<&str, usize> = folders
        .iter()
        .enumerate()
        .map(|(at, folder)| (folder.id.as_str(), at))
        .collect();
    let mut unfiled: Vec<PlaylistCard> = Vec::new();
    let mut imported: Vec<PlaylistCard> = Vec::new();
    for playlist in playlists {
        match playlist
            .folder_id
            .as_deref()
            .and_then(|folder| index.get(folder))
        {
            Some(at) => groups[*at].rows.push(playlist.clone()),
            None if playlist.folder_id.is_none() && playlist.deezer_id.is_some() => {
                imported.push(playlist.clone());
            }
            None => unfiled.push(playlist.clone()),
        }
    }
    let shared: Vec<PlaylistCard> = unfiled
        .iter()
        .chain(imported.iter())
        .filter(|playlist| playlist.shared)
        .cloned()
        .collect();
    let root: Vec<PlaylistCard> = unfiled
        .into_iter()
        .filter(|playlist| !playlist.shared)
        .collect();
    let imported: Vec<PlaylistCard> = imported
        .into_iter()
        .filter(|playlist| !playlist.shared)
        .collect();

    let mut out = groups;
    if !root.is_empty() {
        out.push(Section {
            kind: SectionKind::Root,
            key: "root".to_owned(),
            title: "Sans dossier".to_owned(),
            count: root.len() as u32,
            rows: root,
        });
    }
    if !shared.is_empty() {
        out.push(Section {
            kind: SectionKind::Shared,
            key: "shared".to_owned(),
            title: "Partagées avec moi".to_owned(),
            count: shared.len() as u32,
            rows: shared,
        });
    }
    if !imported.is_empty() {
        out.push(Section {
            kind: SectionKind::Imported,
            key: "imported".to_owned(),
            title: "Depuis Deezer".to_owned(),
            count: imported.len() as u32,
            rows: imported,
        });
    }
    if filtering {
        out.retain(|section| !section.rows.is_empty());
    }
    out
}

/// `playlistfolders.js#moveItem`: one row moved, both ends clamped.
pub fn move_item<T: Clone>(list: &[T], from: usize, to: usize) -> Vec<T> {
    if list.is_empty() {
        return Vec::new();
    }
    let from = from.min(list.len() - 1);
    let to = to.min(list.len() - 1);
    let mut next = list.to_vec();
    if from == to {
        return next;
    }
    let item = next.remove(from);
    next.insert(to, item);
    next
}

/// `playlistfolders.js#idsOf`.
pub fn ids_of<T: HasId>(rows: &[T]) -> Vec<String> {
    rows.iter()
        .map(HasId::id)
        .filter(|id| !id.is_empty())
        .collect()
}

/// What `ids_of` reads: a folder row and a playlist row both have one.
pub trait HasId {
    fn id(&self) -> String;
}

impl HasId for FolderCard {
    fn id(&self) -> String {
        self.id.clone()
    }
}

impl HasId for PlaylistCard {
    fn id(&self) -> String {
        self.id.clone()
    }
}

/// `playlistfolders.js#orderFolders`: the ids' order first, anything the list did
/// not name keeps its place at the end (`Infinity`, then the old index).
pub fn order_folders(folders: &[FolderCard], ids: &[String]) -> Vec<FolderCard> {
    let mut indexed: Vec<(usize, &FolderCard)> = folders.iter().enumerate().collect();
    let rank = |folder: &FolderCard, index: usize| {
        ids.iter()
            .position(|id| *id == folder.id)
            .unwrap_or(ids.len() + index)
    };
    indexed.sort_by_key(|(index, folder)| rank(folder, *index));
    indexed
        .into_iter()
        .enumerate()
        .map(|(position, (_, folder))| FolderCard {
            position: position as i64,
            ..folder.clone()
        })
        .collect()
}

/// `playlistfolders.js#orderContainer`: the target container's rows in `ids`
/// order, then any resident the list omitted; every other container keeps its
/// own rows in its own order, and the whole answer is folders first (in folder
/// order) then the root — exactly the buckets the SPA rebuilds.
///
/// A playlist NAMED by `ids` but sitting in another container moves into the
/// target one: that is how a move is sent (the destination's complete order).
pub fn order_container(
    playlists: &[PlaylistCard],
    folders: &[FolderCard],
    folder: Option<&str>,
    ids: &[String],
) -> Vec<PlaylistCard> {
    use std::collections::HashMap;
    // Each row's new container and rank: named rows go to the target at their
    // `ids` rank; every other row keeps its container and its order (ranked
    // after the named ones, which no bucket can see).
    let rank: HashMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(at, id)| (id.as_str(), at))
        .collect();
    let mut rows: Vec<(Option<String>, usize, usize, &PlaylistCard)> = playlists
        .iter()
        .enumerate()
        .map(|(index, playlist)| {
            let named = rank.get(playlist.id.as_str());
            let container = match named {
                Some(_) => folder.map(str::to_owned),
                None => container_of(playlist, folders).map(str::to_owned),
            };
            let place = named.copied().unwrap_or(ids.len() + index);
            (container, place, index, playlist)
        })
        .collect();
    rows.sort_by_key(|(_, place, index, _)| (*place, *index));

    // The SPA's bucket order: the folders as they are, then the root.
    let mut out: Vec<PlaylistCard> = Vec::with_capacity(playlists.len());
    let emit = |container: Option<&str>, out: &mut Vec<PlaylistCard>| {
        for (position, (_, _, _, playlist)) in rows
            .iter()
            .filter(|(row_container, _, _, _)| row_container.as_deref() == container)
            .enumerate()
        {
            out.push(PlaylistCard {
                folder_id: container.map(str::to_owned),
                position: Some(position as i64),
                ..(*playlist).clone()
            });
        }
    };
    for group in folders {
        emit(Some(group.id.as_str()), &mut out);
    }
    emit(None, &mut out);
    out
}

/// `playlistfolders.js#containerIds`: the ids of one container, in its order.
pub fn container_ids(tree: &LibraryTree, folder: Option<&str>) -> Vec<String> {
    tree.container(folder)
        .into_iter()
        .map(|playlist| playlist.id.clone())
        .filter(|id| !id.is_empty())
        .collect()
}

// -- the other lists ----------------------------------------------------------

/// One followed artist (`/me/artists`), as the « Artistes » grid draws it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtistRow {
    pub id: String,
    pub name: String,
    /// `/api/cover/<id>`, or `None` for the initialled disc.
    pub picture: Option<String>,
    pub tracks: u32,
    pub plays: u32,
    /// When it was followed (ISO date-time) — what « Ajoutés récemment » sorts by.
    pub starred: String,
}

/// One kept album (`/me/albums`), as its card draws it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlbumCard {
    pub id: String,
    pub title: String,
    pub cover: Option<String>,
    pub artist: String,
    pub year: String,
    pub tracks: u32,
}

/// `/api/upload/usage` — the quota bar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UploadUsageRow {
    pub used: u64,
    pub quota: u64,
    pub unlimited: bool,
}

impl UploadUsageRow {
    /// How full the bar is, 0…100 (`Library.svelte#usagePct`).
    pub fn percent(&self) -> f64 {
        if self.quota == 0 {
            return 0.0;
        }
        ((self.used as f64 / self.quota as f64) * 100.0).min(100.0)
    }
}

/// `/api/me/favorite-ids` — the ids only, for the hearts and the car's counter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FavoriteIdSet {
    pub tracks: Vec<String>,
    pub albums: Vec<String>,
    pub artists: Vec<String>,
    pub playlists: Vec<String>,
    /// A dislike is NOT a star (`FavoriteIds`'s own comment).
    pub disliked: Vec<String>,
}

/// One event of the listening history, with the row a tap plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEventRow {
    pub id: i64,
    /// The server's local ISO time, kept as text: the UI groups by the DEVICE's
    /// day, and the core owns no calendar.
    pub at: String,
    pub listened: u32,
    /// `1` a listen, `-1` a skip, `0` neither (`listening.py`'s verdict).
    pub outcome: i32,
    pub source: Option<String>,
    pub track: Row,
}

/// The keyset cursor of the next page: BOTH halves travel together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryCursorRow {
    pub before: String,
    pub before_id: i64,
}

/// One artist of the 30-day summary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TopArtist {
    pub id: String,
    pub name: String,
    pub cover: Option<String>,
    pub count: u32,
}

/// One title of the 30-day summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopTrack {
    pub row: Row,
    pub count: u32,
}

/// The last 30 days, in figures — what the history screen's first card states.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistorySummaryRow {
    pub days: u32,
    pub listens: u32,
    pub skips: u32,
    pub minutes: u32,
    pub top_artists: Vec<TopArtist>,
    pub top_tracks: Vec<TopTrack>,
}

impl HistorySummaryRow {
    /// `History.svelte#emptyHistory`: a history with nothing at all needs the
    /// empty state, not a 30-day card full of zeros.
    pub fn is_empty(&self) -> bool {
        self.listens == 0
            && self.skips == 0
            && self.minutes == 0
            && self.top_artists.is_empty()
            && self.top_tracks.is_empty()
    }
}

/// One page of the history, with the cursor of the next one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryPage {
    pub events: Vec<HistoryEventRow>,
    pub next: Option<HistoryCursorRow>,
    pub summary: HistorySummaryRow,
}

// -- what a screen asks -------------------------------------------------------

impl Core {
    // -- playlists and folders ------------------------------------------------

    /// The playlist tree as this device last saw it — the CACHE directory's
    /// copy, no request.
    pub fn cached_library_playlists(&self) -> Option<LibraryTree> {
        let body = store::read(self, store::PLAYLISTS, "")?;
        let answer = wire::parse_playlists(&body).ok()?;
        Some(tree_of(&answer))
    }

    /// `GET /api/me/playlists` — the SPA's own route, on the `User` lane, one
    /// request; the answer is kept for the next visit.
    pub async fn library_playlists(&self) -> Result<LibraryTree, CoreError> {
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let before = traffic(&api);
        let answer = async {
            let body = wire::fetch_playlists(&api).await?;
            let parsed = wire::parse_playlists(&body)?;
            keep(self, store::PLAYLISTS, "", &body);
            Ok::<_, CoreError>(tree_of(&parsed))
        }
        .await;
        note_traffic(self, &api, before);
        answer
    }

    /// Whether this server has the playlist-folder routes at all (rule 7): one
    /// `OPTIONS` on the `Prefetch` lane asks the route's existence, never a
    /// screen's opening path, and `None` (a doubt) must not hide the feature.
    pub async fn folders_supported(&self) -> Option<bool> {
        let api = self.api().ok()?;
        api.has_route(Feature::Folders.probe()).await
    }

    /// `POST /api/playlist-folders` — a new folder, with the id THIS client
    /// chose (`actions.js#newFolderId`, so a retried POST returns the same
    /// folder instead of making a second one).
    ///
    /// The local tree gains the folder at the end first, the way the SPA's store
    /// does; a refusal answers `Err` and the caller keeps the tree it drew.
    pub async fn create_folder(&self, title: &str) -> Result<LibraryTree, CoreError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(CoreError::Invalid {
                detail: "Le nom du dossier ne peut pas être vide".to_owned(),
            });
        }
        let api = self.api()?;
        let mut tree = self.tree_now().await?;
        let id = new_folder_id();
        let position = tree
            .folders
            .iter()
            .map(|folder| folder.position)
            .max()
            .unwrap_or(-1)
            + 1;
        tree.folders.push(FolderCard {
            id: id.clone(),
            title: title.to_owned(),
            position,
            count: 0,
        });
        tree.regroup();
        self.keep_tree(&tree);
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let before = traffic(&api);
        let answer = async {
            api.create_playlist_folder(title, Some(&id)).await?;
            Ok::<_, CoreError>(tree)
        }
        .await;
        note_traffic(self, &api, before);
        answer
    }

    /// `PATCH /api/playlist-folders/<id>` — rename one of the person's folders.
    pub async fn rename_folder(&self, id: &str, title: &str) -> Result<LibraryTree, CoreError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(CoreError::Invalid {
                detail: "Le nom du dossier ne peut pas être vide".to_owned(),
            });
        }
        let api = self.api()?;
        let mut tree = self.tree_now().await?;
        let folder = tree
            .folders
            .iter_mut()
            .find(|folder| folder.id == id)
            .ok_or_else(|| CoreError::Invalid {
                detail: "Ce dossier n'existe plus".to_owned(),
            })?;
        folder.title = title.to_owned();
        tree.regroup();
        self.keep_tree(&tree);
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let before = traffic(&api);
        let answer = async {
            api.rename_playlist_folder(id, title).await?;
            Ok::<_, CoreError>(tree)
        }
        .await;
        note_traffic(self, &api, before);
        answer
    }

    /// `DELETE /api/playlist-folders/<id>` — the folder goes, its playlists move
    /// to « Sans dossier » (the server cascades the placements; the local tree
    /// mirrors it, `actions.js#deleteFolder`).
    pub async fn delete_folder(&self, id: &str) -> Result<LibraryTree, CoreError> {
        let api = self.api()?;
        let mut tree = self.tree_now().await?;
        if !tree.folders.iter().any(|folder| folder.id == id) {
            return Err(CoreError::Invalid {
                detail: "Ce dossier n'existe plus".to_owned(),
            });
        }
        // What it held, BEFORE the folder goes: `actions.js#deleteFolder` sends
        // them to the END of « Sans dossier », and their count is what the toast
        // states.
        let released = container_ids(&tree, Some(id));
        let mut root_ids = container_ids(&tree, None);
        root_ids.extend(released);
        tree.folders.retain(|folder| folder.id != id);
        tree.playlists = order_container(&tree.playlists, &tree.folders, None, &root_ids);
        tree.regroup();
        self.keep_tree(&tree);
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let before = traffic(&api);
        let answer = async {
            api.delete_playlist_folder(id).await?;
            Ok::<_, CoreError>(tree)
        }
        .await;
        note_traffic(self, &api, before);
        answer
    }

    /// `PUT /api/playlist-folders/order` — the complete desired order of the
    /// folders (the SPA debounces 400 ms, then sends the ids).
    pub async fn reorder_folders(&self, ids: &[String]) -> Result<LibraryTree, CoreError> {
        let api = self.api()?;
        let mut tree = self.tree_now().await?;
        tree.folders = order_folders(&tree.folders, ids);
        tree.regroup();
        self.keep_tree(&tree);
        let ids: Vec<String> = tree
            .folders
            .iter()
            .map(|folder| folder.id.clone())
            .collect();
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let before = traffic(&api);
        let answer = async {
            api.reorder_playlist_folders(&ids).await?;
            Ok::<_, CoreError>(tree)
        }
        .await;
        note_traffic(self, &api, before);
        answer
    }

    /// `PUT /api/playlist-folders/{<id>|root}/order` — the complete desired order
    /// of ONE container.
    pub async fn reorder_container(
        &self,
        folder: Option<&str>,
        ids: &[String],
    ) -> Result<LibraryTree, CoreError> {
        let api = self.api()?;
        let mut tree = self.tree_now().await?;
        tree.playlists = order_container(&tree.playlists, &tree.folders, folder, ids);
        tree.regroup();
        self.keep_tree(&tree);
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let before = traffic(&api);
        let answer = async {
            api.reorder_playlist_container(folder, ids).await?;
            Ok::<_, CoreError>(tree)
        }
        .await;
        note_traffic(self, &api, before);
        answer
    }

    /// `actions.js#movePlaylistTo`: the playlist goes to the END of its new
    /// container locally, and the ONE request is the destination's order (a move
    /// into a folder is the same call as reordering it — the server's own rule).
    ///
    /// Offline this answers `Offline` **before the local move**, exactly like
    /// `movePlaylistTo`'s `if (!get(online)) { offlineFolders(); return false; }`
    /// — which is the acceptance's "the move rolls back with Réessayer".
    pub async fn move_playlist_to(
        &self,
        playlist_id: &str,
        folder: Option<&str>,
    ) -> Result<LibraryTree, CoreError> {
        let api = self.api()?;
        let mut tree = self.tree_now().await?;
        if let Some(folder) = folder
            && tree.folder(folder).is_none()
        {
            return Err(CoreError::Invalid {
                detail: "Ce dossier n'existe plus".to_owned(),
            });
        }
        let playlist = tree
            .playlists
            .iter()
            .find(|playlist| playlist.id == playlist_id)
            .cloned()
            .ok_or_else(|| CoreError::Invalid {
                detail: "Cette playlist n'existe plus".to_owned(),
            })?;
        if container_of(&playlist, &tree.folders) == folder {
            // Already there: no request at all (`movePlaylistTo` returns true).
            return Ok(tree);
        }
        let ids: Vec<String> = container_ids(&tree, folder)
            .into_iter()
            .filter(|id| id != playlist_id)
            .chain(std::iter::once(playlist_id.to_owned()))
            .collect();
        tree.playlists = order_container(&tree.playlists, &tree.folders, folder, &ids);
        tree.regroup();
        self.keep_tree(&tree);
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let before = traffic(&api);
        let answer = async {
            api.reorder_playlist_container(folder, &ids).await?;
            Ok::<_, CoreError>(tree)
        }
        .await;
        note_traffic(self, &api, before);
        answer
    }

    // -- favourites and the light ids -----------------------------------------

    /// The favourites as this device last saw them — disk only, no request. The
    /// 8 000 rows are rebuilt from the kept body by the same converter a live
    /// answer goes through.
    pub fn cached_favorites(&self) -> Option<Vec<Row>> {
        let body = store::read(self, store::FAVORITES, "")?;
        wire::parse_favorites(&body).ok()
    }

    /// `GET /api/me/favorites` — ONE request for the whole list (the server
    /// answers every star in one query), kept for the next visit.
    pub async fn favorites(&self) -> Result<Vec<Row>, CoreError> {
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let before = traffic(&api);
        let answer = async {
            let body = wire::fetch_favorites(&api).await?;
            let rows = wire::parse_favorites(&body)?;
            keep(self, store::FAVORITES, "", &body);
            Ok::<_, CoreError>(rows)
        }
        .await;
        note_traffic(self, &api, before);
        answer
    }

    /// The starred ids, from the device's copy when there is one.
    pub fn cached_favorite_ids(&self) -> Option<FavoriteIdSet> {
        let body = store::read(self, store::FAVORITE_IDS, "")?;
        let answer: FavoriteIds = wire::parse("/me/favorite-ids", &body).ok()?;
        Some(wire::favorite_ids_from(&answer))
    }

    /// `GET /api/me/favorite-ids` — the light route: four short arrays, no
    /// titles. The car's tiles count it, and every heart reads it.
    pub async fn favorite_ids(&self) -> Result<FavoriteIdSet, CoreError> {
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let before = traffic(&api);
        let answer = async {
            let body = wire::fetch_favorite_ids(&api).await?;
            let parsed: FavoriteIds = wire::parse("/me/favorite-ids", &body)?;
            keep(self, store::FAVORITE_IDS, "", &body);
            Ok::<_, CoreError>(wire::favorite_ids_from(&parsed))
        }
        .await;
        note_traffic(self, &api, before);
        answer
    }

    // -- artists, albums, uploads, dislikes, unavailable ----------------------

    pub fn cached_artists(&self) -> Option<Vec<ArtistRow>> {
        let body = store::read(self, store::ARTISTS, "")?;
        let answer: nz_model::ArtistList = wire::parse("/me/artists", &body).ok()?;
        Some(wire::artist_rows(&answer))
    }

    /// `GET /api/me/artists` — the followed artists, ONE request.
    pub async fn artists(&self) -> Result<Vec<ArtistRow>, CoreError> {
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let before = traffic(&api);
        let answer = async {
            let body = wire::fetch_artists(&api).await?;
            let parsed: nz_model::ArtistList = wire::parse("/me/artists", &body)?;
            keep(self, store::ARTISTS, "", &body);
            Ok::<_, CoreError>(wire::artist_rows(&parsed))
        }
        .await;
        note_traffic(self, &api, before);
        answer
    }

    pub fn cached_albums(&self) -> Option<Vec<AlbumCard>> {
        let body = store::read(self, store::ALBUMS, "")?;
        let answer: nz_model::AlbumList = wire::parse("/me/albums", &body).ok()?;
        Some(wire::album_cards(&answer))
    }

    /// `GET /api/me/albums` — the kept albums, ONE request.
    pub async fn albums(&self) -> Result<Vec<AlbumCard>, CoreError> {
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let before = traffic(&api);
        let answer = async {
            let body = wire::fetch_albums(&api).await?;
            let parsed: nz_model::AlbumList = wire::parse("/me/albums", &body)?;
            keep(self, store::ALBUMS, "", &body);
            Ok::<_, CoreError>(wire::album_cards(&parsed))
        }
        .await;
        note_traffic(self, &api, before);
        answer
    }

    /// « Mes fichiers » as this device last saw them.
    pub fn cached_local(&self) -> Option<Vec<Row>> {
        let body = store::read(self, store::LOCAL, "")?;
        wire::parse_local(&body).ok()
    }

    /// `GET /api/me/local` — the person's own files, ONE request.
    pub async fn local(&self) -> Result<Vec<Row>, CoreError> {
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let before = traffic(&api);
        let answer = async {
            let body = wire::fetch_local(&api).await?;
            let rows = wire::parse_local(&body)?;
            keep(self, store::LOCAL, "", &body);
            Ok::<_, CoreError>(rows)
        }
        .await;
        note_traffic(self, &api, before);
        answer
    }

    /// The upload quota as this device last saw it.
    pub fn cached_upload_usage(&self) -> Option<UploadUsageRow> {
        let body = store::read(self, store::UPLOAD_USAGE, "")?;
        let answer: nz_model::UploadUsage = wire::parse("/upload/usage", &body).ok()?;
        Some(UploadUsageRow::from(&answer))
    }

    /// `GET /api/upload/usage` — this person's uploads against their cap.
    pub async fn upload_usage(&self) -> Result<UploadUsageRow, CoreError> {
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let before = traffic(&api);
        let answer = async {
            let body = wire::fetch_upload_usage(&api).await?;
            let parsed: nz_model::UploadUsage = wire::parse("/upload/usage", &body)?;
            keep(self, store::UPLOAD_USAGE, "", &body);
            Ok::<_, CoreError>(UploadUsageRow::from(&parsed))
        }
        .await;
        note_traffic(self, &api, before);
        answer
    }

    /// The dislikes as this device last saw them.
    pub fn cached_dislikes(&self) -> Option<Vec<Row>> {
        let body = store::read(self, store::DISLIKES, "")?;
        wire::parse_tracks("/me/dislikes", &body).ok()
    }

    /// `GET /api/me/dislikes` — the titles the person said they do not like.
    pub async fn dislikes(&self) -> Result<Vec<Row>, CoreError> {
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let before = traffic(&api);
        let answer = async {
            let body = wire::fetch_dislikes(&api).await?;
            let rows = wire::parse_tracks("/me/dislikes", &body)?;
            keep(self, store::DISLIKES, "", &body);
            Ok::<_, CoreError>(rows)
        }
        .await;
        note_traffic(self, &api, before);
        answer
    }

    /// `POST /api/dislike` with `on: false` — « Retirer » on the dislikes list
    /// (`DislikedTracks.svelte#remove`). MB10 owns the ONE menu's like/dislike;
    /// this route is the one thing this screen does itself, exactly as the SPA
    /// does it here. The kept copy is dropped so the next visit cannot resurrect
    /// the row.
    pub async fn remove_dislike(&self, reference: &str) -> Result<(), CoreError> {
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let before = traffic(&api);
        let answer = async {
            api.dislike(reference, false).await?;
            store::forget(self, store::DISLIKES, "");
            Ok::<_, CoreError>(())
        }
        .await;
        note_traffic(self, &api, before);
        answer
    }

    /// The unavailable rows as this device last saw them.
    pub fn cached_unavailable(&self) -> Option<Vec<Row>> {
        let body = store::read(self, store::UNAVAILABLE, "")?;
        wire::parse_tracks("/unavailable", &body).ok()
    }

    /// `GET /api/unavailable` — the titles their provider no longer has, ONE
    /// request, asked when the leaf opens (never on the library's own load).
    pub async fn unavailable(&self) -> Result<Vec<Row>, CoreError> {
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let before = traffic(&api);
        let answer = async {
            let body = wire::fetch_unavailable(&api).await?;
            let rows = wire::parse_tracks("/unavailable", &body)?;
            keep(self, store::UNAVAILABLE, "", &body);
            Ok::<_, CoreError>(rows)
        }
        .await;
        note_traffic(self, &api, before);
        answer
    }

    // -- history --------------------------------------------------------------

    /// The first page of the history for `outcome` (`""`: everything), from the
    /// device's copy.
    ///
    /// Only the FIRST page is kept: a cursor page is a position in a live log,
    /// not "the list" (`apicache.js` refuses `/history` for the same reason).
    pub fn cached_history(&self, outcome: &str) -> Option<HistoryPage> {
        let body = store::read(self, store::HISTORY, outcome)?;
        let answer: History = wire::parse("/history", &body).ok()?;
        Some(wire::history_page(&answer))
    }

    /// `GET /api/history?…` — one page of the person's own listens, newest
    /// first, with the 30-day summary. `before`/`before_id` are the cursor the
    /// previous page answered; both or neither.
    pub async fn history(
        &self,
        outcome: Option<&str>,
        before: Option<&str>,
        before_id: Option<i64>,
        limit: u32,
    ) -> Result<HistoryPage, CoreError> {
        let api = self.api()?;
        let limit = limit.clamp(1, 200);
        let filter = outcome.unwrap_or("");
        let path = wire::history_path(outcome, before, before_id, limit);
        let first_page = before.is_none() && before_id.is_none();
        let api = Arc::new(api.with_tag(TAG_LIBRARY));
        let started = traffic(&api);
        let answer = async {
            let body = wire::fetch_history(&api, &path).await?;
            let parsed: History = wire::parse(&path, &body)?;
            if first_page {
                keep(self, store::HISTORY, filter, &body);
            }
            Ok::<_, CoreError>(wire::history_page(&parsed))
        }
        .await;
        note_traffic(self, &api, started);
        answer
    }

    // -- internals ------------------------------------------------------------

    /// The tree a mutation starts from: the device's copy, or ONE request when
    /// this device never opened the leaf (the SPA's `userPlaylists()` would ask
    /// it too).
    async fn tree_now(&self) -> Result<LibraryTree, CoreError> {
        match self.cached_library_playlists() {
            Some(tree) => Ok(tree),
            None => self.library_playlists().await,
        }
    }

    /// Keeps the optimistic tree on the device: the next visit — and a restart —
    /// paints what the person just did, before any request answers.
    fn keep_tree(&self, tree: &LibraryTree) {
        let answer = PlaylistList {
            folders: tree
                .folders
                .iter()
                .map(|folder| nz_model::PlaylistFolder {
                    id: folder.id.clone(),
                    title: folder.title.clone(),
                    position: Some(folder.position.clamp(0, u32::MAX as i64) as u32),
                    nb_playlists: Some(folder.count),
                })
                .collect(),
            playlists: tree
                .playlists
                .iter()
                .map(|playlist| nz_model::Playlist {
                    id: nz_model::PlaylistId::new(&playlist.id),
                    deezer_id: playlist.deezer_id.clone(),
                    title: playlist.title.clone(),
                    cover: playlist.cover.clone(),
                    owner: Some(playlist.owner.clone()),
                    role: Some(playlist.role.clone()),
                    editable: playlist.editable,
                    can_manage: playlist.can_manage,
                    nb_tracks: playlist.tracks,
                    folder_id: playlist.folder_id.clone(),
                    position: playlist
                        .position
                        .map(|position| position.clamp(0, u32::MAX as i64) as u32),
                    ..nz_model::Playlist::default()
                })
                .collect(),
        };
        let body = match serde_json::to_string(&answer) {
            Ok(body) => body,
            Err(error) => {
                tracing::debug!(%error, "the library tree was not written");
                return;
            }
        };
        keep(self, store::PLAYLISTS, "", &body);
    }
}

/// The tree from the server's answer, grouped.
fn tree_of(answer: &PlaylistList) -> LibraryTree {
    LibraryTree::new(
        answer.folders.iter().map(FolderCard::from).collect(),
        answer.playlists.iter().map(PlaylistCard::from).collect(),
    )
}

/// Keeps one body, reporting a cache that cannot write at debug level only: the
/// answer is already in hand and the next visit asks again.
fn keep(core: &Core, kind: &str, key: &str, body: &str) {
    if let Err(cause) = store::write(core, kind, key, body) {
        tracing::debug!(%cause, "a library cache file was not written");
    }
}

// `nz_model::PlaylistId::new` exists for the tests' own fixtures; the tree it
// rebuilds is what `keep_tree` writes.

/// What one tag spent, as `nz-api` counts it (SC3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Totals {
    requests: u64,
    bytes_in: u64,
    bytes_out: u64,
}

fn traffic(api: &Api) -> Totals {
    api.stats()
        .into_iter()
        .find(|row| row.tag == TAG_LIBRARY)
        .map(|row| Totals {
            requests: row.requests,
            bytes_in: row.bytes_in,
            bytes_out: row.bytes_out,
        })
        .unwrap_or_default()
}

/// Books one leaf's traffic in the core's meter, so `NzCore::stats()` answers
/// what this screen cost without the UI counting anything itself.
fn note_traffic(core: &Core, api: &Api, before: Totals) {
    let after = traffic(api);
    let requests = after.requests.saturating_sub(before.requests);
    if requests == 0 {
        return;
    }
    core.meter().record(
        TAG_LIBRARY,
        after.bytes_in.saturating_sub(before.bytes_in),
        after.bytes_out.saturating_sub(before.bytes_out),
    );
    for _ in 1..requests {
        core.meter().record(TAG_LIBRARY, 0, 0);
    }
}

/// A v4-shaped UUID, chosen by this client so a retried `POST` cannot make two
/// folders (`actions.js#newFolderId`).
///
/// The bytes come from the clock, the process and a counter, mixed with FNV-1a:
/// no dependency for four lines, and the id must only be UNIQUE per client, not
/// unguessable (the server checks it belongs to the caller).
fn new_folder_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or(0);
    let mixed = store::fnv1a(
        &format!(
            "{nanos}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
        .into_bytes(),
    );
    let other = store::fnv1a(&mixed.to_le_bytes());
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&mixed.to_be_bytes());
    bytes[8..].copy_from_slice(&other.to_be_bytes());
    // Version 4, variant 1 (`RFC 4122`, what the server's own `uuid.UUID` parses).
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests;
