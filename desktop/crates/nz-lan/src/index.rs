//! What this node has: the complete files of `nz-cache` (the person's downloads,
//! then complete play-cache files), per quality, with a SHA-256, plus the download
//! sets they belong to (docs/agents/lan-nodes.md, *Protocol messages*).
//!
//! **LN4 filled it.** The inventory is in memory and re-read from `nz-cache` on
//! [`Index::refresh`]; a change bumps [`Index::version`], and the peers that
//! already hold a replica hear it through [`Index::events`] and ask for a delta.
//! Nothing here is on a screen's path: the version is what the snapshot reads.
//!
//! # Where the files come from, and why no path is stored
//!
//! `nz-cache` names every complete file `<track_digest>.<quality>` — FNV-1a of
//! the track id and the `q=` spelling — under the offline directory (a download)
//! or the audio one (the play cache). The inventory therefore keeps
//! `(track, quality, length)` and RE-DERIVES the name, which is what makes a
//! track cost 48 B instead of 90 (a stored path is its own allocation). The
//! derivation is not guessed: `refresh` reads `file` from `nz-cache`'s own rows
//! and leaves out a copy whose name does not match, so a rename there is a logged
//! degradation rather than serving the wrong bytes — and
//! `tests/serve.rs::the_index_derives_the_names_nz_cache_writes` holds the two
//! together with real files created through `nz-cache`'s API.
//!
//! # Reading `nz-cache`'s file directly
//!
//! The index needs `(track, quality, file)` for 8 000 rows in ONE query, which is
//! the acceptance's "≤ 5 statements"; `nz-cache`'s public API answers per track
//! (`Downloads::local`, `PlayCache::lookup`) or without the file name
//! (`Downloads::published`, `PlayCache::entries`). So this module opens its own
//! READ-ONLY connection to the same `desktop.sqlite`, exactly as
//! `nz_cache::schema`'s WAL test does: WAL is what lets a reader answer while the
//! downloads worker writes (0.18 ms measured in that crate's own test). Nothing
//! here ever writes a `nz-cache` row.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, Params};
use tokio::sync::watch;
use tracing::debug;

use crate::proto;
use crate::scope::Scope;
use crate::store::{self, Store};

/// How many inventory items one `InventoryFull` page carries (the note's 4 000).
pub const PAGE_ITEMS: usize = 4_000;

/// How many download sets one `Sets` page carries.
///
/// The note caps a page at 64; this build sends at most 8, because a page of 64
/// sets each holding a 4 000-track playlist would be ~3 MB of JSON and a frame is
/// capped at 1 MiB (`codec::MAX_FRAME_BYTES`). Eight worst-case sets are 384 KB,
/// which every page is guaranteed to fit.
pub const PAGE_SETS: usize = 8;

/// How many changes the inventory remembers, for a peer's delta.
pub const MAX_CHANGES: usize = 64;

/// A change bigger than this clears the delta ring: a peer gets the full pages
/// instead of a delta nobody could fit in a frame (64 changes × 512 items is the
/// worst case the ring may hold, ~900 KB of JSON).
pub const MAX_CHANGE_ITEMS: usize = 512;

/// How many track ids a `Meta` request may carry (the note's cap).
pub const MAX_META_TRACKS: usize = 200;

/// How long a track id may be before a `Meta` request ignores it: a Deezer id or a
/// row UUID, never a payload.
const MAX_TRACK_ID: usize = 64;

/// The largest cover a peer is sent (the note's 2 MiB answer cap).
pub const MAX_COVER_BYTES: u64 = 2 * 1024 * 1024;

/// How many distinct cover URLs a restricted friend's scope may expose before the
/// build stops (a bounded answer, never a 20 000-entry set).
const MAX_VISIBLE_COVERS: usize = 4_096;

/// How many set bodies one `Meta` request parses (a bound on CPU the peer chose).
const MAX_META_SETS: usize = 32;

/// How much a `Sets` page may weigh, ESTIMATED from the sets' own counts before a
/// member is read, leaving room for the frame's tag under the 1 MiB cap.
///
/// The estimate assumes a 40-byte track id (a row UUID is 36): a page that passes
/// it is under 512 KB of real JSON, and the boundaries come from the `tracks`
/// count alone — so a page is a deterministic slice of the set list and no set is
/// ever skipped between two pages.
const SETS_PAGE_BYTES: usize = 512 * 1024;

/// What one member of a set costs on the wire, for the estimate above: its id
/// plus quotes, a comma and slack.
const SET_BYTES_PER_TRACK: usize = 43;

/// The qualities a peer can be served, in the order the bits of [`Copies`] use.
const WIRE_QUALITIES: [proto::WireQuality; 4] = [
    proto::WireQuality::Flac,
    proto::WireQuality::Opus320,
    proto::WireQuality::Opus128,
    proto::WireQuality::Opus64,
];

/// The counted cost of one inventory entry, WITHOUT its track id: the `Box<str>`
/// (16 B) and the [`Copies`] (20 B) padded to the 8-byte alignment the vector
/// holds them at (40 B). Measured on 8 000 tracks with a 7-character id: 48 B
/// each, keys included (`Index::memory`).
const ITEM_BYTES: usize = 40;

/// One complete local file a peer may fetch: where it is, at which quality, how
/// long, and its tag once hashing has caught up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    /// The path on THIS device. It never comes from the wire and it never leaves
    /// the device: a peer names a track id and a quality.
    pub path: PathBuf,
    pub quality: nz_api::Quality,
    pub len: u64,
    /// `None` until the hashing worker has hashed this exact `(len, mtime)`.
    pub tag: Option<proto::FileTag>,
}

/// Which complete files one track has: one bit per quality, one bit saying
/// whether the file is a download, and the length of each.
///
/// 20 bytes, and that is the point: the inventory is 8 000 of these plus their
/// ids, so a stored path (a `PathBuf` is 24 B plus its allocation) would double
/// what a peer's replica costs the person's memory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Copies {
    /// Bit `i` is set when `WIRE_QUALITIES[i]` has a complete file.
    bits: u8,
    /// Bit `i` is set when that file is a DOWNLOAD (`offline/`), clear when it is
    /// the play cache (`audio/`): the same `(track, quality)` can be in both, and
    /// a download wins (it is never evicted).
    downloads: u8,
    /// The length of each copy, in bytes. A track is never 4 GiB; a larger file is
    /// refused rather than truncated.
    lens: [u32; 4],
}

impl Copies {
    fn one(index: usize, len: u64, download: bool) -> Copies {
        let mut copies = Copies::default();
        copies.insert(index, len, download);
        copies
    }

    fn insert(&mut self, index: usize, len: u64, download: bool) {
        let Ok(len) = u32::try_from(len) else {
            return;
        };
        let bit = 1u8 << index;
        if self.bits & bit == 0 || download {
            self.lens[index] = len;
            self.downloads = if download {
                self.downloads | bit
            } else {
                self.downloads & !bit
            };
        }
        self.bits |= bit;
    }

    fn merge(&mut self, other: Copies) {
        for index in 0..WIRE_QUALITIES.len() {
            if other.bits & (1u8 << index) != 0 {
                self.insert(
                    index,
                    u64::from(other.lens[index]),
                    other.downloads & (1u8 << index) != 0,
                );
            }
        }
    }

    /// The length of the copy and whether it is a download.
    fn get(&self, index: usize) -> Option<(u64, bool)> {
        (self.bits & (1u8 << index) != 0).then(|| {
            (
                u64::from(self.lens[index]),
                self.downloads & (1u8 << index) != 0,
            )
        })
    }

    fn is_empty(&self) -> bool {
        self.bits == 0
    }

    fn qualities(&self) -> impl Iterator<Item = (proto::WireQuality, u64)> + '_ {
        WIRE_QUALITIES
            .iter()
            .copied()
            .enumerate()
            .filter_map(move |(index, quality)| self.get(index).map(|(len, _)| (quality, len)))
    }
}

/// One track of the inventory: its id and its copies, sorted by id so a page is
/// deterministic and a lookup is a binary search.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    track: Box<str>,
    copies: Copies,
}

impl Entry {
    fn to_item(&self) -> proto::Item {
        proto::Item {
            track: self.track.to_string(),
            copies: self
                .copies
                .qualities()
                .map(|(quality, len)| proto::Copy { quality, len })
                .collect(),
        }
    }

    fn counted(&self) -> usize {
        ITEM_BYTES + align8(self.track.len())
    }
}

/// One change of the inventory, kept so a peer that already has a replica gets a
/// delta: `from` is the version before it, `to` the version after.
#[derive(Clone, Debug)]
struct Change {
    from: u64,
    to: u64,
    added: Vec<Entry>,
    removed: Vec<proto::ItemKey>,
}

impl Change {
    fn counted(&self) -> usize {
        let added: usize = self
            .added
            .iter()
            .map(|entry| entry.counted() + align8(entry.track.len()))
            .sum();
        // An `ItemKey` is a `String` (24 B) plus a quality byte, and its own id.
        32 * self.removed.len() + added
    }
}

/// The cover URLs a restricted friend may fetch, with the scope they were built
/// for and the sets version that invalidates them.
#[derive(Clone, Debug)]
struct VisibleCovers {
    sets_version: u64,
    /// The `Arc` the scope was built with: the SAME friend shares one, so two of
    /// their connections reuse the build and a different friend rebuilds.
    scope: usize,
    urls: Arc<HashSet<Box<str>>>,
}

#[derive(Debug, Default)]
struct Data {
    /// Sorted by track id.
    items: Vec<Entry>,
    changes: VecDeque<Change>,
    version: u64,
    sets_version: u64,
    /// The fingerprint of the `download_sets` rows the current `sets_version` was
    /// built for: a set edit bumps the version, an unchanged read does not.
    sets_digest: u64,
    /// What [`Index::memory`] answers, kept up to date by a refresh so a screen
    /// never walks 8 000 entries to draw a number.
    bytes: usize,
    covers: Option<VisibleCovers>,
}

impl Data {
    fn binary_search(&self, track: &str) -> Result<usize, usize> {
        self.items
            .binary_search_by(|entry| entry.track.as_ref().cmp(track))
    }

    fn entry(&self, track: &str) -> Option<&Entry> {
        self.binary_search(track)
            .ok()
            .map(|index| &self.items[index])
    }
}

/// The local inventory, shared by the serving, hashing and requester tasks.
///
/// Cloning is cheap and every clone sees the same versions: one `Arc` inside.
#[derive(Clone)]
pub struct Index {
    inner: Arc<Inner>,
}

struct Inner {
    cache: nz_cache::Cache,
    owner: String,
    store: Store,
    /// The read-only connection to `nz-cache`'s file. `None` when it could not be
    /// opened: every read then answers "nothing", which is a node that shares
    /// nothing rather than a crash.
    read: Option<Mutex<Connection>>,
    data: RwLock<Data>,
    events: watch::Sender<u64>,
    hashed: watch::Sender<u64>,
    /// Statements THIS crate issued against the two databases, which is the
    /// acceptance's "≤ 5 statements to index 8 000 tracks".
    statements: AtomicU64,
}

impl fmt::Debug for Index {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let data = self.inner.data.read().unwrap_or_else(|e| e.into_inner());
        f.debug_struct("Index")
            .field("owner", &self.inner.owner)
            .field("tracks", &data.items.len())
            .field("version", &data.version)
            .field("sets_version", &data.sets_version)
            .field("bytes", &data.bytes)
            .finish()
    }
}

impl Index {
    /// Builds an index over `cache`'s files for `owner`'s account, persisting the
    /// hashes in `store`.
    ///
    /// `owner` is the account whose copies this node may serve: the index is one
    /// person's library, and leasing it to a remote-control session must not leak
    /// somebody else's files.
    pub fn new(cache: nz_cache::Cache, owner: String, store: Store) -> Index {
        let read = open_read(&cache).map(Mutex::new);
        if read.is_none() {
            debug!("nz-cache's file could not be read: the LAN inventory stays empty");
        }
        Index {
            inner: Arc::new(Inner {
                cache,
                owner,
                store,
                read,
                data: RwLock::new(Data::default()),
                events: watch::channel(0).0,
                hashed: watch::channel(0).0,
                statements: AtomicU64::new(0),
            }),
        }
    }

    /// The account whose files this node serves.
    pub fn owner(&self) -> &str {
        &self.inner.owner
    }

    /// Re-reads `nz-cache` and bumps the version when anything changed. Called on
    /// the hashing worker's schedule, never on a request.
    ///
    /// Four statements whatever the library's size (the acceptance's cap is 5):
    /// the downloads, the complete play-cache files, the hashes already paid for,
    /// and the download sets' summary.
    pub fn refresh(&self) {
        let started = std::time::Instant::now();
        let rows = self.complete_rows();
        // One statement: every hash this device already holds.
        self.inner.statements.fetch_add(1, Ordering::Relaxed);
        let tags = self.inner.store.tags();
        // One statement: the sets, for their version (their members are read on
        // the request that serves them, so a `Sets` page costs what it sends).
        self.inner.statements.fetch_add(1, Ordering::Relaxed);
        let sets = self.inner.cache.downloads().sets(&self.inner.owner);

        let mut items = self.build_items(rows);
        self.forget_stale_tags(&tags, &items);

        let digest = sets
            .as_ref()
            .map(|sets| sets_fingerprint(sets))
            .unwrap_or(0);
        let (changed, version, sets_changed, sets_version, bytes) = {
            let mut data = self.inner.data.write().unwrap_or_else(|e| e.into_inner());
            let (added, removed) = diff(&data.items, &items);
            let mut changed = false;
            let mut version = data.version;
            if !added.is_empty() || !removed.is_empty() {
                version += 1;
                changed = true;
                let change = Change {
                    from: data.version,
                    to: version,
                    added,
                    removed,
                };
                if change.added.len() + change.removed.len() > MAX_CHANGE_ITEMS {
                    // Too big to be a delta: a peer at an older version gets the
                    // full pages instead.
                    data.changes.clear();
                } else {
                    data.changes.push_back(change);
                }
                while data.changes.len() > MAX_CHANGES {
                    data.changes.pop_front();
                }
            }
            // A set edit (a title, a member) is a new version even when no file
            // moved, which is what makes a peer re-ask its `Sets` pages.
            let mut sets_changed = false;
            if digest != data.sets_digest {
                data.sets_digest = digest;
                data.sets_version += 1;
                sets_changed = true;
            }
            data.version = version;
            data.items = std::mem::take(&mut items);
            data.covers = None;
            data.bytes = counted(&data);
            (
                changed,
                version,
                sets_changed,
                data.sets_version,
                data.bytes,
            )
        };
        if changed {
            let _ = self.inner.events.send(version);
        }
        if changed || sets_changed {
            debug!(
                tracks = self.tracks(),
                version,
                sets_version,
                bytes,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "inventory refreshed"
            );
        }
    }

    /// The inventory's version: it grows on every change, and a delta is only
    /// ever asked for a version the responder still remembers.
    pub fn version(&self) -> u64 {
        self.inner
            .data
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .version
    }

    /// The download sets' version, which moves independently (a playlist can gain
    /// a track that is already on disk).
    pub fn sets_version(&self) -> u64 {
        self.inner
            .data
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .sets_version
    }

    /// The version counter, so a peer that hears "your inventory changed" and a
    /// screen that shows a count both follow the same signal.
    pub fn events(&self) -> watch::Receiver<u64> {
        self.inner.events.subscribe()
    }

    /// How many files the hashing worker has hashed since this index was built: a
    /// counted signal, no payload.
    ///
    /// The worker publishes on it, and a test waits for it instead of sleeping
    /// (the rules forbid a clock-based test).
    pub fn hashed(&self) -> watch::Receiver<u64> {
        self.inner.hashed.subscribe()
    }

    /// Announces that one file was hashed (the hashing worker).
    pub(crate) fn note_hashed(&self) {
        let next = self.inner.hashed.borrow().wrapping_add(1);
        let _ = self.inner.hashed.send(next);
    }

    /// How many SQL statements this index issued since it was built: the
    /// acceptance's "indexing 8 000 tracks costs ≤ 5 statements" is this number.
    pub fn statements(&self) -> u64 {
        self.inner.statements.load(Ordering::Relaxed)
    }

    /// How many tracks the inventory holds (a count for the log and the screens).
    pub fn tracks(&self) -> usize {
        self.inner
            .data
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .items
            .len()
    }

    /// One page of the inventory, filtered by `scope` — for a friend, only the
    /// tracks they were shared.
    pub fn inventory(&self, scope: &Scope, since: Option<u64>, page: u32) -> proto::Response {
        let data = self.inner.data.read().unwrap_or_else(|e| e.into_inner());
        if let Some(since) = since
            && let Some(delta) = delta(&data, scope, since)
        {
            return delta;
        }
        let start = (page as usize).saturating_mul(PAGE_ITEMS);
        let mut items = Vec::new();
        let mut skipped = 0usize;
        let mut last = true;
        for entry in &data.items {
            if !scope.allows(&entry.track) {
                continue;
            }
            if skipped < start {
                skipped += 1;
                continue;
            }
            if items.len() == PAGE_ITEMS {
                last = false;
                break;
            }
            items.push(entry.to_item());
        }
        proto::Response::InventoryFull {
            version: data.version,
            page,
            last,
            items,
        }
    }

    /// One page of the download sets whose members are in `scope`.
    ///
    /// The sets are read from `nz-cache` on the request (a control request, not a
    /// click path): keeping their members in memory would cost ~32 B per track of
    /// the 48 B an inventory entry already has, and a set's members are exactly a
    /// peer's `Sets` page, asked once.
    pub fn sets(&self, scope: &Scope, page: u32) -> proto::Response {
        let version = self.sets_version();
        let Some(all) = self.inner.cache.downloads().sets(&self.inner.owner).ok() else {
            return proto::Response::Sets {
                version,
                page,
                last: true,
                sets: Vec::new(),
            };
        };
        let (start, end, last) = page_bounds(&all, page);
        let mut sets = Vec::new();
        for summary in &all[start..end] {
            let members = self.set_members(&summary.kind, &summary.ident);
            let mut visible = true;
            let mut local = 0u32;
            let mut tracks = Vec::with_capacity(members.len());
            for track in &members {
                // A set the friend cannot see whole is not advertised at all: half
                // a playlist would tell them what they were not shared (T7).
                if !scope.allows(track) {
                    visible = false;
                    break;
                }
                if self.has_track(track) {
                    local += 1;
                }
                tracks.push(track.to_string());
            }
            if !visible {
                continue;
            }
            sets.push(proto::SetInfo {
                kind: summary.kind.clone(),
                ident: summary.ident.clone(),
                title: summary.title.clone(),
                // A friend who sees only some of my tracks must not be handed the
                // playlist's own art for a list they cannot see whole; my own
                // devices, and a friend I share everything with, get it.
                cover: (scope.is_own() || scope.is_all())
                    .then(|| self.set_cover(&summary.kind, &summary.ident))
                    .flatten(),
                local,
                complete: !tracks.is_empty() && local as usize == tracks.len(),
                tracks,
            });
        }
        proto::Response::Sets {
            version,
            page,
            last,
            sets,
        }
    }

    /// Metadata for at most [`MAX_META_TRACKS`] track ids, only the ones in
    /// `scope`.
    ///
    /// Where each field comes from: `title` and `artist` from `nz-offline`'s
    /// `downloads.labels` (what the downloads screen already prints offline), and
    /// everything else (album, its id, duration, the album's cover, `explicit`)
    /// from the `/playlist/<id>` and `/album/<id>` bodies the set pages left in
    /// `nz-cache`'s API cache. A track no cached body knows answers its labels
    /// only — an invented album or duration would be worse than a blank one.
    pub fn meta(&self, scope: &Scope, tracks: &[String]) -> proto::Response {
        let mut wanted: Vec<&str> = Vec::new();
        for track in tracks.iter().take(MAX_META_TRACKS * 4) {
            if wanted.len() == MAX_META_TRACKS {
                break;
            }
            if track.is_empty() || track.len() > MAX_TRACK_ID || !scope.allows(track) {
                continue;
            }
            if !wanted.contains(&track.as_str()) {
                wanted.push(track.as_str());
            }
        }
        if wanted.is_empty() {
            return proto::Response::Meta { rows: Vec::new() };
        }

        let mut known: HashMap<String, proto::TrackMeta> = HashMap::new();
        for (kind, ident) in self.sets_holding(&wanted) {
            if known.len() >= wanted.len() {
                break;
            }
            let Some(body) = self
                .inner
                .cache
                .get(&self.inner.owner, &format!("/{kind}/{ident}"))
            else {
                continue;
            };
            for track in parse_tracks(&body) {
                known.entry(track.id.clone()).or_insert(track);
            }
        }

        let labels = self
            .inner
            .cache
            .kv()
            .get::<HashMap<String, nz_offline::Labels>>(nz_offline::LABELS_KEY)
            .unwrap_or_default();
        let mut rows = Vec::with_capacity(wanted.len());
        for track in wanted {
            if let Some(row) = known.get(track) {
                rows.push(row.clone());
                continue;
            }
            let Some(label) = labels.get(track) else {
                continue;
            };
            rows.push(proto::TrackMeta {
                id: track.to_string(),
                title: label.title.clone(),
                artists: label.artist.clone(),
                album: String::new(),
                album_id: None,
                duration_s: 0,
                cover: None,
                explicit: false,
            });
        }
        proto::Response::Meta { rows }
    }

    /// The complete local file for `(track, quality)`, if this node has one.
    ///
    /// The tag comes from the store, not from memory: 64 hex characters per file
    /// would be 512 KB for a small library, and a `Fetch` pays one indexed read
    /// for it.
    pub fn local(&self, track: &str, quality: nz_api::Quality) -> Option<LocalFile> {
        let index = wire_index(proto::WireQuality::from(quality))?;
        let (len, download, wire) = {
            let data = self.inner.data.read().unwrap_or_else(|e| e.into_inner());
            let entry = data.entry(track)?;
            let (len, download) = entry.copies.get(index)?;
            (len, download, WIRE_QUALITIES[index])
        };
        let path = self.file_path(track, wire, download);
        let tag = self.inner.store.tag(&path);
        Some(LocalFile {
            path,
            quality,
            len,
            tag,
        })
    }

    /// The best complete copy this node has of `track`, whatever was asked: what
    /// the responder sends when the asked-for quality is gone.
    ///
    /// Higher is better, and `WireQuality`'s index order is that order (FLAC,
    /// Opus 320, 128, 64). A requester learns what it actually got in
    /// `FetchOk.quality`, and a peer file is always complete, so sending the copy
    /// that exists beats sending the person to the slower server for the copy the
    /// requester believed was here.
    pub fn best_local(&self, track: &str) -> Option<LocalFile> {
        let data = self.inner.data.read().unwrap_or_else(|e| e.into_inner());
        let entry = data.entry(track)?;
        for (index, wire) in WIRE_QUALITIES.iter().enumerate() {
            if let Some((len, download)) = entry.copies.get(index) {
                let path = self.file_path(track, *wire, download);
                let tag = self.inner.store.tag(&path);
                return Some(LocalFile {
                    path,
                    quality: nz_api::Quality::from(*wire),
                    len,
                    tag,
                });
            }
        }
        None
    }

    /// The cover file for a URL a metadata row named, when a track in `scope`
    /// references it.
    ///
    /// A download's cover is PINNED in `nz-cache` (`put_pinned`), so the LRU never
    /// takes the art of an album prepared for a plane.
    pub fn cover(&self, scope: &Scope, url: &str) -> Option<PathBuf> {
        if !is_cover_url(url) {
            return None;
        }
        if !self.cover_allowed(scope, url) {
            return None;
        }
        let path = self.inner.cache.covers().file(url)?;
        let len = std::fs::metadata(&path).ok()?.len();
        (len <= MAX_COVER_BYTES).then_some(path)
    }

    /// Every complete local file, as the path this node serves it from: what the
    /// hashing worker walks.
    pub fn local_paths(&self) -> Vec<PathBuf> {
        let data = self.inner.data.read().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::with_capacity(data.items.len());
        for entry in &data.items {
            for (index, quality) in WIRE_QUALITIES.iter().enumerate() {
                if let Some((_, download)) = entry.copies.get(index) {
                    out.push(self.file_path(&entry.track, *quality, download));
                }
            }
        }
        out
    }

    /// The bytes this index costs, counted (rule: never the RSS of a shared
    /// process). LN4's target is ≤ 64 B per item, so 8 000 tracks ≈ 384 KB.
    pub fn memory(&self) -> usize {
        self.inner
            .data
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .bytes
    }

    /// Whether `scope` may fetch `url`: only the tracks it may see reference it.
    fn cover_allowed(&self, scope: &Scope, url: &str) -> bool {
        match scope {
            Scope::Own => true,
            Scope::Friend { all: true, .. } => true,
            Scope::Friend { all: false, tracks } => self.visible_covers(tracks).contains(url),
        }
    }

    /// The cover URLs a restricted friend may fetch: the ones the sets whose
    /// members are all in their scope reference, built once per (scope, sets
    /// version).
    ///
    /// It is a bounded build (the sets this node downloaded, not the library): the
    /// alternative — walking the friend's 20 000 ids on every cover request —
    /// would let a peer spend this device's CPU by asking in a loop.
    fn visible_covers(&self, tracks: &Arc<HashSet<String>>) -> Arc<HashSet<Box<str>>> {
        let key = Arc::as_ptr(tracks) as usize;
        let sets_version = self.sets_version();
        {
            let data = self.inner.data.read().unwrap_or_else(|e| e.into_inner());
            if let Some(covers) = &data.covers
                && covers.scope == key
                && covers.sets_version == sets_version
            {
                return Arc::clone(&covers.urls);
            }
        }

        let mut urls: HashSet<Box<str>> = HashSet::new();
        for (kind, ident, _title) in self.set_rows() {
            let members = self.set_members(&kind, &ident);
            if members.is_empty() || !members.iter().all(|track| tracks.contains(track)) {
                continue;
            }
            let Some(body) = self
                .inner
                .cache
                .get(&self.inner.owner, &format!("/{kind}/{ident}"))
            else {
                continue;
            };
            if let Some(cover) = parse_set_cover(&body)
                && is_cover_url(&cover)
            {
                urls.insert(cover.into_boxed_str());
            }
            for track in parse_tracks(&body) {
                if let Some(cover) = track.cover
                    && is_cover_url(&cover)
                {
                    urls.insert(cover.into_boxed_str());
                }
            }
            if urls.len() >= MAX_VISIBLE_COVERS {
                debug!("cover scope reached its cap: the rest is not served");
                break;
            }
        }

        let urls = Arc::new(urls);
        {
            let mut data = self.inner.data.write().unwrap_or_else(|e| e.into_inner());
            data.covers = Some(VisibleCovers {
                sets_version,
                scope: key,
                urls: Arc::clone(&urls),
            });
            data.bytes = counted(&data);
        }
        urls
    }

    /// The path `nz-cache` keeps a `(track, quality)` at.
    fn file_path(&self, track: &str, quality: proto::WireQuality, download: bool) -> PathBuf {
        let dir = if download {
            self.inner.cache.offline_dir()
        } else {
            self.inner.cache.audio_dir()
        };
        dir.join(file_name(track, quality))
    }

    fn has_track(&self, track: &str) -> bool {
        let data = self.inner.data.read().unwrap_or_else(|e| e.into_inner());
        data.entry(track).is_some()
    }

    /// The complete files `nz-cache` knows about, in TWO statements.
    fn complete_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (track, quality, file) in self.rows(
            "SELECT track, quality, file FROM downloads \
             WHERE owner = ?1 AND state = 'done' AND file IS NOT NULL",
            rusqlite::params![self.inner.owner],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        ) {
            if let Some((wire, index)) = wire_of(&quality) {
                rows.push(Row {
                    track,
                    wire,
                    index,
                    file,
                    download: true,
                });
            }
        }
        for (track, quality, file) in self.rows(
            "SELECT track, quality, file FROM play_cache WHERE complete = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        ) {
            if let Some((wire, index)) = wire_of(&quality) {
                rows.push(Row {
                    track,
                    wire,
                    index,
                    file,
                    download: false,
                });
            }
        }
        rows
    }

    /// The sorted, merged inventory, with a copy the disk cannot confirm left out.
    fn build_items(&self, rows: Vec<Row>) -> Vec<Entry> {
        let mut entries = Vec::with_capacity(rows.len());
        let mut renamed = 0usize;
        let mut vanished = 0usize;
        for row in rows {
            let expected = file_name(&row.track, row.wire);
            if expected != row.file {
                // `nz-cache`'s naming moved under us: the file this node would
                // serve is not the row's, so the copy is not advertised. One log
                // line for the whole refresh, never one per track.
                renamed += 1;
                continue;
            }
            let dir = if row.download {
                self.inner.cache.offline_dir()
            } else {
                self.inner.cache.audio_dir()
            };
            let path = dir.join(&expected);
            let Ok(meta) = std::fs::metadata(&path) else {
                vanished += 1;
                continue;
            };
            if !meta.is_file() {
                vanished += 1;
                continue;
            }
            entries.push(Entry {
                track: row.track.into_boxed_str(),
                copies: Copies::one(row.index, meta.len(), row.download),
            });
        }
        if renamed > 0 || vanished > 0 {
            debug!(
                renamed,
                vanished, "inventory: copies this device cannot serve were left out"
            );
        }

        // Sorted, then merged: a track can be a download AND a play-cache entry,
        // and can have several qualities.
        entries.sort_unstable_by(|left, right| left.track.cmp(&right.track));
        let mut items: Vec<Entry> = Vec::with_capacity(entries.len());
        for entry in entries {
            match items.last_mut() {
                Some(last) if last.track == entry.track => last.copies.merge(entry.copies),
                _ => items.push(entry),
            }
        }
        items.retain(|entry| !entry.copies.is_empty());
        items.shrink_to_fit();
        items
    }

    /// Drops the hash of a file nothing lists any more (a download removed, a
    /// play-cache entry evicted), so a re-download is hashed instead of being
    /// served with the old file's digest. One statement per vanished path, and
    /// none in the steady state.
    fn forget_stale_tags(&self, tags: &HashMap<String, store::StoredTag>, items: &[Entry]) {
        if tags.is_empty() {
            return;
        }
        let mut live: HashSet<PathBuf> = HashSet::with_capacity(items.len());
        for entry in items {
            for (index, quality) in WIRE_QUALITIES.iter().enumerate() {
                if let Some((_, download)) = entry.copies.get(index) {
                    // The same `(track, quality)` can be in both directories; the
                    // index serves one of them and the other is a real file too.
                    for download in [download, !download] {
                        let path = self.file_path(&entry.track, *quality, download);
                        if path.is_file() {
                            live.insert(path);
                        }
                    }
                }
            }
        }
        let mut forgotten = 0usize;
        for path in tags.keys() {
            if !live.contains(Path::new(path))
                && self
                    .inner
                    .store
                    .forget_tag(Path::new(path))
                    .unwrap_or(false)
            {
                forgotten += 1;
            }
        }
        if forgotten > 0 {
            debug!(forgotten, "inventory: stale hashes forgotten");
        }
    }

    /// The download sets of this account, newest first: `(kind, ident, title)`.
    fn set_rows(&self) -> Vec<(String, String, String)> {
        self.rows(
            "SELECT kind, ident, title FROM download_sets WHERE owner = ?1 \
             ORDER BY added_at DESC, ident ASC",
            rusqlite::params![self.inner.owner],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
    }

    /// One set's member ids.
    fn set_members(&self, kind: &str, ident: &str) -> Vec<String> {
        self.rows(
            "SELECT track FROM download_members \
             WHERE owner = ?1 AND kind = ?2 AND ident = ?3 ORDER BY track ASC",
            rusqlite::params![self.inner.owner, kind, ident],
            |row| row.get::<_, String>(0),
        )
    }

    /// Which sets hold any of `tracks`: one statement, whatever the count.
    fn sets_holding(&self, tracks: &[&str]) -> Vec<(String, String)> {
        if tracks.is_empty() {
            return Vec::new();
        }
        let placeholders = vec!["?"; tracks.len()].join(", ");
        let sql = format!(
            "SELECT DISTINCT kind, ident FROM download_members \
             WHERE owner = ? AND track IN ({placeholders})"
        );
        let owner = self.inner.owner.as_str();
        let mut values: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(tracks.len() + 1);
        values.push(&owner);
        for track in tracks {
            values.push(track);
        }
        let mut rows = self.rows_dyn(&sql, values, |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        });
        rows.truncate(MAX_META_SETS);
        rows
    }

    /// The cover URL a set's own page carries (`/api/playlist/<id>`,
    /// `/api/album/<id>`), when its body is in the API cache.
    fn set_cover(&self, kind: &str, ident: &str) -> Option<String> {
        let body = self
            .inner
            .cache
            .get(&self.inner.owner, &format!("/{kind}/{ident}"))?;
        parse_set_cover(&body)
    }

    fn rows<T>(
        &self,
        sql: &str,
        params: impl Params,
        f: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Vec<T> {
        let Some(read) = self.inner.read.as_ref() else {
            return Vec::new();
        };
        self.inner.statements.fetch_add(1, Ordering::Relaxed);
        let conn = read.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = match conn.prepare(sql) {
            Ok(stmt) => stmt,
            Err(error) => {
                debug!(%error, "inventory read failed");
                return Vec::new();
            }
        };
        match stmt.query_map(params, f) {
            Ok(rows) => rows.filter_map(|row| row.ok()).collect(),
            Err(error) => {
                debug!(%error, "inventory read failed");
                Vec::new()
            }
        }
    }

    fn rows_dyn<T>(
        &self,
        sql: &str,
        params: Vec<&dyn rusqlite::ToSql>,
        f: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Vec<T> {
        let Some(read) = self.inner.read.as_ref() else {
            return Vec::new();
        };
        self.inner.statements.fetch_add(1, Ordering::Relaxed);
        let conn = read.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = match conn.prepare(sql) {
            Ok(stmt) => stmt,
            Err(error) => {
                debug!(%error, "inventory read failed");
                return Vec::new();
            }
        };
        match stmt.query_map(params.as_slice(), f) {
            Ok(rows) => rows.filter_map(|row| row.ok()).collect(),
            Err(error) => {
                debug!(%error, "inventory read failed");
                Vec::new()
            }
        }
    }
}

/// One complete file `nz-cache` knows about, before its length is read from disk.
struct Row {
    track: String,
    wire: proto::WireQuality,
    index: usize,
    /// The file name `nz-cache` stored, which the derived one must match.
    file: String,
    download: bool,
}

/// The delta from `since` to the current version, when every change since is in
/// the ring and the answer fits one page.
fn delta(data: &Data, scope: &Scope, since: u64) -> Option<proto::Response> {
    let current = data.version;
    if since > current {
        // A peer that holds a version this node never had (a reinstall, another
        // account): the full pages are the only honest answer.
        return None;
    }
    if since == current {
        return Some(proto::Response::InventoryDelta {
            from: since,
            to: current,
            added: Vec::new(),
            removed: Vec::new(),
        });
    }
    let first = data.changes.iter().find(|change| change.to > since)?;
    if first.from != since {
        // A gap: the ring was cleared by a change too big for a delta.
        return None;
    }
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for change in data.changes.iter().filter(|change| change.to > since) {
        added.extend(
            change
                .added
                .iter()
                .filter(|entry| scope.allows(&entry.track))
                .map(Entry::to_item),
        );
        removed.extend(
            change
                .removed
                .iter()
                .filter(|key| scope.allows(&key.track))
                .cloned(),
        );
        if added.len() + removed.len() > PAGE_ITEMS {
            // A page of full inventory is cheaper than a delta that would not fit
            // a frame anyway.
            return None;
        }
    }
    Some(proto::Response::InventoryDelta {
        from: since,
        to: current,
        added,
        removed,
    })
}

/// What changed between two sorted inventories: the copies that appeared and the
/// ones that went.
fn diff(old: &[Entry], new: &[Entry]) -> (Vec<Entry>, Vec<proto::ItemKey>) {
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let (mut left, mut right) = (0usize, 0usize);
    while left < old.len() || right < new.len() {
        match (old.get(left), new.get(right)) {
            (Some(a), Some(b)) if a.track == b.track => {
                for (index, quality) in WIRE_QUALITIES.iter().enumerate() {
                    let was = a.copies.get(index);
                    let is = b.copies.get(index);
                    if was.is_none() && is.is_some() {
                        let len = is.map(|(len, _)| len).unwrap_or(0);
                        added.push(Entry {
                            track: b.track.clone(),
                            copies: Copies::one(index, len, false),
                        });
                    } else if was.is_some() && is.is_none() {
                        removed.push(proto::ItemKey {
                            track: a.track.to_string(),
                            quality: *quality,
                        });
                    }
                }
                left += 1;
                right += 1;
            }
            (Some(a), Some(b)) if a.track < b.track => {
                removed.extend(a.copies.qualities().map(|(quality, _)| proto::ItemKey {
                    track: a.track.to_string(),
                    quality,
                }));
                left += 1;
            }
            (Some(a), None) => {
                removed.extend(a.copies.qualities().map(|(quality, _)| proto::ItemKey {
                    track: a.track.to_string(),
                    quality,
                }));
                left += 1;
            }
            (_, Some(b)) => {
                added.push(b.clone());
                right += 1;
            }
            (None, None) => break,
        }
    }
    (added, removed)
}

/// What one inventory costs, counted: the entries, the delta ring and the cover
/// scope built for a friend.
fn counted(data: &Data) -> usize {
    let items: usize = data.items.iter().map(Entry::counted).sum();
    let changes: usize = data.changes.iter().map(Change::counted).sum();
    let covers = data
        .covers
        .as_ref()
        .map(|covers| counted_covers(&covers.urls))
        .unwrap_or(0);
    items + changes + covers
}

fn counted_covers(urls: &Arc<HashSet<Box<str>>>) -> usize {
    urls.iter().map(|url| 24 + align8(url.len())).sum()
}

/// `nz-cache`'s file name for a `(track, quality)`: FNV-1a 64 of the id, a zero
/// byte and the `q=` spelling, then the spelling as the extension — the
/// derivation [`Index::refresh`] checks against the rows it read.
///
/// Held together with the crate that writes those names by
/// `tests/serve.rs::the_index_derives_the_names_nz_cache_writes`, which creates
/// real files through `nz-cache`'s own API and compares.
pub fn file_name(track: &str, quality: proto::WireQuality) -> String {
    let quality = quality_name(quality);
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in track
        .as_bytes()
        .iter()
        .copied()
        .chain(std::iter::once(0))
        .chain(quality.as_bytes().iter().copied())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}.{quality}")
}

/// The `q=` spelling of a wire quality, which is also the file extension
/// `nz-cache` writes (`FLAC`, `OPUS_320`, `OPUS_128`, `OPUS_64`).
fn quality_name(quality: proto::WireQuality) -> &'static str {
    match quality {
        proto::WireQuality::Flac => "FLAC",
        proto::WireQuality::Opus320 => "OPUS_320",
        proto::WireQuality::Opus128 => "OPUS_128",
        proto::WireQuality::Opus64 => "OPUS_64",
    }
}

/// The wire rung a `nz-cache` quality spelling names, `None` for the two rungs a
/// peer never serves (`OPUS_256`, `OPUS_192`: `WireQuality` has no rung for them,
/// and a file named for a folded quality is not the file this node has).
fn wire_of(raw: &str) -> Option<(proto::WireQuality, usize)> {
    let wire = match nz_api::Quality::parse(Some(raw))? {
        nz_api::Quality::Flac => proto::WireQuality::Flac,
        nz_api::Quality::Opus320 => proto::WireQuality::Opus320,
        nz_api::Quality::Opus128 => proto::WireQuality::Opus128,
        nz_api::Quality::Opus64 => proto::WireQuality::Opus64,
        nz_api::Quality::Opus256 | nz_api::Quality::Opus192 => return None,
    };
    wire_index(wire).map(|index| (wire, index))
}

fn wire_index(quality: proto::WireQuality) -> Option<usize> {
    WIRE_QUALITIES.iter().position(|wire| *wire == quality)
}

/// Whether a cover URL is one this node's own route serves: `/api/cover/<one
/// segment>`, no scheme, no traversal, bounded.
fn is_cover_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("/api/cover/") else {
        return false;
    };
    !rest.is_empty()
        && rest.len() <= 64
        && !rest.contains('/')
        && !rest.contains('.')
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The tracks a cached `/playlist/<id>` or `/album/<id>` body carries, in the
/// screen shape a peer gets.
fn parse_tracks(body: &str) -> Vec<proto::TrackMeta> {
    if let Ok(page) = serde_json::from_str::<nz_model::playlist::PlaylistPage>(body) {
        return page.tracks.iter().map(track_meta).collect();
    }
    if let Ok(page) = serde_json::from_str::<nz_model::album::AlbumPage>(body) {
        return page.tracks.iter().map(track_meta).collect();
    }
    Vec::new()
}

/// The cover URL a cached set page carries: the playlist's or the album's own.
fn parse_set_cover(body: &str) -> Option<String> {
    if let Ok(page) = serde_json::from_str::<nz_model::playlist::PlaylistPage>(body) {
        return page.playlist.cover;
    }
    if let Ok(page) = serde_json::from_str::<nz_model::album::AlbumPage>(body) {
        return page.album.cover;
    }
    None
}

/// One track, in the fields a list draws: no likes, no play counts, no user ids.
fn track_meta(track: &nz_model::track::Track) -> proto::TrackMeta {
    let album_id = track.album.deezer_id.as_str();
    proto::TrackMeta {
        id: track.deezer_id.as_str().to_owned(),
        title: track.title.clone(),
        artists: track
            .display_artist
            .clone()
            .unwrap_or_else(|| track.artist.name.clone()),
        album: track.album.title.clone(),
        album_id: (!album_id.is_empty()).then(|| album_id.to_owned()),
        duration_s: track.duration,
        cover: track.album.cover.clone(),
        explicit: track.explicit,
    }
}

/// A set's fingerprint: what a `Sets` answer would show, so the version moves
/// exactly when the answer would.
fn sets_fingerprint(sets: &[nz_cache::DownloadSet]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for set in sets {
        eat(set.kind.as_bytes());
        eat(b"\0");
        eat(set.ident.as_bytes());
        eat(b"\0");
        eat(set.title.as_bytes());
        eat(&(set.tracks as u64).to_le_bytes());
        eat(&(set.done as u64).to_le_bytes());
        eat(b"\x01");
    }
    hash
}

/// Which slice of the set list one `Sets` page carries: bounded by
/// [`PAGE_SETS`] and by [`SETS_PAGE_BYTES`], computed from the sets' own track
/// counts, and the same for every caller of the same page (so two pages never
/// overlap and none is skipped).
fn page_bounds(sets: &[nz_cache::DownloadSet], page: u32) -> (usize, usize, bool) {
    if page == u32::MAX {
        // A page a hostile peer invented: the empty tail, with no walk at all.
        return (sets.len(), sets.len(), true);
    }
    let mut wanted = page as usize;
    let mut start = 0usize;
    let mut count = 0usize;
    let mut bytes = 0usize;
    for (index, set) in sets.iter().enumerate() {
        let size = 64
            + set.kind.len()
            + set.ident.len()
            + set.title.len()
            + set.tracks.saturating_mul(SET_BYTES_PER_TRACK);
        if count > 0 && (count == PAGE_SETS || bytes + size > SETS_PAGE_BYTES) {
            if wanted == 0 {
                // The page ends here because it is full: the sets after it are the
                // NEXT page's, never dropped.
                return (start, index, false);
            }
            wanted -= 1;
            start = index;
            count = 0;
            bytes = 0;
        }
        count += 1;
        bytes += size;
    }
    // Below PAGE_SETS and under the byte bound: this page is the last of the
    // list — and so is a page past the end, which is empty and says "nothing
    // follows" rather than inviting another request for ever.
    let end = sets.len();
    if wanted > 0 {
        (end, end, true)
    } else {
        (start, end, true)
    }
}

/// Rounds a counted byte length up to the 8-byte alignment an allocation gives it,
/// so a count never looks smaller than the bytes it names.
fn align8(bytes: usize) -> usize {
    (bytes + 7) & !7
}

/// Opens the read-only connection to `nz-cache`'s file.
fn open_read(cache: &nz_cache::Cache) -> Option<Connection> {
    let path = cache.db_path();
    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    conn.busy_timeout(Duration::from_secs(5)).ok()?;
    Some(conn)
}
