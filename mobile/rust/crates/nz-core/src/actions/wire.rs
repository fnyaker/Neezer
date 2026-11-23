//! The wire of the track-actions area: the routes `lib/actions.js`,
//! `lib/replace.js`, `components/PlaylistPicker.svelte` and
//! `components/ReplaceSheet.svelte` call, and the conversions into the
//! screen-shaped records.
//!
//! | This file | The SPA |
//! |---|---|
//! | [`favorite`] | `lib/actions.js#toggleFavorite` → `api.favorite` |
//! | [`dislike`] | `lib/actions.js#toggleDislike` → `api.dislike` |
//! | [`playlists`] | `PlaylistPicker.svelte#loadList` → `userPlaylists()` → `api.myPlaylists` |
//! | [`add_to_playlist`] | `lib/actions.js#addTrackToPlaylist` |
//! | [`create_playlist`] | `PlaylistPicker.svelte#submit` → `createPlaylistFrom` |
//! | [`replace_candidates`] | `ReplaceSheet.svelte#load` |
//! | [`replace`] | `lib/replace.js#applyReplacement` |
//! | [`replace_status`] | `lib/replace.js#pollJob` |
//! | [`delete_tracks`] | `UnavailableList.svelte#deleteSelected` (`/tracks/delete`) |
//! | [`delete_upload`] | `/api/local/<id>`, the server's own "delete one of your uploads" |
//! | [`share_link`] | `ShareSheet.svelte`'s link / `playlist_share.py` |
//!
//! **Every call is `Lane::User` work.** Each one is a mutation the person just
//! asked for (a menu entry they chose) or the sheet they just opened; nothing
//! here is background. The favourites' POST also archives the title server-side,
//! which is `nz-api`'s own lane decision on the server, not this device's.
//!
//! The bodies are parsed into `nz-model`'s own types rather than through a
//! second, hand-rolled shape: `Track`, `Playlist` and `TrackList` are what every
//! other lot of the phone already parses, so a row has one reader.

use nz_api::{Api, Lane, Method};
use nz_model::{Playlist, Track};

use crate::core::CoreError;

/// A route argument, percent-encoded the way `encodeURIComponent` does it: a
/// hostile id (`a/b?c`) is one path segment, never another route.
pub(super) fn seg(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// `GET` one path as text, on the `User` lane. The raw body is what the picker
/// keeps on disk, so a cached list and a fresh one go through the same parser.
pub(super) async fn fetch_text(api: &Api, path: &str) -> Result<String, CoreError> {
    let bytes = api.bytes(Lane::User, path).await?;
    String::from_utf8(bytes).map_err(|error| CoreError::Internal {
        detail: format!("{path} : {error}"),
    })
}

/// `POST /api/favorite` — star or unstar one title. The server resolves the
/// reference the way `trackkey.js#trackRef` names it (a row UUID, or a
/// catalogue's provider id).
pub(super) async fn favorite(api: &Api, reference: &str, on: bool) -> Result<(), CoreError> {
    api.favorite(reference, on).await?;
    Ok(())
}

/// `POST /api/dislike` — the title leaves the Flow, the radios and the mixes.
pub(super) async fn dislike(api: &Api, reference: &str, on: bool) -> Result<(), CoreError> {
    api.dislike(reference, on).await?;
    Ok(())
}

/// `POST /api/playlist/<id>/tracks` — append. **No `version`**: the cross-screen
/// picker sends none, because an append is commutative (the SPA's own note, plan
/// §8.5), so a playlist edited on another device between the open and the tap
/// does not answer `409` for nothing.
pub(super) async fn add_to_playlist(
    api: &Api,
    playlist_id: &str,
    keys: &[String],
) -> Result<u32, CoreError> {
    let answer = api
        .add_to_playlist(playlist_id, keys, None)
        .await
        .map_err(CoreError::from)?;
    Ok(answer
        .get("added")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(keys.len() as u64) as u32)
}

/// `POST /api/playlists` — create one, with `keys` already in it (the picker's
/// "Créer et ajouter").
pub(super) async fn create_playlist(
    api: &Api,
    title: &str,
    keys: &[String],
) -> Result<Playlist, CoreError> {
    api.create_playlist(title, keys, None)
        .await
        .map_err(CoreError::from)
}

/// `GET /api/replace/candidates/<id>` — what could stand in for a dead title.
pub(super) async fn replace_candidates(
    api: &Api,
    reference: &str,
    query: &str,
    fallbacks: bool,
) -> Result<nz_model::ReplaceCandidates, CoreError> {
    api.replacement_candidates(reference, fallbacks, query)
        .await
        .map_err(CoreError::from)
}

/// `POST /api/replace` — swap it everywhere. Answers the job handle: the rewrite
/// runs on a server worker (rule 2), so this returns at once.
pub(super) async fn replace(api: &Api, from: &str, to: &str) -> Result<String, CoreError> {
    let answer = api.replace_track(from, to).await.map_err(CoreError::from)?;
    job_of(&answer)
}

/// `GET /api/replace/status/<job>` — how it is going.
pub(super) async fn replace_status(api: &Api, job: &str) -> Result<serde_json::Value, CoreError> {
    api.replace_status(job).await.map_err(CoreError::from)
}

/// `POST /api/tracks/delete` — the bulk deletion of titles that are gone from
/// both sources. Answers the job handle; each title is re-checked in turn.
pub(super) async fn delete_tracks(
    api: &Api,
    keys: &[String],
) -> Result<serde_json::Value, CoreError> {
    api.delete_tracks(keys.iter().cloned())
        .await
        .map_err(CoreError::from)
}

/// `DELETE /api/local/<id>` — one of YOUR OWN uploads, row and file. The server
/// refuses a shared-library row or a provider row (`403`), and another person's
/// upload reads as `404`; nothing here second-guesses it.
pub(super) async fn delete_upload(api: &Api, id: &str) -> Result<(), CoreError> {
    // A raw DELETE: `nz-api` has no typed route for this one, and building a
    // second HTTP client is what the crate exists to prevent (the lane, the
    // session cookie and the "never retried" rule all live there).
    let path = format!("/local/{}", seg(id));
    let body = serde_json::json!({});
    let _: Option<serde_json::Value> = api
        .send(Lane::User, Method::DELETE, &path, Some(&body))
        .await?;
    Ok(())
}

/// `GET /api/playlist/<id>/share` then, only when there is no live link,
/// `POST` — the public read-only address of a playlist. The "one per playlist is
/// enough" rule is the server's, so a second open costs one GET and no new link.
pub(super) async fn playlist_share_link(api: &Api, id: &str) -> Result<String, CoreError> {
    let existing = api.playlist_share(id).await?;
    if let Some(link) = existing.link.filter(|link| !link.url.is_empty()) {
        return Ok(absolute_hint(&link.url, &link.token));
    }
    let created = api.create_playlist_share(id, None).await?;
    let link = created.link.ok_or_else(|| CoreError::Invalid {
        detail: "le serveur n'a pas rendu de lien de partage".to_owned(),
    })?;
    if link.url.is_empty() {
        return Err(CoreError::Invalid {
            detail: "le serveur n'a pas rendu d'adresse de partage".to_owned(),
        });
    }
    Ok(absolute_hint(&link.url, &link.token))
}

/// `GET /api/track/<id>/share` then, only when there is no live link, `POST`
/// — the public page of ONE track. The server's one-link-per-track rule is
/// what makes a second open cost one GET and no new grant.
pub(super) async fn track_share_link(api: &Api, id: &str) -> Result<String, CoreError> {
    let existing = api.track_share(id).await?;
    if let Some(link) = existing.link.filter(|link| !link.url.is_empty()) {
        return Ok(link.url);
    }
    let created = api.create_track_share(id).await?;
    let link = created.link.ok_or_else(|| CoreError::Invalid {
        detail: "le serveur n'a pas rendu de lien de partage".to_owned(),
    })?;
    if link.url.is_empty() {
        if link.token.is_empty() {
            return Err(CoreError::Invalid {
                detail: "le serveur n'a pas rendu d'adresse de partage".to_owned(),
            });
        }
        return Ok(format!("/t/{}", seg(&link.token)));
    }
    Ok(link.url)
}

/// The address a share answer carries, made whole: the server sends a page path
/// (`/pl/<token>`, `/shared/playlist/<token>`), and what a person pastes into a
/// message is an absolute URL — [`Core::share_link`] prefixes the origin.
fn absolute_hint(url: &str, token: &str) -> String {
    let whole = url.contains("://") || token.is_empty() || url.contains(token);
    if whole {
        url.to_owned()
    } else {
        format!(
            "{}/shared/playlist/{}",
            url.trim_end_matches('/'),
            seg(token)
        )
    }
}

/// The `job` handle a mutation answer carries.
fn job_of(answer: &serde_json::Value) -> Result<String, CoreError> {
    answer
        .get("job")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| CoreError::Invalid {
            detail: "le serveur n'a pas rendu de travaux".to_owned(),
        })
}

/// A candidate, as the sheet's row draws it. A candidate is a track dict (the
/// server's `_track_api`), sometimes with `local: true` (a file already here) and
/// sometimes with a fallback `source` (`youtube`).
pub(super) fn candidate(value: &serde_json::Value) -> Option<super::ReplaceCandidate> {
    let track: Track = serde_json::from_value(value.clone()).ok()?;
    Some(super::ReplaceCandidate {
        key: row_key(&track),
        id: track.deezer_id.to_string(),
        title: track.title.clone(),
        artists: artists_line(&track),
        album: track.album.title.clone(),
        album_id: None,
        duration_s: track.duration,
        cover: track.album.cover.clone(),
        source: track
            .source
            .clone()
            .unwrap_or_else(|| if track.local { "local" } else { "deezer" }.to_owned()),
        local: track.local,
    })
}

/// The display line the SPA's `format.js#artistLine` builds, already joined.
pub(super) fn artists_line(track: &Track) -> String {
    if let Some(display) = track
        .display_artist
        .as_ref()
        .filter(|line| !line.is_empty())
    {
        return display.clone();
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

/// A track's row key: `<source>:<id>` like every other row of this app, where
/// the id is what the server resolves (`trackkey.js#trackRef`: the row's UUID
/// when it has one, else the provider id).
pub(super) fn row_key(track: &Track) -> String {
    let id = track
        .id
        .as_ref()
        .map(|id| id.as_str().to_owned())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| track.deezer_id.to_string());
    format!("{}:{id}", source_of(track))
}

/// `deezer`, `youtube`, `upload`, `local` — what the row's source chip says.
pub(super) fn source_of(track: &Track) -> &'static str {
    if let Some(source) = track.source.as_deref().filter(|source| !source.is_empty()) {
        return match source {
            "youtube" => "youtube",
            "deezer" => "deezer",
            _ => "local",
        };
    }
    if track.local { "local" } else { "deezer" }
}
