//! The person's library: favourites, followed artists, albums, uploads,
//! dislikes, and the mutations a row's menu makes.

use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;

use crate::client::Api;
use crate::error::ApiError;
use crate::lanes::Lane;
use nz_model::{FavoriteIds, MyAlbums, MyArtists, MyDislikes, MyFavorites, MyLocal, PlaylistList};

/// `POST /api/download`'s answer: how much of the batch the archive queue took.
/// `dropped` is not an error — the queue is bounded on purpose, and reporting it
/// beats pretending the whole batch is coming.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct DownloadQueued {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub queued: u32,
    #[serde(default)]
    pub dropped: u32,
    #[serde(default)]
    pub busy: bool,
}

impl Api {
    /// `GET /api/me/playlists` — the playlists and their folders.
    pub async fn my_playlists(&self) -> Result<PlaylistList, ApiError> {
        self.get_value("/me/playlists").await
    }

    /// `GET /api/me/favorites` — the starred tracks.
    pub async fn my_favorites(&self) -> Result<MyFavorites, ApiError> {
        self.get_value("/me/favorites").await
    }

    /// `GET /api/me/artists` — followed artists, with plays and library size.
    pub async fn my_artists(&self) -> Result<MyArtists, ApiError> {
        self.get_value("/me/artists").await
    }

    /// `GET /api/me/albums` — the starred albums.
    pub async fn my_albums(&self) -> Result<MyAlbums, ApiError> {
        self.get_value("/me/albums").await
    }

    /// `GET /api/me/local` — the files uploaded to this server.
    pub async fn my_local(&self) -> Result<MyLocal, ApiError> {
        self.get_value("/me/local").await
    }

    /// `GET /api/me/favorite-ids` — the ids only, for every star toggled at
    /// start (a fraction of the payload of the lists themselves).
    pub async fn favorite_ids(&self) -> Result<FavoriteIds, ApiError> {
        self.get_value("/me/favorite-ids").await
    }

    /// `GET /api/me/dislikes` — the "Je n'aime pas" list.
    pub async fn my_dislikes(&self) -> Result<MyDislikes, ApiError> {
        self.get_value("/me/dislikes").await
    }

    /// `POST /api/favorite` — star or unstar one track. `track` is the reference
    /// the server resolves (`trackkey.js#trackRef`: a Deezer number, a row UUID,
    /// or a catalogue id).
    pub async fn favorite(&self, track: &str, on: bool) -> Result<(), ApiError> {
        let body = serde_json::json!({ "id": track, "on": on });
        self.send_none(Method::POST, "/favorite", &body).await
    }

    /// `POST /api/favorite/<kind>` — star an album, an artist or a playlist.
    ///
    /// A CATALOGUE result the library does not hold yet is named by
    /// `deezer_id` (digits), a row by `id`; `meta` ({title, artist, artist_id})
    /// is what makes the row when the star is the first thing that ever named
    /// it. The choice of key is the SPA's own regex, ported.
    pub async fn favorite_entity(
        &self,
        kind: &str,
        reference: &str,
        on: bool,
        meta: &Value,
    ) -> Result<(), ApiError> {
        let key = if reference.bytes().all(|byte| byte.is_ascii_digit()) {
            "deezer_id"
        } else {
            "id"
        };
        let path = format!("/favorite/{}", crate::client::encode(kind));
        let body = crate::endpoints::with(
            crate::endpoints::with(meta.clone(), key, Value::String(reference.to_owned())),
            "on",
            Value::Bool(on),
        );
        self.send_none(Method::POST, &path, &body).await
    }

    /// `POST /api/dislike` — the title leaves the Flow, the radios and the
    /// mixes, with what sounds almost the same. `on: false` brings it back.
    pub async fn dislike(&self, reference: &str, on: bool) -> Result<(), ApiError> {
        let body = serde_json::json!({ "id": reference, "on": on });
        self.send_none(Method::POST, "/dislike", &body).await
    }

    /// `POST /api/download` — pre-archive a set of tracks now.
    ///
    /// `why` says what the request IS, so the server orders its queue:
    /// `"prefetch"` for the track about to play, `"bulk"` (the default) for a
    /// playlist somebody asked to keep. Never a number — every caller would
    /// claim to be the urgent one. Queueing is instant, which is why this is a
    /// User request: what it QUEUES is the background work.
    pub async fn download<I, S>(&self, ids: I, why: &str) -> Result<DownloadQueued, ApiError>
    where
        I: IntoIterator<Item = S>,
        S: ToString,
    {
        let body = serde_json::json!({
            "ids": crate::endpoints::id_list(ids),
            "why": why,
        });
        self.send_json(Method::POST, "/download", &body).await
    }

    /// A `GET` of one screen's data on the Prefetch lane, for the library lists
    /// a navigation is about to show.
    pub async fn prefetch_library<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, ApiError> {
        self.fetch(Lane::Prefetch, path).await
    }
}
