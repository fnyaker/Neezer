//! What a subject may see: my own devices see everything, a trusted friend sees
//! exactly what my roster shares with them, and nobody else gets a connection
//! (docs/agents/lan-nodes.md, *Authorisation and scope*).
//!
//! **LN4 filled it.** This is the security core of the responder, and its rule is
//! structural: every serving handler takes `&Scope` in its signature, so there is
//! no handler that can forget it, and an out-of-scope request is answered by the
//! SAME function that answers an absent file — byte for byte, so a friend cannot
//! learn from the shape of a refusal whether a track exists (T7).
//!
//! `Scope::None` is never constructed for a live connection: a subject that is
//! neither mine nor in my roster is refused at authentication, before any handler
//! exists to call. [`Scope::for_peer`] therefore answers `None`, and LN6's
//! authentication closes the connection on it.

use std::collections::HashSet;
use std::sync::Arc;

use crate::cred::{Roster, Share};
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
        /// Otherwise: the union of the shared playlists' tracks, as the server saw
        /// them when it issued the roster. Shared behind an `Arc` because every
        /// connection of the same friend clones it without copying 20 000 ids.
        tracks: Arc<HashSet<String>>,
    },
}

impl Scope {
    /// The scope `peer` gets from MY roster, or `None` when it gets no connection
    /// at all.
    ///
    /// `mine` is my own subject: a peer with the same subject is [`Scope::Own`]
    /// whatever the roster says; a peer named in `roster.shares_to` gets exactly
    /// that entry; anybody else is `None` and the connection is closed before the
    /// first request.
    pub fn for_peer(peer: &Subject, mine: &Subject, roster: Option<&Roster>) -> Option<Scope> {
        if peer == mine {
            // My own device: it never needed the roster, and a device of mine
            // keeps seeing everything while the roster is being refreshed.
            return Some(Scope::Own);
        }
        let share = roster?.share_to(peer)?;
        Some(Scope::from_share(share))
    }

    /// The scope one `shares_to` entry of my roster gives: everything, or exactly
    /// the union of the listed playlists' tracks.
    ///
    /// Split out of [`Scope::for_peer`] so the rule ("the union, nothing else") is
    /// testable without a server-signed roster: `Roster` has no public
    /// constructor, a `Share` does.
    pub fn from_share(share: &Share) -> Scope {
        if share.all {
            return Scope::Friend {
                all: true,
                tracks: Arc::new(HashSet::new()),
            };
        }
        let mut tracks = HashSet::new();
        for playlist in &share.playlists {
            for track in &playlist.tracks {
                tracks.insert(track.clone());
            }
        }
        Scope::Friend {
            all: false,
            tracks: Arc::new(tracks),
        }
    }

    /// Whether this scope allows one track. [`Scope::Own`] allows everything; a
    /// friend's set is whatever the server signed.
    pub fn allows(&self, track: &str) -> bool {
        match self {
            Scope::Own => true,
            Scope::Friend { all: true, .. } => true,
            Scope::Friend { all: false, tracks } => tracks.contains(track),
        }
    }

    /// Whether this is one of the person's own devices (everything on disk).
    pub fn is_own(&self) -> bool {
        matches!(self, Scope::Own)
    }

    /// Whether this scope is "everything I have" — the person's own devices, or a
    /// friend the roster says `"all": true` to.
    ///
    /// A handler that would SEND something about the library itself (a set's own
    /// cover, a count) asks this, never [`Scope::allows`] on one id: a track list
    /// is not a whole library.
    pub fn is_all(&self) -> bool {
        match self {
            Scope::Own => true,
            Scope::Friend { all, .. } => *all,
        }
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
    use crate::cred::SharedPlaylist;

    fn share(tracks: &[&[&str]], all: bool) -> Share {
        Share {
            subject: Subject([2; 16]),
            all,
            playlists: tracks
                .iter()
                .enumerate()
                .map(|(n, tracks)| SharedPlaylist {
                    id: format!("p{n}"),
                    title: format!("Liste {n}"),
                    tracks: tracks.iter().map(|track| (*track).to_owned()).collect(),
                })
                .collect(),
        }
    }

    /// T7: a friend's scope is the UNION of the shared playlists and nothing else.
    /// A track that is not in any shared playlist is refused, whether or not this
    /// device has it — `allows` never consults the inventory, so it cannot leak
    /// one either.
    #[test]
    fn a_friend_sees_exactly_the_union_of_the_shared_playlists() {
        let scope = Scope::from_share(&share(&[&["a", "b"], &["c"]], false));
        for allowed in ["a", "b", "c"] {
            assert!(scope.allows(allowed), "{allowed} is shared");
        }
        for refused in ["d", "", "abc", "A"] {
            assert!(!scope.allows(refused), "{refused} is not shared");
        }
        assert_eq!(scope.shared_tracks(), 3, "the union counts each id once");
        assert!(!scope.is_all(), "a list of tracks is not the whole library");
        assert!(!scope.is_own());
    }

    /// `"all": true` is everything, and `Own` is everything without a roster.
    #[test]
    fn all_means_everything_and_own_means_everything() {
        let everything = Scope::from_share(&share(&[], true));
        assert!(everything.allows("anything"));
        assert!(everything.is_all());
        assert_eq!(everything.shared_tracks(), 0);

        assert!(Scope::Own.allows("anything"));
        assert!(Scope::Own.is_all());
        assert!(Scope::Own.is_own());
        assert_eq!(Scope::Own.shared_tracks(), 0);
    }

    /// My own device is `Own` whatever the roster says, and a peer with no roster
    /// at all gets nothing: the two refusals that must never depend on the
    /// inventory.
    ///
    /// The rest of `for_peer` (a signed roster's `shares_to` entry → `from_share`)
    /// is the two lines `Roster::share_to` + `Scope::from_share`, and `Roster` has
    /// no public constructor — a roster only ever comes from a server-signed blob
    /// (LN2's `Roster::decode_verify`, whose stub answers `Err` until that lot
    /// lands). The friend half is therefore proven on `Scope::from_share` here and
    /// end to end by LN15's two-node journey.
    #[test]
    fn my_own_device_is_mine_and_a_stranger_gets_no_connection() {
        let mine = Subject([1; 16]);
        assert_eq!(Scope::for_peer(&mine, &mine, None), Some(Scope::Own));

        let other = Subject([2; 16]);
        assert_eq!(
            Scope::for_peer(&other, &mine, None),
            None,
            "no roster means nobody is a friend yet"
        );
    }
}
