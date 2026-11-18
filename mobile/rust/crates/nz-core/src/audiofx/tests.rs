//! MB12's own proofs: the settings map onto the player's chain exactly, a fader
//! reaches the SOUND within one buffer (rendered, not timed), and the meter's
//! session takes readings only while a card is subscribed.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::player::{AudioSettings, Normalization, Quality, TRIM_DB};

fn audio() -> AudioSettings {
    AudioSettings {
        quality: Quality::Opus320,
        crossfade_s: 0.0,
        trim_silence: false,
        normalization: Normalization::Off,
        eq_db: vec![0.0; BANDS],
        eq_preset: None,
        bass_db: 0.0,
        mono: false,
        balance: 0.0,
        speed: 1.25,
        metered_cap: Quality::Opus128,
    }
}

/// The defaults are `stores.js`'s: nothing engaged, the SPA's six seconds of
/// crossfade (remembered while the switch is off), its skip fade, and its trim
/// ON.
#[test]
fn the_defaults_are_the_spas() {
    let settings = FxSettings::default().clamped();
    assert!(!settings.eq_enabled);
    assert_eq!(settings.bands, vec![0.0; 10]);
    assert_eq!(settings.bass, 0.0);
    assert_eq!(settings.normalization, "off");
    assert!(!settings.mono);
    assert_eq!(settings.balance, 0.0);
    assert!(!settings.crossfade_enabled);
    assert_eq!(settings.crossfade_s, 6.0);
    assert!(settings.crossfade_on_skip);
    assert_eq!(settings.crossfade_skip_s, 1.5);
    assert!(settings.trim, "trimSilence defaults to true in the SPA");
    assert_eq!(settings.output, "auto");
}

/// **The chain the engine is given**: the curve while the equalizer is on, a flat
/// chain while it is off, `bass × 12` dB as the graph asks, the crossfade only
/// when it is on — and the fields this card does not own (the quality, the
/// podcast speed, the metered cap) carried over untouched.
#[test]
fn the_settings_reach_the_chain_unchanged() {
    let mut settings = FxSettings {
        bands: vec![9.0; 10],
        ..FxSettings::default()
    };
    settings.eq_enabled = false;
    let mapped = settings.to_audio(&audio());
    assert_eq!(
        mapped.eq_db,
        vec![0.0; 10],
        "a bypassed equalizer must not reach the chain"
    );

    settings.eq_enabled = true;
    settings.bass = 0.5;
    settings.normalization = "high".to_owned();
    settings.mono = true;
    settings.balance = -0.4;
    settings.crossfade_enabled = true;
    settings.crossfade_s = 4.0;
    settings.trim = true;
    let mapped = settings.to_audio(&audio());
    assert_eq!(mapped.eq_db, vec![9.0; 10]);
    assert_eq!(mapped.bass_db, 6.0, "bass 0.5 is the graph's +6 dB");
    assert_eq!(mapped.normalization, Normalization::High);
    assert!(mapped.mono);
    assert_eq!(mapped.balance, -0.4);
    assert_eq!(mapped.crossfade_s, 4.0);
    assert!(mapped.trim_silence);
    // Not this card's: the quality, the episode speed and the metered cap.
    assert_eq!(mapped.quality, Quality::Opus320);
    assert_eq!(mapped.speed, 1.25);
    assert_eq!(mapped.metered_cap, Quality::Opus128);

    // The crossfade switched off is 0 s, while its length is remembered.
    settings.crossfade_enabled = false;
    assert_eq!(settings.to_audio(&audio()).crossfade_s, 0.0);
    assert_eq!(settings.crossfade_s, 4.0);
}

/// A hostile stored value — a curve of the wrong length, a NaN band, a balance
/// out of range, an unknown level or mode — is clamped, never propagated.
#[test]
fn a_hostile_setting_is_clamped() {
    let hostile = FxSettings {
        bands: vec![f32::NAN, 99.0, -99.0, 1.0],
        bass: f32::INFINITY,
        balance: 9.0,
        crossfade_s: -3.0,
        crossfade_skip_s: f32::NAN,
        normalization: "maximum".to_owned(),
        output: "hologram".to_owned(),
        ..FxSettings::default()
    }
    .clamped();
    assert_eq!(hostile.bands, vec![0.0; 10], "a short curve is a flat one");
    assert_eq!(hostile.bass, 0.0);
    assert_eq!(hostile.balance, 1.0);
    assert_eq!(hostile.crossfade_s, 0.0);
    assert_eq!(hostile.crossfade_skip_s, 0.0);
    assert_eq!(hostile.normalization, "off");
    assert_eq!(hostile.output, "auto");
    let wide = FxSettings {
        bands: vec![99.0; 10],
        ..FxSettings::default()
    }
    .clamped();
    assert_eq!(wide.bands, vec![EQ_MAX_DB; 10], "the chain's ±16 dB");
}

// -- a fader reaches the SOUND within one buffer -------------------------------

mod audio {
    use super::*;

    /// A local FLAC served straight from disk: the engine's opener is the only
    /// thing this measurement replaces, so the mix, the chain and the sink are
    /// the app's own.
    struct FileOpener {
        path: PathBuf,
    }

    /// A file body as the engine's opener hands it out (`Read` + the length it
    /// knows, which is what makes a seek land instead of deferring).
    struct FileStream(std::fs::File);

    impl std::io::Read for FileStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            std::io::Read::read(&mut self.0, buf)
        }
    }

    impl nz_audio::ByteStream for FileStream {
        fn len_hint(&self) -> Option<u64> {
            self.0.metadata().ok().map(|m| m.len())
        }
    }

    impl nz_audio::StreamOpener for FileOpener {
        fn open(
            &self,
            _id: &str,
            _quality: nz_audio::Quality,
            _start: Option<f64>,
            _from_byte: Option<u64>,
            _lane: nz_audio::Lane,
        ) -> Result<Box<dyn nz_audio::ByteStream>, nz_audio::SourceError> {
            let file = std::fs::File::open(&self.path)
                .map_err(|e| nz_audio::SourceError::Message(e.to_string()))?;
            Ok(Box::new(FileStream(file)))
        }

        fn ready(
            &self,
            _id: &str,
            _quality: nz_audio::Quality,
        ) -> Result<nz_audio::StreamReady, nz_audio::SourceError> {
            Ok(nz_audio::StreamReady {
                ready: true,
                ranges: Some("bytes".to_owned()),
                bytes: std::fs::metadata(&self.path).ok().map(|m| m.len()),
                cached: false,
                offset: false,
                streaming: false,
            })
        }

        fn local(&self, _id: &str, _quality: nz_audio::Quality) -> Option<PathBuf> {
            Some(self.path.clone())
        }
    }

    fn tone_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/player/tests/fixtures/tone440.flac")
    }

    /// **Moving a fader changes the sound within ONE buffer.** The person drags a
    /// band; the screen writes the setting; the player hands it to the engine's
    /// atomics. This renders the app's own mixer buffer by buffer and finds the
    /// first buffer whose samples differ from the un-EQ'd reference — the number
    /// the acceptance is held to.
    #[test]
    fn a_fader_change_lands_in_the_next_buffer() {
        let dir = std::env::temp_dir().join(format!("mb12-fx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = nz_audio::EngineConfig {
            sample_rate: 48_000,
            spool_dir: dir.join("spool"),
            ..Default::default()
        };
        let (engine, mut sink) = nz_audio::Engine::new_offline(
            cfg,
            Arc::new(FileOpener {
                path: tone_fixture(),
            }),
        );
        engine.set_track_meta(
            "t",
            nz_audio::TrackMeta {
                duration_s: Some(4.0),
                ..Default::default()
            },
        );
        let events = engine.events();
        engine.load("t", nz_audio::Quality::Flac, None);
        engine.play();

        const FRAMES: usize = 480;
        let mut block = vec![0.0f32; FRAMES * 2];
        // The decoder and the sink are two threads: the engine advances only
        // while the sink pulls, so the wait is "render, then look for the
        // engine's own Started" — an event, never a sleep.
        let mut started = false;
        let mut audible = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline && !audible {
            block.fill(0.0);
            sink.render(&mut block);
            while let Ok(event) = events.try_recv() {
                if matches!(event, nz_audio::EngineEvent::Started { .. }) {
                    started = true;
                }
            }
            audible = started && block.iter().any(|s| s.abs() > 0.01);
            std::thread::yield_now();
        }
        assert!(started, "the engine must announce the track");
        assert!(
            block.iter().any(|s| s.abs() > 0.01),
            "the tone must be playing before the measurement"
        );
        // The reference: a buffer of the tone the decoder really produced, well
        // past a buffer the decode ring starved on (an offline engine renders
        // faster than FLAC decodes, and silence is buffering, not a reading).
        let mut reference: Vec<f32> = Vec::new();
        for _ in 0..200 {
            block.fill(0.0);
            sink.render(&mut block);
            if block.iter().any(|s| s.abs() > 0.01) {
                reference = block.clone();
            }
            std::thread::yield_now();
        }
        assert!(!reference.is_empty(), "an audible reference buffer");
        // The person raises the 440 Hz band: the player applies it at once.
        engine.set_eq(4, 12.0); // 500 Hz, the band over the fixture's tone
        println!("engine eq(4) = {}", engine.eq(4));
        // The very next buffer that carries sound (the decoder's ring is the
        // only reason one would not).
        let mut differing = None;
        for _ in 0..200 {
            block.fill(0.0);
            sink.render(&mut block);
            if !block.iter().any(|s| s.abs() > 0.01) {
                std::thread::yield_now();
                continue;
            }
            differing = block
                .iter()
                .zip(reference.iter())
                .position(|(a, b)| (a - b).abs() > 1e-4);
            break;
        }
        println!(
            "fader change: first differing sample in the FIRST buffer after the change: {:?}",
            differing
        );
        assert!(
            differing.is_some(),
            "the change must be audible in the next buffer, not later"
        );
        // And it really is the band's gain: the tone comes out louder, not
        // merely different.
        let before = reference.iter().fold(0f32, |m, v| m.max(v.abs()));
        let after = block.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(after > before * 1.5, "before {before} after {after}");
    }
}

// -- the meter's session -------------------------------------------------------

struct Counter {
    frames: Arc<AtomicU64>,
    last: Arc<std::sync::Mutex<Option<MeterFrame>>>,
}

impl MeterSink for Counter {
    fn on_frame(&self, frame: MeterFrame) {
        // The frame first, then the count: a reader waiting on the count must
        // never find the frame missing.
        *self.last.lock().unwrap() = Some(frame);
        self.frames.fetch_add(1, Ordering::Relaxed);
    }
}

/// Feed the meter exactly what a mixer produces in one tick — 40 ms, 1 920
/// frames — and wait for that reading on the session's own tick counter, never
/// on the clock. The block is exactly one tick long and a WHOLE number of cycles
/// of the tone: giving the tap more than it can hold would drop frames mid-cycle
/// and make the test's own splices look like a broadband spectrum.
fn feed_one_tick(feed: &mut nz_audio::TapFeed, session: &MeterSession, block: &[f32]) -> u64 {
    let before = session.ticks();
    // EXACTLY what the mixer produces between two readings. More would overflow
    // the tap's ring, and a spliced tone reads as a broadband spectrum that is
    // the test's own doing, not the meter's.
    feed.push(block);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while session.ticks() == before && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    session.ticks()
}

/// One tick of a `hz` tone: 1 920 frames (40 ms), a whole number of periods.
fn tick_of(hz: f64, amp: f64) -> Vec<f32> {
    let mut block = vec![0.0f32; 1_920 * 2];
    for i in 0..1_920 {
        let x = amp * (std::f64::consts::TAU * hz * i as f64 / 48_000.0).sin();
        block[i * 2] = x as f32;
        block[i * 2 + 1] = x as f32;
    }
    block
}

/// **The meter reads while a card is subscribed, and not one tick more once it
/// leaves.** The session's thread is the card's; `stop_and_join` returns only
/// after the last reading, and `ticks()` then stands still — the acceptance's
/// "0 analysis CPU quand la carte disparaît", counted.
///
/// Measured: a 1 kHz tone in the tap's frames puts its energy on band 5, the
/// chain's numbers arrive unchanged, and after the stop **0 new ticks** over
/// 200 ms (five would-be readings).
#[test]
fn the_meter_reads_while_subscribed_and_stops_completely() {
    let (mut feed, subscription) = nz_audio::tap_channel(8_192);
    let frames = Arc::new(AtomicU64::new(0));
    let last = Arc::new(std::sync::Mutex::new(None));
    let sink = Arc::new(Counter {
        frames: Arc::clone(&frames),
        last: Arc::clone(&last),
    });
    // A tap with audio in it: 1 kHz at 0.25 on both channels.
    let block = tick_of(1_000.0, 0.25);

    let chain: meter::ChainReading = Arc::new(|| (-8.5, 1.25));
    let curve: meter::Curve = Arc::new(|| (false, vec![0.0; 10]));
    let mut session = MeterSession::start(subscription, chain, curve, sink);

    // A few readings, fed exactly as the mixer would (40 ms at a time): the
    // 85 ms window fills by the third one.
    for _ in 0..3 {
        feed_one_tick(&mut feed, &session, &block);
    }
    assert!(
        frames.load(Ordering::Relaxed) > 0,
        "the meter took a reading"
    );
    let frame = last.lock().unwrap().clone().expect("a frame");
    assert!(frame.available);
    assert!(frame.measured);
    assert_eq!(frame.reduction_db, 1.3, "the chain's number, rounded once");
    assert_eq!(frame.peak_db, -8.5);
    assert_eq!(frame.headroom_db, 8.5);
    assert!(
        frame.bands[5] > 0.5,
        "1 kHz must light its own band: {:?}",
        frame.bands
    );
    assert!(frame.bands[3] < 0.2, "and not 250 Hz: {:?}", frame.bands);

    // The card leaves.
    session.stop_and_join();
    let ticks = session.ticks();
    assert!(ticks > 0);
    // Nothing arrives afterwards, even with the tap still being fed: five ticks
    // would have landed in this window.
    let frames_at_stop = frames.load(Ordering::Relaxed);
    feed.push(&block);
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_millis(200) {
        std::thread::yield_now();
    }
    assert_eq!(
        session.ticks(),
        ticks,
        "0 analysis ticks after the card leaves"
    );
    assert_eq!(frames.load(Ordering::Relaxed), frames_at_stop);
}

/// **The warning is about the SETTING, not the recording** (`playback.md`): the
/// hottest master with the equalizer off opens nothing; the same reading with a
/// band above 0 dB opens the banner; and a change starts a fresh episode from
/// the level on screen, so lowering the band takes the banner away.
#[test]
fn the_warning_follows_the_setting_not_the_recording() {
    let (mut feed, subscription) = nz_audio::tap_channel(8_192);
    let frames = Arc::new(AtomicU64::new(0));
    let last = Arc::new(std::sync::Mutex::new(None));
    let sink = Arc::new(Counter {
        frames: Arc::clone(&frames),
        last: Arc::clone(&last),
    });
    // The chain says: +7.3 dBFS pre-limiter and 6.4 dB of reduction — the bench's
    // hot master (`playback.md`).
    let chain: meter::ChainReading = Arc::new(|| (7.3, 6.4));
    // A person's curve: the switch off, and one band lifted behind it.
    let enabled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let curve: meter::Curve = {
        let enabled = Arc::clone(&enabled);
        Arc::new(move || {
            let mut bands = vec![0.0f32; 10];
            bands[1] = 6.0;
            (enabled.load(Ordering::Relaxed), bands)
        })
    };
    let block = tick_of(1_000.0, 0.1);
    let mut session = MeterSession::start(subscription, chain, curve, sink);
    feed_one_tick(&mut feed, &session, &block);
    feed_one_tick(&mut feed, &session, &block);

    // The equalizer is off: a loud master is the recording's level, and the card
    // says nothing about it.
    assert_eq!(
        last.lock().unwrap().clone().unwrap().warning,
        None,
        "a bypassed equalizer never warns, however hard the limiter works"
    );

    // Switched on with a band above 0 dB: the same reading is now the setting's
    // doing, and the banner appears with the measurement (no baseline yet, so
    // the raw reduction is the only honest number).
    enabled.store(true, Ordering::Relaxed);
    feed_one_tick(&mut feed, &session, &block);
    let frame = last.lock().unwrap().clone().expect("a frame");
    let warning = frame.warning.expect("the equalizer is now the cause");
    assert_eq!(warning.tone, Tone::Clip, "+7,3 dBFS pre-limiter is a clip");
    assert_eq!(warning.reduction_db, 6.4, "the measured reduction");
    assert_eq!(warning.cut_db, 6.5, "half a decibel, from the measurement");
    assert!(!warning.from_change, "nothing was measured when it started");

    // A deliberate change starts a new episode from the level on screen: with the
    // same chain nothing was ADDED, so no banner — the person's own action is the
    // baseline, and the 160 ms hold lets the analyser's window turn over.
    session.note_change("band:1");
    for _ in 0..8 {
        feed_one_tick(&mut feed, &session, &block);
    }
    assert_eq!(
        last.lock().unwrap().clone().unwrap().warning,
        None,
        "a change starts over: the old banner is gone"
    );
    session.stop_and_join();
}

/// The state's clock is the SPA's, and the trim threshold the card shows is the
/// one the player really sends.
#[test]
fn the_card_shows_the_threshold_the_player_sends() {
    let now = now_ms();
    assert!(
        now > 1_700_000_000_000.0,
        "ms since the epoch, like Date.now()"
    );
    assert_eq!(TRIM_DB, -45.0);
    assert_eq!(TRIM_DB as i32, -45);
}
