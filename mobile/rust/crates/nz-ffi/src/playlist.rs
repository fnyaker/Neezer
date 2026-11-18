//! The playlist screen's boundary: one page as a record with a windowed
//! `ListHandle`, its writes, its people, its public link, its export and the
//! import of somebody else's link.
//!
//! Owned by **MB9** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this file — its own `#[uniffi::export] impl` blocks, its own
//! records — and edits no other file of the crate.
//!
//! # The shape of this boundary
//!
//! - **A 4 000-row playlist crosses as a `ListHandle`, never whole.** The rows
//!   live in `nz-core::playlist`; the screen asks `window(start, count)` for the
//!   pages it is about to paint (rule 4). [`PlaylistData::tracks`] is that
//!   handle, and every write answers a FRESH one, so a reorder is a swap and a
//!   removal is a page, never a 4 000-record copy.
//! - **Reads are `async`** (cancellable, one request on the `User` lane):
//!   [`Screens::cached_playlist`] is the disk-only copy (`Option`), then
//!   [`Screens::playlist`] replaces it. The recommendations are their OWN call,
//!   because the SPA asks them after the list is on screen.
//! - **Writes are optimistic** (`nz-core` edits its state before the request):
//!   a failure rolls back and is an [`NzError`] the screen turns into a toast
//!   with « Réessayer »; a `409` is not an error — [`PlaylistWrite::conflict`]
//!   says which half moved, and the answer already carries the server's truth.
//! - **A reorder has two beats, as the SPA does**: [`Screens::playlist_move`] is
//!   synchronous, in-memory, and answers in the frame;
//!   [`Screens::playlist_save_order`] sends the one coalesced `PUT` and survives
//!   a `409` by re-reading (never replaying).
//! - **No French copy here.** The screen writes its sentences from `conflict`,
//!   `state` and `denied` (the FFI's only French is `nz_error_text`).

use std::sync::Arc;

use nz_core::core as core_type;
use nz_core::playlist as core;

use crate::core::Screens;
use crate::errors::NzError;
use crate::lists::{ListHandle, TrackRow};

/// Who the caller is on this playlist (`role_for` on the wire).
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistRole {
    Owner,
    Editor,
    Viewer,
    /// A server admin: an owner's rights on anybody's playlist.
    Admin,
    /// A reader of the public link.
    Public,
}

impl From<core::Role> for PlaylistRole {
    fn from(role: core::Role) -> Self {
        match role {
            core::Role::Owner => PlaylistRole::Owner,
            core::Role::Editor => PlaylistRole::Editor,
            core::Role::Viewer => PlaylistRole::Viewer,
            core::Role::Admin => PlaylistRole::Admin,
            core::Role::Public => PlaylistRole::Public,
        }
    }
}

impl From<PlaylistRole> for core::Role {
    fn from(role: PlaylistRole) -> Self {
        match role {
            PlaylistRole::Owner => core::Role::Owner,
            PlaylistRole::Editor => core::Role::Editor,
            PlaylistRole::Viewer => core::Role::Viewer,
            PlaylistRole::Admin => core::Role::Admin,
            PlaylistRole::Public => core::Role::Public,
        }
    }
}

/// Which half of a playlist a `409` was about; the screen says so and offers the
/// person to drag again from the server's order.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistConflict {
    Content,
    Members,
}

impl From<core::ConflictScope> for PlaylistConflict {
    fn from(scope: core::ConflictScope) -> Self {
        match scope {
            core::ConflictScope::Content => PlaylistConflict::Content,
            core::ConflictScope::Members => PlaylistConflict::Members,
        }
    }
}

/// The header the page draws above the list.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistMeta {
    pub id: String,
    pub deezer_id: Option<String>,
    pub title: String,
    pub description: String,
    /// A relative server path (`/api/cover/<id>`), or `None` for the grooves.
    pub cover: Option<String>,
    pub owner: String,
    /// The caller's visible row count.
    pub nb_tracks: u32,
    /// Somebody else's private upload: counted, never listed, and only for a
    /// manager.
    pub private_tracks: u32,
    pub is_favorite: bool,
    /// The optimistic-concurrency token every write carries.
    pub version: u32,
    pub members_version: u32,
    pub last_edited_by: Option<String>,
    /// ISO instant, or `None`.
    pub last_edited_at: Option<String>,
}

impl From<core::Meta> for PlaylistMeta {
    fn from(meta: core::Meta) -> Self {
        PlaylistMeta {
            id: meta.id,
            deezer_id: meta.deezer_id,
            title: meta.title,
            description: meta.description,
            cover: meta.cover,
            owner: meta.owner,
            nb_tracks: meta.nb_tracks,
            private_tracks: meta.private_tracks,
            is_favorite: meta.is_favorite,
            version: meta.version,
            members_version: meta.members_version,
            last_edited_by: meta.last_edited_by,
            last_edited_at: meta.last_edited_at,
        }
    }
}

/// One collaborator, as the sheet draws them.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistMember {
    pub id: String,
    pub name: String,
    pub role: PlaylistRole,
    pub added: Option<String>,
}

impl From<core::MemberRow> for PlaylistMember {
    fn from(member: core::MemberRow) -> Self {
        PlaylistMember {
            id: member.id,
            name: member.name,
            role: member.role.into(),
            added: member.added,
        }
    }
}

/// Where the suggestions stand (`Playlist.svelte#recsState`).
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecommendationsState {
    Idle,
    Loading,
    Ready,
    /// The read failed: the section says so with « Réessayer » (rule 3).
    Failed,
}

impl From<core::RecState> for RecommendationsState {
    fn from(state: core::RecState) -> Self {
        match state {
            core::RecState::Idle => RecommendationsState::Idle,
            core::RecState::Loading => RecommendationsState::Loading,
            core::RecState::Ready => RecommendationsState::Ready,
            core::RecState::Failed => RecommendationsState::Failed,
        }
    }
}

/// The bottom of the page: the library's own suggestions, at most twenty.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistRecommendations {
    pub state: RecommendationsState,
    pub tracks: Vec<TrackRow>,
}

impl From<core::Recommendations> for PlaylistRecommendations {
    fn from(recs: core::Recommendations) -> Self {
        PlaylistRecommendations {
            state: recs.state.into(),
            tracks: recs.rows.into_iter().map(TrackRow::from).collect(),
        }
    }
}

/// The whole page, as the screen draws it. No `Debug`: the track list is a
/// `ListHandle` whose own state is behind a lock the boundary has no reason to
/// print (and UniFFI does not need it).
#[derive(uniffi::Record, Clone)]
pub struct PlaylistData {
    pub meta: PlaylistMeta,
    pub role: PlaylistRole,
    /// Whether the person may add, remove and reorder.
    pub editable: bool,
    /// Whether they may rename, delete and share.
    pub can_manage: bool,
    pub admin: bool,
    pub members: Vec<PlaylistMember>,
    /// The tracklist, windowed (`ListWindow` over `len` + `window`).
    pub tracks: Arc<ListHandle>,
    pub recommendations: PlaylistRecommendations,
    /// More than one member: the only case worth the live probe.
    pub collaborative: bool,
}

/// What a write answers: the fresh state, and whether the world had moved.
#[derive(uniffi::Record, Clone)]
pub struct PlaylistWrite {
    pub data: PlaylistData,
    /// `Some` when the write lost its race: the state above is the server's.
    pub conflict: Option<PlaylistConflict>,
}

/// A reorder's answer: the new order as a handle, and where the server stands.
#[derive(uniffi::Record, Clone)]
pub struct PlaylistOrder {
    pub tracks: Arc<ListHandle>,
    pub version: u32,
    /// `Some` when the `PUT` met a `409`: the rows above are the server's.
    pub conflict: Option<PlaylistConflict>,
}

/// The collaborators and what the caller may do (`PlaylistMembersSheet`).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistMembersWrite {
    pub members: Vec<PlaylistMember>,
    pub members_version: u32,
    pub role: PlaylistRole,
    pub can_edit: bool,
    pub can_manage: bool,
    pub admin: bool,
    pub conflict: Option<PlaylistConflict>,
    /// The right went away (`403`): the sheet closes and says so.
    pub denied: bool,
}

/// The public read-only link of one playlist.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistShareLink {
    pub token: String,
    /// The path under the server's origin (`/pl/<token>`).
    pub url: String,
    pub created: Option<String>,
    pub expires: Option<String>,
}

/// One format the bulk (ZIP) export can produce.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistExportFormat {
    pub id: String,
    pub label: String,
    pub ext: String,
}

/// `/api/export/formats`: the formats, and the one the sheet starts on.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistExportFormats {
    pub default: String,
    pub formats: Vec<PlaylistExportFormat>,
}

/// One service the export can reach.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistService {
    pub id: String,
    pub name: String,
    /// The credential is in place: an export can run.
    pub ready: bool,
    pub linked: bool,
    pub account: Option<String>,
    /// A service with no door: it exports a list FILE instead.
    pub file_only: bool,
}

/// `/api/services`: the services and the list formats.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistServices {
    pub services: Vec<PlaylistService>,
    pub formats: Vec<PlaylistExportFormat>,
}

/// A title the export could not find on the other service.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistMissingTitle {
    pub title: String,
    pub artist: String,
}

/// One export job, as the sheet follows it (one poll every 800 ms).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistExportJob {
    pub service: String,
    /// `match`, `create`, `done`.
    pub stage: String,
    pub running: bool,
    pub total: u32,
    pub done: u32,
    pub matched: u32,
    pub missing_count: u32,
    /// The first few missing titles, with their artists.
    pub missing: Vec<PlaylistMissingTitle>,
    /// The other service's page, once the playlist was created there.
    pub url: Option<String>,
    /// Why it failed, in the server's words.
    pub error: Option<String>,
    /// The other service must be linked again.
    pub relink: bool,
    /// `0..=100` for the progress bar.
    pub percent: u32,
}

impl From<core::ExportJob> for PlaylistExportJob {
    fn from(job: core::ExportJob) -> Self {
        let percent = job.percent();
        PlaylistExportJob {
            service: job.service,
            stage: job.stage,
            running: job.running,
            total: job.total,
            done: job.done,
            matched: job.matched,
            missing_count: job.missing_count,
            missing: job
                .missing
                .into_iter()
                .map(|missing| PlaylistMissingTitle {
                    title: missing.title,
                    artist: missing.artist,
                })
                .collect(),
            url: job.url,
            error: job.error,
            relink: job.relink,
            percent,
        }
    }
}

/// One line of the shared link, as the import read it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistImportLine {
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_s: u32,
}

/// One line's outcome: the sound this server will play for it, when there is one.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistImportItem {
    pub line: PlaylistImportLine,
    /// `None` — no safe version: it is listed under « Introuvables ».
    pub track: Option<TrackRow>,
    /// `local`, or the source's name (`deezer`, `youtube`).
    pub via: String,
    /// False when the match is close but not certain (« À vérifier »).
    pub sure: bool,
}

/// What an import job answers while it runs and when it ends.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistImportStatus {
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
    /// The link held more lines than the server reads for one import.
    pub truncated: bool,
    pub done: u32,
    pub matched: u32,
    /// Why the job failed, in the server's words (rule 2: a cause, not a count).
    pub error: Option<String>,
    /// Present once the import was kept.
    pub playlist_id: Option<String>,
    /// How many lines were found where: `local`, a source's name, or `none`.
    pub counts: Vec<PlaylistImportCount>,
    /// Every line's outcome — only once the read is complete.
    pub items: Vec<PlaylistImportItem>,
    /// Whether the job is still doing something.
    pub running: bool,
    /// `0..=100` for the progress bar.
    pub percent: u32,
}

/// One row of an import's « found where » chips.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistImportCount {
    pub via: String,
    pub count: u32,
}

impl From<core::ImportStatus> for PlaylistImportStatus {
    fn from(status: core::ImportStatus) -> Self {
        PlaylistImportStatus {
            running: status.running(),
            percent: status.percent(),
            id: status.id,
            state: status.state,
            source: status.source,
            source_label: status.source_label,
            kind: status.kind,
            url: status.url,
            name: status.name,
            cover: status.cover,
            total: status.total,
            truncated: status.truncated,
            done: status.done,
            matched: status.matched,
            error: status.error,
            playlist_id: status.playlist_id,
            counts: status
                .counts
                .into_iter()
                .map(|(via, count)| PlaylistImportCount { via, count })
                .collect(),
            items: status
                .items
                .into_iter()
                .map(|item| PlaylistImportItem {
                    line: PlaylistImportLine {
                        title: item.line.title,
                        artists: item.line.artists,
                        album: item.line.album,
                        duration_s: item.line.duration_s,
                    },
                    track: item.track.map(TrackRow::from),
                    via: item.via,
                    sure: item.sure,
                })
                .collect(),
        }
    }
}

/// The header of a public page: no ids, no members, no role.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistSharedHeader {
    pub title: String,
    pub description: String,
    pub cover: Option<String>,
    pub nb_tracks: u32,
    pub owner: String,
}

/// One row of a public page, with the capability URL it streams from.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistSharedRow {
    pub row: TrackRow,
    /// The id the capability stream takes, `None` for a row with none.
    pub mid: Option<String>,
    /// The file is on disk NOW: an unarchived title is listed, not playable.
    pub streamable: bool,
    /// `/api/shared/playlist/<token>/stream/<mid>`, built once.
    pub stream_path: Option<String>,
}

/// What the public page draws (`routes/SharedPlaylist.svelte`).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PlaylistSharedData {
    pub header: PlaylistSharedHeader,
    /// The whole list: a public page is read-only and its rows are already on
    /// screen as one answer (the SPA paints 100 at a time from it).
    pub rows: Vec<PlaylistSharedRow>,
    /// The token the page was opened with — the stream's capability.
    pub token: String,
    /// How many rows a player may queue (the archived ones).
    pub playable: u32,
}

impl PlaylistSharedData {
    fn from_core(core: &Arc<core_type::Core>, data: core::SharedData) -> PlaylistSharedData {
        let _ = core;
        let playable = data.playable().len() as u32;
        PlaylistSharedData {
            header: PlaylistSharedHeader {
                title: data.header.title,
                description: data.header.description,
                cover: data.header.cover,
                nb_tracks: data.header.nb_tracks,
                owner: data.header.owner,
            },
            rows: data
                .rows
                .into_iter()
                .map(|row| PlaylistSharedRow {
                    row: TrackRow::from(row.row),
                    mid: row.mid,
                    streamable: row.streamable,
                    stream_path: row.stream_path,
                })
                .collect(),
            token: data.token,
            playable,
        }
    }
}

/// The page as the screen draws it, with its rows behind a handle.
fn data(core: &Arc<core_type::Core>, page: core::Data) -> PlaylistData {
    let handle = crate::lists::open_rows(core, page.rows.as_ref().clone());
    let collaborative = page.collaborative();
    PlaylistData {
        meta: page.meta.into(),
        role: page.role.into(),
        editable: page.editable,
        can_manage: page.can_manage,
        admin: page.admin,
        members: page.members.into_iter().map(Into::into).collect(),
        tracks: handle,
        recommendations: page.recommendations.into(),
        collaborative,
    }
}

fn write(core: &Arc<core_type::Core>, answer: core::Write) -> PlaylistWrite {
    PlaylistWrite {
        data: data(core, answer.data),
        conflict: answer.conflict.map(Into::into),
    }
}

fn order(core: &Arc<core_type::Core>, answer: core::Order) -> PlaylistOrder {
    PlaylistOrder {
        tracks: crate::lists::open_rows(core, answer.rows.as_ref().clone()),
        version: answer.version,
        conflict: answer.conflict.map(Into::into),
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Screens {
    /// The page this device kept for `id` — disk only, **0 requests**. `None` the
    /// first time, which is the one moment the screen has nothing to paint.
    pub fn cached_playlist(&self, id: String) -> Option<PlaylistData> {
        self.core()
            .cached_playlist(&id)
            .map(|page| data(self.core(), page))
    }

    /// ONE request for the whole page, however long the playlist
    /// (`GET /api/playlist/<id>`), on the `User` lane.
    pub async fn playlist(&self, id: String) -> Result<PlaylistData, NzError> {
        let page = self.core().playlist(&id).await.map_err(NzError::from)?;
        Ok(data(self.core(), page))
    }

    /// `GET /api/playlist/<id>/recommendations` — the bottom of the page, asked
    /// once the list is on screen (one request, ≤ 20 rows).
    pub async fn playlist_recommendations(
        &self,
        id: String,
        limit: u32,
    ) -> Result<PlaylistRecommendations, NzError> {
        let recs = self
            .core()
            .playlist_recommendations(&id, limit)
            .await
            .map_err(NzError::from)?;
        Ok(recs.into())
    }

    /// The reorder's FIRST beat: the rows move in memory and nothing is sent —
    /// what makes a drag answer in the frame.
    pub fn playlist_move(&self, id: String, from: u32, to: u32) -> Result<PlaylistOrder, NzError> {
        let answer = self
            .core()
            .playlist_move(&id, from, to)
            .map_err(NzError::from)?;
        Ok(order(self.core(), answer))
    }

    /// The reorder's SECOND beat: one `PUT`, and a `409` is survived by ONE
    /// re-read (never a replay). `conflict` says which half moved.
    pub async fn playlist_save_order(&self, id: String) -> Result<PlaylistOrder, NzError> {
        let answer = self
            .core()
            .playlist_save_order(&id)
            .await
            .map_err(NzError::from)?;
        Ok(order(self.core(), answer))
    }

    /// Appends `tracks` NOW, in memory, and sends nothing: the suggestion sheet
    /// and « Ajouter des titres » answer in the frame.
    pub fn playlist_append(
        &self,
        id: String,
        tracks: Vec<TrackRow>,
    ) -> Result<PlaylistOrder, NzError> {
        let rows: Vec<nz_core::lists::Row> = tracks.into_iter().map(Into::into).collect();
        let answer = self
            .core()
            .playlist_append(&id, &rows)
            .map_err(NzError::from)?;
        Ok(order(self.core(), answer))
    }

    /// `POST /api/playlist/<id>/tracks` for what [`Self::playlist_append`] added:
    /// one request; a failure puts the list back, a `409` re-reads it.
    pub async fn playlist_save_append(&self, id: String) -> Result<PlaylistWrite, NzError> {
        let answer = self
            .core()
            .playlist_save_append(&id)
            .await
            .map_err(NzError::from)?;
        Ok(write(self.core(), answer))
    }

    /// Removes the rows at `indexes` NOW, in memory — the caller's VISIBLE
    /// positions. Nothing is sent.
    pub fn playlist_cut(&self, id: String, indexes: Vec<u32>) -> Result<PlaylistOrder, NzError> {
        let answer = self
            .core()
            .playlist_cut(&id, &indexes)
            .map_err(NzError::from)?;
        Ok(order(self.core(), answer))
    }

    /// `DELETE /api/playlist/<id>/tracks` for what [`Self::playlist_cut`] removed:
    /// one request; a failure puts the rows back where they were.
    pub async fn playlist_save_cut(
        &self,
        id: String,
        indexes: Vec<u32>,
    ) -> Result<PlaylistWrite, NzError> {
        let answer = self
            .core()
            .playlist_save_cut(&id, &indexes)
            .await
            .map_err(NzError::from)?;
        Ok(write(self.core(), answer))
    }

    /// The rows of an open playlist as they stand in memory — **0 requests**.
    /// What the screen asks after a save it does not own (a failed one that put
    /// the list back).
    pub fn playlist_rows(&self, id: String) -> Option<Arc<ListHandle>> {
        self.core()
            .playlist_rows(&id)
            .map(|rows| crate::lists::open_rows(self.core(), rows.as_ref().clone()))
    }

    /// Renames and/or describes the playlist. `None` leaves that field alone.
    pub async fn playlist_edit(
        &self,
        id: String,
        title: Option<String>,
        description: Option<String>,
    ) -> Result<PlaylistWrite, NzError> {
        let answer = self
            .core()
            .playlist_edit(&id, title, description)
            .await
            .map_err(NzError::from)?;
        Ok(write(self.core(), answer))
    }

    /// Deletes the playlist (a terminal act, no version).
    pub async fn playlist_delete(&self, id: String) -> Result<(), NzError> {
        self.core()
            .playlist_delete(&id)
            .await
            .map_err(NzError::from)
    }

    /// Leaves a playlist somebody shared (`leaving` the owner is refused).
    pub async fn playlist_leave(&self, id: String) -> Result<(), NzError> {
        self.core().playlist_leave(&id).await.map_err(NzError::from)
    }

    /// The collaborators and what the caller may do — one request.
    pub async fn playlist_members(&self, id: String) -> Result<PlaylistMembersWrite, NzError> {
        let answer = self
            .core()
            .playlist_members(&id)
            .await
            .map_err(NzError::from)?;
        Ok(answer.into())
    }

    /// Invites one EXACT account name, as an editor or a viewer.
    pub async fn playlist_invite(
        &self,
        id: String,
        username: String,
        role: PlaylistRole,
    ) -> Result<PlaylistMembersWrite, NzError> {
        let answer = self
            .core()
            .playlist_invite(&id, &username, role.into())
            .await
            .map_err(NzError::from)?;
        Ok(answer.into())
    }

    /// Changes somebody's role.
    pub async fn playlist_set_member_role(
        &self,
        id: String,
        user_id: String,
        role: PlaylistRole,
    ) -> Result<PlaylistMembersWrite, NzError> {
        let answer = self
            .core()
            .playlist_set_role(&id, &user_id, role.into())
            .await
            .map_err(NzError::from)?;
        Ok(answer.into())
    }

    /// Removes somebody from a playlist.
    pub async fn playlist_remove_member(
        &self,
        id: String,
        user_id: String,
    ) -> Result<PlaylistMembersWrite, NzError> {
        let answer = self
            .core()
            .playlist_remove_member(&id, &user_id)
            .await
            .map_err(NzError::from)?;
        Ok(answer.into())
    }

    /// The live public link, if the owner made one.
    pub async fn playlist_share_link(
        &self,
        id: String,
    ) -> Result<Option<PlaylistShareLink>, NzError> {
        let link = self
            .core()
            .playlist_share(&id)
            .await
            .map_err(NzError::from)?;
        Ok(link.map(Into::into))
    }

    /// Mints the link (or hands back the live one; one per playlist is enough).
    pub async fn playlist_create_share(
        &self,
        id: String,
        ttl_s: Option<u32>,
    ) -> Result<PlaylistShareLink, NzError> {
        let link = self
            .core()
            .playlist_create_share(&id, ttl_s)
            .await
            .map_err(NzError::from)?;
        Ok(link.into())
    }

    /// Revokes every live link of this playlist — dead on the next request.
    pub async fn playlist_revoke_share(&self, id: String) -> Result<(), NzError> {
        self.core()
            .playlist_revoke_share(&id)
            .await
            .map_err(NzError::from)
    }

    /// What this server can produce as a bulk (ZIP) export.
    pub async fn playlist_export_formats(&self) -> Result<PlaylistExportFormats, NzError> {
        let formats = self.core().export_formats().await.map_err(NzError::from)?;
        Ok(PlaylistExportFormats {
            default: formats.default,
            formats: formats.formats.into_iter().map(Into::into).collect(),
        })
    }

    /// The services this person may export to, and the list formats.
    pub async fn playlist_services(&self) -> Result<PlaylistServices, NzError> {
        let services = self.core().export_services().await.map_err(NzError::from)?;
        Ok(PlaylistServices {
            services: services
                .services
                .into_iter()
                .map(|service| PlaylistService {
                    id: service.id,
                    name: service.name,
                    ready: service.ready,
                    linked: service.linked,
                    account: service.account,
                    file_only: service.file_only,
                })
                .collect(),
            formats: services.formats.into_iter().map(Into::into).collect(),
        })
    }

    /// `POST /api/services/<service>/export` — answers the job to follow.
    pub async fn playlist_export(
        &self,
        service: String,
        kind: String,
        id: String,
        name: String,
        is_public: bool,
    ) -> Result<String, NzError> {
        self.core()
            .export_to_service(&service, &kind, &id, &name, is_public)
            .await
            .map_err(NzError::from)
    }

    /// One poll of an export job (the sheet asks every 800 ms while it runs).
    pub async fn playlist_export_job(&self, job: String) -> Result<PlaylistExportJob, NzError> {
        let job = self.core().export_job(&job).await.map_err(NzError::from)?;
        Ok(job.into())
    }

    /// The LIST file (CSV, XSPF, plain text) as text: small, and what the share
    /// sheet can hand to another app.
    pub async fn playlist_export_list_file(
        &self,
        kind: String,
        id: String,
        fmt: String,
    ) -> Result<String, NzError> {
        self.core()
            .export_list_file(&kind, &id, &fmt)
            .await
            .map_err(NzError::from)
    }

    /// Downloads the audio ZIP to `destination` on the `Bulk` lane (an export
    /// never competes with what the person is listening to, rule 1). Answers the
    /// bytes written.
    pub async fn playlist_export_archive(
        &self,
        kind: String,
        id: String,
        fmt: String,
        destination: String,
    ) -> Result<u64, NzError> {
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.core()
            .export_archive(
                &kind,
                &id,
                &fmt,
                std::path::PathBuf::from(destination),
                cancel,
            )
            .await
            .map_err(NzError::from)
    }

    /// Reads and matches somebody else's shared link as a server job: answers
    /// its id, never the answer (rule 2: the work is not done in this call).
    pub async fn playlist_import_start(&self, url: String) -> Result<String, NzError> {
        self.core()
            .playlist_import_start(&url)
            .await
            .map_err(NzError::from)
    }

    /// One poll of an import job: progress, the per-line causes, and the items
    /// once the read is complete.
    pub async fn playlist_import_status(
        &self,
        job: String,
    ) -> Result<PlaylistImportStatus, NzError> {
        let status = self
            .core()
            .playlist_import_status(&job)
            .await
            .map_err(NzError::from)?;
        Ok(status.into())
    }

    /// Keeps the matched lines as a local playlist. Empty `keep` keeps them all.
    pub async fn playlist_import_save(
        &self,
        job: String,
        title: String,
        keep: Vec<u32>,
    ) -> Result<PlaylistImportStatus, NzError> {
        let keep = if keep.is_empty() { None } else { Some(keep) };
        let status = self
            .core()
            .playlist_import_save(&job, &title, keep)
            .await
            .map_err(NzError::from)?;
        Ok(status.into())
    }

    /// The public, read-only page of a shared link. No session: the token is the
    /// capability.
    pub async fn shared_playlist(&self, token: String) -> Result<PlaylistSharedData, NzError> {
        let page = self
            .core()
            .shared_playlist(&token)
            .await
            .map_err(NzError::from)?;
        Ok(PlaylistSharedData::from_core(self.core(), page))
    }

    /// The public page this device kept — **0 requests**.
    pub fn cached_shared_playlist(&self, token: String) -> Option<PlaylistSharedData> {
        self.core()
            .cached_shared_playlist(&token)
            .map(|page| PlaylistSharedData::from_core(self.core(), page))
    }

    /// The counted bytes the open playlists hold in memory (worker rule 4:
    /// counted bytes, never a process RSS). A diagnostic figure, not a decision.
    pub fn playlist_memory_bytes(&self) -> u64 {
        core::memory_bytes()
    }
}

impl From<core::ShareLink> for PlaylistShareLink {
    fn from(link: core::ShareLink) -> Self {
        PlaylistShareLink {
            token: link.token,
            url: link.url,
            created: link.created,
            expires: link.expires,
        }
    }
}

impl From<core::ExportFormat> for PlaylistExportFormat {
    fn from(format: core::ExportFormat) -> Self {
        PlaylistExportFormat {
            id: format.id,
            label: format.label,
            ext: format.ext,
        }
    }
}

impl From<core::MembersWrite> for PlaylistMembersWrite {
    fn from(answer: core::MembersWrite) -> Self {
        PlaylistMembersWrite {
            members: answer.members.into_iter().map(Into::into).collect(),
            members_version: answer.members_version,
            role: answer.role.into(),
            can_edit: answer.can_edit,
            can_manage: answer.can_manage,
            admin: answer.admin,
            conflict: answer.conflict.map(Into::into),
            denied: answer.denied,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every role and every conflict scope crosses both ways: a screen never maps
    /// a string, and a new core variant is a compile error here.
    #[test]
    fn the_roles_and_the_scopes_cross_both_ways() {
        for role in [
            PlaylistRole::Owner,
            PlaylistRole::Editor,
            PlaylistRole::Viewer,
            PlaylistRole::Admin,
            PlaylistRole::Public,
        ] {
            let inner: core::Role = role.into();
            assert_eq!(PlaylistRole::from(inner), role);
        }
        for scope in [core::ConflictScope::Content, core::ConflictScope::Members] {
            let crossed = PlaylistConflict::from(scope);
            let back = match crossed {
                PlaylistConflict::Content => core::ConflictScope::Content,
                PlaylistConflict::Members => core::ConflictScope::Members,
            };
            assert_eq!(back, scope);
        }
        for state in [
            core::RecState::Idle,
            core::RecState::Loading,
            core::RecState::Ready,
            core::RecState::Failed,
        ] {
            let crossed = RecommendationsState::from(state);
            let back = match crossed {
                RecommendationsState::Idle => core::RecState::Idle,
                RecommendationsState::Loading => core::RecState::Loading,
                RecommendationsState::Ready => core::RecState::Ready,
                RecommendationsState::Failed => core::RecState::Failed,
            };
            assert_eq!(back, state);
        }
    }

    /// The export job's progress is the core's own arithmetic, and the missing
    /// titles cross with their artists (the sheet lists both).
    #[test]
    fn an_export_job_crosses_with_its_progress_and_its_missing_titles() {
        let job = core::ExportJob {
            service: "spotify".into(),
            stage: "match".into(),
            running: true,
            total: 8,
            done: 2,
            matched: 2,
            missing_count: 1,
            missing: vec![core::MissingTitle {
                title: "Absent".into(),
                artist: "Personne".into(),
            }],
            url: None,
            error: None,
            relink: false,
        };
        let crossed: PlaylistExportJob = job.into();
        assert_eq!(crossed.percent, 25);
        assert_eq!(crossed.missing[0].artist, "Personne");
        assert!(crossed.running);
    }

    /// An import's counts keep the server's order (most found first), and a line
    /// with no match crosses with `track: None` rather than a phantom row.
    #[test]
    fn an_import_crosses_its_counts_and_its_unmatched_lines() {
        let status = core::ImportStatus {
            id: "job".into(),
            state: "done".into(),
            source_label: "Spotify".into(),
            total: 3,
            done: 3,
            matched: 2,
            counts: vec![("deezer".into(), 2), ("local".into(), 1)],
            items: vec![
                core::ImportItem {
                    line: core::ImportLine {
                        title: "Titre".into(),
                        artists: vec!["Artiste".into()],
                        album: None,
                        duration_s: 180,
                    },
                    track: Some(nz_core::lists::Row::test_row(0, "Titre")),
                    via: "deezer".into(),
                    sure: true,
                },
                core::ImportItem {
                    line: core::ImportLine {
                        title: "Introuvable".into(),
                        artists: vec![],
                        album: None,
                        duration_s: 0,
                    },
                    track: None,
                    via: String::new(),
                    sure: false,
                },
            ],
            ..core::ImportStatus::default()
        };
        let crossed: PlaylistImportStatus = status.into();
        assert_eq!(crossed.percent, 100);
        assert!(!crossed.running);
        assert_eq!(crossed.counts[0].via, "deezer");
        assert!(crossed.items[0].track.is_some());
        assert!(crossed.items[1].track.is_none());
    }
}
