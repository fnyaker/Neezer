//! The playlist routes, read the way the screen draws them: the payloads, the
//! cache, the one `409` shape, and the counters.
//!
//! Nothing here decides a rule — [`super`] does. This file only converts
//! `nz-model`/JSON into [`super::Data`] and back into requests, so a change in
//! the server's wire is one file, and the tests in `super` read the same code the
//! app runs.
//!
//! **The tag is the whole request budget.** Every call here goes through
//! `Api::with_tag(TAG_PLAYLIST)` and copies `nz-api`'s own counters into the
//! core's meter ([`note_traffic`]), so `NzCore::stats()` answers, in requests and
//! bytes, what this screen spent — the numbers the lot's PR holds.
//!
//! **A `409` keeps its scope.** `nz-api` parses it (`ApiError::Conflict {scope}`)
//! and the core's own [`CoreError::Conflict`] drops the scope, so this module
//! catches it while it is still an [`ApiError`] and raises [`Failure`]: the
//! caller learns WHICH half moved (`content` or `members`) and re-reads exactly
//! that (`Playlist.svelte#onConflict`).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use nz_api::{Api, ApiError, Lane};
use nz_model::{PlaylistPage, PlaylistShare, SharedPlaylist, Track};
use serde::{Deserialize, Serialize};

use super::{
    ConflictScope, Data, ExportFormat, ExportFormats, ExportJob, ImportItem, ImportLine,
    ImportStatus, MemberRow, Meta, MissingTitle, Recommendations, Role, ServiceRow, Services,
    ShareLink, SharedData, SharedHeader, SharedRow,
};
use crate::core::{Core, CoreError};
use crate::lists::Row;

/// The tag every request of this screen is counted under (`NzCore::stats`).
pub const TAG_PLAYLIST: &str = "playlist";

/// What one reply cost, as `nz-api` counts it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Totals {
    pub requests: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

/// A request's failure, with the `409`'s scope kept while it is still known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Failure {
    pub cause: CoreError,
    /// `Some` only for a `409` without `Retry-After`: the world moved, and the
    /// server said which half.
    pub conflict: Option<ConflictScope>,
}

impl From<ApiError> for Failure {
    fn from(error: ApiError) -> Failure {
        let conflict = match &error {
            ApiError::Conflict { scope, .. } => Some(ConflictScope::parse(
                scope.as_deref().unwrap_or(ConflictScope::Content.as_str()),
            )),
            _ => None,
        };
        Failure {
            cause: error.into(),
            conflict,
        }
    }
}

impl From<CoreError> for Failure {
    fn from(cause: CoreError) -> Failure {
        Failure {
            cause,
            conflict: None,
        }
    }
}

/// The client for one call: the core's own, tagged, plus the counters BEFORE it
/// (so [`note_traffic`] records exactly what this call spent).
fn client(core: &Core) -> Result<(Arc<Api>, Totals), Failure> {
    let api = core.api()?;
    let tagged = Arc::new(api.with_tag(TAG_PLAYLIST));
    let before = totals(&tagged);
    Ok((tagged, before))
}

fn totals(api: &Api) -> Totals {
    api.stats()
        .into_iter()
        .find(|row| row.tag == TAG_PLAYLIST)
        .map(|row| Totals {
            requests: row.requests,
            bytes_in: row.bytes_in,
            bytes_out: row.bytes_out,
        })
        .unwrap_or_default()
}

/// Copies what the call spent into the core's meter: one counted request per
/// request really issued, its bytes with the answer.
fn note_traffic(core: &Core, api: &Api, before: Totals) {
    let after = totals(api);
    let requests = after.requests.saturating_sub(before.requests);
    if requests == 0 {
        return;
    }
    core.meter().record(
        TAG_PLAYLIST,
        after.bytes_in.saturating_sub(before.bytes_in),
        after.bytes_out.saturating_sub(before.bytes_out),
    );
    for _ in 1..requests {
        core.meter().record(TAG_PLAYLIST, 0, 0);
    }
}

/// Runs one request with the tag and the counters. The closure gets the tagged
/// client; whatever it answers, the traffic is recorded.
async fn counted<T, F, Fut>(core: &Core, work: F) -> Result<T, Failure>
where
    F: FnOnce(Arc<Api>) -> Fut,
    Fut: std::future::Future<Output = Result<T, Failure>>,
{
    let (api, before) = client(core)?;
    let answer = work(Arc::clone(&api)).await;
    note_traffic(core, &api, before);
    answer
}

// -- the page -----------------------------------------------------------------

/// `GET /api/playlist/<id>` — ONE request, the whole 4 000-row tracklist.
pub(super) async fn read_page(core: &Core, id: &str) -> Result<PlaylistPage, Failure> {
    counted(core, |api| async move {
        api.playlist(id).await.map_err(Failure::from)
    })
    .await
}

/// The page this device kept for `id` (`lib/apicache.js`'s `/playlist/` entry),
/// read from disk: no client, no socket, no request.
pub(super) fn cached_page(core: &Core, id: &str) -> Option<PlaylistPage> {
    let owner = core.me()?.user.id.as_str().to_owned();
    let cache: &dyn nz_api::ResponseCache = core.cache();
    let body = cache.get(&owner, &page_path(id))?;
    serde_json::from_str::<PlaylistPage>(&body).ok()
}

fn page_path(id: &str) -> String {
    format!("/playlist/{id}")
}

/// The server's page, read the way the screen draws it.
pub(super) fn data(page: PlaylistPage) -> Data {
    let playlist = page.playlist;
    let role = Role::parse(playlist.role.as_deref().unwrap_or("owner"));
    let admin = playlist.admin;
    let meta = Meta {
        id: playlist.id.as_str().to_owned(),
        deezer_id: playlist.deezer_id.clone(),
        title: playlist.title.clone(),
        description: playlist.description.clone().unwrap_or_default(),
        cover: playlist.cover.clone(),
        owner: playlist.owner.clone().unwrap_or_default(),
        nb_tracks: playlist.nb_tracks,
        private_tracks: playlist.private_tracks,
        is_favorite: playlist.is_favorite,
        version: playlist.version,
        members_version: playlist.members_version,
        last_edited_by: playlist.last_edited_by.clone(),
        last_edited_at: playlist.last_edited_at.clone(),
    };
    let members = playlist
        .members
        .iter()
        .map(|member| MemberRow {
            id: member.id.as_str().to_owned(),
            name: member.name.clone(),
            role: Role::parse(&member.role),
            added: member.added.clone(),
        })
        .collect();
    Data {
        meta,
        role,
        editable: playlist.editable,
        can_manage: playlist.can_manage,
        admin,
        members,
        rows: Arc::new(rows_of(&page.tracks)),
        recommendations: Recommendations::default(),
    }
}

/// A tracklist as the rows a screen draws, numbered in its own order.
pub(super) fn rows_of(tracks: &[Track]) -> Vec<Row> {
    tracks
        .iter()
        .enumerate()
        .map(|(position, track)| row_of(track, position as u32))
        .collect()
}

/// One track, as the row its list draws. The key is `<source>:<ref>`, the app's
/// own convention (`trackkey.js`): the ref is the row's UUID when it has one,
/// else the provider id — the same string the server resolves a write by.
pub(super) fn row_of(track: &Track, position: u32) -> Row {
    Row {
        key: row_key(track),
        id: reference_of(track),
        title: track.title.clone(),
        artists: artists_line(track),
        album: track.album.title.clone(),
        album_id: Some(track.album.deezer_id.as_str().to_owned()).filter(|id| !id.is_empty()),
        duration_s: track.duration,
        cover: track.album.cover.clone(),
        explicit: track.explicit,
        // A playlist row carries no per-track star of its own: MB10's writes
        // patch every open list (`lists::Registry`), which is the one owner of
        // that flag.
        liked: false,
        disliked: false,
        unavailable: track.unavailable,
        downloaded: false,
        source: source_of(track).to_owned(),
        position,
        played_at: 0,
        added_at: 0,
    }
}

/// `<source>:<id>` — the key every list of this app uses.
fn row_key(track: &Track) -> String {
    format!("{}:{}", source_of(track), reference_of(track))
}

/// What a request names the track by (`trackkey.js#trackRef`): the row's UUID
/// when it has one, the provider id otherwise.
pub(super) fn reference_of(track: &Track) -> String {
    track
        .id
        .as_ref()
        .map(|id| id.as_str().to_owned())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| track.deezer_id.to_string())
}

/// `deezer`, `youtube`, `upload`, `local` — what the row's source chip says.
fn source_of(track: &Track) -> &'static str {
    if let Some(source) = track.source.as_deref().filter(|source| !source.is_empty()) {
        return match source {
            "youtube" => "youtube",
            "deezer" => "deezer",
            _ => "local",
        };
    }
    if track.local { "local" } else { "deezer" }
}

/// The one line the list shows for the credits (`format.js#artistLine`).
fn artists_line(track: &Track) -> String {
    if let Some(display) = track
        .display_artist
        .as_deref()
        .filter(|line| !line.is_empty())
    {
        return display.to_owned();
    }
    let names: Vec<&str> = track
        .artists
        .iter()
        .map(|artist| artist.name.as_str())
        .filter(|name| !name.is_empty())
        .collect();
    if !names.is_empty() {
        return names.join(", ");
    }
    track.artist.name.clone()
}

// -- recommendations ----------------------------------------------------------

/// `GET /api/playlist/<id>/recommendations?limit=` — the bottom of the page.
pub(super) async fn read_recommendations(
    core: &Core,
    id: &str,
    limit: u32,
) -> Result<Vec<Row>, Failure> {
    let page = counted(core, |api| async move {
        api.playlist_recommendations(id, limit)
            .await
            .map_err(Failure::from)
    })
    .await?;
    Ok(rows_of(&page.tracks))
}

// -- writes --------------------------------------------------------------------

/// The counters a mutation answered (`{"ok", "version", "nb_tracks"}`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Counters {
    pub version: Option<u32>,
    pub nb_tracks: Option<u32>,
}

fn counters_of(answer: &serde_json::Value) -> Counters {
    Counters {
        version: answer
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .map(|value| value as u32),
        nb_tracks: answer
            .get("nb_tracks")
            .and_then(serde_json::Value::as_u64)
            .map(|value| value as u32),
    }
}

/// Forgets the kept page after a write: the copy on disk is what the next read
/// must not serve as fresh (`api.forget`, `lib/apicache.js`'s invalidation).
fn invalidate(core: &Core, id: &str) {
    if let Ok(api) = core.api() {
        api.forget(&page_path(id));
        api.forget(&format!("/playlist/{id}/members"));
    }
}

/// `PUT /api/playlist/<id>/order` — the whole visible order, version-guarded.
pub(super) async fn put_order(
    core: &Core,
    id: &str,
    refs: &[String],
    version: u32,
) -> Result<Counters, Failure> {
    let answer = counted(core, |api| async move {
        api.reorder_playlist(id, refs, Some(version))
            .await
            .map_err(Failure::from)
    })
    .await?;
    invalidate(core, id);
    Ok(counters_of(&answer))
}

/// `POST /api/playlist/<id>/tracks` — append, version-guarded.
pub(super) async fn post_tracks(
    core: &Core,
    id: &str,
    refs: &[String],
    version: u32,
) -> Result<Counters, Failure> {
    let answer = counted(core, |api| async move {
        api.add_to_playlist(id, refs, Some(version))
            .await
            .map_err(Failure::from)
    })
    .await?;
    invalidate(core, id);
    Ok(counters_of(&answer))
}

/// `DELETE /api/playlist/<id>/tracks` by the caller's visible INDEXES.
pub(super) async fn delete_indexes(
    core: &Core,
    id: &str,
    indexes: &[usize],
    version: u32,
) -> Result<Counters, Failure> {
    let indexes: Vec<u32> = indexes.iter().map(|index| *index as u32).collect();
    let answer = counted(core, |api| async move {
        api.remove_playlist_indexes(id, &indexes, Some(version))
            .await
            .map_err(Failure::from)
    })
    .await?;
    invalidate(core, id);
    Ok(counters_of(&answer))
}

/// `PATCH /api/playlist/<id>` — title and/or description, version-guarded.
pub(super) async fn patch_meta(
    core: &Core,
    id: &str,
    title: Option<&str>,
    description: Option<&str>,
    version: u32,
) -> Result<Counters, Failure> {
    let mut fields = serde_json::Map::new();
    if let Some(title) = title {
        fields.insert("title".to_owned(), serde_json::Value::from(title));
    }
    if let Some(description) = description {
        fields.insert(
            "description".to_owned(),
            serde_json::Value::from(description),
        );
    }
    let fields = serde_json::Value::Object(fields);
    let answer = counted(core, |api| async move {
        api.edit_playlist(id, &fields, Some(version))
            .await
            .map_err(Failure::from)
    })
    .await?;
    invalidate(core, id);
    Ok(counters_of(&answer))
}

/// `DELETE /api/playlist/<id>` — terminal, no version precondition.
pub(super) async fn delete_playlist(core: &Core, id: &str) -> Result<(), Failure> {
    counted(core, |api| async move {
        api.delete_playlist(id, None).await.map_err(Failure::from)
    })
    .await?;
    invalidate(core, id);
    Ok(())
}

/// `POST /api/playlist/<id>/leave`.
pub(super) async fn leave_playlist(core: &Core, id: &str) -> Result<(), Failure> {
    counted(core, |api| async move {
        api.leave_playlist(id).await.map_err(Failure::from)
    })
    .await?;
    invalidate(core, id);
    Ok(())
}

// -- the collaborators ---------------------------------------------------------

/// `GET /api/playlist/<id>/members`, or a membership mutation's answer: the
/// same three fields plus (for a GET) what the caller may do.
pub(super) struct MemberAnswer {
    pub members: Vec<MemberRow>,
    pub members_version: u32,
    pub role: Option<Role>,
    pub can_edit: bool,
    pub can_manage: bool,
    pub admin: bool,
    /// Whether this answer carried the capabilities (a GET, or a refusal).
    pub has_access: bool,
}

impl MemberAnswer {
    fn parse(value: &serde_json::Value) -> MemberAnswer {
        let members = value
            .get("members")
            .and_then(|members| {
                serde_json::from_value::<Vec<nz_model::PlaylistMember>>(members.clone()).ok()
            })
            .unwrap_or_default()
            .into_iter()
            .map(|member| MemberRow {
                id: member.id.as_str().to_owned(),
                name: member.name,
                role: Role::parse(&member.role),
                added: member.added,
            })
            .collect();
        let flag = |key: &str| value.get(key).and_then(serde_json::Value::as_bool);
        let has_access = flag("can_edit").is_some() || flag("can_manage").is_some();
        MemberAnswer {
            members,
            members_version: value
                .get("members_version")
                .and_then(serde_json::Value::as_u64)
                .map(|value| value as u32)
                .unwrap_or_default(),
            role: value
                .get("role")
                .and_then(serde_json::Value::as_str)
                .map(Role::parse),
            can_edit: flag("can_edit").unwrap_or_default(),
            can_manage: flag("can_manage").unwrap_or_default(),
            admin: flag("admin").unwrap_or_default(),
            has_access,
        }
    }
}

/// `GET /api/playlist/<id>/members` — who is on it and what the caller may do.
pub(super) async fn read_members(core: &Core, id: &str) -> Result<MemberAnswer, Failure> {
    let answer = counted(core, |api| async move {
        api.playlist_members(id).await.map_err(Failure::from)
    })
    .await?;
    Ok(MemberAnswer {
        members: answer
            .members
            .into_iter()
            .map(|member| MemberRow {
                id: member.id.as_str().to_owned(),
                name: member.name,
                role: Role::parse(&member.role),
                added: member.added,
            })
            .collect(),
        members_version: answer.members_version,
        role: answer.role.as_deref().map(Role::parse),
        can_edit: answer.can_edit,
        can_manage: answer.can_manage,
        admin: answer.admin,
        has_access: true,
    })
}

/// `POST /api/playlist/<id>/members` — invite one exact account name.
pub(super) async fn post_member(
    core: &Core,
    id: &str,
    username: &str,
    role: Role,
    version: Option<u32>,
) -> Result<MemberAnswer, Failure> {
    let role = role.as_str();
    let answer = counted(core, |api| async move {
        api.invite_playlist_member(id, username, role, version)
            .await
            .map_err(Failure::from)
    })
    .await?;
    Ok(MemberAnswer::parse(&answer))
}

/// `PATCH /api/playlist/<id>/members/<user>` — change a role.
pub(super) async fn patch_member(
    core: &Core,
    id: &str,
    user_id: &str,
    role: Role,
    version: Option<u32>,
) -> Result<MemberAnswer, Failure> {
    let role = role.as_str();
    let answer = counted(core, |api| async move {
        api.set_playlist_member_role(id, user_id, role, version)
            .await
            .map_err(Failure::from)
    })
    .await?;
    Ok(MemberAnswer::parse(&answer))
}

/// `DELETE /api/playlist/<id>/members/<user>` — remove somebody.
pub(super) async fn delete_member(
    core: &Core,
    id: &str,
    user_id: &str,
    version: Option<u32>,
) -> Result<MemberAnswer, Failure> {
    let answer = counted(core, |api| async move {
        api.remove_playlist_member(id, user_id, version)
            .await
            .map_err(Failure::from)
    })
    .await?;
    Ok(MemberAnswer::parse(&answer))
}

// -- the public link -----------------------------------------------------------

fn share_of(answer: &PlaylistShare) -> Option<ShareLink> {
    let link = answer.link.as_ref()?;
    if link.token.is_empty() {
        return None;
    }
    Some(ShareLink {
        token: link.token.clone(),
        url: link.url.clone(),
        created: link.created.clone(),
        expires: link.expires.clone(),
    })
}

/// `GET /api/playlist/<id>/share` — the live link, if one exists.
pub(super) async fn read_share(core: &Core, id: &str) -> Result<Option<ShareLink>, Failure> {
    let answer = counted(core, |api| async move {
        api.playlist_share(id).await.map_err(Failure::from)
    })
    .await?;
    Ok(share_of(&answer))
}

/// `POST /api/playlist/<id>/share` — mint the link (or hand back the live one).
pub(super) async fn create_share(
    core: &Core,
    id: &str,
    ttl_s: Option<u32>,
) -> Result<ShareLink, Failure> {
    let answer = counted(core, |api| async move {
        api.create_playlist_share(id, ttl_s)
            .await
            .map_err(Failure::from)
    })
    .await?;
    share_of(&answer).ok_or_else(|| {
        Failure::from(CoreError::Invalid {
            detail: "le serveur n'a pas rendu de lien".to_owned(),
        })
    })
}

/// `DELETE /api/playlist/<id>/share`.
pub(super) async fn revoke_share(core: &Core, id: &str) -> Result<(), Failure> {
    counted(core, |api| async move {
        api.revoke_playlist_share(id).await.map_err(Failure::from)
    })
    .await?;
    Ok(())
}

// -- the public page -----------------------------------------------------------

/// The snapshot a public page is kept in: the answer itself, so a warm open
/// paints with no request (`SharedPlaylist.svelte` has no offline copy of its
/// own, and rule 3 asks for one).
#[derive(Debug, Serialize, Deserialize)]
struct SharedSnapshot {
    header: SharedHeader,
    rows: Vec<StoredSharedRow>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSharedRow {
    row: StoredRow,
    mid: Option<String>,
    streamable: bool,
}

/// A [`Row`] on disk: the same fields, because `lists::Row` is the core's own
/// type and carries two sort keys a screen never draws.
#[derive(Debug, Serialize, Deserialize)]
struct StoredRow {
    key: String,
    id: String,
    title: String,
    artists: String,
    album: String,
    album_id: Option<String>,
    duration_s: u32,
    cover: Option<String>,
    explicit: bool,
    unavailable: bool,
    source: String,
    position: u32,
}

impl From<&Row> for StoredRow {
    fn from(row: &Row) -> StoredRow {
        StoredRow {
            key: row.key.clone(),
            id: row.id.clone(),
            title: row.title.clone(),
            artists: row.artists.clone(),
            album: row.album.clone(),
            album_id: row.album_id.clone(),
            duration_s: row.duration_s,
            cover: row.cover.clone(),
            explicit: row.explicit,
            unavailable: row.unavailable,
            source: row.source.clone(),
            position: row.position,
        }
    }
}

impl From<StoredRow> for Row {
    fn from(row: StoredRow) -> Row {
        Row {
            key: row.key,
            id: row.id,
            title: row.title,
            artists: row.artists,
            album: row.album,
            album_id: row.album_id,
            duration_s: row.duration_s,
            cover: row.cover,
            explicit: row.explicit,
            liked: false,
            disliked: false,
            unavailable: row.unavailable,
            downloaded: false,
            source: row.source,
            position: row.position,
            played_at: 0,
            added_at: 0,
        }
    }
}

fn shared_of(page: SharedPlaylist, token: &str) -> SharedData {
    let header = SharedHeader {
        title: page.playlist.title,
        description: page.playlist.description,
        cover: page.playlist.cover,
        nb_tracks: page.playlist.nb_tracks,
        owner: page.playlist.owner,
    };
    let rows = page
        .tracks
        .iter()
        .enumerate()
        .map(|(position, shared)| {
            let mut row = row_of(&shared.track, position as u32);
            // The public page carries its own capability cover (`/api/shared/
            // playlist/<token>/cover/<mid>`): the album's own path would need a
            // session.
            if shared.cover.is_some() {
                row.cover.clone_from(&shared.cover);
            }
            SharedRow {
                row,
                mid: shared.mid.clone(),
                streamable: shared.streamable,
                stream_path: shared
                    .mid
                    .as_deref()
                    .map(|mid| shared_stream_path(token, mid)),
            }
        })
        .collect();
    SharedData {
        header,
        rows,
        token: token.to_owned(),
    }
}

/// `GET /api/shared/playlist/<token>` — no session, the token is the capability.
pub(super) async fn read_shared(core: &Core, token: &str) -> Result<SharedData, Failure> {
    let page = counted(core, |api| async move {
        api.shared_playlist(token).await.map_err(Failure::from)
    })
    .await?;
    let data = shared_of(page, token);
    write_shared(core, &data);
    Ok(data)
}

/// The public page this device kept, with no request.
pub(super) fn cached_shared(core: &Core, token: &str) -> Option<SharedData> {
    let path = shared_path(core, token);
    let body = std::fs::read_to_string(path).ok()?;
    let snapshot: SharedSnapshot = serde_json::from_str(&body).ok()?;
    let rows = snapshot
        .rows
        .into_iter()
        .map(|row| SharedRow {
            row: row.row.into(),
            stream_path: row.mid.as_deref().map(|mid| shared_stream_path(token, mid)),
            mid: row.mid,
            streamable: row.streamable,
        })
        .collect();
    Some(SharedData {
        header: snapshot.header,
        rows,
        token: token.to_owned(),
    })
}

/// `cacheDir/playlists/shared-<hash>.json` — the file name is a hash of the
/// token, never the token itself (a cache directory must not leak a capability),
/// and the last three are kept.
fn shared_path(core: &Core, token: &str) -> PathBuf {
    core.cache_path("playlists")
        .join(format!("shared-{:016x}.json", fnv1a(token)))
}

fn write_shared(core: &Core, data: &SharedData) {
    let directory = core.cache_path("playlists");
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    let snapshot = SharedSnapshot {
        header: data.header.clone(),
        rows: data
            .rows
            .iter()
            .map(|row| StoredSharedRow {
                row: StoredRow::from(&row.row),
                mid: row.mid.clone(),
                streamable: row.streamable,
            })
            .collect(),
    };
    let Ok(body) = serde_json::to_vec(&snapshot) else {
        return;
    };
    let path = shared_path(core, &data.token);
    if std::fs::write(&path, body).is_err() {
        return;
    }
    prune_shared(&directory);
}

/// Keeps the three most recent public pages: a shared link is a visit, not a
/// library, and the app must not grow a copy of every one ever opened.
fn prune_shared(directory: &std::path::Path) {
    const KEEP: usize = 3;
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("shared-"))
        })
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .collect();
    if files.len() <= KEEP {
        return;
    }
    files.sort_by_key(|(modified, _)| *modified);
    for (_, path) in files.iter().take(files.len() - KEEP) {
        let _ = std::fs::remove_file(path);
    }
}

/// FNV-1a, 64-bit: a file name, never a security decision.
fn fnv1a(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The URL the public page's stream takes for one row (`SharedPlaylist.svelte`'s
/// `streamUrl`): the token and the row's `mid`, nothing else — the capability is
/// the token, and the route is under `/api`.
fn shared_stream_path(token: &str, mid: &str) -> String {
    format!(
        "/api/shared/playlist/{}/stream/{}",
        segment(token),
        segment(mid)
    )
}

/// One path segment, percent-encoded: everything but `A-Z a-z 0-9 - . _ ~` goes
/// through `%XX`. The app's ids and tokens are already safe (a UUID, a Deezer
/// number, `<id>.<mac>`), so this is the guard that keeps a hostile one from
/// writing a path of its own, never a formatter for display.
fn segment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte));
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

// -- export --------------------------------------------------------------------

/// `GET /api/export/formats`.
pub(super) async fn read_formats(core: &Core) -> Result<ExportFormats, Failure> {
    let formats = counted(core, |api| async move {
        api.export_formats().await.map_err(Failure::from)
    })
    .await?;
    Ok(ExportFormats {
        default: formats.default,
        formats: formats
            .formats
            .into_iter()
            .map(|format| ExportFormat {
                id: format.id,
                label: format.label,
                ext: format.ext,
            })
            .collect(),
    })
}

/// `GET /api/services` — the services the person may export to, and the list
/// formats.
pub(super) async fn read_services(core: &Core) -> Result<Services, Failure> {
    let services = counted(core, |api| async move {
        api.services().await.map_err(Failure::from)
    })
    .await?;
    Ok(Services {
        services: services
            .services
            .into_iter()
            .map(|service| ServiceRow {
                id: service.id,
                name: service.name,
                ready: service.ready,
                linked: service.linked,
                account: service.account,
                file_only: service.file.unwrap_or(false),
            })
            .collect(),
        formats: services
            .formats
            .into_iter()
            .map(|format| ExportFormat {
                id: format.id,
                label: format.label,
                ext: format.ext,
            })
            .collect(),
    })
}

/// `POST /api/services/<service>/export` — answers the job to follow.
pub(super) async fn start_export(
    core: &Core,
    service: &str,
    kind: &str,
    id: &str,
    name: &str,
    public: bool,
) -> Result<String, Failure> {
    let answer = counted(core, |api| async move {
        api.export_to_service(service, kind, id, name, public)
            .await
            .map_err(Failure::from)
    })
    .await?;
    answer
        .get("job")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            Failure::from(CoreError::Invalid {
                detail: "le serveur n'a pas rendu de travaux".to_owned(),
            })
        })
}

/// `GET /api/services/export/<job>` — one poll of the export.
pub(super) async fn read_export_job(core: &Core, job: &str) -> Result<ExportJob, Failure> {
    let value = counted(core, |api| async move {
        api.service_export_job(job).await.map_err(Failure::from)
    })
    .await?;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let number = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .map(|number| number as u32)
            .unwrap_or_default()
    };
    let missing = value
        .get("missing")
        .and_then(serde_json::Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| MissingTitle {
                    title: row
                        .get("title")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    artist: row
                        .get("artist")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ExportJob {
        service: text("service").unwrap_or_default(),
        stage: text("stage").unwrap_or_default(),
        running: value
            .get("running")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        total: number("total"),
        done: number("done"),
        matched: number("matched"),
        missing_count: number("missing_count"),
        missing,
        url: text("url"),
        error: text("error"),
        relink: value
            .get("relink")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

/// `GET /api/services/list/<kind>/<id>?fmt=` — the LIST file, as text.
pub(super) async fn read_list_file(
    core: &Core,
    kind: &str,
    id: &str,
    fmt: &str,
) -> Result<String, Failure> {
    let path = list_file_path(kind, id, fmt);
    let bytes = counted(core, |api| async move {
        api.bytes(Lane::User, &path).await.map_err(Failure::from)
    })
    .await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn list_file_path(kind: &str, id: &str, fmt: &str) -> String {
    format!(
        "/services/list/{}/{}?fmt={}",
        segment(kind),
        segment(id),
        segment(fmt)
    )
}

/// `GET /api/export/<kind>/<id>?fmt=` into `destination`, on the `Bulk` lane: an
/// export is a download, never what the person just asked to hear (rule 1).
pub(super) async fn download_export(
    core: &Core,
    kind: &str,
    id: &str,
    fmt: &str,
    destination: PathBuf,
    cancel: Arc<AtomicBool>,
) -> Result<u64, Failure> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            Failure::from(CoreError::Internal {
                detail: format!("{} : {error}", parent.display()),
            })
        })?;
    }
    let path = format!(
        "/export/{}/{}?fmt={}",
        segment(kind),
        segment(id),
        segment(fmt)
    );
    let bytes = counted(core, |api| async move {
        let report = api
            .download_to(Lane::Bulk, &path, &destination, |_done, _total| {}, &cancel)
            .await
            .map_err(Failure::from)?;
        Ok(report.bytes)
    })
    .await?;
    Ok(bytes)
}

// -- import --------------------------------------------------------------------

/// `POST /api/import/playlist` — answers the job's id.
pub(super) async fn start_import(core: &Core, url: &str) -> Result<String, Failure> {
    let job = counted(core, |api| async move {
        api.import_playlist(url).await.map_err(Failure::from)
    })
    .await?;
    if job.id.is_empty() {
        return Err(Failure::from(CoreError::Invalid {
            detail: "le serveur n'a pas rendu de travaux".to_owned(),
        }));
    }
    Ok(job.id)
}

/// `GET /api/import/playlist/<job>` — progress, causes, and the items once the
/// read is complete (`_snapshot`).
pub(super) async fn read_import(core: &Core, job: &str) -> Result<ImportStatus, Failure> {
    let value = counted(core, |api| async move {
        api.import_status(job).await.map_err(Failure::from)
    })
    .await?;
    Ok(import_status(&value, job))
}

fn import_status(value: &serde_json::Value, job: &str) -> ImportStatus {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let number = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .map(|number| number as u32)
            .unwrap_or_default()
    };
    let items = value
        .get("items")
        .and_then(serde_json::Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    let line = row.get("line").cloned().unwrap_or_default();
                    let names: Vec<String> = line
                        .get("artists")
                        .and_then(serde_json::Value::as_array)
                        .map(|artists| {
                            artists
                                .iter()
                                .filter_map(|name| name.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default();
                    ImportItem {
                        line: ImportLine {
                            title: line
                                .get("title")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            artists: names,
                            album: line
                                .get("album")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_owned),
                            duration_s: line
                                .get("duration")
                                .and_then(serde_json::Value::as_u64)
                                .unwrap_or_default() as u32,
                        },
                        track: row
                            .get("track")
                            .and_then(|track| serde_json::from_value::<Track>(track.clone()).ok())
                            .map(|track| row_of(&track, 0)),
                        via: row
                            .get("via")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        sure: row
                            .get("sure")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let counts = value
        .get("counts")
        .and_then(serde_json::Value::as_object)
        .map(|counts| {
            let mut rows: Vec<(String, u32)> = counts
                .iter()
                .map(|(via, count)| {
                    (
                        via.clone(),
                        count.as_u64().map(|count| count as u32).unwrap_or_default(),
                    )
                })
                .collect();
            rows.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
            rows
        })
        .unwrap_or_default();
    ImportStatus {
        id: text("id").unwrap_or_else(|| job.to_owned()),
        state: text("state").unwrap_or_default(),
        source: text("source").unwrap_or_default(),
        source_label: text("source_label").unwrap_or_default(),
        kind: text("kind").unwrap_or_default(),
        url: text("url").unwrap_or_default(),
        name: text("name").unwrap_or_default(),
        cover: text("cover"),
        total: number("total"),
        truncated: value
            .get("truncated")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        done: number("done"),
        matched: number("matched"),
        error: text("error"),
        playlist_id: text("playlist_id"),
        counts,
        items,
    }
}

/// `POST /api/import/playlist/<job>/save` then one poll: the answer is
/// `{"ok"}` and the job moves to `saving`, exactly as the SPA follows it.
pub(super) async fn save_import(
    core: &Core,
    job: &str,
    title: &str,
    keep: Option<Vec<u32>>,
) -> Result<ImportStatus, Failure> {
    let body = match keep {
        Some(indexes) => serde_json::json!({ "title": title, "keep": indexes }),
        None => serde_json::json!({ "title": title }),
    };
    let path = format!("/import/playlist/{}/save", segment(job));
    counted(core, |api| async move {
        // `nz-api`'s typed `import_save` sends `keep` as a flag; the SPA sends
        // the item INDEXES (a subset of the matched lines), so this is the one
        // call it does not wrap and `Api::send` is used with the SPA's own body.
        api.send::<_, serde_json::Value>(Lane::User, nz_api::Method::POST, &path, Some(&body))
            .await
            .map_err(Failure::from)
    })
    .await?;
    read_import(core, job).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key and the reference are the app's own convention: a row's UUID when
    /// it has one, the provider id otherwise — what `trackkey.js` decides, and
    /// what every write names the track by.
    #[test]
    fn a_rows_key_and_reference_follow_trackkey() {
        let mut track: Track = serde_json::from_str(
            r#"{"id":"b8a1e106-0000-4000-8000-000000000000","deezer_id":"3000168",
                "title":"Été","duration":183,"artist":{"deezer_id":"1000003","name":"Gold 3"},
                "artists":[{"deezer_id":"1000003","name":"Gold 3","role":"Main"},
                           {"deezer_id":"1000004","name":"June","role":"Featured"}],
                "album":{"deezer_id":"2000044","title":"Fire","cover":"/api/cover/2000044"}}"#,
        )
        .expect("a track");
        let row = row_of(&track, 7);
        assert_eq!(row.id, "b8a1e106-0000-4000-8000-000000000000");
        assert_eq!(row.key, "deezer:b8a1e106-0000-4000-8000-000000000000");
        assert_eq!(row.artists, "Gold 3, June", "the credits line is joined");
        assert_eq!(row.album, "Fire");
        assert_eq!(row.cover.as_deref(), Some("/api/cover/2000044"));
        assert_eq!(row.position, 7);

        // A catalogue result with no row of its own is named by its Deezer id.
        track.id = None;
        let row = row_of(&track, 0);
        assert_eq!(row.id, "3000168");
        assert_eq!(row.key, "deezer:3000168");
    }

    /// A `409` keeps its scope through the wrapper, which is what lets the screen
    /// re-read exactly the half the server named.
    #[test]
    fn a_conflict_keeps_the_scope_the_server_named() {
        let failure = Failure::from(ApiError::Conflict {
            message: "conflict".into(),
            code: None,
            scope: Some("members".into()),
        });
        assert_eq!(failure.conflict, Some(ConflictScope::Members));
        assert!(matches!(failure.cause, CoreError::Conflict { .. }));
        // A `409` with `Retry-After` is a WAIT, never a conflict.
        let wait = Failure::from(ApiError::NotReady { retry_after: 30 });
        assert_eq!(wait.conflict, None);
        assert_eq!(wait.cause, CoreError::NotReady { retry_after_s: 30 });
    }

    /// The hash a public page's file name uses is stable and never the token.
    #[test]
    fn the_shared_page_file_is_named_by_a_hash() {
        let token = "abcdefghijkl.secretmac";
        let first = fnv1a(token);
        assert_eq!(first, fnv1a(token));
        assert_ne!(first, fnv1a("abcdefghijkl.secretmaz"));
        assert!(!format!("{first:016x}").contains("secret"));
        assert_eq!(
            shared_stream_path("tok/en", "3000 168"),
            "/api/shared/playlist/tok%2Fen/stream/3000%20168"
        );
    }
}
