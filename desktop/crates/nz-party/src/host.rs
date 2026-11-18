//! Écoute partagée — the HOST side: this device's player, watched and published.
//!
//! The contract is `neezer/webui/party.py`, the reference behaviour is
//! `webapp/src/lib/party/host.js` + `hostrules.js` + `anchor.js`, and the notes
//! are [party-remote.md](../../../../docs/agents/party-remote.md).
//!
//! - **The host's player is left exactly as it is.** Its queue, its crossfade,
//!   its trim, its effects. This module only WATCHES it: four times a second it
//!   fits the position it HEARS (the engine's own clock, which already subtracts
//!   the device latency) against the party clock, and publishes the line
//!   `{t, p}` — "at server instant `t` the room was at position `p`". Between
//!   those it publishes nothing but a heartbeat, because guests extrapolate from
//!   the line instead of polling for the position.
//! - **The line is a fit, not a reading.** A single position read is good to a
//!   few milliseconds: the value steps per audio callback and the read lands
//!   somewhere in a timer callback. Every reading while the track plays sits on
//!   `position = c + time`, so the MEDIAN of the recent `c`s is far steadier
//!   (`appcore::sync::AnchorFit`, the same Rust the page runs), and a reading
//!   far off the line is a seek or a stall: the fit breaks there and starts over.
//! - **The next track is ANNOUNCED**, with where the handover lands (`at`), its
//!   trimmed start, the crossfade, and `gap` — how late this player usually is
//!   to actually start it, learnt from its own previous handovers. That is what
//!   lets a guest start the next track ON the beat instead of a poll after.
//! - **Rule 1: publishing never competes with the music.** One publish in
//!   flight, coalesced, on `Lane::User` (the lane that never waits), only when
//!   the line MOVED; the rest of the time a heartbeat every 10 s. Both stop the
//!   moment the party is over.
//!
//! A desktop app also has to survive its OWN restart: the launch resume asks the
//! server whether this account hosts a party and re-opens it ([`Host::should_ask`],
//! [`Host::saved_id`]), and a party the SERVER forgot (its registry is in memory)
//! is taken back under the same id by [`Host::recover`], so the link in
//! everybody's hands keeps working.
//!
//! What is NOT here, on purpose: reading this device's player (`state.player`,
//! `state.audio`), the shell's timer that calls [`Host::tick`], and the toasts.
//! Those are the desktop adapter's (`desktop/app/src/wiring/party_host.rs`); the
//! phone has its own.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use appcore::sync::AnchorFit;
use nz_api::{Api, ApiError};
use nz_audio::Normalization;
use nz_cache::Cache;
use nz_model::{Party, PartyNext, PartyPublish, PartyTrack};
use nz_player::Entry;
use serde_json::json;
use tracing::{debug, info};

use crate::clock::{ServerClock, local_ms};
use crate::timeline::{Timeline, position_at, server_time_at};

// --- the SPA's own numbers ----------------------------------------------------

/// One measurement a quarter of a second (`host.js#MEASURE_MS`).
pub const MEASURE: Duration = Duration::from_millis(250);
/// …and one every 40 ms while the line is young (`QUICK_MS`): the first reading
/// after a start is the worst there is, and a line drawn through it was measured
/// 23 ms off (the SPA's own note).
pub const QUICK: Duration = Duration::from_millis(40);
/// A heartbeat says "still here" when nothing else was published for this long.
pub const HEARTBEAT: Duration = Duration::from_secs(10);
/// Republish when the player is this far off its own line, seconds (`DRIFT_S`).
pub const DRIFT_S: f64 = 0.0025;
/// …and at once when it is this far: a seek, a stall, a restart (`JUMP_S`).
pub const JUMP_S: f64 = 0.25;
/// The least time between two publishes of the same track (`MIN_GAP_MS`).
pub const MIN_GAP: Duration = Duration::from_millis(150);
/// Readings a fresh line needs before it goes out (`FIT_FIRST`), and before it
/// is trusted enough to be corrected on drift (`FIT_GOOD`).
pub const FIT_FIRST: usize = 3;
pub const FIT_GOOD: usize = 8;
/// The store key the SPA keeps: the party this device hosts, so a restart picks
/// it up (`HOST_KEY`). Its window is below the server's `PARTY_TTL` (30 min).
pub const HOST_KEY: &str = "party.host";
pub const RESUME_WINDOW: Duration = Duration::from_secs(25 * 60);

// --- what the screen paints ---------------------------------------------------

/// One guest, as the host's list names it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listener {
    pub name: String,
    /// The guest's own sync estimate, milliseconds.
    pub q: Option<f64>,
    /// `sync`, `load`, `pause`, `idle` — its word for what it is doing.
    pub st: Option<String>,
}

impl Listener {
    /// The line under a guest's name (`PartySheet.svelte#statusOf`).
    pub fn status(&self) -> String {
        match self.st.as_deref() {
            Some("sync") => match self.q {
                Some(q) => format!("En phase · ±{:.1} ms", q.max(0.1)),
                None => "En phase".to_owned(),
            },
            Some("load") => "Chargement…".to_owned(),
            Some("pause") => "En pause".to_owned(),
            _ => "Connecté".to_owned(),
        }
    }
}

/// What the sheet and the sidebar read: a clone under a mutex, with a version so
/// a repaint happens once per change.
#[derive(Clone, Debug, Default)]
pub struct HostStatus {
    /// The party this device hosts, when it hosts one.
    pub id: String,
    /// The URL the guests open (`<server>/party/<id>`).
    pub link: String,
    pub listeners: Vec<Listener>,
    /// `(spread, rtt)` of this device's own clock estimate, milliseconds.
    pub clock: Option<(f64, f64)>,
    /// The server forgot the party and the id could not be taken back: the link
    /// the guests hold is dead, and the sheet says so.
    pub lost: bool,
    /// The loops are running.
    pub active: bool,
    /// The last cause, in French.
    pub error: Option<String>,
    pub version: u64,
}

impl HostStatus {
    pub fn hosting(&self) -> bool {
        !self.id.is_empty()
    }

    /// `« Personne à l'écoute »`, `« 1 personne à l'écoute »`, … — the header
    /// line, the SPA's own wording.
    pub fn audience(&self) -> String {
        match self.listeners.len() {
            0 => "Personne à l'écoute".to_owned(),
            1 => "1 personne à l'écoute".to_owned(),
            n => format!("{n} personnes à l'écoute"),
        }
    }
}

// --- the wire (pure) ----------------------------------------------------------

/// The track a publish carries (`host.js#trackInfo`): the id the guest's chunk
/// route takes (a Deezer number, or a UUID), and what the sheet shows.
pub fn track_info(entry: &Entry, gain: Option<f64>) -> PartyTrack {
    PartyTrack {
        id: entry.reference().unwrap_or_default().to_owned(),
        title: entry.title.clone(),
        artist: entry.artist_name().to_owned(),
        album: entry
            .album
            .as_ref()
            .map(|album| album.title.clone())
            .unwrap_or_default(),
        duration: f64::from(entry.duration),
        podcast: entry.is_podcast(),
        gain,
        cover: entry.cover.clone(),
    }
}

/// `off`/`low`/`medium`/`high` — the level the guests apply, so every device in
/// the room plays each track at the same relative level.
pub fn norm_wire(normalization: Normalization) -> &'static str {
    match normalization {
        Normalization::Off => "off",
        Normalization::Low => "low",
        Normalization::Medium => "medium",
        Normalization::High => "high",
    }
}

/// The gap a player leaves between "the next track should start" and "it
/// starts", learnt from its own handovers (`host.js#planNext`'s two defaults).
pub fn default_gap(fade: f64) -> f64 {
    if fade > 0.0 { 0.02 } else { 0.15 }
}

/// A cheap identity for a handover plan: the track, where it lands, its start,
/// its fade and its gap. A plan that differs in any of those is a new plan.
pub fn next_key(next: Option<&PartyNext>) -> String {
    match next {
        None => String::new(),
        Some(next) => format!(
            "{}|{:.3}|{:.3}|{:.3}|{:.3}",
            next.track
                .as_ref()
                .map(|track| track.id.as_str())
                .unwrap_or(""),
            next.at,
            next.start,
            next.fade,
            next.gap
        ),
    }
}

/// What guests currently believe, which is what a new line is compared with
/// (`host.js#measure`'s `need`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Belief {
    pub id: String,
    pub playing: bool,
    pub buf: bool,
    pub next_key: String,
    pub norm: String,
    pub t: f64,
    pub p: f64,
}

impl Belief {
    pub fn of(publish: &PartyPublish) -> Belief {
        Belief {
            id: publish
                .track
                .as_ref()
                .map(|track| track.id.clone())
                .unwrap_or_default(),
            playing: publish.playing,
            buf: publish.buf,
            next_key: next_key(publish.next.as_ref()),
            norm: publish.norm.clone(),
            t: publish.t,
            p: publish.p,
        }
    }

    /// The line guests extrapolate from.
    pub fn timeline(&self) -> Timeline {
        Timeline {
            t: self.t,
            p: self.p,
            playing: self.playing,
        }
    }
}

/// What this device is playing, as one tick reads it — the numbers the fit and
/// the publish are made of. A struct because each one is a measured quantity.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub id: String,
    /// The track's metadata for the publish.
    pub track: PartyTrack,
    /// The engine carries THIS track (not the previous one it still holds
    /// between a skip and the new source being attached).
    pub carries: bool,
    /// The person's own transport says it plays.
    pub playing: bool,
    /// What is heard right now, seconds into the track (the engine's clock).
    pub heard: f64,
    /// The configured crossfade, seconds (0 when the person turned it off).
    pub fade: f64,
    pub duration: f64,
    pub norm: &'static str,
    /// The device's output-clock generation: a step in it invalidates the fit.
    pub generation: u64,
    /// The planned handover, when there is a next track.
    pub next: Option<PartyNext>,
}

/// A position reading and whether it continued the line.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    pub pos: f64,
    pub broke: bool,
}

/// The host's line: the fit, and the two things that start it over (another
/// track, another output mapping).
pub struct Line {
    pub fit: AnchorFit,
    track: String,
    generation: u64,
    seen: bool,
}

impl Default for Line {
    fn default() -> Line {
        Line::new()
    }
}

impl Line {
    pub fn new() -> Line {
        Line {
            fit: AnchorFit::new(24, 0.25),
            track: String::new(),
            generation: 0,
            seen: false,
        }
    }

    /// One reading: `pos` seconds heard at `local_ms` of this device's clock.
    /// `broke` is a reading that did not continue the line (a pause, a seek, a
    /// stall) — the fit starts over from the next one.
    pub fn add(&mut self, track: &str, generation: u64, local_ms: f64, reading: Reading) {
        if !self.seen || self.track != track || self.generation != generation {
            self.seen = true;
            self.track = track.to_owned();
            self.generation = generation;
            self.fit.reset();
        }
        if reading.broke {
            self.fit.reset();
        } else {
            self.fit.add(local_ms, reading.pos);
        }
    }

    pub fn readings(&self) -> usize {
        self.fit.len()
    }

    /// Where the line puts the playhead at `local_ms`.
    pub fn position_at(&mut self, local_ms: f64) -> Option<f64> {
        self.fit.position_at(local_ms)
    }
}

/// The two gaps this player has learnt, one for a cut and one for a crossfade
/// (`host.js`'s `gaps`). Both start at the SPA's defaults.
#[derive(Clone, Copy, Debug)]
pub struct Gaps {
    pub cut: f64,
    pub fade: f64,
}

impl Default for Gaps {
    fn default() -> Self {
        Gaps {
            cut: default_gap(0.0),
            fade: default_gap(1.0),
        }
    }
}

impl Gaps {
    pub fn get(&self, fade: f64) -> f64 {
        if fade > 0.0 { self.fade } else { self.cut }
    }

    /// Blend a measured handover in: `host.js`'s `old * 0.5 + g * 0.5`. A gap
    /// outside what a start-up can explain is not a measurement.
    pub fn learn(&mut self, fade: bool, measured: f64) {
        if !(-0.05..2.5).contains(&measured) {
            return;
        }
        let measured = measured.max(0.0);
        if fade {
            self.fade = self.fade * 0.5 + measured * 0.5;
        } else {
            self.cut = self.cut * 0.5 + measured * 0.5;
        }
    }
}

/// The handover being measured: what was predicted, and for which track.
#[derive(Clone, Debug, PartialEq)]
pub struct GapProbe {
    pub track: String,
    /// The server instant the handover was predicted for.
    pub predicted: f64,
    /// Where the next track starts (its trimmed lead-in, 0 today).
    pub start: f64,
    pub fade: bool,
}

/// The handover this player plans for the next track (`host.js#planNext`).
pub fn next_plan(
    player: &nz_player::PlayerState,
    current: &Entry,
    gaps: &Gaps,
    fade: f64,
) -> Option<PartyNext> {
    let next = player.next_entry()?;
    if !player.playing() || next.reference().is_none() {
        return None;
    }
    let duration = if current.duration > 0 {
        f64::from(current.duration)
    } else {
        player.duration()
    };
    if duration <= 0.0 {
        return None;
    }
    let fade = fade.min(duration / 2.0).max(0.0);
    Some(PartyNext {
        track: Some(track_info(next, next.gain)),
        // Where in the CURRENT track the handover happens: the crossfade begins
        // that much before the end, a cut lands on it.
        at: (duration - fade).max(0.0),
        // The next track's trimmed lead-in would go here; this build keeps its
        // measured bounds private to `state.rs`, so a guest starts at 0 and the
        // seam's crossfade hides the few milliseconds a trim would have removed.
        start: 0.0,
        fade,
        gap: gaps.get(fade),
    })
}

/// Whether a new line has to go out (`host.js#measure`'s `need`, field for
/// field), and the publish to send when it does.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decision {
    pub publish: Option<PartyPublish>,
    /// The handover to measure once its track really starts.
    pub probe: Option<GapProbe>,
    /// A handover that really started: `(a crossfade?, the gap in seconds)`.
    pub gap: Option<(bool, f64)>,
    /// The line is still young: read again in 40 ms instead of 250.
    pub quick: bool,
}

/// One tick, as a pure function of what was read: the fit, the plan, and what
/// the guests believe. Split out of [`Host::tick`] so every rule above is a
/// unit test.
pub struct Tick<'a> {
    pub snapshot: Option<&'a Snapshot>,
    pub line: &'a mut Line,
    pub gaps: &'a Gaps,
    /// The handover this device announced, while it is still being measured.
    pub probe: Option<&'a GapProbe>,
    /// The line this device last sent, `None` while it never sent one.
    pub believed: Option<&'a PartyPublish>,
    pub since_sent: Duration,
    pub local_ms: f64,
    pub offset_ms: f64,
}

pub fn decide(tick: Tick<'_>) -> Decision {
    let Tick {
        snapshot,
        line,
        gaps,
        probe,
        believed,
        since_sent,
        local_ms,
        offset_ms,
    } = tick;
    let _ = gaps;
    let Some(snapshot) = snapshot else {
        // Nothing in the queue at all: the room is told once, and then nothing.
        let already_idle = believed.is_none_or(|publish| publish.track.is_none());
        if already_idle {
            return Decision::default();
        }
        return Decision {
            publish: Some(PartyPublish {
                track: None,
                playing: false,
                ..PartyPublish::default()
            }),
            ..Decision::default()
        };
    };
    let running = snapshot.carries && snapshot.playing;
    line.add(
        &snapshot.id,
        snapshot.generation,
        local_ms,
        Reading {
            pos: snapshot.heard,
            broke: !running,
        },
    );
    let readings = line.readings();
    let quick = running && readings < FIT_GOOD;
    // The handover we announced really happened: how late was this player to
    // start that track? (`host.js`: `startedAt = t + (start - p) * 1000`, taken
    // against the instant the handover was predicted for.)
    let gap = match probe {
        Some(probe) if running && snapshot.id == probe.track && readings >= 6 => {
            let started = local_ms + offset_ms + (probe.start - snapshot.heard) * 1000.0;
            Some((probe.fade, (started - probe.predicted) / 1000.0))
        }
        _ => None,
    };
    if running && readings < FIT_FIRST {
        // One or two readings are not a line yet.
        return Decision {
            quick,
            gap,
            ..Decision::default()
        };
    }
    let position = if running {
        line.position_at(local_ms).unwrap_or(snapshot.heard)
    } else {
        snapshot.heard
    };
    let buffering = !running && snapshot.playing;
    let mut publish = PartyPublish {
        track: Some(snapshot.track.clone()),
        playing: running,
        buf: buffering,
        t: local_ms + offset_ms,
        p: position,
        xfade: if snapshot.fade > 0.0 && position < snapshot.fade {
            snapshot.fade
        } else {
            0.0
        },
        next: snapshot.next.clone(),
        norm: snapshot.norm.to_owned(),
        hb: false,
    };
    if !running && !buffering {
        // Paused: the line stops moving, and `p` is where the person paused.
        publish.p = snapshot.heard;
    }
    let previous = believed.map(Belief::of).unwrap_or_default();
    let plan_key = next_key(publish.next.as_ref());
    let mut need = believed.is_none()
        || previous.id != snapshot.id
        || previous.playing != publish.playing
        || previous.buf != publish.buf
        || previous.next_key != plan_key
        || previous.norm != publish.norm;
    if !need && running {
        let drift = (position_at(previous.timeline(), publish.t) - position).abs();
        if drift > JUMP_S {
            need = true;
        } else if readings >= FIT_GOOD && drift > DRIFT_S {
            need = since_sent >= MIN_GAP;
        }
    } else if !need && !running && (previous.p - position).abs() > 0.05 {
        // Seeked while paused.
        need = true;
    }
    if !need {
        return Decision {
            quick,
            gap,
            ..Decision::default()
        };
    }
    // The handover this line announces: remember where it predicts the next
    // track starts, so the gap can be measured when the player really gets there.
    let announced = publish.next.as_ref().map(|next| GapProbe {
        track: next
            .track
            .as_ref()
            .map(|track| track.id.clone())
            .unwrap_or_default(),
        predicted: server_time_at(previous.timeline(), next.at),
        start: next.start,
        fade: next.fade > 0.0,
    });
    Decision {
        publish: Some(publish),
        probe: announced,
        gap,
        quick,
    }
}

// --- the service --------------------------------------------------------------

struct Shared {
    status: Mutex<HostStatus>,
    api: Mutex<Option<Arc<Api>>>,
    runtime: Mutex<Option<tokio::runtime::Handle>>,
    cache: Mutex<Option<Cache>>,
    /// The address the link is built from (no trailing slash).
    server: Mutex<String>,
    clock: Mutex<Option<Arc<ServerClock>>>,
    /// The party's id, and its own route prefix.
    id: Mutex<String>,
    base: Mutex<String>,
    /// The measure tick's own state: the fit, the learnt gaps, the handover
    /// being measured, and the line guests hold.
    line: Mutex<Line>,
    gaps: Mutex<Gaps>,
    probe: Mutex<Option<GapProbe>>,
    believed: Mutex<Option<PartyPublish>>,
    last_sent: Mutex<Option<Instant>>,
    /// The last state this device wants published, and whether it is due.
    publish: Mutex<Option<PartyPublish>>,
    due: AtomicBool,
    stop: Arc<AtomicBool>,
    version: AtomicU64,
}

static SERVICE: std::sync::OnceLock<Host> = std::sync::OnceLock::new();

/// The host service, cheap to clone. A process-wide slot like
/// `nz_offline::Offline`'s: the party outlives the screen that started it.
#[derive(Clone)]
pub struct Host {
    shared: Arc<Shared>,
}

impl Host {
    pub fn get() -> Option<Host> {
        SERVICE.get().cloned()
    }

    pub fn status(&self) -> HostStatus {
        self.shared.status.lock().expect("party poisoned").clone()
    }

    pub fn id(&self) -> String {
        self.shared.id.lock().expect("party poisoned").clone()
    }

    /// The address the guests' links are built from.
    pub fn server(&self) -> String {
        self.shared.server.lock().expect("party poisoned").clone()
    }

    /// The two gaps this player has learnt, which a snapshot's handover plan
    /// needs.
    pub fn gaps(&self) -> Gaps {
        *self.shared.gaps.lock().expect("party poisoned")
    }

    pub fn running(&self) -> bool {
        !self.shared.stop.load(Ordering::Acquire) && !self.id().is_empty()
    }

    fn service() -> Host {
        SERVICE
            .get_or_init(|| Host {
                shared: Arc::new(Shared {
                    status: Mutex::new(HostStatus::default()),
                    api: Mutex::new(None),
                    runtime: Mutex::new(None),
                    cache: Mutex::new(None),
                    server: Mutex::new(String::new()),
                    clock: Mutex::new(None),
                    id: Mutex::new(String::new()),
                    base: Mutex::new(String::new()),
                    line: Mutex::new(Line::new()),
                    gaps: Mutex::new(Gaps::default()),
                    probe: Mutex::new(None),
                    believed: Mutex::new(None),
                    last_sent: Mutex::new(None),
                    publish: Mutex::new(None),
                    due: AtomicBool::new(false),
                    stop: Arc::new(AtomicBool::new(true)),
                    version: AtomicU64::new(1),
                }),
            })
            .clone()
    }

    /// Give the service this session's client, runtime and store.
    pub fn attach(
        api: Arc<Api>,
        cache: Cache,
        runtime: tokio::runtime::Handle,
        server: String,
    ) -> Host {
        let host = Host::service();
        *host.shared.api.lock().expect("party poisoned") = Some(api);
        *host.shared.runtime.lock().expect("party poisoned") = Some(runtime);
        *host.shared.cache.lock().expect("party poisoned") = Some(cache);
        *host.shared.server.lock().expect("party poisoned") = server;
        host
    }

    /// Forget the last cause before a new attempt, so the sheet does not show a
    /// failure that is being retried.
    pub fn clear_error(&self) {
        self.changed(|status| status.error = None);
    }

    /// The launch resume asks the server only when it can pay (rule 3): this
    /// device hosted a party before, or it still holds one. A feature nobody
    /// used costs a launch no request.
    pub fn should_ask(&self) -> bool {
        self.saved_id().is_some() || self.ever_hosted() || !self.status().id.is_empty()
    }

    /// The party this device remembers hosting, when it is recent enough
    /// (`hostbridge.js#RESUME_WINDOW`, below the server's own 30 minutes).
    pub fn saved_id(&self) -> Option<String> {
        let cache = self.shared.cache.lock().expect("party poisoned").clone()?;
        Host::saved(&cache)
    }

    fn ever_hosted(&self) -> bool {
        self.shared
            .cache
            .lock()
            .expect("party poisoned")
            .as_ref()
            .and_then(|cache| cache.kv().get::<serde_json::Value>(HOST_KEY))
            .is_some()
    }

    /// The party this device remembers hosting, from the store alone.
    pub fn saved(cache: &Cache) -> Option<String> {
        let value = cache.kv().get::<serde_json::Value>(HOST_KEY)?;
        let id = value.get("id").and_then(serde_json::Value::as_str)?;
        let at = value.get("at").and_then(serde_json::Value::as_u64)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0);
        if now.saturating_sub(at) > RESUME_WINDOW.as_millis() as u64 || id.len() < 22 {
            return None;
        }
        Some(id.to_owned())
    }

    /// Adopt a party: its link and its guests for the sheet, the clock, the
    /// publish loop and the measure tick. Returns the link the guests open.
    ///
    /// The shell starts its own tick after this; the crate owns everything else.
    pub fn begin(&self, party: &Party) -> String {
        let server = self.server();
        let id = party.id.clone();
        let link = format!(
            "{}{}",
            server.trim_end_matches('/'),
            if party.path.is_empty() {
                format!("/party/{id}")
            } else {
                party.path.clone()
            }
        );
        let listeners = listeners_of(party);
        *self.shared.id.lock().expect("party poisoned") = id.clone();
        *self.shared.base.lock().expect("party poisoned") = format!("/party/{id}");
        self.shared.stop.store(false, Ordering::Release);
        self.shared.believed.lock().expect("party poisoned").take();
        self.shared.probe.lock().expect("party poisoned").take();
        *self.shared.last_sent.lock().expect("party poisoned") = None;
        *self.shared.line.lock().expect("party poisoned") = Line::new();
        self.changed(|status| {
            status.id = id.clone();
            status.link = link.clone();
            status.listeners = listeners;
            status.lost = false;
            status.active = true;
            status.error = None;
        });
        self.open_loops(&id);
        self.remember(Some(&id));
        info!(%id, "écoute partagée : ce lecteur anime");
        link
    }

    /// Stop hosting, without a request: the loops stop, the store forgets the
    /// party, and the sheet clears. Returns the id the caller should close on the
    /// server.
    pub fn close(&self) -> String {
        let id = self.id();
        self.shutdown();
        self.remember(None);
        self.changed(|status| {
            status.id.clear();
            status.link.clear();
            status.listeners.clear();
            status.active = false;
            status.lost = false;
        });
        id
    }

    /// Somebody else holds the id this device remembered: this account did not
    /// ask for a new party, so none is created and the sheet says why.
    pub fn another_holds_it(&self) {
        self.remember(None);
        self.shutdown();
        self.changed(|status| {
            status.error =
                Some("Ce lien d'écoute appartient maintenant à quelqu'un d'autre.".into());
        });
    }

    /// Stop every loop without a request (a logout, a session that ended).
    pub fn shutdown(&self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(clock) = self
            .shared
            .clock
            .lock()
            .expect("party poisoned")
            .as_ref()
            .cloned()
        {
            clock.stop();
        }
        self.changed(|status| status.active = false);
    }

    pub fn fail(&self, cause: String) {
        self.changed(|status| {
            status.error = Some(cause);
            status.active = false;
        });
    }

    fn changed(&self, change: impl FnOnce(&mut HostStatus)) {
        let mut status = self.shared.status.lock().expect("party poisoned");
        change(&mut status);
        let version = self.shared.version.fetch_add(1, Ordering::AcqRel) + 1;
        status.version = version;
    }

    fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::Acquire)
    }

    /// The clock probes and the publish loop for one party.
    fn open_loops(&self, id: &str) {
        let (Some(api), Some(runtime)) = (
            self.shared.api.lock().expect("party poisoned").clone(),
            self.shared.runtime.lock().expect("party poisoned").clone(),
        ) else {
            return;
        };
        // The party's own base: the clock loop appends `/clock` itself.
        let clock = ServerClock::open(Arc::clone(&api), runtime.clone(), format!("/party/{id}"));
        if let Some(old) = self
            .shared
            .clock
            .lock()
            .expect("party poisoned")
            .replace(clock)
        {
            old.stop();
        }
        let this = self.clone();
        runtime.spawn(async move { this.publish_loop(api).await });
    }

    /// Hand a publish to the network task: one in flight, and a write that
    /// arrives during it coalesces into the next one (`host.js#send`).
    fn publish(&self, publish: PartyPublish) {
        *self.shared.publish.lock().expect("party poisoned") = Some(publish);
        self.shared.due.store(true, Ordering::Release);
    }

    fn take_due(&self) -> Option<PartyPublish> {
        if !self.shared.due.swap(false, Ordering::AcqRel) {
            return None;
        }
        self.shared.publish.lock().expect("party poisoned").clone()
    }

    // --- the network side ----------------------------------------------------

    /// Send what the tick handed over, a heartbeat every ten seconds otherwise,
    /// and take the id back if the server forgot it.
    async fn publish_loop(&self, api: Arc<Api>) {
        let mut last_sent = Instant::now();
        while !self.stopped() {
            let id = self.id();
            if id.is_empty() {
                break;
            }
            let body = match self.take_due() {
                Some(publish) => publish,
                None if last_sent.elapsed() >= HEARTBEAT => PartyPublish {
                    hb: true,
                    ..PartyPublish::default()
                },
                None => {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    continue;
                }
            };
            let answer: Result<Party, ApiError> = api.party_publish(&id, &body).await;
            last_sent = Instant::now();
            match answer {
                Ok(party) => {
                    let listeners = listeners_of(&party);
                    let clock = self
                        .shared
                        .clock
                        .lock()
                        .expect("party poisoned")
                        .as_ref()
                        .and_then(|clock| clock.quality());
                    self.changed(|status| {
                        status.listeners = listeners;
                        status.clock = clock;
                        status.active = true;
                        status.error = None;
                    });
                    self.remember(Some(&id));
                }
                Err(ApiError::Http { status: 404, .. }) => {
                    // The server restarted (its registry is in memory): take the
                    // same id back, so the link the guests hold keeps working.
                    self.recover(&api, &id).await;
                }
                Err(error) => {
                    // It never arrived: the guests still hold the previous line,
                    // and the next tick publishes again.
                    debug!(%error, "publication de l'écoute partagée");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    async fn recover(&self, api: &Api, id: &str) {
        match api.party_start(Some(&json!({ "resume": id }))).await {
            Ok(party) if party.id == id => {
                // The server has no state at all: everything goes out again.
                self.shared.believed.lock().expect("party poisoned").take();
                *self.shared.last_sent.lock().expect("party poisoned") = None;
                self.changed(|status| {
                    status.active = true;
                    status.lost = false;
                });
                debug!("écoute partagée : partie reprise après un redémarrage");
            }
            Ok(party) => {
                let _ = api.party_end(&party.id).await;
                self.shutdown();
                self.changed(|status| {
                    status.lost = true;
                    status.error = Some("Le serveur a redémarré et le lien a changé.".to_owned());
                });
            }
            Err(error) => {
                debug!(%error, "reprise de l'écoute partagée");
                self.changed(|status| status.error = Some(error.cause_fr()));
            }
        }
    }

    /// Remember the party for the next launch (`party.host`), or forget it.
    fn remember(&self, id: Option<&str>) {
        let cache = self.shared.cache.lock().expect("party poisoned").clone();
        let Some(cache) = cache else { return };
        let kv = cache.kv();
        match id {
            Some(id) => {
                let at = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_millis() as u64)
                    .unwrap_or(0);
                let _ = kv.set(HOST_KEY, &json!({ "id": id, "at": at }));
            }
            None => {
                let _ = kv.set(HOST_KEY, &serde_json::Value::Null);
            }
        }
    }

    // --- the measure tick ----------------------------------------------------

    /// One measurement: fit the line, publish it when it moved. The caller reads
    /// this device's player (the shell's own state) and hands the numbers in.
    /// Returns whether the line is still young (`host.js`'s 40 ms cadence).
    pub fn tick(&self, snapshot: Option<&Snapshot>) -> bool {
        let offset = self
            .shared
            .clock
            .lock()
            .expect("party poisoned")
            .as_ref()
            .and_then(|clock| clock.offset_ms());
        let Some(offset) = offset else {
            // No clock estimate yet: an anchor would be meaningless.
            return false;
        };
        let local_ms = local_ms();
        let gaps = *self.shared.gaps.lock().expect("party poisoned");
        let believed = self.shared.believed.lock().expect("party poisoned").clone();
        let since = self
            .shared
            .last_sent
            .lock()
            .expect("party poisoned")
            .map(|at| at.elapsed())
            .unwrap_or(Duration::MAX);
        let mut line = self.shared.line.lock().expect("party poisoned");
        let probe = self.shared.probe.lock().expect("party poisoned").clone();
        let decision = decide(Tick {
            snapshot,
            line: &mut line,
            gaps: &gaps,
            probe: probe.as_ref(),
            believed: believed.as_ref(),
            since_sent: since,
            local_ms,
            offset_ms: offset,
        });
        drop(line);
        // A handover that really started: its measured gap joins the two this
        // player has learnt, and the probe is spent.
        if let Some((fade, measured)) = decision.gap {
            self.shared
                .gaps
                .lock()
                .expect("party poisoned")
                .learn(fade, measured);
            *self.shared.probe.lock().expect("party poisoned") = None;
        }
        if let Some(probe) = decision.probe {
            *self.shared.probe.lock().expect("party poisoned") = Some(probe);
        }
        if let Some(publish) = decision.publish {
            *self.shared.believed.lock().expect("party poisoned") = Some(publish.clone());
            *self.shared.last_sent.lock().expect("party poisoned") = Some(Instant::now());
            self.publish(publish);
        }
        decision.quick
    }
}

fn listeners_of(party: &Party) -> Vec<Listener> {
    party
        .listeners
        .iter()
        .map(|listener| Listener {
            name: listener.name.clone(),
            q: listener.q,
            st: listener.st.clone(),
        })
        .collect()
}

/// The French cause of a start that failed, the sheet's own wording.
pub fn start_cause(error: &ApiError) -> String {
    match error {
        ApiError::Http { status: 503, .. } => {
            "Trop de parties sont ouvertes sur ce serveur : réessayez plus tard.".to_owned()
        }
        other => format!(
            "L'écoute partagée n'a pas pu démarrer : {}",
            other.cause_fr()
        ),
    }
}
