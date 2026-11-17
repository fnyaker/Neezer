//! `nz-api` — the app's ONLY HTTP client.
//!
//! It owns five things, and nothing else in the app may re-implement any of
//! them (docs/agents/desktop.md, lot D2):
//!
//! - **the lanes of rule 1** ([`lanes`]): a request says what it is — the action
//!   just taken, the next track, or background work that must yield — and the
//!   gates make that true;
//! - **the failure shapes of `lib/api.js#req`** ([`ApiError`]): offline, session
//!   ended, provider refusal (`X-NS-Error`, final), not-ready, conflict, busy,
//!   HTTP, undecodable;
//! - **the retry rules**: a GET retries (3 tries, 400·2ⁿ ms + jitter, capped at
//!   6 s), a mutation NEVER does, and a background request repeats only on the
//!   server's own `Retry-After`;
//! - **the offline copy** ([`ResponseCache`], implemented by `nz-cache`): the
//!   paths `lib/apicache.js` keeps, keyed by account;
//! - **the session cookie** ([`SessionJar`]): one file, mode 0600, one entry per
//!   server URL.
//!
//! Plus one typed function per SPA `api.*` method, in [`endpoints`], with the
//! same names in snake_case.
//!
//! ```no_run
//! # async fn example() -> Result<(), nz_api::ApiError> {
//! use nz_api::{Api, Lane, Quality, TrackId};
//!
//! let api = Api::new("http://127.0.0.1:5000")?;
//! api.login("bench", "Bench1").await?;
//! let me = api.me().await?;
//! println!("{}", me.user.name);
//! // The tapped track: User lane, never behind anything else.
//! let mut stream = api
//!     .stream(Lane::User, &TrackId::from("3000168"), Quality::Opus320, None, None)
//!     .await?;
//! while let Some(bytes) = stream.chunk().await? {
//!     let _ = bytes;
//! }
//! # Ok(())
//! # }
//! ```

pub mod cache_trait;
pub mod client;
pub mod endpoints;
pub mod error;
pub mod lanes;
pub mod session;

pub use cache_trait::{NoCache, ResponseCache, is_cacheable};
pub use client::{
    ANSWER_TIMEOUT, Api, ApiBuilder, BACKGROUND_HEADER, CONNECT_TIMEOUT, DownloadReport, Fresh,
    NO_RETRY_HEADER, Quality, STALL_TIMEOUT, START_HEADER, StreamReady, StreamResponse, TrackId,
};
pub use endpoints::entities::RadioSeed;
pub use endpoints::library::DownloadQueued;
pub use endpoints::playlists::ImportJob;
pub use endpoints::podcasts::PodcastAutoArchive;
pub use endpoints::remote::RemoteCreated;
pub use error::ApiError;
pub use lanes::{Lane, bulk_cap_from_stream};
pub use reqwest::Method;
pub use session::SessionJar;
