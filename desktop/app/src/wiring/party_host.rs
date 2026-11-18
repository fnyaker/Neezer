//! Écoute partagée — the HOST side: the desktop's GPUI adapter over `nz-party`.
//!
//! SC6 moved the protocol to `desktop/crates/nz-party`: the wire types, the
//! fitted line, the decision, the clock probe, the publish loop and the recovery
//! after a server restart — and the whole guest, which had no GPUI use. That
//! crate's `host` module note carries the rules (the host is only WATCHED, the
//! line is a fit, the next track is announced, rule 1's one publish in flight)
//! and their numbers; the phone runs the same code through
//! `mobile/rust/crates/nz-ffi/src/party.rs`.
//!
//! What is left here is exactly the part that has to know THIS app:
//!
//! - [`PartyHost::snapshot`] reads [`AppState`]'s player and engine (the queue's
//!   entry and its gain, what the engine really carries, the heard position, the
//!   crossfade, the normalisation level, the output clock's generation, the
//!   planned handover) and hands the numbers to the crate;
//! - [`PartyHost::spawn_tick`] is the shell's timer: 250 ms, and 40 ms while the
//!   line is young, repainting only when the sheet's version moved;
//! - [`PartyHost::start`], [`resume_at_launch`], [`PartyHost::resume_under`] and
//!   [`PartyHost::end`] are the `cx.spawn` seams around the requests, and the
//!   toasts.
//!
//! [`pub use nz_party::host::*`] keeps every screen's path
//! (`crate::wiring::party_host::{HostStatus, …}`) and the sheet's reading
//! unchanged.

pub use nz_party::host::*;

use gpui_kit::Context;
use nz_model::Party;
use serde_json::json;
use tracing::debug;

use crate::state::AppState;

/// The host service behind this app's GPUI seams. Cheap to clone, like the
/// crate's: the party outlives the screen that started it.
#[derive(Clone)]
pub struct PartyHost {
    host: Host,
}

impl PartyHost {
    pub fn get() -> Option<PartyHost> {
        Host::get().map(|host| PartyHost { host })
    }

    pub fn status(&self) -> HostStatus {
        self.host.status()
    }

    pub fn id(&self) -> String {
        self.host.id()
    }

    pub fn running(&self) -> bool {
        self.host.running()
    }

    /// Give the service this session's client, runtime and store.
    fn attach(state: &AppState) -> Option<PartyHost> {
        let api = state.api()?;
        let cache = state.cache.clone()?;
        let runtime = state.runtime.clone();
        Some(PartyHost {
            host: Host::attach(api, cache, runtime, state.server.clone()),
        })
    }

    /// The sheet's « Démarrer l'écoute partagée »: the clock first (a device that
    /// cannot run one must not leave a party open with nobody publishing to it),
    /// then `POST /api/party`, which returns the party this account already
    /// hosts when it hosts one.
    pub fn start(state: &mut AppState, cx: &mut Context<AppState>) {
        let Some(host) = PartyHost::attach(state) else {
            return;
        };
        let api = state.api().expect("attached with a client");
        let runtime = state.runtime.clone();
        host.host.clear_error();
        let task = runtime.spawn(async move { api.party_start(None).await });
        cx.spawn(async move |state, cx| {
            let answer = task.await;
            state
                .update(cx, |_state, cx| match answer {
                    Ok(Ok(party)) => host.begin(party, cx),
                    Ok(Err(error)) => host.host.fail(start_cause(&error)),
                    Err(_) => host.host.fail("La party n'a pas pu démarrer.".to_owned()),
                })
                .ok();
        })
        .detach();
    }

    /// The one call `state.rs` makes at a login or a restore: ask the server
    /// whether this account hosts a party, and pick it back up — so a relaunched
    /// app does not strand the guests on a link nobody publishes to (the SPA's
    /// `maybeResumeHosting`).
    ///
    /// Nothing is asked when this device never hosted and remembers no party: a
    /// feature nobody used costs a launch no request (rule 3).
    pub fn resume_at_launch(state: &mut AppState, cx: &mut Context<AppState>) {
        let Some(host) = PartyHost::attach(state) else {
            return;
        };
        let saved = host.host.saved_id();
        if !host.host.should_ask() {
            // This device never hosted and remembers nothing: asking the server
            // would be one request per launch for a feature nobody used (rule 3).
            return;
        }
        let api = state.api().expect("attached with a client");
        let runtime = state.runtime.clone();
        let task = runtime.spawn(async move { api.party_mine().await });
        cx.spawn(async move |state, cx| {
            let answer = task.await;
            state
                .update(cx, |state, cx| match answer {
                    Ok(Ok(mine)) => {
                        if let Some(party) = mine.party {
                            host.begin(party, cx);
                        } else if let Some(id) = saved {
                            host.resume_under(id, state, cx);
                        }
                    }
                    Ok(Err(error)) => debug!(%error, "party hébergée au démarrage"),
                    Err(_) => {}
                })
                .ok();
        })
        .detach();
    }

    /// Adopt a party: the crate opens its loops and the sheet's status, and the
    /// shell's measure tick starts here (it is GPUI's timer).
    fn begin(&self, party: Party, cx: &mut Context<AppState>) {
        self.host.begin(&party);
        self.spawn_tick(cx);
    }

    /// Take the same id back after the server forgot the party (`{resume}`): the
    /// guests' links keep working, and everything is published again.
    fn resume_under(&self, id: String, state: &mut AppState, cx: &mut Context<AppState>) {
        let api = state.api().expect("attached with a client");
        let runtime = state.runtime.clone();
        let host = self.clone();
        let wanted = id.clone();
        let task =
            runtime.spawn(async move { api.party_start(Some(&json!({ "resume": wanted }))).await });
        cx.spawn(async move |state, cx| {
            let answer = task.await;
            state
                .update(cx, |_state, cx| match answer {
                    Ok(Ok(party)) if party.id == id => host.begin(party, cx),
                    Ok(Ok(other)) => {
                        // Somebody else holds that id now: this account did not
                        // ask for a new party, so none is created.
                        host.host.another_holds_it();
                        let _ = other;
                    }
                    Ok(Err(error)) => debug!(%error, "reprise de l'écoute partagée"),
                    Err(_) => {}
                })
                .ok();
        })
        .detach();
    }

    /// End it: the loops stop BEFORE the answer, so nothing is published on a
    /// party that is being closed.
    pub fn end(&self, state: &mut AppState, cx: &mut Context<AppState>) {
        let id = self.host.close();
        let Some(api) = state.api() else { return };
        let runtime = state.runtime.clone();
        let task = runtime.spawn(async move { api.party_end(&id).await });
        cx.spawn(async move |state, cx| {
            let answer = task.await;
            state
                .update(cx, |_state, cx| {
                    let entity = cx.entity();
                    match answer {
                        Ok(Ok(())) => crate::widgets::sheets::push_toast(
                            cx,
                            &entity,
                            crate::toast::ToastKind::Info,
                            "Écoute partagée terminée.",
                        ),
                        Ok(Err(error)) => crate::widgets::sheets::push_toast(
                            cx,
                            &entity,
                            crate::toast::ToastKind::Error,
                            format!("La party n'a pas pu être fermée : {}", error.cause_fr()),
                        ),
                        Err(_) => {}
                    }
                })
                .ok();
        })
        .detach();
    }

    // --- the GPUI side -------------------------------------------------------

    /// One measurement, on the shell's own executor: read the engine, fit the
    /// line, publish it when it moved.
    fn tick(&self, state: &AppState) -> bool {
        let snapshot = self.snapshot(state);
        self.host.tick(snapshot.as_ref())
    }

    /// What this device is playing, read from the shell's state: the engine's
    /// own heard position when it carries the track, the queue's entry for the
    /// metadata, the plan for the next handover.
    fn snapshot(&self, state: &AppState) -> Option<Snapshot> {
        let entry = state.player.current()?;
        let id = entry.reference().unwrap_or("").to_owned();
        let audio = state.audio.as_ref();
        let carries = audio
            .map(|audio| audio.engine.current_id().as_deref() == Some(id.as_str()))
            .unwrap_or(false);
        let playing = state.player.playing() && carries;
        let heard = if carries {
            audio.map(|audio| audio.position()).unwrap_or(0.0).max(0.0)
        } else {
            state.player.current_time().max(0.0)
        };
        let (fade, norm, generation) = match audio {
            Some(audio) => {
                let settings = audio.engine.fade();
                let fade = if settings.enabled {
                    settings.seconds
                } else {
                    0.0
                };
                (
                    fade,
                    norm_wire(audio.engine.normalization()),
                    audio.engine.output_clock().generation,
                )
            }
            None => (0.0, "off", 0),
        };
        let gaps = self.host.gaps();
        let next = next_plan(&state.player, entry, &gaps, fade);
        Some(Snapshot {
            id,
            track: track_info(entry, entry.gain),
            carries,
            playing,
            heard,
            fade,
            duration: if entry.duration > 0 {
                f64::from(entry.duration)
            } else {
                state.player.duration()
            },
            norm,
            generation,
            next,
        })
    }

    /// The measure tick: 250 ms, and 40 ms while the line is young. It repaints
    /// only when the sheet's own version moved.
    fn spawn_tick(&self, cx: &mut Context<AppState>) {
        let host = self.clone();
        cx.spawn(async move |state, cx| {
            let mut painted = host.status().version;
            loop {
                cx.background_executor().timer(MEASURE).await;
                let alive = state.update(cx, |state, cx| {
                    if !host.running() {
                        return None;
                    }
                    let quick = host.tick(state);
                    let version = host.status().version;
                    if version != painted {
                        painted = version;
                        cx.notify();
                    }
                    Some(quick)
                });
                match alive {
                    Ok(Some(true)) => cx.background_executor().timer(QUICK).await,
                    Ok(Some(false)) => {}
                    // The party ended, or the shell is gone.
                    _ => break,
                }
            }
        })
        .detach();
    }
}

/// The one call `state.rs` makes at a login or a restore (`on_restored` /
/// `on_logged_in`, next to `remote_host::start`): pick back up the party this
/// account hosts, if any. See [`PartyHost::resume_at_launch`].
pub fn resume_at_launch(state: &mut AppState, cx: &mut Context<AppState>) {
    PartyHost::resume_at_launch(state, cx);
}
