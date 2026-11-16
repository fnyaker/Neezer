//! The enrolled-client worker: the token on disk, the HTTP transport of the
//! server's worker routes — owned by lot T3.
//!
//! T1 pinned the routes and the two numbers of the loop that are not in a
//! payload ([`HEARTBEAT_SECONDS`], [`PROTOCOL`]). T3 adds what the container's
//! worker (`docker/transcribe/worker.py`) does with them: the bearer device
//! token, bounded exponential backoff on a rate limit or a server fault, a
//! resumable audio download, and the rule that the token appears in no log, no
//! error and no `Debug` output — an ARL-grade credential is not a string to
//! print because a test failed.
//!
//! The token is stored by the app when the person enrols this PC
//! (`POST /api/transcribe/workers`, shown once), at
//! `$XDG_CONFIG_HOME/neezer/transcribe.token`, mode `0600`. This module only
//! reads it: a transcription worker that could *write* a credential would be a
//! second place a token leaks from.

use std::fmt;
use std::path::Path;

use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde_json::Value;
use tokio::time::sleep;

use crate::protocol::{
    AUDIO_TIMEOUT_SECONDS, Beat, CLAIM, CONNECT_TIMEOUT_SECONDS, ChunkBody, ChunkReply, Claim,
    CompleteBody, CompleteReply, FailBody, LEASE_HEADER, MAX_AUDIO_BYTES, MAX_AUDIO_REQUESTS,
    READ_TIMEOUT_SECONDS, RETRY_AFTER_CAP_SECONDS, RETRY_ATTEMPTS, RETRY_BASE_SECONDS,
    RETRY_MAX_SECONDS, audio, chunk, complete, fail, heartbeat, is_episode_id, release,
};

/// Seconds between two heartbeats (`worker.heartbeat_seconds`, and the
/// `HEARTBEAT_WRITE_SECONDS` the server keeps a lease valid with).
///
/// A lease lives 15 minutes, but the *server* only rewrites it every 60 s, so a
/// 5 s beat is twelve chances per write: one dropped or slow request never loses
/// a job, and a `SIGSTOP`ped engine keeps its lease for the whole pause.
pub const HEARTBEAT_SECONDS: u64 = 5;

/// The protocol version this client speaks (`worker.PROTOCOL`).
pub const PROTOCOL: u32 = 1;

/// The file name the app writes the device token to, under the config dir.
pub const TOKEN_FILE: &str = "transcribe.token";

/// `$XDG_CONFIG_HOME/neezer/transcribe.token`, mode `0600` (the note's path).
///
/// Resolved from the environment rather than through a crate: the three
/// directories this crate needs are three `join`s, and a `directories` call
/// would have to be kept equal to the note's text.
pub fn token_path() -> std::path::PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config")
        .join("neezer")
        .join(TOKEN_FILE)
}

/// `$XDG_DATA_HOME/neezer/transcripts/`: where a local transcript lands.
pub fn transcripts_dir() -> std::path::PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share")
        .join("neezer")
        .join("transcripts")
}

/// `$XDG_CACHE_HOME/neezer/`: scratch space (the downloaded audio, the PCM).
pub fn cache_dir() -> std::path::PathBuf {
    xdg_dir("XDG_CACHE_HOME", ".cache").join("neezer")
}

/// One XDG directory: the environment's value when it is absolute, its `HOME`
/// fallback otherwise.
fn xdg_dir(variable: &str, fallback: &str) -> std::path::PathBuf {
    if let Some(value) = std::env::var_os(variable) {
        let path = std::path::PathBuf::from(value);
        if path.is_absolute() {
            return path;
        }
    }
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
        .join(fallback)
}

/// The `User-Agent` of every request, as `worker.Client` sends it.
fn user_agent() -> String {
    format!("neezer-transcriber/{PROTOCOL}")
}

/// A device token, and the promise that it is never printed.
///
/// Deliberately not a `String`: this type exists so that the one credential of
/// the worker cannot end up in a `tracing` field, an error message or a test's
/// assertion output by accident. [`fmt::Debug`] is the redaction; the real value
/// has one accessor and the transport is its only caller.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// A token, checked against the server's own limits
    /// (`worker.Config.from_env`: at most 200 characters, no whitespace).
    pub fn new(raw: impl Into<String>) -> Result<Token, Failure> {
        let raw = raw.into();
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.len() > 200 || trimmed.chars().any(char::is_whitespace) {
            return Err(Failure::Config(
                "le jeton d'appareil est invalide".to_owned(),
            ));
        }
        Ok(Token(trimmed.to_owned()))
    }

    /// Read a token file and check it.
    ///
    /// A file readable by others is a warning, not a refusal: the credential is
    /// still the person's own, and refusing to transcribe because a mode is
    /// wide would be a bug report about the wrong thing.
    pub fn from_file(path: &Path) -> Result<Token, Failure> {
        let raw = std::fs::read_to_string(path).map_err(|error| {
            Failure::Config(format!(
                "jeton illisible dans {} ({})",
                path.display(),
                error.kind()
            ))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = std::fs::metadata(path)
                && metadata.permissions().mode() & 0o077 != 0
            {
                tracing::warn!(
                    "le jeton de transcription est lisible par d'autres ({:o}) ; chmod 600 {}",
                    metadata.permissions().mode() & 0o777,
                    path.display()
                );
            }
        }
        Token::new(raw)
    }

    /// The credential, for the one place that puts it in a header.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Token(<jeton>)")
    }
}

/// What a worker request can fail with, under the server's own names.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Failure {
    /// The configuration is not usable: no URL, a bad token, no model.
    #[error("configuration : {0}")]
    Config(String),
    /// The device token was refused or revoked: nothing else will work.
    #[error("appareil refusé ({status}) : {message}")]
    Auth {
        /// `401` or `403`.
        status: u16,
        /// The server's code, `unauthorized` or `worker_revoked`.
        code: String,
        /// The server's French sentence.
        message: String,
    },
    /// A final answer that is not a success.
    #[error("le serveur a répondu {status} : {message}")]
    Http {
        /// The status code.
        status: u16,
        /// The server's code (`lease_expired`, `chunk_out_of_order`, …).
        code: String,
        /// The server's sentence.
        message: String,
    },
    /// The server could not be reached, after every attempt.
    #[error("serveur injoignable : {0}")]
    Network(String),
    /// The claim is not one this client may act on.
    #[error("claim invalide : {0}")]
    BadClaim(String),
    /// The job was stopped by the person, the app or the server.
    #[error("transcription interrompue")]
    Cancelled,
    /// The lease is gone (expired, released, or the worker was revoked).
    #[error("lease perdu")]
    LeaseLost,
    /// The episode's audio could not be downloaded.
    #[error("audio de l'épisode : {0}")]
    Audio(String),
    /// The engine could not produce a chunk (a missing model, a dead child, a
    /// JSON that is not one): the crate's own failure, unchanged.
    #[error("{0}")]
    Engine(#[from] crate::error::Error),
    /// `POST …/complete` answered `409 incomplete`: the server holds less than
    /// the whole episode and says where it is.
    #[error("le transcript s'arrête à {done_ms} ms")]
    Incomplete {
        /// The server's own position.
        done_ms: i64,
    },
}

impl Failure {
    /// The `code` a `POST …/fail` would store, when the job is one episode's.
    pub fn code(&self) -> &'static str {
        match self {
            Failure::Config(_) => "config",
            Failure::Auth { .. } => "unauthorized",
            Failure::Http { .. } => "exit",
            Failure::Network(_) => "network",
            Failure::BadClaim(_) => "bad_claim",
            Failure::Cancelled => "cancelled",
            Failure::LeaseLost => "lease_expired",
            Failure::Audio(_) => "audio_http",
            Failure::Engine(inner) => inner.code(),
            Failure::Incomplete { .. } => "incomplete",
        }
    }

    /// Whether the worker can continue at all: a refused device cannot.
    pub fn fatal(&self) -> bool {
        matches!(self, Failure::Auth { .. } | Failure::Config(_))
    }

    /// The `409` code the server answers for a lease it no longer holds.
    pub fn lease_lost(&self) -> bool {
        matches!(self, Failure::LeaseLost)
            || matches!(
                self,
                Failure::Http { status: 409, code, .. }
                    if code == "lease_expired" || code == "lease_required"
            )
    }
}

/// The retry schedule of one client, in seconds.
///
/// The defaults are `worker.py`'s; the tests shrink them, so a backoff test
/// proves the SHAPE (a bounded, growing wait) without spending fifteen seconds
/// asleep.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Retry {
    /// Attempts per request, the first included.
    pub attempts: u32,
    /// First step.
    pub base: f64,
    /// Largest step.
    pub max: f64,
    /// Largest `Retry-After` honoured.
    pub retry_after_cap: f64,
}

impl Default for Retry {
    fn default() -> Retry {
        Retry {
            attempts: RETRY_ATTEMPTS,
            base: RETRY_BASE_SECONDS,
            max: RETRY_MAX_SECONDS,
            retry_after_cap: RETRY_AFTER_CAP_SECONDS,
        }
    }
}

impl Retry {
    /// The wait before the next attempt.
    pub fn delay(&self, attempt: u32, retry_after: Option<&str>) -> std::time::Duration {
        let exponent = attempt.saturating_sub(1).min(20) as i32;
        let mut seconds = (self.base * 2f64.powi(exponent)).min(self.max);
        if let Some(raw) = retry_after
            && let Ok(value) = raw.trim().parse::<f64>()
            && value.is_finite()
        {
            seconds = seconds.max(value.clamp(0.0, self.retry_after_cap));
        }
        std::time::Duration::from_secs_f64(seconds.max(0.0))
    }
}

/// The server's worker routes, as a typed client.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: Token,
    retry: Retry,
}

impl fmt::Debug for Client {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Client")
            .field("base", &self.base)
            .field("token", &self.token)
            .field("retry", &self.retry)
            .finish()
    }
}

impl Client {
    /// A client for `base` (an `http(s)` origin, no path) with `token`.
    pub fn new(base: &str, token: Token) -> Result<Client, Failure> {
        Client::with_retry(base, token, Retry::default())
    }

    /// The same, with a retry schedule of the caller's (the tests').
    pub fn with_retry(base: &str, token: Token, retry: Retry) -> Result<Client, Failure> {
        let base = normalize_base(base)?;
        let http = reqwest::Client::builder()
            .user_agent(user_agent())
            .connect_timeout(std::time::Duration::from_secs(CONNECT_TIMEOUT_SECONDS))
            .timeout(std::time::Duration::from_secs(READ_TIMEOUT_SECONDS))
            // `worker.py` sends `allow_redirects=False`: a redirect is not an
            // answer from the server this token belongs to, and following one
            // would hand the credential to another host.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| Failure::Config(format!("client HTTP : {error}")))?;
        Ok(Client {
            http,
            base,
            token,
            retry,
        })
    }

    /// The base URL this client talks to (no trailing slash).
    pub fn base(&self) -> &str {
        &self.base
    }

    /// `POST …/claim`: the next job, or `None` when the queue is empty (`204`).
    pub async fn claim(&self) -> Result<Option<Claim>, Failure> {
        let answer = self
            .request(Method::POST, CLAIM, None, None, &[200, 204])
            .await?;
        let Some(body) = answer else {
            return Ok(None);
        };
        let claim: Claim = serde_json::from_value(body)
            .map_err(|error| Failure::BadClaim(format!("réponse illisible ({error})")))?;
        claim.checked().map(Some).map_err(Failure::BadClaim)
    }

    /// `POST …/<eid>/heartbeat`: keep the lease, learn the server's decisions.
    pub async fn beat(
        &self,
        episode_id: &str,
        lease: &str,
        chunk_pct: Option<f64>,
    ) -> Result<Beat, Failure> {
        let path = heartbeat(episode_id)
            .ok_or_else(|| Failure::BadClaim("identifiant d'épisode invalide".to_owned()))?;
        let body = match chunk_pct {
            // `worker.Client.heartbeat`: one decimal, never a percentage of NaN.
            Some(pct) => {
                serde_json::json!({ "chunk_pct": (pct.clamp(0.0, 100.0) * 10.0).round() / 10.0 })
            }
            None => serde_json::json!({}),
        };
        let answer = self
            .request(Method::POST, &path, Some(&body), Some(lease), &[200])
            .await?;
        match answer {
            Some(value) => serde_json::from_value(value).map_err(|error| Failure::Http {
                status: 200,
                code: String::new(),
                message: format!("heartbeat illisible ({error})"),
            }),
            None => Ok(Beat::default()),
        }
    }

    /// `POST …/<eid>/chunk`: one chunk's lines.
    pub async fn send_chunk(
        &self,
        episode_id: &str,
        lease: &str,
        body: &ChunkBody,
    ) -> Result<ChunkReply, Failure> {
        let path = chunk(episode_id)
            .ok_or_else(|| Failure::BadClaim("identifiant d'épisode invalide".to_owned()))?;
        let answer = self
            .request(
                Method::POST,
                &path,
                Some(&to_value(body)?),
                Some(lease),
                &[200],
            )
            .await?;
        match answer {
            Some(value) => serde_json::from_value(value).map_err(|error| Failure::Http {
                status: 200,
                code: String::new(),
                message: format!("réponse de chunk illisible ({error})"),
            }),
            None => Ok(ChunkReply::default()),
        }
    }

    /// `POST …/<eid>/complete`: the transcript is whole.
    pub async fn send_complete(
        &self,
        episode_id: &str,
        lease: &str,
        body: &CompleteBody,
    ) -> Result<CompleteReply, Failure> {
        let path = complete(episode_id)
            .ok_or_else(|| Failure::BadClaim("identifiant d'épisode invalide".to_owned()))?;
        let answer = self
            .request(
                Method::POST,
                &path,
                Some(&to_value(body)?),
                Some(lease),
                &[200],
            )
            .await?;
        match answer {
            Some(value) => serde_json::from_value(value).map_err(|error| Failure::Http {
                status: 200,
                code: String::new(),
                message: format!("réponse de fin illisible ({error})"),
            }),
            None => Ok(CompleteReply::default()),
        }
    }

    /// `POST …/<eid>/fail`: store the cause the person will read.
    pub async fn send_fail(
        &self,
        episode_id: &str,
        lease: &str,
        body: &FailBody,
    ) -> Result<(), Failure> {
        let path = fail(episode_id)
            .ok_or_else(|| Failure::BadClaim("identifiant d'épisode invalide".to_owned()))?;
        self.request(
            Method::POST,
            &path,
            Some(&to_value(body)?),
            Some(lease),
            &[200],
        )
        .await
        .map(|_| ())
    }

    /// `POST …/<eid>/release`: give the lease back without a verdict.
    pub async fn send_release(&self, episode_id: &str, lease: &str) -> Result<(), Failure> {
        let path = release(episode_id)
            .ok_or_else(|| Failure::BadClaim("identifiant d'épisode invalide".to_owned()))?;
        self.request(
            Method::POST,
            &path,
            Some(&serde_json::json!({})),
            Some(lease),
            &[200],
        )
        .await
        .map(|_| ())
    }

    /// `GET …/audio/<eid>` into `dest`, resuming what is already there.
    ///
    /// The episode is already on disk? A `Range: bytes=<len>-` request asks only
    /// for the rest, exactly as `worker.download_audio` does: a resumed job on a
    /// 150 MB episode does not download it again. The body is written as it
    /// arrives, so a 1 GiB cap is enforced on the bytes actually received, not on
    /// a `Content-Length` a server could lie about.
    pub async fn download(
        &self,
        episode_id: &str,
        lease: &str,
        dest: &Path,
        stopped: impl Fn() -> bool + Send + Sync,
    ) -> Result<u64, Failure> {
        if !is_episode_id(episode_id) {
            return Err(Failure::BadClaim(
                "identifiant d'épisode invalide".to_owned(),
            ));
        }
        let path = audio(episode_id).expect("an id that passed the check has a route");
        let mut requests = 0u32;
        let mut total: Option<u64> = None;
        loop {
            if stopped() {
                return Err(Failure::Cancelled);
            }
            requests += 1;
            if requests > MAX_AUDIO_REQUESTS {
                return Err(Failure::Audio(
                    "trop de reprises de téléchargement".to_owned(),
                ));
            }
            let offset = std::fs::metadata(dest).map(|meta| meta.len()).unwrap_or(0);
            let request = self
                .http
                .get(format!("{}{path}", self.base))
                .header(
                    reqwest::header::AUTHORIZATION,
                    format!("Bearer {}", self.token.expose()),
                )
                .header(LEASE_HEADER, lease)
                .header(reqwest::header::RANGE, format!("bytes={offset}-"))
                .timeout(std::time::Duration::from_secs(AUDIO_TIMEOUT_SECONDS));
            let response = match request.send().await {
                Ok(response) => response,
                Err(error) => {
                    if requests >= self.retry.attempts {
                        return Err(Failure::Audio(short(&error)));
                    }
                    sleep(self.retry.delay(requests, None)).await;
                    continue;
                }
            };
            let status = response.status();
            if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                let (code, message, _) = read_error(response).await;
                if status == StatusCode::FORBIDDEN && code == "lease_expired" {
                    return Err(Failure::LeaseLost);
                }
                return Err(Failure::Auth {
                    status: status.as_u16(),
                    code: if code.is_empty() {
                        "unauthorized".to_owned()
                    } else {
                        code
                    },
                    message,
                });
            }
            if status == StatusCode::NOT_FOUND || status == StatusCode::GONE {
                return Err(Failure::Audio("l'audio de l'épisode est absent".to_owned()));
            }
            if status == StatusCode::RANGE_NOT_SATISFIABLE {
                // `bytes */<len>`: the file is already complete.
                let complete_at = response
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.rsplit('/').next())
                    .and_then(|value| value.parse::<u64>().ok());
                if complete_at == Some(offset) {
                    return Ok(offset);
                }
                let _ = std::fs::remove_file(dest);
                continue;
            }
            if status.is_client_error() && status != StatusCode::TOO_MANY_REQUESTS {
                return Err(Failure::Audio(format!("HTTP {}", status.as_u16())));
            }
            let append = match status {
                StatusCode::OK => {
                    total = response
                        .headers()
                        .get(reqwest::header::CONTENT_LENGTH)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u64>().ok())
                        .or(total);
                    false
                }
                StatusCode::PARTIAL_CONTENT => {
                    let (from, to, len) = content_range(response.headers());
                    if from != Some(offset) {
                        return Err(Failure::Audio("réponse Range incohérente".to_owned()));
                    }
                    if let Some(len) = len {
                        total = Some(len);
                    }
                    if let Some(to) = to
                        && offset > to
                    {
                        return Err(Failure::Audio("réponse Range incohérente".to_owned()));
                    }
                    true
                }
                status if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS => {
                    if requests >= self.retry.attempts {
                        return Err(Failure::Audio(format!("HTTP {}", status.as_u16())));
                    }
                    let retry_after = response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned);
                    sleep(self.retry.delay(requests, retry_after.as_deref())).await;
                    continue;
                }
                other => return Err(Failure::Audio(format!("HTTP {}", other.as_u16()))),
            };
            if let Some(total) = total
                && total > MAX_AUDIO_BYTES
            {
                return Err(Failure::Audio(
                    "l'audio de l'épisode dépasse 1 Gio".to_owned(),
                ));
            }
            let mut written = write_body(response, dest, append, offset, &stopped).await?;
            if let Some(total) = total {
                if written > total {
                    return Err(Failure::Audio("audio plus long qu'annoncé".to_owned()));
                }
                if written < total {
                    if requests >= self.retry.attempts {
                        return Err(Failure::Audio("téléchargement incomplet".to_owned()));
                    }
                    sleep(self.retry.delay(requests, None)).await;
                    continue;
                }
            }
            if written == 0 && total.is_none() {
                return Err(Failure::Audio("réponse audio vide".to_owned()));
            }
            // One entry, one file: the caller renames it into its work
            // directory once the byte count is the length the server promised.
            written = std::fs::metadata(dest)
                .map(|meta| meta.len())
                .unwrap_or(written);
            return Ok(written);
        }
    }

    /// The transport, with the retry policy of `worker.Client.request`.
    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        lease: Option<&str>,
        expected: &[u16],
    ) -> Result<Option<Value>, Failure> {
        if !path.starts_with('/') {
            return Err(Failure::BadClaim(format!("chemin invalide : {path}")));
        }
        let url = format!("{}{path}", self.base);
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let mut request = self
                .http
                .request(method.clone(), &url)
                .header(
                    reqwest::header::AUTHORIZATION,
                    format!("Bearer {}", self.token.expose()),
                )
                .header(reqwest::header::ACCEPT, "application/json");
            if let Some(lease) = lease {
                request = request.header(LEASE_HEADER, lease);
            }
            if let Some(body) = body {
                let bytes = serde_json::to_vec(body)
                    .map_err(|error| Failure::Config(format!("corps JSON impossible ({error})")))?;
                request = request
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(bytes);
            }
            let response = match request.send().await {
                Ok(response) => response,
                Err(error) => {
                    if attempt >= self.retry.attempts {
                        return Err(Failure::Network(short(&error)));
                    }
                    tracing::warn!(
                        "{} {} : réseau ({}), tentative {}/{}",
                        method,
                        path,
                        error,
                        attempt,
                        self.retry.attempts
                    );
                    sleep(self.retry.delay(attempt, None)).await;
                    continue;
                }
            };
            let status = response.status();
            if expected.contains(&status.as_u16()) {
                if status == StatusCode::NO_CONTENT || response.content_length() == Some(0) {
                    return Ok(None);
                }
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|error| Failure::Network(format!("corps illisible ({})", error)))?;
                if bytes.is_empty() {
                    return Ok(None);
                }
                return match serde_json::from_slice::<Value>(&bytes) {
                    Ok(value) => Ok(Some(value)),
                    Err(_) => Ok(None),
                };
            }
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let (code, message, done_ms) = read_error(response).await;
            if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                return Err(Failure::Auth {
                    status: status.as_u16(),
                    code: if code.is_empty() {
                        "unauthorized".to_owned()
                    } else {
                        code
                    },
                    message: if message.is_empty() {
                        "jeton refusé".to_owned()
                    } else {
                        message
                    },
                });
            }
            if crate::protocol::is_transient(status.as_u16()) {
                if attempt >= self.retry.attempts {
                    return Err(Failure::Http {
                        status: status.as_u16(),
                        code,
                        message: if message.is_empty() {
                            format!("HTTP {}", status.as_u16())
                        } else {
                            message
                        },
                    });
                }
                tracing::warn!(
                    "{} {} : HTTP {}, tentative {}/{}",
                    method,
                    path,
                    status.as_u16(),
                    attempt,
                    self.retry.attempts
                );
                sleep(self.retry.delay(attempt, retry_after.as_deref())).await;
                continue;
            }
            if status == StatusCode::CONFLICT && code == "incomplete" {
                return Err(Failure::Incomplete {
                    done_ms: done_ms.unwrap_or(0),
                });
            }
            return Err(Failure::Http {
                status: status.as_u16(),
                code,
                message: if message.is_empty() {
                    format!("HTTP {}", status.as_u16())
                } else {
                    message
                },
            });
        }
    }
}

/// `worker.Client.url`: an `http(s)` origin, no credentials, no query.
fn normalize_base(raw: &str) -> Result<String, Failure> {
    let trimmed = raw.trim().trim_end_matches('/');
    let url = reqwest::Url::parse(trimmed)
        .map_err(|_| Failure::Config("l'URL du serveur n'est pas une URL absolue".to_owned()))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(Failure::Config(
            "l'URL du serveur doit être http(s)".to_owned(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Failure::Config(
            "l'URL du serveur ne doit pas contenir d'identifiants".to_owned(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(Failure::Config(
            "l'URL du serveur ne doit pas contenir de requête".to_owned(),
        ));
    }
    if url.path() != "/" && !url.path().is_empty() {
        return Err(Failure::Config(
            "l'URL du serveur ne doit pas contenir de chemin".to_owned(),
        ));
    }
    Ok(trimmed.to_owned())
}

/// An error answer: `(code, message, done_ms)`, best effort.
///
/// `done_ms` is `POST …/complete`'s `409 incomplete` payload: the server says
/// where its transcript stops, and the supervisor continues from there rather
/// than from its own idea of the position.
async fn read_error(response: reqwest::Response) -> (String, String, Option<i64>) {
    let status = response.status().as_u16();
    match response.bytes().await {
        Ok(bytes) if !bytes.is_empty() => match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => (
                value
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                value.get("done_ms").and_then(Value::as_i64),
            ),
            Err(_) => (String::new(), format!("HTTP {status}"), None),
        },
        _ => (String::new(), format!("HTTP {status}"), None),
    }
}

/// `Content-Range: bytes <from>-<to>/<len|*>`, if the header is one.
fn content_range(headers: &reqwest::header::HeaderMap) -> (Option<u64>, Option<u64>, Option<u64>) {
    let raw = headers
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let Some(rest) = raw.trim().strip_prefix("bytes ") else {
        return (None, None, None);
    };
    let (range, total) = rest.split_once('/').unwrap_or((rest, ""));
    let (from, to) = range.split_once('-').unwrap_or((range, ""));
    (
        from.trim().parse::<u64>().ok(),
        to.trim().parse::<u64>().ok(),
        if total.trim() == "*" {
            None
        } else {
            total.trim().parse::<u64>().ok()
        },
    )
}

/// Write the body of `response` into `dest`, counting the bytes it actually
/// received.
async fn write_body(
    mut response: reqwest::Response,
    dest: &Path,
    append: bool,
    offset: u64,
    stopped: &(impl Fn() -> bool + Send + Sync),
) -> Result<u64, Failure> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(dest)
        .map_err(|error| Failure::Audio(format!("écriture impossible ({})", error.kind())))?;
    let mut written = offset;
    loop {
        if stopped() {
            return Err(Failure::Cancelled);
        }
        let block = response
            .chunk()
            .await
            .map_err(|error| Failure::Audio(format!("corps interrompu ({})", error)))?;
        let Some(block) = block else { break };
        file.write_all(&block)
            .map_err(|error| Failure::Audio(format!("écriture impossible ({})", error.kind())))?;
        written += block.len() as u64;
        if written > MAX_AUDIO_BYTES {
            return Err(Failure::Audio(
                "l'audio de l'épisode dépasse 1 Gio".to_owned(),
            ));
        }
    }
    file.flush()
        .map_err(|error| Failure::Audio(format!("écriture impossible ({})", error.kind())))?;
    Ok(written)
}

/// A one-line rendering of a transport error, without its URL.
fn short(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    if text.len() > 160 {
        text.truncate(160);
    }
    text
}

/// Serialize a request body.
fn to_value<T: Serialize>(body: &T) -> Result<Value, Failure> {
    serde_json::to_value(body)
        .map_err(|error| Failure::Config(format!("corps JSON impossible ({error})")))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Client, Failure, Retry, TOKEN_FILE, Token, content_range, normalize_base};
    use reqwest::header::{HeaderMap, HeaderValue};

    /// The one credential of the worker is never printable: a log line, a panic
    /// message or a `Debug` of the client must not carry it.
    #[test]
    fn the_token_is_never_printed() {
        let token = Token::new("secret-device-token").expect("a token");
        assert_eq!(format!("{token:?}"), "Token(<jeton>)");
        assert!(!format!("{token:?}").contains("secret"));
        assert_eq!(token.expose(), "secret-device-token");

        let client = Client::new("http://127.0.0.1:1", token.clone()).expect("a client");
        let debug = format!("{client:?}");
        assert!(!debug.contains("secret-device-token"), "{debug}");
        assert!(debug.contains("<jeton>"));
        assert_eq!(client.base(), "http://127.0.0.1:1");
        let failure = Failure::Auth {
            status: 401,
            code: "unauthorized".to_owned(),
            message: "jeton d'appareil invalide".to_owned(),
        };
        assert!(!format!("{failure} {failure:?}").contains("secret"));
        assert_eq!(token.expose().len(), 19);
    }

    /// The server's token limits (`worker.Config.from_env`): not empty, at most
    /// 200 characters, no whitespace, and the file's own newline is trimmed.
    #[test]
    fn a_token_is_checked_before_it_is_used() {
        assert!(Token::new("").is_err());
        assert!(Token::new("   ").is_err());
        assert!(Token::new("two words").is_err());
        assert!(Token::new("x".repeat(201)).is_err());
        assert!(Token::new("x".repeat(200)).is_ok());
        assert_eq!(Token::new(" abc \n").expect("trimmed").expose(), "abc");

        let directory = scratch();
        let path = directory.join(TOKEN_FILE);
        std::fs::write(&path, "fichier-jeton\n").expect("write");
        assert_eq!(
            Token::from_file(&path).expect("read").expose(),
            "fichier-jeton",
            "a file ends with a newline"
        );
        std::fs::write(&path, "").expect("write");
        assert!(Token::from_file(&path).is_err());
        assert!(Token::from_file(&directory.join("absent")).is_err());
        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// A base URL is an origin: a path, a query or credentials in it would make
    /// every route below it a guess.
    #[test]
    fn a_base_url_is_an_origin() {
        assert_eq!(
            normalize_base("http://127.0.0.1:5722/").expect("ok"),
            "http://127.0.0.1:5722"
        );
        assert_eq!(
            normalize_base(" https://neezer.example ").expect("ok"),
            "https://neezer.example"
        );
        for bad in [
            "neezer.example",
            "ftp://neezer.example",
            "http://user:pass@neezer.example",
            "http://neezer.example/api",
            "http://neezer.example/?a=1",
            "http://neezer.example/#x",
        ] {
            assert!(normalize_base(bad).is_err(), "{bad}");
        }
    }

    /// The retry schedule of `worker.Client._backoff`, with the client's own
    /// bounds so a test can shrink it without changing its shape.
    #[test]
    fn the_clients_backoff_has_the_servers_shape() {
        let retry = Retry::default();
        let seconds: Vec<f64> = (1..=6)
            .map(|attempt| retry.delay(attempt, None).as_secs_f64())
            .collect();
        assert_eq!(seconds, [1.0, 2.0, 4.0, 8.0, 16.0, 30.0]);
        assert_eq!(retry.delay(1, Some("7")).as_secs_f64(), 7.0);
        assert_eq!(retry.delay(1, Some("0")).as_secs_f64(), 1.0);
        assert_eq!(retry.delay(1, Some("9999")).as_secs_f64(), 120.0);
        let fast = Retry {
            attempts: 3,
            base: 0.01,
            max: 0.05,
            retry_after_cap: 0.05,
        };
        assert_eq!(fast.delay(1, None).as_secs_f64(), 0.01);
        assert_eq!(fast.delay(2, None).as_secs_f64(), 0.02);
        assert_eq!(fast.delay(9, None).as_secs_f64(), 0.05);
        assert_eq!(fast.delay(1, Some("30")).as_secs_f64(), 0.05);
    }

    /// `Content-Range` is parsed the way `worker._CONTENT_RANGE` parses it, and
    /// a header that is not one is not guessed at.
    #[test]
    fn a_content_range_is_read_or_refused() {
        let mut headers = HeaderMap::new();
        assert_eq!(content_range(&headers), (None, None, None));
        headers.insert(
            reqwest::header::CONTENT_RANGE,
            HeaderValue::from_static("bytes 100-199/200"),
        );
        assert_eq!(content_range(&headers), (Some(100), Some(199), Some(200)));
        headers.insert(
            reqwest::header::CONTENT_RANGE,
            HeaderValue::from_static("bytes 0-9/*"),
        );
        assert_eq!(content_range(&headers), (Some(0), Some(9), None));
        headers.insert(
            reqwest::header::CONTENT_RANGE,
            HeaderValue::from_static("nonsense"),
        );
        assert_eq!(content_range(&headers), (None, None, None));
    }

    /// A job that was asked to stop never starts a request.
    #[tokio::test]
    async fn a_stopped_download_asks_for_nothing() {
        let token = Token::new("t").expect("token");
        let retry = Retry {
            attempts: 1,
            ..Retry::default()
        };
        let client = Client::with_retry("http://127.0.0.1:1", token, retry).expect("client");
        let directory = scratch();
        let outcome = client
            .download(
                "b7b1c0de-0000-4000-8000-000000000000",
                "lease",
                &directory.join("a"),
                || true,
            )
            .await;
        assert_eq!(outcome, Err(Failure::Cancelled));
        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// An id that is not one never becomes a URL, even in a download.
    #[tokio::test]
    async fn a_bad_episode_id_is_refused_before_the_request() {
        let client =
            Client::new("http://127.0.0.1:1", Token::new("t").expect("token")).expect("client");
        let outcome = client
            .download(
                "../../settings",
                "lease",
                &PathBuf::from("/tmp/nz-never"),
                || false,
            )
            .await;
        assert!(matches!(outcome, Err(Failure::BadClaim(_))), "{outcome:?}");
    }

    /// A scratch directory of this test's own (tests run in parallel).
    fn scratch() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nz-transcribe-worker-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("scratch directory");
        path
    }
}
