//! Écoute partagée — the PHONE's half: host the party from the player that is
//! already playing, join one by its link, and keep the guest in the room.
//!
//! Owned by **MB20** (docs/agents/mobile.md, "Module and file ownership"): the
//! behaviour is the SPA's (`webapp/src/lib/party/`, `routes/Party.svelte`,
//! `components/PartySheet.svelte`), ported and never re-invented, and the
//! protocol, the line, the correction rule, the planner and the two loops are
//! **`nz-party`'s** (SC6, extracted from desktop D15b). This module starts that
//! crate over the phone's own pieces and turns what it publishes into the values
//! two screens draw.
//!
//! Five rules shape it, each one the note's or the SPA's:
//!
//! - **The host's player is only WATCHED.** [`host`] reads four hot values a
//!   quarter of a second and hands them to `nz-party`'s fit; nothing here seeks,
//!   pauses or loads. The person keeps the hand (`host.js`).
//! - **The guest plays through the PLAYER's engine**, never a second one:
//!   `Player::party()` is `nz_audio::PartyChannel`, so a chunk is scheduled on the
//!   output clock the engine reports, goes through the person's DSP chain and
//!   output, and `take_over` leaves their queue in place. "The player is the
//!   singleton" is CLAUDE.md's rule, and a second engine would be a second output.
//! - **Rule 1: the sync traffic never costs the music a frame.** The clock
//!   probes, the state polls and the chunks are all `Lane::User` — the lane that
//!   never waits — through `nz-api`'s gates (tagged [`TAG`]), coalesced by
//!   `nz-party` to one poll and two chunk requests. The pulse thread reads
//!   atomics and never touches the network.
//! - **Rule 3: a flaky link fails fast and says why.** A poll that fails keeps
//!   the line in hand (the music carries on, extrapolated) and after six seconds
//!   the screen says « Connexion perdue — la musique continue »; a peek that
//!   fails with a copy on disk shows the copy, and without one a `Failed` phase
//!   with the cause and « Réessayer » — never a spinner past the first failure.
//! - **Rule 7: an older server hides the door.** [`Party::supported`] asks the
//!   route's existence with one `OPTIONS` (the `nz-api` feature probe), so a
//!   server that predates the party never shows a button that cannot work.
//!
//! ## Attaching (why an FFI object is not enough)
//!
//! `NzCore::party()` answers an `Arc<PartyApi>` that MB0 declared as a unit
//! struct, so it cannot carry `Arc<Core>` the way `PlayerApi::open(core)` does.
//! The Kotlin side therefore calls `PartyApi::attach(core, core.player())` once
//! (`:feature:party`'s `PartyCore`), which binds this module's ONE service for the
//! process — the same seam MB15's offline module uses. Every FFI method answers
//! `NzError::Unsupported { feature: "party:attach" }` before that.
//!
//! ## Threads
//!
//! One pulse thread ([`Party::pulse`]) owns the 250/40 ms host measurement and
//! the change notification; `nz-party` owns the guest's scheduler thread and
//! stops it in `Guest::leave`. The pulse thread borrows no tokio runtime, and
//! [`Party::shutdown`] joins it, stops the guest and the host, then drops the
//! runtime this module created LAST and from a plain thread — a `Runtime` dropped
//! inside an async context panics (worker rule 5).

mod guest;
mod host;
mod store;
#[cfg(test)]
mod tests;
mod views;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use nz_api::Api;
use nz_api::features::Feature;
use nz_party::{Guest, Host};
use tracing::{debug, info};

use crate::core::{Core, CoreError};
use crate::player::Player;

pub use store::Prefs;
pub use views::{Member, PartyView, Phase, Role, TrackView};

/// What this module's requests are counted under (`nz-api`'s per-tag totals):
/// the acceptance's own number, read by `NzCore::stats()`.
pub const TAG: &str = "party";

/// How often the pulse looks when nothing is happening: a screen that is not on
/// a party costs one comparison a second.
const IDLE_PULSE: Duration = Duration::from_millis(1_000);

/// Who hears about the party. `nz-ffi` implements this over `PartyListener`; the
/// pulse thread holds no lock while calling it, and the Kotlin side hands the
/// value to a Flow and returns at once.
pub trait PartySink: Send + Sync {
    /// One painted change (never the playhead, which a screen reads hot).
    fn changed(&self, view: &PartyView);
}

struct Inner {
    core: Arc<Core>,
    /// The ONE player of the process, when the FFI has built it.
    player: Mutex<Option<Arc<Player>>>,
    /// This module's own runtime: the probes, the polls and the chunk fetches.
    runtime: OnceLock<Arc<tokio::runtime::Runtime>>,
    host: Mutex<Option<Host>>,
    guest: Mutex<Option<Guest>>,
    /// What a screen paints: a clone under a mutex, with a version per change.
    view: Mutex<PartyView>,
    listeners: Mutex<Vec<Arc<dyn PartySink>>>,
    stop: Arc<AtomicBool>,
    version: AtomicU64,
    pulse: Mutex<Option<std::thread::JoinHandle<()>>>,
    attached: AtomicBool,
    /// The listener's three values, kept in memory so a slider never reads SQLite
    /// on a frame (the store is still the truth after a restart).
    prefs: Mutex<Prefs>,
    /// The last phase reached, where a `Failed`/`Missing` landing report starts
    /// from (there is no `nz-party` view for a screen that never joined).
    phase: Mutex<Phase>,
    /// The dead-link or failure sentence, kept while no `nz-party` view exists.
    error: Mutex<Option<String>>,
}

static SERVICE: OnceLock<Arc<Party>> = OnceLock::new();

/// The process's party service.
#[derive(Clone)]
pub struct Party {
    inner: Arc<Inner>,
}

/// Bind the process's party service to `core` and (when it exists) its player,
/// once. Idempotent: `:bridge` and the feature module both reach it, and a second
/// call must neither start a second pulse thread nor a second guest.
pub fn attach(core: &Arc<Core>, player: Option<Arc<Player>>) -> Arc<Party> {
    let service = SERVICE.get_or_init(|| {
        let inner = Arc::new(Inner {
            core: Arc::clone(core),
            player: Mutex::new(None),
            runtime: OnceLock::new(),
            host: Mutex::new(None),
            guest: Mutex::new(None),
            view: Mutex::new(PartyView::default()),
            listeners: Mutex::new(Vec::new()),
            stop: Arc::new(AtomicBool::new(false)),
            version: AtomicU64::new(1),
            pulse: Mutex::new(None),
            attached: AtomicBool::new(false),
            prefs: Mutex::new(store::load(core.cache())),
            phase: Mutex::new(Phase::Idle),
            error: Mutex::new(None),
        });
        if let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("nz-party-net")
            .enable_all()
            .build()
        {
            let _ = inner.runtime.set(Arc::new(runtime));
        }
        let party = Arc::new(Party { inner });
        party.start_pulse();
        party
    });
    // A later `core.player()` hands the player over: the phone builds its player
    // lazily, and the party must not refuse for ever because it arrived second.
    if let Some(player) = player {
        *lock(&service.inner.player) = Some(player);
    }
    service.inner.attached.store(true, Ordering::Release);
    Arc::clone(service)
}

/// The service, when [`attach`] has run.
pub fn get() -> Option<Arc<Party>> {
    SERVICE.get().cloned()
}

/// Stop the pulse thread, the guest and the host, then drop the runtime (worker
/// rule 5). Called from the app's own death path and by the tests.
pub fn shutdown() {
    if let Some(service) = SERVICE.get() {
        service.shutdown();
    }
}

/// A poisoned lock is a bug in another thread, not a reason to leave a screen
/// empty: every lock here is a small value with no invariant across calls.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Party {
    /// Whether this server has the party routes at all (rule 7): one `OPTIONS` on
    /// the `Prefetch` lane, and a doubt (`None`) never hides the door.
    pub async fn supported(&self) -> Option<bool> {
        let api = self.api().ok()?;
        api.has_route(Feature::Party.probe()).await
    }

    /// The party as a screen paints it. Never blocks on the network; the playhead
    /// is a hot read.
    pub fn view(&self) -> PartyView {
        self.compose(true)
    }

    /// The party's playhead, seconds into the track: the guest's own line
    /// extrapolation, or the host player's atomic. Cheap enough for a frame, and
    /// never a request.
    pub fn position_s(&self) -> f64 {
        if let Some(guest) = self.guest() {
            return guest.position_now().max(0.0);
        }
        self.player()
            .map(|player| player.position().position_s.max(0.0))
            .unwrap_or(0.0)
    }

    /// The members of the party: the host's guest list. A guest's public state
    /// names people without their ids, and its screen draws a count.
    pub fn members(&self) -> Vec<Member> {
        self.compose(false).members
    }

    pub fn state_version(&self) -> u64 {
        self.inner.version.load(Ordering::Acquire)
    }

    pub fn subscribe(&self, sink: Arc<dyn PartySink>) {
        lock(&self.inner.listeners).push(sink);
    }

    /// The three values the listener owns.
    pub fn prefs(&self) -> Prefs {
        lock(&self.inner.prefs).clone()
    }

    pub fn set_name(&self, name: &str) {
        let trimmed: String = name.trim().chars().take(32).collect();
        store::save_name(self.inner.core.cache(), &trimmed);
        lock(&self.inner.prefs).name = trimmed;
    }

    /// The party's own volume, the SPA's `session.setVolume`.
    pub fn set_volume(&self, volume: f64) {
        let volume = volume.clamp(0.0, 1.0);
        if let Some(guest) = self.guest() {
            guest.set_volume(volume);
        }
        store::save_volume(self.inner.core.cache(), volume);
        lock(&self.inner.prefs).volume = volume;
        self.publish_now();
    }

    /// This device's own output delay: positive plays earlier, so the room lines
    /// up. The screen applies a run of taps once (`Party.svelte#nudge`).
    pub fn set_trim(&self, ms: f64) {
        let ms = ms.clamp(-500.0, 1000.0).round();
        if let Some(guest) = self.guest() {
            guest.set_trim(ms);
        }
        store::save_trim(self.inner.core.cache(), ms);
        lock(&self.inner.prefs).trim_ms = ms;
        self.publish_now();
    }

    /// `POST /api/party` — start (or re-open) the party this person hosts, and
    /// publish its first line. Returns the link guests open.
    pub async fn host(&self) -> Result<String, CoreError> {
        let api = self.api()?;
        let player = self.player().ok_or_else(|| CoreError::Internal {
            detail: "le lecteur n'a pas démarré : la soirée ne peut pas être animée".to_owned(),
        })?;
        // A guest session and a hosted party are exclusive on one device: the
        // guest's audio would be taken over by the person's own queue.
        if self.guest().is_some() {
            self.leave_guest();
        }
        let server = self
            .inner
            .core
            .origin()
            .map(|origin| origin.as_str())
            .ok_or(CoreError::Invalid {
                detail: "aucun serveur configuré".to_owned(),
            })?;
        let nz_host = Host::attach(
            Arc::clone(&api),
            self.inner.core.cache().clone(),
            self.runtime()?.handle().clone(),
            server,
        );
        let party = api.party_start(None).await.map_err(CoreError::from)?;
        let link = nz_host.begin(&party);
        *lock(&self.inner.host) = Some(nz_host);
        // The first line goes out at once: a guest's landing card then names the
        // track the person is already hearing.
        self.tick_host(&player);
        lock(&self.inner.phase).clone_from(&Phase::Live);
        lock(&self.inner.error).take();
        info!(%link, "écoute partagée : ce téléphone anime");
        self.publish_now();
        Ok(link)
    }

    /// Re-open the party this person was hosting before the app was killed, when
    /// there is one — ONE request, and only for somebody who hosted before
    /// (`Host::should_ask`'s rule, ported).
    pub async fn resume(&self) -> Result<Option<String>, CoreError> {
        if self.host_running() {
            return Ok(self.view().role.host_link());
        }
        let saved = Host::saved(self.inner.core.cache());
        let host = match self.nz_host() {
            Some(host) if host.should_ask() => host,
            _ if saved.is_some() => {
                let api = self.api()?;
                let server = self
                    .inner
                    .core
                    .origin()
                    .map(|origin| origin.as_str())
                    .unwrap_or_default();
                let host = Host::attach(
                    Arc::clone(&api),
                    self.inner.core.cache().clone(),
                    self.runtime()?.handle().clone(),
                    server,
                );
                *lock(&self.inner.host) = Some(host.clone());
                host
            }
            // A feature nobody used costs a launch no request at all.
            _ => return Ok(None),
        };
        let _ = host;
        self.ask_mine().await
    }

    async fn ask_mine(&self) -> Result<Option<String>, CoreError> {
        let api = self.api()?;
        let mine = api.party_mine().await.map_err(CoreError::from)?;
        let Some(party) = mine.party else {
            return Ok(None);
        };
        let Some(nz_host) = self.nz_host() else {
            return Ok(None);
        };
        let link = nz_host.begin(&party);
        if let Some(player) = self.player() {
            self.tick_host(&player);
        }
        lock(&self.inner.phase).clone_from(&Phase::Live);
        self.publish_now();
        info!(%link, "écoute partagée : la soirée de ce téléphone a repris");
        Ok(Some(link))
    }

    /// `GET /api/party/<id>` without joining: the landing card. `None` when the
    /// party does not exist (ended, or never did) — not an error. The answer is
    /// kept on disk, which is what makes a re-open free.
    pub async fn peek(&self, id: &str) -> Result<Option<PartyView>, CoreError> {
        let api = self.api()?;
        let state = guest::peek(&api, id).await?;
        match state {
            Some(state) => {
                store::save_peek(self.inner.core.cache(), &state);
                lock(&self.inner.phase).clone_from(&Phase::Landing);
                lock(&self.inner.error).take();
                Ok(Some(self.peek_view(&state)))
            }
            None => {
                lock(&self.inner.phase).clone_from(&Phase::Missing);
                self.publish_now();
                Ok(None)
            }
        }
    }

    /// The card's copy, disk only: what a warm open paints before the peek
    /// answers (rule 3).
    pub fn cached_peek(&self, id: &str) -> Option<PartyView> {
        store::cached_peek(self.inner.core.cache(), id).map(|state| self.peek_view(&state))
    }

    /// `POST /api/party/<id>/join` and the guest session: the clock, the poll
    /// loop and the scheduler, in `nz-party`.
    pub async fn join(&self, id: &str, name: &str) -> Result<PartyView, CoreError> {
        let api = self.api()?;
        let player = self.player().ok_or_else(|| CoreError::Internal {
            detail: "le lecteur n'a pas démarré : le son de la soirée ne peut pas jouer".to_owned(),
        })?;
        let name: String = name.trim().chars().take(32).collect();
        let name = if name.is_empty() {
            self.prefs().display_name()
        } else {
            name
        };
        store::save_name(self.inner.core.cache(), &name);
        lock(&self.inner.prefs).name = name.clone();
        // A party this device hosts cannot also be listened to (one party per
        // device, the SPA's own rule).
        if self.host_running() {
            self.end_host().await;
        }
        let Some(state) = guest::peek(&api, id).await? else {
            lock(&self.inner.phase).clone_from(&Phase::Missing);
            self.publish_now();
            return Err(CoreError::Invalid {
                detail: "Cette party n'existe plus".to_owned(),
            });
        };
        store::save_peek(self.inner.core.cache(), &state);
        match guest::take_seat(&api, id, &name).await {
            Ok(lid) => {
                let prefs = self.prefs();
                let runtime = self.runtime()?.handle().clone();
                let rate = player.output_stats().sample_rate.max(1);
                let guest = Guest::start(
                    api,
                    runtime,
                    id.to_owned(),
                    name,
                    (!lid.is_empty()).then_some(lid),
                    Some(player.party()),
                    rate,
                    prefs.volume,
                    prefs.trim_ms,
                );
                *lock(&self.inner.guest) = Some(guest);
                lock(&self.inner.phase).clone_from(&Phase::Joining);
                lock(&self.inner.error).take();
                self.publish_now();
                Ok(self.view())
            }
            Err(error) => {
                let cause = guest::join_cause(&error);
                lock(&self.inner.phase).clone_from(&Phase::Failed);
                *lock(&self.inner.error) = Some(cause);
                self.publish_now();
                Err(error.into())
            }
        }
    }

    /// Leave the party (a guest) or end it (a host), and tell the server.
    pub async fn leave(&self) -> Result<(), CoreError> {
        if self.host_running() {
            self.end_host().await;
            return Ok(());
        }
        self.leave_guest();
        self.publish_now();
        Ok(())
    }

    /// Stop every loop without a request (a logout, a session that ended).
    pub fn stop(&self) {
        self.leave_guest();
        self.stop_host();
        lock(&self.inner.phase).clone_from(&Phase::Idle);
        self.publish_now();
    }

    /// Joins the pulse thread and stops everything, then drops the runtime from a
    /// plain thread (a `Runtime` dropped inside an async context panics).
    pub fn shutdown(&self) {
        self.inner.stop.store(true, Ordering::Release);
        self.leave_guest();
        self.stop_host();
        self.join_pulse();
        lock(&self.inner.listeners).clear();
        let runtime = self.inner.runtime.get().cloned();
        if let Some(runtime) = runtime {
            // The guest's clock tasks and poll loop live on it: `leave_guest`
            // has already told them to stop, and `Runtime::drop` waits for them.
            let _ = std::thread::Builder::new()
                .name("nz-party-drop".into())
                .spawn(move || drop(runtime))
                .map(|thread| thread.join());
        }
    }

    // --- internals -----------------------------------------------------------

    fn api(&self) -> Result<Arc<Api>, CoreError> {
        let api = self.inner.core.api()?;
        Ok(Arc::new(api.with_tag(TAG)))
    }

    fn runtime(&self) -> Result<&Arc<tokio::runtime::Runtime>, CoreError> {
        self.inner.runtime.get().ok_or_else(|| CoreError::Internal {
            detail: "le runtime de l'écoute partagée n'a pas démarré".to_owned(),
        })
    }

    fn player(&self) -> Option<Arc<Player>> {
        lock(&self.inner.player).clone()
    }

    fn guest(&self) -> Option<Guest> {
        lock(&self.inner.guest).clone()
    }

    fn nz_host(&self) -> Option<Host> {
        lock(&self.inner.host).clone()
    }

    fn host_running(&self) -> bool {
        self.nz_host().is_some_and(|host| host.running())
    }

    /// One host measurement: the numbers the player publishes, handed to
    /// `nz-party`'s fit. Returns whether the line is still young (its 40 ms
    /// cadence).
    fn tick_host(&self, player: &Arc<Player>) -> bool {
        let Some(host) = self.nz_host() else {
            return false;
        };
        let gaps = host.gaps();
        let snapshot = host::snapshot(player, &player.party(), &gaps);
        host.tick(snapshot.as_ref())
    }

    fn leave_guest(&self) {
        if let Some(guest) = lock(&self.inner.guest).take() {
            guest.leave();
            lock(&self.inner.phase).clone_from(&Phase::Left);
        }
    }

    async fn end_host(&self) {
        // The loops stop BEFORE the request, so nothing is published on a party
        // being closed (`nz-party`'s own order).
        let Some(host) = lock(&self.inner.host).take() else {
            return;
        };
        let id = host.close();
        if !id.is_empty()
            && let Ok(api) = self.api()
            && let Err(error) = api.party_end(&id).await
        {
            debug!(%error, "fin de l'écoute partagée");
        }
        lock(&self.inner.phase).clone_from(&Phase::Idle);
        self.publish_now();
    }

    fn stop_host(&self) {
        if let Some(host) = lock(&self.inner.host).take() {
            host.shutdown();
        }
    }

    /// The pulse: while hosting, one measurement every 250 ms (40 while the line
    /// is young); otherwise the guest's own threads change their view and this
    /// only notices. Every change is pushed once, and the playhead is never
    /// pushed at all — a screen reads it hot (`position_s`).
    fn pulse(&self) {
        while !self.inner.stop.load(Ordering::Acquire) {
            let mut period = IDLE_PULSE;
            if self.host_running() {
                let quick = self
                    .player()
                    .map(|player| self.tick_host(&player))
                    .unwrap_or(false);
                period = Duration::from_millis(if quick {
                    host::QUICK_MS
                } else {
                    host::MEASURE_MS
                });
            }
            self.publish_changed();
            std::thread::sleep(period);
        }
    }

    fn start_pulse(&self) {
        let inner = Arc::clone(&self.inner);
        let handle = std::thread::Builder::new()
            .name("nz-party-pulse".into())
            .stack_size(256 * 1024)
            .spawn(move || Party { inner }.pulse());
        if let Ok(handle) = handle {
            *lock(&self.inner.pulse) = Some(handle);
        }
    }

    fn join_pulse(&self) {
        if let Some(handle) = lock(&self.inner.pulse).take() {
            let _ = handle.join();
        }
    }

    /// Compose the view and push it to the listeners when it CHANGED.
    fn publish_changed(&self) {
        let view = self.compose(false);
        let changed = {
            let mut current = lock(&self.inner.view);
            let mut next = view.clone();
            next.version = current.version;
            if *current == next {
                false
            } else {
                *current = next;
                true
            }
        };
        if !changed {
            return;
        }
        let version = self.inner.version.fetch_add(1, Ordering::AcqRel) + 1;
        let view = {
            let mut current = lock(&self.inner.view);
            current.version = version;
            current.clone()
        };
        let listeners: Vec<Arc<dyn PartySink>> = lock(&self.inner.listeners).clone();
        for listener in listeners {
            listener.changed(&view);
        }
    }

    /// A change the caller KNOWS happened (a tap, a departure): compose at once
    /// and push it even if the pulse is asleep.
    fn publish_now(&self) {
        let view = self.compose(false);
        {
            let mut current = lock(&self.inner.view);
            let mut next = view;
            next.version = current.version;
            *current = next;
        }
        let version = self.inner.version.fetch_add(1, Ordering::AcqRel) + 1;
        let view = {
            let mut current = lock(&self.inner.view);
            current.version = version;
            current.clone()
        };
        let listeners: Vec<Arc<dyn PartySink>> = lock(&self.inner.listeners).clone();
        for listener in listeners {
            listener.changed(&view);
        }
    }

    /// The party, from whichever side this device is on.
    fn compose(&self, hot: bool) -> PartyView {
        if let Some(host) = self.nz_host().filter(|host| host.running()) {
            return self.compose_host(&host, hot);
        }
        if let Some(guest) = self.guest() {
            return self.compose_guest(&guest, hot);
        }
        let phase = *lock(&self.inner.phase);
        let prefs = self.prefs();
        PartyView {
            phase,
            error: lock(&self.inner.error).clone(),
            volume: prefs.volume,
            trim_ms: prefs.trim_ms,
            version: self.state_version(),
            ..PartyView::default()
        }
    }

    fn compose_host(&self, host: &Host, hot: bool) -> PartyView {
        let status = host.status();
        let members: Vec<Member> = status.listeners.iter().map(Member::of_host).collect();
        let player = self.player();
        let track = player.as_ref().and_then(|player| {
            player
                .snapshot()
                .current
                .as_ref()
                .map(|now| TrackView::of(&host::track_of(now)))
        });
        let position_s = if hot { self.position_s() } else { 0.0 };
        let duration_s = track.as_ref().map(|track| track.duration_s).unwrap_or(0.0);
        PartyView {
            role: Role::Host {
                id: status.id.clone(),
                link: status.link.clone(),
                active: status.active,
                lost: status.lost,
            },
            phase: Phase::Live,
            host: self
                .inner
                .core
                .me()
                .map(|me| me.user.name)
                .unwrap_or_default(),
            track,
            live: status.active,
            playing: player
                .as_ref()
                .map(|player| player.position().playing)
                .unwrap_or(false),
            position_s,
            duration_s,
            status_text: host_clock_line(&status.clock),
            clock: status.clock,
            members,
            listeners: status.listeners.len() as u32,
            link: status.link,
            volume: self.prefs().volume,
            trim_ms: self.prefs().trim_ms,
            error: status.error,
            version: status.version,
            ..PartyView::default()
        }
    }

    fn compose_guest(&self, guest: &Guest, hot: bool) -> PartyView {
        let view = guest.view();
        let (tone, status_text) = view.status_line();
        let phase = match view.phase {
            nz_party::Phase::Idle => Phase::Landing,
            nz_party::Phase::Joining => Phase::Joining,
            nz_party::Phase::Live => Phase::Live,
            nz_party::Phase::Ended => Phase::Ended,
            nz_party::Phase::Left => Phase::Left,
        };
        *lock(&self.inner.phase) = phase;
        PartyView {
            role: Role::Guest {
                id: guest.pid(),
                host_name: view.host.clone(),
            },
            phase,
            host: view.host.clone(),
            track: view.track.as_ref().map(TrackView::of),
            next: view.next.as_ref().map(TrackView::of),
            live: view.live,
            playing: view.playing,
            loading: view.loading,
            position_s: if hot {
                guest.position_now().max(0.0)
            } else {
                view.position_s
            },
            duration_s: view.duration_s,
            status_ok: matches!(tone, nz_party::Tone::Ok),
            status_warn: matches!(tone, nz_party::Tone::Warn),
            status_text,
            clock: view.clock,
            offline: view.offline,
            // The public state names the guests, not their ids (a listener id
            // authorises its own `leave`): the count is what a guest screen draws.
            listeners: view.listeners as u32,
            members: Vec::new(),
            volume: view.volume,
            trim_ms: view.trim_ms,
            os_latency_ms: view.os_latency_ms,
            error: view.error,
            version: view.version,
            link: String::new(),
        }
    }

    /// The landing card, from one `GET /api/party/<id>` answer.
    fn peek_view(&self, state: &nz_model::PartyState) -> PartyView {
        let prefs = self.prefs();
        PartyView {
            role: Role::Guest {
                id: state.id.clone(),
                host_name: state.host.clone(),
            },
            phase: *lock(&self.inner.phase),
            host: state.host.clone(),
            track: state.track.as_ref().map(TrackView::of),
            next: state
                .next
                .as_ref()
                .and_then(|next| next.track.as_ref())
                .map(TrackView::of),
            live: state.live,
            playing: state.playing,
            loading: state.buf,
            position_s: state.anchor.as_ref().map(|anchor| anchor.p).unwrap_or(0.0),
            duration_s: state
                .track
                .as_ref()
                .map(|track| track.duration)
                .unwrap_or(0.0),
            // The count is the guests the card will join (`_public_state`'s list).
            listeners: state.listeners.len() as u32,
            volume: prefs.volume,
            trim_ms: prefs.trim_ms,
            error: lock(&self.inner.error).clone(),
            version: self.state_version(),
            link: String::new(),
            ..PartyView::default()
        }
    }
}

/// `« Horloge de cet appareil calée à ±0.4 ms · aller-retour 8 ms »`
/// (`PartySheet.svelte`'s clock line, to one decimal and no unit rounding).
fn host_clock_line(clock: &Option<(f64, f64)>) -> String {
    match clock {
        Some((spread, rtt)) => format!(
            "Horloge de cet appareil calée à ±{:.1} ms · aller-retour {:.0} ms",
            spread.max(0.1),
            rtt.max(0.0)
        ),
        None => "Synchronisation de l'horloge…".to_owned(),
    }
}
