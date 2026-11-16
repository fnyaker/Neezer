//! Search: the one screen that asks every kind at once, plus the podcast and
//! YouTube fallbacks.

use serde::{Deserialize, Serialize};

use crate::album::Album;
use crate::artist::Artist;
use crate::podcast::PodcastCard;
use crate::track::Track;

/// `/api/search?q=` — the five families in one answer.
///
/// With Deezer unreachable (or switched off) the server still answers: `tracks`
/// and the two entity arrays are the LIBRARY's own matches, and `playlists` /
/// `podcasts` come back empty. The desktop must therefore never treat an empty
/// family as an error.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchResults {
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub albums: Vec<Album>,
    #[serde(default)]
    pub artists: Vec<Artist>,
    #[serde(default)]
    pub playlists: Vec<SearchPlaylist>,
    #[serde(default)]
    pub podcasts: Vec<PodcastCard>,
}

impl SearchResults {
    /// Whether the answer holds nothing at all (what the screen's empty state
    /// checks, instead of testing five vectors itself).
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
            && self.albums.is_empty()
            && self.artists.is_empty()
            && self.playlists.is_empty()
            && self.podcasts.is_empty()
    }
}

/// A playlist a catalogue search found: provider fields only, no row id, no
/// members — it is opened by its `deezer_id` or not at all.
///
/// Taken from `neezer/webui/__init__.py#_playlist_api`; the fixture bench has
/// Deezer switched off, so it never answers one (a real server does).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchPlaylist {
    #[serde(default)]
    pub deezer_id: String,
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
    /// Always false on this list: a catalogue playlist cannot be starred.
    #[serde(default)]
    pub is_favorite: bool,
}

/// `/api/search/podcasts?q=` — shows found in the provider's catalogue.
///
/// [`PodcastCard`] comes from `#_podcast_card`; empty on the bench (Deezer off).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PodcastSearch {
    #[serde(default)]
    pub podcasts: Vec<PodcastCard>,
}

/// `/api/youtube/search?q=` — the fallback's hits, shaped as ordinary tracks
/// (`_catalog_track`'s shape: no row id, no archive fields, `"source":
/// "youtube"`). A refusal is a `503` with `X-NS-Error`, never an empty list, so
/// an empty `tracks` really means "nothing found".
pub type YoutubeSearch = crate::track::TrackList;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_search_answer_is_not_an_error_and_says_so() {
        let results: SearchResults = serde_json::from_str(
            r#"{"albums":[],"artists":[],"playlists":[],"podcasts":[],"tracks":[]}"#,
        )
        .expect("empty search");
        assert!(results.is_empty());
    }
}
