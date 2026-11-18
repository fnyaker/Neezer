//! The wire of the now-playing area: the two reads the full-screen player opens
//! (`GET /api/lyrics/<id>`, `GET /api/radio/track/<id>`) and the conversions into
//! the records a screen draws.
//!
//! Both are `Lane::User` work: the words of the title that is playing, and the
//! list behind « À propos » — what the person is looking at NOW (rule 1).
//!
//! The bodies are read as TEXT and parsed here rather than through a typed
//! endpoint, for the entity area's reason: the raw text is also what the device
//! keeps, so a remembered answer and a fresh one go through exactly the same
//! converter and a cache hit cannot drift from a live answer.
//!
//! No rule lives here: which lines a card can offer is [`super::card`], and the
//! French copy is the screen's.

use serde::de::DeserializeOwned;

use nz_api::{Api, Lane};
use nz_model::{Track, TrackList};

use crate::core::CoreError;
use crate::lists::Row;

use super::{LyricLine, Lyrics};

/// `GET` one path as text, on the `User` lane. The request and its body bytes are
/// counted by `nz-api`'s own per-tag meter (SC3), so `NzCore::stats()` answers
/// what the screen cost.
async fn fetch_text(api: &Api, path: &str) -> Result<String, CoreError> {
    let bytes = api.bytes(Lane::User, path).await?;
    String::from_utf8(bytes).map_err(|error| CoreError::Internal {
        detail: format!("{path} : {error}"),
    })
}

/// A body, parsed. A refusal to parse is an internal fault with the path in it —
/// never a silent empty view.
fn parse<T: DeserializeOwned>(path: &str, body: &str) -> Result<T, CoreError> {
    serde_json::from_str(body).map_err(|error| CoreError::Internal {
        detail: format!("{path} : {error}"),
    })
}

/// A route argument, percent-encoded the way `encodeURIComponent` does it: every
/// id here is a UUID or a provider number today, and a hostile one is still one
/// path segment.
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

/// `GET /api/lyrics/<id>` — the body, kept by the caller.
pub(super) async fn fetch_lyrics(api: &Api, key: &str) -> Result<String, CoreError> {
    fetch_text(api, &format!("/lyrics/{}", seg(key))).await
}

/// `GET /api/radio/track/<id>` — a radio seeded from the playing title.
pub(super) async fn fetch_track_radio(api: &Api, key: &str) -> Result<String, CoreError> {
    fetch_text(api, &format!("/radio/track/{}", seg(key))).await
}

/// The server's own answer shape (`neezer/webui/__init__.py#lyrics`):
/// `{"lyrics": {"text": …, "synced": [{"time": ms, "text": …}], "source": …}}`,
/// or `{"lyrics": null}` for a title it has no words for.
#[derive(serde::Deserialize)]
struct LyricsBody {
    #[serde(default)]
    lyrics: Option<LyricsPayload>,
}

#[derive(serde::Deserialize)]
struct LyricsPayload {
    #[serde(default)]
    text: String,
    #[serde(default)]
    synced: Vec<SyncedLine>,
    #[serde(default)]
    source: String,
}

#[derive(serde::Deserialize)]
struct SyncedLine {
    /// Milliseconds, as `parse_lrc` writes them.
    #[serde(default)]
    time: i64,
    #[serde(default)]
    text: String,
}

/// `/api/lyrics/<id>`'s body, as the view and the card draw it.
pub(super) fn lyrics(body: &str) -> Result<Lyrics, CoreError> {
    let parsed: LyricsBody = parse("/lyrics", body)?;
    let Some(payload) = parsed.lyrics else {
        return Ok(Lyrics::default());
    };
    Ok(Lyrics {
        synced: payload
            .synced
            .into_iter()
            .map(|line| LyricLine {
                // A negative stamp is a corrupt file, not a line at minus time.
                time_ms: line.time.clamp(0, i64::from(u32::MAX)) as u32,
                text: line.text,
            })
            .collect(),
        plain: payload.text,
        source: payload.source,
    })
}

/// `/api/radio/track/<id>`'s body, as rows a list draws.
pub(super) fn track_rows(body: &str) -> Result<Vec<Row>, CoreError> {
    let list: TrackList = parse("/radio/track", body)?;
    Ok(list
        .tracks
        .iter()
        .enumerate()
        .map(|(position, track)| row(track, position as u32))
        .collect())
}

/// ONE track, as the shared row — the same shape every list draws and the same
/// key the queue uses (`trackkey.js#trackRef`): the row's UUID when it has one,
/// else the provider's id.
fn row(track: &Track, position: u32) -> Row {
    let deezer = track.deezer_id.to_string();
    let row_id = track
        .id
        .as_ref()
        .map(|id| id.to_string())
        .filter(|id| !id.is_empty());
    let album_id = track.album.deezer_id.to_string();
    let source = match track.source.as_deref() {
        Some("youtube") => "youtube",
        _ if track.local => "upload",
        _ => "deezer",
    };
    Row {
        key: row_id.clone().unwrap_or_else(|| deezer.clone()),
        id: row_id.unwrap_or(deezer),
        title: track.title.clone(),
        artists: artist_line(track),
        album: track.album.title.clone(),
        album_id: (!album_id.is_empty()).then_some(album_id),
        duration_s: track.duration,
        cover: track.album.cover.clone(),
        explicit: track.explicit,
        // A like is MB10's state and reaches the row through `update_rows`.
        liked: false,
        disliked: false,
        unavailable: track.unavailable,
        downloaded: false,
        source: source.to_owned(),
        position,
        played_at: 0,
        added_at: 0,
    }
}

/// `format.js#artistLine`: the server's own line when it sent one, else the
/// credited names joined — the same sentence every list shows.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hostile_key_is_one_path_segment() {
        assert_eq!(seg("12345"), "12345");
        assert_eq!(seg("a/b?c#d e"), "a%2Fb%3Fc%23d%20e");
        assert!(!seg("../x").contains('/'));
    }

    /// The rows a radio answers are the rows a list draws: the queue's key, the
    /// artist line, and the album a sub-line opens.
    #[test]
    fn a_radio_becomes_rows() {
        let body = r#"{"tracks":[{"deezer_id":"42","title":"Un titre","duration":231,
            "artist":{"deezer_id":"7","name":"Artiste"},"artists":[{"name":"Artiste"}],
            "album":{"deezer_id":"9","title":"Album","cover":"https://cdn/x.jpg"}}]}"#;
        let rows = track_rows(body).expect("a radio");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "42");
        assert_eq!(rows[0].artists, "Artiste");
        assert_eq!(rows[0].album_id.as_deref(), Some("9"));
        assert_eq!(rows[0].position, 0);
    }
}
