//! The object graph and everything a screen asks the core itself
//! (docs/agents/mobile.md, "The FFI boundary"). Owned by **MB2**: it fills the
//! bodies and adds the JNI `Platform_init` export the Android trust store
//! needs. MB0 declared the types and the signatures, and made `nz_version()`
//! work for real — that one call is what proves the `.so` is loaded.

use std::sync::Arc;

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

/// The signed-in person, as the shell draws it.
#[derive(uniffi::Record, Debug, Clone)]
pub struct Me {
    pub id: String,
    pub name: String,
    /// The administrator sections of Réglages are only shown when true.
    pub admin: bool,
    pub avatar: Option<String>,
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

/// Counted bytes of the core's own stores, never a process RSS
/// (docs/agents/mobile.md, *Measures*, and worker rule 4).
#[derive(uniffi::Record, Debug, Clone)]
pub struct MemoryReport {
    pub queue_bytes: u64,
    pub lists_bytes: u64,
    pub api_cache_mem_bytes: u64,
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

/// The end of a session, pushed to the shell. Kotlin hands it to a Flow and
/// returns at once: this runs on a Rust thread.
#[uniffi::export(callback_interface)]
pub trait SessionListener: Send + Sync {
    /// `reason` is the French sentence to show, never a code.
    fn on_session_ended(&self, reason: String);
    /// The signed-in person changed (restored after a launch, or logged out).
    fn on_me_changed(&self, me: Option<Me>);
}

/// The read-only screen data: one `#[uniffi::export] impl Screens` block per
/// feature lot, in that lot's own file. MB0 declares the object here so every
/// lot can add its block without editing a shared file.
#[derive(uniffi::Object)]
pub struct Screens;

/// The app's core, once, built by `NzBridge.install`.
#[derive(uniffi::Object)]
pub struct NzCore;

#[uniffi::export(async_runtime = "tokio")]
impl NzCore {
    /// Builds the graph. **MB0**: answers a stub object; MB2 opens the caches,
    /// starts `nz-net` and restores the session.
    #[uniffi::constructor]
    pub fn new(cfg: CoreConfig) -> Result<Arc<Self>, NzError> {
        tracing::debug!(
            data_dir = %cfg.data_dir,
            cache_dir = %cfg.cache_dir,
            "nz-core built (MB0 skeleton: nothing is opened yet)"
        );
        Ok(Arc::new(NzCore))
    }

    /// The configured origin and its pin, or `None` before setup.
    pub fn server(&self) -> Option<ServerInfo> {
        None
    }

    /// Saves the server and checks it answers `GET /api/version`.
    pub async fn set_server(&self, url: String) -> Result<ServerInfo, NzError> {
        Err(NzError::Unsupported {
            feature: format!("mb2:set_server:{url}"),
        })
    }

    /// Learns the certificate with a handshake that sends nothing.
    pub async fn probe_certificate(&self, url: String) -> Result<CertInfo, NzError> {
        Err(NzError::Unsupported {
            feature: format!("mb4:probe_certificate:{url}"),
        })
    }

    /// Pins one leaf for one origin (TOFU), or clears it with an empty hash.
    pub fn pin_certificate(&self, origin: String, sha256: String) {
        tracing::debug!(%origin, %sha256, "pin_certificate (MB0 skeleton)");
    }

    pub async fn login(&self, username: String, password: String) -> Result<Me, NzError> {
        let _ = (username, password);
        Err(NzError::Unsupported {
            feature: "mb2:login".into(),
        })
    }

    /// Restores the session cookie from `filesDir/neezer/session.json`.
    pub async fn restore(&self) -> Result<Option<Me>, NzError> {
        Ok(None)
    }

    pub async fn logout(&self) -> Result<(), NzError> {
        Err(NzError::Unsupported {
            feature: "mb2:logout".into(),
        })
    }

    /// `ConnectivityManager.registerDefaultNetworkCallback`'s answer.
    pub fn set_network(&self, net: NetworkState) {
        tracing::debug!(?net, "set_network (MB0 skeleton)");
    }

    /// `ProcessLifecycleOwner`'s answer: prefetch only while somebody is looking.
    pub fn set_foreground(&self, visible: bool) {
        tracing::debug!(visible, "set_foreground (MB0 skeleton)");
    }

    pub fn subscribe_session(&self, listener: Box<dyn SessionListener>) {
        drop(listener);
    }

    /// Requests and bytes per screen tag, cold and warm (rule 3).
    pub fn stats(&self) -> Vec<NetStats> {
        Vec::new()
    }

    pub fn screens(&self) -> Arc<Screens> {
        Arc::new(Screens)
    }

    pub fn player(&self) -> Arc<PlayerApi> {
        Arc::new(PlayerApi)
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
        Arc::new(SettingsApi)
    }

    /// Opens a list over a set of keys (rule 4). MB2 fills the window.
    pub fn open_list(&self, keys: Vec<String>) -> Arc<ListHandle> {
        crate::lists::open(keys)
    }

    /// Joins every thread that borrows the runtime, then stops it. Android kills
    /// the process instead, so nothing correct may depend on this running.
    pub fn shutdown(&self) {
        tracing::debug!("shutdown (MB0 skeleton: no thread to join yet)");
    }
}

/// Resolves an app link (`/app/#/...`, `neezer://...`, a Deezer or Spotify
/// URL) into a route. MB2 owns the rule; MB0 answers nothing.
#[uniffi::export]
pub fn resolve_link(route: String) -> Option<AppLink> {
    crate::links::parse_app_link(route)
}

/// The Rust version shown in Aide. **The one call that really works in MB0**:
/// it proves JNA found `libnz_ffi.so` and its symbols.
#[uniffi::export]
pub fn nz_version() -> String {
    nz_core::version()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> CoreConfig {
        CoreConfig {
            data_dir: "/tmp/neezer-test/data".into(),
            cache_dir: "/tmp/neezer-test/cache".into(),
            app_version: "0.0.0".into(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".into(),
        }
    }

    /// The skeleton builds, answers its version, and refuses the rest with the
    /// error the UI knows how to write in French.
    #[test]
    fn skeleton_builds_and_refuses_what_it_cannot_do() {
        let core = NzCore::new(cfg()).expect("MB0 constructor");
        assert_eq!(nz_version(), env!("CARGO_PKG_VERSION"));
        assert!(!nz_version().is_empty());
        assert!(core.server().is_none());
        assert!(core.stats().is_empty());
        assert_eq!(core.open_list(vec!["a".into(), "b".into()]).len(), 0);

        let err = futures_lite_block_on(core.login("a".into(), "b".into()));
        match err {
            Err(NzError::Unsupported { feature }) => assert!(feature.starts_with("mb2")),
            other => panic!("expected Unsupported, got {other:?}"),
        }
        core.shutdown();
    }

    /// A one-shot executor for the async stubs: the test owns its runtime, and
    /// nothing is left borrowing it (worker rule 5).
    fn futures_lite_block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(fut)
    }
}
