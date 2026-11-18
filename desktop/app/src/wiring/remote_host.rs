//! Remote control, the CONTROLLED side — the shell's half.
//!
//! The protocol moved to the `nz-remote` crate (SC5, `docs/agents/mobile.md`
//! *SC5*): the wire session (poll `since`/`chan`, publish, `ack`, the settings
//! and the links a controller may read), the levels, the two keys this device
//! keeps (`remote.device`, `remote.hosted`) and the poll loop live in
//! `nz_remote::host`, because the phone lends its player the same way and none of
//! it needed GPUI. The crate's doc is the contract; this module is the re-export
//! every screen keeps using (`crate::wiring::remote_host::…`) plus the part that
//! has to know this app:
//!
//! - [`tick`], the foreground tick: it drains what the network task queued,
//!   applies it through `AppState::command` — the ONE transport path a click, a
//!   key and MPRIS already take, so the engine, the crossfade and MPRIS follow —
//!   and hands the state and the queue back to publish. 120 ms while a link is
//!   live, 2 s otherwise;
//! - [`apply_setting`], the store's write PLUS this build's own half (the
//!   engine's rung and the toast that says it applies at the next track);
//! - [`snapshot_of`], the projection of `AppState` onto the wire (the engine's
//!   clock while it plays — it subtracts the device latency — the state machine's
//!   playhead otherwise);
//! - [`start`], the one call `AppState::on_restored`/`on_logged_in` make, and
//!   [`RemoteHost::cut_everything`], which needs a whole `AppState`;
//! - [`spawn_tick`], the GPUI task that runs [`tick`].
//!
//! Rules the moved code keeps: polling never a held connection, every request on
//! [`nz_api::Lane::User`], the queue sent only when its fingerprint moved, and
//! nothing at all sent while no link is live (rule 1 and rule 3).

pub use nz_remote::host::*;

use gpui_kit::Context;
use nz_api::ApiError;
use serde_json::Value;
use tracing::debug;

use crate::state::AppState;

/// Run one foreground tick: apply what arrived, hand over what to publish.
/// Called from the task [`start`] spawns, and by the tests that drive it by hand.
pub fn tick(host: &RemoteHost, state: &mut AppState, cx: &mut Context<AppState>) {
    let pending: Vec<Pending> = host.drain_pending();
    let ran = pending.len();
    let mut settings_changed = false;
    for item in pending {
        match item {
            // The ONE transport path: a remote `next` is `PlayerState::apply`
            // here, exactly what the owner's own button calls, so the engine,
            // the crossfade, MPRIS and the queue top-ups follow.
            Pending::Command(command) => state.command(command, cx),
            Pending::Setting { key, value } => {
                if apply_setting(state, &key, &value).is_some() {
                    settings_changed = true;
                }
            }
            Pending::Ignored(op) => debug!(op, "commande de contrôle ignorée"),
        }
    }
    if settings_changed && let Some(cache) = state.cache.clone() {
        // The controller must see the value it just set, and a second
        // controller must see it too: a new `sv` with the new values.
        host.hand_over_settings(&cache);
    }
    host.note_applied(ran);
    // The state and the queue's fingerprint in one call, so the rows and the
    // state they describe can never be published out of step.
    let fingerprint = host.hand_over_queue(state.player.seq(), state.player.queue());
    host.publish_state(snapshot_of(state, fingerprint));
}

/// Apply one `set` command from a controller to this device: the crate writes the
/// `kv` keys Réglages writes, and this build does its own half — the engine's
/// rung, said in a toast. Returns the name of what changed, for the log, or
/// `None` when this build cannot honour it (the controller's value then snaps
/// back on the next state read, which is the honest answer).
pub fn apply_setting(app: &mut AppState, key: &str, value: &Value) -> Option<String> {
    match nz_remote::host::apply_setting(app.cache.as_ref(), key, value)? {
        Changed::Quality(rung) => {
            app.quality = crate::wiring::engine::PlaybackQuality::parse(rung);
            // The SPA's chip says the same thing: the rung applies at the next
            // track, so a change during one does not restart it.
            app.toasts.push(
                crate::toast::ToastKind::Info,
                format!(
                    "Qualité : {} — au prochain titre",
                    crate::state::quality_label(app.quality)
                ),
            );
            Some(format!("player.quality={rung}"))
        }
        Changed::Stored(changed) => Some(changed),
    }
}

/// Build what the wire publishes from the live state. The engine's own clock
/// while it plays (it subtracts the device latency), the state machine's
/// playhead otherwise — D9's rule.
pub fn snapshot_of(state: &AppState, _fingerprint: u64) -> Published {
    let player = &state.player;
    let position = match &state.audio {
        Some(audio) if player.playing() => audio.position(),
        _ => player.current_time(),
    };
    let position = if position.is_finite() {
        position.max(0.0)
    } else {
        0.0
    };
    let status = if player.current().is_none() {
        "idle"
    } else if player.playing() {
        "playing"
    } else {
        "paused"
    };
    Published {
        index: player.index(),
        playing: player.playing(),
        p: position,
        duration: player.duration(),
        volume: player.volume(),
        muted: player.muted(),
        shuffle: player.shuffle(),
        repeat: repeat_wire(player.repeat()).to_owned(),
        status: status.to_owned(),
        track: player
            .current()
            .and_then(|entry| serde_json::to_value(entry).ok()),
        context: player
            .context()
            .and_then(|context| serde_json::to_value(context).ok()),
    }
}

/// Cut every link onto this device — the owner's always-one-tap-away button
/// (the chip over the player bar, and the sheet's own). The caller hears the
/// outcome: a failure says why (rule 2).
pub fn cut_everything(state: &AppState) -> Option<tokio::task::JoinHandle<Result<(), ApiError>>> {
    let api = state.api()?;
    let cache = state.cache.clone()?;
    let device = device_id(&cache);
    if let Some(host) = RemoteHost::get() {
        // Nothing is sent on a link that is being cut.
        host.stop();
    }
    Some(
        state
            .runtime
            .spawn(async move { api.remote_revoke_device(&device).await }),
    )
}

// --- the session's entry point ----------------------------------------------

/// What the session does at a login or a restore: start the CONTROLLER when
/// this session is a loan, start the HOST otherwise.
///
/// This is the one function `AppState::on_restored`/`on_logged_in` call, so a
/// link that was live before a restart answers again without anybody opening a
/// screen — the SPA's `initRemoteHost` at boot, in one call.
pub fn start(state: &mut AppState, cx: &mut Context<AppState>) {
    if let Some(me) = state.auth.user()
        && me.remote.is_some()
    {
        // A loan: this device drives somebody else's player and has no business
        // sounding anything (the SPA mounts no audio engine for a controller).
        state.audio = None;
        state.viz = None;
        crate::wiring::remote_controller::start(state, cx);
        return;
    }
    let Some(api) = state.api() else {
        return;
    };
    let Some(cache) = state.cache.clone() else {
        return;
    };
    let device = device_id(&cache);
    let hosted = RemoteHost::hosted(&cache);
    let service = RemoteHost::ensure(api, cache.clone(), device, state.runtime.clone());
    // A device that has never been lent asks NOTHING and runs NO task: the
    // feature costs a launch nothing until somebody uses it (rule 3), and no
    // other area's tests inherit a task they do not use.
    if !hosted {
        cx.notify();
        return;
    }
    // The settings a controller may change: published once here, so a controller
    // that opens its own Réglages sees them instead of an empty map.
    service.hand_over_settings(&cache);
    // A link may still point at this device: ask, and answer again.
    service.resume(true);
    spawn_tick(service, cx);
    cx.notify();
}

/// The host's foreground tick: apply what the network task queued, and hand the
/// state back to publish. 120 ms while a link is live, 2 s otherwise, and it
/// repaints the shell only when what it publishes MOVED — the chip over the
/// player bar is what shows it.
fn spawn_tick(host: RemoteHost, cx: &mut Context<AppState>) {
    cx.spawn(async move |state, cx| {
        let mut painted = host.status().version;
        loop {
            let running = host.running();
            cx.background_executor()
                .timer(if running { TICK } else { IDLE_TICK })
                .await;
            let alive = state.update(cx, |state, cx| {
                if !host.running() {
                    return;
                }
                tick(&host, state, cx);
                let version = host.status().version;
                if version != painted {
                    painted = version;
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        }
    })
    .detach();
}
