//! The library's own lists: favourites, followed artists, kept albums, uploads,
//! dislikes, unavailable rows, and the cheap id sets behind the hearts.

use serde::{Deserialize, Serialize};

use crate::album::AlbumList;
use crate::artist::ArtistList;
use crate::track::TrackList;

/// `/api/me/favorites` — the starred tracks, newest star first.
///
/// It is an ordinary [`TrackList`] on the wire; the alias exists so the screens
/// read `MyFavorites` and never confuse it with a playlist's rows.
pub type MyFavorites = TrackList;

/// `/api/me/local` — the uploads this person may play.
pub type MyLocal = TrackList;

/// `/api/me/dislikes` — the titles the person said they do not like.
pub type MyDislikes = TrackList;

/// `/api/unavailable` — the rows whose track is gone from its provider.
pub type Unavailable = TrackList;

/// `/api/me/albums` — the kept albums. Each row is an [`Album`]: a grid card
/// plus `is_favorite` when the album page's own shape was reused.
pub type MyAlbums = AlbumList;

/// `/api/me/artists` — the followed artists. Each row is an [`Artist`], which
/// carries the `plays`, `tracks` and `starred` this list adds.
pub type MyArtists = ArtistList;

/// `/api/me/favorite-ids` — the ids of everything starred, for heart state.
///
/// Every array holds the same key the routes use: a Deezer number as a string
/// when the row has one, else the row's UUID. They stay `String` because the
/// caller looks its own refs up unchanged (see [`crate::ids::TrackId`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FavoriteIds {
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub albums: Vec<String>,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub playlists: Vec<String>,
    /// Kept apart on purpose: a dislike is NOT a star, and the menus offer the
    /// opposite action.
    #[serde(default)]
    pub disliked: Vec<String>,
}
