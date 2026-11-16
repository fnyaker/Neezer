//! Artists: the page, the followed list, and the card the discovery rows use.

use serde::{Deserialize, Serialize};

use crate::album::Album;
use crate::ids::ArtistId;
use crate::track::Track;

/// An artist, in every shape the API serves one.
///
/// One struct rather than five: the page header, the followed-artists list, the
/// album column and the discovery cards all name an artist with the same keys,
/// each filling a different subset. `id` is the row UUID (absent from a
/// catalogue card), `deezer_id` the provider's own id — `""` when there is no
/// page to open.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Artist {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub deezer_id: ArtistId,
    #[serde(default)]
    pub name: String,
    /// Relative server path (`/api/cover/<id>`), or `null`.
    #[serde(default)]
    pub picture: Option<String>,
    /// Deezer's fan count, when the provider sent one.
    #[serde(default)]
    pub nb_fan: Option<u32>,
    /// `/api/artist/<id>` only.
    #[serde(default)]
    pub is_favorite: Option<bool>,
    /// `/api/me/artists` only: plays and library size.
    #[serde(default)]
    pub plays: Option<u32>,
    #[serde(default)]
    pub tracks: Option<u32>,
    /// `/api/me/artists` only: when it was followed (ISO date-time).
    #[serde(default)]
    pub starred: Option<String>,
    /// `/api/recommend`'s cards only: the one-line reason it is here.
    #[serde(default)]
    pub subtitle: Option<String>,
    /// `/api/recommend`'s cards only: `"deezer"` or `"local"`.
    #[serde(default)]
    pub provider: Option<String>,
}

/// `/api/artist/<id>` — the page: header, top titles, albums, neighbours.
///
/// `credited` is how many library titles the artist is credited on (not only as
/// performer); `library` says whether the library holds any.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtistPage {
    #[serde(default)]
    pub artist: Artist,
    /// The artist's best titles in the library (at most 15).
    #[serde(default)]
    pub top: Vec<Track>,
    #[serde(default)]
    pub albums: Vec<Album>,
    /// Artists the provider says are close; empty offline or when unknown.
    #[serde(default)]
    pub related: Vec<Artist>,
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default)]
    pub library: bool,
    #[serde(default)]
    pub credited: u32,
}

/// `{"artists": [...]}` — `/api/me/artists`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtistList {
    #[serde(default)]
    pub artists: Vec<Artist>,
}

/// The id an artist page is opened with: the provider's id when there is one,
/// else the row's UUID (an artist the library only knows from an upload).
impl Artist {
    pub fn request_id(&self) -> ArtistId {
        if self.deezer_id.as_str().is_empty() {
            ArtistId::new(self.id.clone().unwrap_or_default())
        } else {
            self.deezer_id.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_followed_artist_and_a_catalogue_card_are_the_same_type() {
        let followed: Artist = serde_json::from_str(
            r#"{"deezer_id":"1000001","id":"13a64185-e624-5144-8c2e-215936aa6637",
                "name":"Blue 1","picture":null,"plays":0,
                "starred":"2026-11-16T14:17:29","tracks":17}"#,
        )
        .expect("followed artist");
        assert_eq!(followed.tracks, Some(17));

        let card: Artist = serde_json::from_str(
            r#"{"deezer_id":"","id":"6f1c4b1e","name":"Camille","picture":null,
                "subtitle":"dans votre bibliothèque","provider":"local"}"#,
        )
        .expect("discovery card");
        assert_eq!(card.request_id(), ArtistId::new("6f1c4b1e"));
    }
}
