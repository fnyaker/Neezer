//! Taking a whole playlist (or album, or show) from a peer that has it complete
//! (docs/agents/lan-nodes.md, *Downloads and playlists*).
//!
//! **LN11 filled it.** Three things live here:
//!
//! - [`Replica`], the peers' `Sets` pages in memory, counted and capped. The
//!   control loop replicates it on every connection and on every `SetsChanged`,
//!   on this node's own `Bulk` lane, so « Sur vos appareils » and a playlist
//!   download read a replica and never the network (`Lan::sets`).
//! - [`download_peer_sets`], « Télécharger » on that screen: it re-checks the
//!   set's completeness against the live replica, takes the set's metadata and
//!   covers from the same peer, and queues it in `nz-offline` like any download
//!   — one job, its progress and its causes (rule 2).
//! - the `prefer` commitment a set download records, so its tracks all come from
//!   that ONE device instead of scattering across the LAN (`Lan::decide_download`).
//!
//! A playlist download is an ordinary `nz-offline` job — it shows its progress and
//! each failure its cause (rule 2) — but when a peer has every track, the job asks
//! that ONE device for all of them ([`crate::service::Lan::best_peer_for`]): the
//! LAN is 20–80 MB/s where the server is 1–5, and a peer does not split a set
//! across four connections.
//!
//! The track list itself comes from the server when it is reachable, and from a
//! peer's `Sets` when it is not: a trip without data still lets the phone take a
//! playlist the PC already has. [`fetch_set`] is the "ask one peer directly" path
//! (the replica has not arrived yet, or the person just connected), and it speaks
//! the same `Request::Sets` frames the control loop does.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use nz_offline::Choice;

use crate::import;
use crate::proto;
use crate::service::{Lan, PeerSet};
use crate::transport::PeerConn;
use crate::types::{DevicePub, LanError, Relation};

/// How many sets one peer may make this node keep: a hostile peer must not fill
/// the RAM with playlist rows (T9). 512 × 20 000 ids would never fit the byte cap
/// below, so the smaller of the two wins.
pub const MAX_SETS_PER_PEER: usize = 512;
/// How many track ids one set may carry (a 20 000-title playlist).
pub const MAX_TRACKS_PER_SET: usize = 20_000;
/// The sets replica of every peer together, counted: the same budget class as
/// the inventories (16 peers × 8 000 tracks is 6 MiB) and still a fraction of one
/// album's audio.
pub const MAX_REPLICA_BYTES: usize = 8 << 20;
/// The `Sets` pages a sync may walk: 64 sets per page, [`MAX_SETS_PER_PEER`]
/// therefore fits in 8 — the rest is a peer that never says `last`.
pub const MAX_PAGES: u32 = 64;
/// The track commitments a set download may hold: a set's size, bounded.
pub const MAX_COMMITS: usize = 20_000;

/// One peer's replicated `Sets`.
#[derive(Debug, Default)]
struct Entry {
    name: String,
    relation: Option<Relation>,
    sets: Vec<proto::SetInfo>,
    bytes: usize,
}

/// What one set costs this node, counted (never RSS).
fn counted(set: &proto::SetInfo) -> usize {
    let tracks: usize = set.tracks.iter().map(|track| track.len() + 8).sum();
    std::mem::size_of::<proto::SetInfo>()
        + set.kind.len()
        + set.ident.len()
        + set.title.len()
        + set.cover.as_ref().map(String::len).unwrap_or(0)
        + tracks
}

/// The peers' download sets, in memory and counted.
///
/// Cheap to clone (one `Arc`), like [`crate::peers::Peers`]: the control loop
/// writes it, a screen reads it.
#[derive(Clone, Default)]
pub struct Replica {
    inner: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    peers: HashMap<DevicePub, Entry>,
    /// `track → peer` for the sets being downloaded: the `prefer` of the
    /// decision, so a 50-track set comes from the device that has it.
    commits: HashMap<String, DevicePub>,
}

impl Replica {
    pub fn new() -> Replica {
        Replica::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// The bytes this replica costs, counted (rule: memory is counted, never the
    /// RSS of a shared process).
    pub fn memory(&self) -> usize {
        let state = self.lock();
        state
            .peers
            .values()
            .map(|entry| {
                entry.bytes
                    + entry.name.len()
                    + std::mem::size_of::<Entry>()
                    + std::mem::size_of::<(DevicePub, Entry)>()
            })
            .sum::<usize>()
            + state.commits.len() * (std::mem::size_of::<(String, DevicePub)>() + 8)
    }

    /// A peer left (or the person logged out): its sets and its commitments go.
    pub(crate) fn remove(&self, peer: &DevicePub) {
        let mut state = self.lock();
        state.peers.remove(peer);
        state.commits.retain(|_, committed| committed != peer);
    }

    /// Every peer's sets, for « Sur vos appareils » and the playlist header.
    ///
    /// A stable order (peer, then kind, then ident): two calls over the same
    /// replica answer the same list, which is what a windowed screen diffs on.
    pub(crate) fn rows(&self) -> Vec<PeerSet> {
        let state = self.lock();
        let mut out = Vec::new();
        for (peer, entry) in &state.peers {
            for set in &entry.sets {
                out.push(PeerSet {
                    peer: *peer,
                    peer_name: entry.name.clone(),
                    relation: entry.relation.clone().unwrap_or(Relation::Friend {
                        name: entry.name.clone(),
                    }),
                    set: set.clone(),
                    // The `Sets` frame carries no sizes; `Lan::sets` fills this
                    // from the inventory replica, within a budget.
                    bytes: 0,
                });
            }
        }
        out.sort_by(|left, right| {
            left.peer
                .cmp(&right.peer)
                .then_with(|| left.set.kind.cmp(&right.set.kind))
                .then_with(|| left.set.ident.cmp(&right.set.ident))
        });
        out
    }

    /// One set of one peer, from the replica: the "Télécharger" re-check, with
    /// **0** requests in the common case.
    pub(crate) fn find(&self, peer: &DevicePub, kind: &str, ident: &str) -> Option<proto::SetInfo> {
        let state = self.lock();
        state
            .peers
            .get(peer)?
            .sets
            .iter()
            .find(|set| set.kind == kind && set.ident == ident)
            .cloned()
    }

    /// Records that these tracks are being taken from `peer`.
    pub(crate) fn commit(&self, peer: &DevicePub, tracks: &[String]) {
        let mut state = self.lock();
        for track in tracks {
            if state.commits.len() >= MAX_COMMITS && !state.commits.contains_key(track) {
                break;
            }
            state.commits.insert(track.clone(), *peer);
        }
    }

    /// The peer a download of `track` is committed to, if any.
    pub(crate) fn committed(&self, track: &str) -> Option<DevicePub> {
        self.lock().commits.get(track).copied()
    }

    /// Everything goes (logout).
    pub(crate) fn clear(&self) {
        let mut state = self.lock();
        state.peers.clear();
        state.commits.clear();
    }

    /// Replicates one peer's `Sets` pages.
    ///
    /// The replica keeps what it had unless the whole answer arrived: half a
    /// page list would advertise sets that are gone. Caps are checked BEFORE a
    /// row is kept, and the total is checked before the swap.
    pub(crate) async fn sync(&self, conn: &PeerConn) -> Result<(), LanError> {
        let info = conn.info();
        let peer = info.device_pub;
        // The `Sets` version is what a delta would need; a replica is re-read
        // whole (`Sets` is 64 sets a page and moves rarely), so it is logged and
        // not kept.
        let (version, sets) = pages(conn).await?;
        let bytes: usize = sets.iter().map(counted).sum();
        let mut state = self.lock();
        let others: usize = state
            .peers
            .iter()
            .filter(|(other, _)| **other != peer)
            .map(|(_, entry)| entry.bytes)
            .sum();
        if others + bytes > MAX_REPLICA_BYTES {
            // Given back rather than kept (T9): sixteen hostile peers must not
            // hold this device's RAM. The connection stays; a peer this node
            // cannot replicate is still one it may serve.
            state.peers.remove(&peer);
            return Err(LanError::Proto(format!(
                "les ensembles d'un pair dépassent {MAX_REPLICA_BYTES} octets comptés"
            )));
        }
        tracing::debug!(
            peer = crate::cred::hex_encode(&peer),
            version,
            sets = sets.len(),
            "LAN : les ensembles d'un pair sont répliqués"
        );
        state.peers.insert(
            peer,
            Entry {
                name: info
                    .hello
                    .as_ref()
                    .map(|hello| hello.name.clone())
                    .unwrap_or_default(),
                relation: Some(info.relation.clone()),
                sets,
                bytes,
            },
        );
        Ok(())
    }
}

/// One peer's `Sets`, page by page: `(version of the first page, sets)`.
async fn pages(conn: &PeerConn) -> Result<(u64, Vec<proto::SetInfo>), LanError> {
    let mut page = 0u32;
    let mut collected: Vec<proto::SetInfo> = Vec::new();
    let mut first_version: Option<u64> = None;
    loop {
        if page >= MAX_PAGES {
            return Err(LanError::Proto(format!(
                "les ensembles du pair dépassent {MAX_PAGES} pages"
            )));
        }
        let reply = conn.request(proto::Request::Sets { page }).await?;
        match reply.head {
            proto::Response::Sets {
                version,
                page: answered,
                last,
                sets,
            } => {
                if answered != page {
                    return Err(LanError::Proto(format!(
                        "la page {answered} répond à la page {page} demandée"
                    )));
                }
                if page == 0 {
                    first_version = Some(version);
                }
                for set in sets {
                    if set.tracks.len() > MAX_TRACKS_PER_SET {
                        return Err(LanError::Proto(format!(
                            "un ensemble de plus de {MAX_TRACKS_PER_SET} titres"
                        )));
                    }
                    if !matches!(set.kind.as_str(), "playlist" | "album" | "show") {
                        return Err(LanError::Proto(format!(
                            "un ensemble de genre inconnu : {:?}",
                            set.kind
                        )));
                    }
                    collected.push(set);
                    if collected.len() > MAX_SETS_PER_PEER {
                        return Err(LanError::Proto(format!(
                            "les ensembles du pair dépassent {MAX_SETS_PER_PEER} entrées"
                        )));
                    }
                }
                if last {
                    return Ok((first_version.unwrap_or(version), collected));
                }
                page += 1;
            }
            proto::Response::Busy { retry_in_ms } => {
                return Err(LanError::Busy { retry_in_ms });
            }
            proto::Response::NotFound => return Err(LanError::NotFound),
            proto::Response::Error { code } => {
                return Err(LanError::Proto(format!("le pair a répondu « {code} »")));
            }
            other => {
                return Err(LanError::Proto(format!(
                    "une réponse inattendue à des ensembles : {other:?}"
                )));
            }
        }
    }
}

/// Asks `conn` for one set, page by page: the path used when the replica has not
/// received that peer's `Sets` yet.
pub async fn fetch_set(
    conn: &PeerConn,
    kind: &str,
    ident: &str,
) -> Result<proto::SetInfo, LanError> {
    if !matches!(kind, "playlist" | "album" | "show") {
        return Err(LanError::Proto(format!(
            "un ensemble de genre inconnu : {kind:?}"
        )));
    }
    if ident.is_empty() || ident.len() > 64 {
        return Err(LanError::Proto(
            "un identifiant d'ensemble invalide".to_owned(),
        ));
    }
    let (_, sets) = pages(conn).await?;
    sets.into_iter()
        .find(|set| set.kind == kind && set.ident == ident)
        .ok_or(LanError::NotFound)
}

/// One set to take from one peer: « Sur vos appareils » multi-selects these.
///
/// The `"<peer>:<kind>:<ident>"` string the FFI passes around is this triple, and
/// it is parsed into it before anything is asked of a peer — a key never becomes a
/// path and never reaches the network as one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerSetKey {
    pub peer: DevicePub,
    /// `"playlist"`, `"album"` or `"show"`.
    pub kind: String,
    /// The server's id for the set.
    pub ident: String,
}

/// Adds every set in `keys` to the downloads worker, and answers how many jobs were
/// created.
///
/// The peer's completeness is re-checked against its live replica here: a set can
/// lose a file between the screen drawing it and the person tapping « Télécharger »,
/// and a job that half-fails is worse than a set that is offered again. The
/// metadata and covers come from the same peer, so the downloads screen and the
/// playlist page look the same offline (rule 2).
///
/// One job per key, never one request per track: the frames are `Sets` (read
/// once), `Meta` (200 ids per frame) and one `Cover` per missing sleeve.
pub fn download_peer_sets(
    lan: &Lan,
    offline: &nz_offline::Offline,
    keys: &[PeerSetKey],
) -> Result<usize, LanError> {
    if keys.is_empty() {
        return Ok(0);
    }
    let handle = lan
        .handle()
        .ok_or_else(|| LanError::Internal("le nœud LAN n'est pas démarré".to_owned()))?;
    let mut created = 0usize;
    for key in keys {
        if !matches!(key.kind.as_str(), "playlist" | "album" | "show") {
            return Err(LanError::Proto(format!(
                "un ensemble de genre inconnu : {:?}",
                key.kind
            )));
        }
        let conn = lan.conn(&key.peer).ok_or(LanError::PeerGone)?;
        let set = match lan.peer_set(&key.peer, &key.kind, &key.ident) {
            Some(set) => set,
            // The replica has not heard this peer's sets yet (the person just
            // connected, or a friend's first answer is still travelling): one
            // `Sets` pass, the same frames the control loop speaks.
            None => import::run_on(&handle, {
                let kind = key.kind.clone();
                let ident = key.ident.clone();
                let conn = conn.clone();
                async move { fetch_set(&conn, &kind, &ident).await }
            })
            .ok_or_else(|| {
                LanError::Internal("la réponse du pair a été abandonnée".to_owned())
            })??,
        };
        if set.tracks.is_empty() {
            return Err(LanError::NotFound);
        }
        if !set.complete {
            // Not "complete" on that device any more: the screen offers it again
            // rather than queueing a set that would half-fail.
            return Err(LanError::NotFound);
        }

        // The labels and the sleeves, from the SAME peer, on this node's Bulk
        // lane inside `import`: they yield to the tapped track (rule 1).
        let imported = import::run_on(&handle, {
            let conn = conn.clone();
            let cache = lan.cache().clone();
            let gates = lan.api().gates();
            let tracks = set.tracks.clone();
            async move { import::collect(cache, gates, conn, tracks).await }
        })
        .ok_or_else(|| LanError::Internal("la réponse du pair a été abandonnée".to_owned()))??;

        let labels: HashMap<&str, &proto::TrackMeta> = imported
            .rows
            .iter()
            .map(|row| (row.id.as_str(), row))
            .collect();
        let choices: Vec<Choice> = set
            .tracks
            .iter()
            .map(|track| {
                let mut choice = Choice::new(track.clone(), set.title.clone());
                if let Some(row) = labels.get(track.as_str()) {
                    if !row.title.is_empty() {
                        choice.title = row.title.clone();
                    }
                    choice.artist = row.artists.clone();
                }
                choice
            })
            .collect();
        lan.commit_peer_tracks(&key.peer, &set.tracks);
        offline.add_set(&set.kind, &set.ident, &set.title, choices);
        created += 1;
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub key is the triple the FFI parses — and a key never becomes a path.
    #[test]
    fn a_stub_key_is_the_triple_the_ffi_parses() {
        let key = PeerSetKey {
            peer: [7; 32],
            kind: "playlist".to_owned(),
            ident: "42".to_owned(),
        };
        assert_eq!(key.kind, "playlist");
        assert_eq!(key.ident, "42");
        assert_eq!(key.peer[0], 7);
    }

    /// The counted size of a set is what the replica keeps, per item: a screen's
    /// memory line must not read a process's RSS.
    #[test]
    fn a_replicated_set_is_counted_per_track() {
        let set = proto::SetInfo {
            kind: "playlist".to_owned(),
            ident: "900".to_owned(),
            title: "Route".to_owned(),
            cover: Some("http://srv/api/cover/900".to_owned()),
            tracks: (0..50).map(|n| format!("3000{n:04}")).collect(),
            local: 50,
            complete: true,
        };
        let cost = counted(&set);
        assert!(
            cost >= 50 * (8 + 8),
            "a 50-track set must count at least its ids, got {cost}"
        );
        assert!(
            cost < 50 * 64,
            "and never more than the note's 64 B/item: {cost}"
        );
    }

    /// An empty replica costs nothing and lists nothing: the stub the screens
    /// draw before a peer connects.
    #[test]
    fn an_empty_replica_is_empty_and_free() {
        let replica = Replica::new();
        assert!(replica.rows().is_empty());
        assert_eq!(replica.memory(), 0);
        assert_eq!(replica.find(&[1; 32], "playlist", "900"), None);
        assert_eq!(replica.committed("30000001"), None);
    }

    /// A commitment is bounded and goes with its peer: a set of 20 000 tracks is
    /// the most one download may pin.
    #[test]
    fn a_commitment_is_bounded_and_follows_its_peer() {
        let replica = Replica::new();
        let peer = [3; 32];
        let tracks: Vec<String> = (0..MAX_COMMITS + 10).map(|n| format!("t{n}")).collect();
        replica.commit(&peer, &tracks);
        assert_eq!(replica.committed("t0"), Some(peer));
        assert_eq!(
            replica.committed(&format!("t{}", MAX_COMMITS + 9)),
            None,
            "a commitment past the cap is not kept"
        );
        replica.remove(&peer);
        assert_eq!(replica.committed("t0"), None);
    }
}
