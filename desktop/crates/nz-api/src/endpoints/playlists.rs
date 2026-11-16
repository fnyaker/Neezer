//! Playlists: the page, its edits, its members, its public link, its
//! recommendations, and importing one from another service's link.
//!
//! Every edit carries the version the screen had (`version` /
//! `members_version`), which is how two people editing the same playlist fail
//! loudly instead of silently overwriting each other. The SPA sends `null` when
//! it has none; so does this.

use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;

use crate::client::{Api, encode};
use crate::endpoints::{query, with};
use crate::error::ApiError;
use nz_model::{
    Playlist, PlaylistMembers, PlaylistPage, PlaylistRecommendations, PlaylistShare,
    PlaylistVersion, SharedPlaylist,
};

/// `POST /api/import/playlist`'s answer: the job to poll.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ImportJob {
    #[serde(default)]
    pub id: String,
}

impl Api {
    /// `GET /api/playlist/<id>` — the playlist and its tracks.
    pub async fn playlist(&self, id: &str) -> Result<PlaylistPage, ApiError> {
        self.get_value(&format!("/playlist/{}", encode(id))).await
    }

    /// `POST /api/playlists` — create one, optionally with its first tracks and
    /// inside a folder.
    pub async fn create_playlist(
        &self,
        title: &str,
        tracks: &[String],
        folder: Option<&str>,
    ) -> Result<Playlist, ApiError> {
        let body = serde_json::json!({ "title": title, "tracks": tracks, "folder": folder });
        self.send_json(Method::POST, "/playlists", &body).await
    }

    /// `PATCH /api/playlist/<id>` — title, description, privacy, folder… with
    /// the version the screen had.
    pub async fn edit_playlist(
        &self,
        id: &str,
        fields: &Value,
        version: Option<u32>,
    ) -> Result<Value, ApiError> {
        let path = format!("/playlist/{}", encode(id));
        let body = match version {
            Some(version) => with(fields.clone(), "version", Value::from(version)),
            None => fields.clone(),
        };
        self.send_json(Method::PATCH, &path, &body).await
    }

    /// `DELETE /api/playlist/<id>` — delete it, refusing a stale version.
    pub async fn delete_playlist(&self, id: &str, version: Option<u32>) -> Result<(), ApiError> {
        let path = format!("/playlist/{}", encode(id));
        let body = match version {
            Some(version) => serde_json::json!({ "version": version }),
            None => serde_json::json!({}),
        };
        self.send_none(Method::DELETE, &path, &body).await
    }

    /// `POST /api/playlist-folders` — a new folder, optionally with the
    /// client-chosen id a move needs.
    pub async fn create_playlist_folder(
        &self,
        title: &str,
        id: Option<&str>,
    ) -> Result<Value, ApiError> {
        let body = match id {
            Some(id) => serde_json::json!({ "id": id, "title": title }),
            None => serde_json::json!({ "title": title }),
        };
        self.send_json(Method::POST, "/playlist-folders", &body)
            .await
    }

    /// `PATCH /api/playlist-folders/<id>` — rename a folder.
    pub async fn rename_playlist_folder(&self, id: &str, title: &str) -> Result<(), ApiError> {
        let body = serde_json::json!({ "title": title });
        self.send_none(
            Method::PATCH,
            &format!("/playlist-folders/{}", encode(id)),
            &body,
        )
        .await
    }

    /// `DELETE /api/playlist-folders/<id>` — delete a folder (its playlists are
    /// kept).
    pub async fn delete_playlist_folder(&self, id: &str) -> Result<(), ApiError> {
        self.send_none(
            Method::DELETE,
            &format!("/playlist-folders/{}", encode(id)),
            &serde_json::json!({}),
        )
        .await
    }

    /// `PUT /api/playlist-folders/order` — the folders' order, as the person
    /// dropped them.
    pub async fn reorder_playlist_folders(&self, folders: &[String]) -> Result<(), ApiError> {
        let body = serde_json::json!({ "folders": folders });
        self.send_none(Method::PUT, "/playlist-folders/order", &body)
            .await
    }

    /// `PUT /api/playlist-folders/{<id>|root}/order` — the playlists' order
    /// inside one folder (or at the root).
    pub async fn reorder_playlist_container(
        &self,
        folder_id: Option<&str>,
        playlists: &[String],
    ) -> Result<(), ApiError> {
        let path = match folder_id {
            Some(id) => format!("/playlist-folders/{}/order", encode(id)),
            None => "/playlist-folders/root/order".to_owned(),
        };
        let body = serde_json::json!({ "playlists": playlists });
        self.send_none(Method::PUT, &path, &body).await
    }

    /// `POST /api/playlist/<id>/tracks` — add tracks, with the version.
    pub async fn add_to_playlist(
        &self,
        id: &str,
        tracks: &[String],
        version: Option<u32>,
    ) -> Result<Value, ApiError> {
        let path = format!("/playlist/{}/tracks", encode(id));
        let body = match version {
            Some(version) => serde_json::json!({ "tracks": tracks, "version": version }),
            None => serde_json::json!({ "tracks": tracks }),
        };
        self.send_json(Method::POST, &path, &body).await
    }

    /// `DELETE /api/playlist/<id>/tracks` — remove tracks.
    pub async fn remove_from_playlist(
        &self,
        id: &str,
        tracks: &[String],
        version: Option<u32>,
    ) -> Result<Value, ApiError> {
        let path = format!("/playlist/{}/tracks", encode(id));
        let body = match version {
            Some(version) => serde_json::json!({ "tracks": tracks, "version": version }),
            None => serde_json::json!({ "tracks": tracks }),
        };
        self.send_json(Method::DELETE, &path, &body).await
    }

    /// `DELETE /api/playlist/<id>/tracks` by ROW INDEX — what a list removes
    /// when the same track appears several times.
    pub async fn remove_playlist_indexes(
        &self,
        id: &str,
        indexes: &[u32],
        version: Option<u32>,
    ) -> Result<Value, ApiError> {
        let path = format!("/playlist/{}/tracks", encode(id));
        let body = match version {
            Some(version) => serde_json::json!({ "indexes": indexes, "version": version }),
            None => serde_json::json!({ "indexes": indexes }),
        };
        self.send_json(Method::DELETE, &path, &body).await
    }

    /// `PUT /api/playlist/<id>/order` — the whole tracklist, reordered.
    pub async fn reorder_playlist(
        &self,
        id: &str,
        tracks: &[String],
        version: Option<u32>,
    ) -> Result<Value, ApiError> {
        let path = format!("/playlist/{}/order", encode(id));
        let body = match version {
            Some(version) => serde_json::json!({ "tracks": tracks, "version": version }),
            None => serde_json::json!({ "tracks": tracks }),
        };
        self.send_json(Method::PUT, &path, &body).await
    }

    /// `GET /api/playlist/<id>/version` — the cheap poll a screen makes before
    /// it writes.
    pub async fn playlist_version(&self, id: &str) -> Result<PlaylistVersion, ApiError> {
        self.get_value(&format!("/playlist/{}/version", encode(id)))
            .await
    }

    /// `GET /api/playlist/<id>/members` — who shares it, and whether this
    /// person may manage them.
    pub async fn playlist_members(&self, id: &str) -> Result<PlaylistMembers, ApiError> {
        self.get_value(&format!("/playlist/{}/members", encode(id)))
            .await
    }

    /// `POST /api/playlist/<id>/members` — invite somebody by username.
    pub async fn invite_playlist_member(
        &self,
        id: &str,
        username: &str,
        role: &str,
        members_version: Option<u32>,
    ) -> Result<Value, ApiError> {
        let path = format!("/playlist/{}/members", encode(id));
        let body = match members_version {
            Some(version) => {
                serde_json::json!({ "username": username, "role": role, "members_version": version })
            }
            None => serde_json::json!({ "username": username, "role": role }),
        };
        self.send_json(Method::POST, &path, &body).await
    }

    /// `PATCH /api/playlist/<id>/members/<user>` — change somebody's role.
    pub async fn set_playlist_member_role(
        &self,
        id: &str,
        user_id: &str,
        role: &str,
        members_version: Option<u32>,
    ) -> Result<Value, ApiError> {
        let path = format!("/playlist/{}/members/{}", encode(id), encode(user_id));
        let body = match members_version {
            Some(version) => serde_json::json!({ "role": role, "members_version": version }),
            None => serde_json::json!({ "role": role }),
        };
        self.send_json(Method::PATCH, &path, &body).await
    }

    /// `DELETE /api/playlist/<id>/members/<user>` — remove somebody.
    pub async fn remove_playlist_member(
        &self,
        id: &str,
        user_id: &str,
        members_version: Option<u32>,
    ) -> Result<Value, ApiError> {
        let path = format!("/playlist/{}/members/{}", encode(id), encode(user_id));
        let body = match members_version {
            Some(version) => serde_json::json!({ "members_version": version }),
            None => serde_json::json!({}),
        };
        self.send_json(Method::DELETE, &path, &body).await
    }

    /// `POST /api/playlist/<id>/leave` — stop being a member (the owner cannot).
    pub async fn leave_playlist(&self, id: &str) -> Result<(), ApiError> {
        let path = format!("/playlist/{}/leave", encode(id));
        self.send_none(Method::POST, &path, &serde_json::json!({}))
            .await
    }

    /// `GET /api/playlist/<id>/share` — the public link, if one exists.
    pub async fn playlist_share(&self, id: &str) -> Result<PlaylistShare, ApiError> {
        self.get_value(&format!("/playlist/{}/share", encode(id)))
            .await
    }

    /// `POST /api/playlist/<id>/share` — mint (or rotate) the public link.
    /// `ttl` is seconds; `None` keeps the server's default.
    pub async fn create_playlist_share(
        &self,
        id: &str,
        ttl: Option<u32>,
    ) -> Result<PlaylistShare, ApiError> {
        let path = format!("/playlist/{}/share", encode(id));
        let body = match ttl {
            Some(ttl) => serde_json::json!({ "ttl": ttl }),
            None => serde_json::json!({}),
        };
        self.send_json(Method::POST, &path, &body).await
    }

    /// `DELETE /api/playlist/<id>/share` — the link stops working.
    pub async fn revoke_playlist_share(&self, id: &str) -> Result<(), ApiError> {
        let path = format!("/playlist/{}/share", encode(id));
        self.send_none(Method::DELETE, &path, &serde_json::json!({}))
            .await
    }

    /// `GET /api/shared/playlist/<token>` — the public page. No session is
    /// needed; the token IS the capability.
    pub async fn shared_playlist(&self, token: &str) -> Result<SharedPlaylist, ApiError> {
        self.get_value(&format!("/shared/playlist/{}", encode(token)))
            .await
    }

    /// `GET /api/playlist/<id>/recommendations?limit=` — the bottom of a
    /// playlist: library titles near its tracklist that it does not hold yet.
    pub async fn playlist_recommendations(
        &self,
        id: &str,
        limit: u32,
    ) -> Result<PlaylistRecommendations, ApiError> {
        let path = format!(
            "/playlist/{}/recommendations{}",
            encode(id),
            query(&[("limit", Some(limit.to_string()))])
        );
        self.get_value(&path).await
    }

    /// The path of a playlist's recommendations, for a prefetch or a log line.
    pub fn playlist_recommendations_path(id: &str, limit: u32) -> String {
        format!(
            "/playlist/{}/recommendations{}",
            encode(id),
            query(&[("limit", Some(limit.to_string()))])
        )
    }

    /// `POST /api/import/playlist` — read and match a shared link (Deezer,
    /// Spotify, YouTube, Apple Music) as a background job (rule 2).
    pub async fn import_playlist(&self, url: &str) -> Result<ImportJob, ApiError> {
        let body = serde_json::json!({ "url": url });
        self.send_json(Method::POST, "/import/playlist", &body)
            .await
    }

    /// `GET /api/import/playlist/<id>` — the import's progress and what it
    /// matched. Its failures are per track and carry a cause.
    pub async fn import_status(&self, id: &str) -> Result<Value, ApiError> {
        self.get_value(&format!("/import/playlist/{}", encode(id)))
            .await
    }

    /// `POST /api/import/playlist/<id>/save` — turn the matched set into a real
    /// playlist.
    pub async fn import_save(&self, id: &str, title: &str, keep: bool) -> Result<Value, ApiError> {
        let path = format!("/import/playlist/{}/save", encode(id));
        let body = serde_json::json!({ "title": title, "keep": keep });
        self.send_json(Method::POST, &path, &body).await
    }
}
