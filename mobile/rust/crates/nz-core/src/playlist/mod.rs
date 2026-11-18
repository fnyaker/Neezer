//! One playlist: its page, its edits, its people, its public link, its
//! recommendations, its export and the import of somebody else's link.
//!
//! Owned by **MB9** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this directory and edits no other file of the crate.
//!
//! The behaviour is `webapp/src/routes/Playlist.svelte` and the sheets it opens,
//! PORTED — not re-invented. What lives where:
//!
//! | This module | The SPA |
//! |---|---|
//! | [`Core::playlist`], [`Core::cached_playlist`] | `Playlist.svelte#load` (the kept copy first, `lib/apicache.js`'s `/playlist/` entry) |
//! | [`Core::playlist_recommendations`] | `Playlist.svelte#loadRecommendations` (after the list, one request) |
//! | [`Core::playlist_move`], [`Core::playlist_save_order`] | `#reorder` then `#queueOrder` (instant, then one coalesced `PUT`) |
//! | [`Core::playlist_add`], [`Core::playlist_remove`] | `#onAdded`, `#removeAt` |
//! | [`Core::playlist_edit`] | `#saveMeta` |
//! | [`Core::playlist_members`] + the three writes | `PlaylistMembersSheet.svelte` |
//! | [`Core::playlist_share`] and its two writes | `ExportSheet.svelte`, tab « Lien public » |
//! | [`Core::export_formats`], [`Core::export_services`], [`Core::export_to_service`], [`Core::export_job`] | `ExportSheet.svelte`, tabs « Audio (ZIP) », « Service », « Liste » |
//! | [`Core::playlist_import_start`], [`Core::playlist_import_status`], [`Core::playlist_import_save`] | `routes/Import.svelte`, `neezer/webui/playlist_import.py` |
//! | [`Core::shared_playlist`] | `routes/SharedPlaylist.svelte` (the public page) |
//!
//! # The request budget (rule 3)
//!
//! A 4 000-row playlist opens with **one** request: `GET /api/playlist/<id>`
//! carries the whole tracklist (never a page loop, unlike the SPA's older
//! lists). The recommendations are a SECOND request, made by the screen once the
//! list is on screen — the SPA's own beat, so the page never waits for them.
//! A warm visit paints from the copy `nz-api` kept for `/playlist/<id>`
//! (`lib/apicache.js`'s rule): **0 blocking requests**. Every request here is
//! counted under [`TAG_PLAYLIST`], so `NzCore::stats()` answers what the screen
//! cost.
//!
//! # What is kept in memory, and why
//!
//! A reload of a 4 000-row page per drag would be absurd, so the core keeps the
//! open playlist — meta, capabilities, members, rows, recommendations — in a
//! tiny LRU ([`OPEN_MAX`] entries, the one on screen and the one just left).
//! Every mutation edits that state FIRST (the optimistic write), and a failure
//! puts it back; the counted bytes of the whole registry are
//! [`memory_bytes`] — never a process RSS (worker rule 4).
//!
//! # The conflict rule (`Playlist.svelte`'s 409 machine)
//!
//! Every order-sensitive write carries the `version` (or `members_version`) the
//! screen had. A `409` is **never replayed**: the core drops the optimistic
//! order, makes ONE re-read of the scope the server named (`content` or
//! `members`), and answers a [`Conflict`] alongside the fresh state, so the
//! screen shows the server's truth and says so. [`Core::playlist_move`] is the
//! in-memory half (no request, answers in the frame) and
//! [`Core::playlist_save_order`] the network half (one `PUT`), exactly the SPA's
//! two beats.

#[cfg(test)]
mod testing;
mod wire;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};

use crate::core::{Core, CoreError};
use crate::lists::Row;

pub use wire::TAG_PLAYLIST;

/// How many recommendations the screen asks for (`Playlist.svelte`: 12), and the
/// server's own default (`neezer/webui/playlistrecs.py#DEFAULT_LIMIT`).
pub const RECOMMENDATIONS_LIMIT: u32 = 12;

/// How many playlists stay in memory: the one on screen, and the one just left
/// so going back paints at once. A 4 000-row page is ~1 MB of rows, which is
/// what the cap bounds.
pub const OPEN_MAX: usize = 2;

/// The longest title or description the server accepts (`Playlist.svelte`'s
/// `maxlength="256"`, `_playlist_title`).
pub const TEXT_MAX: usize = 256;

/// Who the caller is on this playlist, as the server named it (`role_for`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    #[default]
    Owner,
    Editor,
    Viewer,
    /// A server admin: everything an owner may do, on anybody's playlist.
    Admin,
    /// A reader of the public link: no members, no history, nothing to keep.
    Public,
}

impl Role {
    /// The server's own word (`_playlist_guard`'s `role`).
    pub fn parse(text: &str) -> Role {
        match text {
            "editor" => Role::Editor,
            "viewer" => Role::Viewer,
            "admin" => Role::Admin,
            "public" => Role::Public,
            _ => Role::Owner,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Editor => "editor",
            Role::Viewer => "viewer",
            Role::Admin => "admin",
            Role::Public => "public",
        }
    }

    /// Whether the list may be changed (`playlist_access.role_can_edit`).
    pub fn can_edit(self) -> bool {
        matches!(self, Role::Owner | Role::Editor | Role::Admin)
    }

    /// Whether the playlist itself may be renamed, deleted, shared.
    pub fn can_manage(self) -> bool {
        matches!(self, Role::Owner | Role::Admin)
    }
}

/// One collaborator, as the sheet draws them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberRow {
    pub id: String,
    pub name: String,
    pub role: Role,
    /// ISO date the server added them, when it says.
    pub added: Option<String>,
}

/// The header of a playlist: what the page draws above the list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub id: String,
    pub deezer_id: Option<String>,
    pub title: String,
    pub description: String,
    /// A relative server path (`/api/cover/<id>`), or `None` for the grooves.
    pub cover: Option<String>,
    /// The owner's name (the first member, as the server orders them).
    pub owner: String,
    /// The caller's visible row count.
    pub nb_tracks: u32,
    /// Somebody else's private upload: counted, never listed
    /// (`playlist.private_tracks`), and only for a manager.
    pub private_tracks: u32,
    pub is_favorite: bool,
    /// The optimistic-concurrency token every write carries.
    pub version: u32,
    pub members_version: u32,
    pub last_edited_by: Option<String>,
    pub last_edited_at: Option<String>,
}

/// What the bottom of a playlist shows: library titles near its own, and the
/// state of that read (`Playlist.svelte#recsState`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecState {
    /// Nothing asked yet.
    #[default]
    Idle,
    Loading,
    Ready,
    /// The read failed: the screen offers « Réessayer » (rule 3).
    Failed,
}

/// The recommendations and where their read stands.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Recommendations {
    pub state: RecState,
    pub rows: Vec<Row>,
}

impl Recommendations {
    fn memory_bytes(&self) -> u64 {
        self.rows.iter().map(row_bytes).sum()
    }
}

/// The counted bytes of one row: the strings it owns, plus the row itself.
///
/// `lists::Row` keeps its own counter private to MB2's module (one writer per
/// file), so this is the same arithmetic on the same fields — the number
/// `NzCore::memory()`'s playlist line reports (worker rule 4: counted bytes,
/// never a process RSS).
fn row_bytes(row: &Row) -> u64 {
    (std::mem::size_of::<Row>()
        + row.key.len()
        + row.id.len()
        + row.title.len()
        + row.artists.len()
        + row.album.len()
        + row.album_id.as_deref().map_or(0, str::len)
        + row.cover.as_deref().map_or(0, str::len)
        + row.source.len()) as u64
}

/// Which half of a playlist a `409` was about (`{"scope": "content"|"members"}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictScope {
    /// The tracklist, its order, its title: re-read the page.
    Content,
    /// The collaborators: re-read the members only (a few hundred bytes).
    Members,
}

impl ConflictScope {
    pub fn as_str(self) -> &'static str {
        match self {
            ConflictScope::Content => "content",
            ConflictScope::Members => "members",
        }
    }

    pub fn parse(text: &str) -> ConflictScope {
        if text == "members" {
            ConflictScope::Members
        } else {
            ConflictScope::Content
        }
    }
}

/// The whole page, as one answer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Data {
    pub meta: Meta,
    pub role: Role,
    pub editable: bool,
    pub can_manage: bool,
    pub admin: bool,
    pub members: Vec<MemberRow>,
    /// The tracklist in its own order — the FFI opens a `ListHandle` over it,
    /// never a 4 000-record copy across the boundary.
    pub rows: Arc<Vec<Row>>,
    pub recommendations: Recommendations,
}

impl Data {
    /// Whether the caller may add/remove/reorder (the SPA's `editable`).
    pub fn editable(&self) -> bool {
        self.editable
    }

    /// Whether this is a collaborative playlist — more than one member, the only
    /// case where the live probe is worth a request (`Playlist.svelte`).
    pub fn collaborative(&self) -> bool {
        self.members.len() > 1
    }

    /// The counted bytes this answer's rows occupy in the core.
    pub fn memory_bytes(&self) -> u64 {
        self.rows.iter().map(row_bytes).sum()
    }
}

/// What a write answers: the fresh state, and whether the world had moved.
///
/// A `409` is NOT an error here: the core re-read the scope the server named,
/// so the screen gets the server's truth in the same answer plus the cause it
/// prints ("Modifiée entre-temps — la liste a été rechargée"). A real failure
/// (offline, `403`, `500`) is an [`CoreError`] and the core has already put its
/// state back.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Write {
    pub data: Data,
    /// The write lost its race; the state above is the server's.
    pub conflict: Option<ConflictScope>,
}

/// What a reorder answers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Order {
    /// The row order now in memory (a screen swaps its handle for these).
    pub rows: Arc<Vec<Row>>,
    /// The order the server confirmed, or the one it had before a conflict.
    pub version: u32,
    pub conflict: Option<ConflictScope>,
}

/// The collaborators and what the caller may do — the shape every membership
/// answer has (`playlist_members.py#_members_answer`, plus the capabilities).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MembersWrite {
    pub members: Vec<MemberRow>,
    pub members_version: u32,
    pub role: Role,
    pub can_edit: bool,
    pub can_manage: bool,
    pub admin: bool,
    pub conflict: Option<ConflictScope>,
    /// The server refused the right (`403`): the capabilities above are the
    /// refreshed ones, and the screen says the right is gone.
    pub denied: bool,
}

/// The public read-only link of one playlist.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareLink {
    pub token: String,
    /// The path under the server's origin (`/pl/<token>`).
    pub url: String,
    pub created: Option<String>,
    pub expires: Option<String>,
}

/// One format the bulk (ZIP) export can produce.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportFormat {
    pub id: String,
    pub label: String,
    pub ext: String,
}

/// The formats and the one the sheet starts on (`/api/export/formats`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportFormats {
    pub default: String,
    pub formats: Vec<ExportFormat>,
}

/// One service the export can reach (`/api/services`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceRow {
    pub id: String,
    pub name: String,
    /// The credential is in place: an export can run.
    pub ready: bool,
    pub linked: bool,
    pub account: Option<String>,
    /// A service with no door: it exports a list FILE instead.
    pub file_only: bool,
}

/// The services and the list formats, as the sheet draws them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Services {
    pub services: Vec<ServiceRow>,
    pub formats: Vec<ExportFormat>,
}

/// One title the export could not find on the other service, with its artist —
/// the sheet lists them and offers to copy the list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingTitle {
    pub title: String,
    pub artist: String,
}

/// One export job, as the sheet follows it (a poll every 800 ms, the SPA's own).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExportJob {
    pub service: String,
    pub stage: String,
    pub running: bool,
    pub total: u32,
    pub done: u32,
    pub matched: u32,
    /// How many titles are missing in all (the list below is capped by the
    /// server at `MISSING_SHOWN`).
    pub missing_count: u32,
    pub missing: Vec<MissingTitle>,
    /// The other service's page, once the playlist was created there.
    pub url: Option<String>,
    /// Why it failed, in the server's own words.
    pub error: Option<String>,
    /// The other service must be linked again (Réglages → Comptes).
    pub relink: bool,
}

impl ExportJob {
    /// `0..=100`, the number the progress bar reads.
    pub fn percent(&self) -> u32 {
        self.done
            .checked_mul(100)
            .map_or(0, |done| (done / self.total).min(100))
    }
}

/// One line of a shared playlist, as the import read it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportLine {
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_s: u32,
}

/// One line's outcome: the sound this server will play for it, when there is
/// one, and how sure the match is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportItem {
    pub line: ImportLine,
    /// `None` — no safe version: the list below is what the person searches for.
    pub track: Option<Row>,
    /// `local`, or the source's name (`deezer`, `youtube`).
    pub via: String,
    /// False when the match is close but not certain ("À vérifier" in the SPA).
    pub sure: bool,
}

/// What an import job answers while it runs and when it ends
/// (`playlist_import.py#_snapshot`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportStatus {
    pub id: String,
    /// `reading`, `matching`, `saving`, `done`, `error`, `saved`.
    pub state: String,
    pub source: String,
    pub source_label: String,
    pub kind: String,
    pub url: String,
    pub name: String,
    pub cover: Option<String>,
    pub total: u32,
    /// The link held more lines than `linkimport.MAX_TRACKS`.
    pub truncated: bool,
    pub done: u32,
    pub matched: u32,
    /// Why the job failed, in the server's words (rule 2: a cause, not a count).
    pub error: Option<String>,
    /// Present once the import was kept.
    pub playlist_id: Option<String>,
    /// How many lines were found where: `local`, a source's name, or `none`.
    pub counts: Vec<(String, u32)>,
    /// Every line's outcome — only once the read is complete, which is the
    /// server's own rule (a 1 000-line import is not re-sent as it grows).
    pub items: Vec<ImportItem>,
}

impl ImportStatus {
    /// Whether the job is still doing something (the poll's own question).
    pub fn running(&self) -> bool {
        matches!(self.state.as_str(), "reading" | "matching" | "saving")
    }

    /// The matched titles, in the link's order: what « Lire » queues.
    pub fn rows(&self) -> Vec<Row> {
        self.items
            .iter()
            .filter_map(|item| item.track.clone())
            .collect()
    }

    /// `0..=100` for the progress bar (`done / total`, 0 while reading).
    pub fn percent(&self) -> u32 {
        self.done
            .checked_mul(100)
            .map_or(0, |done| (done / self.total).min(100))
    }
}

/// The header of a public page: no ids, no members, no role.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedHeader {
    pub title: String,
    pub description: String,
    pub cover: Option<String>,
    pub nb_tracks: u32,
    pub owner: String,
}

/// One row of a public page. Its `mid` is the id the capability stream takes,
/// and `streamable` says whether the file is on disk NOW — an unarchived title
/// is listed and not playable, never a `409` on a tap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedRow {
    pub row: Row,
    pub mid: Option<String>,
    pub streamable: bool,
    /// The capability URL this row streams from (`/api/shared/playlist/<token>/
    /// stream/<mid>`), built once so a screen never assembles one.
    pub stream_path: Option<String>,
}

/// What the public page draws (`routes/SharedPlaylist.svelte`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SharedData {
    pub header: SharedHeader,
    pub rows: Vec<SharedRow>,
    /// The token the page was opened with — the stream's capability.
    pub token: String,
}

impl SharedData {
    /// The rows a player may queue: the archived ones only.
    pub fn playable(&self) -> Vec<&SharedRow> {
        self.rows.iter().filter(|row| row.streamable).collect()
    }
}

// -- the open playlists -------------------------------------------------------

/// One playlist, open: the single source of truth a screen's writes edit first.
#[derive(Debug)]
pub struct Playlist {
    meta: RwLock<Meta>,
    role: RwLock<Role>,
    access: RwLock<Access>,
    members: RwLock<Vec<MemberRow>>,
    rows: RwLock<Arc<Vec<Row>>>,
    recommendations: RwLock<Recommendations>,
    /// How to put the list back after the write that has not landed yet — one
    /// slot, because a screen sends its writes in order (see `PlaylistModel`).
    undo: Mutex<Undo>,
    /// Bumped on every in-memory change; a screen re-windows when it moves.
    epoch: AtomicU64,
}

/// The list as it was before the write in flight, so a failure is an exact undo
/// and not a second read (which a dead link cannot make).
#[derive(Debug)]
enum Undo {
    None,
    Appended {
        len: usize,
    },
    Cut {
        /// The rows removed, with the index each occupied.
        removed: Vec<(usize, Row)>,
    },
}

#[derive(Debug, Clone, Copy, Default)]
struct Access {
    editable: bool,
    can_manage: bool,
    admin: bool,
}

impl Playlist {
    fn new(data: Data) -> Playlist {
        Playlist {
            meta: RwLock::new(data.meta),
            role: RwLock::new(data.role),
            access: RwLock::new(Access {
                editable: data.editable,
                can_manage: data.can_manage,
                admin: data.admin,
            }),
            members: RwLock::new(data.members),
            rows: RwLock::new(data.rows),
            recommendations: RwLock::new(data.recommendations),
            undo: Mutex::new(Undo::None),
            epoch: AtomicU64::new(1),
        }
    }

    /// The id this state is about.
    pub fn id(&self) -> String {
        self.meta
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .id
            .clone()
    }

    /// The header as it stands now.
    pub fn meta(&self) -> Meta {
        self.meta
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    fn data(&self) -> Data {
        let access = *self.access.read().unwrap_or_else(|err| err.into_inner());
        Data {
            meta: self.meta(),
            role: *self.role.read().unwrap_or_else(|err| err.into_inner()),
            editable: access.editable,
            can_manage: access.can_manage,
            admin: access.admin,
            members: self
                .members
                .read()
                .unwrap_or_else(|err| err.into_inner())
                .clone(),
            rows: self.rows(),
            recommendations: self
                .recommendations
                .read()
                .unwrap_or_else(|err| err.into_inner())
                .clone(),
        }
    }

    fn rows(&self) -> Arc<Vec<Row>> {
        Arc::clone(&self.rows.read().unwrap_or_else(|err| err.into_inner()))
    }

    fn version(&self) -> u32 {
        self.meta
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .version
    }

    fn members_version(&self) -> u32 {
        self.meta
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .members_version
    }

    /// Replaces everything with an authoritative answer (a read or a write).
    fn adopt(&self, data: Data) {
        *self.meta.write().unwrap_or_else(|err| err.into_inner()) = data.meta;
        *self.role.write().unwrap_or_else(|err| err.into_inner()) = data.role;
        *self.access.write().unwrap_or_else(|err| err.into_inner()) = Access {
            editable: data.editable,
            can_manage: data.can_manage,
            admin: data.admin,
        };
        *self.members.write().unwrap_or_else(|err| err.into_inner()) = data.members;
        *self.rows.write().unwrap_or_else(|err| err.into_inner()) = data.rows;
        {
            let mut held = self
                .recommendations
                .write()
                .unwrap_or_else(|err| err.into_inner());
            // A re-read of the PAGE keeps the suggestions already on screen (the
            // SPA's module cache does the same): only their own read replaces
            // them. A page answer carries `Idle`, which means "unchanged here".
            if data.recommendations.state != RecState::Idle || held.rows.is_empty() {
                *held = data.recommendations;
            }
        }
        self.bump();
    }

    /// Adopts an answer that only moved the counters (an add, a reorder).
    fn adopt_counters(&self, version: Option<u32>, nb_tracks: Option<u32>) {
        {
            let mut meta = self.meta.write().unwrap_or_else(|err| err.into_inner());
            if let Some(version) = version {
                meta.version = version;
            }
            if let Some(nb_tracks) = nb_tracks {
                meta.nb_tracks = nb_tracks;
            }
        }
        self.bump();
    }

    fn bump(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    /// Puts the rows back after a failed write.
    fn set_rows(&self, rows: Vec<Row>) {
        *self.rows.write().unwrap_or_else(|err| err.into_inner()) = Arc::new(rows);
        self.bump();
    }

    /// Moves one row, in memory only. The frame's answer.
    fn move_row(&self, from: u32, to: u32) -> Arc<Vec<Row>> {
        let mut rows = Vec::clone(&self.rows());
        let len = rows.len();
        let (from, to) = (from as usize, to as usize);
        if from < len && to < len && from != to {
            let row = rows.remove(from);
            rows.insert(to, row);
            reindex(&mut rows);
            *self.rows.write().unwrap_or_else(|err| err.into_inner()) = Arc::new(rows);
            self.bump();
        }
        self.rows()
    }

    /// Remembers how to undo the write in flight.
    fn remember_undo(&self, undo: Undo) {
        *self.undo.lock().unwrap_or_else(|err| err.into_inner()) = undo;
    }

    /// The refs the append in flight is for (the rows it added, in order).
    fn appended_rows(&self) -> Vec<String> {
        let undo = self.undo.lock().unwrap_or_else(|err| err.into_inner());
        let Undo::Appended { len } = *undo else {
            return Vec::new();
        };
        drop(undo);
        self.rows()
            .iter()
            .skip(len)
            .map(|row| row.id.clone())
            .collect()
    }

    /// Puts the list back as it was before the write that failed.
    fn undo(&self) {
        let undo = {
            let mut slot = self.undo.lock().unwrap_or_else(|err| err.into_inner());
            std::mem::replace(&mut *slot, Undo::None)
        };
        match undo {
            Undo::None => {}
            Undo::Appended { len } => {
                let mut rows = Vec::clone(&self.rows());
                rows.truncate(len);
                self.set_rows(rows);
            }
            Undo::Cut { removed } => {
                let mut rows = Vec::clone(&self.rows());
                for (index, row) in removed {
                    let at = index.min(rows.len());
                    rows.insert(at, row);
                }
                reindex(&mut rows);
                self.set_rows(rows);
            }
        }
    }

    fn memory_bytes(&self) -> u64 {
        let rows: u64 = self.rows().iter().map(row_bytes).sum();
        let members: u64 = self
            .members
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .iter()
            .map(|member| (member.id.len() + member.name.len()) as u64 + 32)
            .sum();
        let recs = self
            .recommendations
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .memory_bytes();
        rows + members + recs
    }
}

/// The open playlists, most recently used first, keyed by the install they
/// belong to and the playlist's id.
///
/// Module-level on purpose: the core's own object graph belongs to MB2, and a
/// screen looks at ONE playlist at a time — so a two-entry LRU PER INSTALL is
/// the whole state, bounded in bytes. The key carries the data directory so two
/// accounts (this app's own `<filesDir>/neezer` per profile) never share a page,
/// and so a test's server never answers another test's playlist.
static OPEN: Mutex<Vec<(String, Arc<Playlist>)>> = Mutex::new(Vec::new());

/// `"<data dir>|<playlist id>"`: the one key a page is held under.
fn key_of(core: &Core, id: &str) -> String {
    format!("{}|{id}", core.config().data_dir.display())
}

fn held(core: &Core, id: &str) -> Option<Arc<Playlist>> {
    let key = key_of(core, id);
    let mut open = OPEN.lock().unwrap_or_else(|err| err.into_inner());
    let at = open.iter().position(|(held, _)| *held == key)?;
    let (key, found) = open.remove(at);
    open.insert(0, (key, Arc::clone(&found)));
    Some(found)
}

fn adopt(core: &Core, state: Arc<Playlist>) {
    let key = key_of(core, &state.id());
    let owner = key.split('|').next().unwrap_or_default().to_owned();
    let mut open = OPEN.lock().unwrap_or_else(|err| err.into_inner());
    open.retain(|(held, _)| *held != key);
    open.insert(0, (key, state));
    // At most [`OPEN_MAX`] pages per install: the ones behind the person, oldest
    // first, are dropped.
    let mut seen = 0usize;
    open.retain(|(held, _)| {
        if !held.starts_with(&format!("{owner}|")) {
            return true;
        }
        seen += 1;
        seen <= OPEN_MAX
    });
}

fn forget(core: &Core, id: &str) {
    let key = key_of(core, id);
    let mut open = OPEN.lock().unwrap_or_else(|err| err.into_inner());
    open.retain(|(held, _)| *held != key);
}

/// The counted bytes of every open playlist (`NzCore::memory`'s playlist line).
pub fn memory_bytes() -> u64 {
    let open = OPEN.lock().unwrap_or_else(|err| err.into_inner());
    open.iter()
        .map(|(_, playlist)| playlist.memory_bytes())
        .sum()
}

/// How many pages are open right now (the LRU's own invariant: ≤ [`OPEN_MAX`]
/// per install). A test's assertion, and what a diagnostic screen would print.
pub fn open_pages() -> usize {
    let open = OPEN.lock().unwrap_or_else(|err| err.into_inner());
    open.len()
}

/// The rows of a tracklist, numbered in its own order.
fn reindex(rows: &mut [Row]) {
    for (position, row) in rows.iter_mut().enumerate() {
        row.position = position as u32;
    }
}

/// The playlist a write is about, or the cause the screen prints.
fn open_state(core: &Core, id: &str) -> Result<Arc<Playlist>, CoreError> {
    held(core, id).ok_or_else(|| CoreError::Invalid {
        detail: "cette playlist n'est plus ouverte".to_owned(),
    })
}

/// What the membership calls share: adopt the answer, or turn the refusal into
/// what the sheet draws (a `409` says which half moved, a `403` that the right
/// is gone — `PlaylistMembersSheet.svelte#handle`).
async fn finish_members(
    core: &Core,
    id: &str,
    state: &Arc<Playlist>,
    answer: Result<wire::MemberAnswer, wire::Failure>,
) -> Result<MembersWrite, CoreError> {
    match answer {
        Ok(answer) => {
            adopt_members(state, &answer);
            Ok(answer_with(state, None, false))
        }
        Err(failure) => {
            if let Some(scope) = failure.conflict {
                resync(core, state, scope).await?;
                return Ok(answer_with(state, Some(scope), false));
            }
            if matches!(failure.cause, CoreError::Server { status: 403, .. }) {
                // The right went away under the sheet: one small GET says what the
                // caller may do now, and the sheet closes itself.
                if let Ok(answer) = wire::read_members(core, id).await {
                    adopt_members(state, &answer);
                }
                return Ok(answer_with(state, None, true));
            }
            Err(failure.cause)
        }
    }
}

/// One re-read of the scope the server named — the SPA's `resyncOnce`, with no
/// replay and no merge: what the server holds becomes what the screen shows.
async fn resync(core: &Core, state: &Arc<Playlist>, scope: ConflictScope) -> Result<(), CoreError> {
    let id = state.id();
    match scope {
        ConflictScope::Content => {
            let page = wire::read_page(core, &id).await.map_err(cause_of)?;
            state.adopt(wire::data(page));
        }
        ConflictScope::Members => {
            let answer = wire::read_members(core, &id).await.map_err(cause_of)?;
            adopt_members(state, &answer);
        }
    }
    Ok(())
}

fn cause_of(failure: wire::Failure) -> CoreError {
    failure.cause
}

fn adopt_members(state: &Arc<Playlist>, answer: &wire::MemberAnswer) {
    if !answer.members.is_empty() || answer.members_version != 0 {
        *state.members.write().unwrap_or_else(|err| err.into_inner()) = answer.members.clone();
    }
    {
        let mut meta = state.meta.write().unwrap_or_else(|err| err.into_inner());
        if answer.members_version != 0 {
            meta.members_version = answer.members_version;
        }
    }
    if let Some(role) = answer.role {
        *state.role.write().unwrap_or_else(|err| err.into_inner()) = role;
    }
    if answer.has_access {
        *state.access.write().unwrap_or_else(|err| err.into_inner()) = Access {
            editable: answer.can_edit,
            can_manage: answer.can_manage,
            admin: answer.admin,
        };
    }
    state.bump();
}

fn answer_with(
    state: &Arc<Playlist>,
    conflict: Option<ConflictScope>,
    denied: bool,
) -> MembersWrite {
    let access = *state.access.read().unwrap_or_else(|err| err.into_inner());
    MembersWrite {
        members: state
            .members
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone(),
        members_version: state.members_version(),
        role: *state.role.read().unwrap_or_else(|err| err.into_inner()),
        can_edit: access.editable,
        can_manage: access.can_manage,
        admin: access.admin,
        conflict,
        denied,
    }
}

impl Core {
    /// The playlist this device already has on disk for `id`, and nothing else:
    /// **0 requests**. This is what a warm visit paints before a socket opens
    /// (`nz-api` kept the answer to `/playlist/<id>`, `lib/apicache.js`'s rule).
    pub fn cached_playlist(&self, id: &str) -> Option<Data> {
        if let Some(state) = held(self, id) {
            return Some(state.data());
        }
        let page = wire::cached_page(self, id)?;
        let data = wire::data(page);
        adopt(self, Arc::new(Playlist::new(data.clone())));
        Some(data)
    }

    /// `GET /api/playlist/<id>` — the whole page in ONE request (a 4 000-row
    /// playlist is not a page loop), on the `User` lane, counted under
    /// [`TAG_PLAYLIST`].
    pub async fn playlist(&self, id: &str) -> Result<Data, CoreError> {
        let page = wire::read_page(self, id).await.map_err(cause_of)?;
        let data = wire::data(page);
        match held(self, id) {
            Some(state) => {
                state.adopt(data.clone());
                adopt(self, state);
            }
            None => adopt(self, Arc::new(Playlist::new(data.clone()))),
        }
        Ok(data)
    }

    /// `GET /api/playlist/<id>/recommendations?limit=` — the bottom of the page,
    /// asked AFTER the list is on screen (the SPA's own beat), one request.
    pub async fn playlist_recommendations(
        &self,
        id: &str,
        limit: u32,
    ) -> Result<Recommendations, CoreError> {
        let state = open_state(self, id)?;
        let limit = limit.clamp(1, 20);
        {
            let mut held = state
                .recommendations
                .write()
                .unwrap_or_else(|err| err.into_inner());
            held.state = RecState::Loading;
        }
        match wire::read_recommendations(self, id, limit).await {
            Ok(rows) => {
                let answer = Recommendations {
                    state: RecState::Ready,
                    rows,
                };
                *state
                    .recommendations
                    .write()
                    .unwrap_or_else(|err| err.into_inner()) = answer.clone();
                state.bump();
                Ok(answer)
            }
            Err(failure) => {
                let mut held = state
                    .recommendations
                    .write()
                    .unwrap_or_else(|err| err.into_inner());
                held.state = RecState::Failed;
                drop(held);
                state.bump();
                Err(failure.cause)
            }
        }
    }

    /// The state of the open playlist, for a lot that needs its counters (a test
    /// and MB9's own measurements; nothing on a screen calls it).
    pub fn playlist_state(&self, id: &str) -> Option<Arc<Playlist>> {
        held(self, id)
    }

    /// The reorder's FIRST beat: the rows move now, in memory, and nothing is
    /// sent (`Playlist.svelte#reorder`). The screen swaps its handle for the new
    /// order, so the drag answers in the frame.
    pub fn playlist_move(&self, id: &str, from: u32, to: u32) -> Result<Order, CoreError> {
        let state = open_state(self, id)?;
        if !state.data().editable {
            return Err(CoreError::Server {
                status: 403,
                detail: "lecture seule".to_owned(),
            });
        }
        let rows = state.move_row(from, to);
        Ok(Order {
            rows,
            version: state.version(),
            conflict: None,
        })
    }

    /// The reorder's SECOND beat: ONE `PUT /api/playlist/<id>/order` carrying the
    /// version the screen had. A `409` drops the order, re-reads the page once
    /// (never a replay) and answers the server's order with the cause
    /// (`Playlist.svelte#onConflict`).
    pub async fn playlist_save_order(&self, id: &str) -> Result<Order, CoreError> {
        let state = open_state(self, id)?;
        let refs: Vec<String> = state.rows().iter().map(|row| row.id.clone()).collect();
        if refs.is_empty() {
            return Err(CoreError::Invalid {
                detail: "aucun ordre à enregistrer".to_owned(),
            });
        }
        match wire::put_order(self, id, &refs, state.version()).await {
            Ok(counters) => {
                state.adopt_counters(counters.version, None);
                Ok(Order {
                    rows: state.rows(),
                    version: state.version(),
                    conflict: None,
                })
            }
            Err(failure) => match failure.conflict {
                Some(scope) => {
                    // One re-read of the scope the server named; the order it holds
                    // replaces ours, and the person drags again from the truth.
                    resync(self, &state, scope).await?;
                    Ok(Order {
                        rows: state.rows(),
                        version: state.version(),
                        conflict: Some(scope),
                    })
                }
                None => Err(failure.cause),
            },
        }
    }

    /// Appends `added` to the rows NOW, in memory, and remembers how to undo it
    /// (`Playlist.svelte#onAdded`'s instant half). Answers the new order; nothing
    /// is sent.
    pub fn playlist_append(&self, id: &str, added: &[Row]) -> Result<Order, CoreError> {
        let state = open_state(self, id)?;
        if added.is_empty() {
            return Err(CoreError::Invalid {
                detail: "aucun titre à ajouter".to_owned(),
            });
        }
        let before = state.rows();
        let mut rows = Vec::clone(&before);
        rows.extend(added.iter().cloned());
        reindex(&mut rows);
        state.set_rows(rows);
        state.remember_undo(Undo::Appended { len: before.len() });
        Ok(Order {
            rows: state.rows(),
            version: state.version(),
            conflict: None,
        })
    }

    /// `POST /api/playlist/<id>/tracks` — the appended rows in ONE request; a
    /// failure puts the list back exactly as it was, a `409` re-reads the page.
    pub async fn playlist_save_append(&self, id: &str) -> Result<Write, CoreError> {
        let state = open_state(self, id)?;
        let appended = state.appended_rows();
        if appended.is_empty() {
            return Err(CoreError::Invalid {
                detail: "aucun titre à ajouter".to_owned(),
            });
        }
        match wire::post_tracks(self, id, &appended, state.version()).await {
            Ok(counters) => {
                state.adopt_counters(counters.version, counters.nb_tracks);
                state.remember_undo(Undo::None);
                Ok(Write {
                    data: state.data(),
                    conflict: None,
                })
            }
            Err(failure) => match failure.conflict {
                Some(scope) => {
                    resync(self, &state, scope).await?;
                    Ok(Write {
                        data: state.data(),
                        conflict: Some(scope),
                    })
                }
                None => {
                    state.undo();
                    Err(failure.cause)
                }
            },
        }
    }

    /// Removes the rows at `indexes` NOW, in memory, and remembers them
    /// (`#removeAt`'s instant half). The indexes are the caller's VISIBLE
    /// positions, which is what the server maps onto its stored rows.
    pub fn playlist_cut(&self, id: &str, indexes: &[u32]) -> Result<Order, CoreError> {
        let state = open_state(self, id)?;
        if indexes.is_empty() {
            return Err(CoreError::Invalid {
                detail: "aucun titre à retirer".to_owned(),
            });
        }
        let before = state.rows();
        let mut sorted: Vec<usize> = indexes
            .iter()
            .map(|index| *index as usize)
            .filter(|index| *index < before.len())
            .collect();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.is_empty() {
            return Err(CoreError::Invalid {
                detail: "aucun titre à retirer".to_owned(),
            });
        }
        let removed: Vec<(usize, Row)> = sorted
            .iter()
            .map(|index| (*index, before[*index].clone()))
            .collect();
        let mut rows = Vec::clone(&before);
        for index in sorted.iter().rev() {
            rows.remove(*index);
        }
        reindex(&mut rows);
        state.set_rows(rows);
        state.remember_undo(Undo::Cut { removed });
        Ok(Order {
            rows: state.rows(),
            version: state.version(),
            conflict: None,
        })
    }

    /// `DELETE /api/playlist/<id>/tracks` — one request for those indexes; a
    /// failure puts the removed rows back where they were.
    pub async fn playlist_save_cut(&self, id: &str, indexes: &[u32]) -> Result<Write, CoreError> {
        let state = open_state(self, id)?;
        let mut sorted: Vec<usize> = indexes.iter().map(|index| *index as usize).collect();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.is_empty() {
            return Err(CoreError::Invalid {
                detail: "aucun titre à retirer".to_owned(),
            });
        }
        match wire::delete_indexes(self, id, &sorted, state.version()).await {
            Ok(counters) => {
                state.adopt_counters(counters.version, counters.nb_tracks);
                state.remember_undo(Undo::None);
                Ok(Write {
                    data: state.data(),
                    conflict: None,
                })
            }
            Err(failure) => match failure.conflict {
                Some(scope) => {
                    resync(self, &state, scope).await?;
                    Ok(Write {
                        data: state.data(),
                        conflict: Some(scope),
                    })
                }
                None => {
                    state.undo();
                    Err(failure.cause)
                }
            },
        }
    }

    /// The rows of an open playlist, as they stand in memory: **0 requests**.
    /// What a screen asks after a write it does not own (a failed save that put
    /// the list back).
    pub fn playlist_rows(&self, id: &str) -> Option<Arc<Vec<Row>>> {
        held(self, id).map(|state| state.rows())
    }

    /// `PATCH /api/playlist/<id>` — the title and/or the description, optimistic
    /// and version-guarded (`Playlist.svelte#saveMeta`).
    pub async fn playlist_edit(
        &self,
        id: &str,
        title: Option<String>,
        description: Option<String>,
    ) -> Result<Write, CoreError> {
        let state = open_state(self, id)?;
        let title_text = title.as_deref().map(str::trim);
        if let Some(title) = title_text
            && (title.is_empty() || title.chars().count() > TEXT_MAX)
        {
            return Err(CoreError::Invalid {
                detail: "nom ou description invalide".to_owned(),
            });
        }
        if let Some(description) = description.as_deref()
            && description.trim().chars().count() > TEXT_MAX
        {
            return Err(CoreError::Invalid {
                detail: "nom ou description invalide".to_owned(),
            });
        }
        let previous = state.meta();
        {
            let mut meta = state.meta.write().unwrap_or_else(|err| err.into_inner());
            if let Some(title) = title_text {
                meta.title = title.to_owned();
            }
            if let Some(description) = description.as_deref() {
                meta.description = description.trim().to_owned();
            }
        }
        state.bump();
        let description_text = description.as_deref().map(str::trim);
        match wire::patch_meta(self, id, title_text, description_text, previous.version).await {
            Ok(counters) => {
                state.adopt_counters(counters.version, None);
                Ok(Write {
                    data: state.data(),
                    conflict: None,
                })
            }
            Err(failure) => match failure.conflict {
                Some(scope) => {
                    resync(self, &state, scope).await?;
                    Ok(Write {
                        data: state.data(),
                        conflict: Some(scope),
                    })
                }
                None => {
                    *state.meta.write().unwrap_or_else(|err| err.into_inner()) = previous;
                    state.bump();
                    Err(failure.cause)
                }
            },
        }
    }

    /// `DELETE /api/playlist/<id>` — terminal, no version (`#deletePlaylist`).
    pub async fn playlist_delete(&self, id: &str) -> Result<(), CoreError> {
        wire::delete_playlist(self, id).await.map_err(cause_of)?;
        forget(self, id);
        Ok(())
    }

    /// `POST /api/playlist/<id>/leave` — stop being a member; the owner cannot.
    pub async fn playlist_leave(&self, id: &str) -> Result<(), CoreError> {
        wire::leave_playlist(self, id).await.map_err(cause_of)?;
        forget(self, id);
        Ok(())
    }

    /// `GET /api/playlist/<id>/members` — who is on it and what the caller may
    /// do. One request; the sheet paints the page's own list until then.
    pub async fn playlist_members(&self, id: &str) -> Result<MembersWrite, CoreError> {
        let state = open_state(self, id)?;
        let answer = wire::read_members(self, id).await;
        finish_members(self, id, &state, answer).await
    }

    /// `POST /api/playlist/<id>/members` — invite one EXACT account name.
    pub async fn playlist_invite(
        &self,
        id: &str,
        username: &str,
        role: Role,
    ) -> Result<MembersWrite, CoreError> {
        let state = open_state(self, id)?;
        let name = username.trim();
        if name.is_empty() {
            return Err(CoreError::Invalid {
                detail: "indiquez un nom d'utilisateur".to_owned(),
            });
        }
        let answer = wire::post_member(self, id, name, role, Some(state.members_version())).await;
        finish_members(self, id, &state, answer).await
    }

    /// `PATCH /api/playlist/<id>/members/<user>` — change somebody's role.
    pub async fn playlist_set_role(
        &self,
        id: &str,
        user_id: &str,
        role: Role,
    ) -> Result<MembersWrite, CoreError> {
        let state = open_state(self, id)?;
        let answer =
            wire::patch_member(self, id, user_id, role, Some(state.members_version())).await;
        finish_members(self, id, &state, answer).await
    }

    /// `DELETE /api/playlist/<id>/members/<user>` — remove somebody.
    pub async fn playlist_remove_member(
        &self,
        id: &str,
        user_id: &str,
    ) -> Result<MembersWrite, CoreError> {
        let state = open_state(self, id)?;
        let answer = wire::delete_member(self, id, user_id, Some(state.members_version())).await;
        finish_members(self, id, &state, answer).await
    }

    /// `GET /api/playlist/<id>/share` — the live public link, if any.
    pub async fn playlist_share(&self, id: &str) -> Result<Option<ShareLink>, CoreError> {
        wire::read_share(self, id).await.map_err(cause_of)
    }

    /// `POST /api/playlist/<id>/share` — mint the link (or hand back the live
    /// one; one per playlist is enough).
    pub async fn playlist_create_share(
        &self,
        id: &str,
        ttl_s: Option<u32>,
    ) -> Result<ShareLink, CoreError> {
        wire::create_share(self, id, ttl_s).await.map_err(cause_of)
    }

    /// `DELETE /api/playlist/<id>/share` — dead on the very next public request.
    pub async fn playlist_revoke_share(&self, id: &str) -> Result<(), CoreError> {
        wire::revoke_share(self, id).await.map_err(cause_of)
    }

    /// `GET /api/export/formats` — what this server can produce (ffmpeg may be
    /// missing, so the list is the server's, never a constant).
    pub async fn export_formats(&self) -> Result<ExportFormats, CoreError> {
        wire::read_formats(self).await.map_err(cause_of)
    }

    /// `GET /api/services` — the linked services and the list formats.
    pub async fn export_services(&self) -> Result<Services, CoreError> {
        wire::read_services(self).await.map_err(cause_of)
    }

    /// `POST /api/services/<service>/export` — match the list on the other
    /// service and create it there; the job is followed by [`Self::export_job`].
    pub async fn export_to_service(
        &self,
        service: &str,
        kind: &str,
        id: &str,
        name: &str,
        public: bool,
    ) -> Result<String, CoreError> {
        wire::start_export(self, service, kind, id, name, public)
            .await
            .map_err(cause_of)
    }

    /// `GET /api/services/export/<job>` — one poll.
    pub async fn export_job(&self, job: &str) -> Result<ExportJob, CoreError> {
        wire::read_export_job(self, job).await.map_err(cause_of)
    }

    /// `GET /api/services/list/<kind>/<id>?fmt=` — the LIST file (CSV, XSPF,
    /// plain text), small enough to hand to the share sheet as text.
    pub async fn export_list_file(
        &self,
        kind: &str,
        id: &str,
        fmt: &str,
    ) -> Result<String, CoreError> {
        wire::read_list_file(self, kind, id, fmt)
            .await
            .map_err(cause_of)
    }

    /// `GET /api/export/<kind>/<id>?fmt=` into `destination` — the audio ZIP,
    /// streamed on the `Bulk` lane so an export never competes with what the
    /// person is listening to (rule 1). Answers the bytes written.
    pub async fn export_archive(
        &self,
        kind: &str,
        id: &str,
        fmt: &str,
        destination: std::path::PathBuf,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<u64, CoreError> {
        wire::download_export(self, kind, id, fmt, destination, cancel)
            .await
            .map_err(cause_of)
    }

    /// `POST /api/import/playlist` — read and match somebody else's shared link
    /// as a server job (rule 2): answers the job's id, never the answer.
    pub async fn playlist_import_start(&self, url: &str) -> Result<String, CoreError> {
        wire::start_import(self, url).await.map_err(cause_of)
    }

    /// `GET /api/import/playlist/<job>` — progress, the per-line causes, and the
    /// items once the read is complete.
    pub async fn playlist_import_status(&self, job: &str) -> Result<ImportStatus, CoreError> {
        wire::read_import(self, job).await.map_err(cause_of)
    }

    /// `POST /api/import/playlist/<job>/save` — keep the matched lines as a local
    /// playlist. `keep` names the item indexes, `None` keeps every matched line.
    pub async fn playlist_import_save(
        &self,
        job: &str,
        title: &str,
        keep: Option<Vec<u32>>,
    ) -> Result<ImportStatus, CoreError> {
        wire::save_import(self, job, title, keep)
            .await
            .map_err(cause_of)
    }

    /// `GET /api/shared/playlist/<token>` — the public, read-only page. No
    /// session is needed; the token IS the capability.
    pub async fn shared_playlist(&self, token: &str) -> Result<SharedData, CoreError> {
        wire::read_shared(self, token).await.map_err(cause_of)
    }

    /// The public page this device kept, and nothing else: **0 requests**.
    pub fn cached_shared_playlist(&self, token: &str) -> Option<SharedData> {
        wire::cached_shared(self, token)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{PlaylistServer, TRACKS};
    use super::*;

    /// What the screen costs, held to the numbers the lot's PR claims:
    /// **one** request for a 4 000-row page, and the whole tracklist in it.
    #[tokio::test]
    async fn a_four_thousand_row_playlist_opens_with_one_request() {
        let server = PlaylistServer::start(TRACKS);
        let core = server.core("one-request").await;

        let data = core.playlist(server.id()).await.expect("the page");
        assert_eq!(data.rows.len(), TRACKS);
        assert_eq!(data.meta.nb_tracks, TRACKS as u32);
        assert_eq!(data.role, Role::Owner);
        assert!(data.editable);
        assert_eq!(
            server.gets(&server.path()),
            1,
            "a 4 000-row playlist is ONE request, never a page loop"
        );
        assert_eq!(data.rows[0].title, "Titre 0");
        assert_eq!(data.rows[0].position, 0);
        assert_eq!(data.rows[TRACKS - 1].position, (TRACKS - 1) as u32);

        // The meter is the core's own answer: one request, the server's bytes.
        let stats: Vec<_> = core
            .stats()
            .into_iter()
            .filter(|row| row.tag == TAG_PLAYLIST)
            .collect();
        assert_eq!(stats.len(), 1, "one tag, one row");
        assert_eq!(stats[0].requests, 1);
        assert!(
            stats[0].bytes_in > 4_000 * 100,
            "a 4 000-row page carries its rows: {} bytes",
            stats[0].bytes_in
        );
    }

    /// The open page is kept in memory: a second read costs one request (the
    /// SPA re-reads too), and the warm copy on disk costs none.
    #[tokio::test]
    async fn the_page_is_kept_on_disk_and_reads_warm_with_no_request() {
        let server = PlaylistServer::start(64);
        let core = server.core("warm").await;
        core.playlist(server.id()).await.expect("the first read");
        let before = server.accepted();

        // Offline: the kept copy paints, and not one socket is opened.
        core.set_network(crate::core::NetworkState::None);
        let kept = core.cached_playlist(server.id()).expect("the kept page");
        assert_eq!(kept.rows.len(), 64);
        assert_eq!(kept.meta.title, "Playlist de 64 titres");
        assert_eq!(server.accepted(), before, "the kept copy opened no socket");

        // A page this device never opened has no copy: a cause, not a blank page.
        assert!(core.cached_playlist("jamais-ouverte").is_none());
        assert_eq!(
            core.playlist("jamais-ouverte").await.expect_err("offline"),
            CoreError::Offline
        );
        assert_eq!(server.accepted(), before, "still not one socket");
    }

    /// The acceptance's reorder: the move answers in memory (no request), and a
    /// `409` from another editor is survived — the server's order replaces ours
    /// after ONE re-read, and the screen is told which half moved.
    #[tokio::test]
    async fn a_reorder_answers_at_once_and_survives_a_409_with_one_re_read() {
        let server = PlaylistServer::start(64);
        let core = server.core("reorder").await;
        core.playlist(server.id()).await.expect("the page");
        let gets = server.gets(&server.path());
        let puts = server.mutations("PUT", &server.path());

        // The frame's answer: row 0 goes to position 2, nothing is sent.
        let order = core.playlist_move(server.id(), 0, 2).expect("the move");
        assert_eq!(
            server.mutations("PUT", &server.path()),
            puts,
            "a move sends nothing"
        );
        assert_eq!(server.gets(&server.path()), gets);
        assert_eq!(order.rows[0].title, "Titre 1");
        assert_eq!(
            order.rows[2].title, "Titre 0",
            "the dragged row landed there"
        );
        assert_eq!(order.rows[2].position, 2, "positions are renumbered");

        // Another editor saved first: our version is stale from now on.
        server.bump_version();

        // The network half: ONE PUT, a 409, ONE re-read — never a replay.
        let outcome = core
            .playlist_save_order(server.id())
            .await
            .expect("the answer");
        assert_eq!(
            outcome.conflict,
            Some(ConflictScope::Content),
            "the race is named"
        );
        assert_eq!(
            server.mutations("PUT", &server.path()),
            puts + 1,
            "one PUT, no replay"
        );
        assert_eq!(
            server.gets(&server.path()),
            gets + 1,
            "a 409 costs exactly ONE re-read, not another PUT"
        );
        // The server's order is what the person sees now (it swapped 0 and 1).
        assert_eq!(
            outcome.rows[0].title, "Titre 1",
            "the server's order replaced the stale optimistic one"
        );
        assert!(outcome.version > 1, "the adopted version is the server's");
    }

    /// The other half of the same machine: a reorder with the right version is
    /// one `PUT`, and the version it answers is what the next write carries.
    #[tokio::test]
    async fn a_reorder_with_the_right_version_is_one_put_and_adopts_the_answer() {
        let server = PlaylistServer::start(8);
        let core = server.core("reorder-ok").await;
        let page = core.playlist(server.id()).await.expect("the page");
        core.playlist_move(server.id(), 0, 1).expect("the move");
        let outcome = core.playlist_save_order(server.id()).await.expect("saved");
        assert_eq!(outcome.conflict, None);
        assert_eq!(server.mutations("PUT", &server.path()), 1);
        let open = core.playlist_state(server.id()).expect("open");
        assert_eq!(open.version(), outcome.version);
        assert!(
            outcome.version > page.meta.version,
            "a write bumps the version"
        );
        assert_eq!(
            server.order()[..2],
            ["t1", "t0"],
            "the server has our order"
        );
        assert_eq!(server.last_body()["tracks"][0], "t1");
    }

    /// 4 000 rows: the move and the list the screen re-windows stay inside the
    /// frame — measured, with the work counted and the one wall-clock number the
    /// acceptance names (16.7 ms) beside the assertion.
    #[test]
    fn a_four_thousand_row_move_stays_inside_a_frame() {
        let server = PlaylistServer::start(TRACKS);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let core = runtime.block_on(server.core("reorder-budget"));
        let data = runtime
            .block_on(core.playlist(server.id()))
            .expect("the page");
        assert_eq!(data.rows.len(), TRACKS);

        let started = std::time::Instant::now();
        let order = core
            .playlist_move(server.id(), 12, 3_998)
            .expect("the move");
        let elapsed = started.elapsed();
        assert_eq!(order.rows[3_998].title, "Titre 12");
        assert_eq!(order.rows[3].title, "Titre 3", "the rest kept their places");
        assert_eq!(order.rows[3_998].position, 3_998);
        println!(
            "MB9 measure: playlist_move over {} rows = {:.3} ms, {} rows rebuilt",
            order.rows.len(),
            elapsed.as_secs_f64() * 1_000.0,
            order.rows.len()
        );
        assert!(
            elapsed < std::time::Duration::from_millis(16),
            "a move must answer inside a frame (16.7 ms): {elapsed:?}"
        );
    }

    /// An add is optimistic and appended at once, in ONE request; a `409` re-reads
    /// the page and says so; a dead link puts the list back and is a cause.
    #[tokio::test]
    async fn an_add_is_optimistic_and_a_failure_puts_the_list_back() {
        let server = PlaylistServer::start(4);
        let core = server.core("add").await;
        let page = core.playlist(server.id()).await.expect("the page");
        let added: Vec<Row> = (0..2).map(|n| server.row(100 + n)).collect();

        // The instant half: the rows are in the list, and nothing was sent.
        let order = core.playlist_append(server.id(), &added).expect("appended");
        assert_eq!(order.rows.len(), 6);
        assert_eq!(order.rows[5].title, "Titre 101");
        assert_eq!(
            server.mutations("POST", &server.path()),
            0,
            "a frame, no request"
        );

        let write = core.playlist_save_append(server.id()).await.expect("saved");
        assert_eq!(write.data.rows.len(), 6);
        assert!(write.data.meta.version > page.meta.version);
        assert_eq!(
            server.mutations("POST", &server.path()),
            1,
            "both refs in ONE request"
        );
        assert_eq!(
            server.last_body()["tracks"].as_array().map(Vec::len),
            Some(2),
            "the body names them both"
        );

        // Another editor moved the list: the 409 re-reads and answers the truth.
        server.bump_version();
        core.playlist_append(server.id(), &[server.row(200)])
            .expect("appended");
        let raced = core
            .playlist_save_append(server.id())
            .await
            .expect("an answer");
        assert_eq!(raced.conflict, Some(ConflictScope::Content));
        assert_eq!(
            raced.data.rows.len(),
            6,
            "the server never took the append; its page is what we show"
        );
        assert!(
            !raced.data.rows.iter().any(|row| row.title == "Titre 200"),
            "the raced row was never stored"
        );

        // A dead link is a real failure: the optimistic rows are gone, with a cause.
        core.set_network(crate::core::NetworkState::None);
        core.playlist_append(server.id(), &added).expect("appended");
        assert!(core.playlist_save_append(server.id()).await.is_err());
        assert_eq!(
            core.playlist_state(server.id()).expect("open").rows().len(),
            6,
            "the appended rows are gone"
        );
    }

    /// A remove is by the index the person tapped, instantaneous in memory, and a
    /// failed save puts the row back exactly where it was.
    #[tokio::test]
    async fn a_remove_takes_the_index_the_person_tapped() {
        let server = PlaylistServer::start(6);
        let core = server.core("remove").await;
        let page = core.playlist(server.id()).await.expect("the page");
        let title = page.rows[2].title.clone();

        let order = core.playlist_cut(server.id(), &[2]).expect("cut");
        assert_eq!(order.rows.len(), 5);
        assert!(!order.rows.iter().any(|row| row.title == title));
        assert_eq!(order.rows[2].title, "Titre 3", "the next row slid up");
        assert_eq!(server.mutations("DELETE", &server.path()), 0);

        let write = core
            .playlist_save_cut(server.id(), &[2])
            .await
            .expect("saved");
        assert_eq!(write.data.rows.len(), 5);
        assert_eq!(server.last_body()["indexes"][0], 2);

        // Offline: the row comes back at its own index.
        core.set_network(crate::core::NetworkState::None);
        core.playlist_cut(server.id(), &[0]).expect("cut");
        assert!(core.playlist_save_cut(server.id(), &[0]).await.is_err());
        let rows = core.playlist_rows(server.id()).expect("open");
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].title, "Titre 0", "the removed row is back first");
    }

    /// Renaming is optimistic, version-guarded, and refuses what the server would
    /// refuse before spending a request (`#saveMeta`'s 400).
    #[tokio::test]
    async fn a_rename_is_optimistic_and_guarded() {
        let server = PlaylistServer::start(2);
        let core = server.core("rename").await;
        core.playlist(server.id()).await.expect("the page");
        let write = core
            .playlist_edit(
                server.id(),
                Some("Nouveau nom".into()),
                Some("Une description".into()),
            )
            .await
            .expect("renamed");
        assert_eq!(write.data.meta.title, "Nouveau nom");
        assert_eq!(write.data.meta.description, "Une description");
        assert_eq!(server.mutations("PATCH", &server.path()), 1);

        let before = core.playlist_state(server.id()).expect("open").meta().title;
        assert!(
            core.playlist_edit(server.id(), Some("   ".into()), None)
                .await
                .is_err()
        );
        assert_eq!(
            core.playlist_state(server.id()).expect("open").meta().title,
            before,
            "an invalid name never reaches the state"
        );
        let long = "x".repeat(TEXT_MAX + 1);
        assert!(
            core.playlist_edit(server.id(), Some(long), None)
                .await
                .is_err()
        );
    }

    /// The collaborators: the owner invites by exact name, a stale
    /// `members_version` re-reads the members only (never the page), and a
    /// viewer's invite comes back as a refreshed, denied answer.
    #[tokio::test]
    async fn the_members_are_role_driven_and_a_409_re_reads_the_members_only() {
        let server = PlaylistServer::start(4);
        let core = server.core("members").await;
        let page = core.playlist(server.id()).await.expect("the page");
        assert_eq!(page.members.len(), 1);
        assert_eq!(page.members[0].role, Role::Owner);

        let write = core
            .playlist_invite(server.id(), "camille", Role::Editor)
            .await
            .expect("invited");
        assert_eq!(write.members.len(), 2);
        assert_eq!(write.conflict, None);
        assert_eq!(server.mutations("POST", &server.path()), 1);

        // Somebody else refreshed the members: ONE members GET, no page re-read.
        server.bump_members();
        let pages = server.gets(&server.path());
        let members = server.gets(&server.members_path());
        let race = core
            .playlist_invite(server.id(), "june", Role::Viewer)
            .await
            .expect("the cause, not an error");
        assert_eq!(race.conflict, Some(ConflictScope::Members));
        assert_eq!(server.gets(&server.path()), pages, "no page re-read");
        assert_eq!(server.gets(&server.members_path()), members + 1);
        assert_eq!(
            race.members.len(),
            3,
            "the server's list, after the re-read"
        );

        // A viewer's invite is refused: the capabilities are refreshed, not faked.
        server.set_role("viewer");
        let denied = core
            .playlist_invite(server.id(), "sam", Role::Viewer)
            .await
            .expect("a refusal is an answer");
        assert!(denied.denied);
        assert!(!denied.can_edit);
        assert!(!denied.can_manage);
    }

    /// The public link: read, mint, revoke — and the public page itself, whose
    /// unarchived rows are listed but not playable. The page is kept on disk.
    #[tokio::test]
    async fn the_public_link_is_minted_revoked_and_the_page_lists_archived_only() {
        let server = PlaylistServer::start(6);
        let core = server.core("share").await;
        core.playlist(server.id()).await.expect("the page");
        assert!(
            core.playlist_share(server.id())
                .await
                .expect("read")
                .is_none()
        );

        let link = core
            .playlist_create_share(server.id(), Some(86_400))
            .await
            .expect("minted");
        assert!(link.url.starts_with("/pl/"), "{}", link.url);
        assert_eq!(link.expires.as_deref(), Some("2026-12-01T00:00:00"));
        let again = core.playlist_share(server.id()).await.expect("read");
        assert_eq!(
            again.expect("a live link").token,
            link.token,
            "one link per playlist is enough"
        );

        let page = core
            .shared_playlist(&link.token)
            .await
            .expect("the public page");
        assert_eq!(page.rows.len(), 6);
        assert_eq!(page.rows[0].mid.as_deref(), Some("t0"));
        assert!(page.rows[0].streamable);
        assert!(
            !page.rows[4].streamable,
            "an unarchived row is listed, not playable"
        );
        assert_eq!(page.playable().len(), 4);
        assert_eq!(page.header.title, "Partagée");

        // The page is kept: a warm open costs nothing.
        let reads = server.gets("/api/shared/");
        assert!(core.cached_shared_playlist(&link.token).is_some());
        assert_eq!(server.gets("/api/shared/"), reads, "0 requests warm");
        assert!(core.cached_shared_playlist("un-autre-jeton").is_none());

        core.playlist_revoke_share(server.id())
            .await
            .expect("revoked");
        assert_eq!(server.mutations("DELETE", &server.path()), 1);
    }

    /// The export sheet's four questions: the formats, the services, the job's
    /// progress with its missing titles, and the list file.
    #[tokio::test]
    async fn the_export_asks_what_the_sheet_draws() {
        let server = PlaylistServer::start(4);
        let core = server.core("export").await;
        core.playlist(server.id()).await.expect("the page");

        let formats = core.export_formats().await.expect("formats");
        assert_eq!(formats.default, "opus320");
        assert_eq!(formats.formats.len(), 2);
        let services = core.export_services().await.expect("services");
        assert_eq!(services.services.len(), 2);
        assert!(services.services[0].linked);
        assert!(services.services[1].file_only);
        assert_eq!(services.formats[0].id, "csv");

        let job = core
            .export_to_service("spotify", "playlist", server.id(), "Ma playlist", false)
            .await
            .expect("the job");
        let first = core.export_job(&job).await.expect("poll");
        assert!(first.running);
        assert_eq!(first.percent(), 0);
        server.finish_export();
        let done = core.export_job(&job).await.expect("poll");
        assert!(!done.running);
        assert_eq!(done.matched, 3);
        assert_eq!(done.missing_count, 1);
        assert_eq!(done.missing[0].title, "Absent");
        assert!(
            done.url
                .as_deref()
                .unwrap_or_default()
                .starts_with("https://open.spotify")
        );

        let file = core
            .export_list_file("playlist", server.id(), "csv")
            .await
            .expect("the list");
        assert!(file.contains("Artiste 1 - Titre 1"), "{file}");
    }

    /// The import of somebody else's link: a job with progress, a per-line cause,
    /// the matched titles, and keeping it (`routes/Import.svelte`).
    #[tokio::test]
    async fn the_import_reports_progress_causes_and_saves() {
        let server = PlaylistServer::start(2);
        let core = server.core("import").await;
        let job = core
            .playlist_import_start("https://open.spotify.com/playlist/abc")
            .await
            .expect("started");
        assert_eq!(job, "job-import");
        let running = core.playlist_import_status(&job).await.expect("status");
        assert_eq!(running.state, "matching");
        assert!(running.running());
        assert_eq!(running.total, 4);
        assert_eq!(running.done, 1);
        assert_eq!(running.items.len(), 0, "no item list while it grows");

        server.finish_import();
        let done = core.playlist_import_status(&job).await.expect("done");
        assert_eq!(done.state, "done");
        assert!(!done.running());
        assert_eq!(done.items.len(), 4);
        assert_eq!(done.rows().len(), 3, "three lines found a sound");
        assert_eq!(done.counts[0], ("deezer".to_owned(), 2));
        let unsure = done
            .items
            .iter()
            .filter(|item| item.track.is_some() && !item.sure)
            .count();
        assert_eq!(unsure, 1, "one line was matched closely, not surely");
        assert!(
            done.items.iter().any(|item| item.track.is_none()),
            "one line has no safe version, and says so"
        );
        assert_eq!(done.source_label, "Spotify");

        let saved = core
            .playlist_import_save(&job, "Ma playlist importée", None)
            .await
            .expect("saved");
        assert_eq!(saved.state, "saved");
        assert_eq!(saved.playlist_id.as_deref(), Some("playlist-importee"));
    }

    /// The counted bytes of what the lot holds, and the LRU's cap: a third page
    /// drops the first, so a 4 000-row playlist cannot pile up behind a screen.
    #[tokio::test]
    async fn the_open_pages_count_their_bytes_and_the_cap_drops_the_oldest() {
        let server = PlaylistServer::start(TRACKS);
        let core = server.core("memory").await;
        let data = core.playlist(server.id()).await.expect("the page");
        let state = core.playlist_state(server.id()).expect("open");
        let counted = state.memory_bytes();
        assert!(
            counted >= data.memory_bytes(),
            "the page counts at least its rows: {counted} >= {}",
            data.memory_bytes()
        );
        assert!(
            counted > 4_000 * 100,
            "4 000 rows count their own strings: {counted}"
        );

        // A second and a third page on THIS install: the LRU keeps two, so the
        // oldest is dropped — and re-read from the server when asked for again.
        core.playlist("pl-deux").await.expect("a second page");
        core.playlist("pl-trois").await.expect("a third page");
        assert!(
            core.playlist_state(server.id()).is_none(),
            "the cap drops the oldest page: {} open",
            open_pages()
        );
        let gets = server.gets(&server.path());
        core.playlist(server.id()).await.expect("back to the first");
        assert!(
            server.gets(&server.path()) > gets,
            "a dropped page is re-read, never served from nowhere"
        );
    }

    /// A playlist of somebody else's is read-only, and a move says so instead of
    /// pretending: the viewer's answer is the SPA's "Lecture seule" block.
    #[tokio::test]
    async fn a_read_only_playlist_refuses_the_move_with_a_cause() {
        let server = PlaylistServer::start(4);
        server.set_role("viewer");
        let core = server.core("read-only").await;
        let data = core.playlist(server.id()).await.expect("the page");
        assert!(!data.editable);
        assert_eq!(data.role, Role::Viewer);
        match core.playlist_move(server.id(), 0, 2) {
            Err(CoreError::Server { status, .. }) => assert_eq!(status, 403),
            other => panic!("a viewer's move is refused, not applied: {other:?}"),
        }
    }
}
