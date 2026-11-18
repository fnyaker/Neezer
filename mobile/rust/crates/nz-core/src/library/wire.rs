//! The wire of the library area: the ten reads the SPA's library opens with, the
//! conversions into the screen-shaped records, and the query strings the routes
//! take.
//!
//! | This file | The SPA |
//! |---|---|
//! | [`fetch_playlists`] | `routes/Library.svelte#onMount` → `userPlaylists()` (`/me/playlists`) |
//! | [`fetch_favorites`] | `#onMount` → `loadMyFavorites()` (`/me/favorites`) |
//! | [`fetch_favorite_ids`] | `lib/actions.js#loadFavoriteIds` (`/me/favorite-ids`) |
//! | [`fetch_artists`] | the « Artistes » tab, `#loadArtists` |
//! | [`fetch_albums`] | the « Albums » tab, `#loadAlbums` |
//! | [`fetch_local`] | `#loadLocal` (« Mes fichiers ») |
//! | [`fetch_dislikes`] | `components/DislikedTracks.svelte#load` |
//! | [`fetch_unavailable`] | the « Indisponibles » leaf, `#loadGone` |
//! | [`fetch_upload_usage`] | `#loadUsage` (the quota bar) |
//! | [`fetch_history`] | `routes/History.svelte#loadFirst`/`#loadMore` |
//!
//! Every call is `Lane::User` work: it is the screen the person just opened, or
//! the leaf they just tapped (rule 1). Bodies are read as BYTES and parsed here
//! rather than through the typed endpoints, for one reason: the raw text is also
//! what the device keeps ([`super::store`]), so a cached list and a live one go
//! through exactly the same converter — a cache hit cannot drift from an answer.
//!
//! No rule lives here: what a row SHOWS is the SPA's `lib/format.js#artistLine`
//! and `lib/trackkey.js#trackKey`, restated once for every list in the app.

use serde::de::DeserializeOwned;

use nz_api::{Api, Lane};
use nz_model::{
    Album, AlbumList, Artist, ArtistList, FavoriteIds, History, HistoryCursor, MyFavorites,
    MyLocal, Playlist, PlaylistFolder, PlaylistList, Track, TrackList, UploadUsage,
};

use super::{
    AlbumCard, ArtistRow, FolderCard, HistoryEventRow, HistoryPage, HistorySummaryRow,
    PlaylistCard, TopArtist, TopTrack, UploadUsageRow,
};
use crate::core::CoreError;
use crate::lists::Row;

/// `GET` one path as text, on the `User` lane. The request and its body bytes
/// are counted by `nz-api`'s own per-tag meter (SC3), so `NzCore::stats()`
/// answers what this list cost.
pub(super) async fn fetch_text(api: &Api, path: &str) -> Result<String, CoreError> {
    let bytes = api.bytes(Lane::User, path).await?;
    String::from_utf8(bytes).map_err(|error| CoreError::Internal {
        detail: format!("{path} : {error}"),
    })
}

/// A body, parsed. A refusal to parse is an internal fault with the path in it —
/// never a silent empty list.
pub(super) fn parse<T: DeserializeOwned>(path: &str, body: &str) -> Result<T, CoreError> {
    serde_json::from_str(body).map_err(|error| CoreError::Internal {
        detail: format!("{path} : {error}"),
    })
}

/// A route argument, percent-encoded the way `encodeURIComponent` does it.
///
/// Every id here is a UUID today; encoding anyway is what makes a hostile
/// argument (a `/`, a `?`, a `#`) one query value instead of another route.
pub(super) fn enc(value: &str) -> String {
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

/// `GET /api/me/playlists` — the playlists and the folders they sit in, one
/// answer (the SPA reads both from this route).
pub(super) async fn fetch_playlists(api: &Api) -> Result<String, CoreError> {
    fetch_text(api, "/me/playlists").await
}

/// `GET /api/me/favorites` — every starred title, newest star first.
pub(super) async fn fetch_favorites(api: &Api) -> Result<String, CoreError> {
    fetch_text(api, "/me/favorites").await
}

/// `GET /api/me/favorite-ids` — the ids only: the cheap route the car's counter
/// and every heart read.
pub(super) async fn fetch_favorite_ids(api: &Api) -> Result<String, CoreError> {
    fetch_text(api, "/me/favorite-ids").await
}

/// `GET /api/me/artists` — followed artists, with plays and library size.
pub(super) async fn fetch_artists(api: &Api) -> Result<String, CoreError> {
    fetch_text(api, "/me/artists").await
}

/// `GET /api/me/albums` — kept albums, newest star first.
pub(super) async fn fetch_albums(api: &Api) -> Result<String, CoreError> {
    fetch_text(api, "/me/albums").await
}

/// `GET /api/me/local` — the files this person uploaded (the server caps it).
pub(super) async fn fetch_local(api: &Api) -> Result<String, CoreError> {
    fetch_text(api, "/me/local").await
}

/// `GET /api/me/dislikes` — the titles the person said they do not like.
pub(super) async fn fetch_dislikes(api: &Api) -> Result<String, CoreError> {
    fetch_text(api, "/me/dislikes").await
}

/// `GET /api/unavailable` — the titles their provider no longer has.
pub(super) async fn fetch_unavailable(api: &Api) -> Result<String, CoreError> {
    fetch_text(api, "/unavailable").await
}

/// `GET /api/upload/usage` — the person's uploads against their quota.
pub(super) async fn fetch_upload_usage(api: &Api) -> Result<String, CoreError> {
    fetch_text(api, "/upload/usage").await
}

/// The path one history page asks: a keyset cursor, never a page number
/// (`routes/History.svelte#loadFirst`), so an event recorded while somebody
/// reads page one cannot push one off page two.
pub(super) fn history_path(
    outcome: Option<&str>,
    before: Option<&str>,
    before_id: Option<i64>,
    limit: u32,
) -> String {
    let mut path = format!("/history?limit={limit}");
    if let Some(outcome) = outcome.filter(|value| !value.is_empty()) {
        path.push_str("&outcome=");
        path.push_str(&enc(outcome));
    }
    // Both halves of the cursor travel together or not at all: one without the
    // other is a `400` on the server, and a half cursor would restart the list.
    if let (Some(before), Some(id)) = (before, before_id) {
        path.push_str("&before=");
        path.push_str(&enc(before));
        path.push_str("&before_id=");
        path.push_str(&id.to_string());
    }
    path
}

/// `GET /api/history?…` — the caller's own listens, newest first, with the
/// 30-day summary.
pub(super) async fn fetch_history(api: &Api, path: &str) -> Result<String, CoreError> {
    fetch_text(api, path).await
}

// -- conversions --------------------------------------------------------------

/// `lib/format.js#artistLine`: who performed it, joined for a single line.
fn artist_line(track: &Track) -> String {
    if let Some(line) = track
        .display_artist
        .as_deref()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        return line.to_owned();
    }
    let names: Vec<&str> = track
        .artists
        .iter()
        .map(|artist| artist.name.trim())
        .filter(|name| !name.is_empty())
        .collect();
    if !names.is_empty() {
        return names.join(", ");
    }
    track.artist.name.clone()
}

/// What the source chip says (`lib/sources.js#sourceTitle`'s four cases).
fn source_of(track: &Track) -> &'static str {
    match track.source.as_deref() {
        Some("youtube") => "youtube",
        _ if track.local => "upload",
        _ => "deezer",
    }
}

/// `lib/trackkey.js#trackKey`: the row's UUID when it has one (the same key the
/// queue and every other list use), else the provider id.
fn track_key(track: &Track) -> (String, String) {
    let provider = track.deezer_id.to_string();
    let row = track.id.as_ref().map(|id| id.to_string());
    let key = row.clone().unwrap_or_else(|| provider.clone());
    (key, row.unwrap_or(provider))
}

/// One catalogue or library title, as every list in this area draws it.
pub(super) fn track_row(track: &Track, position: u32, liked: bool) -> Row {
    let (key, id) = track_key(track);
    Row {
        key,
        id,
        title: track.title.clone(),
        artists: artist_line(track),
        album: track.album.title.clone(),
        album_id: (!track.album.deezer_id.to_string().is_empty())
            .then(|| track.album.deezer_id.to_string()),
        duration_s: track.duration,
        cover: track.album.cover.clone(),
        explicit: track.explicit,
        liked,
        disliked: false,
        unavailable: track.unavailable,
        downloaded: false,
        source: source_of(track).to_owned(),
        position,
        played_at: 0,
        added_at: 0,
    }
}

/// A title list, in the server's own order. `liked` is the list's own answer:
/// the favourites are stars by definition, every other list is not.
pub(super) fn track_rows(tracks: &[Track], liked: bool) -> Vec<Row> {
    tracks
        .iter()
        .enumerate()
        .map(|(position, track)| track_row(track, position as u32, liked))
        .collect()
}

impl From<&Playlist> for PlaylistCard {
    fn from(playlist: &Playlist) -> Self {
        let role = playlist
            .role
            .clone()
            .filter(|role| !role.is_empty())
            .unwrap_or_else(|| "owner".to_owned());
        PlaylistCard {
            id: playlist.id.to_string(),
            deezer_id: playlist.deezer_id.clone().filter(|id| !id.is_empty()),
            title: playlist.title.clone(),
            cover: playlist.cover.clone(),
            tracks: playlist.nb_tracks,
            // `lib/actions.js#isSharedPlaylist`: a role that is not the owner's.
            shared: role != "owner",
            role,
            owner: playlist.owner.clone().unwrap_or_default(),
            editable: playlist.editable,
            can_manage: playlist.can_manage,
            folder_id: playlist.folder_id.clone().filter(|id| !id.is_empty()),
            position: playlist.position.map(i64::from),
        }
    }
}

impl From<&PlaylistFolder> for FolderCard {
    fn from(folder: &PlaylistFolder) -> Self {
        FolderCard {
            id: folder.id.clone(),
            title: folder.title.clone(),
            position: i64::from(folder.position.unwrap_or(0)),
            count: folder.nb_playlists.unwrap_or(0),
        }
    }
}

/// One followed artist, as the « Artistes » grid draws it. `starred` is the
/// ISO date (`/me/artists` only) that « Ajoutés récemment » sorts by.
pub(super) fn artist_row(artist: &Artist) -> ArtistRow {
    ArtistRow {
        id: artist
            .id
            .clone()
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| artist.deezer_id.to_string()),
        name: artist.name.clone(),
        picture: artist.picture.clone(),
        tracks: artist.tracks.unwrap_or(0),
        plays: artist.plays.unwrap_or(0),
        starred: artist.starred.clone().unwrap_or_default(),
    }
}

pub(super) fn artist_rows(answer: &ArtistList) -> Vec<ArtistRow> {
    answer.artists.iter().map(artist_row).collect()
}

/// One kept album, as the « Albums » grid card draws it.
pub(super) fn album_card(album: &Album) -> AlbumCard {
    let provider = album.deezer_id.to_string();
    AlbumCard {
        id: album
            .id
            .as_ref()
            .map(|id| id.to_string())
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| provider.clone()),
        title: album.title.clone(),
        cover: album.cover.clone(),
        artist: album.artist.name.clone(),
        year: album.year.clone().unwrap_or_default(),
        tracks: album.nb_tracks,
    }
}

pub(super) fn album_cards(answer: &AlbumList) -> Vec<AlbumCard> {
    answer.albums.iter().map(album_card).collect()
}

impl From<&UploadUsage> for UploadUsageRow {
    fn from(usage: &UploadUsage) -> Self {
        UploadUsageRow {
            used: usage.used,
            quota: usage.quota,
            unlimited: usage.unlimited,
        }
    }
}

/// The starred ids, as the light route answers them. A dislike is kept apart on
/// purpose (`FavoriteIds`'s own comment): it is not a star.
pub(super) fn favorite_ids_from(answer: &FavoriteIds) -> super::FavoriteIdSet {
    super::FavoriteIdSet {
        tracks: answer.ids.clone(),
        albums: answer.albums.clone(),
        artists: answer.artists.clone(),
        playlists: answer.playlists.clone(),
        disliked: answer.disliked.clone(),
    }
}

/// One history page, converted. `_track` keeps the server's own payload for the
/// day groups the UI builds; the row is what a tap plays.
pub(super) fn history_page(answer: &History) -> HistoryPage {
    let events: Vec<HistoryEventRow> = answer
        .events
        .iter()
        .enumerate()
        .map(|(position, event)| HistoryEventRow {
            id: event.id,
            at: event.at.clone().unwrap_or_default(),
            listened: event.listened,
            outcome: event.outcome,
            source: event.source.clone(),
            track: track_row(&event.track, position as u32, false),
        })
        .collect();
    HistoryPage {
        events,
        next: answer.next.as_ref().map(cursor_of),
        summary: summary_of(&answer.summary),
    }
}

fn cursor_of(cursor: &HistoryCursor) -> super::HistoryCursorRow {
    super::HistoryCursorRow {
        before: cursor.before.clone(),
        before_id: cursor.before_id,
    }
}

fn summary_of(summary: &nz_model::HistorySummary) -> HistorySummaryRow {
    HistorySummaryRow {
        days: summary.days,
        listens: summary.listens,
        skips: summary.skips,
        minutes: summary.minutes,
        top_artists: summary
            .top_artists
            .iter()
            .map(|artist| TopArtist {
                id: artist.id.clone(),
                name: artist.name.clone(),
                cover: artist.cover.clone(),
                count: artist.count,
            })
            .collect(),
        top_tracks: summary
            .top_tracks
            .iter()
            .enumerate()
            .map(|(position, entry)| TopTrack {
                row: track_row(&entry.track, position as u32, true),
                count: entry.count,
            })
            .collect(),
    }
}

/// The three lists whose body is `{"tracks": […]}` — the favourites, the
/// uploads, the dislikes and the unavailable rows all answer it.
pub(super) fn parse_tracks(path: &str, body: &str) -> Result<Vec<Row>, CoreError> {
    let list: TrackList = parse(path, body)?;
    Ok(track_rows(&list.tracks, false))
}

/// The favourites keep a `liked` flag: they ARE the stars.
pub(super) fn parse_favorites(body: &str) -> Result<Vec<Row>, CoreError> {
    let list: MyFavorites = parse("/me/favorites", body)?;
    Ok(track_rows(&list.tracks, true))
}

pub(super) fn parse_local(body: &str) -> Result<Vec<Row>, CoreError> {
    let list: MyLocal = parse("/me/local", body)?;
    Ok(track_rows(&list.tracks, false))
}

pub(super) fn parse_playlists(body: &str) -> Result<PlaylistList, CoreError> {
    parse("/me/playlists", body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_model::{AlbumId, TrackAlbum, TrackArtist, TrackId};

    fn track(title: &str, artists: &[&str]) -> Track {
        Track {
            deezer_id: TrackId::new("3000168"),
            title: title.to_owned(),
            duration: 200,
            artist: TrackArtist {
                deezer_id: nz_model::ArtistId::new("42"),
                name: artists.first().copied().unwrap_or_default().to_owned(),
            },
            artists: artists
                .iter()
                .map(|name| nz_model::TrackArtistRole {
                    deezer_id: nz_model::ArtistId::new(""),
                    name: (*name).to_owned(),
                    role: "Main".to_owned(),
                })
                .collect(),
            album: TrackAlbum {
                deezer_id: AlbumId::new("7"),
                title: "Album".to_owned(),
                cover: Some("/api/cover/7".to_owned()),
            },
            ..Track::default()
        }
    }

    /// A row is keyed the way the queue and every list key it
    /// (`trackkey.js#trackKey`): the row's UUID, else the provider's id; and the
    /// source chip says which of the four kinds it is.
    #[test]
    fn a_title_becomes_the_row_every_list_draws() {
        let row = track_row(&track("Titre", &["Artiste"]), 3, true);
        assert_eq!(row.key, "3000168");
        assert_eq!(row.id, "3000168", "the id /stream accepts");
        assert_eq!(row.position, 3);
        assert_eq!(row.artists, "Artiste");
        assert_eq!(row.source, "deezer");
        assert!(row.liked, "the favourites are stars by definition");

        let mut library = track("Titre", &["Artiste"]);
        library.id = Some(nz_model::TrackRowId::new(
            "11111111-1111-4111-8111-111111111111",
        ));
        let row = track_row(&library, 0, false);
        assert_eq!(
            row.key, "11111111-1111-4111-8111-111111111111",
            "a library row is keyed by its UUID"
        );
        assert!(!row.liked);

        let mut upload = track("Titre", &["Artiste"]);
        upload.local = true;
        assert_eq!(track_row(&upload, 0, false).source, "upload");
        let mut youtube = track("Titre", &["Artiste"]);
        youtube.source = Some("youtube".to_owned());
        assert_eq!(track_row(&youtube, 0, false).source, "youtube");
    }

    /// The artist line is the server's own when it sent one, else the credited
    /// names joined (`format.js#artistLine`).
    #[test]
    fn the_artist_line_is_one_sentence() {
        assert_eq!(artist_line(&track("T", &["A", "B"])), "A, B");
        let mut one = track("T", &["A"]);
        one.display_artist = Some("A feat. B".to_owned());
        assert_eq!(artist_line(&one), "A feat. B");
    }

    /// A history page keeps the server's cursor halves together, and its rows
    /// carry what a row draws (time, seconds listened, verdict).
    #[test]
    fn a_history_page_keeps_its_cursor_and_its_verdicts() {
        let body = r#"{
            "events": [
                {"id": 9, "at": "2026-11-17T21:04:00", "listened": 187, "outcome": 1,
                 "source": "album", "track": {"deezer_id": "3000168", "title": "T",
                 "duration": 200, "artist": {"name": "A"},
                 "album": {"deezer_id": "7", "title": "Album"}}},
                {"id": 8, "at": "2026-11-17T20:58:00", "listened": 12, "outcome": -1,
                 "track": {"deezer_id": "3000169", "title": "S", "duration": 90,
                 "artist": {"name": "B"}, "album": {"deezer_id": "8", "title": "Autre"}}}
            ],
            "next": {"before": "2026-11-17T20:58:00", "before_id": 8},
            "summary": {"days": 30, "listens": 41, "skips": 3, "minutes": 220,
                        "top_artists": [{"id": "42", "name": "A", "count": 9}],
                        "top_tracks": [{"count": 4, "track": {"deezer_id": "3000168",
                                        "title": "T", "artist": {"name": "A"},
                                        "album": {"title": "Album"}}}]}
        }"#;
        let answer: History = parse("/history", body).expect("the server's own shape");
        let page = history_page(&answer);
        assert_eq!(page.events.len(), 2);
        assert_eq!(page.events[0].listened, 187);
        assert_eq!(page.events[0].outcome, 1);
        assert_eq!(page.events[1].outcome, -1);
        assert_eq!(page.events[0].track.title, "T");
        assert_eq!(page.events[0].track.position, 0);
        assert_eq!(
            page.next,
            Some(super::super::HistoryCursorRow {
                before: "2026-11-17T20:58:00".to_owned(),
                before_id: 8
            })
        );
        assert_eq!(page.summary.listens, 41);
        assert_eq!(page.summary.top_artists.len(), 1);
        assert_eq!(page.summary.top_tracks[0].count, 4);
    }

    /// The query a page asks is the SPA's own: `limit`, the filter, and BOTH
    /// halves of the cursor together.
    #[test]
    fn the_history_path_carries_both_halves_of_the_cursor() {
        assert_eq!(history_path(None, None, None, 50), "/history?limit=50");
        assert_eq!(
            history_path(Some("skip"), None, None, 50),
            "/history?limit=50&outcome=skip"
        );
        assert_eq!(
            history_path(Some(""), Some("2026-11-17T20:58:00"), Some(8), 50),
            "/history?limit=50&before=2026-11-17T20%3A58%3A00&before_id=8",
            "a filter of \"\" is no filter, and the timestamp is encoded"
        );
        assert_eq!(
            history_path(None, Some("2026-11-17T20:58:00"), None, 50),
            "/history?limit=50",
            "half a cursor is no cursor at all"
        );
    }

    /// A playlist row carries the two facts its card needs (`owner · 12 titres`)
    /// and the shared flag the sections split on.
    #[test]
    fn a_playlist_row_keeps_its_role_and_its_folder() {
        let body = r#"{"playlists": [
            {"id": "11111111-1111-4111-8111-111111111111", "title": "Route",
             "nb_tracks": 12, "role": "editor", "owner": "Nadia",
             "folder_id": "22222222-2222-4222-8222-222222222222", "position": 3},
            {"id": "33333333-3333-4333-8333-333333333333", "title": "À moi",
             "nb_tracks": 0}
        ], "folders": [{"id": "22222222-2222-4222-8222-222222222222",
                         "title": "Voyage", "position": 0, "nb_playlists": 1}]}"#;
        let answer = parse_playlists(body).expect("the server's own shape");
        let shared = PlaylistCard::from(&answer.playlists[0]);
        assert!(shared.shared, "an editor's list is shared with them");
        assert_eq!(shared.owner, "Nadia");
        assert_eq!(shared.tracks, 12);
        assert_eq!(shared.position, Some(3));
        assert_eq!(
            shared.folder_id.as_deref(),
            Some("22222222-2222-4222-8222-222222222222")
        );
        let mine = PlaylistCard::from(&answer.playlists[1]);
        assert!(
            !mine.shared,
            "a role the server did not send is the owner's"
        );
        assert_eq!(mine.role, "owner");
        let folder = FolderCard::from(&answer.folders[0]);
        assert_eq!(folder.title, "Voyage");
        assert_eq!(folder.count, 1);
        assert_eq!(folder.position, 0);
    }
}
