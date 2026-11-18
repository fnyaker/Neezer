//! THIS device driving somebody else's player: the controlling side.
//!
//! The protocol is `nz_remote::controller`'s (`SC5`, extracted from the desktop's
//! `wiring/remote_controller.rs`): the read of `/api/remote/state` (the position
//! line, the `ack` hold that keeps a tap from flickering back, the queue read
//! once per `qv`), the outbox and its rule "a command is never carried over an
//! outage", the grant and its levels, the reconnect loop, and the token a pasted
//! link yields. What lives here is the half that knows this app:
//!
//! - [`Loan`], the local VIEW of the owner's player — `nz_player`'s state
//!   machine, whose effects are DROPPED (`mode.js`: a controller sounds nothing,
//!   reports nothing and stores nothing of the owner's). It is what the "Contrôle
//!   distant" screen draws, next to the owner's queue;
//! - [`Loan::send`], one transport command from that screen: applied locally at
//!   once (a tap answers in the same frame), then POSTed; a level that refuses it
//!   changes NOTHING and returns the sentence the toast prints;
//! - [`Loan::tick`]: what the owner said (adopted), the playhead along the
//!   published line, and the owner's mirrored settings — the changed ones only.
//!
//! # Why a screen rather than rebinding the whole app
//!
//! The SPA rebinds its player store (`controller.js#rebindPlayer`), so every
//! screen drives the owner's player. The desktop deliberately does not, for a
//! reason that holds on the phone too: the shell's player goes through ONE action
//! path (`PlaybackConnection`, the Media3 session, the notification) that a lot
//! may not replace, and a loan that leaves the phone's own queue playing in the
//! background must not be reachable by a stray tap. The loan's transport
//! therefore lives in its own screen — the desktop's `Contrôle distant`, the
//! phone's `Routes.RemoteClaim` — with the owner's queue under it, and the
//! banner says whose player it drives everywhere else (party-remote.md).

use std::sync::{Arc, Mutex, MutexGuard};

use nz_api::Api;
use nz_player::{Command as WireCommand, PlayerState};
use nz_remote::controller::{self, Controller, Grant, Phase, Sent};

use crate::core::{Core, CoreError};
use crate::lists::Row;

use super::view::{GrantView, NowView, StatusView};

/// How long a lease on the view is held: nothing waits inside these calls, so a
/// poisoned lock is a bug, not a state a screen must handle.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// The controller side of the service.
pub(crate) struct Loan {
    core: Arc<Core>,
    api: Arc<Api>,
    /// The owner's player as this device sees it. Replaced wholesale by every
    /// read (`nz_remote::controller::adopt`), never mutated in place.
    view: Mutex<PlayerState>,
    /// The queue `/remote/queue` last answered, and the version it belongs to.
    queue: Mutex<(String, u64, Vec<Row>)>,
}

impl Loan {
    pub(crate) fn new(core: Arc<Core>, api: Arc<Api>) -> Loan {
        Loan {
            core,
            api,
            view: Mutex::new(PlayerState::new()),
            queue: Mutex::new((String::new(), 0, Vec::new())),
        }
    }

    /// The loan `/api/me` reports, if this session is one. `None` for every
    /// ordinary login — which is how a screen knows it is on its own app.
    pub(crate) fn grant(&self) -> Option<Grant> {
        let me = self.core.me()?;
        Grant::from_me(&me)
    }

    /// Wire the process's controller to a grant WITHOUT starting its loop: the
    /// banner paints before the first read, and a test drives `flush` /
    /// `read_once` itself.
    pub(crate) fn wire(self: &Arc<Self>, grant: Grant, runtime: tokio::runtime::Handle) {
        Controller::ensure(Arc::clone(&self.api), runtime, grant);
    }

    /// Start (or resume) the traffic for this grant: the process's one controller,
    /// on the session's client, with the rules of the crate's own loop.
    pub(crate) fn start(self: &Arc<Self>, grant: Grant, runtime: tokio::runtime::Handle) {
        let controller = Controller::ensure(Arc::clone(&self.api), runtime, grant);
        controller.spawn(Arc::clone(&self.api));
    }

    pub(crate) fn controller(&self) -> Option<Controller> {
        Controller::get()
    }

    /// The banner: phase, whether the owner's player answers, the grant, the last
    /// cause, and what the person may do.
    pub(crate) fn status(&self, restored: Option<String>) -> StatusView {
        let mut view = StatusView {
            restored,
            ..StatusView::default()
        };
        let Some(controller) = Controller::get() else {
            return view;
        };
        let status = controller.status();
        view.phase = status.phase();
        view.online = status.online;
        view.error = status.error.clone();
        view.grant = status.grant.as_ref().map(GrantView::from);
        view.version = status.version;
        view
    }

    /// Where the owner's track is right now, on the owner's published line
    /// (`controller.js#position`): the hot read the progress bar calls per frame,
    /// and nothing is pushed while the screen is off.
    pub fn position(&self) -> f64 {
        Controller::get()
            .map(|controller| controller.position())
            .unwrap_or(0.0)
    }

    /// The loan's view, as the "Contrôle distant" screen draws it.
    pub(crate) fn now(&self) -> NowView {
        let (position, online, qv) = match Controller::get() {
            Some(controller) => (
                controller.position(),
                controller.status().online,
                controller.queue_version(),
            ),
            None => (0.0, false, String::new()),
        };
        let view = lock(&self.view);
        let track = view.current().map(row_of);
        let read = lock(&self.queue);
        NowView {
            track,
            playing: view.playing(),
            index: view.index(),
            len: view.len() as u32,
            position_s: position,
            duration_s: view.duration(),
            volume: view.volume(),
            muted: view.muted(),
            shuffle: view.shuffle(),
            repeat: repeat_wire(view.repeat()),
            status: if view.current().is_none() {
                "idle".to_owned()
            } else if view.playing() {
                "playing".to_owned()
            } else {
                "paused".to_owned()
            },
            online,
            queue_version: if read.0 == qv {
                read.1
            } else {
                read.1.wrapping_add(1)
            },
        }
    }

    /// The owner's queue, as the rows a windowed list draws — read once per `qv`
    /// the server reports (`controller.js#loadQueue`).
    pub(crate) fn queue(&self) -> Vec<Row> {
        lock(&self.queue).2.clone()
    }

    /// One transport command from the loan's screen: applied at ONCE, then sent.
    ///
    /// The local application is the SPA's `rebindPlayer`, and it happens BEFORE
    /// the network so the button answers in the same frame. A command the grant
    /// does not allow is refused before it touches the view, so a `queue`-only
    /// link can never make this screen lie about the owner's player.
    pub(crate) fn send(&self, transport: Transport) -> Result<(), CoreError> {
        let Some(controller) = Controller::get() else {
            return Err(CoreError::Unsupported {
                feature: "remote:loan".to_owned(),
            });
        };
        let now = controller::now_ms().max(0) as u64;
        let mut view = lock(&self.view);
        match controller.submit(&mut view, transport.wire(), now) {
            Sent::Sent => Ok(()),
            // The refusal is a French sentence from the crate's own table: the UI
            // shows it and nothing changed.
            Sent::Refused(refusal) => Err(CoreError::Invalid {
                detail: refusal.to_owned(),
            }),
        }
    }

    /// Send what is waiting, now (the tap's own POST). A refusal is the server's
    /// and is reported, never retried.
    pub(crate) async fn flush(&self) {
        if let Some(controller) = Controller::get() {
            controller.flush(&self.api).await;
        }
    }

    /// One step of the owner's traffic, for a test that owns the events: read
    /// `/remote/state`, adopt, follow the line.
    pub(crate) async fn read_once(&self) -> controller::ReadStep {
        let Some(controller) = Controller::get() else {
            return controller::ReadStep::Failed;
        };
        let step = controller.read_once(&self.api).await;
        self.adopt();
        step
    }

    /// Take what the owner said and apply it to the view; then move the playhead
    /// along the published line at the shell's own rate (`controller.js#tick`).
    /// Returns whether anything moved, so the screen repaints once per change.
    pub(crate) fn tick(&self) -> bool {
        let Some(controller) = Controller::get() else {
            return false;
        };
        let now = controller::now_ms().max(0) as u64;
        let mut changed = self.adopt();
        if controller.follow_line(&mut lock(&self.view)) {
            changed = true;
        }
        let _ = now;
        changed
    }

    /// Adopt the owner's state and the owner's mirrored settings. The settings
    /// come back from the crate for the caller to apply, because the store is
    /// this app's; only the ones that DIFFER are written, so a 1 Hz read costs
    /// zero writes.
    fn adopt(&self) -> bool {
        let Some(controller) = Controller::get() else {
            return false;
        };
        let now = controller::now_ms().max(0) as u64;
        let outcome = controller.take_read(&mut lock(&self.view), now);
        if let Some(settings) = outcome.settings {
            self.mirror_settings(&settings);
        }
        // The queue only when its version moved: 4 000 rows are cloned once per
        // change, never once per read.
        let qv = controller.queue_version();
        if lock(&self.queue).0 != qv {
            let rows: Vec<Row> = controller.queue().iter().map(row_of).collect();
            let mut held = lock(&self.queue);
            held.0 = qv.clone();
            held.1 = held.1.wrapping_add(1);
            held.2 = rows;
        }
        outcome.adopted
    }

    /// The owner's device settings, applied here: the same keys, validated by the
    /// same `check` Réglages uses. A key this build does not lend, or a value of
    /// the wrong shape, is dropped — the controller's card then shows what this
    /// device really holds on its next read.
    fn mirror_settings(&self, settings: &serde_json::Value) {
        let current = super::settings::snapshot(&self.core);
        for key in super::settings::changed(&current, settings) {
            let Some(value) = settings.get(&key) else {
                continue;
            };
            if let Some(applied) = super::settings::apply(&self.core, &key, value)
                && let Err(error) = applied
            {
                tracing::debug!(%key, %error, "réglage du lecteur piloté refusé");
            }
        }
    }

    /// `POST /api/remote/leave`: the loan ends here, and nothing is sent anywhere
    /// afterwards. The answer names the account the server handed back, which the
    /// "Contrôle terminé" card prints.
    pub(crate) async fn leave(&self) -> Result<Option<String>, CoreError> {
        if let Some(controller) = Controller::get() {
            controller.stop();
        }
        let left = super::wire::leave(&self.api).await?;
        Ok(left
            .restored
            .or_else(|| self.grant().map(|grant| grant.owner)))
    }

    /// Stop the loop without touching the server (a logout, a cut).
    pub(crate) fn stop(&self) {
        if let Some(controller) = Controller::get() {
            controller.stop();
        }
    }
}

/// The wire's spelling of a repeat mode (`host::repeat_wire`, `remote.py`).
pub(crate) fn repeat_wire(repeat: nz_player::Repeat) -> String {
    nz_remote::host::repeat_wire(repeat).to_owned()
}

/// A wire entry as the row a list draws.
fn row_of(entry: &nz_player::Entry) -> Row {
    let artists = {
        let names = entry
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        if names.is_empty() {
            entry.artist_name().to_owned()
        } else {
            names
        }
    };
    Row {
        key: entry.key().to_owned(),
        id: entry.reference().unwrap_or_default().to_owned(),
        title: entry.title.clone(),
        artists,
        album: entry
            .album
            .as_ref()
            .map(|album| album.title.clone())
            .unwrap_or_default(),
        album_id: entry
            .album
            .as_ref()
            .and_then(|album| album.id.clone().or_else(|| album.deezer_id.clone())),
        duration_s: entry.duration,
        cover: entry.cover.clone(),
        explicit: entry.is_explicit(),
        liked: false,
        disliked: false,
        unavailable: entry.is_unavailable(),
        downloaded: entry.is_local(),
        source: if entry.is_local() {
            "local".to_owned()
        } else {
            "deezer".to_owned()
        },
        position: 0,
        played_at: 0,
        added_at: 0,
    }
}

/// One transport command from the loan's screen, in the phone's own vocabulary.
///
/// `commands.js#commandFor`'s table is the SPA's; this is what the "Contrôle
/// distant" screen offers, and it names no op that table does not have. What a
/// level refuses is the crate's to say (`Grant::allows`), never this file's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transport {
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
    /// A drag on the seek bar (`controller.js#watchSeek`).
    Seek(f64),
    /// A tap on the owner's queue.
    Jump(u32),
    /// A drag on the volume fader — the crate's own 150 ms coalescing keeps a
    /// burst of these to one request.
    Volume(f64),
    Mute,
    Shuffle,
    Repeat,
}

impl Transport {
    /// The wire command this button means.
    pub fn wire(self) -> WireCommand {
        match self {
            Transport::Play => WireCommand::play(),
            Transport::Pause => WireCommand::pause(),
            Transport::Toggle => WireCommand::toggle(),
            Transport::Next => WireCommand::next(),
            Transport::Prev => WireCommand::prev(),
            Transport::Seek(s) => WireCommand::seek(s.max(0.0)),
            Transport::Jump(index) => WireCommand::jump(index as usize),
            Transport::Volume(v) => WireCommand::set_volume(v.clamp(0.0, 1.0)),
            Transport::Mute => WireCommand::toggle_mute(),
            Transport::Shuffle => WireCommand::toggle_shuffle(),
            Transport::Repeat => WireCommand::cycle_repeat(),
        }
    }
}

/// The phase a banner prints as one word, for the Kotlin side's own copy test.
pub fn phase_label(phase: Phase, online: bool) -> &'static str {
    phase.label(online)
}
