//! The wire of the car's tree: the four routes a head unit browses, and their
//! conversion into [`Item`]s.
//!
//! | Node | Route | The SPA's own reader |
//! |---|---|---|
//! | Favoris | `GET /api/me/favorites` | `lib/actions.js#loadMyFavorites` |
//! | Playlists | `GET /api/me/playlists` | `routes/Library.svelte#onMount` |
//! | one playlist | `GET /api/playlist/<id>` | `routes/Playlist.svelte#load` |
//! | Récents | `GET /api/history?limit=30` | `routes/History.svelte#loadFirst` |
//!
//! Bodies are read as BYTES and parsed here rather than through `nz-api`'s typed
//! endpoints for the reason MB8 gives in its own wire: the bytes are what the
//! device's copy is made from, so a cache hit and a live answer go through
//! exactly one converter.
//!
//! The lane is the caller's and is always [`nz_api::Lane::Prefetch`] from a
//! browse: nobody on the phone asked for this list (rule 1).

use nz_api::{Api, Lane};
use nz_model::{History, Playlist, PlaylistList, PlaylistPage, TrackList};

use super::{Item, RECENT_LIMIT, Request};
use crate::core::CoreError;

/// One route, read on `lane`, as text.
pub(super) async fn fetch(
    api: &Api,
    request: Request,
    argument: &str,
    lane: Lane,
) -> Result<String, CoreError> {
    let path = path_of(request, argument).ok_or_else(|| CoreError::Invalid {
        detail: "Ce nœud n'a pas de route".to_owned(),
    })?;
    let bytes = api.bytes(lane, &path).await?;
    String::from_utf8(bytes).map_err(|error| CoreError::Internal {
        detail: format!("{path} : {error}"),
    })
}

/// What the `auto` tag cost so far: `(requests, bytes in, bytes out)`.
///
/// `nz-api`'s own per-tag meter (SC3), read as a DELTA around one read so the
/// core's meter (`Core::stats`, which `NzCore::stats` answers) carries the same
/// number the screens and the acceptance quote.
pub(super) fn totals(api: &Api) -> (u64, u64, u64) {
    api.stats()
        .into_iter()
        .find(|row| row.tag == super::TAG_AUTO)
        .map(|row| (row.requests, row.bytes_in, row.bytes_out))
        .unwrap_or((0, 0, 0))
}

/// The route of a browsable node, or `None` for one that is served locally
/// (the root, a title, the Flow, the downloads).
fn path_of(request: Request, argument: &str) -> Option<String> {
    match request {
        Request::Favorites => Some("/me/favorites".to_owned()),
        Request::Playlists => Some("/me/playlists".to_owned()),
        Request::Recent => Some(format!("/history?limit={RECENT_LIMIT}")),
        Request::Playlist => Some(format!("/playlist/{}", enc(argument))),
        Request::Root | Request::Flow | Request::Downloads | Request::Track => None,
    }
}

/// A body, converted into the rows the car draws. A refusal to parse is an
/// internal fault with the route in it — never a silent empty list.
pub(super) fn items(body: &str, request: Request) -> Result<Vec<Item>, CoreError> {
    match request {
        Request::Favorites => {
            let answer = parse::<TrackList>("/me/favorites", body)?;
            Ok(answer
                .tracks
                .iter()
                .map(|track| Item::from_track(track, true))
                .collect())
        }
        Request::Playlists => {
            let answer = parse::<PlaylistList>("/me/playlists", body)?;
            let mut playlists: Vec<&Playlist> = answer.playlists.iter().collect();
            // The person's own lists first, then the ones shared with them, each
            // in the server's own order (see the module's "Ordering of the
            // playlists": the pinned-first rule waits for MB24's pin store).
            playlists.sort_by_key(|playlist| order_key(playlist));
            Ok(playlists.into_iter().map(Item::playlist).collect())
        }
        Request::Playlist => {
            let answer = parse::<PlaylistPage>("/playlist/<id>", body)?;
            Ok(answer
                .tracks
                .iter()
                .map(|track| Item::from_track(track, false))
                .collect())
        }
        Request::Recent => {
            let answer = parse::<History>("/history", body)?;
            let mut items: Vec<Item> = Vec::with_capacity(answer.events.len());
            // Newest first, one row per title: the SPA's own « récemment joués »
            // keeps a title once (`lib/stores.js#pushRecent`).
            for event in &answer.events {
                let item = Item::from_track(&event.track, false);
                if item.id.is_empty() || items.iter().any(|seen| seen.media_id == item.media_id) {
                    continue;
                }
                items.push(item);
            }
            Ok(items)
        }
        Request::Root | Request::Flow | Request::Downloads | Request::Track => Ok(Vec::new()),
    }
}

/// `(someone else's first?, its place, its name)`: the order a car's list reads
/// in, and a total order, so two browses never swap two rows.
fn order_key(playlist: &Playlist) -> (u8, u32, String) {
    let shared = !matches!(playlist.role.as_deref(), None | Some("") | Some("owner"));
    (
        u8::from(shared),
        playlist.position.unwrap_or(u32::MAX),
        playlist.title.clone(),
    )
}

fn parse<T: serde::de::DeserializeOwned>(route: &str, body: &str) -> Result<T, CoreError> {
    serde_json::from_str(body).map_err(|error| CoreError::Internal {
        detail: format!("{route} : {error}"),
    })
}

/// A route argument, percent-encoded the way `encodeURIComponent` does it, so a
/// hostile id (`/`, `?`, `#`) is one path segment instead of another route.
fn enc(value: &str) -> String {
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
