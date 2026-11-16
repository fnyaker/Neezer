//! Remote control: the links the OWNER creates onto one of their players, and
//! the grant a controller runs under.
//!
//! The live traffic (the host's publish/poll, the controller's state/commands)
//! is not in this crate: it is the player state machine's and the API's job
//! (D15a). Here is only what the two READ routes answer.

use serde::{Deserialize, Serialize};

use crate::ids::UserId;

/// One live remote-control link.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoteLink {
    #[serde(default)]
    pub id: String,
    /// The capability token the link URL carries.
    #[serde(default)]
    pub token: String,
    /// `"queue"`, `"read"`, `"full"`, `"admin"`.
    #[serde(default)]
    pub level: String,
    /// The person's own label for this controller.
    #[serde(default)]
    pub label: String,
    /// The device id the controller claimed.
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub expires: Option<String>,
    #[serde(default)]
    pub last_used: Option<String>,
    /// How many controllers hold this link right now.
    #[serde(default)]
    pub controllers: u32,
}

/// `/api/remote/links` — this person's live links, and the levels to pick from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoteLinks {
    #[serde(default)]
    pub links: Vec<RemoteLink>,
    /// The levels the server would accept, weakest first.
    #[serde(default)]
    pub levels: Vec<String>,
}

/// The ceiling a controller may not cross, and how strong it is.
///
/// The server enforces it; the UI only hides what would be refused, so this is
/// the one place the ordering is written down on the desktop's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RemoteLevel {
    /// Only queue/transport commands.
    Queue,
    /// Queue plus reading what is playing.
    Read,
    /// Everything the owner can do with the player.
    Full,
    /// Administration too.
    Admin,
}

impl RemoteLevel {
    /// Reads a level name as the server spells it.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "queue" => Some(RemoteLevel::Queue),
            "read" => Some(RemoteLevel::Read),
            "full" => Some(RemoteLevel::Full),
            "admin" => Some(RemoteLevel::Admin),
            _ => None,
        }
    }

    /// The level's wire name.
    pub fn wire(self) -> &'static str {
        match self {
            RemoteLevel::Queue => "queue",
            RemoteLevel::Read => "read",
            RemoteLevel::Full => "full",
            RemoteLevel::Admin => "admin",
        }
    }
}

/// The owner of a borrowed player, as a remote link names them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoteOwner {
    #[serde(default)]
    pub id: Option<UserId>,
    #[serde(default)]
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_levels_order_the_way_the_server_ranks_them() {
        assert!(RemoteLevel::Queue < RemoteLevel::Read);
        assert!(RemoteLevel::Read < RemoteLevel::Full);
        assert!(RemoteLevel::Full < RemoteLevel::Admin);
        assert_eq!(RemoteLevel::parse("full"), Some(RemoteLevel::Full));
        assert_eq!(RemoteLevel::parse("god"), None);
        assert_eq!(RemoteLevel::Admin.wire(), "admin");
    }

    #[test]
    fn a_link_lists_its_live_controllers() {
        let links: RemoteLinks = serde_json::from_str(
            r#"{"levels":["queue","read","full","admin"],
                "links":[{"id":"abc","token":"abc.mac","level":"queue","label":"Salon",
                  "device":"dev1","device_name":"Salon","created":null,"expires":null,
                  "last_used":null,"controllers":0}]}"#,
        )
        .expect("links");
        assert_eq!(links.links[0].device_name, "Salon");
        assert_eq!(links.levels.len(), 4);
    }
}
