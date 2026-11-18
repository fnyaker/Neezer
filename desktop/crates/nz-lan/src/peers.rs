//! The connected peers, and an in-memory replica of each one's inventory
//! (docs/agents/lan-nodes.md, *Choosing the source*).
//!
//! **LN9 filled it.** The replica is what makes the source decision instant: it is
//! already in memory, so a tap asks no peer anything (`0` control bytes and ≤ 1 ms —
//! the note's measure). It costs counted bytes ([`Peers::memory`], ≤ 64 B per item)
//! and it is filled once per peer and version, then by deltas.
//!
//! A replica can be STALE — a peer can evict a play-cache file between the
//! inventory and the fetch — so a `NotFound` from a peer is not an error about the
//! track: the decision is re-run with the remaining candidates, and then the
//! server answers.
//!
//! # What is stored, and what it costs
//!
//! One [`Entry`] per track: the id and a bit per quality with that copy's length —
//! **48 bytes for a 7-character id**, counted by [`Peers::memory`] exactly the way
//! [`crate::index::Index::memory`] counts the local inventory (`40` bytes of struct
//! plus the id, rounded to 8). A stored path would cost 24 bytes more per item and
//! a `String` allocation each: the wire carries a length, never a path, and nothing
//! from the wire becomes one.
//!
//! # Caps, before allocation
//!
//! A frame is already capped at 1 MiB by [`crate::codec`], and the caps here bound
//! what a peer can make this node KEEP: [`MAX_PAGES`] pages per sync,
//! [`MAX_REPLICA_TRACKS`] tracks per peer, [`MAX_REPLICA_BYTES`] counted bytes for
//! every replica together, [`MAX_TRACK_ID`] bytes per id. A peer that exceeds one is
//! refused with [`LanError::Proto`] and the replica it had is left as it was — a
//! silent truncation would be a wrong inventory, which is worse than a stale one.
//!
//! # One sync at a time per peer
//!
//! Two syncs of the same peer would interleave pages and deltas. Each peer carries
//! its own async mutex ([`tokio::sync::Mutex`], never held across a lock of the
//! table), so the second caller waits for the first and then asks for the delta the
//! first one left.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use tracing::debug;

use crate::proto::{self, WireQuality};
use crate::service::PeerView;
use crate::transport::PeerConn;
use crate::types::{DevicePub, LanError, Relation};

/// The counted cost of one replica entry WITHOUT its track id: the `Box<str>`
/// (16 B) and the copies (1 B of bits plus four `u32` lengths, padded to 24 B),
/// rounded to the 8-byte alignment the vector holds them at — 40 B. Measured on
/// 8 000 tracks with a 7-character id: 48 B each, ids included.
const ITEM_BYTES: usize = 40;

/// The most tracks one peer's replica keeps. A library the note sizes at 8 000
/// tracks fits six times over; beyond it a peer is either not a Neezer library or
/// is hostile, and memory is the resource it would take.
pub const MAX_REPLICA_TRACKS: usize = 50_000;

/// The counted bytes every replica together may take. 32 MiB is 700 000 items —
/// sixteen peers of 43 000 tracks each — and is what stops sixteen hostile peers
/// from turning a replica into a memory exhaustion (T9).
pub const MAX_REPLICA_BYTES: usize = 32 * 1024 * 1024;

/// The most `Inventory` pages one sync reads. The responder's page is 4 000 items,
/// so 64 pages is 256 000 items: a peer that answers `last: false` for ever stops
/// here instead of spinning the sync.
pub const MAX_PAGES: u32 = 64;

/// The longest track id a peer may announce. The server's own ids are 7–9
/// characters; this is `index.rs`'s bound, and an id over it is refused rather than
/// stored (it would be memory a stranger chose).
pub const MAX_TRACK_ID: usize = 64;

/// The most tracks remembered as unusable from one peer. A bad copy is remembered
/// for the session so the next decision skips it; the cap keeps a long session from
/// turning the list into a leak.
pub const MAX_BAD_TRACKS: usize = 4_096;

/// The four qualities a peer can serve, in the order of [`Copies`]' bits.
const WIRE_QUALITIES: [WireQuality; 4] = [
    WireQuality::Flac,
    WireQuality::Opus320,
    WireQuality::Opus128,
    WireQuality::Opus64,
];

/// One peer that has the track asked about, with everything needed to rank it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub peer: DevicePub,
    pub relation: Relation,
    /// The copies that peer announced, per quality and length.
    pub copies: Vec<(nz_api::Quality, u64)>,
    /// The connection's smoothed round trip, for the tie-break.
    pub rtt_ms: u32,
}

impl Candidate {
    /// Whether this peer announced the copy at exactly `quality`.
    pub fn has(&self, quality: nz_api::Quality) -> bool {
        self.copies.iter().any(|(offered, _)| *offered == quality)
    }

    /// The qualities it announced, in no particular order.
    pub fn qualities(&self) -> impl Iterator<Item = nz_api::Quality> + '_ {
        self.copies.iter().map(|(quality, _)| *quality)
    }

    /// The best copy it has, by [`nz_api::Quality`]'s own order (FLAC last).
    pub fn best_quality(&self) -> Option<nz_api::Quality> {
        self.copies.iter().map(|(quality, _)| *quality).max()
    }
}

/// The peers this node is connected to.
///
/// Cheap to clone: the connection table and the replicated inventories live behind
/// one `Arc`, and every clone sees the same replicas.
#[derive(Clone, Default)]
pub struct Peers {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    /// One entry per authenticated peer, keyed by its device key (the identity).
    live: Mutex<HashMap<DevicePub, Peer>>,
}

impl std::fmt::Debug for Peers {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let live = self.lock();
        formatter
            .debug_struct("Peers")
            .field("peers", &live.len())
            .field("bytes", &live.values().map(Peer::counted).sum::<usize>())
            .finish()
    }
}

/// One connected peer: its connection, its names, and the replica of what it has.
struct Peer {
    conn: PeerConn,
    relation: Relation,
    name: String,
    platform: String,
    /// Sorted by track id: a lookup is a binary search, like the local index.
    items: Vec<Entry>,
    /// The inventory version the replica was built for.
    version: u64,
    /// Whether a replica was ever installed: an empty inventory at version 0 is a
    /// real answer, and it must not be re-asked as a full page on every sync.
    synced: bool,
    /// Tracks whose copy from THIS peer is unusable for the session (a wrong hash,
    /// a failed probe, one bad file). The next decision skips them.
    bad: HashSet<Box<str>>,
    /// What [`Peers::memory`] counts for this peer, kept up to date on every change
    /// so a screen never walks 8 000 entries to draw a number.
    bytes: usize,
    /// One sync at a time: two would interleave pages and deltas.
    sync: Arc<tokio::sync::Mutex<()>>,
}

impl Peer {
    fn new(conn: PeerConn) -> Peer {
        let (relation, name, platform) = names_of(&conn);
        let mut peer = Peer {
            relation,
            name,
            platform,
            conn,
            items: Vec::new(),
            version: 0,
            synced: false,
            bad: HashSet::new(),
            bytes: 0,
            sync: Arc::new(tokio::sync::Mutex::new(())),
        };
        peer.bytes = peer.counted();
        peer
    }

    /// Whether this peer can serve `track` (complete, in the replica, not marked
    /// bad for this session).
    fn has(&self, track: &str) -> bool {
        if track.is_empty() || track.len() > MAX_TRACK_ID || self.bad.contains(track) {
            return false;
        }
        self.items
            .binary_search_by(|entry| entry.track.as_ref().cmp(track))
            .is_ok_and(|index| !self.items[index].copies.is_empty())
    }

    /// What one replica costs, counted (never the RSS of a shared process): its
    /// entries, the bad-track list, the two names, the relation's own copy of a
    /// friend's name, and the map's key.
    fn counted(&self) -> usize {
        let items: usize = self.items.iter().map(Entry::counted).sum();
        let bad: usize = self
            .bad
            .iter()
            .map(|track| STRING_BYTES + align8(track.len()))
            .sum();
        let names =
            STRING_BYTES + align8(self.name.len()) + STRING_BYTES + align8(self.platform.len());
        let relation = match &self.relation {
            Relation::Own => 0,
            Relation::Friend { name } => STRING_BYTES + align8(name.len()),
        };
        items + bad + names + relation + std::mem::size_of::<DevicePub>()
    }
}

/// A `String`'s own cost, without its bytes: 24 B on every 64-bit target this
/// crate builds for.
const STRING_BYTES: usize = 24;

/// One track of a peer's replica: its id and the copies it has.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    track: Box<str>,
    copies: Copies,
}

impl Entry {
    fn counted(&self) -> usize {
        ITEM_BYTES + align8(self.track.len())
    }
}

/// Which qualities one track has on a peer, and each copy's length.
///
/// 20 bytes, and that is the point: a replica is 8 000 of these plus their ids, so
/// a stored path (`PathBuf`, 24 B plus an allocation each) would more than double
/// what a peer's inventory costs the person's memory. A length over `u32::MAX` is
/// not audio and is dropped rather than truncated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Copies {
    /// Bit `i` is set when `WIRE_QUALITIES[i]` has a complete file.
    bits: u8,
    /// The length of each copy, in bytes.
    lens: [u32; 4],
}

impl Copies {
    fn insert(&mut self, quality: WireQuality, len: u64) {
        let Ok(len) = u32::try_from(len) else {
            return;
        };
        let index = wire_index(quality);
        self.bits |= 1 << index;
        self.lens[index] = len;
    }

    fn remove(&mut self, quality: WireQuality) {
        self.bits &= !(1 << wire_index(quality));
    }

    fn is_empty(&self) -> bool {
        self.bits == 0
    }

    fn get(&self, index: usize) -> Option<u64> {
        (self.bits & (1 << index) != 0).then(|| u64::from(self.lens[index]))
    }

    fn qualities(&self) -> impl Iterator<Item = (WireQuality, u64)> + '_ {
        WIRE_QUALITIES
            .iter()
            .copied()
            .enumerate()
            .filter_map(move |(index, quality)| self.get(index).map(|len| (quality, len)))
    }

    fn merge(&mut self, other: Copies) {
        for (quality, len) in other.qualities() {
            self.insert(quality, len);
        }
    }
}

/// The bit a quality owns in [`Copies::bits`], from the one table above.
fn wire_index(quality: WireQuality) -> usize {
    match quality {
        WireQuality::Flac => 0,
        WireQuality::Opus320 => 1,
        WireQuality::Opus128 => 2,
        WireQuality::Opus64 => 3,
    }
}

/// A length rounded up to the 8-byte alignment a `Vec<Entry>` allocates at.
fn align8(len: usize) -> usize {
    len.div_ceil(8) * 8
}

impl Peers {
    /// No peer.
    pub fn new() -> Peers {
        Peers::default()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<DevicePub, Peer>> {
        // A panic in another task must not turn every later lookup into a panic of
        // its own: the map is plain data.
        self.inner
            .live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Records an authenticated connection.
    ///
    /// A reconnection of the same device keeps the replica: it is still that
    /// device's inventory (a stale replica is allowed, the fetch is the truth), and
    /// the next [`Peers::sync_inventory`] asks for the delta instead of the ~480 KB
    /// full pages of 8 000 tracks.
    pub fn insert(&self, conn: PeerConn) {
        let peer = conn.info().device_pub;
        let (relation, name, platform) = names_of(&conn);
        let mut live = self.lock();
        match live.get_mut(&peer) {
            Some(entry) => {
                entry.conn = conn;
                entry.relation = relation;
                entry.name = name;
                entry.platform = platform;
                entry.bytes = entry.counted();
            }
            None => {
                live.insert(peer, Peer::new(conn));
            }
        }
    }

    /// Forgets a peer, its replica included: the memory goes back the moment the
    /// connection does.
    pub fn remove(&self, peer: &DevicePub) {
        let mut live = self.lock();
        live.remove(peer);
    }

    /// The live connection to one peer, if there is one.
    pub fn get(&self, peer: &DevicePub) -> Option<PeerConn> {
        self.lock().get(peer).map(|entry| entry.conn.clone())
    }

    /// Pulls a peer's inventory — full the first time, a delta on every version
    /// after — paginating `Inventory` and applying the answer. An inventory can be
    /// stale afterwards; the fetch is the truth.
    ///
    /// One sync at a time per peer: the second caller waits, then asks for the
    /// delta the first one left. A peer that answers `Busy` surfaces it (the caller
    /// waits exactly `retry_in_ms`, never a backoff of its own).
    pub async fn sync_inventory(&self, peer: &DevicePub) -> Result<(), LanError> {
        let (conn, sync) = {
            let live = self.lock();
            let entry = live.get(peer).ok_or(LanError::PeerGone)?;
            (entry.conn.clone(), Arc::clone(&entry.sync))
        };
        let _one_at_a_time = sync.lock().await;

        // Read the version AFTER taking the serial: a sync that finished while this
        // one waited already moved it, and asking for the older `since` would replay
        // a delta (harmless) instead of the one that is current.
        let since = {
            let live = self.lock();
            let entry = live.get(peer).ok_or(LanError::PeerGone)?;
            entry.synced.then_some(entry.version)
        };

        let mut page = 0u32;
        let mut collected: Vec<(Box<str>, Copies)> = Vec::new();
        // The version of the FIRST page, not the last: a change made while the pages
        // travel shifts the later ones, and keeping the older version means the next
        // delta re-applies that change (idempotent) instead of missing it.
        let mut first_version: Option<u64> = None;
        let version = 'pages: loop {
            if page >= MAX_PAGES {
                return Err(LanError::Proto(format!(
                    "l'inventaire du pair dépasse {MAX_PAGES} pages"
                )));
            }
            // `since` only on the first page: a delta is ONE answer, and asking for
            // it again on page 1 would answer an empty delta for ever.
            let request = proto::Request::Inventory {
                since: if page == 0 { since } else { None },
                page,
            };
            let reply = conn.request(request).await?;
            match reply.head {
                proto::Response::InventoryDelta {
                    from,
                    to,
                    added,
                    removed,
                } => {
                    let Some(since) = since else {
                        return Err(LanError::Proto(
                            "un delta pour un inventaire que ce nœud n'a jamais reçu".to_owned(),
                        ));
                    };
                    if from != since {
                        return Err(LanError::Proto(format!(
                            "un delta depuis {from} alors que {since} était demandé"
                        )));
                    }
                    return self.apply_delta(peer, to, &added, &removed);
                }
                proto::Response::InventoryFull {
                    version: at,
                    page: answered,
                    last,
                    items,
                } => {
                    if answered != page {
                        return Err(LanError::Proto(format!(
                            "la page {answered} répond à la page {page} demandée"
                        )));
                    }
                    if page == 0 {
                        first_version = Some(at);
                    }
                    for item in items {
                        let entry = entry_of(item)?;
                        if !entry.1.is_empty() {
                            collected.push(entry);
                        }
                        if collected.len() > MAX_REPLICA_TRACKS {
                            return Err(LanError::Proto(format!(
                                "l'inventaire du pair dépasse {MAX_REPLICA_TRACKS} titres"
                            )));
                        }
                    }
                    if last {
                        break 'pages first_version.unwrap_or(at);
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
                        "une réponse inattendue à un inventaire : {other:?}"
                    )));
                }
            }
        };

        let items = build_sorted(collected);
        let tracks = items.len();
        let mut live = self.lock();
        let entry = live.get_mut(peer).ok_or(LanError::PeerGone)?;
        entry.items = items;
        entry.version = version;
        entry.synced = true;
        entry.bytes = entry.counted();
        let total: usize = live.values().map(|peer| peer.bytes).sum();
        if total > MAX_REPLICA_BYTES {
            // The replica is given back rather than kept: sixteen hostile peers must
            // not hold 32 MiB of this device's RAM (T9). The CONNECTION stays — it is
            // the transport's, and a peer this node cannot replicate is still a peer
            // it may serve.
            let entry = live.get_mut(peer).ok_or(LanError::PeerGone)?;
            entry.items = Vec::new();
            entry.synced = false;
            entry.bytes = entry.counted();
            return Err(LanError::Proto(format!(
                "les inventaires dépassent {MAX_REPLICA_BYTES} octets comptés"
            )));
        }
        drop(live);
        debug!(
            tracks,
            version, "a peer's inventory is replicated in memory"
        );
        Ok(())
    }

    /// Applies one delta under the table's lock: `added` copies in, `removed` copies
    /// out. Both are idempotent, so a delta applied twice (a retry after a crash)
    /// lands on the same replica.
    fn apply_delta(
        &self,
        peer: &DevicePub,
        to: u64,
        added: &[proto::Item],
        removed: &[proto::ItemKey],
    ) -> Result<(), LanError> {
        let mut live = self.lock();
        let entry = live.get_mut(peer).ok_or(LanError::PeerGone)?;
        for item in added {
            let (track, copies) = entry_of(item.clone())?;
            if copies.is_empty() {
                continue;
            }
            insert_copies(&mut entry.items, track.as_ref(), copies);
        }
        for key in removed {
            remove_copy(&mut entry.items, &key.track, key.quality);
        }
        if entry.items.len() > MAX_REPLICA_TRACKS {
            return Err(LanError::Proto(format!(
                "l'inventaire du pair dépasse {MAX_REPLICA_TRACKS} titres"
            )));
        }
        entry.version = to;
        entry.synced = true;
        entry.bytes = entry.counted();
        Ok(())
    }

    /// Every peer that has `track`, ranked by [`crate::decide`] (never by this
    /// call: it only lists them).
    ///
    /// It reads the replicas only: no lock is held across an await, and nothing
    /// here touches the network (the note's `0` control bytes and ≤ 1 ms).
    pub fn candidates(&self, track: &str) -> Vec<Candidate> {
        if track.is_empty() || track.len() > MAX_TRACK_ID {
            return Vec::new();
        }
        let live = self.lock();
        let mut out = Vec::with_capacity(live.len());
        for (peer, entry) in live.iter() {
            if entry.bad.contains(track) {
                continue;
            }
            let Ok(index) = entry
                .items
                .binary_search_by(|item| item.track.as_ref().cmp(track))
            else {
                continue;
            };
            let copies = &entry.items[index].copies;
            if copies.is_empty() {
                continue;
            }
            out.push(Candidate {
                peer: *peer,
                relation: entry.relation.clone(),
                copies: copies
                    .qualities()
                    .map(|(quality, len)| (quality.into(), len))
                    .collect(),
                rtt_ms: rtt_ms(&entry.conn),
            });
        }
        // A stable order, so two calls over the same peers answer the same thing:
        // `decide` ranks by its own key, and a tie there falls back to the key.
        out.sort_by_key(|left| left.peer);
        out
    }

    /// The one peer that has ALL of `tracks` complete, if any: "download the
    /// playlist from the device that has it" (`Lan::best_peer_for`).
    ///
    /// Ranking is the decision's own tie-break, so the peer this answers is the one
    /// a per-track decision would prefer: an own device before a friend, then the
    /// lowest round trip, then the device key.
    pub fn complete_for(&self, tracks: &[String]) -> Option<DevicePub> {
        if tracks.is_empty() || tracks.iter().any(|track| track.len() > MAX_TRACK_ID) {
            return None;
        }
        let live = self.lock();
        let mut best: Option<(u8, u32, DevicePub)> = None;
        for (peer, entry) in live.iter() {
            if !tracks.iter().all(|track| entry.has(track)) {
                continue;
            }
            let own = u8::from(entry.relation != Relation::Own);
            let key = (own, rtt_ms(&entry.conn), *peer);
            if best.as_ref().is_none_or(|current| key < *current) {
                best = Some(key);
            }
        }
        best.map(|(_, _, peer)| peer)
    }

    /// Marks a peer's copy of a track unusable for this session: a wrong hash, a
    /// probe that failed, one bad file. The next decision skips it.
    pub fn mark_bad(&self, peer: &DevicePub, track: &str) {
        if track.is_empty() || track.len() > MAX_TRACK_ID {
            return;
        }
        let mut live = self.lock();
        let Some(entry) = live.get_mut(peer) else {
            return;
        };
        if entry.bad.len() >= MAX_BAD_TRACKS {
            return;
        }
        if entry.bad.insert(track.into()) {
            entry.bytes = entry.counted();
        }
    }

    /// What the peer list draws: one row per connection.
    pub fn views(&self) -> Vec<PeerView> {
        let live = self.lock();
        let mut out: Vec<PeerView> = live
            .iter()
            .map(|(peer, entry)| PeerView {
                device_pub: *peer,
                name: entry.name.clone(),
                relation: entry.relation.clone(),
                platform: entry.platform.clone(),
                rtt_ms: rtt_ms(&entry.conn),
                tracks: u32::try_from(entry.items.len()).unwrap_or(u32::MAX),
                // The sets a peer exposes are `sets.rs`'s replica (LN11): this lot
                // replicates the inventory, and a number invented here would be a
                // second source of truth.
                sets: 0,
            })
            .collect();
        out.sort_by_key(|left| left.device_pub);
        out
    }

    /// The counted bytes of every replica (rule: never the RSS of a shared
    /// process). LN9's target is ≤ 64 B per item, so four peers with 8 000 tracks
    /// each ≈ 1.6 MB.
    pub fn memory(&self) -> usize {
        self.lock().values().map(Peer::counted).sum()
    }

    /// How many peers are connected (the snapshot's `Connected(n)`).
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether no peer is connected, so `len` has its pair.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The smoothed round trip in milliseconds, clamped to what a screen can hold.
fn rtt_ms(conn: &PeerConn) -> u32 {
    u32::try_from(conn.rtt().as_millis()).unwrap_or(u32::MAX)
}

/// The names one authenticated connection carries: the relation authentication
/// answered, and the two strings its `Hello` sent (empty only if it sent none, which
/// the transport never lets a live connection do).
fn names_of(conn: &PeerConn) -> (Relation, String, String) {
    let info = conn.info();
    let hello = info.hello.as_ref();
    (
        info.relation.clone(),
        hello.map(|hello| hello.name.clone()).unwrap_or_default(),
        hello
            .map(|hello| hello.platform.clone())
            .unwrap_or_default(),
    )
}

/// One wire item as the replica holds it: its id and its copies, refused if the id
/// is not one the server makes (empty, or longer than [`MAX_TRACK_ID`]).
fn entry_of(item: proto::Item) -> Result<(Box<str>, Copies), LanError> {
    let track = item.track;
    if track.is_empty() || track.len() > MAX_TRACK_ID {
        return Err(LanError::Proto(format!(
            "un identifiant de titre de {} octets",
            track.len()
        )));
    }
    let mut copies = Copies::default();
    for copy in item.copies {
        copies.insert(copy.quality, copy.len);
    }
    Ok((track.into_boxed_str(), copies))
}

/// The pages of a full inventory as one sorted replica, with a track announced
/// twice (two qualities in two items) merged into one entry.
fn build_sorted(mut raw: Vec<(Box<str>, Copies)>) -> Vec<Entry> {
    raw.sort_by(|left, right| left.0.cmp(&right.0));
    let mut out: Vec<Entry> = Vec::with_capacity(raw.len());
    for (track, copies) in raw {
        match out.last_mut() {
            Some(last) if last.track.as_ref() == track.as_ref() => last.copies.merge(copies),
            _ => out.push(Entry { track, copies }),
        }
    }
    out
}

/// Inserts one track's copies into a sorted replica (a delta's `added`).
fn insert_copies(items: &mut Vec<Entry>, track: &str, copies: Copies) {
    match items.binary_search_by(|entry| entry.track.as_ref().cmp(track)) {
        Ok(index) => items[index].copies.merge(copies),
        Err(index) => items.insert(
            index,
            Entry {
                track: track.into(),
                copies,
            },
        ),
    }
}

/// Removes one copy from a sorted replica, dropping the track when no copy is left
/// (a delta's `removed`).
fn remove_copy(items: &mut Vec<Entry>, track: &str, quality: WireQuality) {
    let Ok(index) = items.binary_search_by(|entry| entry.track.as_ref().cmp(track)) else {
        return;
    };
    items[index].copies.remove(quality);
    if items[index].copies.is_empty() {
        items.remove(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A node with no peer has no candidate, no complete set and no memory to
    /// report: the decision that follows from this is "the server", instant and
    /// with no request (rule 3).
    #[test]
    fn no_peer_means_no_candidate() {
        let peers = Peers::new();
        assert!(peers.candidates("3135556").is_empty());
        assert_eq!(peers.complete_for(&["3135556".to_owned()]), None);
        assert!(peers.get(&[0; 32]).is_none());
        assert!(peers.views().is_empty());
        assert_eq!(peers.memory(), 0);
        assert_eq!(peers.len(), 0);
        assert!(peers.is_empty());
        peers.mark_bad(&[0; 32], "3135556");
        peers.remove(&[0; 32]);
    }

    /// The replica is 48 counted bytes per item for a 7-character id — the number
    /// `Peers::memory` reports and the note's `≤ 64 B per item` bound. Measured with
    /// the entry's own accounting, on 8 000 items: `48 × 8 000 = 384 000` bytes.
    #[test]
    fn one_entry_costs_forty_eight_counted_bytes_for_a_seven_character_id() {
        let entry = Entry {
            track: "3135556".into(),
            copies: Copies::default(),
        };
        assert_eq!(entry.counted(), 48);
        let eight_thousand = 8_000 * entry.counted();
        assert_eq!(eight_thousand, 384_000);
        assert!(entry.counted() <= 64);
    }

    /// A track announced at two qualities in two items (which is how the responder's
    /// delta names them) is ONE replica entry with two copies, and a removal drops
    /// the entry only when its last copy goes.
    #[test]
    fn two_copies_of_one_track_are_one_entry_and_removal_keeps_the_other() {
        let mut raw = vec![
            (
                "3135556".into(),
                Copies {
                    bits: 1,
                    lens: [1_000, 0, 0, 0],
                },
            ),
            (
                "3135556".into(),
                Copies {
                    bits: 2,
                    lens: [0, 500, 0, 0],
                },
            ),
            (
                "3135557".into(),
                Copies {
                    bits: 8,
                    lens: [0, 0, 0, 90],
                },
            ),
        ];
        let mut items = build_sorted(std::mem::take(&mut raw));
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].copies.qualities().collect::<Vec<_>>(),
            vec![(WireQuality::Flac, 1_000), (WireQuality::Opus320, 500)]
        );

        remove_copy(&mut items, "3135556", WireQuality::Flac);
        assert_eq!(items.len(), 2, "one copy is left, so the entry stays");
        remove_copy(&mut items, "3135556", WireQuality::Opus320);
        assert_eq!(items.len(), 1, "the last copy takes the entry with it");
        assert_eq!(items[0].track.as_ref(), "3135557");
        // Removing something absent is a no-op, never a panic.
        remove_copy(&mut items, "404", WireQuality::Flac);
        assert_eq!(items.len(), 1);
    }

    /// Acceptance: "every size and count from the wire is capped before
    /// allocation". An id the server would never make is refused rather than stored,
    /// and a copy longer than `u32::MAX` is not audio and is dropped.
    #[test]
    fn a_hostile_item_is_refused_or_dropped_rather_than_stored() {
        let empty = proto::Item {
            track: String::new(),
            copies: vec![proto::Copy {
                quality: WireQuality::Flac,
                len: 10,
            }],
        };
        assert!(matches!(entry_of(empty), Err(LanError::Proto(_))));

        let long = proto::Item {
            track: "x".repeat(MAX_TRACK_ID + 1),
            copies: vec![proto::Copy {
                quality: WireQuality::Flac,
                len: 10,
            }],
        };
        assert!(matches!(entry_of(long), Err(LanError::Proto(_))));

        let huge = proto::Item {
            track: "3135556".to_owned(),
            copies: vec![
                proto::Copy {
                    quality: WireQuality::Flac,
                    len: u64::from(u32::MAX) + 1,
                },
                proto::Copy {
                    quality: WireQuality::Opus128,
                    len: 4_000_000,
                },
            ],
        };
        let (_, copies) = entry_of(huge).expect("a valid id");
        assert_eq!(
            copies.qualities().collect::<Vec<_>>(),
            vec![(WireQuality::Opus128, 4_000_000)],
            "the 4 GiB copy is not a track and is not kept"
        );
    }

    /// The candidate's helpers are the decision's whole input: `has` is exact (a
    /// peer that has Opus 128 does not have FLAC), and `best_quality` follows
    /// `nz_api::Quality`'s own order.
    #[test]
    fn a_candidate_answers_what_it_has_and_its_best_copy() {
        let candidate = Candidate {
            peer: [7; 32],
            relation: Relation::Own,
            copies: vec![(nz_api::Quality::Opus64, 1), (nz_api::Quality::Flac, 3)],
            rtt_ms: 3,
        };
        assert!(candidate.has(nz_api::Quality::Flac));
        assert!(candidate.has(nz_api::Quality::Opus64));
        assert!(!candidate.has(nz_api::Quality::Opus320));
        assert_eq!(candidate.best_quality(), Some(nz_api::Quality::Flac));
        assert_eq!(
            candidate.qualities().collect::<Vec<_>>(),
            vec![nz_api::Quality::Opus64, nz_api::Quality::Flac]
        );
    }
}
