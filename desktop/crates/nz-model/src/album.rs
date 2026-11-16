//! Albums: the grid card, the page, a discography, and a release.

use serde::{Deserialize, Serialize};

use crate::ids::{AlbumId, AlbumRowId};
use crate::track::{Track, TrackArtist};

/// An album as a card or a page header carries it.
///
/// `year` is deliberately loose: a browse card built from Deezer's answer sends
/// the four first characters of `release_date` (a string), a card built from the
/// database sends `MIN(track.year)` (an integer). See [`crate::loose::opt_string`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Album {
    /// The row's UUID (`/api/cover/<id>` takes it). Absent from a catalogue card.
    #[serde(default)]
    pub id: Option<AlbumRowId>,
    #[serde(default)]
    pub deezer_id: AlbumId,
    #[serde(default)]
    pub title: String,
    /// Relative server path, or `null` when the server holds no art.
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default, deserialize_with = "crate::loose::opt_string")]
    pub year: Option<String>,
    #[serde(default)]
    pub nb_tracks: u32,
    #[serde(default)]
    pub artist: TrackArtist,
    /// `/api/album/<id>` only: whether the album is starred.
    #[serde(default)]
    pub is_favorite: Option<bool>,
    /// `"album"`, `"single"`, `"ep"` — set by the discography and search paths.
    #[serde(default)]
    pub record_type: Option<String>,
}

/// `/api/album/<id>` — the album, its tracks in disc/track order, and whether
/// the library holds it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlbumPage {
    #[serde(default)]
    pub album: Album,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub library: bool,
}

/// `{"albums": [...]}` — `/api/me/albums` and the grid pages.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlbumList {
    #[serde(default)]
    pub albums: Vec<Album>,
}

/// `/api/artist/<id>/discography` — one artist's releases, sorted newest first.
///
/// The server groups by release type and always adds `all`; the other three
/// arrays are absent until the artist has one of that type.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Discography {
    #[serde(default)]
    pub discography: DiscographyGroups,
}

/// The discography's groups: `all`, then one array per release type.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiscographyGroups {
    #[serde(default)]
    pub album: Vec<Album>,
    #[serde(default)]
    pub single: Vec<Album>,
    #[serde(default)]
    pub ep: Vec<Album>,
    #[serde(default)]
    pub all: Vec<Album>,
}

/// The release a `/api/releases` preview row belongs to.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReleaseRef {
    #[serde(default)]
    pub deezer_id: AlbumId,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cover: Option<String>,
    /// `"album"` or `"single"`.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub nb_tracks: Option<u32>,
    /// `YYYY-MM-DD`.
    #[serde(default)]
    pub released: Option<String>,
}

/// One row of `/api/releases`'s shelf.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Release {
    #[serde(default)]
    pub deezer_id: AlbumId,
    #[serde(default)]
    pub title: String,
    /// The artist's name, already flattened by the server; `artist` repeats it
    /// with its id.
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub artist: TrackArtist,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub nb_tracks: Option<u32>,
    /// `YYYY-MM-DD`.
    #[serde(default)]
    pub released: Option<String>,
    #[serde(default, deserialize_with = "crate::loose::opt_string")]
    pub year: Option<String>,
    /// Arrived with the latest check (what the screen badges).
    #[serde(default)]
    pub new: bool,
    /// On the shelf already (the second visit after "vu").
    #[serde(default)]
    pub seen: Option<bool>,
    /// "+N sorties" grouping: the name of the favourite this release is close
    /// to, when the row is a neighbour of one.
    #[serde(default)]
    pub close_to: Option<String>,
}

/// `/api/releases` — the shelf, its preview tracks and the unread counters.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Releases {
    #[serde(default)]
    pub releases: Vec<Release>,
    #[serde(default)]
    pub tracks: Vec<Track>,
    /// New releases the person has not opened yet.
    #[serde(default)]
    pub unseen: u32,
    /// Releases still being checked by the server.
    #[serde(default)]
    pub pending: u32,
}

/// `/api/releases/settings` — what the background check watches and when.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReleasesSettings {
    #[serde(default)]
    pub on: bool,
    /// Hours between two checks.
    #[serde(default)]
    pub every: u32,
    /// Days a release stays "new".
    #[serde(default)]
    pub window: u32,
    #[serde(default)]
    pub watched: u32,
    #[serde(default)]
    pub favourites: u32,
    #[serde(default)]
    pub links: u32,
    #[serde(default)]
    pub asked: u32,
    #[serde(default)]
    pub releases: u32,
    #[serde(default)]
    pub close: u32,
    #[serde(default)]
    pub listened: u32,
    #[serde(default)]
    pub last_run: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub running: bool,
}
