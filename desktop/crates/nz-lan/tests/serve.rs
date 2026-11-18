//! LN4's acceptance, measured: what the index lists, what a friend sees, what a
//! refusal is worth, what a `Fetch` serves, and what serving costs the person.
//!
//! Every number the note asks for is asserted here with the command that produced
//! it written beside the assertion. Nothing in this file sleeps to let a state
//! settle: a test waits on a watch (`Index::hashed`, `Hashing::state`), on a
//! future's first poll (the `Bulk` gate), or on a blocking join.
//!
//! The fixture walks the same files `nz-cache` writes — through its PUBLIC API for
//! the naming test, and with a direct `INSERT` for the 8 000-track measure, because
//! creating 8 000 downloads through the API costs four statements and a rename each
//! and this is a bench, not a download.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use nz_lan::index::{Index, LocalFile, MAX_COVER_BYTES, file_name};
use nz_lan::proto::{self, FileTag, Request, Response, WireQuality};
use nz_lan::scope::Scope;
use nz_lan::serve::{BUSY_RETRY_MS, ServeLimits, Server};
use nz_lan::store::Store;
use nz_lan::types::Urgency;

const OWNER: &str = "1";

// --- the fixture ------------------------------------------------------------

/// A throwaway data + cache directory under `target/`, never `/tmp` (tmpfs is
/// RAM and this test writes 8 MB of fake audio).
struct Fixture {
    root: PathBuf,
    cache: nz_cache::Cache,
    store: Store,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("ln4-{tag}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("fixture dir");
        let cache = nz_cache::Cache::open_with_cache_dir(root.join("data"), root.join("cache"))
            .expect("cache");
        let store = Store::open(&root.join("lan")).expect("lan store");
        Fixture { root, cache, store }
    }

    fn index(&self) -> Index {
        Index::new(self.cache.clone(), OWNER.to_owned(), self.store.clone())
    }

    /// One download through `nz-cache`'s own API: the file is where and how the
    /// app would put it, and `nz-cache` names it.
    fn download(&self, track: &str, quality: nz_model::Quality, bytes: &[u8]) -> PathBuf {
        let id = nz_model::TrackId::new(track);
        let downloads = self.cache.downloads();
        downloads
            .add_set(
                OWNER,
                "album",
                "900",
                "Un album",
                &[nz_cache::DownloadTrack {
                    track: id.clone(),
                    quality,
                }],
            )
            .expect("add set");
        let partial = downloads.reserve(OWNER, &id, quality).expect("reserve");
        std::fs::write(&partial, bytes).expect("bytes");
        downloads
            .complete(OWNER, &id, bytes.len() as u64)
            .expect("complete")
    }

    /// One complete play-cache entry (evictable, never a download).
    fn play(&self, track: &str, quality: nz_model::Quality, bytes: &[u8]) -> PathBuf {
        let id = nz_model::TrackId::new(track);
        let play = self.cache.play_cache();
        let partial = play.reserve(&id, quality).expect("reserve");
        std::fs::write(&partial, bytes).expect("bytes");
        play.complete(&id, quality, bytes.len() as u64)
            .expect("complete")
    }

    /// `count` downloads of `len` bytes each, inserted the way `nz-cache` would
    /// have stored them: one transaction, one file each.
    fn many_downloads(&self, first: usize, count: usize, len: usize) {
        let offline = self.cache.offline_dir();
        let mut rows = Vec::with_capacity(count);
        for n in first..(first + count) {
            let track = format!("3{n:07}");
            let name = file_name(&track, WireQuality::Flac);
            std::fs::write(offline.join(&name), vec![0u8; len]).expect("fake audio");
            rows.push((track, name, len as i64));
        }
        let conn = rusqlite::Connection::open(self.cache.db_path()).expect("second handle");
        let tx = conn.unchecked_transaction().expect("tx");
        {
            let mut insert = tx
                .prepare(
                    "INSERT OR REPLACE INTO downloads(owner, track, quality, file, bytes, state, \
                     error, added_at) VALUES(?1, ?2, 'FLAC', ?3, ?4, 'done', NULL, 0)",
                )
                .expect("prepare");
            for (track, name, len) in &rows {
                insert
                    .execute(rusqlite::params![OWNER, track, name, len])
                    .expect("insert");
            }
        }
        tx.commit().expect("commit");
    }

    fn tags(&self) -> HashMap<String, nz_lan::store::StoredTag> {
        self.store.tags()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A friend's scope over exactly `tracks`.
fn friend(tracks: &[&str]) -> Scope {
    Scope::Friend {
        all: false,
        tracks: Arc::new(tracks.iter().map(|track| (*track).to_owned()).collect()),
    }
}

fn fetch(
    track: &str,
    quality: WireQuality,
    from: u64,
    expect: Option<FileTag>,
    urgency: Urgency,
) -> Request {
    Request::Fetch {
        track: track.to_owned(),
        quality,
        from,
        expect,
        urgency,
    }
}

fn head_bytes(response: &Response) -> Vec<u8> {
    serde_json::to_vec(response).expect("json")
}

/// Reads a body to its end, without `AsyncReadExt` (nz-lan's manifest enables
/// `fs`/`time`/`sync` on tokio, not `io-util`, and this lot does not own it).
async fn read_all(body: &mut Box<dyn tokio::io::AsyncRead + Unpin + Send>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let read = std::future::poll_fn(|cx| {
            use tokio::io::{AsyncRead, ReadBuf};
            let mut scratch = ReadBuf::new(&mut buffer);
            match std::pin::Pin::new(&mut *body).poll_read(cx, &mut scratch) {
                std::task::Poll::Pending => std::task::Poll::Pending,
                std::task::Poll::Ready(Err(error)) => std::task::Poll::Ready(Err(error)),
                std::task::Poll::Ready(Ok(())) => {
                    std::task::Poll::Ready(Ok(scratch.filled().len()))
                }
            }
        })
        .await
        .expect("a served body reads");
        if read == 0 {
            return out;
        }
        out.extend_from_slice(&buffer[..read]);
    }
}

/// `nz-cache`'s own mtime rule, so a test can hand the store the pair the index
/// will compare.
fn mtime_of(path: &Path) -> i64 {
    std::fs::metadata(path)
        .expect("metadata")
        .modified()
        .expect("mtime")
        .duration_since(UNIX_EPOCH)
        .expect("after the epoch")
        .as_nanos()
        .min(i64::MAX as u128) as i64
}

// --- the naming contract ----------------------------------------------------

/// The index derives `nz-cache`'s file names instead of storing them (48 B per
/// track instead of 90). This holds the two derivations together on REAL files
/// created through `nz-cache`'s own API, at the four rungs a peer serves: a
/// rename there fails here instead of silently emptying the inventory.
#[test]
fn the_index_derives_the_names_nz_cache_writes() {
    let fixture = Fixture::new("names");
    let mut checked = 0;
    for (n, quality) in [
        nz_model::Quality::Flac,
        nz_model::Quality::Opus320,
        nz_model::Quality::Opus128,
        nz_model::Quality::Opus64,
    ]
    .iter()
    .enumerate()
    {
        let track = format!("313555{n}");
        // A download for the even rungs, the play cache for the odd ones, so both
        // directories' naming is checked.
        let path = if n % 2 == 0 {
            fixture.download(&track, *quality, b"download")
        } else {
            fixture.play(&track, *quality, b"play")
        };
        let wire = WireQuality::from(*quality);
        let expected = file_name(&track, wire);
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(expected.as_str()),
            "nz-cache and the index disagree about {track} at {quality:?}"
        );
        checked += 1;
    }
    assert_eq!(checked, 4, "the four rungs a peer serves");
}

// --- indexing: time, statements, memory -------------------------------------

/// Acceptance: "an 8 000-track cache indexed in ≤ 200 ms (counted SQL statements
/// ≤ 5), `Index::memory()` ≤ 64 B per item".
///
/// Run: `cargo test -p nz-lan --release --test serve the_eight_thousand`.
/// A debug build is 3-4× slower (the assertion says which budget it applies).
#[test]
fn the_eight_thousand_track_cache_is_indexed_within_the_budget() {
    let fixture = Fixture::new("8000");
    fixture.many_downloads(0, 8_000, 1_024);

    let index = fixture.index();
    let started = Instant::now();
    index.refresh();
    let elapsed = started.elapsed();

    assert_eq!(index.tracks(), 8_000, "every complete download is listed");
    assert!(
        index.statements() <= 5,
        "indexing 8 000 tracks took {} statements (the cap is 5)",
        index.statements()
    );
    let per_item = index.memory() / index.tracks().max(1);
    assert!(
        per_item <= 64,
        "the inventory costs {per_item} B per item ({} B for 8 000 tracks), the cap is 64",
        index.memory()
    );
    let budget = if cfg!(debug_assertions) {
        // The measured debug cost is ~3.5× the release one; the note's 200 ms is
        // held in the profile it was measured in.
        Duration::from_millis(700)
    } else {
        Duration::from_millis(200)
    };
    eprintln!(
        "8 000 tracks indexed in {elapsed:?} ({} B/item, {} statements, {} ms budget)",
        per_item,
        index.statements(),
        budget.as_millis()
    );
    assert!(
        elapsed <= budget,
        "indexing 8 000 tracks took {elapsed:?}, the budget is {budget:?}"
    );
}

/// Acceptance: "an inventory page for `Scope::Own` lists every complete file and
/// nothing `.part`".
#[test]
fn an_inventory_page_lists_every_complete_file_and_nothing_part() {
    let fixture = Fixture::new("inventory");
    fixture.download("a", nz_model::Quality::Flac, b"aaaa");
    fixture.download("b", nz_model::Quality::Opus320, b"bbbb");
    let evicted = fixture.play("c", nz_model::Quality::Opus128, b"cccc");
    // A reservation is a file being written: a row, a `.part`, and nothing a peer
    // may fetch.
    let partial = fixture
        .cache
        .play_cache()
        .reserve(&nz_model::TrackId::new("d"), nz_model::Quality::Opus64)
        .expect("reserve");
    std::fs::write(&partial, vec![9u8; 512]).expect("partial bytes");

    let index = fixture.index();
    index.refresh();
    assert_eq!(
        index.tracks(),
        3,
        "a, b and the complete play-cache copy of c"
    );
    assert!(evicted.is_file());

    let Response::InventoryFull {
        items,
        last,
        version,
        ..
    } = index.inventory(&Scope::Own, None, 0)
    else {
        panic!("a full page");
    };
    assert!(last);
    assert_eq!(version, index.version());
    let tracks: Vec<&str> = items.iter().map(|item| item.track.as_str()).collect();
    assert_eq!(tracks, vec!["a", "b", "c"], "sorted, and d is not complete");

    let copy_of_c = items
        .iter()
        .find(|item| item.track == "c")
        .and_then(|item| item.copies.first())
        .expect("c has one copy");
    assert_eq!(copy_of_c.quality, WireQuality::Opus128);
    assert_eq!(copy_of_c.len, 4);

    // The same track at two qualities lists two copies. (A second DOWNLOAD at
    // another quality replaces the first file — that is `Downloads::reserve`'s own
    // rule — so the second copy here is a play-cache one.)
    fixture.play("a", nz_model::Quality::Opus320, b"a320");
    index.refresh();
    let Response::InventoryFull { items, .. } = index.inventory(&Scope::Own, None, 0) else {
        panic!("a full page");
    };
    let a = items.iter().find(|item| item.track == "a").expect("a");
    assert_eq!(a.copies.len(), 2, "FLAC and Opus 320");
    assert_eq!(a.copies.iter().map(|copy| copy.len).sum::<u64>(), 4 + 4);

    // A track that is both a download and a play-cache copy at the SAME quality is
    // served from the download: it is the copy that never disappears.
    fixture.play("b", nz_model::Quality::Opus320, b"bbb-play");
    index.refresh();
    let local = index.local("b", nz_model::Quality::Opus320).expect("b");
    assert_eq!(
        local.path.parent().and_then(|parent| parent.file_name()),
        Some(std::ffi::OsStr::new("offline")),
        "the download wins over the evictable copy"
    );

    // The second page is empty and says so, so a peer stops asking.
    let Response::InventoryFull { items, last, .. } = index.inventory(&Scope::Own, None, 1) else {
        panic!("a page");
    };
    assert!(items.is_empty() && last, "page 1 of a 3-item inventory");

    // And nothing in the inventory points at a `.part`.
    for item in &items {
        assert!(!item.track.contains("part"));
    }
    for path in index.local_paths() {
        assert!(
            !path.to_string_lossy().ends_with(".part"),
            "{} is a reservation",
            path.display()
        );
    }
}

/// Acceptance: "for `Scope::Friend` only allowed tracks — **mutant**: remove the
/// scope filter → the friend test fails".
///
/// The mutant was run by hand: deleting `if !scope.allows(&entry.track) { continue }`
/// in `Index::inventory` makes this test list four tracks instead of two.
#[test]
fn a_friend_sees_only_the_tracks_shared_with_them() {
    let fixture = Fixture::new("friend-inventory");
    for track in ["shared-1", "shared-2", "private-1", "private-2"] {
        fixture.download(track, nz_model::Quality::Flac, b"x");
    }
    let index = fixture.index();
    index.refresh();
    assert_eq!(index.tracks(), 4);

    let scope = friend(&["shared-1", "shared-2"]);
    let Response::InventoryFull { items, .. } = index.inventory(&scope, None, 0) else {
        panic!("a full page");
    };
    let tracks: Vec<&str> = items.iter().map(|item| item.track.as_str()).collect();
    assert_eq!(tracks, vec!["shared-1", "shared-2"]);

    // A friend shared everything sees everything.
    let all = Scope::Friend {
        all: true,
        tracks: Arc::new(HashSet::new()),
    };
    let Response::InventoryFull { items, .. } = index.inventory(&all, None, 0) else {
        panic!("a full page");
    };
    assert_eq!(items.len(), 4);
}

/// Acceptance: "an out-of-scope `Fetch`/`Meta`/`Cover` returns the byte-identical
/// frame of an absent track (compared bytes)".
#[tokio::test]
async fn an_out_of_scope_request_is_the_byte_identical_frame_of_an_absent_one() {
    let fixture = Fixture::new("refusals");
    fixture.download("private", nz_model::Quality::Flac, b"secret");
    let index = fixture.index();
    index.refresh();
    let server = Server::new(
        index,
        Arc::new(nz_api::lanes::Gates::new()),
        ServeLimits::default(),
    );
    // "shared-absent" is IN the friend's scope and on no disk; "private" is on this
    // disk and OUT of their scope. T7 is that the two answers are the same bytes.
    let scope = friend(&["shared", "shared-absent"]);

    // Fetch: an unshared track that EXISTS versus an in-scope one that does not.
    let unshared = server
        .answer(
            &scope,
            &fetch("private", WireQuality::Flac, 0, None, Urgency::Play),
        )
        .await;
    let absent = server
        .answer(
            &scope,
            &fetch("shared-absent", WireQuality::Flac, 0, None, Urgency::Play),
        )
        .await;
    assert_eq!(
        head_bytes(&unshared.head),
        head_bytes(&absent.head),
        "a friend must not learn that 'private' exists"
    );
    assert!(!unshared.has_body() && !absent.has_body());

    // Meta: the same, for the whole frame. A `Meta` request carries many ids, so
    // its refusal is an empty row list — identical for ids that are unshared and
    // for ids nothing knows.
    let unshared = server
        .answer(
            &scope,
            &Request::Meta {
                tracks: vec!["private".to_owned()],
            },
        )
        .await;
    let absent = server
        .answer(
            &scope,
            &Request::Meta {
                tracks: vec!["shared-absent".to_owned()],
            },
        )
        .await;
    assert_eq!(head_bytes(&unshared.head), head_bytes(&absent.head));
    assert_eq!(
        head_bytes(&unshared.head),
        br#"{"t":"Meta","rows":[]}"#.to_vec()
    );

    // Cover: an art URL this node HAS, referenced by an unshared track, versus one
    // it does not have at all.
    let art = b"\xFF\xD8\xFF\xE0JFIF-secret-art";
    fixture
        .cache
        .covers()
        .put("/api/cover/900", art)
        .expect("cover");
    let unshared = server
        .answer(
            &scope,
            &Request::Cover {
                url: "/api/cover/900".to_owned(),
            },
        )
        .await;
    let absent = server
        .answer(
            &scope,
            &Request::Cover {
                url: "/api/cover/404".to_owned(),
            },
        )
        .await;
    assert_eq!(head_bytes(&unshared.head), head_bytes(&absent.head));
    assert_eq!(head_bytes(&unshared.head), br#"{"t":"NotFound"}"#.to_vec());

    // My own device gets the art it has, and a URL that is not a cover URL at all
    // (a path, a scheme) never reaches the cover store.
    let mine = server
        .answer(
            &Scope::Own,
            &Request::Cover {
                url: "/api/cover/900".to_owned(),
            },
        )
        .await;
    assert!(mine.has_body(), "the person's own device gets its art");
    for hostile in [
        "../../etc/passwd",
        "/api/cover/../../../etc/passwd",
        "https://evil.example/api/cover/1",
        "/api/cover/",
        "/api/cover/a/b",
    ] {
        let answer = server
            .answer(
                &Scope::Own,
                &Request::Cover {
                    url: hostile.to_owned(),
                },
            )
            .await;
        assert_eq!(
            head_bytes(&answer.head),
            br#"{"t":"NotFound"}"#.to_vec(),
            "{hostile} must not reach the cover store"
        );
    }
}

// --- fetch ------------------------------------------------------------------

/// Acceptance: "`Fetch` serves from `from`, refuses `from > len`, answers
/// `NotFound` when `expect` differs".
#[tokio::test]
async fn a_fetch_serves_from_the_asked_offset_and_refuses_a_bad_one() {
    let fixture = Fixture::new("fetch");
    let bytes: Vec<u8> = (0..50_000u32).map(|n| (n % 251) as u8).collect();
    let path = fixture.download("song", nz_model::Quality::Flac, &bytes);
    let index = fixture.index();
    index.refresh();
    // The tag the hashing worker would have written.
    let tag = nz_lan_hash(&bytes);
    fixture
        .store
        .put_tag(&path, bytes.len() as u64, mtime_of(&path), &tag)
        .expect("tag");

    let server = Server::new(
        index.clone(),
        Arc::new(nz_api::lanes::Gates::new()),
        ServeLimits::default(),
    );

    // From 0: the whole file, with its tag.
    let mut answer = server
        .answer(
            &Scope::Own,
            &fetch("song", WireQuality::Flac, 0, None, Urgency::Play),
        )
        .await;
    let (len, served, quality) = match &answer.head {
        Response::FetchOk { len, tag, quality } => (*len, tag.clone(), *quality),
        other => panic!("a fetch: {other:?}"),
    };
    assert_eq!(len, bytes.len() as u64);
    assert_eq!(quality, WireQuality::Flac);
    assert_eq!(served.expect("a tag").sha256, tag);
    let mut body = answer.body.take().expect("body");
    assert_eq!(read_all(&mut body).await, bytes);
    drop(body);
    drop(answer);

    // From the middle: the tail only.
    let mut answer = server
        .answer(
            &Scope::Own,
            &fetch("song", WireQuality::Flac, 12_345, None, Urgency::Play),
        )
        .await;
    assert!(matches!(answer.head, Response::FetchOk { .. }));
    let mut body = answer.body.take().expect("body");
    assert_eq!(read_all(&mut body).await, bytes[12_345..]);
    drop(body);
    drop(answer);

    // From the end: nothing follows, which is a legal resume point.
    let mut answer = server
        .answer(
            &Scope::Own,
            &fetch(
                "song",
                WireQuality::Flac,
                bytes.len() as u64,
                None,
                Urgency::Play,
            ),
        )
        .await;
    {
        let mut body = answer.body.take().expect("body");
        assert!(read_all(&mut body).await.is_empty());
    }
    drop(answer);

    // Past the end: refused, exactly like a track that is not here.
    let past = server
        .answer(
            &Scope::Own,
            &fetch(
                "song",
                WireQuality::Flac,
                bytes.len() as u64 + 1,
                None,
                Urgency::Play,
            ),
        )
        .await;
    assert_eq!(head_bytes(&past.head), br#"{"t":"NotFound"}"#.to_vec());

    // A resume naming ANOTHER file: refused, so a restart happens from 0 instead
    // of appending two different files.
    let other = FileTag {
        len: bytes.len() as u64,
        sha256: "00".repeat(32),
    };
    let wrong = server
        .answer(
            &Scope::Own,
            &fetch(
                "song",
                WireQuality::Flac,
                100,
                Some(other.clone()),
                Urgency::Play,
            ),
        )
        .await;
    assert_eq!(head_bytes(&wrong.head), br#"{"t":"NotFound"}"#.to_vec());

    // A resume naming THIS file: served.
    let right = server
        .answer(
            &Scope::Own,
            &fetch(
                "song",
                WireQuality::Flac,
                100,
                Some(FileTag {
                    len: bytes.len() as u64,
                    sha256: tag.clone(),
                }),
                Urgency::Play,
            ),
        )
        .await;
    assert!(matches!(right.head, Response::FetchOk { .. }));
    drop(right);

    // A track this device does not have: the same refusal.
    let missing = server
        .answer(
            &Scope::Own,
            &fetch("absent", WireQuality::Flac, 0, None, Urgency::Play),
        )
        .await;
    assert_eq!(head_bytes(&missing.head), br#"{"t":"NotFound"}"#.to_vec());

    // A quality this device does not have is served from the one it does, and the
    // answer names it: a peer file is always complete, and refusing would send the
    // person to the slower server for a copy already here.
    let answer = server
        .answer(
            &Scope::Own,
            &fetch("song", WireQuality::Opus64, 0, None, Urgency::Play),
        )
        .await;
    match &answer.head {
        Response::FetchOk { quality, .. } => {
            assert_eq!(*quality, WireQuality::Flac, "what is served")
        }
        other => panic!("a fetch: {other:?}"),
    }
    drop(answer);

    // A play-cache file that was evicted after the inventory was built: NotFound,
    // never an error (the note's pitfall).
    let evicted = fixture.play("cache-me", nz_model::Quality::Opus320, b"evictable");
    index.refresh();
    std::fs::remove_file(&evicted).expect("evict");
    let gone = server
        .answer(
            &Scope::Own,
            &fetch("cache-me", WireQuality::Opus320, 0, None, Urgency::Play),
        )
        .await;
    assert_eq!(head_bytes(&gone.head), br#"{"t":"NotFound"}"#.to_vec());
}

/// Acceptance: "the 3rd concurrent `Play` → `Busy`".
#[tokio::test]
async fn the_third_concurrent_play_is_busy_and_the_slot_comes_back() {
    let fixture = Fixture::new("limits");
    fixture.download("a", nz_model::Quality::Flac, b"aaaaaaaaaa");
    let index = fixture.index();
    index.refresh();
    let server = Server::new(
        index,
        Arc::new(nz_api::lanes::Gates::new()),
        ServeLimits::default(),
    );

    // Two plays answer with a body and hold their slots until the body is dropped.
    let first = server
        .answer(
            &Scope::Own,
            &fetch("a", WireQuality::Flac, 0, None, Urgency::Play),
        )
        .await;
    let second = server
        .answer(
            &Scope::Own,
            &fetch("a", WireQuality::Flac, 0, None, Urgency::Play),
        )
        .await;
    assert!(first.has_body() && second.has_body());
    assert_eq!(server.counts(), (2, 0, 0));

    let third = server
        .answer(
            &Scope::Own,
            &fetch("a", WireQuality::Flac, 0, None, Urgency::Play),
        )
        .await;
    let Response::Busy { retry_in_ms } = third.head else {
        panic!("the third play must be told to come back: {:?}", third.head);
    };
    assert_eq!(retry_in_ms, BUSY_RETRY_MS);
    assert!(!third.has_body(), "a Busy answer carries no bytes");

    // The prefetch and bulk budgets are separate and smaller.
    let prefetch = server
        .answer(
            &Scope::Own,
            &fetch("a", WireQuality::Flac, 0, None, Urgency::Prefetch),
        )
        .await;
    assert!(prefetch.has_body());
    let second_prefetch = server
        .answer(
            &Scope::Own,
            &fetch("a", WireQuality::Flac, 0, None, Urgency::Prefetch),
        )
        .await;
    assert!(matches!(second_prefetch.head, Response::Busy { .. }));

    // Dropping an answer frees its slot: a peer that hangs up does not keep it.
    drop(first);
    assert_eq!(server.counts(), (1, 1, 0));
    let next = server
        .answer(
            &Scope::Own,
            &fetch("a", WireQuality::Flac, 0, None, Urgency::Play),
        )
        .await;
    assert!(next.has_body(), "the freed Play slot admits the next tap");
    drop(second);
    drop(next);
    assert_eq!(server.counts(), (0, 1, 0));
}

/// Acceptance: "a `Bulk` serve waits while the node's own `Gates` show a `User`
/// request in flight".
///
/// The wait is proven by the future's own state, not by a clock: polled once it is
/// pending, and it completes only after the `User` guard is released. (`start_paused`
/// needs tokio's `test-util` feature, which nz-lan's manifest does not enable and
/// this lot does not own; the assertion below does not read the clock either.)
#[tokio::test]
async fn a_bulk_serve_waits_while_a_user_request_is_in_flight() {
    let fixture = Fixture::new("bulk-gate");
    fixture.download("bulk", nz_model::Quality::Flac, b"0123456789");
    let index = fixture.index();
    index.refresh();
    let gates = Arc::new(nz_api::lanes::Gates::new());
    let server = Server::new(index, Arc::clone(&gates), ServeLimits::default());

    let user = gates.acquire(nz_api::lanes::Lane::User).await;
    let request = fetch("bulk", WireQuality::Flac, 0, None, Urgency::Bulk);
    let mut answer = Box::pin(server.answer(&Scope::Own, &request));
    // One poll: it reached the gate and is waiting there.
    assert!(
        tokio::time::timeout(Duration::ZERO, &mut answer)
            .await
            .is_err(),
        "a Bulk serve must not run while the person's own request is in flight"
    );
    // The slot is already counted, so a SECOND bulk request is told to wait
    // instead of piling up behind this one.
    assert_eq!(server.counts(), (0, 0, 1));
    let crowded = server
        .answer(
            &Scope::Own,
            &fetch("bulk", WireQuality::Flac, 0, None, Urgency::Bulk),
        )
        .await;
    assert!(matches!(crowded.head, Response::Busy { .. }));

    // The person's request ends: the gate admits the serve, which then answers.
    drop(user);
    let mut answer = answer.await;
    assert!(matches!(answer.head, Response::FetchOk { .. }));
    let mut body = answer.body.take().expect("body");
    assert_eq!(read_all(&mut body).await, b"0123456789");
    drop(answer);
    assert_eq!(server.counts(), (0, 0, 0), "the bulk slot comes back");
}

/// A `Play` serve never waits for the bulk lane: the tapped track is the whole
/// point of rule 1.
#[tokio::test]
async fn a_play_serve_never_waits_for_a_bulk_one() {
    let fixture = Fixture::new("play-never-waits");
    fixture.download("tap", nz_model::Quality::Flac, b"tap");
    let index = fixture.index();
    index.refresh();
    let gates = Arc::new(nz_api::lanes::Gates::new());
    let server = Server::new(index, Arc::clone(&gates), ServeLimits::default());

    // A bulk transfer holds the lane (`BULK_QUIET` is irrelevant: the guard is
    // held, so no other bulk request is admitted).
    let bulk = server
        .answer(
            &Scope::Own,
            &fetch("tap", WireQuality::Flac, 0, None, Urgency::Bulk),
        )
        .await;
    assert!(bulk.has_body());
    // 500 ms: a `Play` that waited for the bulk lane would take the gate's whole
    // 1 s quiet period, so this still tells the two apart while tolerating a
    // loaded runner's file open.
    let tapped = tokio::time::timeout(
        Duration::from_millis(500),
        server.answer(
            &Scope::Own,
            &fetch("tap", WireQuality::Flac, 0, None, Urgency::Play),
        ),
    )
    .await
    .expect("the tapped track is answered at once");
    assert!(tapped.has_body());
}

// --- the delta ring ---------------------------------------------------------

/// Acceptance: "`delta` since a version ≤ 64 changes old, `full` otherwise".
#[test]
fn a_delta_covers_recent_changes_and_a_big_one_falls_back_to_the_full_pages() {
    let fixture = Fixture::new("delta");
    fixture.download("a", nz_model::Quality::Flac, b"a");
    let index = fixture.index();
    index.refresh();
    let first = index.version();
    assert_eq!(first, 1, "the first inventory is version 1");

    // A small change: a delta from the previous version names it.
    fixture.download("b", nz_model::Quality::Opus320, b"bb");
    index.refresh();
    assert_eq!(index.version(), 2);
    match index.inventory(&Scope::Own, Some(first), 0) {
        Response::InventoryDelta {
            from,
            to,
            added,
            removed,
        } => {
            assert_eq!((from, to), (first, 2));
            assert_eq!(added.len(), 1);
            assert_eq!(added[0].track, "b");
            assert!(removed.is_empty());
        }
        other => panic!("a delta was expected: {other:?}"),
    }

    // A peer that is already up to date gets an empty delta, not a full page.
    match index.inventory(&Scope::Own, Some(index.version()), 0) {
        Response::InventoryDelta { added, removed, .. } => {
            assert!(added.is_empty() && removed.is_empty());
        }
        other => panic!("an empty delta was expected: {other:?}"),
    }

    // A change too big to be a delta clears the ring: an older peer gets the full
    // pages, which is the only honest answer once the gap exists.
    fixture.many_downloads(0, 600, 16);
    index.refresh();
    let after_big = index.version();
    assert_eq!(after_big, 3);
    match index.inventory(&Scope::Own, Some(first), 0) {
        Response::InventoryFull { items, .. } => assert!(items.len() >= 602),
        other => panic!("the full pages were expected: {other:?}"),
    }
    // And a small change after the gap is a delta from its own predecessor.
    fixture.download("c", nz_model::Quality::Flac, b"ccc");
    index.refresh();
    assert!(matches!(
        index.inventory(&Scope::Own, Some(after_big), 0),
        Response::InventoryDelta { .. }
    ));
    // A peer from the future (a reinstall) gets the full pages too.
    assert!(matches!(
        index.inventory(&Scope::Own, Some(after_big + 99), 0),
        Response::InventoryFull { .. }
    ));
}

// --- sets and metadata ------------------------------------------------------

/// Acceptance (from *Protocol messages*): `Sets` lists the sets whose members are
/// all in the requester's scope, with `complete`/`local`, and a friend never gets
/// the art of a set they cannot see whole.
#[test]
fn a_friend_sees_only_the_sets_they_can_see_whole() {
    let fixture = Fixture::new("sets");
    let fully_shared = nz_model::TrackId::new("s1");
    let second_shared = nz_model::TrackId::new("s2");
    let private = nz_model::TrackId::new("p1");
    fixture
        .cache
        .downloads()
        .add_set(
            OWNER,
            "playlist",
            "7",
            "Route",
            &[
                nz_cache::DownloadTrack {
                    track: fully_shared.clone(),
                    quality: nz_model::Quality::Flac,
                },
                nz_cache::DownloadTrack {
                    track: second_shared.clone(),
                    quality: nz_model::Quality::Flac,
                },
            ],
        )
        .expect("shared set");
    fixture
        .cache
        .downloads()
        .add_set(
            OWNER,
            "playlist",
            "8",
            "Privé",
            &[nz_cache::DownloadTrack {
                track: private.clone(),
                quality: nz_model::Quality::Flac,
            }],
        )
        .expect("private set");
    for track in [&fully_shared, &second_shared] {
        let partial = fixture
            .cache
            .downloads()
            .reserve(OWNER, track, nz_model::Quality::Flac)
            .expect("reserve");
        std::fs::write(&partial, b"abcdefgh").expect("bytes");
        fixture
            .cache
            .downloads()
            .complete(OWNER, track, 8)
            .expect("complete");
    }

    let index = fixture.index();
    index.refresh();
    assert_eq!(
        index.sets_version(),
        1,
        "the first read of the sets is version 1"
    );

    let Response::Sets { sets, last, .. } = index.sets(&Scope::Own, 0) else {
        panic!("a sets page");
    };
    assert!(last);
    assert_eq!(sets.len(), 2, "both sets are mine");
    let route = sets.iter().find(|set| set.ident == "7").expect("Route");
    assert_eq!(route.title, "Route");
    assert_eq!(route.local, 2);
    assert!(route.complete, "both members are on disk");
    assert!(!route.tracks.is_empty());
    let private_set = sets.iter().find(|set| set.ident == "8").expect("Privé");
    assert_eq!(private_set.local, 0, "p1 was never downloaded");
    assert!(!private_set.complete);

    let Response::Sets { sets, .. } = index.sets(&friend(&["s1", "s2"]), 0) else {
        panic!("a sets page");
    };
    assert_eq!(sets.len(), 1, "the private playlist is not advertised");
    assert_eq!(sets[0].ident, "7");
    assert_eq!(
        sets[0].cover, None,
        "a restricted friend does not get the playlist's own art"
    );

    // Editing a set's title moves the sets version, so a peer re-asks.
    fixture
        .cache
        .downloads()
        .add_set(OWNER, "playlist", "7", "Route du soir", &[])
        .expect("rename");
    index.refresh();
    assert_eq!(index.sets_version(), 2);

    // Paging: ten tiny sets are 8 on the first page and 2 on the second, and no
    // set is skipped between them (the page bound is computed from the set list,
    // not from what one page happened to send).
    for n in 0..10 {
        fixture
            .cache
            .downloads()
            .add_set(
                OWNER,
                "album",
                &format!("al{n}"),
                &format!("Album {n}"),
                &[],
            )
            .expect("set");
    }
    index.refresh();
    let Response::Sets { sets, last, .. } = index.sets(&Scope::Own, 0) else {
        panic!("a sets page");
    };
    assert_eq!(sets.len(), 8, "the first page is full");
    assert!(!last, "and more follows");
    let Response::Sets { sets, last, .. } = index.sets(&Scope::Own, 1) else {
        panic!("a sets page");
    };
    assert_eq!(sets.len(), 4, "12 sets: 4 remain");
    assert!(last, "nothing follows the last page");
    // Past the end: an empty page that says so, never a panic.
    let Response::Sets { sets, last, .. } = index.sets(&Scope::Own, u32::MAX) else {
        panic!("a sets page");
    };
    assert!(sets.is_empty() && last);
}

/// `Meta`: only in-scope tracks, screen-shaped, and each field from the place the
/// note names — `downloads.labels` for title/artist, the cached album page for the
/// rest.
#[test]
fn meta_answers_in_scope_tracks_from_the_labels_and_the_cached_pages() {
    let fixture = Fixture::new("meta");
    let track = nz_model::TrackId::new("42");
    let private = nz_model::TrackId::new("99");
    fixture
        .cache
        .downloads()
        .add_set(
            OWNER,
            "album",
            "900",
            "Un album",
            &[
                nz_cache::DownloadTrack {
                    track: track.clone(),
                    quality: nz_model::Quality::Flac,
                },
                nz_cache::DownloadTrack {
                    track: private.clone(),
                    quality: nz_model::Quality::Flac,
                },
            ],
        )
        .expect("set");
    for id in [&track, &private] {
        let partial = fixture
            .cache
            .downloads()
            .reserve(OWNER, id, nz_model::Quality::Flac)
            .expect("reserve");
        std::fs::write(&partial, b"1234").expect("bytes");
        fixture
            .cache
            .downloads()
            .complete(OWNER, id, 4)
            .expect("complete");
    }
    // The album page the set was opened from, as the API cache stores it.
    let page = nz_model::album::AlbumPage {
        album: nz_model::album::Album {
            title: "Un album".to_owned(),
            cover: Some("/api/cover/900".to_owned()),
            ..nz_model::album::Album::default()
        },
        tracks: vec![
            nz_model::track::Track {
                deezer_id: nz_model::TrackId::new("42"),
                title: "Une piste".to_owned(),
                duration: 213,
                explicit: true,
                artist: nz_model::track::TrackArtist {
                    name: "Camille".to_owned(),
                    ..nz_model::track::TrackArtist::default()
                },
                album: nz_model::track::TrackAlbum {
                    title: "Un album".to_owned(),
                    cover: Some("/api/cover/900".to_owned()),
                    ..nz_model::track::TrackAlbum::default()
                },
                ..nz_model::track::Track::default()
            },
            nz_model::track::Track {
                deezer_id: nz_model::TrackId::new("99"),
                title: "Une autre".to_owned(),
                ..nz_model::track::Track::default()
            },
        ],
        ..nz_model::album::AlbumPage::default()
    };
    fixture.cache.put(
        OWNER,
        "/album/900",
        &serde_json::to_string(&page).expect("json"),
    );
    // And a label for a track no cached page knows (a single-track download).
    let mut labels = HashMap::new();
    labels.insert(
        "42".to_owned(),
        nz_offline::Labels {
            title: "Le libellé".to_owned(),
            artist: "L'artiste".to_owned(),
        },
    );
    fixture
        .cache
        .kv()
        .set(nz_offline::LABELS_KEY, &labels)
        .expect("labels");

    let index = fixture.index();
    index.refresh();
    let scope = friend(&["42"]);
    let Response::Meta { rows } = index.meta(&scope, &["42".to_owned(), "99".to_owned()]) else {
        panic!("a meta answer");
    };
    assert_eq!(rows.len(), 1, "99 is not shared, so it is not even named");
    let row = &rows[0];
    assert_eq!(row.id, "42");
    assert_eq!(row.title, "Une piste");
    assert_eq!(row.artists, "Camille");
    assert_eq!(row.album, "Un album");
    assert_eq!(row.duration_s, 213);
    assert_eq!(row.cover.as_deref(), Some("/api/cover/900"));
    assert!(row.explicit);

    // The cap: a request with 1 000 ids answers at most the note's 200 rows.
    let many: Vec<String> = (0..1_000).map(|n| format!("id-{n}")).collect();
    let Response::Meta { rows } = index.meta(&Scope::Own, &many) else {
        panic!("a meta answer");
    };
    assert!(rows.len() <= 200, "{} rows", rows.len());
}

/// A cover a restricted friend may fetch: exactly the art of the sets they can see
/// whole.
#[test]
fn a_friend_gets_only_the_art_of_the_sets_they_see_whole() {
    let fixture = Fixture::new("cover-scope");
    fixture.download("s1", nz_model::Quality::Flac, b"1");
    fixture
        .cache
        .downloads()
        .add_set(
            OWNER,
            "playlist",
            "7",
            "Route",
            &[nz_cache::DownloadTrack {
                track: nz_model::TrackId::new("s1"),
                quality: nz_model::Quality::Flac,
            }],
        )
        .expect("set");
    let page = nz_model::playlist::PlaylistPage {
        playlist: nz_model::playlist::Playlist {
            id: nz_model::PlaylistId::new("7"),
            title: "Route".to_owned(),
            cover: Some("/api/cover/7".to_owned()),
            ..nz_model::playlist::Playlist::default()
        },
        tracks: vec![nz_model::track::Track {
            deezer_id: nz_model::TrackId::new("s1"),
            title: "Une".to_owned(),
            album: nz_model::track::TrackAlbum {
                cover: Some("/api/cover/900".to_owned()),
                ..nz_model::track::TrackAlbum::default()
            },
            ..nz_model::track::Track::default()
        }],
    };
    fixture.cache.put(
        OWNER,
        "/playlist/7",
        &serde_json::to_string(&page).expect("json"),
    );
    fixture
        .cache
        .covers()
        .put("/api/cover/7", b"playlist art")
        .expect("cover");
    fixture
        .cache
        .covers()
        .put("/api/cover/900", b"album art")
        .expect("cover");
    fixture
        .cache
        .covers()
        .put("/api/cover/666", b"someone else")
        .expect("cover");

    let index = fixture.index();
    index.refresh();
    let shared = friend(&["s1"]);
    assert!(
        index.cover(&shared, "/api/cover/7").is_some(),
        "the set's own art comes with the set"
    );
    assert!(
        index.cover(&shared, "/api/cover/900").is_some(),
        "a shared track's album art comes with the track"
    );
    assert!(
        index.cover(&shared, "/api/cover/666").is_none(),
        "art nothing shared references must not be served"
    );
    assert!(index.cover(&Scope::Own, "/api/cover/666").is_some());
}

// --- the cover cap ----------------------------------------------------------

/// A cover over the note's 2 MiB answer cap is refused rather than truncated.
#[test]
fn a_cover_over_the_answer_cap_is_not_served() {
    let fixture = Fixture::new("cover-cap");
    let big = vec![0u8; (MAX_COVER_BYTES + 1) as usize];
    fixture
        .cache
        .covers()
        .put("/api/cover/big", &big)
        .expect("cover");
    let index = fixture.index();
    assert!(index.cover(&Scope::Own, "/api/cover/big").is_none());
    assert_eq!(MAX_COVER_BYTES, 2 * 1024 * 1024);
}

// --- hashing: the pause, and the yield --------------------------------------

/// A hashed file: its tag, the length/mtime pair that makes the next boot skip it,
/// and the `nice +10` thread. (`hashing`'s unit tests hold the rule and the join;
/// this one holds the END of the story: the tag a `Fetch` announces.)
#[test]
fn hashing_tags_every_listed_file_and_the_tag_is_the_fetch_tag() {
    let fixture = Fixture::new("hashing");
    let bytes = vec![3u8; 3 * 1024 * 1024];
    let path = fixture.download("long", nz_model::Quality::Flac, &bytes);
    let index = fixture.index();
    index.refresh();

    let worker = nz_lan::hashing::Hashing::start(index.clone(), fixture.store.clone());
    let mut hashed = index.hashed();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    runtime
        .block_on(hashed.wait_for(|count| *count >= 1))
        .expect("the worker hashed the file");
    worker.stop();

    let stored = fixture.store.tag(&path).expect("a tag");
    assert_eq!(stored.len, bytes.len() as u64);
    assert_eq!(stored.sha256, nz_lan_hash(&bytes));
    let local = index.local("long", nz_model::Quality::Flac).expect("local");
    assert_eq!(local.tag.as_ref().expect("the fetch tag"), &stored);
    assert!(
        !nz_lan::hashing::needs_hash(&path, &fixture.tags()),
        "a second boot does not rehash"
    );
}

/// Acceptance: "hashes ≥ 500 MB/s (release, reported)".
///
/// Run: `cargo test -p nz-lan --release --test serve hashing_reaches`.
/// The budget is asserted in release only: a debug build of the surrounding code
/// (the file read loop, the store write) is several times slower, and the note's
/// number was measured in release.
#[test]
fn hashing_reaches_the_notes_throughput() {
    let fixture = Fixture::new("hash-speed");
    const SIZE: usize = 64 * 1024 * 1024;
    let path = fixture.download("big", nz_model::Quality::Flac, &vec![0x5au8; SIZE]);
    let index = fixture.index();
    index.refresh();

    let started = Instant::now();
    let worker = nz_lan::hashing::Hashing::start(index.clone(), fixture.store.clone());
    let mut hashed = index.hashed();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    runtime
        .block_on(hashed.wait_for(|count| *count >= 1))
        .expect("the worker hashed the file");
    let elapsed = started.elapsed();
    worker.stop();
    assert!(fixture.store.tag(&path).is_some());

    let megabytes = SIZE as f64 / (1024.0 * 1024.0);
    let mb_per_s = megabytes / elapsed.as_secs_f64();
    eprintln!("hashed 64 MiB in {elapsed:?}: {mb_per_s:.0} MB/s");
    if !cfg!(debug_assertions) {
        assert!(
            mb_per_s >= 500.0,
            "the hashing worker reached {mb_per_s:.0} MB/s, the note asks for 500"
        );
    }
}

/// Acceptance: "the hashing … pauses while a `User` request is in flight".
///
/// Event-driven: the worker publishes `Waiting` when it reaches the gate, the test
/// checks the tag is absent at that instant, then releases the person's request
/// and waits for the tag. No sleep and no clock read.
#[test]
fn hashing_waits_while_a_user_request_is_in_flight() {
    let fixture = Fixture::new("hash-pause");
    // Two tiles: the worker reaches the gate before reading anything, and would
    // keep it busy for a while.
    let bytes = vec![5u8; 2 * 1024 * 1024];
    let path = fixture.download("paused", nz_model::Quality::Flac, &bytes);
    let index = fixture.index();
    index.refresh();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let gates = Arc::new(nz_api::lanes::Gates::new());
    let user = runtime.block_on(gates.acquire(nz_api::lanes::Lane::User));

    let worker = nz_lan::hashing::Hashing::start_with(
        index.clone(),
        fixture.store.clone(),
        Arc::clone(&gates),
    );
    let mut state = worker.state();
    // A WATCHDOG, not a wait: the correct worker reports `Waiting` within
    // microseconds of starting, so this only fires on a worker that ignores the
    // gate (the mutant this test was run against) — and it then says so instead of
    // hanging the run.
    match runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(10),
            state.wait_for(|state| *state == nz_lan::hashing::HashState::Waiting),
        )
        .await
    }) {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => panic!("the worker's state channel closed: {error}"),
        Err(_) => panic!(
            "the worker never said it was waiting for the gate: it does not yield to the person"
        ),
    }
    assert!(
        fixture.store.tag(&path).is_none(),
        "nothing is hashed while the person's own request is in flight"
    );

    // The person's request ends: the worker proceeds and the tag appears.
    drop(user);
    let mut hashed = index.hashed();
    match runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(10),
            hashed.wait_for(|count| *count >= 1),
        )
        .await
    }) {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => panic!("the index's hash counter closed: {error}"),
        Err(_) => panic!("the worker did not hash the file once the gate opened"),
    }
    worker.stop();
    assert!(fixture.store.tag(&path).is_some());
}

// --- helpers ----------------------------------------------------------------

/// The SHA-256 the crate's worker computes, for a test to compare against.
fn nz_lan_hash(bytes: &[u8]) -> String {
    use aws_lc_rs::digest::{SHA256, digest};
    digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `LocalFile` is part of the contract; a compile-time use keeps its shape.
#[test]
fn the_local_file_shape_is_the_notes() {
    let file = LocalFile {
        path: PathBuf::from("/x.oga"),
        quality: nz_model::Quality::Opus128,
        len: 1,
        tag: None,
    };
    assert_eq!(file.len, 1);
    // `proto::Copy` is what the wire carries for the same information.
    let copy = proto::Copy {
        quality: WireQuality::Opus128,
        len: file.len,
    };
    assert_eq!(copy.len, 1);
}
