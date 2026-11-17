//! Remote control: be controlled by another player, and control one. Owned by
//! **MB19** (the transport is SC5's `nz-remote`).
//!
//! The note pins "links create/revoke/list, host loop; control: claim
//! `/rc/<token>`, the remote player's state, commands" (docs/agents/mobile.md,
//! "Screen lots, wave 3"). MB0 declared those names; MB19 fills the bodies.

use crate::errors::NzError;
use crate::player::PlayerSnapshot;

/// This device's role in a remote session.
#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum RemoteRole {
    /// Nobody is controlling this player.
    None,
    /// Another device drives this one (the banner names it).
    Controlled { host_name: String },
    /// This device drives another one (the host chip names it).
    Controller { host_name: String },
}

/// A link this device made for another one to claim.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct RemoteLink {
    /// The `/rc/<token>` URL to share, built from the configured origin.
    pub url: String,
    pub token: String,
    /// The device that claimed it, once one did.
    pub claimed_by: Option<String>,
    /// ISO-8601, as the server answered.
    pub expires_at: String,
}

/// Remote control. MB19 fills it; MB0 refuses every call.
#[derive(uniffi::Object)]
pub struct RemoteApi;

#[uniffi::export(async_runtime = "tokio")]
impl RemoteApi {
    pub fn role(&self) -> RemoteRole {
        RemoteRole::None
    }

    /// Makes a claim link and returns it, so the sheet can share it.
    pub async fn create_link(&self) -> Result<RemoteLink, NzError> {
        Err(NzError::Unsupported {
            feature: "mb19:create_link".into(),
        })
    }

    pub async fn revoke_link(&self, token: String) -> Result<(), NzError> {
        let _ = token;
        Err(NzError::Unsupported {
            feature: "mb19:revoke_link".into(),
        })
    }

    /// Every link this device made, so the sheet can list and revoke them.
    pub async fn links(&self) -> Result<Vec<RemoteLink>, NzError> {
        Err(NzError::Unsupported {
            feature: "mb19:links".into(),
        })
    }

    /// Claims `/rc/<token>`: from now on this device controls that player.
    pub async fn claim(&self, token: String) -> Result<(), NzError> {
        let _ = token;
        Err(NzError::Unsupported {
            feature: "mb19:claim".into(),
        })
    }

    /// The controlled player's state (the one this device drives).
    pub fn remote_state(&self) -> Option<PlayerSnapshot> {
        None
    }

    /// Drops the link (host side and controller side are one call).
    pub async fn release(&self) -> Result<(), NzError> {
        Err(NzError::Unsupported {
            feature: "mb19:release".into(),
        })
    }

    /// Sends a transport command to the controlled player, on the `User` lane.
    /// A command is never queued across an outage (party-remote.md's rule).
    pub async fn send(&self, command: String) -> Result<(), NzError> {
        let _ = command;
        Err(NzError::Unsupported {
            feature: "mb19:send".into(),
        })
    }
}
