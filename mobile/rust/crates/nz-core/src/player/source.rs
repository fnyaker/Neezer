//! Where the bytes come from: the person's download, then the play cache, then
//! the network on the right lane — and the throughput the quality ladder learns
//! from them.
//!
//! Owned by **MB3b**. `nz-audio` is deliberately synchronous
//! ([`StreamOpener::open`] returns a blocking [`ByteStream`]) and `nz-api` is
//! async, so this module is the one place where the two meet: a request is
//! issued on the player's own runtime and awaited from the engine's loader or
//! decode thread. Nothing in the audio callback ever reaches it.
//!
//! **The order is the note's** (`docs/agents/mobile.md`, *MB3b*): *download →
//! complete play-cache file → network*. The download comes FIRST and not
//! second, which is the one deliberate difference from the desktop's
//! `NetOpener`: a file the person chose to download was fetched at the quality
//! they chose for offline listening, and a phone in a tunnel must play it
//! whatever the stream bar says. The play cache is what a previous play left
//! behind — complete files only, published by rename (`nz-cache`).
//!
//! **Rule 1 is not re-implemented here.** The current track asks on the `User`
//! lane, the next one on `Prefetch` (which `nz-api` holds back until no `User`
//! request has been in flight for 250 ms, and marks `X-NS-Background: 1`), a
//! download on `Bulk`. The measured throughput of a stream is also handed to
//! `nz-api` (`set_bulk_cap_from_stream`), so a download already running yields
//! to the track that is playing: one mechanism, no lease.
//!
//! **A failure is stored, not printed.** The engine flattens every failure into
//! a French sentence ([`EngineEvent::Error`]), which is right for the person
//! and useless for the fault policy (a 503 is retried, a 404 skips the title —
//! the sentence does not say which). So the typed cause is kept in
//! [`Failures`], keyed by track, and [`crate::player::fault`] reads it.

use std::collections::HashMap;
use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use nz_api::{Api, ApiError, StreamResponse};
use nz_audio::{ByteStream, SourceError, StreamOpener, StreamReady};
use tokio::sync::watch;

use super::quality::{Ladder, MIN_SAMPLE_BYTES};

/// The player's window of the server's `/stream` routes.
pub struct ServerOpener {
    /// The core, asked for its client on every call: the person may configure a
    /// server, sign in or lose the link while this player exists, and
    /// `Core::api()` is where rule 3's offline gate lives (0 sockets).
    core: Arc<crate::core::Core>,
    runtime: tokio::runtime::Handle,
    /// `true` means "the player is closing": every wait races it, so the close
    /// joins the engine's threads instead of waiting for the next chunk.
    gone: watch::Sender<bool>,
    /// What the last three streams measured (the Auto ladder).
    ladder: Arc<Mutex<Ladder>>,
    /// The typed cause of each track's last failure, for the fault policy.
    failures: Arc<Failures>,
    /// Whether a network body is being read right now (the snapshot's
    /// `streaming`, rule 1's "the Wi-Fi lock is held, the Bulk lane yields").
    streaming: Arc<AtomicBool>,
    /// The rate the last streams measured, in bits of an `f64` (0 = none): the
    /// ladder's answer without a lock, which is what the snapshot reads.
    rate: Arc<AtomicU64>,
}

impl ServerOpener {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        core: Arc<crate::core::Core>,
        runtime: tokio::runtime::Handle,
        ladder: Arc<Mutex<Ladder>>,
        rate: Arc<AtomicU64>,
        failures: Arc<Failures>,
        streaming: Arc<AtomicBool>,
        gone: watch::Sender<bool>,
    ) -> ServerOpener {
        ServerOpener {
            core,
            runtime,
            gone,
            ladder,
            failures,
            streaming,
            rate,
        }
    }

    /// The stop every wait races: the player is closing.
    pub fn gone(&self) -> watch::Receiver<bool> {
        self.gone.subscribe()
    }

    /// The typed causes the fault policy reads.
    pub fn failures(&self) -> Arc<Failures> {
        Arc::clone(&self.failures)
    }

    /// The client, or the cause it cannot be had. `Core::api()` answers
    /// `Offline` in zero sockets, which is what makes an airplane-mode play
    /// fail fast instead of after a timeout.
    fn api(&self) -> Result<Arc<Api>, SourceError> {
        self.core.api().map_err(|error| match error {
            crate::core::CoreError::Offline => SourceError::Offline,
            other => SourceError::Message(super::core_cause(&other)),
        })
    }

    /// Abandon every request in flight (the player is closing).
    pub fn shutdown(&self) {
        let _ = self.gone.send(true);
    }
}

impl StreamOpener for ServerOpener {
    fn open(
        &self,
        id: &str,
        quality: nz_audio::Quality,
        start: Option<f64>,
        from_byte: Option<u64>,
        lane: nz_audio::Lane,
    ) -> Result<Box<dyn ByteStream>, SourceError> {
        let api = self.api()?;
        let track = nz_api::TrackId::from(id);
        let mut gone = self.gone.subscribe();
        let answer = block_on(
            &self.runtime,
            &mut gone,
            api.stream(
                api_lane(lane),
                &track,
                api_quality(quality),
                start,
                from_byte,
            ),
        )
        .ok_or_else(closed)?;
        match answer {
            Ok(response) => {
                // A network body is being read from here on: the snapshot says
                // so, and the Bulk lane learns this stream's own speed.
                self.streaming.store(true, Ordering::SeqCst);
                self.failures.clear(id);
                let length = response.content_length;
                Ok(Box::new(MeasuredStream {
                    response,
                    runtime: self.runtime.clone(),
                    gone,
                    pending: Vec::new(),
                    done: false,
                    ladder: Arc::clone(&self.ladder),
                    rate: Arc::clone(&self.rate),
                    streaming: Arc::clone(&self.streaming),
                    api: Some(Arc::clone(&api)),
                    bytes: 0,
                    started: Instant::now(),
                    length,
                    done_once: false,
                }))
            }
            Err(error) => {
                let cause = source_error(error);
                self.failures.record(id, cause.clone());
                Err(cause)
            }
        }
    }

    fn ready(&self, id: &str, quality: nz_audio::Quality) -> Result<StreamReady, SourceError> {
        let api = self.api()?;
        let track = nz_api::TrackId::from(id);
        let mut gone = self.gone.subscribe();
        let answer = block_on(
            &self.runtime,
            &mut gone,
            api.ready(&track, api_quality(quality)),
        )
        .ok_or_else(closed)?;
        match answer {
            Ok(ready) => Ok(StreamReady {
                ready: ready.ready,
                ranges: ready.ranges,
                bytes: ready.bytes,
                cached: ready.cached,
                offset: ready.offset,
                streaming: ready.streaming,
            }),
            Err(error) => {
                let cause = source_error(error);
                self.failures.record(id, cause.clone());
                Err(cause)
            }
        }
    }

    /// A complete file for this track: the person's DOWNLOAD first (it is
    /// permanent, at the quality they picked, and it must play with the link
    /// cut), then the play cache.
    fn local(&self, id: &str, quality: nz_audio::Quality) -> Option<PathBuf> {
        let cache = self.core.cache();
        let track = nz_model::TrackId::new(id);
        if let Some(owner) = self.core.me().map(|me| me.user.id.to_string())
            && let Ok(Some((path, _quality))) = cache.downloads().local(&owner, &track)
        {
            return Some(path);
        }
        cache
            .play_cache()
            .lookup(&track, api_quality(quality))
            .ok()
            .flatten()
    }

    fn shutdown(&self) {
        ServerOpener::shutdown(self);
    }
}

/// A body read synchronously by the decoder, and the only place a stream's
/// throughput is measured.
struct MeasuredStream {
    response: StreamResponse,
    runtime: tokio::runtime::Handle,
    gone: watch::Receiver<bool>,
    pending: Vec<u8>,
    done: bool,
    ladder: Arc<Mutex<Ladder>>,
    /// The ladder's answer in an atomic, so the snapshot reads it with no lock.
    rate: Arc<AtomicU64>,
    streaming: Arc<AtomicBool>,
    /// Kept so the Bulk lane's cap can be fed the stream's own speed.
    api: Option<Arc<Api>>,
    bytes: u64,
    started: Instant,
    /// What the server said the body's length is, when it said (a generated
    /// stream has none).
    length: Option<u64>,
    /// The measurement is taken once, on the read that ends the body.
    done_once: bool,
}

impl MeasuredStream {
    /// One stream finished or was cut: teach the ladder and hand the rate to
    /// the Bulk cap. Called once (a skip and an end must not count twice).
    fn measure(&mut self) {
        if self.done_once {
            return;
        }
        self.done_once = true;
        self.streaming.store(false, Ordering::SeqCst);
        if self.bytes < MIN_SAMPLE_BYTES {
            return;
        }
        let seconds = self.started.elapsed().as_secs_f64();
        let (rung, measured) = {
            let mut ladder = self.ladder.lock().unwrap_or_else(|err| err.into_inner());
            let rung = ladder.observe(self.bytes, seconds);
            (rung, ladder.bytes_per_s())
        };
        if let Some(measured) = measured {
            self.rate.store(measured.to_bits(), Ordering::SeqCst);
        }
        tracing::debug!(
            bytes = self.bytes,
            seconds,
            rate = self.bytes as f64 / seconds.max(f64::MIN_POSITIVE),
            ?rung,
            "stream measured"
        );
        if let Some(api) = &self.api {
            api.set_bulk_cap_from_stream(Some((self.bytes as f64 / seconds.max(0.001)) as u64));
        }
    }
}

impl ByteStream for MeasuredStream {
    fn len_hint(&self) -> Option<u64> {
        self.length
    }

    fn start_offset_s(&self) -> f64 {
        self.response.start_s.unwrap_or(0.0)
    }
}

impl Read for MeasuredStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pending.is_empty() {
            if self.done {
                return Ok(0);
            }
            let Self {
                response,
                runtime,
                gone,
                pending,
                done,
                bytes,
                ..
            } = self;
            match block_on(runtime, gone, response.chunk()) {
                Some(Ok(Some(chunk))) => {
                    *bytes += chunk.len() as u64;
                    *pending = chunk;
                }
                Some(Ok(None)) => {
                    *done = true;
                    self.measure();
                    return Ok(0);
                }
                Some(Err(error)) => {
                    self.measure();
                    return Err(io::Error::other(error.cause_fr()));
                }
                // The player is closing. An ERROR, never an end of body: the
                // engine must not rename a half body into the play cache as a
                // complete track.
                None => {
                    self.measure();
                    return Err(io::Error::other("lecteur fermé"));
                }
            }
        }
        let take = out.len().min(self.pending.len());
        out[..take].copy_from_slice(&self.pending[..take]);
        self.pending.drain(..take);
        Ok(take)
    }
}

impl Drop for MeasuredStream {
    fn drop(&mut self) {
        // A body cut by a skip measures too, when it delivered enough to say
        // anything about the link: a person who skips every 15 s is exactly who
        // the ladder exists for.
        self.measure();
    }
}

/// The typed cause of each track's last failure, so a retry and a skip can be
/// told apart without parsing a French sentence.
#[derive(Debug, Default)]
pub struct Failures {
    causes: Mutex<HashMap<String, SourceError>>,
}

impl Failures {
    pub fn new() -> Failures {
        Failures::default()
    }

    pub fn record(&self, id: &str, cause: SourceError) {
        self.causes
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(id.to_owned(), cause);
    }

    pub fn clear(&self, id: &str) {
        self.causes
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(id);
    }

    /// The cause, or `None` when nothing here knows it (a decode error the
    /// engine found on its own never reached this table).
    pub fn take(&self, id: &str) -> Option<SourceError> {
        self.causes
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(id)
    }

    pub fn peek(&self, id: &str) -> Option<SourceError> {
        self.causes
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(id)
            .cloned()
    }

    pub fn len(&self) -> usize {
        self.causes
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Rule 1's lanes exist twice — `nz-audio` names one for its opener, `nz-api`
/// gates on its own — so the conversion is spelled once, here.
fn api_lane(lane: nz_audio::Lane) -> nz_api::Lane {
    match lane {
        nz_audio::Lane::User => nz_api::Lane::User,
        nz_audio::Lane::Prefetch => nz_api::Lane::Prefetch,
        nz_audio::Lane::Bulk => nz_api::Lane::Bulk,
    }
}

fn api_quality(quality: nz_audio::Quality) -> nz_api::Quality {
    match quality {
        nz_audio::Quality::Flac => nz_api::Quality::Flac,
        nz_audio::Quality::Opus320 => nz_api::Quality::Opus320,
        nz_audio::Quality::Opus128 => nz_api::Quality::Opus128,
        nz_audio::Quality::Opus64 => nz_api::Quality::Opus64,
    }
}

/// `nz-api`'s refusal, as the engine understands it. The distinctions the fault
/// policy needs survive: `Offline`/`NotReady` are worth waiting for, a
/// `Provider` refusal carries its status, anything else is final.
pub fn source_error(error: ApiError) -> SourceError {
    match error {
        ApiError::Offline => SourceError::Offline,
        ApiError::Busy { retry_after } | ApiError::NotReady { retry_after } => {
            SourceError::NotReady { retry_after }
        }
        ApiError::Provider {
            status, message, ..
        } => SourceError::Provider { status, message },
        other => SourceError::Message(other.cause_fr()),
    }
}

/// Why an `open`/`ready` gave up: the player is closing.
fn closed() -> SourceError {
    SourceError::Message("lecteur fermé".into())
}

/// Run one future on the player's runtime, and give it up the moment the player
/// closes.
///
/// `None` means "closing": the future was dropped before it finished. Every
/// caller of this runs on a thread the engine joins before the runtime stops
/// (`Engine::shutdown`), which is the lifetime tokio's "timers panic on a
/// shutting-down runtime" would otherwise break.
pub(crate) fn block_on<T>(
    runtime: &tokio::runtime::Handle,
    gone: &mut watch::Receiver<bool>,
    future: impl std::future::Future<Output = T>,
) -> Option<T> {
    if *gone.borrow() {
        return None;
    }
    runtime.block_on(async {
        tokio::select! {
            biased;
            _ = gone.wait_for(|stop| *stop) => None,
            value = future => Some(value),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The distinctions the fault policy lives on survive the conversion: an
    /// offline answer and a 503 are "wait", a 404 is final, a decode failure is
    /// final.
    #[test]
    fn a_refusal_keeps_the_distinctions_the_policy_needs() {
        assert!(matches!(
            source_error(ApiError::Offline),
            SourceError::Offline
        ));
        assert!(matches!(
            source_error(ApiError::NotReady { retry_after: 30 }),
            SourceError::NotReady { retry_after: 30 }
        ));
        match source_error(ApiError::Provider {
            status: 503,
            code: "queued".into(),
            message: "pas encore archivé".into(),
            retry_in: 0,
        }) {
            SourceError::Provider { status, .. } => assert_eq!(status, 503),
            other => panic!("expected a provider refusal, got {other:?}"),
        }
        assert!(matches!(
            source_error(ApiError::Decode("corps".into())),
            SourceError::Message(_)
        ));
    }

    /// The failure table is per track and forgets a track once it plays.
    #[test]
    fn the_failure_table_is_per_track_and_cleared_by_a_success() {
        let failures = Failures::new();
        failures.record("a", SourceError::Offline);
        failures.record(
            "b",
            SourceError::Provider {
                status: 404,
                message: "introuvable".into(),
            },
        );
        assert_eq!(failures.len(), 2);
        assert!(matches!(failures.peek("a"), Some(SourceError::Offline)));
        failures.clear("a");
        assert!(failures.take("a").is_none());
        assert_eq!(failures.len(), 1);
        assert!(matches!(
            failures.take("b"),
            Some(SourceError::Provider { status: 404, .. })
        ));
        assert!(failures.is_empty());
    }
}
