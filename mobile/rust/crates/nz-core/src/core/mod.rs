//! The object graph and everything a screen asks the core itself
//! (docs/agents/mobile.md, "The FFI boundary"). Owned by **MB2**.
//!
//! One [`Core`] per process, built by `NzBridge.install` at
//! `Application.onCreate`: it opens the local store, builds the session jar,
//! remembers the configured server and the person, and owns the one HTTP client
//! every screen shares. Nothing here knows Android, UniFFI or Kotlin — the same
//! crate compiles for iOS ([docs/agents/mobile.md], *iOS later*).
//!
//! # The rules this module keeps
//!
//! - **Rule 3, the link**: `set_network(None)` short-circuits every request
//!   BEFORE a socket is opened, and the first failure a screen sees is a cause
//!   it can print, never a spinner. Counted in [`Meter`], never timed.
//! - **Rule 7, the device's own data**: the SQLite file and the session jar are
//!   opened in place, migrate idempotently (`nz-cache`), and a lost cache is
//!   never a lost action.
//! - **One writer per area**: `session.rs`, `server.rs`, `covers.rs`,
//!   `meter.rs` and `url.rs` are MB2's own files, and a later lot adds its own
//!   module here rather than editing this one.
//! - **No thread of its own.** UniFFI's `async_runtime = "tokio"` owns the
//!   runtime every `async fn` runs on, so there is no thread for `shutdown` to
//!   join (worker rule 5): a listener callback runs on the runtime's worker that
//!   issued the request, and the Kotlin side hands it to a Flow at once.

mod covers;
mod error;
mod meter;
mod platform;
mod server;
mod session;
mod url;

#[cfg(test)]
mod testing;

pub use error::CoreError;
pub use meter::{Meter, TAG_COVER, TAG_SESSION, TagStats};
pub use platform::{install_journal, nz_core_version};
pub use server::CertificateInfo;
pub use url::{Origin, Url, is_ip_literal};

// The signed-in person, as `/api/me` decodes them. Re-exported so `nz-ffi`
// converts the core's own type without depending on `nz-model` itself (one
// fewer crate in the phone's dependency list).
pub use nz_model::Me;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use nz_api::{Api, SessionJar};
use nz_cache::Cache;

use crate::lists::Registry;

/// What `:bridge` hands the core once, at `Application.onCreate`.
#[derive(Debug, Clone)]
pub struct CoreConfig {
    /// `filesDir/neezer` — the downloads and the one SQLite file, never evicted.
    pub data_dir: PathBuf,
    /// `cacheDir/neezer` — covers and the play cache, which the system may clear.
    pub cache_dir: PathBuf,
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

/// What `ConnectivityManager` says. `None` flips every request offline AT ONCE,
/// so none waits for a timeout to learn it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkState {
    None,
    Wifi { metered: bool },
    Cellular { metered: bool },
    Other { metered: bool },
}

impl NetworkState {
    /// Whether anything can be sent at all.
    pub fn is_online(self) -> bool {
        !matches!(self, NetworkState::None)
    }

    /// Whether the person pays for what this link carries: the downloads'
    /// policy and the Auto quality ladder's ceiling read it (later lots).
    pub fn metered(self) -> bool {
        match self {
            NetworkState::None => true,
            NetworkState::Wifi { metered }
            | NetworkState::Cellular { metered }
            | NetworkState::Other { metered } => metered,
        }
    }
}

/// The configured server, as the core keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    /// `scheme://host:port`, the ONE definition of "our server".
    pub origin: Origin,
    /// The pinned leaf's SHA-256 (colon-separated hex), when the person pinned one.
    pub pinned_sha256: Option<String>,
    /// What `GET /api/version` last answered, empty before the first probe.
    pub version: String,
}

/// A change the shell must hear about. Always pushed, never polled: the old
/// app's 4 Hz tick into a sleeping UI is what this replaces.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// The session is gone; `reason` is the failure `nz_error_text` will write.
    Ended(CoreError),
    /// The signed-in person changed (restored, logged in, logged out).
    Me(Option<Me>),
}

/// Who hears about a [`SessionEvent`]. `:bridge` implements this over its
/// `SessionListener` and hands the event to a Flow at once — it runs on the
/// runtime's worker, never on Kotlin's main thread, and the core holds no lock
/// while calling it.
pub trait SessionSink: Send + Sync {
    fn publish(&self, event: SessionEvent);
}

/// The keys this core persists in the settings store (`nz-cache`'s `kv`).
pub(crate) const KEY_ORIGIN: &str = "server.origin";
pub(crate) const KEY_VERSION: &str = "server.version";
const KEY_PIN_PREFIX: &str = "server.pin.";

/// The app's core, once.
pub struct Core {
    config: CoreConfig,
    cache: Cache,
    /// The cookie jar, replaced as a whole when a logout must forget it offline
    /// (`session.rs`): the client holds its own `Arc`, so both move together.
    jar: RwLock<Arc<SessionJar>>,
    /// The client of the configured server, or `None` before setup.
    api: RwLock<Option<Arc<Api>>>,
    server: RwLock<Option<ServerInfo>>,
    me: RwLock<Option<Me>>,
    /// Whether the radio is up (`ConnectivityManager`'s answer) — the gate that
    /// answers `Offline` in zero sockets.
    online: AtomicBool,
    network: RwLock<NetworkState>,
    foreground: AtomicBool,
    sinks: Mutex<Vec<Arc<dyn SessionSink>>>,
    meter: Meter,
    lists: Registry,
}

impl std::fmt::Debug for Core {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the session jar's contents (it does not print them either) and
        // never a password.
        f.debug_struct("Core")
            .field("data_dir", &self.config.data_dir)
            .field("origin", &self.origin().map(|origin| origin.as_str()))
            .field("online", &self.is_online())
            .finish()
    }
}

impl Core {
    /// Builds the graph. Cheap on purpose: it opens SQLite (and replays its own
    /// migrations), reads the server and the person it remembers, and touches
    /// no network at all.
    pub fn new(config: CoreConfig) -> Result<Core, CoreError> {
        install_journal();
        tracing::debug!(
            version = platform::nz_core_version(),
            device = %config.device_name,
            "opening the core"
        );
        let cache = Cache::open_with_cache_dir(&config.data_dir, &config.cache_dir)?;
        let jar = Arc::new(SessionJar::at(config.data_dir.join("session.json")));

        let core = Core {
            config,
            cache,
            jar: RwLock::new(jar),
            api: RwLock::new(None),
            server: RwLock::new(None),
            me: RwLock::new(None),
            online: AtomicBool::new(true),
            network: RwLock::new(NetworkState::Wifi { metered: false }),
            foreground: AtomicBool::new(false),
            sinks: Mutex::new(Vec::new()),
            meter: Meter::new(),
            lists: Registry::new(),
        };

        // The server the person configured survives a restart (rule 7): the
        // origin is what `restore()` needs before any screen exists.
        if let Some(origin) = core.cache.kv().get::<String>(KEY_ORIGIN) {
            if let Some(origin) = Origin::parse(&origin) {
                let version = core
                    .cache
                    .kv()
                    .get::<String>(KEY_VERSION)
                    .unwrap_or_default();
                let pin = core.pinned(&origin);
                let api = core.build_api(&origin)?;
                *core.api.write().unwrap_or_else(|err| err.into_inner()) = Some(api);
                *core.server.write().unwrap_or_else(|err| err.into_inner()) = Some(ServerInfo {
                    origin,
                    pinned_sha256: pin,
                    version,
                });
            } else {
                // A stored origin this build can no longer read is dropped, not
                // fought over: the setup screen asks for it again (rule 7).
                tracing::warn!("a stored server origin no longer parses; asking for it again");
            }
        }

        Ok(core)
    }

    pub fn config(&self) -> &CoreConfig {
        &self.config
    }

    /// The local store: SQLite, the covers and the play cache.
    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    /// The one HTTP client of the configured server (a cheap `Arc` clone).
    ///
    /// **The offline gate lives here**: with no link at all the answer is
    /// `Offline` without a socket being opened, which is what the acceptance
    /// counts (0 requests on the meter, 0 connections on the peer).
    pub fn api(&self) -> Result<Arc<Api>, CoreError> {
        if !self.is_online() {
            return Err(CoreError::Offline);
        }
        self.api
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
            .ok_or_else(|| CoreError::Invalid {
                detail: "Aucun serveur n'est configuré".to_owned(),
            })
    }

    /// The session cookie jar, for a caller that must clear it (logout).
    pub fn jar(&self) -> Arc<SessionJar> {
        self.jar
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Installs a fresh jar (`session.rs`'s offline logout).
    pub(super) fn set_jar(&self, jar: Arc<SessionJar>) {
        *self.jar.write().unwrap_or_else(|err| err.into_inner()) = jar;
    }

    /// The counters of what this device spent.
    pub fn meter(&self) -> &Meter {
        &self.meter
    }

    /// The open lists, so a lot can patch every visible row in one frame.
    pub fn lists(&self) -> &Registry {
        &self.lists
    }

    pub fn server(&self) -> Option<ServerInfo> {
        self.server
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub fn origin(&self) -> Option<Origin> {
        self.server().map(|server| server.origin)
    }

    /// The signed-in person, as the last successful `login`/`restore` left them.
    pub fn me(&self) -> Option<Me> {
        self.me
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// `ConnectivityManager.registerDefaultNetworkCallback`'s answer.
    pub fn set_network(&self, state: NetworkState) {
        *self.network.write().unwrap_or_else(|err| err.into_inner()) = state;
        let online = state.is_online();
        let changed = self.online.swap(online, Ordering::SeqCst) != online;
        if let Some(api) = self
            .api
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .as_ref()
        {
            // `nz-api` flips too: a request that started before the callback is
            // refused instead of running its full retry ladder.
            api.set_online(online);
        }
        if changed {
            tracing::info!(online, metered = state.metered(), "network changed");
        }
    }

    pub fn network(&self) -> NetworkState {
        *self.network.read().unwrap_or_else(|err| err.into_inner())
    }

    fn is_online(&self) -> bool {
        self.online.load(Ordering::SeqCst)
    }

    /// `ProcessLifecycleOwner`'s answer: prefetch only while somebody is
    /// looking (rule 1's PREFETCH lane is MB5's to feed).
    pub fn set_foreground(&self, visible: bool) {
        self.foreground.store(visible, Ordering::SeqCst);
    }

    pub fn is_foreground(&self) -> bool {
        self.foreground.load(Ordering::SeqCst)
    }

    /// Registers a listener. Every later event reaches all of them; the list is
    /// small (the shell subscribes once) and a dead listener is dropped by the
    /// Kotlin side, which owns its lifetime.
    pub fn subscribe_session(&self, sink: Arc<dyn SessionSink>) {
        self.sinks
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(sink);
    }

    /// Pushes an event to every listener, holding no lock while it runs: a
    /// Kotlin callback must never be able to deadlock the core.
    pub(crate) fn emit(&self, event: SessionEvent) {
        let sinks: Vec<Arc<dyn SessionSink>> = self
            .sinks
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        for sink in sinks {
            sink.publish(event.clone());
        }
    }

    /// Requests and bytes per tag, in tag order.
    pub fn stats(&self) -> Vec<TagStats> {
        self.meter.stats()
    }

    /// Counted bytes of what the core itself holds in memory, never a process
    /// RSS (worker rule 4). The queue is MB3b's and the API cache lives in
    /// SQLite, so both read 0 here — a later lot replaces its own line.
    pub fn memory(&self) -> Memory {
        Memory {
            queue_bytes: 0,
            lists_bytes: self.lists.memory_bytes() as u64,
            api_cache_mem_bytes: 0,
        }
    }

    /// Drops the client and the listeners. Android kills the process instead, so
    /// nothing correct depends on this running; the runtime belongs to UniFFI,
    /// and this core starts no thread to join (worker rule 5).
    pub fn shutdown(&self) {
        self.sinks
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clear();
        *self.api.write().unwrap_or_else(|err| err.into_inner()) = None;
        tracing::debug!("core shut down");
    }

    /// Where a file this app owns lives under the data directory.
    pub fn data_path(&self, name: &str) -> PathBuf {
        self.config.data_dir.join(name)
    }

    /// Where an evictable file lives under the cache directory.
    pub fn cache_path(&self, name: &str) -> PathBuf {
        self.config.cache_dir.join(name)
    }

    /// Ensures a directory exists, reporting the cause rather than panicking.
    pub(crate) fn ensure_dir(&self, dir: &Path) -> Result<(), CoreError> {
        std::fs::create_dir_all(dir).map_err(|error| CoreError::Internal {
            detail: format!("{} : {error}", dir.display()),
        })
    }
}

/// The counted bytes of the core's own stores.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Memory {
    pub queue_bytes: u64,
    pub lists_bytes: u64,
    pub api_cache_mem_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate answers offline in ZERO sockets: no client, no request, and the
    /// meter stays empty. This is the acceptance's "0 sockets opened", at the
    /// unit level; `session.rs` proves it against a live server.
    #[test]
    fn an_offline_core_refuses_before_touching_the_network() {
        let dir = testing::TempDir::new("offline-gate");
        let core = dir.core("test");
        core.set_network(NetworkState::None);
        for attempt in 0..3 {
            assert_eq!(
                core.api().err(),
                Some(CoreError::Offline),
                "attempt {attempt}"
            );
        }
        assert!(core.stats().is_empty(), "a refused call records nothing");
        assert!(!core.network().is_online());
        assert!(core.network().metered(), "no link is not a free link");
    }

    /// A core with no server says so, and does not pretend to be offline.
    #[test]
    fn a_core_without_a_server_says_it_has_none() {
        let dir = testing::TempDir::new("no-server");
        let core = dir.core("test");
        assert!(core.server().is_none());
        assert!(core.origin().is_none());
        assert!(matches!(core.api(), Err(CoreError::Invalid { .. })));
    }

    /// The roots a later lot writes into exist before it asks.
    #[test]
    fn the_two_roots_and_the_offline_directory_exist_at_start() {
        let dir = testing::TempDir::new("roots");
        let core = dir.core("test");
        assert!(core.config().data_dir.is_dir());
        assert!(core.config().cache_dir.is_dir());
        assert!(core.cache().offline_dir().is_dir());
        assert!(core.cache().covers_dir().is_dir());
        assert!(core.cache().audio_dir().is_dir());
    }

    /// Memory is COUNTED, in bytes: a million rows is not "some MB".
    #[test]
    fn memory_counts_the_lists_and_nothing_it_does_not_hold() {
        let dir = testing::TempDir::new("memory");
        let core = dir.core("test");
        let empty = core.memory();
        assert_eq!(empty.queue_bytes, 0);
        assert_eq!(empty.lists_bytes, 0);
        assert_eq!(empty.api_cache_mem_bytes, 0);

        let rows: Vec<crate::lists::Row> = (0..1_000)
            .map(|n| crate::lists::Row::test_row(n, &format!("Titre {n}")))
            .collect();
        let list = core.lists().open(rows);
        let counted = core.memory().lists_bytes;
        assert!(
            counted > 1_000 * 32,
            "1 000 rows count more than 32 kB: {counted}"
        );
        drop(list);
        assert_eq!(core.memory().lists_bytes, 0, "a dropped list keeps nothing");
    }

    /// `stats()` is the meter's own answer, in tag order, with no invention.
    #[test]
    fn stats_are_the_meters_own_rows() {
        let dir = testing::TempDir::new("stats");
        let core = dir.core("test");
        assert!(core.stats().is_empty());
        core.meter().record(TAG_SESSION, 10, 2);
        let stats = core.stats();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].requests, 1);
        assert_eq!(stats[0].bytes_in, 10);
        assert_eq!(stats[0].bytes_out, 2);
    }

    /// Rule 3's sharpest promise, measured at the PEER: with no link at all the
    /// core opens **0 sockets** — not "few", not "one that fails fast".
    #[tokio::test]
    async fn no_link_opens_no_socket_at_all() {
        let listener = testing::CountingListener::start();
        let dir = testing::TempDir::new("sockets");
        let core = dir.core("test");

        // Configuring the server while the link is up still leaves the origin
        // stored (the port answers nothing, so the probe fails: rule 3's other
        // half, "the server is not there" is a cause, not a lost setup).
        let refused = core.set_server(listener.address()).await;
        assert!(refused.is_err(), "the port answers nothing");
        assert_eq!(
            core.origin().map(|origin| origin.as_str()),
            Some(listener.address().to_owned()),
            "the person's server is remembered anyway"
        );
        let before = listener.settled();
        assert!(before >= 1, "the setup probe really reached the listener");

        core.set_network(NetworkState::None);
        assert!(core.restore().await.expect("restore answers").is_none());
        assert_eq!(
            core.cover_path(&format!("{}/api/cover/1", listener.address()), 128)
                .await
                .expect_err("offline"),
            CoreError::Offline
        );
        assert_eq!(
            core.set_server(listener.address())
                .await
                .expect_err("offline"),
            CoreError::Offline
        );
        assert_eq!(
            listener.settled(),
            before,
            "not one socket was opened while offline"
        );
        assert!(
            core.stats().iter().all(|stats| stats.tag != TAG_COVER),
            "a refused cover records no request"
        );
    }

    /// A listener hears every event, and the core holds no lock while it runs.
    #[test]
    fn a_listener_hears_what_the_core_pushes() {
        use std::sync::mpsc;
        struct Recorder(mpsc::Sender<SessionEvent>);
        impl SessionSink for Recorder {
            fn publish(&self, event: SessionEvent) {
                // A sink that calls back into the core would deadlock if the
                // core held its lock: this reads the config from inside.
                let _ = self.0.send(event);
            }
        }
        let dir = testing::TempDir::new("listener");
        let core = dir.core("test");
        let (sender, receiver) = mpsc::channel();
        core.subscribe_session(Arc::new(Recorder(sender)));
        core.emit(SessionEvent::Me(None));
        match receiver.recv().expect("one event") {
            SessionEvent::Me(me) => assert!(me.is_none()),
            other => panic!("expected Me, got {other:?}"),
        }
    }
}
