//! Listen party: the host's own view of the party it is running.
//!
//! The guest side is public (`/api/party/<id>/…`) and has no session; the host
//! side is this one read route plus the publish call. The audio scheduling lives
//! in `nz-audio` and `appcore::sync` (D15b) — here is only the payload.

use serde::{Deserialize, Serialize};

use crate::ids::TrackId;

/// One guest of a party, as the host sees it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartyListener {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// The quality the guest asked for (`"opus"`…), when it said.
    #[serde(default)]
    pub q: Option<String>,
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
    /// The server's clock, in milliseconds, at the moment of the answer — what
    /// the guests anchor their playback on.
    #[serde(default)]
    pub now: u64,
}

/// `/api/party/mine` — `{"party": … | null}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartyMine {
    #[serde(default)]
    pub party: Option<Party>,
}

/// The anchor a host publishes: the track, its position, and the server instant.
///
/// The desktop publishes this from the OUTPUT clock (cpal's timestamps), which is
/// more precise than a browser's `performance.now()`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartyState {
    pub id: TrackId,
    /// The position inside the track, in milliseconds.
    #[serde(default)]
    pub p: u64,
    /// The server instant the position refers to, in milliseconds.
    #[serde(default)]
    pub t: u64,
    /// The gap the host will leave after this track, in milliseconds.
    #[serde(default)]
    pub gap: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_with_no_party_reads_as_null_not_as_an_error() {
        let mine: PartyMine = serde_json::from_str(r#"{"party":null}"#).expect("no party");
        assert!(mine.party.is_none());
    }

    #[test]
    fn a_party_carries_its_anchor_and_its_guests() {
        let mine: PartyMine = serde_json::from_str(
            r#"{"party":{"id":"p1","v":3,"path":"/party/p1",
                "listeners":[{"id":"l1","name":"Salon","q":"opus","st":"playing"}],
                "now":1794835151000}}"#,
        )
        .expect("party");
        let party = mine.party.expect("a party");
        assert_eq!(party.listeners[0].st.as_deref(), Some("playing"));
        assert_eq!(party.now, 1_794_835_151_000);
    }
}
