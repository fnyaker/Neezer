//! The object graph and everything a screen asks the core itself
//! (docs/agents/mobile.md, "The FFI boundary"). Owned by **MB2**: it fills the
//! bodies and adds the JNI `Platform_init` export the Android trust store
//! needs. MB0 declared the types and the signatures, and made `nz_version()`
//! work for real — that one call is what proves the `.so` is loaded.
//!
//! # What this file owns, and what it does not
//!
//! - The **conversion** from `nz-core`'s types to the records Kotlin sees, in
//!   both directions, so no other file spells a field twice.
//! - The **listeners**: `SessionListener` is UniFFI's callback interface, and a
//!   `SessionSink` adapter hands it what the core pushes. It runs on the
//!   runtime's worker (never Kotlin's main thread), and the Kotlin side hands it
//!   to a Flow IMMEDIATELY and returns — the core holds no lock while calling it.
//! - The **JNI export** for `rustls-platform-verifier`: `init_with_env` must run
//!   before the first handshake or every TLS request fails, and the `Context`
//!   only exists on the Kotlin side.
//!
//! Everything else (`cover_path`, the list windows, the link parser) lives in
//! its own file, so a later lot adds its own rather than editing this one.

use std::sync::Arc;

use nz_core::core::Me as MeInner;
use nz_core::core::{Core, CoreConfig as CoreConfigInner, SessionEvent, SessionSink};
use nz_core::core::{Memory, NetworkState as NetworkStateInner};
use nz_core::lists::Row;

use crate::errors::NzError;
use crate::links::AppLink;
use crate::lists::ListHandle;
use crate::offline::OfflineApi;
use crate::party::PartyApi;
use crate::player::PlayerApi;
use crate::remote::RemoteApi;
use crate::settings::SettingsApi;
use crate::viz::VizApi;

/// What `:bridge` hands the core once, at `Application.onCreate`. Cheap: it
/// opens SQLite and starts the runtime; it never touches the network.
#[derive(uniffi::Record, Debug, Clone)]
pub struct CoreConfig {
    /// `filesDir/neezer` — the downloads and the one SQLite file, never evicted.
    pub data_dir: String,
    /// `cacheDir/neezer` — covers and the play cache, which the system may clear.
    pub cache_dir: String,
    /// `BuildConfig.VERSION_NAME`, shown in Aide beside `nz_version()`.
    pub app_version: String,
    /// `BuildConfig.VERSION_CODE`; the migration checks it against what the old
    /// app wrote (rule 7).
    pub version_code: u64,
    /// `Build.VERSION.SDK_INT`, for the AAudio (API >= 27) / AudioTrack (26) split.
    pub sdk_int: u32,
    /// `Build.MODEL`, for the diagnostic log and the remote-control host name.
    pub device_name: String,
}

impl From<CoreConfig> for CoreConfigInner {
    fn from(config: CoreConfig) -> Self {
        CoreConfigInner {
            data_dir: config.data_dir.into(),
            cache_dir: config.cache_dir.into(),
            app_version: config.app_version,
            version_code: config.version_code,
            sdk_int: config.sdk_int,
            device_name: config.device_name,
        }
    }
}

/// The server this app is configured for.
#[derive(uniffi::Record, Debug, Clone)]
pub struct ServerInfo {
    /// Scheme, host and port, the ONE definition of "our server".
    pub origin: String,
    /// The pinned leaf's SHA-256 (colon-separated hex), when the person pinned one.
    pub pinned_sha256: Option<String>,
    /// What `GET /api/version` answered, so Aide can name the server's version.
    pub version: String,
}

impl From<nz_core::core::ServerInfo> for ServerInfo {
    fn from(info: nz_core::core::ServerInfo) -> Self {
        ServerInfo {
            origin: info.origin.as_str(),
            pinned_sha256: info.pinned_sha256,
            version: info.version,
        }
    }
}

/// A certificate a handshake presented, learnt without sending a request
/// (`Ssl.kt`'s rule, SC3's `probe_certificate`).
#[derive(uniffi::Record, Debug, Clone)]
pub struct CertInfo {
    /// The leaf's SHA-256, colon-separated hex — the value the pin stores.
    pub sha256: String,
    pub subject: String,
    pub issuer: String,
    /// ISO-8601, as the certificate says it.
    pub not_after: String,
    /// Whether Android's own trust store would accept this chain.
    pub system_trusted: bool,
}

impl From<nz_core::core::CertificateInfo> for CertInfo {
    fn from(info: nz_core::core::CertificateInfo) -> Self {
        CertInfo {
            sha256: info.sha256,
            subject: info.subject,
            issuer: info.issuer,
            not_after: info.not_after,
            system_trusted: info.system_trusted,
        }
    }
}

/// The signed-in person, as the shell draws it.
#[derive(uniffi::Record, Debug, Clone)]
pub struct Me {
    pub id: String,
    pub name: String,
    /// The administrator sections of Réglages are only shown when true.
    pub admin: bool,
    pub avatar: Option<String>,
}

impl From<MeInner> for Me {
    fn from(me: MeInner) -> Self {
        Me {
            id: me.user.id.as_str().to_owned(),
            name: me.user.name,
            admin: me.user.admin,
            // `/api/me` carries no avatar: the shell paints the person's initial
            // until a route provides one (MB16).
            avatar: None,
        }
    }
}

/// What one screen cost on the link (rule 3, "Counted, not timed").
#[derive(uniffi::Record, Debug, Clone)]
pub struct NetStats {
    /// The screen's tag (`home`, `search`, …), or `background`.
    pub tag: String,
    pub requests: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

impl From<nz_core::core::TagStats> for NetStats {
    fn from(stats: nz_core::core::TagStats) -> Self {
        NetStats {
            tag: stats.tag,
            requests: stats.requests,
            bytes_in: stats.bytes_in,
            bytes_out: stats.bytes_out,
        }
    }
}

/// Counted bytes of the core's own stores, never a process RSS
/// (docs/agents/mobile.md, *Measures*, and worker rule 4).
#[derive(uniffi::Record, Debug, Clone, Copy)]
pub struct MemoryReport {
    pub queue_bytes: u64,
    pub lists_bytes: u64,
    pub api_cache_mem_bytes: u64,
}

impl From<Memory> for MemoryReport {
    fn from(memory: Memory) -> Self {
        MemoryReport {
            queue_bytes: memory.queue_bytes,
            lists_bytes: memory.lists_bytes,
            api_cache_mem_bytes: memory.api_cache_mem_bytes,
        }
    }
}

/// What `ConnectivityManager` says. `None` flips `nz-api` offline AT ONCE, so
/// no request waits for a timeout to learn it.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkState {
    None,
    Wifi { metered: bool },
    Cellular { metered: bool },
    Other { metered: bool },
}

impl From<NetworkState> for NetworkStateInner {
    fn from(state: NetworkState) -> Self {
        match state {
            NetworkState::None => NetworkStateInner::None,
            NetworkState::Wifi { metered } => NetworkStateInner::Wifi { metered },
            NetworkState::Cellular { metered } => NetworkStateInner::Cellular { metered },
            NetworkState::Other { metered } => NetworkStateInner::Other { metered },
        }
    }
}

/// The end of a session, pushed to the shell. Kotlin hands it to a Flow and
/// returns at once: this runs on a Rust thread.
#[uniffi::export(callback_interface)]
pub trait SessionListener: Send + Sync {
    /// `reason` is the French sentence to show, never a code.
    fn on_session_ended(&self, reason: String);
    /// The signed-in person changed (restored after a launch, or logged out).
    fn on_me_changed(&self, me: Option<Me>);
}

/// The core's events, carried to the Kotlin listener.
struct SessionSinkAdapter(Arc<dyn SessionListener>);

impl SessionSink for SessionSinkAdapter {
    fn publish(&self, event: SessionEvent) {
        match event {
            SessionEvent::Ended(error) => {
                let error = NzError::from(error);
                self.0
                    .on_session_ended(crate::errors::nz_error_text(&error));
            }
            SessionEvent::Me(me) => self.0.on_me_changed(me.map(Me::from)),
        }
    }
}

/// The read-only screen data: one `#[uniffi::export] impl Screens` block per
/// feature lot, in that lot's own file. MB0 declared the object here so every
/// lot can add its block without editing a shared file.
///
/// It carries the one core the process has, so a lot's method reaches the client
/// and the caches through `self.core()` and never through a second global.
#[derive(uniffi::Object)]
pub struct Screens {
    core: Arc<Core>,
}

impl Screens {
    /// The core behind this view — for a lot's own `impl Screens` block.
    pub(crate) fn core(&self) -> &Arc<Core> {
        &self.core
    }
}

/// The app's core, once, built by `NzBridge.install`.
#[derive(uniffi::Object)]
pub struct NzCore {
    core: Arc<Core>,
    screens: Arc<Screens>,
    /// The ONE player of the process (MB3c's measurement). `player()` used to
    /// build a fresh `PlayerApi` per call, and a `PlayerApi` owns a `Player`, an
    /// engine and the engine's sink: the media service opened AAudio on ITS
    /// instance while the caller's queue lived in another — 40 stream requests
    /// served, `position()` still 0, nothing audible. "Built once, by
    /// `NzCore::player()`" (MB3b) is a promise this field keeps.
    player: std::sync::OnceLock<Arc<PlayerApi>>,
}

impl NzCore {
    /// The core, for a lot that needs it outside an exported method.
    pub(crate) fn core(&self) -> &Arc<Core> {
        &self.core
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl NzCore {
    /// Builds the graph. Cheap: opens SQLite (its own migrations included),
    /// reads the server and the person it remembers, and touches no network.
    #[uniffi::constructor]
    pub fn new(cfg: CoreConfig) -> Result<Arc<Self>, NzError> {
        let core = Arc::new(Core::new(cfg.into()).map_err(NzError::from)?);
        Ok(Arc::new(NzCore {
            screens: Arc::new(Screens {
                core: Arc::clone(&core),
            }),
            core,
            player: std::sync::OnceLock::new(),
        }))
    }

    /// The configured origin and its pin, or `None` before setup.
    pub fn server(&self) -> Option<ServerInfo> {
        self.core.server().map(ServerInfo::from)
    }

    /// Saves the server and checks it answers `GET /api/version`.
    pub async fn set_server(&self, url: String) -> Result<ServerInfo, NzError> {
        self.core
            .set_server(&url)
            .await
            .map(ServerInfo::from)
            .map_err(NzError::from)
    }

    /// Learns the certificate with a handshake that sends nothing.
    pub async fn probe_certificate(&self, url: String) -> Result<CertInfo, NzError> {
        self.core
            .probe_certificate(&url)
            .await
            .map(CertInfo::from)
            .map_err(NzError::from)
    }

    /// Pins one leaf for one origin (TOFU), or clears it with an empty hash.
    pub fn pin_certificate(&self, origin: String, sha256: String) -> Result<(), NzError> {
        self.core
            .pin_certificate(&origin, &sha256)
            .map_err(NzError::from)
    }

    pub async fn login(&self, username: String, password: String) -> Result<Me, NzError> {
        self.core
            .login(&username, &password)
            .await
            .map(Me::from)
            .map_err(NzError::from)
    }

    /// Restores the session cookie from `filesDir/neezer/session.json`.
    pub async fn restore(&self) -> Result<Option<Me>, NzError> {
        self.core
            .restore()
            .await
            .map(|me| me.map(Me::from))
            .map_err(NzError::from)
    }

    pub async fn logout(&self) -> Result<(), NzError> {
        self.core.logout().await.map_err(NzError::from)
    }

    /// `ConnectivityManager.registerDefaultNetworkCallback`'s answer.
    pub fn set_network(&self, net: NetworkState) {
        self.core.set_network(net.into());
    }

    /// `ProcessLifecycleOwner`'s answer: prefetch only while somebody is looking.
    pub fn set_foreground(&self, visible: bool) {
        self.core.set_foreground(visible);
    }

    pub fn subscribe_session(&self, listener: Box<dyn SessionListener>) {
        self.core
            .subscribe_session(Arc::new(SessionSinkAdapter(Arc::from(listener))));
    }

    /// Requests and bytes per screen tag, cold and warm (rule 3).
    pub fn stats(&self) -> Vec<NetStats> {
        self.core.stats().into_iter().map(NetStats::from).collect()
    }

    /// Counted bytes of the core's own stores (worker rule 4).
    pub fn memory(&self) -> MemoryReport {
        MemoryReport::from(self.core.memory())
    }

    pub fn screens(&self) -> Arc<Screens> {
        Arc::clone(&self.screens)
    }

    pub fn player(&self) -> Arc<PlayerApi> {
        // MB3b: the player needs the core's client, cache and lists, so it is
        // built over it. `PlayerApi::open` is infallible by contract — a runtime
        // that cannot start answers empty values with `error` set, never a
        // panic on a click.
        //
        // ONE per process (MB3c): `PlayerApi` owns the `Player`, its engine and
        // the engine's ONE sink, and the media service, the screens and the
        // tests must all speak to the same one.
        Arc::clone(
            self.player
                .get_or_init(|| PlayerApi::open(Arc::clone(&self.core))),
        )
    }

    pub fn offline(&self) -> Arc<OfflineApi> {
        Arc::new(OfflineApi)
    }

    pub fn remote(&self) -> Arc<RemoteApi> {
        Arc::new(RemoteApi)
    }

    pub fn party(&self) -> Arc<PartyApi> {
        Arc::new(PartyApi)
    }

    pub fn viz(&self) -> Arc<VizApi> {
        Arc::new(VizApi)
    }

    pub fn settings(&self) -> Arc<SettingsApi> {
        // MB16: the settings need the core's store, client and meter, so they
        // are built over it (the same one-line seam MB3b's `player()` used).
        Arc::new(SettingsApi::open(Arc::clone(&self.core)))
    }

    /// Opens an EMPTY list over `keys` (rule 4). A lot that has rows in hand
    /// asks [`NzCore::open_track_list`]; a screen that only needs the keys (a
    /// queue editor dragging placeholders) asks this.
    pub fn open_list(&self, keys: Vec<String>) -> Arc<ListHandle> {
        crate::lists::open_empty(self.core(), keys)
    }

    /// Opens a windowed list over `rows` — how a screen draws 4 000 or 8 000
    /// rows without copying them across the boundary.
    pub fn open_track_list(&self, rows: Vec<crate::lists::TrackRow>) -> Arc<ListHandle> {
        let rows: Vec<Row> = rows.into_iter().map(Row::from).collect();
        crate::lists::open_rows(self.core(), rows)
    }

    /// Joins every thread that borrows the runtime, then stops it. Android kills
    /// the process instead, so nothing correct may depend on this running.
    pub fn shutdown(&self) {
        // The player is this core's child: its worker, event and net threads
        // borrow the engine and a runtime of their own, and none of them may
        // outlive the process' exit path (worker rule 5). It is only shut down if
        // somebody ever asked for it — `OnceLock::get` does not build one.
        if let Some(player) = self.player.get() {
            player.shutdown();
        }
        self.core.shutdown();
    }
}

/// Resolves an app link (`/app/#/...`, `neezer://...`, a Deezer or Spotify
/// URL) into a route. MB2 owns the rule; the core does the parsing.
#[uniffi::export]
pub fn resolve_link(route: String) -> Option<AppLink> {
    nz_core::links::parse(&route).map(AppLink::from)
}

/// The Rust version shown in Aide. **The one call that really works in MB0**:
/// it proves JNA found `libnz_ffi.so` and its symbols.
#[uniffi::export]
pub fn nz_version() -> String {
    nz_core::version()
}

// --- the Android trust store -------------------------------------------------

/// `rustls-platform-verifier`'s Android side needs the process' `Context`
/// before the first handshake: without it, every TLS request fails, including
/// the login (`docs/agents/mobile.md`, *Pitfalls*). Kotlin calls
/// `Platform.init(this)` FIRST in `NzBridge.install`, before anything else.
///
/// The export's name is JNI's own mangling of
/// `fr.neezer.mobile.bridge.Platform.init(Context)`.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_fr_neezer_mobile_bridge_Platform_init<'local>(
    mut env: jni::EnvUnowned<'local>,
    _self: jni::objects::JObject<'local>,
    context: jni::objects::JObject<'local>,
) {
    use jni::errors::ThrowRuntimeExAndDefault;
    let outcome = env.with_env(|env| -> jni::errors::Result<()> {
        rustls_platform_verifier::android::init_with_env(env, context)
    });
    let resolved: () = outcome.resolve::<ThrowRuntimeExAndDefault>();
    let _ = resolved;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// One throwaway data + cache pair, removed on drop: tiny by nature.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let number = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.subsec_nanos())
                .unwrap_or(0);
            let path =
                std::env::temp_dir().join(format!("nz-ffi-{}-{tag}-{number}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("data")).expect("data");
            std::fs::create_dir_all(path.join("cache")).expect("cache");
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn config(name: &str) -> (TempDir, CoreConfig) {
        let dir = TempDir::new(name);
        let config = CoreConfig {
            data_dir: dir.0.join("data").display().to_string(),
            cache_dir: dir.0.join("cache").display().to_string(),
            app_version: "0.0.0".into(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".into(),
        };
        (dir, config)
    }

    /// This process's live "nz-player" threads, as thread ids (Linux `/proc`).
    ///
    /// A SET OF IDS and not a count: `cargo test` runs this crate's tests in one
    /// process, so another test's player is alive at the same time. A test owns
    /// exactly the ids that appeared when IT built its player, and those are the
    /// only ones it may expect to disappear.
    fn player_thread_ids() -> std::collections::BTreeSet<u64> {
        std::fs::read_dir("/proc/self/task")
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| {
                        let name = std::fs::read_to_string(entry.path().join("comm")).ok()?;
                        name.trim_start()
                            .starts_with("nz-player")
                            .then(|| entry.file_name().to_string_lossy().parse().ok())?
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `player()` answers the SAME player on every call.
    ///
    /// Why it matters beyond a test: a `PlayerApi` owns a `Player`, an engine and
    /// the engine's ONE sink. Before MB3c's measurement, each call built a new
    /// one, so the media service opened AAudio on ITS instance while the caller's
    /// queue lived in another: 40 streams were fetched and `position()` stayed at
    /// 0 — the sound of a phone that says "Lecture".
    #[test]
    fn the_player_is_one_per_core() {
        let core = NzCore::new(config("one-player").1).expect("the core");
        let first = core.player();
        for _ in 0..20 {
            assert!(
                Arc::ptr_eq(&first, &core.player()),
                "player() handed out a new instance"
            );
        }
        // `PlayerApi::open` is the ONLY thing that builds a `Player` (and with it
        // an engine and the engine's one sink), so `ptr_eq` over 21 calls IS the
        // "one engine" proof — a second one would come with a second `Arc`.
        core.shutdown();
    }

    /// A command sent through one handle is visible in the other's snapshot, and
    /// `shutdown()` really joins the one engine (no "nz-player" thread left).
    #[test]
    fn one_player_sees_one_queue_and_shutdown_joins_it() {
        let before = player_thread_ids();
        let (_dir, config) = config("shared-queue");
        let core = NzCore::new(config).expect("the core");
        let sender = core.player();
        let reader = core.player();
        // The threads this test's player brought: the ones that may go.
        //
        // A spawned thread takes its name INSIDE itself (`prctl(PR_SET_NAME)`), so a
        // read that lands between `spawn` and that call sees the parent's name and
        // misses it — measured on loaded CI runners, where this census ran before the
        // child had been scheduled and the test failed with "the player started no
        // thread" while the same suite was 95/95 green locally. Wait for the ids to
        // appear, bounded, the way `nz-lan`'s `thread_by_name` does: a player that
        // never starts one still fails, and the bound is the deadline written below.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut mine: std::collections::BTreeSet<u64> =
            player_thread_ids().difference(&before).copied().collect();
        while mine.is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
            mine = player_thread_ids().difference(&before).copied().collect();
        }
        assert!(!mine.is_empty(), "the player started no thread");
        assert!(Arc::ptr_eq(&sender, &reader));

        let rows: Vec<crate::player::TrackRef> = (0..3)
            .map(|index| crate::player::TrackRef {
                key: format!("k{index}"),
                id: format!("id{index}"),
                title: format!("Titre {index}"),
                ..Default::default()
            })
            .collect();
        sender.command(
            crate::player::PlayerCommand::PlayQueue {
                tracks: rows,
                start: 0,
                context: None,
            },
            0,
        );
        assert_eq!(reader.snapshot().len, 3, "the other handle sees the queue");

        core.shutdown();
        // Only THIS player's threads: `cargo test` shares the process with the
        // other tests, whose players are theirs.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut alive = mine.clone();
        while !alive.is_empty() && std::time::Instant::now() < deadline {
            let now = player_thread_ids();
            alive.retain(|id| now.contains(id));
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            alive.is_empty(),
            "shutdown left engine threads running: {alive:?}"
        );
    }

    /// The skeleton's own proof, kept: the object graph builds, answers its
    /// version, and tells the truth about a server it does not have.
    #[test]
    fn the_graph_builds_and_answers_its_version() {
        let (_dir, config) = config("build");
        let core = NzCore::new(config).expect("the core");
        assert_eq!(nz_version(), env!("CARGO_PKG_VERSION"));
        assert!(!nz_version().is_empty());
        assert!(core.server().is_none());
        assert!(core.stats().is_empty());
        assert_eq!(core.memory().lists_bytes, 0);
        let screens = core.screens();
        assert!(Arc::ptr_eq(&screens, &core.screens()));
        core.shutdown();
    }

    /// An app link the core can read crosses the boundary as the record `:app`
    /// maps to a route; an unknown one is `None`, never a wrong screen.
    #[test]
    fn a_link_crosses_the_boundary_as_the_same_route() {
        assert_eq!(
            resolve_link("https://neezer.example/app/#/album/42".into()),
            Some(AppLink::Album { id: "42".into() })
        );
        // The app's own root is Home; a path this app does not open is nothing.
        assert_eq!(
            resolve_link("https://neezer.example/".into()),
            Some(AppLink::Home)
        );
        assert_eq!(
            resolve_link("https://neezer.example/app/#/library".into()),
            None
        );
        assert_eq!(
            resolve_link("neezer://party/abc".into()),
            Some(AppLink::Party { id: "abc".into() })
        );
    }

    /// A listener hears the event the core pushed, with the FRENCH sentence —
    /// the reason is never a code (rule 3), and the adapter is what translates.
    #[test]
    fn a_listener_hears_a_french_cause() {
        use nz_core::core::CoreError;
        use std::sync::mpsc;
        struct Recorder(mpsc::Sender<(String, Option<Me>)>);
        impl SessionListener for Recorder {
            fn on_session_ended(&self, reason: String) {
                let _ = self.0.send((reason, None));
            }
            fn on_me_changed(&self, me: Option<Me>) {
                let _ = self.0.send((String::new(), me));
            }
        }
        let (sender, receiver) = mpsc::channel();
        let adapter = SessionSinkAdapter(Arc::new(Recorder(sender)));
        adapter.publish(SessionEvent::Ended(CoreError::Session));
        let (reason, me) = receiver.recv().expect("an event");
        assert!(me.is_none());
        assert!(reason.contains("Session"), "{reason}");
        assert!(!reason.contains("401"), "never a code: {reason}");
    }

    /// A screen-shaped list crosses with what it draws: 8 000 rows stay in Rust
    /// and one page comes across, in the order the list was sorted into.
    #[test]
    fn a_list_crosses_one_page_at_a_time() {
        let (_dir, config) = config("list");
        let core = NzCore::new(config).expect("the core");
        let rows: Vec<crate::lists::TrackRow> = (0..8_000)
            .map(|number| crate::lists::TrackRow::test_row(number, &format!("Titre {number}")))
            .collect();
        let handle = core.open_track_list(rows);
        assert_eq!(handle.len(), 8_000);
        let page = handle.window(0, 64);
        assert_eq!(page.len(), 64);
        assert_eq!(page[0].title, "Titre 0");
        assert_eq!(page[63].position, 63);
        // The registry saw it, so a later lot's patch reaches it.
        assert_eq!(core.core().lists().live(), 1);
        assert!(core.memory().lists_bytes > 0);
    }
}
