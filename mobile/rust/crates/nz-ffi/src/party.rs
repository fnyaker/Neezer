//! The listen party: host and guest, through `/party/<id>`. Owned by **MB20**
//! (the planner and the clock are SC6's `nz-party`).
//!
//! The note pins "host, join `/party/<id>`, leave, members" (docs/agents/mobile.md,
//! "Screen lots, wave 3"), and the guest's sync reads the ENGINE's output clock
//! (SC2's latency), never a wall clock. MB0 declared the names; MB20 fills them.

use crate::errors::NzError;

/// What this device is in the party.
#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum PartyRole {
    None,
    Host { id: String, guests: u32 },
    Guest { id: String, host_name: String },
}

/// One member of the party, as the sheet lists them.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct PartyMember {
    pub name: String,
    /// `host` or `guest`.
    pub role: String,
    /// The latency the guest reported, in milliseconds.
    pub latency_ms: u32,
}

/// A party's on-screen state.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct PartyState {
    pub role: PartyRole,
    /// The track everyone is on, as the host named it.
    pub key: Option<String>,
    /// The shared wall clock's offset, in milliseconds.
    pub clock_offset_ms: i64,
    /// `true` while this device waits for the host to start the next track.
    pub waiting: bool,
    pub members: Vec<PartyMember>,
}

/// The party. MB20 fills it; MB0 refuses every call.
#[derive(uniffi::Object)]
pub struct PartyApi;

#[uniffi::export(async_runtime = "tokio")]
impl PartyApi {
    pub fn state(&self) -> PartyState {
        PartyState {
            role: PartyRole::None,
            key: None,
            clock_offset_ms: 0,
            waiting: false,
            members: Vec::new(),
        }
    }

    /// Opens a party from the current queue and returns its id.
    pub async fn host(&self) -> Result<String, NzError> {
        Err(NzError::Unsupported {
            feature: "mb20:host".into(),
        })
    }

    pub async fn join(&self, id: String) -> Result<(), NzError> {
        let _ = id;
        Err(NzError::Unsupported {
            feature: "mb20:join".into(),
        })
    }

    pub async fn leave(&self) -> Result<(), NzError> {
        Err(NzError::Unsupported {
            feature: "mb20:leave".into(),
        })
    }

    pub fn members(&self) -> Vec<PartyMember> {
        Vec::new()
    }
}
