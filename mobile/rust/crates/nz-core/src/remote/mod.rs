//! Remote control: be driven by another device, and drive one. Owned by **MB19**
//! (the protocol is `SC5`'s `nz-remote`).
//!
//! The behaviour is `webapp/src/lib/remote/*`, `components/RemoteSheet.svelte`,
//! `RemoteHostChip.svelte`, `RemoteBanner.svelte` and `routes/RemoteClaim.svelte`,
//! PORTED — never re-invented. What that means, file by file:
//!
//! | This module | The SPA |
//! |---|---|
//! | [`wire`] | `lib/remote/wire.js` (the fast, quiet, never-retried request) and the routes `RemoteSheet`, `host.js#cutAll` and `controller.js#leaveRemote` call |
//! | [`host`] | `lib/remote/host.js` — this device's player, lent (publish, poll, run commands) |
//! | [`controller`] | `lib/remote/controller.js` — this device driving another player (the loan's view, `ack` hold, the position line) |
//! | [`settings`] | `lib/remote/settings.js` — which device settings are lent, and which never are |
//! | [`view`] | what `RemoteSheet`, `RemoteHostChip`, `RemoteBanner` and `RemoteClaim` print |
//! | [`Remote`] | `hoststate.js` + `mode.js`: the two roles, one service, the links a launch carries |
//!
//! # The two roles, and the one rule they share
//!
//! A link drives ONE device: the owner makes it on the player that PLAYS
//! (`RemoteSheet`), and whoever opens `/rc/<token>` drives that player from their
//! own phone, in the same app. So this core has two faces:
//!
//! - **lent** ([`host`]): the phone's player answers a controller — poll, publish,
//!   run what arrives through the ONE transport path;
//! - **driving** ([`controller`]): the phone draws the owner's player and sends it
//!   commands, through `nz-remote`'s outbox and its `ack` hold.
//!
//! Both are `nz-remote`'s state machines. Nothing of the protocol is copied here:
//! what this file owns is the SEAM to this app (the player, the store, the client)
//! and the one thread that steps it.
//!
//! # Rule 1 and the request budget
//!
//! - Every request of both roles is `Lane::User`, tagged [`wire::TAG_REMOTE`].
//! - **Nothing is sent while nothing is lent and nothing is driven.** The host's
//!   loop starts only when a link points here, its launch probe runs once for a
//!   device that was lent before, and it stops the moment the server says no link
//!   is left: a phone nobody drives spends **0 requests**.
//! - The service's thread is PARKED when neither role is running, and ticks at
//!   `TICK` (120 ms, the desktop's cadence) only while one is. It pushes a status
//!   only when the composed status CHANGED — never on a timer.
//! - A command is never queued across an outage (`nz-remote`'s own rule): a pause
//!   pressed while the link is down is dropped and said, never replayed.

pub mod controller;
pub mod host;
pub mod settings;
pub mod view;
mod wire;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use serde_json::json;

use crate::core::{Core, CoreError};
use crate::lists::Row;
use crate::settings::LocalSettings;

pub use controller::Transport;
/// The loan's phase, re-exported so `nz-ffi` names ONE crate (it does not depend
/// on `nz-remote` itself, and a lot may not add a dependency to a shared
/// manifest).
pub use nz_remote::controller::{Grant, Phase};
/// `off`, `all`, `one` — the wire's repeat spellings, from this machine's own.
pub fn repeat_wire(repeat: crate::player::Repeat) -> &'static str {
    nz_remote::host::repeat_wire(repeat)
}
pub use host::{HostNow, HostPlayer};
pub use view::{ControllerView, GrantView, LinkView, NowView, StatusView};
pub use wire::Totals;

/// The step of the service's thread while somebody is involved: the desktop's
/// own `TICK` (a remote tap must reach the owner inside its 700 ms poll period).
pub const TICK: Duration = Duration::from_millis(120);

/// The four levels, weakest first, and how long a link may live.
///
/// `remote.py#LEVELS` and `RemoteSheet.svelte#DURATIONS`, ported: the phone offers
/// the same four levels (the `admin` one only to an administrator) and the same
/// four durations, because a person comparing the two apps must find the same
/// words.
pub const LEVELS: [&str; 4] = ["queue", "read", "full", "admin"];

/// The durations the sheet offers, in the order it shows them.
pub const DURATIONS: [&str; 4] = ["1h", "1d", "7d", "none"];

/// Who hears about a status change. `nz-ffi`'s listener adapter hands it to a
/// Kotlin Flow at once; the core holds no lock while calling it.
pub trait StatusSink: Send + Sync {
    fn status(&self, status: &StatusView);
}

/// The process's remote-control service: both roles, one thread, one listener
/// list.
pub struct Remote {
    inner: Arc<Inner>,
}

struct Inner {
    core: Arc<Core>,
    /// The phone's player, attached once by `nz-ffi/src/remote.rs` (it is the
    /// FFI's object, so the core cannot build it).
    player: OnceLock<Arc<dyn HostPlayer>>,
    host: OnceLock<Arc<host::Host>>,
    loan: OnceLock<Arc<controller::Loan>>,
    listeners: Mutex<Vec<Arc<dyn StatusSink>>>,
    /// The last status pushed, so a repaint happens once per CHANGE.
    last: Mutex<Option<StatusView>>,
    /// The last `/api/remote/links` answer, in MEMORY only: a token is a
    /// capability and does not belong in a file this app keeps (the SPA keeps
    /// none either).
    links: Mutex<Vec<LinkView>>,
    /// The account the server handed back when a loan ended.
    restored: Mutex<Option<String>>,
    stop: AtomicBool,
    /// The thread's parking spot: an event, never a poll.
    gate: Mutex<()>,
    wake: Condvar,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

static SERVICE: OnceLock<Arc<Remote>> = OnceLock::new();

/// Bind the process's remote-control service to `core`, once.
///
/// Idempotent on purpose: the remote sheet, a claim screen and `:bridge`'s own
/// attach all reach it, and the second call must not start a second thread (two
/// steps over one player would publish twice and apply a command twice).
pub fn attach(core: &Arc<Core>) -> Arc<Remote> {
    Arc::clone(SERVICE.get_or_init(|| Remote::start(Arc::clone(core))))
}

/// The service, when [`attach`] has run.
pub fn get() -> Option<Arc<Remote>> {
    SERVICE.get().cloned()
}

/// Stop the service's thread and join it (worker rule 5: it owns no runtime, but
/// a thread left behind is still a thread). Both loops are stopped first, so
/// nothing is sent after this.
pub fn shutdown() {
    if let Some(service) = SERVICE.get() {
        service.shutdown();
    }
}

/// What the phone says it is, in the owner's list of links: the SPA's own
/// `deviceName()` wording (`lib/remote/device.js`), so both clients read alike.
pub fn device_label() -> String {
    if cfg!(target_os = "android") {
        "Android · app Neezer".to_owned()
    } else {
        nz_remote::host::device_name()
    }
}

impl Remote {
    /// Start the service over `core`. Cheap: no request, no loop — the thread
    /// parks until a role runs.
    pub fn start(core: Arc<Core>) -> Arc<Remote> {
        let inner = Arc::new(Inner {
            core,
            player: OnceLock::new(),
            host: OnceLock::new(),
            loan: OnceLock::new(),
            listeners: Mutex::new(Vec::new()),
            last: Mutex::new(None),
            links: Mutex::new(Vec::new()),
            restored: Mutex::new(None),
            stop: AtomicBool::new(false),
            gate: Mutex::new(()),
            wake: Condvar::new(),
            thread: Mutex::new(None),
        });
        let service = Arc::new(Remote {
            inner: Arc::clone(&inner),
        });
        let looped = Arc::clone(&inner);
        match std::thread::Builder::new()
            .name("nz-remote".to_owned())
            .spawn(move || run(looped))
        {
            Ok(thread) => *lock(&inner.thread) = Some(thread),
            Err(error) => tracing::warn!(%error, "le fil de télécommande n'a pas démarré"),
        }
        service
    }

    pub fn core(&self) -> &Arc<Core> {
        &self.inner.core
    }

    /// Wire the phone's player: the host side needs it to publish and to run what
    /// a controller sends. Called once, by `nz-ffi/src/remote.rs`, from
    /// `NzCore::player()`'s own object.
    pub fn attach_player(&self, player: Arc<dyn HostPlayer>) {
        let _ = self.inner.player.set(player);
        // The client is the session's own; a re-login replaces it, so the host is
        // rebuilt when the pointer moved (`ensure` reads it every time).
        self.wake();
    }

    /// The phone's player, when `nz-ffi`'s attach handed it over.
    pub fn has_player(&self) -> bool {
        self.inner.player.get().is_some()
    }

    /// The grant this session runs under, if it is a loan.
    pub fn grant(&self) -> Option<nz_remote::controller::Grant> {
        self.loan().and_then(|loan| loan.grant())
    }

    fn player(&self) -> Option<&Arc<dyn HostPlayer>> {
        self.inner.player.get()
    }

    fn host(&self) -> Option<Arc<host::Host>> {
        let api = wire::client(&self.inner.core.api().ok()?);
        let player = Arc::clone(self.player()?);
        Some(Arc::clone(self.inner.host.get_or_init(|| {
            Arc::new(host::Host::new(Arc::clone(&self.inner.core), player, api))
        })))
    }

    fn loan(&self) -> Option<Arc<controller::Loan>> {
        let api = wire::client(&self.inner.core.api().ok()?);
        Some(Arc::clone(self.inner.loan.get_or_init(|| {
            Arc::new(controller::Loan::new(Arc::clone(&self.inner.core), api))
        })))
    }

    /// Registers a listener. Every later CHANGE reaches all of them; the list is
    /// small (the shell subscribes once).
    pub fn subscribe(&self, sink: Arc<dyn StatusSink>) {
        lock(&self.inner.listeners).push(sink);
        self.wake();
    }

    pub fn wake(&self) {
        let _gate = lock(&self.inner.gate);
        self.inner.wake.notify_all();
    }

    /// Stop both roles and the thread. Idempotent.
    pub fn shutdown(&self) {
        self.inner.stop.store(true, Ordering::Release);
        if let Some(loan) = self.inner.loan.get() {
            loan.stop();
        }
        if let Some(host) = self.inner.host.get() {
            host.stop();
        }
        self.wake();
        let thread = lock(&self.inner.thread).take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }

    /// Whether either role is running — what the thread reads to decide between
    /// a 120 ms step and a park.
    fn active(&self) -> bool {
        let loan = self
            .inner
            .loan
            .get()
            .and_then(|loan| loan.controller())
            .is_some_and(|controller| controller.looping());
        let host = self.inner.host.get().is_some_and(|host| host.running());
        loan || host
    }

    // --- what the screens read ---------------------------------------------

    /// The chip and the banner, composed from both roles. A clone, cheap.
    pub fn status(&self) -> StatusView {
        let host = self.inner.host.get().map(|host| host.status());
        let mut status = match self.inner.loan.get() {
            Some(loan) => loan.status(lock(&self.inner.restored).clone()),
            None => StatusView {
                restored: lock(&self.inner.restored).clone(),
                ..StatusView::default()
            },
        };
        if let Some(host) = host {
            status.host_active = host.active;
            status.who = host.who();
            status.controllers = host.controllers.iter().cloned().map(Into::into).collect();
            status.host_level = host.level().map(str::to_owned);
            status.applied = u32::try_from(host.applied).unwrap_or(u32::MAX);
            status.host_error = host.error.clone();
            status.version ^= host.version.rotate_left(32);
        }
        if let Some(controller) = self.inner.loan.get().and_then(|loan| loan.controller()) {
            status.line = controller.status().line();
            status.version ^= controller.status().version.rotate_left(16);
        }
        status
    }

    /// The owner's player as this device drives it (the "Contrôle distant"
    /// screen). Synchronous and cheap: the position is the published line
    /// extrapolated, read per frame while a progress bar shows.
    pub fn now(&self) -> NowView {
        self.inner
            .loan
            .get()
            .map(|loan| loan.now())
            .unwrap_or_default()
    }

    /// The owner's queue, as the rows a windowed list draws. Cloned on every call
    /// — the screen asks once per queue version, not per frame.
    pub fn queue_rows(&self) -> Vec<Row> {
        self.inner
            .loan
            .get()
            .map(|loan| loan.queue())
            .unwrap_or_default()
    }

    /// The position of the owner's track, on the owner's published line — the
    /// HOT read a progress bar calls per visible frame (`controller.js#tick`
    /// does the same at its own rate), and nothing while the screen is off.
    pub fn position(&self) -> f64 {
        self.inner
            .loan
            .get()
            .map(|loan| loan.position())
            .unwrap_or(0.0)
    }

    /// What this screen's requests spent, cold and warm: requests and bytes as
    /// `nz-api` counts them under [`wire::TAG_REMOTE`] (rule 3 — counted, never
    /// timed).
    pub fn traffic(&self) -> Totals {
        match self.inner.core.api() {
            Ok(api) => wire::totals(&api),
            Err(_) => Totals::default(),
        }
    }

    /// The links this device made, last read (memory only: a token is a
    /// capability, and the SPA keeps no offline copy either).
    pub fn cached_links(&self) -> Vec<LinkView> {
        lock(&self.inner.links).clone()
    }

    /// `GET /api/remote/links?device=` — every live link onto THIS device. One
    /// request, on the `User` lane.
    pub async fn links(&self) -> Result<Vec<LinkView>, CoreError> {
        let api = wire::client(&self.inner.core.api()?);
        let device = self.device();
        let answer = wire::links(&api, Some(&device)).await?;
        let rows: Vec<LinkView> = answer
            .links
            .iter()
            .map(|link| nz_remote::host::LinkRow::from_link(&api, link))
            .map(LinkView::from)
            .collect();
        *lock(&self.inner.links) = rows.clone();
        Ok(rows)
    }

    /// This device's stable remote id (`remote.device`): the same after a reload,
    /// a restart and an update, which is what makes a link point at THIS phone.
    pub fn device(&self) -> String {
        nz_remote::host::device_id(self.inner.core.cache())
    }

    /// `POST /api/remote/links` — lend this phone's player. The link's URL is
    /// built from the server's ROOT (`applink.js#shortLink`), so a deployment
    /// under a path prefix keeps it.
    ///
    /// Starting the loops is the caller's next step ([`Remote::start_host`]), and
    /// the SPA does the same (`startHosting()` right after a create): a link that
    /// nothing answers would read "Lecteur hors ligne" for its whole life.
    pub async fn create_link(
        &self,
        level: &str,
        duration: &str,
        label: Option<&str>,
    ) -> Result<LinkView, CoreError> {
        if !LEVELS.contains(&level) {
            return Err(CoreError::Invalid {
                detail: "Niveau de contrôle inconnu".to_owned(),
            });
        }
        let me = self.inner.core.me().ok_or(CoreError::Session)?;
        if level == "admin" && !me.user.admin {
            // The server refuses it too (`remote_link_create`); refusing here
            // means the sheet cannot even offer what would be a 403.
            return Err(CoreError::Invalid {
                detail: "Seul un administrateur peut prêter l'administration".to_owned(),
            });
        }
        let ttl = ttl_for(duration).ok_or_else(|| CoreError::Invalid {
            detail: "Durée de lien inconnue".to_owned(),
        })?;
        let api = wire::client(&self.inner.core.api()?);
        let device = self.device();
        let link = wire::create(
            &api,
            level,
            &device,
            &device_label(),
            label.map(str::trim).filter(|text| !text.is_empty()),
            ttl,
        )
        .await?;
        let row = LinkView::from(nz_remote::host::LinkRow::from_link(&api, &link));
        {
            let mut links = lock(&self.inner.links);
            links.retain(|held| held.id != row.id);
            links.insert(0, row.clone());
        }
        Ok(row)
    }

    /// `DELETE /api/remote/links/<id>` — cut ONE link. The controller it let in
    /// ends on its very next request.
    pub async fn revoke_link(&self, id: &str) -> Result<(), CoreError> {
        let api = wire::client(&self.inner.core.api()?);
        wire::revoke(&api, id).await?;
        lock(&self.inner.links).retain(|held| held.id != id);
        Ok(())
    }

    /// « Tout couper » — every link onto this device, and the loops stop FIRST:
    /// nothing is sent on a link that is being cut.
    pub async fn revoke_all(&self) -> Result<(), CoreError> {
        match self.host() {
            Some(host) => host.cut_all().await?,
            None => {
                let api = wire::client(&self.inner.core.api()?);
                wire::revoke_device(&api, &self.device()).await?;
            }
        }
        lock(&self.inner.links).clear();
        self.wake();
        Ok(())
    }

    /// `POST /api/remote/claim` — this phone now drives the player `token` names.
    /// The session becomes the owner's (`/api/me` reports the loan), so the read
    /// after it is what fills the banner.
    pub async fn claim(&self, token: &str) -> Result<GrantView, CoreError> {
        let api = wire::client(&self.inner.core.api()?);
        wire::claim(&api, token).await?;
        // The loan is the session's now: read it back, so `me().remote` carries
        // the grant the banner shows (the SPA reloads its stores for the same
        // reason).
        self.inner.core.restore().await?;
        let grant =
            self.loan()
                .and_then(|loan| loan.grant())
                .ok_or_else(|| CoreError::Internal {
                    detail: "Le serveur n'a pas accordé le contrôle".to_owned(),
                })?;
        Ok(GrantView::from(&grant))
    }

    /// Wire the loan WITHOUT starting its loop: the status is painted before the
    /// first read lands, and a test drives the reads itself
    /// ([`Remote::step_loan`]).
    pub async fn open_loan(&self) -> Result<(), CoreError> {
        let loan = self.loan().ok_or_else(|| CoreError::Unsupported {
            feature: "remote:loan".to_owned(),
        })?;
        let grant = loan.grant().ok_or(CoreError::Session)?;
        loan.wire(grant, tokio::runtime::Handle::current());
        lock(&self.inner.restored).take();
        self.wake();
        Ok(())
    }

    /// Start (or resume) driving: the process's controller over this grant.
    pub async fn start_loan(&self) -> Result<(), CoreError> {
        let loan = self.loan().ok_or_else(|| CoreError::Unsupported {
            feature: "remote:loan".to_owned(),
        })?;
        let grant = loan.grant().ok_or(CoreError::Session)?;
        loan.start(grant, tokio::runtime::Handle::current());
        lock(&self.inner.restored).take();
        self.wake();
        Ok(())
    }

    /// One step of the loan, for a test that owns the events: read
    /// `/api/remote/state`, adopt, follow the line, push the status.
    pub async fn step_loan(&self) -> Result<(), CoreError> {
        let loan = self.loan().ok_or_else(|| CoreError::Unsupported {
            feature: "remote:loan".to_owned(),
        })?;
        match loan.read_once().await {
            nz_remote::controller::ReadStep::Read => Ok(()),
            nz_remote::controller::ReadStep::Failed => Err(CoreError::Offline),
            nz_remote::controller::ReadStep::Ended(error) => Err(error.into()),
        }
    }

    /// `POST /api/remote/leave` — hand the loan back. Returns the account the
    /// server restored, when it names one.
    pub async fn leave(&self) -> Result<Option<String>, CoreError> {
        let loan = self.loan().ok_or_else(|| CoreError::Unsupported {
            feature: "remote:loan".to_owned(),
        })?;
        let restored = loan.leave().await?;
        self.inner.core.restore().await.ok();
        *lock(&self.inner.restored) = restored.clone();
        self.wake();
        Ok(restored)
    }

    /// One transport command for the owner's player: optimistic (the view changes
    /// now), then POSTed. A level that refuses it changes NOTHING.
    pub async fn send(&self, transport: Transport) -> Result<(), CoreError> {
        let loan = self.loan().ok_or_else(|| CoreError::Unsupported {
            feature: "remote:loan".to_owned(),
        })?;
        loan.send(transport)?;
        loan.flush().await;
        self.wake();
        Ok(())
    }

    /// Start answering a controller: called right after a link is made, and again
    /// at a launch for a device that was lent before.
    pub async fn start_host(&self) -> Result<(), CoreError> {
        let host = self.host().ok_or_else(|| CoreError::Unsupported {
            feature: "remote:host".to_owned(),
        })?;
        host.start(tokio::runtime::Handle::current());
        self.wake();
        Ok(())
    }

    /// Ask the server once whether a link still points here, and answer again if
    /// one does. A device that was never lent asks NOTHING.
    pub async fn resume_host(&self) -> Result<bool, CoreError> {
        let host = self.host().ok_or_else(|| CoreError::Unsupported {
            feature: "remote:host".to_owned(),
        })?;
        if !host.hosted_before() {
            return Ok(false);
        }
        host.resume(tokio::runtime::Handle::current());
        self.wake();
        Ok(true)
    }

    /// Stop answering (a logout, the person's own switch).
    pub fn stop_host(&self) {
        if let Some(host) = self.inner.host.get() {
            host.stop();
        }
        self.wake();
    }

    /// Whether this phone was ever lent — the key that decides whether a launch
    /// probes at all.
    /// Whether the host loops are running right now (the chip says so, and a
    /// test waits on it going false after a cut).
    pub fn host_running(&self) -> bool {
        self.inner.host.get().is_some_and(|host| host.running())
    }

    pub fn hosted_before(&self) -> bool {
        self.inner
            .host
            .get()
            .is_some_and(|host| host.hosted_before())
    }

    /// The levels the sheet offers: the three a person may lend, plus `admin` to
    /// an administrator (`RemoteSheet.svelte#levels`).
    pub fn levels(&self) -> Vec<LevelView> {
        let admin = self.inner.core.me().is_some_and(|me| me.user.admin);
        LEVELS
            .iter()
            .filter(|level| **level != "admin" || admin)
            .map(|level| LevelView {
                id: (*level).to_owned(),
                label: nz_remote::host::level_label(level).to_owned(),
                blurb: nz_remote::host::level_blurb(level).to_owned(),
                short: short_label(level).to_owned(),
            })
            .collect()
    }

    /// The settings a controller may see and change, as this device holds them.
    pub fn settings_snapshot(&self) -> serde_json::Value {
        settings::snapshot(&self.inner.core)
    }

    /// The local settings the loan's screen shows, resolved (it is a controller's
    /// own Réglages: `mode.js` keeps them under a `remote:` namespace, which on
    /// the phone is the same `kv` — this device's values are never rewritten).
    pub fn local_settings(&self) -> LocalSettings {
        LocalSettings::read(&self.inner.core)
    }

    /// The thread's body: step while a role runs, park when neither does.
    fn run_tick(&self) {
        let mut dirty = false;
        if let Some(host) = self.inner.host.get() {
            host.tick();
        }
        if let Some(loan) = self.inner.loan.get() {
            // A loan that ended on a `401` says so once, and names the account
            // the server handed back (one `GET /api/me`, on the event, never on a
            // timer).
            if loan.tick() {
                dirty = true;
            }
            let ended = loan.controller().is_some_and(|controller| {
                controller.status().phase() == nz_remote::controller::Phase::Ended
            });
            if ended && lock(&self.inner.restored).is_none() {
                *lock(&self.inner.restored) = Some(String::new());
                dirty = true;
            }
        }
        let status = self.status();
        let changed = dirty
            || lock(&self.inner.last)
                .as_ref()
                .is_none_or(|last| *last != status);
        if changed {
            *lock(&self.inner.last) = Some(status.clone());
            for sink in lock(&self.inner.listeners).clone() {
                sink.status(&status);
            }
        }
    }
}

/// One level, as the sheet's radio buttons print it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LevelView {
    /// `queue`, `read`, `full`, `admin` — what `create_link` takes.
    pub id: String,
    /// `File d'attente`, `Lecture seule`, `Complet`, `Administrateur`.
    pub label: String,
    /// The SPA's own one-line blurb (`LEVEL_INFO[l].blurb`).
    pub blurb: String,
    /// The two-or-three-word form the chip and the banner use.
    pub short: String,
}

/// `LEVEL_INFO[l].short` (`mode.js`).
fn short_label(level: &str) -> &'static str {
    match level {
        "queue" => "File",
        "read" => "Lecture seule",
        "full" => "Complet",
        "admin" => "Admin",
        _ => "",
    }
}

/// `RemoteSheet.svelte#DURATIONS`: an id from [`DURATIONS`] to a TTL in seconds,
/// `None` for unlimited. `None` also for an id this build does not know, which
/// the caller turns into a French cause.
pub fn ttl_for(duration: &str) -> Option<Option<u32>> {
    match duration {
        "1h" => Some(Some(3600)),
        "1d" => Some(Some(86_400)),
        "7d" => Some(Some(7 * 86_400)),
        "none" => Some(None),
        _ => None,
    }
}

/// The thread: park until a role runs, step at [`TICK`] while one does, push a
/// status once per change.
fn run(inner: Arc<Inner>) {
    loop {
        if inner.stop.load(Ordering::Acquire) {
            break;
        }
        let service = Remote {
            inner: Arc::clone(&inner),
        };
        let active = service.active();
        if active {
            service.run_tick();
        }
        // Park without a timeout when nothing moves; wait at most `TICK` while a
        // role runs. A notification (a start, a stop, a new listener) cuts the
        // wait short, so a command never waits for the next step.
        let guard = lock(&inner.gate);
        if inner.stop.load(Ordering::Acquire) {
            break;
        }
        if active {
            let _ = inner.wake.wait_timeout(guard, TICK);
        } else {
            let _parked = inner
                .wake
                .wait_while(guard, |_| {
                    !inner.stop.load(Ordering::Acquire) && !service.active()
                })
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// The JSON body of a `set` command, as the controller's own Réglages sends it —
/// kept here so a test can drive the host without a controller.
pub fn setting_command(key: &str, value: serde_json::Value) -> serde_json::Value {
    json!({"op": "set", "args": {"key": key, "value": value}})
}
