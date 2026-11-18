//! Every frame that crosses a `nz-lan` connection, **final**: a later lot fills
//! a body, never a frame.
//!
//! # Framing
//!
//! Every request opens a new bidirectional stream: one request frame, one
//! response frame, and for `Fetch`/`Cover` the raw bytes after it until FIN. A
//! frame is `u32` big-endian length + UTF-8 JSON (`serde_json`,
//! `#[serde(tag = "t")]`), **≤ 1 MiB** ([`crate::codec::MAX_FRAME_BYTES`]) —
//! `Inventory` and `Sets` paginate, they are never one huge frame.
//!
//! # Rule 7 on the wire
//!
//! Unknown fields are ignored (`serde`'s default) and an optional field that is
//! missing falls back to its default, so a newer peer talking to an older one
//! does not fail a parse; an unknown `t` answers `Error { code: "unknown" }`
//! rather than closing the connection. A different protocol MAJOR is refused at
//! the handshake (`0x11`), because a peer that speaks another language would
//! only produce errors a person cannot act on.
//!
//! # The control stream
//!
//! The authentication stream stays open as the connection's control stream:
//! [`Control`] carries `Auth`/`AuthOk` first, then `Hello`, the gossip of
//! revocation lists and rosters, the two "my inventory/sets changed" notices and
//! `Bye`. Request frames never carry any of that: one request, one stream, one
//! answer.

use serde::{Deserialize, Serialize};

/// What a caller asks a peer for. One request per stream; the responder answers
/// exactly one [`Response`].
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "t")]
pub enum Request {
    /// Everything the responder has COMPLETE on disk, per quality, filtered by
    /// the requester's scope. `since` is the requester's last version: a delta
    /// when the responder still remembers it (it keeps the last 64 changes),
    /// otherwise the full inventory. Page size: 4 000 items.
    Inventory {
        #[serde(default)]
        since: Option<u64>,
        #[serde(default)]
        page: u32,
    },
    /// The responder's download sets (playlists, albums, shows) whose members
    /// are all in scope, with their track ids and whether every member is on
    /// disk. 64 sets per page.
    Sets {
        #[serde(default)]
        page: u32,
    },
    /// The screen-shaped metadata of at most 200 tracks, for titles and artists
    /// the requester does not have locally (an offline playlist still looks
    /// right).
    Meta {
        #[serde(default)]
        tracks: Vec<String>,
    },
    /// The cover art a metadata row named, ≤ 2 MiB.
    Cover {
        #[serde(default)]
        url: String,
    },
    /// The file itself. `from` > 0 resumes; `expect` is the `FileTag` the
    /// requester already holds, so a resume onto a file that changed in between
    /// is refused instead of producing a wrong file.
    Fetch {
        #[serde(default)]
        track: String,
        #[serde(default)]
        quality: WireQuality,
        #[serde(default)]
        from: u64,
        #[serde(default)]
        expect: Option<FileTag>,
        #[serde(default)]
        urgency: Urgency,
    },
    /// Liveness and round-trip time, with no content at all.
    Ping,
}

/// The answer to one [`Request`]. `NotFound` is also the answer for anything
/// outside the requester's scope: a friend must not learn that a track exists
/// from the shape of the refusal.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "t")]
pub enum Response {
    /// A page of the full inventory: `last` says whether another page follows.
    InventoryFull {
        #[serde(default)]
        version: u64,
        #[serde(default)]
        page: u32,
        #[serde(default)]
        last: bool,
        #[serde(default)]
        items: Vec<Item>,
    },
    /// What changed between two versions of the responder's inventory.
    InventoryDelta {
        #[serde(default)]
        from: u64,
        #[serde(default)]
        to: u64,
        #[serde(default)]
        added: Vec<Item>,
        #[serde(default)]
        removed: Vec<ItemKey>,
    },
    /// A page of the responder's sets.
    Sets {
        #[serde(default)]
        version: u64,
        #[serde(default)]
        page: u32,
        #[serde(default)]
        last: bool,
        #[serde(default)]
        sets: Vec<SetInfo>,
    },
    /// Metadata rows, in the order asked where the responder has them.
    Meta {
        #[serde(default)]
        rows: Vec<TrackMeta>,
    },
    /// A cover follows: `len` raw bytes on this stream.
    Cover {
        #[serde(default)]
        mime: String,
        #[serde(default)]
        len: u32,
    },
    /// The file follows: raw bytes from `from` until FIN (or until `len` is
    /// reached). `tag` is present once the responder has hashed its copy.
    FetchOk {
        #[serde(default)]
        len: u64,
        #[serde(default)]
        tag: Option<FileTag>,
        #[serde(default)]
        quality: WireQuality,
    },
    /// Absent, or out of scope: the identical frame for both.
    NotFound,
    /// Serve limits reached (rule 1 on the responder's own resources).
    Busy {
        #[serde(default)]
        retry_in_ms: u32,
    },
    Pong,
    /// A refusal this build can name (`unknown` for a frame tag it does not
    /// know, `proto` for a version it cannot read).
    Error {
        #[serde(default)]
        code: String,
    },
}

/// What two peers tell each other once they are authenticated: names, at last,
/// and the two versions that decide whether the other side's replica is stale.
///
/// [`Control::Hello`] is its WIRE form (the same five fields, inline in the tagged
/// frame, which is what the note pins); this struct is the in-memory one the rest
/// of the crate holds, and `transport` converts once on receipt so no other module
/// matches on a control variant to read a name.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct Hello {
    /// The device's display name, from the person (1–64 characters).
    #[serde(default)]
    pub name: String,
    /// `"desktop"`, `"android"` or `"ios"`.
    #[serde(default)]
    pub platform: String,
    /// The app's version, so a peer with a newer protocol can say « mettez à jour
    /// l'app sur <appareil> ».
    #[serde(default)]
    pub app_version: String,
    /// The sender's inventory version: a receiver whose replica is behind asks for
    /// a delta.
    #[serde(default)]
    pub inventory_version: u64,
    /// The sender's download-sets version.
    #[serde(default)]
    pub sets_version: u64,
}

/// One track the responder has, with every quality it can serve it at.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Item {
    #[serde(default)]
    pub track: String,
    #[serde(default)]
    pub copies: Vec<Copy>,
}

/// One complete local file: the quality and its length.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Copy {
    #[serde(default)]
    pub quality: WireQuality,
    #[serde(default)]
    pub len: u64,
}

/// Which copy of which track left the inventory (a delta's `removed`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ItemKey {
    #[serde(default)]
    pub track: String,
    #[serde(default)]
    pub quality: WireQuality,
}

/// A file's identity, announced before its bytes and carried back by a resume:
/// the length and the SHA-256 (hex) of the COMPLETE file.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FileTag {
    #[serde(default)]
    pub len: u64,
    /// Lowercase hex, 64 characters. The requester compares, it never parses
    /// this into a path or a name.
    #[serde(default)]
    pub sha256: String,
}

/// A download set a peer has: a playlist, an album or a show.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SetInfo {
    /// `"playlist"`, `"album"` or `"show"`.
    #[serde(default)]
    pub kind: String,
    /// The server's id for it (a playlist id, an album id, a show id).
    #[serde(default)]
    pub ident: String,
    #[serde(default)]
    pub title: String,
    /// The cover URL the set's page draws, when it has one.
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub tracks: Vec<String>,
    /// How many of `tracks` this peer has complete on disk.
    #[serde(default)]
    pub local: u32,
    /// `local == tracks.len()`: the whole set can come from this peer.
    #[serde(default)]
    pub complete: bool,
}

/// One track's screen-shaped metadata. Deliberately smaller than the server's
/// own row: no likes, no play counts, no user ids — a peer sends what a list
/// draws and nothing about the person.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TrackMeta {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artists: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub album_id: Option<String>,
    #[serde(default)]
    pub duration_s: u32,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub explicit: bool,
}

/// A quality as it travels: the four rungs this crate can name, with the server's
/// own `q=` spelling in the JSON.
///
/// It is narrower than `nz_api::Quality` on purpose: `Opus256`/`Opus192` are
/// accepted by the server but are never what a peer serves (its files come from
/// the archive and the app's own downloads), and a frame that could carry them
/// would invite a decision nobody specified.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WireQuality {
    /// The archived FLAC.
    #[default]
    #[serde(rename = "FLAC")]
    Flac,
    #[serde(rename = "OPUS_320")]
    Opus320,
    #[serde(rename = "OPUS_128")]
    Opus128,
    #[serde(rename = "OPUS_64")]
    Opus64,
}

impl From<nz_api::Quality> for WireQuality {
    fn from(quality: nz_api::Quality) -> Self {
        match quality {
            nz_api::Quality::Flac => WireQuality::Flac,
            nz_api::Quality::Opus64 => WireQuality::Opus64,
            nz_api::Quality::Opus128 => WireQuality::Opus128,
            // The two rungs a peer never serves: the nearest one below is what a
            // request may honestly ask for, and the answer says what was served.
            nz_api::Quality::Opus320 | nz_api::Quality::Opus256 | nz_api::Quality::Opus192 => {
                WireQuality::Opus320
            }
        }
    }
}

impl From<WireQuality> for nz_api::Quality {
    fn from(quality: WireQuality) -> Self {
        match quality {
            WireQuality::Flac => nz_api::Quality::Flac,
            WireQuality::Opus320 => nz_api::Quality::Opus320,
            WireQuality::Opus128 => nz_api::Quality::Opus128,
            WireQuality::Opus64 => nz_api::Quality::Opus64,
        }
    }
}

/// How urgent a transfer is, carried on the wire so the RESPONDER can order what
/// it serves (rule 1: the tapped track first, a download last).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum Urgency {
    /// The track the person just tapped: it never waits, and it is what the
    /// responder serves first.
    Play,
    /// The next track (`Lane::Prefetch`).
    #[default]
    Prefetch,
    /// Downloads, offline sync, covers: background work that yields.
    Bulk,
}

/// The connection's control stream, which is the authentication stream kept
/// open.
///
/// The two blobs of the gossip are the SIGNED bytes as they were received
/// (base64), never a re-serialisation: a signature is over the exact bytes, and
/// re-encoding is how a valid roster starts failing.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "t")]
pub enum Control {
    /// The initiator proves itself first. Nothing — not even the responder's own
    /// credential — is sent before this verifies.
    Auth {
        /// The 154-byte credential, base64.
        cred: String,
        /// `Ed25519(device key, "neezer-lan/1 auth\0" ‖ role ‖ exporter)`,
        /// base64.
        proof: String,
        revocation_serial: u64,
        /// The protocol major. A different one is closed with `0x11`.
        proto: u32,
    },
    /// The responder's half of the same proof, once it accepted the initiator.
    AuthOk {
        cred: String,
        proof: String,
        revocation_serial: u64,
        proto: u32,
    },
    /// Names, at last: they only ever travel inside an authenticated connection.
    Hello {
        name: String,
        platform: String,
        app_version: String,
        inventory_version: u64,
        sets_version: u64,
    },
    /// A newer revocation list (base64 of the signed blob).
    Revocations { blob: String },
    /// A newer roster (base64 of the signed blob), only between devices of the
    /// same subject.
    Roster { blob: String },
    /// The sender's inventory moved: the receiver's replica is stale.
    InventoryChanged { version: u64 },
    /// The sender's sets moved.
    SetsChanged { version: u64 },
    /// Closing politely, with no cause on the wire.
    Bye,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frames are tagged with `t`, and an older peer's missing optional field
    /// does not fail the parse (rule 7 on the wire).
    #[test]
    fn a_request_carries_its_tag_and_tolerates_missing_optionals() {
        let json = serde_json::to_value(Request::Ping).expect("ping");
        assert_eq!(json["t"], "Ping");

        let parsed: Request = serde_json::from_str(r#"{"t":"Inventory"}"#).expect("inventory");
        assert_eq!(
            parsed,
            Request::Inventory {
                since: None,
                page: 0
            }
        );
        // A field this build does not know is ignored rather than refused.
        let parsed: Request = serde_json::from_str(r#"{"t":"Ping","future":42}"#).expect("ping");
        assert_eq!(parsed, Request::Ping);
    }

    /// The qualities travel with the server's own spelling, because the same
    /// ladder names the files on disk on both sides.
    #[test]
    fn a_quality_travels_with_its_wire_spelling() {
        assert_eq!(
            serde_json::to_value(WireQuality::Opus320).expect("json"),
            serde_json::json!("OPUS_320")
        );
        assert_eq!(
            serde_json::to_value(WireQuality::Flac).expect("json"),
            serde_json::json!("FLAC")
        );
        // The two rungs a peer never serves fold onto the nearest one below.
        assert_eq!(
            WireQuality::from(nz_api::Quality::Opus256),
            WireQuality::Opus320
        );
        assert_eq!(
            WireQuality::from(nz_api::Quality::Opus192),
            WireQuality::Opus320
        );
        // And the four that exist round trip.
        for quality in [
            WireQuality::Flac,
            WireQuality::Opus320,
            WireQuality::Opus128,
            WireQuality::Opus64,
        ] {
            assert_eq!(WireQuality::from(nz_api::Quality::from(quality)), quality);
        }
    }

    /// The answer to a track a friend may not see is the answer to a track nobody
    /// has: the same variant, so the same bytes, built by the same function.
    #[test]
    fn absent_and_out_of_scope_are_one_frame() {
        assert_eq!(
            serde_json::to_string(&Response::NotFound).expect("json"),
            r#"{"t":"NotFound"}"#
        );
    }

    /// The control stream starts with the two authentication frames and never
    /// carries content: a name is the most that travels before `Hello`.
    #[test]
    fn control_frames_are_tagged_and_carry_no_content() {
        let auth = Control::Auth {
            cred: "AA".into(),
            proof: "BB".into(),
            revocation_serial: 7,
            proto: 1,
        };
        let json = serde_json::to_value(&auth).expect("auth");
        assert_eq!(json["t"], "Auth");
        assert_eq!(json["proto"], 1);

        let cover = Request::Cover {
            url: "/api/cover/1".into(),
        };
        let json = serde_json::to_value(&cover).expect("cover");
        assert_eq!(json["url"], "/api/cover/1");
        // Cover answers with a length, then raw bytes: the frame never inlines
        // an image.
        let json = serde_json::to_value(Response::Cover {
            mime: "image/jpeg".into(),
            len: 12,
        })
        .expect("cover answer");
        assert_eq!(json["len"], 12);
    }

    /// A frame is one line of JSON at most a MiB: the cap lives in `codec.rs`,
    /// which enforces it before allocating.
    #[test]
    fn the_frame_cap_is_one_mebibyte() {
        assert_eq!(crate::codec::MAX_FRAME_BYTES, 1 << 20);
    }
}
