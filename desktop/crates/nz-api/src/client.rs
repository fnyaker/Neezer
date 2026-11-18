//! The one HTTP client: `Api`.
//!
//! Everything the app asks the server goes through [`Api`]: the lanes of rule 1
//! ([`crate::lanes`]), the retry and cache rules of the SPA's `lib/api.js#req`
//! ([`Api::get`], [`Api::send`]), the stream contract of
//! `docs/agents/desktop.md` (*Streams*) ([`Api::stream`], [`Api::ready`]), and
//! the session cookie ([`crate::session`]).
//!
//! Three invariants this file exists to keep:
//!
//! - **Nothing bypasses the gates.** Every call takes a [`Lane`] and holds its
//!   slot for exactly as long as the request is in flight.
//! - **A GET retries, a POST never does.** Replaying a mutation could double it;
//!   the optimistic UI rolls back and offers "Réessayer" instead.
//! - **`X-NS-Error` is final.** The server already retried the provider; asking
//!   again only buys the same instant refusal three more times.
//!
//! The client sends neither `Origin` nor `Sec-Fetch-Site`: the server's
//! `same_origin_guard` lets a request without both through, and those headers
//! exist to stop BROWSERS on other origins. Never forge them.
//!
//! Four rules the PHONE needs and the desktop gets for free (`Net.kt`,
//! `Ssl.kt`; SC3):
//!
//! - **A redirect is followed by hand** ([`Api::attempt`]), never by reqwest:
//!   each hop is checked before a socket opens, so a server (or anything in the
//!   path) cannot send the session somewhere else or downgrade it to plaintext.
//!   Cleartext is allowed to the configured origin and to nobody else.
//! - **The session cookie goes to the configured origin only** ([`OriginJar`]),
//!   before a redirect and after one. The jar itself is still origin-keyed;
//!   this is the second lock on the same door.
//! - **A pinned leaf may pass where the platform's store does not**
//!   ([`crate::cert`]): one extra certificate, per origin, and the desktop —
//!   which never pins one — is built exactly as it was.
//! - **Every request is counted per screen tag** ([`Api::with_tag`],
//!   [`Api::stats`]), because rule 3 asks a screen's cost in requests and bytes,
//!   counted, never timed.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use reqwest::Method;
use reqwest::cookie::CookieStore;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::watch;

use crate::cache_trait::{NoCache, ResponseCache, is_cacheable};
use crate::cert::{PinState, pinned_client_config};
use crate::error::{ApiError, MAX_GET_ATTEMPTS, backoff};
use crate::lanes::{Gates, Lane, bulk_cap_from_stream};
use crate::session::SessionJar;
use crate::stats::{Stats, TAG_NONE, TagStats};

pub use nz_model::{Quality, StreamReady, TrackId};

/// How long a download may go without a byte before its link is called dead
/// (`Streams`: the stream's own 15 s watchdog, reused for every other file).
pub const STALL_TIMEOUT: Duration = Duration::from_secs(15);

/// One file the API served, written where the caller asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadReport {
    pub path: PathBuf,
    pub bytes: u64,
    pub content_type: Option<String>,
}

/// Where a download speaks: the client, and whether the endpoint is this
/// server's own route (`true`, which is what `X-NS-Background` and the session
/// classification apply to) or a pinned third-party host (`false`).
struct Target<'a> {
    http: &'a reqwest::Client,
    api: bool,
}

/// An IO failure, said in the words a screen can print.
fn write_error(error: std::io::Error) -> ApiError {
    ApiError::Http {
        status: 0,
        message: format!("écriture impossible : {error}"),
        code: Some("io".to_owned()),
    }
}

/// A refusal from a pinned third-party host, kept plain: its status, and at most
/// 200 characters of its body (`Api::download_url_to`). Nothing in it is read as
/// a neezer sentence: no `X-NS-Error`, no session, and a `404` here means "the
/// file moved", never "the route is missing".
fn external_error(status: u16, body: &str) -> ApiError {
    let text = body.trim();
    let message = if text.is_empty() {
        format!("HTTP {status}")
    } else {
        text.chars().take(200).collect::<String>()
    };
    ApiError::Http {
        status,
        message,
        code: None,
    }
}

/// The client every pinned-host download uses ([`Api::download_url_to`]).
///
/// Two properties are structural, not cosmetic:
///
/// - **no `cookie_provider`**: there is no jar in this client, so no `Cookie`
///   and no `Authorization` header can be sent to a third party — before a
///   redirect or after one, on any hop;
/// - **`https_only`**: a plaintext request is refused, and so is a redirect that
///   would downgrade to plaintext. The explicit scheme check in
///   [`Api::download_url_to`] is the first refusal, this one is the second.
///
/// `https_only` is a parameter only so the tests can drive a loopback HTTP
/// server: no production caller passes `false` (there is exactly one call site,
/// with `true`).
fn pinned_client(https_only: bool) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .https_only(https_only)
        .build()
}

/// How long a connection may take to establish (`docs/agents/desktop.md`).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
/// How long an API answer may take, headers and body included. A stream is
/// deliberately NOT under this timeout: the player's own 15 s byte watchdog
/// governs it, and a long FLAC must not be cut by an API deadline.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(12);
/// The header the server reads to know a request is nobody's priority.
pub const BACKGROUND_HEADER: &str = "X-NS-Background";
/// The header that makes a failure about the PROVIDER, not this server.
pub const NO_RETRY_HEADER: &str = "X-NS-Error";
/// Where the server says an earlier stream actually began (`?start=`).
pub const START_HEADER: &str = "X-NS-Start";
/// How many redirect hops one request may take before it is refused. The same
/// ten reqwest's own policy used, and the reason it is hand-followed: a hop is
/// checked (`Api::hop`) and counted before a socket opens.
pub const MAX_REDIRECTS: usize = 10;

const USER_AGENT: &str = concat!("neezer-desktop/", env!("CARGO_PKG_VERSION"));

/// What an answer is read as. The retry policy and the failure classification
/// are identical for both — a sleeve the server cannot serve is refused the
/// same way a JSON route is — so only the `Accept` header differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Want {
    Json,
    Bytes,
}

impl Want {
    fn accept(self) -> &'static str {
        match self {
            // An archived sleeve keeps its own format (`jpg`, `png`, `webp`…),
            // so nothing is claimed about it.
            Want::Bytes => "image/*, */*",
            Want::Json => "application/json",
        }
    }
}

/// A value that also says whether it came off the disk. `from_cache: true`
/// paints at once; the caller may then revalidate (the SPA's `swr`).
#[derive(Debug, Clone, PartialEq)]
pub struct Fresh<T> {
    pub value: T,
    pub from_cache: bool,
}

/// An open audio stream. The body is read with [`StreamResponse::chunk`], which
/// is also where a Bulk download is paced (`Gates::throttle`).
pub struct StreamResponse {
    /// `200` (whole / generated as sent) or `206` (a byte range of a file).
    pub status: u16,
    pub content_type: Option<String>,
    /// Present for a finished file; absent for a body generated as it is sent.
    pub content_length: Option<u64>,
    /// `"none"` for a generated stream, `"bytes"` for a finished file.
    pub accept_ranges: Option<String>,
    pub content_range: Option<String>,
    /// Where the server actually began (`X-NS-Start`), in seconds, when the
    /// request carried `?start=`.
    pub start_s: Option<f64>,
    response: reqwest::Response,
    gates: Arc<Gates>,
    lane: Lane,
    /// The counters this body's bytes belong to, and the tag they count under:
    /// a streamed FLAC is the largest thing a screen ever costs, so it is
    /// counted where it arrives ([`StreamResponse::chunk`]).
    stats: Arc<Stats>,
    tag: String,
}

impl std::fmt::Debug for StreamResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamResponse")
            .field("status", &self.status)
            .field("content_type", &self.content_type)
            .field("content_length", &self.content_length)
            .field("accept_ranges", &self.accept_ranges)
            .field("start_s", &self.start_s)
            .field("lane", &self.lane)
            .finish()
    }
}

impl StreamResponse {
    /// A finished file: it answers byte ranges, so a seek lands anywhere.
    pub fn is_file(&self) -> bool {
        self.accept_ranges.as_deref() == Some("bytes")
    }

    /// Generated as it is sent: no length, no ranges (a first play).
    pub fn is_generated(&self) -> bool {
        self.accept_ranges.as_deref() == Some("none")
    }

    /// `true` when this answer is `206 Partial Content`.
    pub fn is_partial(&self) -> bool {
        self.status == 206
    }

    /// The next piece of the body, or `None` at its end.
    ///
    /// A Bulk stream is paced here: 25 % of the playing stream's throughput,
    /// never under 64 kB/s (`bulk_cap_from_stream`). The chunk is copied out of
    /// reqwest's buffer so this crate needs no `bytes` dependency — a memcpy of
    /// at most 64 kB per chunk, on the network thread, never in the audio
    /// callback.
    pub async fn chunk(&mut self) -> Result<Option<Vec<u8>>, ApiError> {
        match self.response.chunk().await {
            Ok(Some(bytes)) => {
                let len = bytes.len() as u64;
                self.stats.bytes_in(&self.tag, len);
                if self.lane == Lane::Bulk {
                    self.gates.throttle(len).await;
                }
                Ok(Some(bytes.to_vec()))
            }
            Ok(None) => Ok(None),
            // A body that stops mid-way is a dead link, not a verdict about the
            // track: the player reconnects (byte range, or `?start=`).
            Err(_) => Err(ApiError::Offline),
        }
    }
}

/// The HTTP client of the app.
///
/// Cheap to CLONE, and every clone shares one state: the session jar, the lane
/// gates, the counters and the pin. That is what lets a screen say
/// `api.with_tag("home")` and get a client whose requests are counted as the
/// home screen's without a second connection pool, a second cookie or a second
/// offline copy.
#[derive(Clone)]
pub struct Api {
    base: String,
    base_url: reqwest::Url,
    http: reqwest::Client,
    /// The client of [`Api::download_url_to`]: no cookie jar (a session must not
    /// leave this server), redirects followed (a pinned host answers `302` to its
    /// CDN), same `User-Agent`.
    external: reqwest::Client,
    jar: Arc<SessionJar>,
    gates: Arc<Gates>,
    cache: Arc<dyn ResponseCache>,
    owner: Arc<RwLock<Option<String>>>,
    online: watch::Sender<bool>,
    caching: Arc<AtomicBool>,
    jitter: Arc<AtomicU64>,
    jitter_seed: u64,
    /// What this clone counts under: a screen's tag, [`TAG_NONE`] when the
    /// client was never tagged (`crate::stats`).
    tag: String,
    stats: Arc<Stats>,
    /// The trust-on-first-use pin, when this client installed one
    /// ([`ApiBuilder::pinned_leaf_sha256`]). `None` on the desktop: no pin, no
    /// custom TLS verifier, exactly the client it always built.
    pin: Option<Arc<PinState>>,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the cookie jar's contents, never a token.
        f.debug_struct("Api")
            .field("base", &self.base)
            .field("owner", &self.owner())
            .field("online", &self.is_online())
            .field("tag", &self.tag)
            .field("pinned", &self.pin.is_some())
            .finish()
    }
}

/// The session jar, scoped to the ONE origin it belongs to.
///
/// [`SessionJar`] already keys its cookies by origin, so a redirect to another
/// host would find nothing to send. This wrapper says it structurally, at the
/// client's own boundary (`Net.kt`'s rule): a request to any other origin
/// carries no cookie, and a `Set-Cookie` from any other origin is dropped
/// instead of being written to the person's session file.
struct OriginJar {
    jar: Arc<SessionJar>,
    origin: reqwest::Url,
}

impl CookieStore for OriginJar {
    fn set_cookies(
        &self,
        cookie_headers: &mut dyn Iterator<Item = &reqwest::header::HeaderValue>,
        url: &reqwest::Url,
    ) {
        if url.origin() == self.origin.origin() {
            self.jar.set_cookies(cookie_headers, url);
        }
        // Off-origin `Set-Cookie` headers are dropped: the person's session file
        // is the configured server's, and a redirect target has no business
        // writing into it.
    }

    fn cookies(&self, url: &reqwest::Url) -> Option<reqwest::header::HeaderValue> {
        if url.origin() == self.origin.origin() {
            self.jar.cookies(url)
        } else {
            None
        }
    }
}

/// Builds an [`Api`]: the app adds the cache (`nz-cache`), the session file and
/// the base URL; a test may add an in-memory cache.
pub struct ApiBuilder {
    base: String,
    cache: Option<Arc<dyn ResponseCache>>,
    session: Option<Arc<SessionJar>>,
    pin: Option<Option<[u8; 32]>>,
}

impl Api {
    /// A client for `base` (the server's root: `http://host:port`), with an
    /// in-memory session and no offline cache.
    pub fn new(base: impl Into<String>) -> Result<Self, ApiError> {
        ApiBuilder::new(base).build()
    }

    /// A client with everything the app gives it.
    pub fn builder(base: impl Into<String>) -> ApiBuilder {
        ApiBuilder::new(base)
    }

    /// The server root this client speaks to, without a trailing slash.
    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// A clone of this client that counts under `tag` — a screen's name
    /// (`home`, `search`, `player`), which is what [`Api::stats`] answers per
    /// screen (rule 3). Everything else is shared: the same pool, the same
    /// session, the same lanes, the same pin.
    ///
    /// Cheap by design: a `String` and an `Arc` per screen, and nothing is
    /// allocated per request because the tag lives on the clone.
    pub fn with_tag(&self, tag: impl Into<String>) -> Self {
        let mut clone = self.clone();
        clone.tag = tag.into();
        clone
    }

    /// What this client's requests are counted under ([`TAG_NONE`] when it was
    /// never tagged).
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// Requests and bytes per screen tag, cumulated since the client was built
    /// (rule 3: counted, not timed). Sorted by tag; empty until a request
    /// leaves.
    pub fn stats(&self) -> Vec<TagStats> {
        self.stats.snapshot()
    }

    /// Forget every counted total: what a bench needs between two measures, so
    /// the second one is not the sum of both.
    pub fn clear_stats(&self) {
        self.stats.clear();
    }

    /// The pinned leaf's SHA-256, colon-separated hex, when this client was told
    /// to pin one ([`ApiBuilder::pinned_leaf_sha256`]).
    pub fn pinned_fingerprint(&self) -> Option<String> {
        self.pin
            .as_ref()
            .and_then(|pin| pin.pin())
            .map(|leaf| crate::cert::hex_fingerprint(&leaf))
    }

    /// The account whose cached answers this client reads and writes: the
    /// `user.id` of `/api/me`. Two accounts never see each other's copy.
    pub fn owner(&self) -> Option<String> {
        self.owner
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Set (or forget, with `None`) the account the cache is keyed by. Called
    /// on login, on `/api/me`, and on logout.
    pub fn set_owner(&self, owner: Option<String>) {
        *self.owner.write().unwrap_or_else(|err| err.into_inner()) = owner;
    }

    pub fn is_online(&self) -> bool {
        *self.online.borrow()
    }

    /// Announce reachability: every answer at all proves the server is there,
    /// and a connection failure proves it is not. The app's network indicator
    /// follows this receiver and probes when it says offline.
    pub fn online(&self) -> watch::Receiver<bool> {
        self.online.subscribe()
    }

    /// The app's own verdict (a probe, or the person reconnecting), which also
    /// flips what the cache paths do (`lib/net.js#reportOnline`).
    pub fn set_online(&self, online: bool) {
        self.online.send_if_modified(|was| {
            if *was == online {
                false
            } else {
                *was = online;
                true
            }
        });
    }

    /// Cap a Bulk download at `bytes_per_s` (`None`: no cap). The app derives
    /// it from the playing stream's measured throughput with
    /// [`bulk_cap_from_stream`]; nothing else should call this with a number it
    /// invented.
    pub fn set_bulk_cap(&self, bytes_per_s: Option<u64>) {
        self.gates.set_bulk_cap(bytes_per_s);
    }

    /// The same, from a measured stream throughput
    /// (25 %, never under 64 kB/s). `None` removes the cap.
    pub fn set_bulk_cap_from_stream(&self, stream_bytes_per_s: Option<u64>) {
        self.gates
            .set_bulk_cap(bulk_cap_from_stream(stream_bytes_per_s));
    }

    /// The gates of rule 1, shared with whoever else spends this device's link.
    ///
    /// `nz-lan` takes them for a `Bulk` transfer between two devices
    /// (`docs/agents/lan-nodes.md`, *Rule 1 on the LAN*): a playlist pulled from
    /// a peer waits behind the tapped track exactly like a download from the
    /// server does, and the two never both hold the one Bulk slot. ONE
    /// mechanism, so a second throttle cannot pace the same link differently.
    pub fn gates(&self) -> Arc<Gates> {
        Arc::clone(&self.gates)
    }

    /// `false` while a remote-control loan drives somebody else's library: what
    /// this device reads then must not enter its own offline copy
    /// (`lib/apicache.js`, the `REMOTE` guard).
    pub fn set_caching(&self, enabled: bool) {
        self.caching.store(enabled, Ordering::Relaxed);
    }

    /// `true` when a session cookie is held for this server: the login screen
    /// asks for a password only when there is none.
    pub fn has_session(&self) -> bool {
        !self.jar.is_empty(&self.base_url)
    }

    /// Forget the stored session cookie (logout, or a plain `401`): the jar is
    /// emptied on disk too, and the cache stops being keyed to anybody.
    pub(crate) fn forget_session(&self) {
        self.jar.clear(&self.base_url);
        self.set_owner(None);
    }

    /// Forget one cached path (an optimistic mutation just invalidated it).
    pub fn forget(&self, path: &str) {
        if let Some(owner) = self.cache_owner(path) {
            self.cache.delete(&owner, path);
        }
    }

    fn cache_owner(&self, path: &str) -> Option<String> {
        if !self.caching.load(Ordering::Relaxed) || !is_cacheable(path) {
            return None;
        }
        self.owner()
    }

    fn cached_value<T: DeserializeOwned>(&self, owner: &str, path: &str) -> Option<T> {
        let body = self.cache.get(owner, path)?;
        match serde_json::from_str::<T>(&body) {
            Ok(value) => Some(value),
            Err(err) => {
                // Rule 7: an older copy of a payload this build reads
                // differently is dropped, not served, and never panics.
                tracing::debug!(path, error = %err, "a cached answer no longer decodes; dropped");
                self.cache.delete(owner, path);
                None
            }
        }
    }

    /// The absolute URL of one API path (`/me` → `http://host:port/api/me`).
    pub fn url(&self, path: &str) -> String {
        format!("{}/api{}", self.base, path)
    }

    fn jitter(&self) -> u64 {
        // A tiny xorshift over a counter and a per-client seed: the backoff's
        // 0–250 ms of jitter needs no cryptographic source and no new
        // dependency (every pin lives in the workspace).
        let counter = self
            .jitter
            .fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
        let mut x = counter ^ self.jitter_seed;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^= x >> 33;
        x % 251
    }

    fn note_reachable(&self) {
        self.set_online(true);
    }

    fn note_unreachable(&self) {
        self.set_online(false);
    }

    /// A typed GET. Cacheable content comes off the disk at once when the link
    /// is known dead, and a dead link that begins mid-request serves the last
    /// copy instead of an error (rule 3: what was fetched once is kept).
    pub async fn get<T: DeserializeOwned>(
        &self,
        lane: Lane,
        path: &str,
    ) -> Result<Fresh<T>, ApiError> {
        let owner = self.cache_owner(path);
        let was_online = self.is_online();
        if !was_online
            && let Some(owner) = owner.as_deref()
            && let Some(value) = self.cached_value(owner, path)
        {
            return Ok(Fresh {
                value,
                from_cache: true,
            });
        }

        match self
            .fetch_want(Want::Json, lane, Method::GET, path, None, was_online)
            .await
        {
            Ok(body) => {
                let body = String::from_utf8_lossy(&body).into_owned();
                if let Some(owner) = owner.as_deref() {
                    self.cache.put(owner, path, &body);
                }
                serde_json::from_str::<T>(&body)
                    .map(|value| Fresh {
                        value,
                        from_cache: false,
                    })
                    .map_err(|err| ApiError::Decode(format!("{path}: {err}")))
            }
            Err(ApiError::Offline) => {
                if let Some(owner) = owner.as_deref()
                    && let Some(value) = self.cached_value(owner, path)
                {
                    return Ok(Fresh {
                        value,
                        from_cache: true,
                    });
                }
                Err(ApiError::Offline)
            }
            Err(err) => {
                // A refusal means this copy is not what the server would say:
                // it must not be served again offline (`api.js#REFUSED`).
                let refused = matches!(
                    err,
                    ApiError::Unauthorized
                        | ApiError::Http {
                            status: 403 | 404,
                            ..
                        }
                );
                if let (true, Some(owner)) = (refused, owner.as_deref()) {
                    self.cache.delete(owner, path);
                }
                Err(err)
            }
        }
    }

    /// A mutation. NEVER retried: replaying a POST/PUT/PATCH/DELETE could
    /// double it (two playlists, two listens). `Ok(None)` is the server's `204`.
    pub async fn send<B: Serialize, T: DeserializeOwned>(
        &self,
        lane: Lane,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<Option<T>, ApiError> {
        let payload = match body {
            Some(value) => Some(
                serde_json::to_vec(value)
                    .map_err(|err| ApiError::Decode(format!("{path}: {err}")))?,
            ),
            None => None,
        };
        let answer = self
            .fetch_want(Want::Json, lane, method, path, payload.as_deref(), false)
            .await?;
        if answer.is_empty() {
            return Ok(None);
        }
        let answer = String::from_utf8_lossy(&answer);
        serde_json::from_str::<T>(&answer)
            .map(Some)
            .map_err(|err| ApiError::Decode(format!("{path}: {err}")))
    }

    /// A GET whose answer is BYTES: an archived sleeve (`/api/cover/<id>`).
    ///
    /// D8a added this and nothing else to the client. A cover is an image, and
    /// every route here answers JSON or a typed value, so a screen had no way
    /// to ask for one without building its own HTTP client — which is exactly
    /// what this crate exists to prevent (rule 1's lanes, the session cookie
    /// and rule 3's retry policy all live here). It is a plain GET on a lane:
    /// the same three tries as `get`, the same `Offline`/`Provider` causes, and
    /// no offline copy of its own — `nz-cache` keeps sleeves, not this cache.
    pub async fn bytes(&self, lane: Lane, path: &str) -> Result<Vec<u8>, ApiError> {
        let was_online = self.is_online();
        self.fetch_want(Want::Bytes, lane, Method::GET, path, None, was_online)
            .await
    }

    /// Whether the SERVER has this route at all: what an older server is
    /// missing (`crate::features`).
    ///
    /// `Some(true)` — a registered route (200, or 405 when the route exists for
    /// another method); `Some(false)` — the server answered 404 for it;
    /// `None` — the question could not be asked (offline, a 5xx, a decode
    /// failure), and a feature must never be hidden on a doubt.
    ///
    /// One `OPTIONS` on the `Prefetch` lane: the app asks this at login, once
    /// per feature, and never on a screen's opening path.
    pub async fn has_route(&self, path: &str) -> Option<bool> {
        match self
            .fetch_want(
                Want::Bytes,
                Lane::Prefetch,
                Method::OPTIONS,
                path,
                None,
                false,
            )
            .await
        {
            Ok(_) => Some(true),
            Err(ApiError::Http { status: 405, .. }) => Some(true),
            Err(ApiError::Http { status: 404, .. }) => Some(false),
            Err(ApiError::Unauthorized) => Some(true),
            Err(_) => None,
        }
    }

    /// One file served by the API, written to disk as it arrives.
    ///
    /// `GET /api/export/<kind>/<id>?fmt=`, `GET /api/services/list/…` and the
    /// share routes answer a FILE, not JSON: a playlist's ZIP is gigabytes and
    /// rule 3 forbids holding it in memory (`Api::bytes` is for a sleeve). The
    /// body is therefore appended to `<destination>.part` chunk by chunk and
    /// only renamed once the whole file arrived, so a reader never sees half an
    /// archive (the app's own download rule). `on_progress` is called after each
    /// chunk with the bytes written and the length when the server sent one;
    /// `cancel` is read before each chunk, and a cancelled transfer removes its
    /// partial file. A body that stops arriving for [`STALL_TIMEOUT`] is a dead
    /// link, not a verdict about the file: it fails fast with
    /// [`ApiError::Offline`].
    ///
    /// The lane slot is held for the WHOLE body, unlike [`Api::stream`]: an
    /// export is a download, it is never what the person just asked to hear, and
    /// `Lane::Bulk` allows exactly one at a time.
    pub async fn download_to<F>(
        &self,
        lane: Lane,
        path: &str,
        destination: &Path,
        on_progress: F,
        cancel: &AtomicBool,
    ) -> Result<DownloadReport, ApiError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        self.download_from(
            Target {
                http: &self.http,
                api: true,
            },
            self.url(path),
            lane,
            destination,
            on_progress,
            cancel,
        )
        .await
    }

    /// One file from an ABSOLUTE `https` URL — a pinned third-party host, not an
    /// API path — written to disk as it arrives.
    ///
    /// [`download_to`](Api::download_to) builds `{base}/api{path}`: handing it
    /// the absolute model URL asked the neezer SERVER for
    /// `/apihttps://huggingface.co/…`, which is a 404 on every whisper model
    /// download in Réglages → Transcription locale (v1.6.0's bug, fixed in
    /// v1.6.1). This entry point takes the URL as it is, and differs from the API
    /// one in three ways that matter for a host nobody here administers:
    ///
    /// - **`https` only.** A `http`, `file`, `ftp` or relative address is refused
    ///   before a byte moves: plaintext would let anything on the path rewrite a
    ///   model body (the pinned SHA-256 would catch it, but only after 874 MB),
    ///   and `file://` would read the person's disk. A URL carrying credentials
    ///   is refused too, so none can reach a log line.
    /// - it uses its own client, which carries NO cookie jar and sends no
    ///   `X-NS-Background` header: a neezer session never leaves the server it
    ///   belongs to (not on the first hop, not after a redirect), and a server
    ///   header means nothing to a CDN;
    /// - it follows the redirects such a host answers with (Hugging Face: `302`
    ///   to its own CDN, with a signed URL). A body is only accepted at the end
    ///   of that chain, a chain longer than ten hops is refused by the client
    ///   instead of looping, and [`pinned_client`]'s `https_only` refuses a hop
    ///   that would downgrade to plaintext.
    ///
    /// The lane, the Bulk token bucket and the stall watchdog are the same ones
    /// [`download_to`](Api::download_to) uses, so a model transfer still waits
    /// for what the person is doing (rule 1). A caller only passes a URL from
    /// its own compile-time table: this is not a proxy for a URL a provider or a
    /// server handed us (that would be an SSRF).
    pub async fn download_url_to<F>(
        &self,
        lane: Lane,
        url: &str,
        destination: &Path,
        on_progress: F,
        cancel: &AtomicBool,
    ) -> Result<DownloadReport, ApiError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let parsed = reqwest::Url::parse(url).map_err(|error| ApiError::Http {
            status: 0,
            message: format!("adresse du fichier invalide ({url}) : {error}"),
            code: Some("bad_url".to_owned()),
        })?;
        if parsed.scheme() != "https" {
            return Err(ApiError::Http {
                status: 0,
                message: format!(
                    "adresse du fichier refusée ({url}) : HTTPS seulement, jamais {}",
                    parsed.scheme()
                ),
                code: Some("insecure_url".to_owned()),
            });
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(ApiError::Http {
                status: 0,
                message: "adresse du fichier refusée : identifiants dans l’URL".to_owned(),
                code: Some("credentialed_url".to_owned()),
            });
        }
        self.download_from(
            Target {
                http: &self.external,
                api: false,
            },
            parsed.to_string(),
            lane,
            destination,
            on_progress,
            cancel,
        )
        .await
    }

    /// The body of both download entry points: one URL, one `.part` file, one
    /// verified rename.
    async fn download_from<F>(
        &self,
        target: Target<'_>,
        endpoint: String,
        lane: Lane,
        destination: &Path,
        mut on_progress: F,
        cancel: &AtomicBool,
    ) -> Result<DownloadReport, ApiError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let Target { http, api } = target;
        let partial = destination.with_extension(format!(
            "{}.part",
            destination
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("part")
        ));
        if let Some(parent) = destination.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            return Err(write_error(error));
        }
        let guard = self.gates.acquire(lane).await;
        // The API target's hops are followed HERE (and checked, and counted);
        // a pinned third-party host keeps its own client, which follows its own
        // redirects (`pinned_client`) and is not a screen's traffic.
        let mut endpoint = if api {
            self.hop(reqwest::Url::parse(&endpoint).map_err(|error| {
                ApiError::Invalid(format!("adresse refusée ({endpoint}) : {error}"))
            })?)?
            .to_string()
        } else {
            endpoint
        };
        let mut hops = 0usize;
        let (response, status, headers) = loop {
            let mut request = http
                .request(Method::GET, endpoint.clone())
                // A compressed body would be written compressed under the file's
                // name and the byte count would lie.
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .header(reqwest::header::ACCEPT, "*/*");
            if api && guard.lane().is_background() {
                request = request.header(BACKGROUND_HEADER, "1");
            }
            if api {
                self.stats.request(&self.tag, 0);
            }
            let mark = self.pin.as_ref().map(|pin| pin.mark());
            let response = request.send().await.map_err(|err| {
                if !api {
                    // A pinned host that does not answer says nothing about THIS
                    // server: the app stays online when a model host is down.
                    return if is_network_failure(&err) {
                        ApiError::Offline
                    } else {
                        ApiError::Http {
                            status: 0,
                            message: err.to_string(),
                            code: None,
                        }
                    };
                }
                self.send_error(&err, mark)
            })?;
            if api {
                self.note_reachable();
            }
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            if api {
                let current = reqwest::Url::parse(&endpoint)
                    .expect("an endpoint this function just built or checked");
                if let Some(next) = self.redirect(status, &headers, &current)? {
                    if hops == MAX_REDIRECTS {
                        return Err(ApiError::Invalid(format!(
                            "trop de redirections ({MAX_REDIRECTS} au plus)"
                        )));
                    }
                    endpoint = self.hop(next)?.to_string();
                    hops += 1;
                    continue;
                }
            }
            break (response, status, headers);
        };
        if !(200..300).contains(&status) {
            let text = response.text().await.unwrap_or_default();
            if api {
                self.stats.bytes_in(&self.tag, text.len() as u64);
            }
            return Err(if api {
                self.classify(status, &headers, &text, lane)
            } else {
                // Never `classify` here: a third-party `401` is not a neezer
                // session ending, and clearing the jar over it would log the
                // person out of their own server.
                external_error(status, &text)
            });
        }
        let content_type = header_text(&headers, reqwest::header::CONTENT_TYPE);
        let length = header_text(&headers, reqwest::header::CONTENT_LENGTH)
            .and_then(|value| value.parse::<u64>().ok());

        let mut response = response;
        let mut file = match std::fs::File::create(&partial) {
            Ok(file) => file,
            Err(error) => return Err(write_error(error)),
        };
        let mut written = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                drop(file);
                let _ = std::fs::remove_file(&partial);
                return Err(ApiError::Http {
                    status: 0,
                    message: "téléchargement annulé".to_owned(),
                    code: Some("cancelled".to_owned()),
                });
            }
            let next = tokio::time::timeout(STALL_TIMEOUT, response.chunk()).await;
            let chunk = match next {
                Ok(Ok(Some(chunk))) => chunk,
                Ok(Ok(None)) => break,
                // A body that stops, or a link that dies mid-transfer: the same
                // sentence the stream's own watchdog uses.
                Ok(Err(_)) | Err(_) => {
                    drop(file);
                    let _ = std::fs::remove_file(&partial);
                    if api {
                        self.note_unreachable();
                    }
                    return Err(ApiError::Offline);
                }
            };
            let len = chunk.len() as u64;
            if api {
                self.stats.bytes_in(&self.tag, len);
            }
            if let Err(error) = file.write_all(&chunk) {
                drop(file);
                let _ = std::fs::remove_file(&partial);
                return Err(write_error(error));
            }
            written += len;
            on_progress(written, length);
            if lane == Lane::Bulk {
                self.gates.throttle(len).await;
            }
        }
        drop(file);
        if let Err(error) = std::fs::rename(&partial, destination) {
            let _ = std::fs::remove_file(&partial);
            return Err(write_error(error));
        }
        drop(guard);
        Ok(DownloadReport {
            path: destination.to_path_buf(),
            bytes: written,
            content_type,
        })
    }

    /// Open one audio stream (`GET /api/stream/<id>?q=&start=`).
    ///
    /// `from_byte` asks for a byte range of a finished file (`Range`); `start`
    /// asks the server to begin encoding at that many seconds (`?start=`, one
    /// request, `X-NS-Start` says where it began). No retry here: a stream that
    /// stops is the player's reconnect decision, with its own byte range.
    ///
    /// The lane slot covers the REQUEST, up to the first byte of the answer, and
    /// is released before the body streams: what bounds a body is the player's
    /// read-ahead cap and, for Bulk, the token bucket in
    /// [`StreamResponse::chunk`], not a slot count.
    pub async fn stream(
        &self,
        lane: Lane,
        id: &TrackId,
        q: Quality,
        start: Option<f64>,
        from_byte: Option<u64>,
    ) -> Result<StreamResponse, ApiError> {
        let path = stream_path(id, q, start);
        let guard = self.gates.acquire(lane).await;
        let mut url = self.hop(self.endpoint(&path))?;
        // A stream is a GET with no body, so a `307`/`308` hops without
        // changing anything; the `Range`/`start` the caller asked for does,
        // because it describes the FILE, not the address.
        for hop in 0..=MAX_REDIRECTS {
            let mut request = self
                .http
                .request(Method::GET, url.clone())
                // A gzip'd body has no usable byte offsets, and a Range over it
                // is meaningless (`docs/agents/desktop.md`, D2 *Pitfalls*).
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .header(reqwest::header::ACCEPT, "*/*");
            if guard.lane().is_background() {
                request = request.header(BACKGROUND_HEADER, "1");
            }
            if let Some(byte) = from_byte {
                request = request.header(reqwest::header::RANGE, format!("bytes={byte}-"));
            }
            self.stats.request(&self.tag, 0);
            let mark = self.pin.as_ref().map(|pin| pin.mark());
            let response = request
                .send()
                .await
                .map_err(|err| self.send_error(&err, mark))?;
            self.note_reachable();
            let status = response.status().as_u16();
            let headers = response.headers().clone();

            if let Some(next) = self.redirect(status, &headers, &url)? {
                if hop == MAX_REDIRECTS {
                    return Err(ApiError::Invalid(format!(
                        "trop de redirections ({MAX_REDIRECTS} au plus)"
                    )));
                }
                url = self.hop(next)?;
                continue;
            }
            if !(200..300).contains(&status) {
                let text = response.text().await.unwrap_or_default();
                self.stats.bytes_in(&self.tag, text.len() as u64);
                return Err(self.classify(status, &headers, &text, lane));
            }
            let content_type = header_text(&headers, reqwest::header::CONTENT_TYPE);
            let content_length = header_text(&headers, reqwest::header::CONTENT_LENGTH)
                .and_then(|value| value.parse::<u64>().ok());
            let accept_ranges = header_text(&headers, reqwest::header::ACCEPT_RANGES);
            let content_range = header_text(&headers, reqwest::header::CONTENT_RANGE);
            let start_s =
                header_text(&headers, START_HEADER).and_then(|value| value.parse::<f64>().ok());
            drop(guard);
            return Ok(StreamResponse {
                status,
                content_type,
                content_length,
                accept_ranges,
                content_range,
                start_s,
                response,
                gates: Arc::clone(&self.gates),
                lane,
                stats: Arc::clone(&self.stats),
                tag: self.tag.clone(),
            });
        }
        // Ten hops without an answer, on a request the server keeps moving.
        Err(ApiError::Invalid(format!(
            "trop de redirections ({MAX_REDIRECTS} au plus)"
        )))
    }

    /// `GET /api/stream/<id>/ready?q=` — what a stream would answer right now.
    /// Cheap by contract: it never starts a download.
    pub async fn ready(&self, id: &TrackId, q: Quality) -> Result<StreamReady, ApiError> {
        let mut path = format!("/stream/{}/ready", encode(id.as_str()));
        if let Some(wire) = q.wire() {
            path.push_str(&format!("?q={wire}"));
        }
        Ok(self.get(Lane::User, &path).await?.value)
    }

    /// One attempt at an answer, with the retry policy applied. `was_online`
    /// is the link's state when the FIRST attempt was decided, threaded through
    /// the retries: a request started while already dead fails fast instead of
    /// burning the whole backoff budget.
    ///
    /// The body comes back as BYTES and `want` only decides what is asked for:
    /// a screen reads JSON, a sleeve is an image (`Api::bytes`). One retry
    /// policy for both, because the rules of rule 3 are about the request, not
    /// about what the answer is written in.
    async fn fetch_want(
        &self,
        want: Want,
        lane: Lane,
        method: Method,
        path: &str,
        body: Option<&[u8]>,
        was_online: bool,
    ) -> Result<Vec<u8>, ApiError> {
        let attempts = if method == Method::GET {
            match lane {
                // A GET started online gets the SPA's three tries on a
                // connection failure or a transient 5xx.
                Lane::User if was_online => MAX_GET_ATTEMPTS,
                // A background request only ever repeats on the server's own
                // `Retry-After`: one more attempt, never a backoff storm.
                Lane::Prefetch | Lane::Bulk => 2,
                _ => 1,
            }
        } else {
            // Mutations never retry: replaying one could double it.
            1
        };

        let mut last = ApiError::Offline;
        for attempt in 0..attempts {
            match self.attempt(want, lane, method.clone(), path, body).await {
                Ok(bytes) => return Ok(bytes),
                Err(err) => {
                    let retry_in = match &err {
                        ApiError::Busy { retry_after }
                            if lane.is_background() && attempt + 1 < attempts =>
                        {
                            Some(Duration::from_secs(u64::from(*retry_after)))
                        }
                        ApiError::Offline | ApiError::Http { .. }
                            if lane == Lane::User
                                && method == Method::GET
                                && attempt + 1 < attempts
                                && is_transient(&err) =>
                        {
                            Some(backoff(attempt, self.jitter()))
                        }
                        _ => None,
                    };
                    let Some(delay) = retry_in else {
                        return Err(err);
                    };
                    tracing::debug!(
                        lane = lane.as_str(),
                        path,
                        status = err_status(&err),
                        wait_ms = delay.as_millis() as u64,
                        "retrying"
                    );
                    last = err;
                    tokio::time::sleep(delay).await;
                }
            }
        }
        Err(last)
    }

    /// Send one request (holding a lane slot) and classify what came back.
    ///
    /// The redirects are followed HERE, one hop at a time (`Net.kt`'s rule),
    /// not by reqwest: every hop goes through [`Api::hop`] before a socket is
    /// opened, so a `Location` pointing at another origin in cleartext is
    /// refused instead of dialled, and each hop is counted as the request it is.
    /// The semantics reqwest's own policy had are kept, so the desktop sees the
    /// same thing it always did: ten hops at most, `301`/`302`/`303` become a
    /// `GET` without a body, `307`/`308` keep both.
    async fn attempt(
        &self,
        want: Want,
        lane: Lane,
        method: Method,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<Vec<u8>, ApiError> {
        let guard = self.gates.acquire(lane).await;
        let mut url = self.hop(self.endpoint(path))?;
        let mut method = method;
        let mut body = body;
        for hop in 0..=MAX_REDIRECTS {
            let mut request = self
                .http
                .request(method.clone(), url.clone())
                .header(reqwest::header::ACCEPT, want.accept())
                .timeout(ANSWER_TIMEOUT);
            if guard.lane().is_background() {
                request = request.header(BACKGROUND_HEADER, "1");
            }
            if let Some(bytes) = body {
                request = request
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(bytes.to_vec());
            }
            self.stats
                .request(&self.tag, body.map_or(0, |bytes| bytes.len() as u64));
            let mark = self.pin.as_ref().map(|pin| pin.mark());
            let response = request
                .send()
                .await
                .map_err(|err| self.send_error(&err, mark))?;
            // Any answer at all — even a 401 — proves the server is reachable.
            self.note_reachable();
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            if let Some(next) = self.redirect(status, &headers, &url)? {
                if hop == MAX_REDIRECTS {
                    return Err(ApiError::Invalid(format!(
                        "trop de redirections ({MAX_REDIRECTS} au plus)"
                    )));
                }
                // The body of a `301`/`302`/`303` is not replayed (a browser
                // turns the follow-up into a plain `GET`); a `307`/`308` keeps
                // it, because the server said the request itself moved.
                if matches!(status, 301..=303) {
                    method = Method::GET;
                    body = None;
                }
                url = self.hop(next)?;
                continue;
            }
            let bytes = response.bytes().await.map_err(|err| {
                if is_network_failure(&err) {
                    self.note_unreachable();
                    ApiError::Offline
                } else {
                    ApiError::Http {
                        status,
                        message: err.to_string(),
                        code: None,
                    }
                }
            })?;
            self.stats.bytes_in(&self.tag, bytes.len() as u64);
            let bytes = bytes.to_vec();
            drop(guard);

            if (200..300).contains(&status) {
                return Ok(bytes);
            }
            // A refusal is written in JSON whatever was asked for; a body that
            // is not text still yields the server's status and headers.
            let body = String::from_utf8_lossy(&bytes).into_owned();
            return Err(self.classify(status, &headers, &body, lane));
        }
        Err(ApiError::Invalid(format!(
            "trop de redirections ({MAX_REDIRECTS} au plus)"
        )))
    }

    /// `{base}/api{path}` as a parsed URL — the first hop of every request.
    fn endpoint(&self, path: &str) -> reqwest::Url {
        // `Url::parse` cannot fail on a string this client built from a base it
        // already parsed, and a panic here would be a bug in this file, not in
        // the server's answer.
        reqwest::Url::parse(&self.url(path)).expect("a URL built from a parsed base")
    }

    /// The ONE gate every hop goes through (`Net.kt`'s rules, `Ssl.kt`'s scope).
    ///
    /// Cleartext is allowed to the configured origin and to nobody else: the
    /// desktop's own base may well be `http://192.168.1.10` (its manifest-free
    /// world has no such rule), but a redirect must not be able to take the
    /// session to a plaintext host, and a phone's app cannot trust the platform
    /// to police it. `http` is refused anywhere but the configured origin,
    /// before DNS, before a socket.
    fn hop(&self, url: reqwest::Url) -> Result<reqwest::Url, ApiError> {
        match url.scheme() {
            "https" => Ok(url),
            "http" if url.origin() == self.base_url.origin() => Ok(url),
            "http" => Err(ApiError::Invalid(format!(
                "adresse refusée ({url}) : le trafic en clair n’est permis que vers le serveur configuré"
            ))),
            other => Err(ApiError::Invalid(format!(
                "adresse refusée ({url}) : {other} n’est pas http ou https"
            ))),
        }
    }

    /// The next hop a `3xx` asks for, when it asks for one.
    ///
    /// `None` — not a redirect, or a `3xx` that carries no usable `Location`:
    /// the answer is then classified like any other (a `3xx` status, which is
    /// what reqwest's policy did with it too).
    fn redirect(
        &self,
        status: u16,
        headers: &HeaderMap,
        current: &reqwest::Url,
    ) -> Result<Option<reqwest::Url>, ApiError> {
        if !matches!(status, 301 | 302 | 303 | 307 | 308) {
            return Ok(None);
        }
        let Some(location) = header_text(headers, reqwest::header::LOCATION) else {
            return Ok(None);
        };
        current.join(&location).map(Some).map_err(|error| {
            ApiError::Invalid(format!("redirection illisible ({location}) : {error}"))
        })
    }

    /// A failed `send`: a certificate this client refused is said as such, and
    /// everything else keeps the classification it always had.
    fn send_error(&self, error: &reqwest::Error, mark: Option<u64>) -> ApiError {
        if let (Some(pin), Some(mark)) = (self.pin.as_ref(), mark)
            && let Some(refusal) = pin.certificate_error(mark)
        {
            // The server DID answer — with a certificate this client refused —
            // so the network indicator must not call the link dead.
            self.note_reachable();
            return refusal;
        }
        if is_network_failure(error) {
            self.note_unreachable();
            ApiError::Offline
        } else {
            ApiError::Http {
                status: 0,
                message: error.to_string(),
                code: None,
            }
        }
    }

    /// The SPA's own failure classification, in the same order.
    fn classify(&self, status: u16, headers: &HeaderMap, body: &str, lane: Lane) -> ApiError {
        if status == 401 {
            let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
            if parsed.get("error").and_then(|v| v.as_str()) == Some("remote ended") {
                // The loan ended, not the session: the server restored the
                // person's own account and minted a new cookie.
                return ApiError::RemoteEnded {
                    restored: parsed.get("restored").cloned().filter(|v| !v.is_null()),
                };
            }
            self.jar.clear(&self.base_url);
            self.set_owner(None);
            return ApiError::Unauthorized;
        }
        if let Some(code) = header_text(headers, NO_RETRY_HEADER) {
            let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
            let message = parsed
                .get("error")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("HTTP {status}"));
            let retry_in = parsed
                .get("retry_in")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .min(u64::from(u32::MAX)) as u32;
            return ApiError::Provider {
                status,
                code,
                message,
                retry_in,
            };
        }
        let retry_after =
            header_text(headers, RETRY_AFTER.as_str()).and_then(|value| value.parse::<u32>().ok());
        if status == 409 {
            // `neezer/webui` uses 409 for two different things, and the header
            // is what tells them apart: `_not_archived` ("nothing on disk yet, a
            // writer is at work") sets `Retry-After`, and every other 409 — a
            // playlist version that moved (`_playlist_conflict`), a track that
            // became available, a group that already exists, a finished file
            // asked for a `?start=` slice — does not. The first is a wait, the
            // second is a verdict: reading them the same way made a playlist
            // screen retry a stale order and called a real conflict "pas encore
            // prêt".
            return match retry_after {
                Some(retry_after) => ApiError::NotReady { retry_after },
                None => {
                    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
                    ApiError::Conflict {
                        message: parsed
                            .get("error")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("HTTP {status}")),
                        code: parsed
                            .get("code")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                        scope: parsed
                            .get("scope")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                    }
                }
            };
        }
        if lane.is_background()
            && matches!(status, 429 | 503)
            && let Some(retry_after) = retry_after
        {
            // `_admit_by_priority`, or a cold track queued at prefetch priority:
            // the server said when, exactly.
            return ApiError::Busy { retry_after };
        }
        let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
        let message = parsed
            .get("error")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if body.trim().is_empty() {
                    format!("HTTP {status}")
                } else {
                    body.chars().take(200).collect()
                }
            });
        let code = parsed
            .get("code")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        ApiError::Http {
            status,
            message,
            code,
        }
    }
}

impl ApiBuilder {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            cache: None,
            session: None,
            pin: None,
        }
    }

    /// The offline copy of cacheable GETs (`nz-cache`).
    pub fn cache(mut self, cache: Arc<dyn ResponseCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// A cookie jar, for a test that wants its own.
    pub fn session(mut self, jar: Arc<SessionJar>) -> Self {
        self.session = Some(jar);
        self
    }

    /// Mirror the session to `path` (`$XDG_STATE_HOME/neezer/session.json` by
    /// default; the app passes the real one).
    pub fn session_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.session = Some(Arc::new(SessionJar::at(path)));
        self
    }

    /// Install this client's trust-on-first-use pin (`Ssl.kt`'s model, SC3).
    ///
    /// `Some(leaf)` — the SHA-256 of the certificate the person accepted, as
    /// [`CertificateInfo::sha256`](crate::CertificateInfo::sha256) gave it (or
    /// [`parse_fingerprint`](crate::parse_fingerprint) read it back) — accepts
    /// the platform's own verdict **or** exactly that leaf for the configured
    /// origin, and nothing else; another leaf is
    /// [`ApiError::Certificate`] with `changed: true`.
    ///
    /// `None` installs the same verifier with no pin: the platform's verdict
    /// only, and a chain it refuses is [`ApiError::Certificate`] with
    /// `changed: false`, which is what the setup screen shows before anything
    /// has been pinned. That is the phone's app: it always asks, so a home
    /// server's self-signed certificate never looks like a dead link.
    ///
    /// **Not calling it at all keeps the client exactly as it was** — the
    /// desktop's case: no custom verifier, no extra TLS configuration, the
    /// platform's verdict and nothing else. The pin only ever ADDS one leaf for
    /// one host; it never relaxes another host and never removes a chain a real
    /// CA signed.
    pub fn pinned_leaf_sha256(mut self, pin: Option<[u8; 32]>) -> Self {
        self.pin = Some(pin);
        self
    }

    pub fn build(self) -> Result<Api, ApiError> {
        let base = self.base.trim_end_matches('/').to_owned();
        let base_url = reqwest::Url::parse(&base).map_err(|err| ApiError::Http {
            status: 0,
            message: format!("adresse du serveur invalide ({base}) : {err}"),
            code: None,
        })?;
        let jar = match self.session {
            Some(jar) => jar,
            None => Arc::new(SessionJar::default_jar()),
        };
        // The pin, when one was asked for. Its host is the configured origin's:
        // the only place a pin may relieve, or blame, a certificate.
        let pin = self
            .pin
            .map(|leaf| PinState::new(leaf, base_url.host_str().unwrap_or_default().to_owned()));
        let mut http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            // The jar, scoped to the configured origin: a redirect to another
            // host carries no cookie, and no off-origin `Set-Cookie` reaches the
            // person's session file (`Net.kt`'s rule).
            .cookie_provider(Arc::new(OriginJar {
                jar: Arc::clone(&jar),
                origin: base_url.clone(),
            }))
            .connect_timeout(CONNECT_TIMEOUT)
            // The hops are followed by hand (`Api::attempt`), which is the only
            // way each one can be checked before a socket opens.
            .redirect(reqwest::redirect::Policy::none())
            // No client-wide timeout: an API answer is bounded per request
            // (`ANSWER_TIMEOUT`) while a stream must be allowed to run.
            ;
        if let Some(pin) = pin.as_ref() {
            http = http.use_preconfigured_tls(pinned_client_config(pin)?);
        }
        let http = http.build().map_err(|err| ApiError::Http {
            status: 0,
            message: format!("client HTTP impossible : {err}"),
            code: None,
        })?;
        // The model hosts' client: the same timeouts, NO cookie jar (a neezer
        // session never leaves the server it belongs to), the redirects spelled
        // out, and `https_only` so a redirect cannot downgrade the transfer to
        // plaintext either.
        let external = pinned_client(true).map_err(|err| ApiError::Http {
            status: 0,
            message: format!("client HTTP impossible : {err}"),
            code: None,
        })?;
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0x5EED);
        let (online, _) = watch::channel(true);
        Ok(Api {
            base,
            base_url,
            http,
            external,
            jar,
            gates: Arc::new(Gates::new()),
            cache: self.cache.unwrap_or_else(|| Arc::new(NoCache)),
            owner: Arc::new(RwLock::new(None)),
            online,
            caching: Arc::new(AtomicBool::new(true)),
            jitter: Arc::new(AtomicU64::new(seed | 1)),
            jitter_seed: seed,
            tag: TAG_NONE.to_owned(),
            stats: Arc::new(Stats::default()),
            pin,
        })
    }
}

fn is_network_failure(err: &reqwest::Error) -> bool {
    err.is_connect() || err.is_timeout() || err.is_request() || err.is_body()
}

/// The SPA's `TRANSIENT` set: a restart, a cold worker.
fn is_transient(err: &ApiError) -> bool {
    match err {
        ApiError::Offline => true,
        ApiError::Http { status, .. } => matches!(status, 502..=504),
        _ => false,
    }
}

fn err_status(err: &ApiError) -> u16 {
    match err {
        ApiError::Provider { status, .. } | ApiError::Http { status, .. } => *status,
        _ => 0,
    }
}

fn header_text(headers: &HeaderMap, name: impl reqwest::header::AsHeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

/// `/stream/<id>?q=&start=` — the SPA's `api.streamUrl` grammar, shared by the
/// request and by the URL the app hands to anything else ([`crate`]'s
/// `stream_url`).
pub(crate) fn stream_path(id: &TrackId, q: Quality, start: Option<f64>) -> String {
    let mut path = format!("/stream/{}", encode(id.as_str()));
    let mut pairs: Vec<String> = Vec::new();
    if let Some(wire) = q.wire() {
        pairs.push(format!("q={wire}"));
    }
    // The SPA's own threshold: below 50 ms the server would slice for nothing.
    if let Some(seconds) = start.filter(|seconds| *seconds > 0.05) {
        pairs.push(format!("start={seconds:.3}"));
    }
    if !pairs.is_empty() {
        path.push('?');
        path.push_str(&pairs.join("&"));
    }
    path
}

/// `encodeURIComponent`, byte for byte: the SPA builds its paths with it, and a
/// style name with a space or an accent must land on the same route.
pub(crate) fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_base_url_must_parse_and_keeps_its_root() {
        let api = Api::new("http://127.0.0.1:5000/").expect("a valid base");
        assert_eq!(api.base_url(), "http://127.0.0.1:5000");
        assert_eq!(api.url("/me"), "http://127.0.0.1:5000/api/me");
        assert!(Api::new("not a url").is_err());
    }

    /// The paths the SPA builds with `encodeURIComponent`, checked against its
    /// output for the same inputs (a genre with a space, an id with a slash).
    #[test]
    fn the_path_encoder_matches_encode_uri_component() {
        assert_eq!(encode("Techno"), "Techno");
        assert_eq!(encode("Rap FR"), "Rap%20FR");
        assert_eq!(encode("Électro"), "%C3%89lectro");
        assert_eq!(encode("a/b?c=d&e"), "a%2Fb%3Fc%3Dd%26e");
        assert_eq!(encode("-_.!~*'()"), "-_.!~*'()");
    }

    #[test]
    fn the_client_state_starts_online_with_no_owner_and_no_cache() {
        let api = Api::new("http://127.0.0.1:5000").unwrap();
        assert!(api.is_online());
        assert_eq!(api.owner(), None);
        assert!(*api.online().borrow());
        api.set_online(false);
        assert!(!api.is_online());
        api.set_owner(Some("u1".into()));
        assert_eq!(api.owner().as_deref(), Some("u1"));
        // The Bulk cap follows the stream, never a client's own idea of urgency.
        assert_eq!(bulk_cap_from_stream(Some(800_000)), Some(200_000));
    }

    // --- `download_url_to`: v1.6.0's model downloads were 404s ----------------
    //
    // A loopback server cannot serve HTTPS (that would need a certificate
    // fixture and a second crypto provider), so the wire tests drive
    // `download_from` with `pinned_client(false)` — the PRODUCTION pinned client
    // with only `https_only` off — and the refusals `download_url_to` itself
    // enforces are asserted on their own. `pinned_client(true)` is what
    // `ApiBuilder::build` installs, and one test below proves it refuses
    // plaintext.

    /// One answer of the throwaway server below, plus what the client sent.
    struct Hop {
        status: u16,
        location: Option<&'static str>,
        body: Vec<u8>,
    }

    /// One request the server saw: its target, and its headers lower-cased.
    struct Seen {
        target: String,
        headers: String,
    }

    impl Seen {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{name}:")))
                .map(str::trim)
        }
    }

    /// A local HTTP server that answers `script` in order, one connection per
    /// answer (`Connection: close`), and sends back every request it saw. No port
    /// is assumed and nothing leaves the loopback.
    fn local_server(
        script: Vec<Hop>,
    ) -> (
        String,
        std::sync::mpsc::Receiver<Seen>,
        std::thread::JoinHandle<()>,
    ) {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let address = listener.local_addr().expect("address");
        let (sender, seen) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            for hop in script {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => head.push(byte[0]),
                    }
                }
                let text = String::from_utf8_lossy(&head).into_owned();
                let target = text
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                let _ = sender.send(Seen {
                    target,
                    headers: text.to_lowercase(),
                });
                let mut answer = format!(
                    "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                    hop.status,
                    hop.body.len()
                );
                if let Some(location) = hop.location {
                    answer.push_str(&format!("Location: {location}\r\n"));
                }
                answer.push_str("\r\n");
                let _ = stream.write_all(answer.as_bytes());
                let _ = stream.write_all(&hop.body);
            }
        });
        (format!("http://{address}"), seen, handle)
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nz-api-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// `download_from` with the pinned client on a loopback (plaintext) server.
    async fn pinned_get(
        api: &Api,
        url: &str,
        destination: &Path,
    ) -> Result<DownloadReport, ApiError> {
        api.download_from(
            Target {
                http: &pinned_client(false).expect("the test client"),
                api: false,
            },
            url.to_owned(),
            Lane::User,
            destination,
            |_, _| {},
            &AtomicBool::new(false),
        )
        .await
    }

    /// v1.6.0's bug, pinned: `download_to` builds `{base}/api{path}`, so a caller
    /// that handed it an absolute URL asked the NEZER SERVER for
    /// `/apihttps://huggingface.co/…` — a 404 on every whisper model in Réglages
    /// → Transcription locale. The pinned path asks the URL as it is, and follows
    /// the `302` the host answers with (that is how Hugging Face serves these
    /// files).
    #[tokio::test]
    async fn an_absolute_url_is_requested_as_it_is_and_its_redirect_is_followed() {
        let body = b"ggml-tiny model bytes".to_vec();
        let (base, seen, server) = local_server(vec![
            Hop {
                status: 302,
                location: Some("/cdn/ggml-tiny.bin?sig=1"),
                body: Vec::new(),
            },
            Hop {
                status: 200,
                location: None,
                body: body.clone(),
            },
        ]);
        let api = Api::new(&base).expect("a client");
        let dir = scratch("download");
        let destination = dir.join("ggml-tiny.bin");
        let url = format!("{base}/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin");
        let report = pinned_get(&api, &url, &destination)
            .await
            .expect("the redirect chain is followed to the body");
        assert_eq!(report.bytes, body.len() as u64);
        assert_eq!(std::fs::read(&destination).expect("the file"), body);
        assert_eq!(
            seen.recv().expect("the first request").target,
            "/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
            "the URL is used as it is: prefixed with /api it was a 404 on the neezer server"
        );
        assert_eq!(
            seen.recv().expect("the redirected request").target,
            "/cdn/ggml-tiny.bin?sig=1"
        );
        server.join().expect("the server thread");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Guardrail**: no Neezer session reaches a pinned host, before or after a
    /// redirect. The jar holds a cookie for the very origin being downloaded
    /// from, and the pinned client still sends none — while the API client, with
    /// the same jar and the same URL, does: the positive control that proves this
    /// test would catch a regression.
    #[tokio::test]
    async fn no_session_reaches_a_pinned_host_even_after_a_redirect() {
        use reqwest::cookie::CookieStore;

        let body = b"model bytes".to_vec();
        let (base, seen, server) = local_server(vec![
            Hop {
                status: 302,
                location: Some("/cdn/model.bin"),
                body: Vec::new(),
            },
            Hop {
                status: 200,
                location: None,
                body: body.clone(),
            },
            // The control: the API client asking the same origin.
            Hop {
                status: 200,
                location: None,
                body: Vec::new(),
            },
        ]);
        let jar = Arc::new(SessionJar::in_memory());
        let api = Api::builder(&base)
            .session(Arc::clone(&jar))
            .build()
            .expect("a client");
        // The session cookie the server would have set, for THIS origin.
        let value = reqwest::header::HeaderValue::from_static("session=topsecret; Path=/");
        let mut headers = std::iter::once(&value);
        let origin = reqwest::Url::parse(&base).expect("origin");
        jar.set_cookies(&mut headers, &origin);
        assert_eq!(
            CookieStore::cookies(&*jar, &origin)
                .expect("a cookie")
                .to_str()
                .expect("ascii"),
            "session=topsecret"
        );

        let dir = scratch("nocookie");
        let destination = dir.join("model.bin");
        pinned_get(&api, &format!("{base}/model.bin"), &destination)
            .await
            .expect("a complete download");
        for hop in ["the first hop", "the redirected hop"] {
            let request = seen.recv().expect(hop);
            assert!(
                request.header("cookie").is_none(),
                "{hop} carried a session cookie: {:?}",
                request.headers
            );
            assert!(
                request.header("authorization").is_none(),
                "{hop} carried an authorization header"
            );
            assert!(
                request.header("x-ns-background").is_none(),
                "{hop} carried a neezer server header to a third party"
            );
        }
        // The control: the SAME jar, through the API's own client, does send it —
        // so the assertions above are about the pinned client, not about a jar
        // that never held a cookie.
        api.http
            .get(format!("{base}/control"))
            .send()
            .await
            .expect("the control request");
        let control = seen.recv().expect("the control request");
        assert_eq!(
            control.header("cookie"),
            Some("session=topsecret"),
            "the control proves the cookie would be visible if it were sent"
        );
        server.join().expect("the server thread");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Guardrail**: HTTPS only, and nothing that is not a plain pinned URL. A
    /// `http`, `file`, `ftp` or relative address, or one carrying credentials, is
    /// refused before a byte moves; the production client refuses plaintext on
    /// its own too, so a redirect cannot downgrade either.
    #[tokio::test]
    async fn a_pinned_url_must_be_https_and_free_of_credentials() {
        let api = Api::new("http://127.0.0.1:1").expect("a client");
        let dir = scratch("schemes");
        for bad in [
            "http://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
            "file:///etc/passwd",
            "ftp://huggingface.co/x.bin",
            "/ggerganov/x.bin",
            "https://user:secret@huggingface.co/x.bin",
            "not a url",
        ] {
            let refused = api
                .download_url_to(
                    Lane::User,
                    bad,
                    &dir.join("x.bin"),
                    |_, _| {},
                    &AtomicBool::new(false),
                )
                .await;
            assert!(refused.is_err(), "{bad} must be refused");
            assert!(
                !dir.join("x.bin").exists() && !dir.join("x.bin.part").exists(),
                "{bad} wrote a file"
            );
        }
        // The client the production path uses refuses plaintext by itself.
        let client = pinned_client(true).expect("the pinned client");
        let plain = client.get("http://127.0.0.1:1/x.bin").send().await;
        assert!(
            plain.is_err(),
            "`https_only` must refuse a plaintext request"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A refused model host says its status and leaves nothing behind — and a
    /// third-party status is never the neezer session's: a `401` from a model
    /// host must not clear the cookie jar and log the person out.
    #[tokio::test]
    async fn a_pinned_host_refusal_is_a_plain_status_and_keeps_the_session() {
        let (base, _seen, server) = local_server(vec![
            Hop {
                status: 404,
                location: None,
                body: b"Entry not found".to_vec(),
            },
            Hop {
                status: 401,
                location: None,
                body: b"unauthorized".to_vec(),
            },
        ]);
        let api = Api::new(&base).expect("a client");
        api.set_owner(Some("u1".to_owned()));
        let dir = scratch("refused");
        let destination = dir.join("ggml-gone.bin");
        let error = pinned_get(&api, &format!("{base}/ggml-gone.bin"), &destination)
            .await
            .expect_err("404");
        match error {
            ApiError::Http {
                status, message, ..
            } => {
                assert_eq!(status, 404);
                assert!(message.contains("Entry not found"), "{message}");
            }
            other => panic!("a third-party 404 is a plain HTTP failure: {other:?}"),
        }
        assert!(
            !destination.exists() && !dir.join("ggml-gone.bin.part").exists(),
            "a refused download writes nothing"
        );
        let unauthorized = pinned_get(&api, &format!("{base}/x.bin"), &dir.join("x.bin"))
            .await
            .expect_err("401");
        assert!(
            matches!(unauthorized, ApiError::Http { status: 401, .. }),
            "{unauthorized:?}"
        );
        assert_eq!(
            api.owner().as_deref(),
            Some("u1"),
            "the session is untouched"
        );
        assert!(api.is_online(), "a third-party failure is not our server");
        server.join().expect("the server thread");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
