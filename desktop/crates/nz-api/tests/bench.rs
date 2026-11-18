//! The real server, not a stub: `tools/perf_api.py --serve` on a throwaway
//! SQLite database, exactly as D1's fixture script starts it.
//!
//! What this proves is the part a mock cannot: the SPA's session contract
//! survives a real Flask cookie (login → `/api/me` → `401` → logout), and real
//! payloads come back through the typed endpoints and `nz-model`'s types.
//!
//! The bench needs an interpreter that can `import neezer` (the project
//! installed, `pip install -e .`). `NZ_PYTHON=/path/to/python` overrides the
//! search; otherwise `python3` and the repository's `.venv` are tried. There is
//! no `#[ignore]`: the desktop job installs the server, and a skipped test here
//! would hide a broken session round trip.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use nz_api::{Api, ApiError, Lane, Method, Quality, RadioSeed, SessionJar, TrackId};

/// The repository root (`desktop/crates/nz-api` → up three).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("the repository root")
}

/// The first interpreter that can import the server.
fn python() -> String {
    let candidates: Vec<String> = std::env::var("NZ_PYTHON")
        .ok()
        .into_iter()
        .chain([
            "python3".to_owned(),
            repo_root().join(".venv/bin/python").display().to_string(),
            repo_root().join("venv/bin/python").display().to_string(),
        ])
        .collect();
    for candidate in &candidates {
        let usable = Command::new(candidate)
            .args(["-c", "import peewee, neezer"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if usable {
            return candidate.clone();
        }
    }
    panic!(
        "no Python interpreter can `import neezer` (tried: {}). Install the server: \
         `python3 -m venv .venv && .venv/bin/pip install -e .`, or point NZ_PYTHON at one.",
        candidates.join(", ")
    );
}

/// A `perf_api.py` server on a throwaway database.
struct Bench {
    child: Child,
    port: u16,
    ids: serde_json::Value,
}

impl Bench {
    fn start() -> Self {
        let script = repo_root().join("tools/perf_api.py");
        let mut child = Command::new(python())
            .arg(script)
            // The seed is small on purpose: this test proves the SESSION and the
            // typed routes, not the size of a library. One linked provider
            // account, because `/api/me/accounts` is otherwise an empty list and
            // the `PUT` contract (the `auto` shape) has nothing to act on.
            .args([
                "--tracks",
                "12",
                "--playlists",
                "2",
                "--favorites",
                "4",
                "--big-playlist",
                "4",
                "--seed-provider-account",
                "--podcasts",
                "--serve",
                "--port",
                "0",
            ])
            .current_dir(repo_root())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .stdin(Stdio::null())
            .spawn()
            .expect("start perf_api.py");

        let stdout = child.stdout.take().expect("its stdout");
        let (sender, receiver) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = sender.send(line);
            }
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(180);
        let mut port = None;
        let mut ids = serde_json::Value::Null;
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            match receiver.recv_timeout(left) {
                Ok(line) if line.starts_with("READY ") => {
                    let mut parts = line.splitn(3, ' ');
                    parts.next();
                    port = parts.next().and_then(|port| port.parse::<u16>().ok());
                    ids = parts
                        .next()
                        .and_then(|raw| serde_json::from_str(raw).ok())
                        .unwrap_or(serde_json::Value::Null);
                    break;
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        let port = port.expect("perf_api.py never printed READY");
        Self { child, port, ids }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn first_track(&self) -> TrackId {
        let id = self.ids["ids"]
            .get(0)
            .unwrap_or_else(|| panic!("the bench printed no track ids: {}", self.ids));
        TrackId::from(match id {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        })
    }

    /// The linked provider account `--seed-provider-account` created.
    fn account(&self) -> String {
        self.ids["account"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("the bench printed no account: {}", self.ids))
            .to_owned()
    }

    /// A playlist of the seed, for the version-conflict proof.
    fn a_show(&self) -> String {
        self.ids["podcast"]["show"]
            .as_str()
            .unwrap_or_else(|| panic!("the bench printed no show: {}", self.ids))
            .to_owned()
    }

    fn a_playlist(&self) -> String {
        self.ids["small"]
            .as_str()
            .unwrap_or_else(|| panic!("the bench printed no playlist id: {}", self.ids))
            .to_owned()
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        stop(&mut self.child);
    }
}

/// Stop the bench the way it expects: SIGTERM first, SIGKILL only as a last
/// resort.
///
/// `tools/perf_api.py` removes its throwaway library (5 MB for a small seed,
/// 2.3 GB for the 20 000-track one) when it is asked to stop, and `/tmp` is a
/// tmpfs on most machines: `Child::kill()` is SIGKILL, which cannot be caught
/// and left that library to the next launch's sweep. Measured 2026-11-17:
/// 20 of 20 SIGTERM stops left no directory at all.
fn stop(child: &mut Child) {
    const SIGTERM: i32 = 15;
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    // SAFETY: `kill` is handed the pid of a child this struct owns and the
    // platform's own SIGTERM number, and nothing else.
    unsafe {
        kill(child.id() as i32, SIGTERM);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn client(bench: &Bench) -> Api {
    Api::builder(bench.base())
        .session(Arc::new(SessionJar::in_memory()))
        .build()
        .expect("a client")
}

/// **Login + me + 401 round trip against `perf_api.py`.**
///
/// Counted: one real session cookie, one `/api/me` that carries it, one `401`
/// on a client that has none, and a logout that clears it. Plus the typed
/// routes decoding the real server's payloads through `nz-model`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn login_me_and_401_round_trip_against_the_bench() {
    let bench = Bench::start();
    let api = client(&bench);

    // A client with no session is refused, with the SPA's `Unauthorized`.
    let err = api.me().await.expect_err("no session yet");
    assert_eq!(err, ApiError::Unauthorized);
    assert!(!api.has_session());

    let me = api.login("bench", "Bench1").await.expect("a real login");
    assert_eq!(me.user.name, "bench");
    assert!(me.user.admin, "the bench account is an admin");
    assert!(api.has_session(), "the cookie was stored");
    assert_eq!(
        api.owner().as_deref(),
        Some(me.user.id.as_str()),
        "the cache is keyed to this account"
    );

    let again = api.me().await.expect("/api/me with the cookie");
    assert_eq!(again.user.id, me.user.id);

    // Real payloads, through the typed endpoints and D1's types.
    let playlists = api.my_playlists().await.expect("/api/me/playlists");
    assert_eq!(playlists.playlists.len(), 2, "the seed made two playlists");
    let search = api.search("night").await.expect("/api/search");
    assert!(
        search.tracks.len() + search.albums.len() + search.artists.len() <= 50,
        "a bounded answer"
    );
    let ready = api
        .ready(&bench.first_track(), Quality::Opus128)
        .await
        .expect("/api/stream/<id>/ready");
    assert!(
        !ready.ready && ready.bytes.is_none(),
        "nothing is archived on the bench: {ready:?}"
    );
    api.get::<serde_json::Value>(Lane::User, "/version")
        .await
        .expect("/api/version");

    // A SECOND device with no jar is refused by the same server.
    let other = client(&bench);
    assert_eq!(
        other.me().await.expect_err("no cookie on this one"),
        ApiError::Unauthorized
    );

    api.logout().await.expect("logout");
    assert!(!api.has_session(), "logout dropped the cookie");
    assert_eq!(
        api.me().await.expect_err("logged out"),
        ApiError::Unauthorized
    );
}

/// **A route the server has, and one it does not** — D17's "an older server
/// without a route hides the feature".
///
/// `OPTIONS` on a registered route answers 200 (or 405 for another method); an
/// unknown one answers 404. Measured against the real bench: `/genre/status`
/// is `Some(true)`, `/no-such-route-at-all` is `Some(false)`, and the probe
/// costs one request each.
///
/// **Every feature's route is required, with no exemption list.** LN0 landed
/// `Feature::Lan` one wave before `/api/lan/status` existed and named it in a
/// `PENDING_SERVER_ROUTES` list so this bench could stay green; LN1 registers
/// the route and deleted that list, so the loop below is back to requiring
/// `Some(true)` for every `Feature::ALL`. A feature added in the future the same
/// way gets the same one-lot exemption, never a weaker "any answer".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_optional_route_is_told_apart_from_a_missing_one() {
    use nz_api::features::Feature;

    let bench = Bench::start();
    let api = client(&bench);
    api.login("bench", "Bench1").await.expect("a real login");

    for feature in Feature::ALL {
        let path = feature.probe();
        assert_eq!(
            api.has_route(path).await,
            Some(true),
            "{feature:?}: {path} exists on this server"
        );
    }
    // The pair the rule itself rests on, spelled out: a route the server HAS and
    // a route no server has. Read on a fixed route, so the lesson survives
    // every feature being moved in or out of the pending list.
    assert_eq!(api.has_route("/genre/status").await, Some(true));
    assert_eq!(
        api.has_route("/no-such-route-at-all").await,
        Some(false),
        "a route no server has is what an older server answers for a new feature"
    );
}

/// **Two `409`s the server means differently, told apart by its own header.**
///
/// `neezer/webui` answers `409 + Retry-After` when a file is not on disk yet
/// (`_not_archived`) and a bare `409` when the world disagreed with the request
/// (a playlist whose `version` moved, `_playlist_conflict`). The client used to
/// read both as `NotReady`, which made a playlist screen retry a stale order and
/// told the person "pas encore prêt" for a real conflict.
///
/// Measured against the real routes: the seek on an unarchived track answers
/// **409, Retry-After: 2, 1 request** (a `NotReady { retry_after: 2 }`), and the
/// stale-version `PATCH` answers **409 with `{"error":"conflict",
/// "scope":"content"}`, no `Retry-After`, 1 request** (a `Conflict`, not
/// retried because the same body would fetch it again).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_not_ready_and_a_conflict_are_told_apart_by_the_real_server() {
    let bench = Bench::start();
    let api = client(&bench);
    api.login("bench", "Bench1").await.expect("a real login");

    // A seek on a track nothing archived: the server says when to come back.
    let not_ready = api
        .stream(
            Lane::User,
            &bench.first_track(),
            Quality::Opus128,
            Some(1.0),
            None,
        )
        .await
        .expect_err("nothing is archived on the bench");
    assert_eq!(
        not_ready,
        ApiError::NotReady { retry_after: 2 },
        "the server's 409 + Retry-After is a wait, not a verdict"
    );
    assert!(not_ready.retryable());

    // A playlist edited with the version the screen no longer has: the server
    // sends the authoritative counters and no Retry-After.
    let conflict = api
        .edit_playlist(
            &bench.a_playlist(),
            &serde_json::json!({"title": "Suivi"}),
            // The server's own ceiling (`PLAYLIST_MAX_VERSION`): a valid version
            // this playlist cannot have, so the answer is the CAS conflict and
            // not a 400 "invalid version" (which u32::MAX would be).
            Some(2_147_483_647),
        )
        .await
        .expect_err("a stale version is refused");
    match &conflict {
        ApiError::Conflict {
            message,
            code,
            scope,
        } => {
            assert_eq!(message, "conflict", "the server's own label");
            assert_eq!(code, &None, "this refusal carries no machine code");
            assert_eq!(
                scope.as_deref(),
                Some("content"),
                "which half of the playlist moved"
            );
        }
        other => panic!("a stale version is a Conflict, not {other:?}"),
    }
    assert!(
        !conflict.retryable(),
        "re-sending the same body fetches the same 409: the screen re-reads first"
    );
    assert_eq!(conflict.retry_after(), None);
}

/// **`PUT /api/me/accounts/<id>` takes `{"auto": {kind: bool}}`, not a boolean.**
///
/// `my_accounts_options` checks `isinstance(auto, dict)` and then that every key
/// is a `libraryimport.KINDS` entry, and answers `400 auto invalide` otherwise.
/// `nz-api`'s `set_account_auto(id, bool)` posted `{"auto": true}`, so every
/// toggle from Réglages → Sources lost the request and the screen had to build
/// the body itself through `Api::send`.
///
/// Measured against the real route: the typed call answers **200** and the
/// account it returns carries every announced kind at the value asked; the old
/// bare boolean answers **400 `auto invalide`**; a kind left out keeps its saved
/// value.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_accounts_auto_payload_is_the_shape_the_server_reads() {
    let bench = Bench::start();
    let api = client(&bench);
    api.login("bench", "Bench1").await.expect("a real login");

    let accounts = api.my_accounts().await.expect("/api/me/accounts");
    let account = accounts
        .accounts
        .iter()
        .find(|account| account.id == bench.account())
        .unwrap_or_else(|| panic!("the seeded account is not listed: {accounts:?}"));
    assert_eq!(
        account.kinds,
        vec![
            "artists".to_owned(),
            "tracks".to_owned(),
            "albums".to_owned(),
            "playlists".to_owned()
        ],
        "the server's own KIND list, which is what the payload may name"
    );

    // The exact call the settings screen makes.
    let answer = api
        .set_account_auto(&account.id, &account.kinds, true)
        .await
        .expect("the server accepts the kind map");
    let auto = answer["account"]["auto"]
        .as_object()
        .unwrap_or_else(|| panic!("no auto map in {answer}"));
    for kind in &account.kinds {
        assert_eq!(
            auto.get(kind).and_then(|on| on.as_bool()),
            Some(true),
            "{kind}"
        );
    }

    // A kind left out keeps its value: this is what "the account announced its
    // kinds" means for a caller that only touches one.
    let one = vec!["tracks".to_owned()];
    let answer = api
        .set_account_auto(&account.id, &one, false)
        .await
        .expect("one kind is a valid map too");
    let auto = answer["account"]["auto"].as_object().expect("an auto map");
    assert_eq!(auto.get("tracks").and_then(|on| on.as_bool()), Some(false));
    assert_eq!(
        auto.get("artists").and_then(|on| on.as_bool()),
        Some(true),
        "a kind the body does not name is untouched"
    );

    // And the old shape is refused, with the server's own words.
    let legacy = api
        .send::<serde_json::Value, serde_json::Value>(
            Lane::User,
            Method::PUT,
            &format!("/me/accounts/{}", account.id),
            Some(&serde_json::json!({ "auto": true })),
        )
        .await
        .expect_err("the bare boolean is what the server refuses");
    match legacy {
        ApiError::Http {
            status, message, ..
        } => {
            assert_eq!(status, 400);
            assert_eq!(message, "auto invalide");
        }
        other => panic!("expected the server's 400, got {other:?}"),
    }
}

/// **`POST /api/radio/next` reads an OBJECT seed, not a bare string.**
///
/// `neezer/webui#radio_next` does `seed = data.get("seed") or {}` and refuses
/// anything that is not a dict (`400 invalid seed`), then branches on
/// `seed.kind` (`"track"`/`"artist"`); `lib/queryradio.js#radioSeed` builds that
/// object. `nz-api` sent `{"seed": "3000123"}` and its owner could not use the
/// typed call — every radio refill in the app went through `Api::send`.
///
/// Measured here: the typed call with a real track seed answers **200** (an
/// empty list is a fine answer on a bench with nothing archived), and the old
/// bare string is **400 `invalid seed`**, in one request each.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_radio_seed_is_the_object_the_route_reads() {
    let bench = Bench::start();
    let api = client(&bench);
    api.login("bench", "Bench1").await.expect("a real login");

    let seed = RadioSeed {
        kind: "track".to_owned(),
        id: bench.first_track().to_string(),
        artist: None,
    };
    let answer = api
        .radio_next(&seed, Some(4), &[], &[], &[])
        .await
        .expect("the object seed is what the route reads");
    assert!(
        answer.tracks.len() <= 4,
        "the batch size is honoured: {} tracks",
        answer.tracks.len()
    );

    // The shape this function used to send, refused with the server's own words.
    let legacy = api
        .send::<serde_json::Value, serde_json::Value>(
            Lane::User,
            Method::POST,
            "/radio/next",
            Some(&serde_json::json!({ "seed": bench.first_track().to_string(), "n": 4 })),
        )
        .await
        .expect_err("a bare string is what the route refuses");
    match legacy {
        ApiError::Http {
            status, message, ..
        } => {
            assert_eq!(status, 400);
            assert_eq!(message, "invalid seed");
        }
        other => panic!("expected the server's 400, got {other:?}"),
    }
}

/// **`POST /api/podcast/<id>/auto_archive` answers the show PLUS `queued`.**
///
/// `podcast_auto_archive` returns `{**_channel(c), "queued": n}` (`n` from
/// `backfill.archive_show`, which queues the episodes already listed), and the
/// SPA says « N épisodes en file » with it. `nz-api` dropped the body
/// (`send_none`), so a screen could only re-read the whole show to count what it
/// had just queued.
///
/// Measured against `perf_api.py --podcasts`: switching it on answers **200**
/// with the show's own payload (`title`, `episode_count: 8`) AND `queued: 0` —
/// the bench's eight episodes already have a file, so `archive_show` queues
/// none; the COUNT is what a screen prints and what `send_none` used to throw
/// away. The answer is a body, not an empty 204.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_auto_archive_answer_carries_how_many_episodes_it_queued() {
    let bench = Bench::start();
    let api = client(&bench);
    api.login("bench", "Bench1").await.expect("a real login");

    let on = api
        .set_podcast_auto_archive(&bench.a_show(), true)
        .await
        .expect("the switch answers the show");
    let title = on
        .show
        .get("title")
        .and_then(|title| title.as_str())
        .unwrap_or_default();
    assert!(!title.is_empty(), "the show's own payload is kept: {on:?}");
    assert_eq!(
        on.show
            .get("episode_count")
            .and_then(|count| count.as_u64()),
        Some(8),
        "the whole channel payload is kept beside the count: {on:?}"
    );
    assert_eq!(
        on.queued, 0,
        "every bench episode is already on disk, so the switch queues none"
    );

    let off = api
        .set_podcast_auto_archive(&bench.a_show(), false)
        .await
        .expect("switching it off answers too");
    assert_eq!(off.queued, 0, "nothing is queued when the switch is off");
}
