//! The car overlay's data (docs/agents/mobile.md, MB24).
//!
//! Owned by **MB24**: this file holds its own `#[uniffi::export] impl NzCore`
//! block, its own records and its own tests, and edits no other file of the
//! crate. The overlay itself is the old app's, ported under its old class names;
//! what changed is exactly the two seams the note names. This file is the second
//! one: the `__nsCarPlay(kind, id, title, payload)` / `__nsCarFav()` page hooks
//! are gone with the WebView, so the three things they did are here.
//!
//! | The old hook (`webapp/src/lib/car.js`) | Here |
//! |---|---|
//! | `playEntity(kind, id)` — a playlist, an album, an artist | [`NzCore::car_play`] → `PlayerCommand::PlayEntity` |
//! | `startFlow()` — the `flow` kind | [`NzCore::car_play`] → `PlayerCommand::StartFlow` |
//! | `player.playQueue([track], 0, {kind:"track"})` — a searched title | [`NzCore::car_play_track`] → `PlayerCommand::PlayQueue` (the SPA's own JSON payload was that row) |
//! | `playFavorites()` — one read of `/api/me/favorites`, then the queue | [`NzCore::car_play`] with `favorites` → `PlayEntity{kind:"favorites"}`, which `nz-core::player::flow` resolves itself |
//! | `__nsCarFav()` — the current title's heart | [`NzCore::car_toggle_favorite`] (`nz-core::actions#set_track_favorite`) |
//! | `NFNative`/`Net` — the catalogue and its counts | [`NzCore::car_catalogue`] (`nz-core::library`), session cookie and allow-lists in Rust |
//!
//! # Rule 1 on the car
//!
//! A tapped tile or title is the person's own action, so the queue is handed to
//! the player on the spot (`PlayerCommand` runs on the player's worker; nothing
//! here waits for a request) and the catalogue is the ONE screen read the panel
//! makes, on the `User` lane inside `nz-core::library`. Nothing here reads a
//! cover: those go through `Screens::cover_path`, whose CDN allow-list and 300 MB
//! cache already are *Caches*' rule, and whose lane is the visible-first one.
//!
//! # What is deliberately NOT here
//!
//! No trigger, no placement, no speed, no notification: those are the old app's
//! policies, ported byte for byte into `:car`'s Kotlin and held by its JVM tests
//! (`CarVisibilityTest`, `CarForegroundTest`, `CarMotionTest`, …). This file
//! answers questions about the LIBRARY and the PLAYER, and invents no rule.

use nz_core::library as library_core;

use crate::core::NzCore;
use crate::errors::NzError;
use crate::player::{PlayerCommand, PlayContext, TrackRef};

/// The longest id or key `lib/car.js` accepted; anything longer is refused
/// before it becomes a request (`KINDS`' own bounds kept).
const MAX_ID: usize = 128;

/// The kinds [`NzCore::car_play`] accepts — `lib/car.js`'s `KINDS`, minus
/// `track`, which needs its ROW and therefore [`NzCore::car_play_track`].
pub const CAR_ENTITY_KINDS: [&str; 5] = ["flow", "favorites", "playlist", "album", "artist"];

/// One row of the car's playlist panel (`CarPlaylistsView`'s tiles, and the
/// settings screen's catalogue).
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct CarPlaylist {
    /// The playlist's UUID — what a pin stores and what plays it.
    pub id: String,
    /// The provider's id for a Deezer playlist: a pin copied from a URL may
    /// carry this one instead, and `CarPlaylistSource.find` matches either.
    pub deezer_id: Option<String>,
    pub title: String,
    /// The server's count; 0 when it did not say.
    pub nb_tracks: u32,
    /// A cover URL or path (`/api/cover/<id>`, a `*.dzcdn.net` address).
    pub cover: Option<String>,
}

/// What the panel and the settings screen draw, in one answer.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct CarCatalogue {
    pub playlists: Vec<CarPlaylist>,
    /// The Favoris tile's counter: the track stars of `/api/me/favorite-ids`.
    /// `None` when that light read failed — the catalogue still paints, and the
    /// tile says "tes titres aimés" instead of a number (the old app's rule).
    pub favorites: Option<u32>,
}

impl From<library_core::PlaylistCard> for CarPlaylist {
    fn from(card: library_core::PlaylistCard) -> Self {
        CarPlaylist {
            id: card.id,
            deezer_id: card.deezer_id,
            title: card.title,
            nb_tracks: card.tracks,
            cover: card.cover,
        }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl NzCore {
    /// The car's catalogue: the person's playlists and the Favoris tile's count.
    ///
    /// **Two requests, never more**, and they are the routes the old app already
    /// read: `GET /api/me/playlists` and the light `GET /api/me/favorite-ids`
    /// (never `/api/me/favorites`, which is a whole hydrated list — 2 MB against
    /// 45 KB for 4 000 stars). The count is asked **after** the catalogue and its
    /// failure is swallowed: a missing counter must not blank the panel.
    pub async fn car_catalogue(&self) -> Result<CarCatalogue, NzError> {
        let core = self.core();
        let tree = core
            .library_playlists()
            .await
            .map_err(NzError::from)?;
        let favorites = core
            .favorite_ids()
            .await
            .ok()
            .map(|ids| ids.tracks.len() as u32);
        Ok(CarCatalogue {
            playlists: tree.playlists.into_iter().map(CarPlaylist::from).collect(),
            favorites,
        })
    }

    /// Launch what an overlay tile named: a Flow, the person's favourites, or a
    /// playlist/album/artist by id. `true` means the player took the command —
    /// the SPA's own answer, and what makes the toast tell the truth.
    ///
    /// Nothing here waits for a request: `PlayEntity` and `StartFlow` are queued
    /// on the player's own worker, which is rule 1 (the tapped thing plays now).
    /// An unknown kind, an empty or absurd id, and a player that cannot start all
    /// answer `false`, which the service turns into "Ouvre Neezer…".
    pub fn car_play(&self, kind: String, id: String) -> bool {
        let kind = kind.trim().to_ascii_lowercase();
        if !CAR_ENTITY_KINDS.contains(&kind.as_str()) {
            return false;
        }
        let id = id.trim();
        if id.len() > MAX_ID {
            return false;
        }
        // `lib/car.js`'s own gate: a signed-out app would answer 401 on the
        // resolve, so say `false` before the request and let the card say
        // "Ouvre Neezer…".
        if !self.car_signed_in() {
            return false;
        }
        // `flow` is a STYLE, and none is the person's default: an empty style is
        // what `StartFlow` with no style means.
        if kind == "flow" {
            self.player().command(PlayerCommand::StartFlow { style: None }, 0);
            return true;
        }
        if id.is_empty() {
            return false;
        }
        self.player().command(
            PlayerCommand::PlayEntity {
                kind,
                id: id.to_owned(),
                shuffle: false,
            },
            0,
        );
        true
    }

    /// Play ONE title the car's own search found. The SPA carried the track's
    /// JSON as `__nsCarPlay`'s payload and queued it as a one-row list; the row
    /// crosses as a [`TrackRef`], so the queue is the same thing without a JSON
    /// step (and the context stays `track`, as `lib/car.js` wrote it).
    pub fn car_play_track(&self, track: TrackRef) -> bool {
        let key = track.key.trim().to_owned();
        let id = track.id.trim().to_owned();
        if key.is_empty() || key.len() > MAX_ID || id.len() > MAX_ID {
            return false;
        }
        if !self.car_signed_in() {
            return false;
        }
        let context_id = if id.is_empty() { key } else { id };
        self.player().command(
            PlayerCommand::PlayQueue {
                tracks: vec![track],
                start: 0,
                context: Some(PlayContext {
                    kind: "track".to_owned(),
                    id: context_id,
                }),
            },
            0,
        );
        true
    }

    /// The heart's state for the title that is playing: `None` when nothing is
    /// (`__nsCarFav` existed only while a track was loaded, for the same reason).
    ///
    /// A read of the player's PUSHED snapshot, never a request, so the card can
    /// call it on every repaint.
    pub fn car_favorite(&self) -> Option<bool> {
        self.player().snapshot().current.map(|now| now.liked)
    }

    /// Toggle the current title's star, and answer its new state — what
    /// `__nsCarFav()` returned the moment the person tapped the heart.
    ///
    /// `Ok(None)` is "nothing is playing"; `Err` is the cause in French
    /// (`nz_error_text`), for the toast the old hook's `false` produced. The
    /// write is MB10's own (`nz-core::actions#set_track_favorite`: the rows flip
    /// before the request, a refusal rolls them back).
    pub async fn car_toggle_favorite(&self) -> Result<Option<bool>, NzError> {
        let Some(now) = self.player().snapshot().current else {
            return Ok(None);
        };
        if now.key.is_empty() {
            return Ok(None);
        }
        let next = !now.liked;
        self.core()
            .set_track_favorite(&now.key, next)
            .await
            .map_err(NzError::from)?;
        Ok(Some(next))
    }
}

impl NzCore {
    /// Whether this device holds a session: the JAR has a cookie for the
    /// configured origin.
    ///
    /// It is deliberately NOT `me().is_some()`: a process the quick-settings
    /// tile starts (the person taps the car mode with no activity, no screen)
    /// never runs `restore()`, so `me` stays `None` while the cookie the last
    /// launch saved is right there — and the first emulator run of this lot
    /// answered "Ouvre Neezer pour lancer la musique" for a signed-in app.
    /// `has_session` is the SPA's own `isSignedIn` (`get(user)`), which is what
    /// `playFromCar` refused on.
    fn car_signed_in(&self) -> bool {
        self.core()
            .api()
            .map(|api| api.has_session())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CoreConfig;
    use nz_core::library::PlaylistCard;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    /// What the fake server was asked, in order.
    type Log = Arc<Mutex<Vec<String>>>;

    /// A loopback server that answers the four routes the car reads, and counts
    /// them. A thread per connection; a test reads the log after its `await`
    /// returned, never on a clock (worker rule 6).
    struct Server {
        base: String,
        log: Log,
        stop: Arc<std::sync::atomic::AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
        refused: Arc<Mutex<Vec<String>>>,
    }

    impl Server {
        fn start() -> Server {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let base = format!("http://{}", listener.local_addr().expect("addr"));
            let log: Log = Arc::new(Mutex::new(Vec::new()));
            let refused: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let shared = Arc::clone(&log);
            let broken = Arc::clone(&refused);
            let stopping = Arc::clone(&stop);
            let thread = std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stopping.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let log = Arc::clone(&shared);
                    let broken = Arc::clone(&broken);
                    std::thread::spawn(move || answer(stream, log, broken));
                }
            });
            Server {
                base,
                log,
                stop,
                thread: Some(thread),
                refused,
            }
        }

        /// The routes asked, `"GET /api/…"`, in arrival order.
        fn routes(&self) -> Vec<String> {
            self.log.lock().expect("log").clone()
        }

        fn forget(&self) {
            self.log.lock().expect("log").clear();
        }

        /// Make the Favoris count fail with a 500 from now on.
        fn break_favorites(&self) {
            self.refused
                .lock()
                .expect("refused")
                .push("/api/me/favorite-ids".to_owned());
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect(self.base.trim_start_matches("http://"));
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn answer(mut stream: TcpStream, log: Log, broken: Arc<Mutex<Vec<String>>>) {
        let mut buffer = [0u8; 4096];
        let Ok(read) = stream.read(&mut buffer) else {
            return;
        };
        let request = String::from_utf8_lossy(&buffer[..read]).to_string();
        let mut lines = request.lines();
        let first = lines.next().unwrap_or_default();
        let mut parts = first.split_whitespace();
        let method = parts.next().unwrap_or_default().to_owned();
        let path = parts.next().unwrap_or_default().to_owned();
        let route = path.split('?').next().unwrap_or(&path).to_owned();

        // `nz-api` sends `/api/…` on the wire; the log names it as the person
        // would read it in a journal.
        log.lock()
            .expect("log")
            .push(format!("{method} {route}"));

        // The library's routes are `login_required` on the real server: a
        // request without the session cookie is a 401, which is the cause the
        // car tells apart from "server down".
        let signed_in = request.lines().any(|line| {
            let lower = line.to_ascii_lowercase();
            lower.starts_with("cookie:") && lower.contains("session=abc123")
        });

        let refuse = broken.lock().expect("refused").contains(&route);
        let (status, body, cookie) = if refuse {
            ("500 Internal Server Error", "{}".to_owned(), false)
        } else if route.starts_with("/api/me/") && !signed_in {
            ("401 Unauthorized", "{}".to_owned(), false)
        } else {
            match (method.as_str(), route.as_str()) {
                ("GET", "/api/version") => {
                    ("200 OK", r#"{"version":"1.9.9"}"#.to_owned(), false)
                }
                ("POST", "/api/login") => (
                    "200 OK",
                    r#"{"user":{"id":"u1","name":"Nadia","admin":false}}"#.to_owned(),
                    true,
                ),
                ("GET", "/api/me/playlists") => (
                    "200 OK",
                    r#"{"folders":[],"playlists":[
                        {"id":"0b7d5f3a-9c2e-4b41-8d6f-1a2b3c4d5e6f","title":"Voiture",
                         "nb_tracks":42,"cover":"/api/cover/abc","deezer_id":"908622995"},
                        {"id":"6c9a1e2d-0000-4000-8000-123456789abc","title":"Route",
                         "nb_tracks":7}
                    ]}"#
                    .to_owned(),
                    false,
                ),
                ("GET", "/api/me/favorite-ids") => (
                    "200 OK",
                    r#"{"ids":["a","b","c"],"albums":[],"artists":[],"playlists":[]}"#.to_owned(),
                    false,
                ),
                _ => ("404 Not Found", "{}".to_owned(), false),
            }
        };
        let cookie = if cookie {
            "Set-Cookie: session=abc123; Path=/\r\n"
        } else {
            ""
        };
        let answer = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{cookie}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(answer.as_bytes());
        let _ = stream.flush();
    }

    fn dirs(tag: &str) -> std::path::PathBuf {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-ffi-car-tests")
            .join(format!("{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn core(tag: &str) -> Arc<NzCore> {
        let path = dirs(tag);
        NzCore::new(CoreConfig {
            data_dir: path.join("data").display().to_string(),
            cache_dir: path.join("cache").display().to_string(),
            app_version: "0.0.0".into(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".into(),
        })
        .expect("a core")
    }

    /// Sets the server, and nothing else: the state of a process the tile
    /// started (a server configured by a previous launch, no screen, no
    /// `restore()`).
    async fn core_without_restore(server: &Server, tag: &str) -> Arc<NzCore> {
        let core = core(tag);
        core.set_server(server.base.clone()).await.expect("server");
        server.forget();
        core
    }

    /// Sets the server and signs in: the two calls `NzBridge` and MB4 make, so
    /// the catalogue is read with exactly the session the app has.
    async fn signed_in(server: &Server, tag: &str) -> Arc<NzCore> {
        let core = core(tag);
        core.set_server(server.base.clone()).await.expect("server");
        core.login("nadia".into(), "secret".into())
            .await
            .expect("login");
        server.forget();
        core
    }

    /// The catalogue the panel draws, in TWO requests (never more), with the
    /// Favoris counter read from the light route — the numbers the old app
    /// measured (15 ms / 45 KB for 4 000 stars against 99 ms / 2 MB).
    #[tokio::test]
    async fn the_catalogue_costs_two_requests_and_carries_the_star_count() {
        let server = Server::start();
        let core = signed_in(&server, "catalogue").await;

        let catalogue = core.car_catalogue().await.expect("the catalogue");
        assert_eq!(
            server.routes(),
            ["GET /api/me/playlists", "GET /api/me/favorite-ids"],
            "the panel reads the catalogue and the light count, in that order"
        );
        assert_eq!(catalogue.playlists.len(), 2);
        assert_eq!(catalogue.playlists[0].id, "0b7d5f3a-9c2e-4b41-8d6f-1a2b3c4d5e6f");
        assert_eq!(catalogue.playlists[0].title, "Voiture");
        assert_eq!(catalogue.playlists[0].nb_tracks, 42);
        assert_eq!(catalogue.playlists[0].cover.as_deref(), Some("/api/cover/abc"));
        assert_eq!(
            catalogue.playlists[0].deezer_id.as_deref(),
            Some("908622995"),
            "a pin copied from a Deezer URL matches this id"
        );
        assert_eq!(catalogue.playlists[1].nb_tracks, 7);
        assert_eq!(catalogue.favorites, Some(3));
        // Counted by the core's own meter, not by the fan: one screen read.
        println!(
            "MEASURE car catalogue cold: requests={} playlists={} favorites={:?}",
            server.routes().len(),
            catalogue.playlists.len(),
            catalogue.favorites
        );
        core.shutdown();
    }

    /// A count route that fails leaves the counter UNKNOWN and the catalogue
    /// whole: a decoration never blanks the panel (the old app's rule).
    #[tokio::test]
    async fn a_failed_count_leaves_the_catalogue_whole() {
        let server = Server::start();
        let core = signed_in(&server, "count-fails").await;
        server.break_favorites();

        let catalogue = core.car_catalogue().await.expect("the catalogue");
        assert_eq!(catalogue.playlists.len(), 2);
        assert_eq!(catalogue.favorites, None, "unknown, never a wrong 0");
        core.shutdown();
    }

    /// No session, no catalogue: the panel paints its pins and says « Session
    /// absente » instead of pretending the server is down (a 401 is not a 5xx).
    #[tokio::test]
    async fn a_missing_session_is_its_own_cause() {
        let server = Server::start();
        let core = core("no-session");
        core.set_server(server.base.clone()).await.expect("server");
        server.forget();
        let error = core.car_catalogue().await.expect_err("no session");
        assert!(
            matches!(error, NzError::Session),
            "a 401 must stay a session cause: {error:?}"
        );
        core.shutdown();
    }

    /// The catalogue is asked on the ORIGIN's own root: the old app's `/app`
    /// path can never end up in front of `/api`, and a reverse-proxied address
    /// is refused at the door rather than silently mis-asked. (The Kotlin
    /// `endpointFor` test this replaces lived in `CarPlaylistSourceTest`.)
    #[tokio::test]
    async fn the_catalogue_never_carries_the_app_path() {
        let server = Server::start();
        let core = signed_in(&server, "app-path").await;
        let _ = core.car_catalogue().await.expect("the catalogue");
        assert!(
            server
                .routes()
                .iter()
                .any(|route| route == "GET /api/me/playlists")
        );
        assert!(
            server.routes().iter().all(|route| !route.contains("/app")),
            "nothing under /app is ever asked"
        );

        // A server address carrying the SPA's path is refused by the core
        // itself (`Origin::parse`), so the car cannot be the one place that
        // quietly accepted it.
        let error = core
            .set_server(format!("{}/app/", server.base))
            .await
            .expect_err("a path is not an origin");
        assert!(matches!(error, NzError::Invalid { .. }), "{error:?}");
        core.shutdown();
    }

    /// What `car_play` answers, kind by kind: the SPA's `KINDS`, its bounds and
    /// its refusals (an unknown kind, an empty id, an id past 128 characters,
    /// and a signed-out app — `lib/car.js`'s own gate).
    #[test]
    fn only_the_kinds_the_page_accepted_are_launched() {
        let core = core("kinds");
        // No person: every launch is refused before a request, exactly as
        // `playFromCar` refused one without a session.
        assert!(!core.car_play("playlist".into(), "p1".into()));
        assert!(!core.car_play("podcast".into(), "x".into()));
        assert!(!core.car_play("track".into(), "x".into()), "a track needs its row");
        assert!(!core.car_play("flow".into(), String::new()));
        assert!(!core.car_play("playlist".into(), "   ".into()));
        assert!(!core.car_play("playlist".into(), "x".repeat(129)));
        assert!(!core.car_play_track(TrackRef::default()));
        core.shutdown();
    }

    /// The launch gate is the SESSION, not the person: a process the tile
    /// started (no activity, so no `restore()`) holds the cookie and no `me`,
    /// and it must still play. Measured on the emulator: the first run answered
    /// "Ouvre Neezer pour lancer la musique" for a signed-in app.
    #[tokio::test]
    async fn a_process_that_never_restored_still_plays() {
        let server = Server::start();
        let core = core_without_restore(&server, "tile-only").await;
        assert_eq!(core.core().me(), None, "nobody restored: no person in memory");
        // The cookie the previous launch saved is in the jar.
        let seeded = core.core().jar().seed(
            &server.base,
            "session=abc123",
        );
        assert_eq!(seeded, 1, "one cookie seeded");
        assert!(
            core.car_play("playlist".into(), "34b37fae-19e3-48b7-a976-e6496d9bf5c9".into()),
            "a signed-in app plays even when no screen ever loaded"
        );
        // No `shutdown()` here: this test is the one that BUILDS the player, and
        // shutting an engine down joins its threads — blocking that is refused
        // inside an async test (`Cannot drop a runtime in a context where
        // blocking is not allowed`). The process ends with the test.
    }

    /// The heart is the player's own answer, and its absence is the SPA's own
    /// condition: `__nsCarFav` existed only while a title was loaded.
    #[test]
    fn the_heart_follows_the_current_title() {
        let core = core("heart");
        // Nothing plays: no track, so no heart (never "not a favourite").
        assert_eq!(core.car_favorite(), None);
        assert_eq!(
            core.player().snapshot().current,
            None,
            "a fresh core has no current title"
        );
        core.shutdown();
    }

    /// The conversion the panel reads, with the facts a tile states.
    #[test]
    fn a_playlist_card_becomes_a_car_row() {
        let row = CarPlaylist::from(PlaylistCard {
            id: "p1".to_owned(),
            deezer_id: Some("42".to_owned()),
            title: "Route".to_owned(),
            tracks: 12,
            cover: Some("/api/cover/p1".to_owned()),
            ..PlaylistCard::default()
        });
        assert_eq!(row.id, "p1");
        assert_eq!(row.deezer_id.as_deref(), Some("42"));
        assert_eq!(row.nb_tracks, 12);
        assert_eq!(row.cover.as_deref(), Some("/api/cover/p1"));
    }
}
