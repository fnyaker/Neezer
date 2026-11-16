//! The playback side of the contract: the quality of a stream, whether a stream
//! can be seeked right now, and what the player reports when a track ends.

use serde::{Deserialize, Serialize};

use crate::ids::TrackId;

/// The quality asked of `GET /api/stream/<id>?q=`.
///
/// The server ALWAYS archives FLAC and transcodes to Opus on demand; an absent
/// `q` means FLAC. `Opus256` and `Opus192` are accepted by `?q=` too (an older
/// client may ask for them) but the desktop's own picker offers 320/128/64 —
/// never MP3, which the server never sends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Quality {
    /// The archived FLAC itself. Its wire form is the ABSENCE of `q`.
    #[default]
    Flac,
    Opus320,
    Opus128,
    Opus64,
    /// Accepted by the server, not offered by the desktop's picker.
    Opus256,
    /// Accepted by the server, not offered by the desktop's picker.
    Opus192,
}

impl Quality {
    /// The `q=` value, or `None` for FLAC (which is what "no `q`" means).
    pub fn wire(self) -> Option<&'static str> {
        match self {
            Quality::Flac => None,
            Quality::Opus320 => Some("OPUS_320"),
            Quality::Opus128 => Some("OPUS_128"),
            Quality::Opus64 => Some("OPUS_64"),
            Quality::Opus256 => Some("OPUS_256"),
            Quality::Opus192 => Some("OPUS_192"),
        }
    }

    /// Reads a `q=` value (case-insensitively, as the server does), `None` or an
    /// empty string meaning FLAC.
    pub fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.map(str::trim).filter(|s| !s.is_empty()) {
            None => Some(Quality::Flac),
            Some(text) => match text.to_ascii_uppercase().as_str() {
                "FLAC" => Some(Quality::Flac),
                "OPUS_320" => Some(Quality::Opus320),
                "OPUS_128" => Some(Quality::Opus128),
                "OPUS_64" => Some(Quality::Opus64),
                "OPUS_256" => Some(Quality::Opus256),
                "OPUS_192" => Some(Quality::Opus192),
                _ => None,
            },
        }
    }

    /// The French label Réglages → Lecture shows.
    pub fn label(self) -> &'static str {
        match self {
            Quality::Flac => "FLAC",
            Quality::Opus320 => "Opus 320",
            Quality::Opus128 => "Opus 128",
            Quality::Opus64 => "Opus 64",
            Quality::Opus256 => "Opus 256",
            Quality::Opus192 => "Opus 192",
        }
    }

    /// `true` for the lossless one, which the server serves as the archive.
    pub fn lossless(self) -> bool {
        matches!(self, Quality::Flac)
    }
}

/// `/api/stream/<id>/ready` — what a stream would answer RIGHT NOW.
///
/// `ready`: a finished FILE will be served (a length, `Accept-Ranges: bytes`), so
/// a byte range seeks anywhere. `ranges`: `"bytes"` when that is the next answer.
/// `bytes`: the archive's size on disk, when there is one. `cached`: the Opus
/// transcode for this quality is already in the cache. `offset`: `?start=` is
/// honoured (the source is a finished file, so one request can begin mid-track).
/// `streaming`: nothing is published yet and an archive is being written, so the
/// wait is real and it ends — the seek must be deferred to the buffered part.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StreamReady {
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub ranges: Option<String>,
    #[serde(default)]
    pub bytes: Option<u64>,
    #[serde(default)]
    pub cached: bool,
    #[serde(default)]
    pub offset: bool,
    #[serde(default)]
    pub streaming: bool,
}

impl StreamReady {
    /// `true` when a seek can be asked of the server in one request.
    pub fn can_seek(&self) -> bool {
        self.ready || self.offset
    }
}

/// The context a queue was started from, as the SPA sends it with a listen.
///
/// It is a free-form little object (`{kind, id, title?, artist?}`); the server
/// reads `kind` and `id` only, to decide whether playing this track should
/// archive the whole container (`on_play_context`). Keeping the other keys lets
/// the player restore a label from its own snapshot.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListenContext {
    /// `"album"`, `"playlist"`, `"artist"`, `"flow"`, `"radio"`, `"track"`, `"favorites"`, …
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
}

/// The body of `POST /api/listen`, sent on every track change with the seconds
/// actually PLAYED (pauses excluded; `Player.svelte#flushListen`).
///
/// The server answers `204` and never blocks the player. Offline, the desktop
/// skips it (a doomed POST per track change is noise).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListenPayload {
    /// The track's universal id (`deezer_id`, or the row UUID for an upload).
    pub id: TrackId,
    #[serde(default)]
    pub listened: u32,
    #[serde(default)]
    pub next_id: Option<TrackId>,
    #[serde(default)]
    pub context: Option<ListenContext>,
    #[serde(default)]
    pub shuffle: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flac_is_the_absence_of_q_and_every_other_quality_has_its_wire_form() {
        assert_eq!(Quality::Flac.wire(), None);
        assert_eq!(Quality::Opus320.wire(), Some("OPUS_320"));
        assert_eq!(Quality::parse(None), Some(Quality::Flac));
        assert_eq!(Quality::parse(Some("")), Some(Quality::Flac));
        assert_eq!(Quality::parse(Some("opus_64")), Some(Quality::Opus64));
        assert_eq!(
            Quality::parse(Some("MP3_320")),
            None,
            "the server never sends MP3"
        );
    }

    #[test]
    fn a_generated_stream_cannot_be_seeked_by_byte_range_yet() {
        let ready: StreamReady = serde_json::from_str(
            r#"{"ready":false,"ranges":null,"bytes":null,"cached":false,
                "offset":false,"streaming":true}"#,
        )
        .expect("generated stream");
        assert!(!ready.can_seek());
        assert!(ready.streaming);
    }

    #[test]
    fn a_listen_payload_matches_the_body_the_web_player_sends() {
        let payload = ListenPayload {
            id: TrackId::new("3000000"),
            listened: 187,
            next_id: Some(TrackId::new("3000001")),
            context: Some(ListenContext {
                kind: "album".to_owned(),
                id: Some("2000000".to_owned()),
                ..Default::default()
            }),
            shuffle: false,
        };
        let json = serde_json::to_value(&payload).expect("listen json");
        assert_eq!(json["id"], "3000000");
        assert_eq!(json["context"]["kind"], "album");
        // A skipped track has no next id and no context, and both are optional.
        let bare: ListenPayload = serde_json::from_str(
            r#"{"id":"3000001","listened":0,"next_id":null,"context":null,"shuffle":true}"#,
        )
        .expect("bare listen");
        assert!(bare.next_id.is_none());
        assert!(bare.context.is_none());
    }
}
