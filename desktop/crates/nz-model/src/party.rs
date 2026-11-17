//! Listen party: the host's own view of the party it is running.
//!
//! The guest side is public (`/api/party/<id>/…`) and has no session; the host
//! side is this one read route plus the publish call. The audio scheduling lives
//! in `nz-audio` and `appcore::sync` (D15b) — here is only the payload.

use serde::{Deserialize, Serialize};

/// One guest of a party, as the host sees it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartyListener {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// The guest's current sync estimate, in MILLISECONDS (`entry["q"]`, a
    /// rounded float), when the guest reported one.
    #[serde(default)]
    pub q: Option<f64>,
    /// The guest's own state (`"playing"`, `"buffering"`, …).
    #[serde(default)]
    pub st: Option<String>,
}

/// The party this person is hosting.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Party {
    #[serde(default)]
    pub id: String,
    /// The host's own state version: a controller that sees it change re-reads.
    #[serde(default)]
    pub v: u64,
    /// The public page path the host shares (`/party/<id>`).
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub listeners: Vec<PartyListener>,
    /// The server's clock, in milliseconds (`server_ms`, a FLOAT), at the moment
    /// of the answer — what the guests anchor their playback on.
    #[serde(default)]
    pub now: f64,
}

/// `/api/party/mine` — `{"party": … | null}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartyMine {
    #[serde(default)]
    pub party: Option<Party>,
}

/// One track of a party, as the server publishes it (`_public_track`): the
/// host's own fields, plus the party's cover route, which is how a guest with no
/// session sees the sleeve.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartyTrack {
    /// The media id the stream route takes (a Deezer number or a UUID).
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    /// Seconds.
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub podcast: bool,
    /// The server's ReplayGain loudness for this track, `null` when unmeasured.
    #[serde(default)]
    pub gain: Option<f64>,
    /// `/api/party/<pid>/cover/<id>` — relative, like every other cover.
    #[serde(default)]
    pub cover: Option<String>,
}

/// The anchor a party plays against: at server instant `t` (milliseconds) the
/// track was `p` seconds in. A paused party keeps its position and `playing`
/// says not to advance it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartyAnchor {
    /// The server instant, in milliseconds. A FLOAT on the wire
    /// (`webui/party.py#server_ms`): `1794878678797.0752` is not a `u64` this
    /// type may round — a guest schedules audio against it.
    #[serde(default)]
    pub t: f64,
    #[serde(default)]
    pub p: f64,
}

/// The track after the current one, and where the handover lands (`at` is the
/// media time in the CURRENT track; `start`, `fade` and `gap` are the next
/// one's trimmed lead-in, the crossfade, and how late the host usually is).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartyNext {
    #[serde(default)]
    pub track: Option<PartyTrack>,
    #[serde(default)]
    pub at: f64,
    #[serde(default)]
    pub start: f64,
    #[serde(default)]
    pub fade: f64,
    #[serde(default)]
    pub gap: f64,
}

/// The chunked-audio parameters a guest streams with.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartyChunk {
    /// Seconds per chunk.
    #[serde(default)]
    pub len: f64,
    /// Seconds of overlap between two chunks.
    #[serde(default)]
    pub ov: f64,
}

/// One guest, as the PUBLIC state names it: a name and nothing else (a listener
/// id authorises its own `leave`, so a guest must never learn another's).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartyGuestName {
    #[serde(default)]
    pub name: String,
}

/// `GET /api/party/<id>/state` — the timeline a guest polls.
///
/// The whole shape (`neezer/webui/party.py#_public_state`): the track, whether
/// it plays or buffers, the ANCHOR (`{"t", "p"}`, never flat `t`/`p`), the
/// crossfade length, the next handover, the host's normalisation level, the
/// chunk parameters, the guests' names, and the server's instant `now`. This
/// type used to be `{id, p, t, gap}` — the host's publish body, not this one —
/// which is how D15b ended up reading the guest state as raw JSON.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartyState {
    #[serde(default)]
    pub id: String,
    /// The host's display name.
    #[serde(default)]
    pub host: String,
    /// The host's state version: a guest that sees it change re-reads.
    #[serde(default)]
    pub v: u64,
    /// Whether the host is still there (its heartbeat has not timed out).
    #[serde(default)]
    pub live: bool,
    #[serde(default)]
    pub track: Option<PartyTrack>,
    #[serde(default)]
    pub playing: bool,
    /// Loading or rebuffering: the host is audible but not at its position yet.
    #[serde(default)]
    pub buf: bool,
    #[serde(default)]
    pub anchor: Option<PartyAnchor>,
    #[serde(default)]
    pub xfade: f64,
    #[serde(default)]
    pub next: Option<PartyNext>,
    /// `"off"`, `"low"`, `"medium"`, `"high"` — the host's own level.
    #[serde(default)]
    pub norm: String,
    #[serde(default)]
    pub chunk: PartyChunk,
    #[serde(default)]
    pub listeners: Vec<PartyGuestName>,
    /// The server's clock, in milliseconds, at the moment of the answer — a
    /// float, like `anchor.t` (`server_ms`).
    #[serde(default)]
    pub now: f64,
    /// Whether the server still knows the listener that asked (`?l=`), which is
    /// what makes a dropped guest take a new seat.
    #[serde(default)]
    pub me: Option<bool>,
}

/// `POST /api/party/<id>/state` — the HOST's own timeline, field for field what
/// `webapp/src/lib/party/host.js#flush` sends (and the SPA's page-hide call:
/// `{track, playing: false, t, p}`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartyPublish {
    #[serde(default)]
    pub track: Option<PartyTrack>,
    #[serde(default)]
    pub playing: bool,
    /// Present on some publishes only; the server reads a missing one as false.
    #[serde(default)]
    pub buf: bool,
    /// The server instant the position refers to, in milliseconds (float, like
    /// `PartyState::now`).
    #[serde(default)]
    pub t: f64,
    /// The position inside the track, in seconds.
    #[serde(default)]
    pub p: f64,
    #[serde(default)]
    pub xfade: f64,
    #[serde(default)]
    pub next: Option<PartyNext>,
    /// `"off"` when absent: the server refuses anything outside its levels.
    #[serde(default)]
    pub norm: String,
    /// `true` on a heartbeat, which carries no timeline at all.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hb: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_with_no_party_reads_as_null_not_as_an_error() {
        let mine: PartyMine = serde_json::from_str(r#"{"party":null}"#).expect("no party");
        assert!(mine.party.is_none());
    }

    /// The guest state the route really answers (`_public_state`): the anchor is
    /// an OBJECT under `anchor`, not flat `t`/`p`, and the answer carries the
    /// chunk parameters, the guests' names and the server's instant. The type
    /// used to declare the host's publish body, which is why nobody could read
    /// this with it.
    #[test]
    fn the_guest_state_is_the_servers_public_timeline() {
        let state: PartyState = serde_json::from_str(
            r#"{"id":"p1","host":"Camille","v":7,"live":true,
                "track":{"id":"3000001","title":"Balancing the Dark","artist":"Kaya Project",
                         "album":"Songs from the Inside","duration":312.0,"podcast":false,
                         "gain":-8.5,"cover":"/api/party/p1/cover/3000001"},
                "playing":true,"buf":false,"anchor":{"t":1794835151000,"p":42.25},
                "xfade":2.0,
                "next":{"track":{"id":"3000002","title":"Suivant","artist":"A","album":"B",
                                 "duration":200.0,"podcast":false,"gain":null},
                        "at":310.0,"start":0.4,"fade":2.0,"gap":0.18},
                "norm":"medium","chunk":{"len":6.0,"ov":0.5},
                "listeners":[{"name":"Salon"},{"name":"Cuisine"}],
                "now":1794835151012,"me":true}"#,
        )
        .expect("the guest state");
        let anchor = state.anchor.expect("an anchor");
        assert_eq!(anchor.p, 42.25);
        assert!(state.playing && state.live);
        let next = state.next.expect("a handover");
        assert_eq!(
            next.track.as_ref().map(|track| track.id.as_str()),
            Some("3000002")
        );
        assert_eq!(next.gap, 0.18);
        assert_eq!(state.listeners.len(), 2);
        assert_eq!(state.chunk.len, 6.0);
        assert_eq!(state.me, Some(true));
        assert!(
            (state.now - 1_794_835_151_012.0).abs() < 1.0,
            "the server's instant is a float on the wire, got {}",
            state.now
        );
    }

    /// The publish body is the SPA's own (`host.js#flush`), and it is not the
    /// guest state: no `id`/`host`/`v`, a flat `t`/`p`, and the track without a
    /// cover.
    #[test]
    fn the_publish_body_is_the_hosts_own() {
        let published = PartyPublish {
            track: Some(PartyTrack {
                id: "3000001".to_owned(),
                title: "Balancing the Dark".to_owned(),
                artist: "Kaya Project".to_owned(),
                album: "Songs from the Inside".to_owned(),
                duration: 312.0,
                podcast: false,
                gain: Some(-8.5),
                cover: None,
            }),
            playing: true,
            buf: false,
            t: 1_794_835_151_000.5,
            p: 42.25,
            xfade: 2.0,
            next: None,
            norm: "medium".to_owned(),
            hb: false,
        };
        let body = serde_json::to_value(&published).expect("json");
        assert_eq!(body["track"]["id"], "3000001");
        assert_eq!(body["p"], 42.25);
        assert!(body.get("hb").is_none(), "a timeline is not a heartbeat");
        assert!(
            body.get("anchor").is_none(),
            "the body is flat, the guest state is not"
        );
        assert!(
            body["track"].get("cover").is_none() || body["track"]["cover"].is_null(),
            "the host names no cover: the party route builds it"
        );
    }

    #[test]
    fn a_party_carries_its_anchor_and_its_guests() {
        let mine: PartyMine = serde_json::from_str(
            r#"{"party":{"id":"p1","v":3,"path":"/party/p1",
                "listeners":[{"id":"l1","name":"Salon","q":12.4,"st":"sync"}],
                "now":1794835151000}}"#,
        )
        .expect("party");
        let party = mine.party.expect("a party");
        assert_eq!(party.listeners[0].st.as_deref(), Some("sync"));
        assert_eq!(party.listeners[0].q, Some(12.4));
        assert_eq!(party.now, 1_794_835_151_000.0);
    }
}
