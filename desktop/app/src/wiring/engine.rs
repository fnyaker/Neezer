//! The audio engine, and the only bridge between it and the network.
//!
//! `nz-audio` is deliberately synchronous (`StreamOpener::open` returns a
//! blocking `Read`), while `nz-api` is async: this module is where the two meet
//! and nowhere else. A request is issued on `nz-net` and awaited from the
//! engine's own loader/decode thread with `Handle::block_on`; the body is
//! pulled chunk by chunk, so a stream that stalls cannot hold a slot on the
//! User lane (D2 releases it once the headers arrive) and the decoder's
//! read-ahead cap is what bounds memory (rule 1).
//!
//! The engine starts on a named thread so the first painted frame never waits
//! for PipeWire; its events are forwarded through [`crate::wiring::events`] and
//! the spool of a completed stream is published into the play cache here, which
//! is the only place that knows both file names.

use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nz_api::{Api, ApiError, StreamResponse};
use nz_audio::{ByteStream, Engine, EngineConfig, SourceError, StreamOpener, StreamReady};
use nz_cache::Cache;
use tracing::{debug, warn};

use crate::state::AppEvent;
use crate::wiring::events::Sender;

/// The engine plus the rate it settled on.
#[derive(Clone)]
pub struct Audio {
    pub engine: Engine,
    pub sample_rate: f32,
}

impl Audio {
    /// Where the bar reads the playhead from: the mixer's own clock, not a
    /// timer in the UI.
    pub fn position(&self) -> f64 {
        self.engine.position()
    }
}

/// The quality the person chose. `Auto` is the plan's ladder starting point;
/// the measured step-down (>= 600 kB/s → 320, …) lands with the player screen
/// that shows it, and until then Auto is exactly Opus 320.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackQuality {
    #[default]
    Auto,
    Flac,
    Opus320,
    Opus128,
    Opus64,
}

impl PlaybackQuality {
    pub fn key(self) -> &'static str {
        match self {
            PlaybackQuality::Auto => "auto",
            PlaybackQuality::Flac => "flac",
            PlaybackQuality::Opus320 => "opus_320",
            PlaybackQuality::Opus128 => "opus_128",
            PlaybackQuality::Opus64 => "opus_64",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "flac" => PlaybackQuality::Flac,
            "opus_320" | "320" => PlaybackQuality::Opus320,
            "opus_128" | "128" => PlaybackQuality::Opus128,
            "opus_64" | "64" => PlaybackQuality::Opus64,
            _ => PlaybackQuality::Auto,
        }
    }

    /// What the engine asks the server for.
    pub fn audio(self) -> nz_audio::Quality {
        match self {
            PlaybackQuality::Auto | PlaybackQuality::Opus320 => nz_audio::Quality::Opus320,
            PlaybackQuality::Flac => nz_audio::Quality::Flac,
            PlaybackQuality::Opus128 => nz_audio::Quality::Opus128,
            PlaybackQuality::Opus64 => nz_audio::Quality::Opus64,
        }
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

fn source_error(error: ApiError) -> SourceError {
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

fn engine_lane(lane: nz_audio::Lane) -> nz_api::Lane {
    match lane {
        nz_audio::Lane::User => nz_api::Lane::User,
        nz_audio::Lane::Prefetch => nz_api::Lane::Prefetch,
        nz_audio::Lane::Bulk => nz_api::Lane::Bulk,
    }
}

/// `nz-audio`'s window of the server's `/stream` routes, over `nz-api`.
pub struct NetOpener {
    api: Arc<Api>,
    runtime: tokio::runtime::Handle,
    cache: Cache,
}

impl NetOpener {
    pub fn new(api: Arc<Api>, runtime: tokio::runtime::Handle, cache: Cache) -> Self {
        Self {
            api,
            runtime,
            cache,
        }
    }
}

impl StreamOpener for NetOpener {
    fn open(
        &self,
        id: &str,
        quality: nz_audio::Quality,
        start: Option<f64>,
        from_byte: Option<u64>,
        lane: nz_audio::Lane,
    ) -> Result<Box<dyn ByteStream>, SourceError> {
        let id = nz_api::TrackId::from(id);
        let response = self
            .runtime
            .block_on(self.api.stream(
                engine_lane(lane),
                &id,
                api_quality(quality),
                start,
                from_byte,
            ))
            .map_err(source_error)?;
        Ok(Box::new(ApiByteStream {
            response,
            runtime: self.runtime.clone(),
            pending: Vec::new(),
            done: false,
        }))
    }

    fn ready(&self, id: &str, quality: nz_audio::Quality) -> Result<StreamReady, SourceError> {
        let id = nz_api::TrackId::from(id);
        let ready = self
            .runtime
            .block_on(self.api.ready(&id, api_quality(quality)))
            .map_err(source_error)?;
        Ok(StreamReady {
            ready: ready.ready,
            ranges: ready.ranges,
            bytes: ready.bytes,
            cached: ready.cached,
            offset: ready.offset,
            streaming: ready.streaming,
        })
    }

    fn local(&self, id: &str, quality: nz_audio::Quality) -> Option<PathBuf> {
        let track = nz_model::TrackId::new(id);
        self.cache
            .play_cache()
            .lookup(&track, api_quality(quality))
            .ok()
            .flatten()
    }
}

/// A streaming body read synchronously by the decoder.
struct ApiByteStream {
    response: StreamResponse,
    runtime: tokio::runtime::Handle,
    pending: Vec<u8>,
    done: bool,
}

impl ByteStream for ApiByteStream {
    fn len_hint(&self) -> Option<u64> {
        self.response.content_length
    }

    fn start_offset_s(&self) -> f64 {
        self.response.start_s.unwrap_or(0.0)
    }
}

impl Read for ApiByteStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pending.is_empty() {
            if self.done {
                return Ok(0);
            }
            match self.runtime.block_on(self.response.chunk()) {
                Ok(Some(chunk)) => self.pending = chunk,
                Ok(None) => {
                    self.done = true;
                    return Ok(0);
                }
                Err(error) => {
                    return Err(io::Error::other(error.cause_fr()));
                }
            }
        }
        let take = out.len().min(self.pending.len());
        out[..take].copy_from_slice(&self.pending[..take]);
        self.pending.drain(..take);
        Ok(take)
    }
}

/// Start the engine on its own thread and announce it. Returns immediately: the
/// window paints first, the sound arrives when PipeWire is ready.
pub fn spawn_audio(
    api: Arc<Api>,
    cache: Cache,
    runtime: tokio::runtime::Handle,
    events: Sender<AppEvent>,
) {
    let failure = events.clone();
    let spawned = std::thread::Builder::new()
        .name("nz-audio-start".to_owned())
        .spawn(move || {
            let opener = Arc::new(NetOpener::new(api, runtime, cache.clone()));
            // A 48 kHz preference: the device's own rate wins when it cannot do
            // this one, and the decode threads resample.
            let config = EngineConfig {
                spool_dir: cache.audio_dir(),
                sample_rate: 48_000,
                ..EngineConfig::default()
            };
            let engine = Engine::new(config, opener);
            let sample_rate = engine.output_info().sample_rate as f32;
            let receiver = engine.events();
            let tx = events.clone();
            let _ = std::thread::Builder::new()
                .name("nz-audio-events".to_owned())
                .spawn(move || {
                    while let Ok(event) = receiver.recv() {
                        tx.send(AppEvent::Engine(event));
                    }
                });
            events.send(AppEvent::AudioReady(Ok(Box::new(Audio {
                engine,
                sample_rate,
            }))));
        });
    if let Err(error) = spawned {
        failure.send(AppEvent::AudioReady(Err(format!(
            "le moteur audio n'a pas pu démarrer : {error}"
        ))));
    }
}

/// Publish the engine's completed spool into the play cache: the spool is the
/// track's bytes, and the cache is what lets the next play come off disk.
///
/// A `?start=` slice is never announced by the engine (`StreamComplete` is only
/// emitted for a whole body), so a window can never become a "complete" entry.
pub fn publish_spool(
    cache: &Cache,
    id: &str,
    quality: nz_audio::Quality,
    path: &PathBuf,
    bytes: u64,
) {
    let track = nz_model::TrackId::new(id);
    let quality = api_quality(quality);
    let play_cache = cache.play_cache();
    let reserved = match play_cache.reserve(&track, quality) {
        Ok(path) => path,
        Err(error) => {
            warn!(track = id, %error, "réservation du cache de lecture impossible");
            return;
        }
    };
    if reserved != *path {
        // Same directory in the default install (the engine spools into
        // `$XDG_CACHE_HOME/neezer/audio`), so this is a rename; a custom spool
        // directory falls back to a copy rather than losing the entry.
        if fs::rename(path, &reserved).is_err()
            && let Err(error) = fs::copy(path, &reserved)
        {
            debug!(track = id, %error, "copie du spool dans le cache de lecture impossible");
            let _ = fs::remove_file(&reserved);
            return;
        }
    }
    if let Err(error) = play_cache.complete(&track, quality, bytes) {
        warn!(track = id, %error, "publication du cache de lecture impossible");
    }
    // The engine reuses its spool until the cache published it; once renamed it
    // is already gone, and a copied one is removed here.
    let _ = fs::remove_file(path);
}

/// The player's periodic refresh: position and MPRIS, at 4 Hz. Kept here so the
/// shell's task and `nz-audio`'s clock cannot disagree about what "now" is.
pub const TICK: Duration = Duration::from_millis(250);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quality_ladder_names_the_wire_and_the_engine_the_same() {
        for (setting, audio, api) in [
            (
                PlaybackQuality::Auto,
                nz_audio::Quality::Opus320,
                nz_api::Quality::Opus320,
            ),
            (
                PlaybackQuality::Opus320,
                nz_audio::Quality::Opus320,
                nz_api::Quality::Opus320,
            ),
            (
                PlaybackQuality::Opus128,
                nz_audio::Quality::Opus128,
                nz_api::Quality::Opus128,
            ),
            (
                PlaybackQuality::Opus64,
                nz_audio::Quality::Opus64,
                nz_api::Quality::Opus64,
            ),
            (
                PlaybackQuality::Flac,
                nz_audio::Quality::Flac,
                nz_api::Quality::Flac,
            ),
        ] {
            assert_eq!(setting.audio(), audio);
            assert_eq!(api_quality(audio), api);
        }
        for setting in [
            PlaybackQuality::Auto,
            PlaybackQuality::Flac,
            PlaybackQuality::Opus320,
            PlaybackQuality::Opus128,
            PlaybackQuality::Opus64,
        ] {
            assert_eq!(PlaybackQuality::parse(setting.key()), setting);
        }
    }

    #[test]
    fn an_api_failure_becomes_the_cause_the_bar_can_print() {
        assert_eq!(source_error(ApiError::Offline), SourceError::Offline);
        assert_eq!(
            source_error(ApiError::Busy { retry_after: 30 }),
            SourceError::NotReady { retry_after: 30 }
        );
        match source_error(ApiError::Provider {
            status: 502,
            code: "deezer-unreachable".into(),
            message: "Deezer ne répond pas".into(),
            retry_in: 12,
        }) {
            SourceError::Provider { status, message } => {
                assert_eq!(status, 502);
                assert_eq!(message, "Deezer ne répond pas");
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
