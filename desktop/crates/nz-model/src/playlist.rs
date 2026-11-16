//! Playlists: the page, the library's list with its folders, members, the
//! share link and the recommendations.

use serde::{Deserialize, Serialize};

use crate::ids::{PlaylistId, UserId};
use crate::track::Track;

/// A playlist as `/api/playlist/<id>` and `/api/me/playlists` describe it.
///
/// One struct for both: the page fills the version/member fields, the library
/// list fills `folder_id`/`position`. `deezer_id` is the provider's id when the
/// playlist came from Deezer and `null` for a local one — never use it to open
/// the page, `id` is what every route takes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Playlist {
    pub id: PlaylistId,
    #[serde(default)]
    pub deezer_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Relative server path (`/api/cover/<id>`), or `null`.
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    /// The caller's role on this playlist: `"owner"`, `"editor"`, `"viewer"`.
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub editable: bool,
    #[serde(default)]
    pub can_manage: bool,
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default)]
    pub nb_tracks: u32,
    /// Uploads of other people: listed as a count, never as rows.
    #[serde(default)]
    pub private_tracks: u32,
    #[serde(default)]
    pub members: Vec<PlaylistMember>,
    #[serde(default)]
    pub members_version: u32,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub last_edited_at: Option<String>,
    #[serde(default)]
    pub last_edited_by: Option<String>,
    /// `/api/me/playlists` only: the folder it sits in, `null` at the root.
    #[serde(default)]
    pub folder_id: Option<String>,
    /// `/api/me/playlists` only: its place inside its folder.
    #[serde(default)]
    pub position: Option<u32>,
}

/// One member of a playlist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistMember {
    pub id: UserId,
    pub name: String,
    /// `"owner"`, `"editor"` or `"viewer"`.
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub added: Option<String>,
}

/// `/api/playlist/<id>` — the playlist and its tracks in order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistPage {
    pub playlist: Playlist,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

/// A folder of the playlist library (`playlist_folders.list_shape`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistFolder {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub position: Option<u32>,
    /// How many playlists the person has in it (added by `/api/me/playlists`).
    #[serde(default)]
    pub nb_playlists: Option<u32>,
}

/// `/api/me/playlists` — the folders and the playlists they hold.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistList {
    #[serde(default)]
    pub folders: Vec<PlaylistFolder>,
    #[serde(default)]
    pub playlists: Vec<Playlist>,
}

/// `/api/playlist/<id>/version` — the optimistic-concurrency token.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistVersion {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub members_version: u32,
    #[serde(default)]
    pub nb_tracks: u32,
    #[serde(default)]
    pub last_edited_at: Option<String>,
    #[serde(default)]
    pub last_edited_by: Option<String>,
}

/// `/api/playlist/<id>/members` — the members' sheet.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistMembers {
    #[serde(default)]
    pub members: Vec<PlaylistMember>,
    #[serde(default)]
    pub members_version: u32,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub can_edit: bool,
    #[serde(default)]
    pub can_manage: bool,
    #[serde(default)]
    pub admin: bool,
}

/// The capability link of a playlist, when one is live.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistShareLink {
    #[serde(default)]
    pub token: String,
    /// The relative page path (`/pl/<token>`).
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub expires: Option<String>,
}

/// `/api/playlist/<id>/share` — `{"link": … | null}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistShare {
    #[serde(default)]
    pub link: Option<PlaylistShareLink>,
}

/// The header of a public shared page: no ids, no members, no role.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SharedPlaylistHeader {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub nb_tracks: u32,
    #[serde(default)]
    pub owner: String,
}

/// `/api/shared/playlist/<token>` — a read-only page anyone holding the link
/// sees. Its tracks carry three keys no other payload has: `mid` (the id to
/// stream), `cover` (a capability URL) and `streamable` (the file is on disk
/// NOW, so a dead archive shows as unplayable instead of answering 409).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SharedPlaylist {
    #[serde(default)]
    pub playlist: SharedPlaylistHeader,
    #[serde(default)]
    pub tracks: Vec<SharedTrack>,
}

/// One row of a shared playlist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SharedTrack {
    #[serde(flatten)]
    pub track: Track,
    #[serde(default)]
    pub mid: Option<String>,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub streamable: bool,
}

/// `/api/playlist/<id>/recommendations` — the bottom of a playlist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistRecommendations {
    #[serde(default)]
    pub playlist: PlaylistRef,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

/// `{"id", "title"}`: the playlist a recommendations answer belongs to.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistRef {
    #[serde(default)]
    pub id: PlaylistId,
    #[serde(default)]
    pub title: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_playlist_has_no_provider_id_and_a_row_id_to_open_it() {
        let page: PlaylistPage = serde_json::from_str(
            r#"{"playlist":{"id":"8fa2d4a1-17f3-459a-8482-524a0d0ee1d9",
                "deezer_id":null,"title":"Horizon","description":"","cover":null,
                "owner":"bench","role":"owner","editable":true,"can_manage":true,
                "admin":true,"is_favorite":false,"nb_tracks":0,"private_tracks":0,
                "members":[],"members_version":1,"version":1,
                "last_edited_at":null,"last_edited_by":null},
                "tracks":[]}"#,
        )
        .expect("playlist page");
        assert!(page.playlist.deezer_id.is_none());
        assert_eq!(
            page.playlist.id.as_str(),
            "8fa2d4a1-17f3-459a-8482-524a0d0ee1d9"
        );
    }

    #[test]
    fn a_shared_track_keeps_the_ordinary_track_fields_and_adds_its_capability() {
        let shared: SharedTrack = serde_json::from_str(
            r#"{"id":"b8a1e106","deezer_id":"3000168","title":"Été","duration":183,
                "explicit":false,"unavailable":false,"gain":null,
                "artist":{"deezer_id":"1000003","name":"Gold 3"},
                "artists":[{"deezer_id":"1000003","name":"Gold 3","role":"Main"}],
                "display_artist":"Gold 3",
                "album":{"deezer_id":"2000044","title":"Fire","cover":null},
                "mid":"3000168",
                "cover":"/api/shared/playlist/tok/cover/3000168",
                "streamable":false}"#,
        )
        .expect("shared track");
        assert_eq!(shared.track.title, "Été");
        assert_eq!(shared.mid.as_deref(), Some("3000168"));
        assert!(!shared.streamable);
    }
}
