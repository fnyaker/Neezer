//! What a subject may see: my own devices see everything, a trusted friend sees
//! exactly what my roster shares with them, and nobody else gets a connection
//! (docs/agents/lan-nodes.md, *Authorisation and scope*).
//!
//! **LN0 declares `Scope`; LN4 fills it.**
//!
//! This is the security core of the responder, and its rule is structural: every
//! serving handler takes `&Scope` in its signature, so there is no handler that
//! can forget it, and an out-of-scope request is answered by the SAME function
//! that answers an absent file — byte for byte, so a friend cannot learn from the
//! shape of a refusal whether a track exists (T7).
//!
//! `Scope::None` is never constructed for a live connection: a subject that is
//! neither mine nor in my roster is refused at authentication, before any handler
//! exists to call.

use std::collections::HashSet;
use std::sync::Arc;

use crate::cred::Roster;
use crate::types::Subject;

/// What one peer may fetch from this device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// The same subject: the person's own devices, which see everything on disk.
    Own,
    /// A trusted friend, and the union of what my roster shares with them.
    Friend {
        /// Everything I have locally (`"all": true` in my roster).
        all: bool,
        /// Otherwise: the union of the shared playlists' tracks, as the server
        /// saw them when it issued the roster. Shared behind an `Arc` because
        /// every connection of the same friend clones it without copying 20 000
        /// ids.
        tracks: Arc<HashSet<String>>,
    },
}

impl Scope {
    /// The scope `peer` gets from MY roster, or `None` when it gets no connection
    /// at all.
    ///
    /// `mine` is my own subject: a peer with the same subject is [`Scope::Own`]
    /// whatever the roster says; a peer named in `roster.shares_to` gets exactly
    /// that entry; anybody else is [`None`] and the connection is closed before
    /// the first request.
    pub fn for_peer(peer: &Subject, mine: &Subject, roster: Option<&Roster>) -> Option<Scope> {
        let _ = (peer, mine, roster);
        None
    }

    /// Whether this scope allows one track. [`Scope::Own`] allows everything; a
    /// friend's set is whatever the server signed.
    pub fn allows(&self, track: &str) -> bool {
        let _ = track;
        false
    }

    /// The number of track ids a friend's scope carries, 0 for one's own devices.
    /// A counted number, for the memory measure and the log — never the ids.
    pub fn shared_tracks(&self) -> usize {
        match self {
            Scope::Own => 0,
            Scope::Friend { tracks, .. } => tracks.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub authorises nobody. That is the safe direction: a lot that forgot to
    /// fill `for_peer` closes every connection instead of serving one stranger.
    #[test]
    fn an_unimplemented_scope_authorises_nobody() {
        let mine = Subject([1; 16]);
        assert_eq!(Scope::for_peer(&mine, &mine, None), None);
        assert!(!Scope::Own.allows("3135556"));
        assert_eq!(Scope::Own.shared_tracks(), 0);
        let friend = Scope::Friend {
            all: false,
            tracks: Arc::new(HashSet::from(["3135556".to_owned()])),
        };
        assert_eq!(friend.shared_tracks(), 1);
    }
}
