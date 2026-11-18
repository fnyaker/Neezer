//! Remote control, the CONTROLLING side — the shell's half.
//!
//! The protocol moved to the `nz-remote` crate (SC5, `docs/agents/mobile.md`
//! *SC5*): the read of `/api/remote/state` (the position line, the `ack` hold,
//! the queue read once per `qv`), the outbox and its rule "never queued across
//! an outage", the grant and its levels, the reconnect loop and the token a pasted
//! link yields live in `nz_remote::controller`, because the phone drives a friend's
//! player the same way and none of it needed GPUI. The crate's doc is the
//! contract; this module is the re-export every screen keeps using
//! (`crate::wiring::remote_controller::…`) plus the part that has to know this
//! app:
//!
//! - [`Controller`], the handle a screen holds: the crate's controller plus the
//!   three methods that need this app — [`Controller::command`] (a tap applied
//!   through this app's player, with a toast when the loan refuses it),
//!   [`Controller::tick`] (what the owner said, adopted into `AppState`) and
//!   [`Controller::apply_settings`] (the store plus the shell's own half);
//! - [`start`], the one call `remote_host::start` makes when `/api/me` reports a
//!   loan, and [`spawn_tick`], the GPUI task that runs the tick;
//! - [`adopt`], the app-side rule that a controller sounds NOTHING: the owner's
//!   state becomes this player's view and every effect is dropped.
//!
//! Rules the moved code keeps: a tap answers in the same frame and is posted
//! after (the SPA's `rebindPlayer` + `pending` pair), the position runs on the
//! owner's published line, every request on [`nz_api::Lane::User`], and a
//! command pressed while the link is down is DROPPED, never carried over the
//! outage.

pub use nz_remote::controller::*;

use gpui_kit::Context;
use nz_player::{Command, Snapshot};
use serde_json::Value;
use tracing::info;

use crate::state::AppState;

/// The process's controller, as a screen holds it: `nz-remote`'s state machine,
/// plus the methods whose signature is this app's (`AppState`, `Context`).
#[derive(Clone)]
pub struct Controller {
    inner: nz_remote::controller::Controller,
}

impl From<nz_remote::controller::Controller> for Controller {
    fn from(inner: nz_remote::controller::Controller) -> Self {
        Self { inner }
    }
}

impl Controller {
    /// The process's controller, when one was started.
    pub fn get() -> Option<Self> {
        nz_remote::controller::Controller::get().map(Self::from)
    }

    /// What the screen paints.
    pub fn status(&self) -> Status {
        self.inner.status()
    }

    /// The owner's queue, as last read.
    pub fn queue(&self) -> Vec<nz_player::Entry> {
        self.inner.queue()
    }

    /// The version of the queue those rows belong to.
    pub fn queue_version(&self) -> String {
        self.inner.queue_version()
    }

    /// The grant, when this session is a loan.
    pub fn grant(&self) -> Option<Grant> {
        self.inner.grant()
    }

    /// Whether `op` may be sent on this loan (the UI hides the rest).
    pub fn allows(&self, op: &str, args: Option<&Value>) -> bool {
        self.inner.allows(op, args)
    }

    /// The length of the owner's track, as its line published it.
    pub fn duration(&self) -> f64 {
        self.inner.duration()
    }

    /// Where the owner's track is right now, on its published line.
    pub fn position(&self) -> f64 {
        self.inner.position()
    }

    /// Ask for a fast read now (the screen's « Réessayer », a tap).
    pub fn poke(&self) {
        self.inner.poke()
    }

    /// A leave: the loan ends here, and nothing is sent anywhere afterwards.
    pub fn stop(&self) {
        self.inner.stop()
    }

    /// Send one command, and show its effect at once.
    ///
    /// The local application is the SPA's `rebindPlayer`: the button answers in
    /// the same frame, and the owner's own state replaces it once the owner's
    /// player has RUN it. A command this loan may not send is refused BEFORE it
    /// touches the local state, so a `queue`-level link cannot make this screen
    /// lie about the owner's player.
    pub fn command(&self, state: &mut AppState, command: Command, cx: &mut Context<AppState>) {
        let now = state.now_ms();
        match self.inner.submit(&mut state.player, command, now) {
            Sent::Refused(refusal) => {
                state.toasts.push(crate::toast::ToastKind::Info, refusal);
            }
            Sent::Sent => {}
        }
        cx.notify();
    }

    /// Apply what the owner says, and move the playhead along its line. Called by
    /// the foreground tick ([`TICK`]); public so a test drives it by hand.
    pub fn tick(&self, state: &mut AppState, cx: &mut Context<AppState>) {
        let now = state.now_ms();
        let outcome = self.inner.take_read(&mut state.player, now);
        let mut changed = outcome.adopted;
        if let Some(settings) = outcome.settings {
            self.apply_settings(state, settings);
        }
        // Between two reads the playhead follows the published line, at the
        // shell's own rate (`controller.js#tick`).
        if self.inner.follow_line(&mut state.player) {
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    /// The owner's mirrored settings, applied to this device: only the ones that
    /// differ, so a 1 Hz read writes nothing.
    fn apply_settings(&self, state: &mut AppState, settings: Value) {
        let Some(object) = settings.as_object() else {
            return;
        };
        let Some(cache) = state.cache.clone() else {
            return;
        };
        for (key, value) in object {
            if nz_remote::host::mirrored_value(&cache, key).is_some_and(|current| current == *value)
            {
                continue;
            }
            crate::wiring::remote_host::apply_setting(state, key, value);
        }
    }
}

/// Adopt the owner's state: the local player becomes a view of it, and its
/// effects are DROPPED — a controller sounds nothing, reports nothing and stores
/// nothing of the owner's (`mode.js`). The crate's `adopt` is that rule; this is
/// the same call with the app's own clock.
pub fn adopt(state: &mut AppState, snapshot: Snapshot, playing: bool) {
    let now = state.now_ms();
    nz_remote::controller::adopt(&mut state.player, snapshot, playing, now);
}

// --- the session's side ------------------------------------------------------

/// Start the controller for a loan (`remote_host::start` calls this).
pub fn start(state: &mut AppState, cx: &mut Context<AppState>) {
    let Some(me) = state.auth.user().cloned() else {
        return;
    };
    let Some(grant) = Grant::from_me(&me) else {
        return;
    };
    let Some(api) = state.api() else {
        return;
    };
    // A loan keeps nothing of the owner's on this device (`lib/apicache.js`).
    api.set_caching(false);
    // The engine (started before `/api/me` reported the loan) is not this
    // device's to sound: a controller mounts no audio at all.
    state.audio = None;
    state.viz = None;
    let controller: Controller =
        nz_remote::controller::Controller::ensure(api.clone(), state.runtime.clone(), grant).into();
    controller.inner.spawn(api);
    spawn_tick(controller.clone(), cx);
    info!(
        level = controller
            .grant()
            .map(|grant| grant.level.wire())
            .unwrap_or("?"),
        "contrôle à distance : ce poste pilote un autre lecteur"
    );
}

/// The controller's foreground tick, spawned once per loan. It ends when the
/// grant does, so a leave costs no wake.
fn spawn_tick(controller: Controller, cx: &mut Context<AppState>) {
    cx.spawn(async move |state, cx| {
        loop {
            cx.background_executor().timer(TICK).await;
            let alive = state.update(cx, |state, cx| {
                controller.tick(state, cx);
                controller.status().phase() != Phase::Ended
            });
            match alive {
                Ok(true) => {}
                _ => break,
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_player::Entry;

    #[test]
    fn adopting_the_owner_state_sounds_nothing_and_keeps_nothing() {
        // A controller must not load a stream, report a listen or persist a
        // queue: `adopt` replaces the player and DROPS every effect.
        let dir = std::env::temp_dir().join(format!(
            "neezer-d15a-adopt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let (events, _receiver) = crate::state::event_channel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let mut app = AppState::new(
            crate::state::AppConfig::for_test(&dir),
            runtime.handle().clone(),
            events,
            None,
        );
        let snapshot = Snapshot {
            queue: vec![Entry::new("abc"), Entry::new("def")],
            index: 1,
            position: 42.0,
            volume: 0.4,
            shuffle: true,
            ..Snapshot::default()
        };
        adopt(&mut app, snapshot, true);
        assert!(app.player.playing(), "le propriétaire joue");
        assert_eq!(app.player.index(), 1);
        assert_eq!(app.player.len(), 2);
        assert!((app.player.current_time() - 42.0).abs() < 1e-9);
        assert!((app.player.volume() - 0.4).abs() < 1e-9);
        assert!(
            app.last_effect.is_none(),
            "aucun effet n'est exécuté sur un poste de contrôle"
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
