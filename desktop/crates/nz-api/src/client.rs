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

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use reqwest::Method;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::watch;

use crate::cache_trait::{NoCache, ResponseCache, is_cacheable};
use crate::error::{ApiError, MAX_GET_ATTEMPTS, backoff};
use crate::lanes::{Gates, Lane, bulk_cap_from_stream};
use crate::session::SessionJar;

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

/// An IO failure, said in the words a screen can print.
fn write_error(error: std::io::Error) -> ApiError {
    ApiError::Http {
        status: 0,
        message: format!("écriture impossible : {error}"),
        code: Some("io".to_owned()),
    }
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
pub struct Api {
    base: String,
    base_url: reqwest::Url,
    http: reqwest::Client,
    jar: Arc<SessionJar>,
    gates: Arc<Gates>,
    cache: Arc<dyn ResponseCache>,
    owner: RwLock<Option<String>>,
    online: watch::Sender<bool>,
    caching: AtomicBool,
    jitter: AtomicU64,
    jitter_seed: u64,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the cookie jar's contents, never a token.
        f.debug_struct("Api")
            .field("base", &self.base)
            .field("owner", &self.owner())
            .field("online", &self.is_online())
            .finish()
    }
}

/// Builds an [`Api`]: the app adds the cache (`nz-cache`), the session file and
/// the base URL; a test may add an in-memory cache.
pub struct ApiBuilder {
    base: String,
    cache: Option<Arc<dyn ResponseCache>>,
    session: Option<Arc<SessionJar>>,
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
        mut on_progress: F,
        cancel: &AtomicBool,
    ) -> Result<DownloadReport, ApiError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
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
        let mut request = self
            .http
            .request(Method::GET, self.url(path))
            // A compressed body would be written compressed under the file's
            // name and the byte count would lie.
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .header(reqwest::header::ACCEPT, "*/*");
        if guard.lane().is_background() {
            request = request.header(BACKGROUND_HEADER, "1");
        }
        let response = request.send().await.map_err(|err| {
            if is_network_failure(&err) {
                self.note_unreachable();
                ApiError::Offline
            } else {
                ApiError::Http {
                    status: 0,
                    message: err.to_string(),
                    code: None,
                }
            }
        })?;
        self.note_reachable();
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        if !(200..300).contains(&status) {
            let text = response.text().await.unwrap_or_default();
            return Err(self.classify(status, &headers, &text, lane));
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
                    self.note_unreachable();
                    return Err(ApiError::Offline);
                }
            };
            let len = chunk.len() as u64;
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
        let mut request = self
            .http
            .request(Method::GET, self.url(&path))
            // A gzip'd body has no usable byte offsets, and a Range over it is
            // meaningless (`docs/agents/desktop.md`, D2 *Pitfalls*).
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .header(reqwest::header::ACCEPT, "*/*");
        if guard.lane().is_background() {
            request = request.header(BACKGROUND_HEADER, "1");
        }
        if let Some(byte) = from_byte {
            request = request.header(reqwest::header::RANGE, format!("bytes={byte}-"));
        }
        let response = request.send().await.map_err(|err| {
            if is_network_failure(&err) {
                self.note_unreachable();
                ApiError::Offline
            } else {
                ApiError::Http {
                    status: 0,
                    message: err.to_string(),
                    code: None,
                }
            }
        })?;
        self.note_reachable();
        let status = response.status().as_u16();
        let headers = response.headers().clone();

        if !(200..300).contains(&status) {
            let text = response.text().await.unwrap_or_default();
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
        Ok(StreamResponse {
            status,
            content_type,
            content_length,
            accept_ranges,
            content_range,
            start_s,
            response,
            gates: Arc::clone(&self.gates),
            lane,
        })
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
    async fn attempt(
        &self,
        want: Want,
        lane: Lane,
        method: Method,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<Vec<u8>, ApiError> {
        let guard = self.gates.acquire(lane).await;
        let mut request = self
            .http
            .request(method.clone(), self.url(path))
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
        let response = request.send().await.map_err(|err| {
            if is_network_failure(&err) {
                self.note_unreachable();
                ApiError::Offline
            } else {
                ApiError::Http {
                    status: 0,
                    message: err.to_string(),
                    code: None,
                }
            }
        })?;
        // Any answer at all — even a 401 — proves the server is reachable.
        self.note_reachable();
        let status = response.status().as_u16();
        let headers = response.headers().clone();
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
        let bytes = bytes.to_vec();
        drop(guard);

        if (200..300).contains(&status) {
            return Ok(bytes);
        }
        // A refusal is written in JSON whatever was asked for; a body that is
        // not text still yields the server's status and headers.
        let body = String::from_utf8_lossy(&bytes).into_owned();
        Err(self.classify(status, &headers, &body, lane))
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
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .cookie_provider(Arc::clone(&jar))
            .connect_timeout(CONNECT_TIMEOUT)
            // No client-wide timeout: an API answer is bounded per request
            // (`ANSWER_TIMEOUT`) while a stream must be allowed to run.
            .build()
            .map_err(|err| ApiError::Http {
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
            jar,
            gates: Arc::new(Gates::new()),
            cache: self.cache.unwrap_or_else(|| Arc::new(NoCache)),
            owner: RwLock::new(None),
            online,
            caching: AtomicBool::new(true),
            jitter: AtomicU64::new(seed | 1),
            jitter_seed: seed,
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
}
