//! A track, its credits, its gain, its lyrics and the measurements the player
//! reads around it.
//!
//! The shape is the server's `neezer/webui/__init__.py#_db_track` (Deezer rows),
//! `#_local_track` (uploads) and `#_db_track_rows` (the big lists, same output).
//! A `Track` is what `/api/me/favorites`, `/api/playlist/<id>`, `/api/album/<id>`,
//! `/api/artist/<id>/tracks`, `/api/search`, `/api/radio/*`, `/api/flow` and the
//! queue all return, so it is the one type a desktop touches most.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::album::ReleaseRef;
use crate::ids::{AlbumId, ArtistId, TrackId, TrackRowId};

/// One track, as every list in the API carries it.
///
/// `deezer_id` is the IDENTITY (see [`TrackId`]); `id` is the row's UUID and is
/// missing from the preview rows a release page sends. `cover` is a path
/// relative to the server (`/api/cover/<id>`), never an absolute URL.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// The row's UUID. Absent from a release's preview rows.
    #[serde(default)]
    pub id: Option<TrackRowId>,
    /// The universal id: a Deezer number, or the row UUID for an upload.
    pub deezer_id: TrackId,
    pub title: String,
    /// Seconds. The server sends 0 for a row whose file has no duration.
    #[serde(default)]
    pub duration: u32,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub explicit: bool,
    /// The track is gone from its provider (or its row was flagged): the UI
    /// greys it and offers the replacement sheet.
    #[serde(default)]
    pub unavailable: bool,
    /// ReplayGain in dB, when the server has measured it.
    #[serde(default)]
    pub gain: Option<f64>,
    /// An uploaded/podcast/YouTube row: no provider to ask.
    #[serde(default)]
    pub local: bool,
    /// `"youtube"` for a fallback row, absent otherwise.
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub youtube_id: Option<String>,
    /// Present only once the file is on disk (`_quality_fields`), so a track
    /// that is only a catalogue row carries none of the three.
    #[serde(default)]
    pub bitrate: Option<u32>,
    #[serde(default)]
    pub suffix: Option<String>,
    #[serde(default)]
    pub quality: Option<String>,
    /// The track's main artist, as the album/list column shows it.
    #[serde(default)]
    pub artist: TrackArtist,
    /// Every credited performer, main artist first (`role` is `"Main"`).
    #[serde(default)]
    pub artists: Vec<TrackArtistRole>,
    /// The one line the list shows when there are several artists.
    #[serde(default)]
    pub display_artist: Option<String>,
    #[serde(default)]
    pub album: TrackAlbum,
    /// Set by `/api/credits/person` only: which of the asked person's roles this
    /// track carries.
    #[serde(default)]
    pub credit_roles: Vec<String>,
    /// Set by `/api/releases` only: the row arrived with the latest check.
    #[serde(default)]
    pub new: Option<bool>,
    /// Set by `/api/releases` only: the release the preview row belongs to.
    #[serde(default)]
    pub release: Option<ReleaseRef>,
    /// Set by `/api/flow` and `/api/recommend` only: why this title was picked
    /// (the Flow screen shows it as a short phrase).
    #[serde(default)]
    pub why: Option<String>,
    /// Set by `/api/flow` only: the measured energy this title brought, 0..1.
    #[serde(default)]
    pub energy: Option<f64>,
}

/// `{"deezer_id", "name"}`: an artist as a track's column carries it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackArtist {
    /// `""` when there is no artist page to open (a YouTube label).
    #[serde(default)]
    pub deezer_id: ArtistId,
    #[serde(default)]
    pub name: String,
}

/// `{"deezer_id", "name", "role"}`: one performer of a track.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackArtistRole {
    #[serde(default)]
    pub deezer_id: ArtistId,
    #[serde(default)]
    pub name: String,
    /// `"Main"`, `"Featured"`, … — the server's own labels.
    #[serde(default)]
    pub role: String,
}

/// `{"deezer_id", "title", "cover"}`: the album a track sits on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackAlbum {
    #[serde(default)]
    pub deezer_id: AlbumId,
    #[serde(default)]
    pub title: String,
    /// Relative path on the server (`/api/cover/<id>`), or `null`.
    #[serde(default)]
    pub cover: Option<String>,
}

impl Track {
    /// The id every `/api` route and the stream accept for this track.
    pub fn id_for_requests(&self) -> &TrackId {
        &self.deezer_id
    }

    /// Whether the row is backed by a file the server can serve.
    pub fn is_archived(&self) -> bool {
        self.suffix.is_some() || self.quality.is_some()
    }
}

/// `{"tracks": [...]}` — the envelope of every list of tracks without a page of
/// its own (`/api/me/favorites`, `/api/me/local`, `/api/me/dislikes`,
/// `/api/unavailable`, `/api/artist/<id>/tracks`, `/api/radio/*`, `/api/flow`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackList {
    #[serde(default)]
    pub tracks: Vec<Track>,
}

/// `/api/gain/<id>`: `{"gain": dB | null}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Gain {
    #[serde(default)]
    pub gain: Option<f64>,
}

/// `/api/gains` (POST): `{"gains": {<id as asked>: dB | null}}`.
///
/// Keyed by the id as it was ASKED (a Deezer number or a row UUID), which is why
/// the key stays a `String`: the caller looks its own ids up unchanged.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Gains {
    #[serde(default)]
    pub gains: BTreeMap<String, Option<f64>>,
}

/// `/api/lyrics/<id>`: `{"lyrics": <object> | null}`.
///
/// The object is the server's canonical shape, the one LRCLIB fills
/// (`neezer/deezer/lyrics.py`): `{"text", "synced": [{"time", "text"}],
/// "source"}`. The fixture this type was first proven against answered `null`
/// (the bench had nothing archived), so the string shape it declared was never
/// the server's — a plain STRING is still accepted, because an older server and
/// a hand-written fixture may answer one (rule 7).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Lyrics {
    #[serde(default)]
    pub lyrics: Option<LyricsBody>,
}

/// The words: the server's object, or the bare text an older answer carried.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LyricsBody {
    /// A bare string: the words, with no timing at all.
    Plain(String),
    /// The canonical document.
    Doc(LyricsDoc),
}

impl LyricsBody {
    /// The words as plain text, whether or not they carry timings.
    pub fn text(&self) -> &str {
        match self {
            LyricsBody::Plain(text) => text,
            LyricsBody::Doc(doc) => &doc.text,
        }
    }

    /// The timed lines, empty for a plain answer.
    pub fn synced(&self) -> &[LyricLine] {
        match self {
            LyricsBody::Plain(_) => &[],
            LyricsBody::Doc(doc) => &doc.synced,
        }
    }
}

/// One synced line: `{"time": <milliseconds>, "text": "…"}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LyricLine {
    #[serde(default)]
    pub time: i64,
    #[serde(default)]
    pub text: String,
}

/// The lyrics document itself: `text`, its timed lines, and where they came
/// from (`"archive"`, or the provider the sidecar was filled from).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LyricsDoc {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub synced: Vec<LyricLine>,
    /// Never read by a screen; it says where the words came from.
    #[serde(default)]
    pub source: Option<String>,
}

/// `/api/audio/edges/<id>?db=` — where the audio really starts and stops.
///
/// `ready: false` (with the row's duration, when known) means the file is not on
/// disk or was not measured: the player then plays the track untrimmed. `start`
/// and `end` are seconds, and `threshold` the dBFS the measurement used.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioEdges {
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub start: Option<f64>,
    #[serde(default)]
    pub end: Option<f64>,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub threshold: Option<f64>,
}

/// `/api/track/<id>/probe` — is this track dead, or was that a bad moment?
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackProbe {
    #[serde(default)]
    pub available: bool,
    /// The server's own reason when it is not (`"unavailable"`, `"unknown"`, …).
    #[serde(default)]
    pub reason: Option<String>,
}

/// `/api/track/<id>/credits` — the people behind one track, grouped by role.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackCredits {
    #[serde(default)]
    pub credits: Vec<CreditGroup>,
    /// Reserved by the server (an empty object today).
    #[serde(default)]
    pub info: BTreeMap<String, serde_json::Value>,
}

/// One role of a track's credits, with its people.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreditGroup {
    /// The machine role (`"main_artist"`, `"composer"`, `"author"`, …).
    #[serde(default)]
    pub role: String,
    /// The French label the server already chose.
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub people: Vec<CreditPerson>,
}

/// One person of one role. A library artist opens a page and carries both ids; a
/// name only the tags know has a name and how many tracks it is credited on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreditPerson {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub deezer_id: Option<ArtistId>,
    /// The artist row's UUID, when the library holds it.
    #[serde(default)]
    pub id: Option<String>,
    /// How many library tracks this person is credited on under this role.
    #[serde(default)]
    pub credited: Option<u32>,
}

/// `/api/credits/person?name=` — everything in the library one person worked on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PersonCredits {
    #[serde(default)]
    pub name: String,
    /// `{role: French label}` for the roles this page shows.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub roles: Vec<PersonRole>,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

/// One role a person appears in, with how many titles.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PersonRole {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub count: u32,
}

/// The public link of one track (`/api/track/<id>/share`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackShareLink {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub token: String,
    /// The relative page path (`/t/<token>`).
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub expires: Option<String>,
    #[serde(default)]
    pub last_used: Option<String>,
}

/// `/api/track/<id>/share` — `{"link": … | null}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackShare {
    #[serde(default)]
    pub link: Option<TrackShareLink>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACK: &str = r#"{
        "id": "b8a1e106-0732-5988-918c-4c4a4d1e4b1f",
        "deezer_id": "3000168",
        "title": "Été",
        "duration": 183,
        "release_date": null,
        "explicit": false,
        "unavailable": false,
        "gain": null,
        "artist": {"deezer_id": "1000003", "name": "Gold 3"},
        "artists": [{"deezer_id": "1000003", "name": "Gold 3", "role": "Main"}],
        "display_artist": "Gold 3",
        "album": {"deezer_id": "2000044", "title": "Fire mer 44", "cover": "/api/cover/cdb616f5"}
    }"#;

    #[test]
    fn a_db_track_parses_and_keeps_its_cover_relative() {
        let track: Track = serde_json::from_str(TRACK).expect("track");
        assert_eq!(track.deezer_id.as_str(), "3000168");
        assert_eq!(track.display_artist.as_deref(), Some("Gold 3"));
        assert_eq!(
            track.album.cover.as_deref(),
            Some("/api/cover/cdb616f5"),
            "the cover stays a server path: the base URL is the client's"
        );
        assert!(!track.is_archived(), "a catalogue row has no file yet");
    }

    #[test]
    fn a_release_preview_row_has_no_row_id_and_still_parses() {
        // `/api/releases` previews carry what Deezer published: no `id`, no
        // `unavailable`, no `release_date`.
        let json = r#"{
            "deezer_id": "9000000",
            "title": "Echo hier",
            "duration": 122,
            "explicit": false,
            "gain": null,
            "artist": {"deezer_id": "1000015", "name": "Blue 15"},
            "artists": [{"deezer_id": "1000015", "name": "Blue 15", "role": "Main"}],
            "display_artist": "Blue 15",
            "album": {"deezer_id": "8000000", "title": "Été", "cover": null}
        }"#;
        let track: Track = serde_json::from_str(json).expect("preview row");
        assert!(track.id.is_none());
        assert!(!track.unavailable);
    }
}
