//! download_control: the five states a download shows, and the chip that says
//! one of them (lot `D18`).
//!
//! D13 gave the app a worker, a Réglages page and a badge, but the *screens* had
//! nothing to press: « Télécharger » lived only in the settings page and on a
//! show's episodes. This module is what an album, a playlist, the favourites, a
//! podcast, the queue and a row's menu share, so one state reads the same way in
//! all of them.
//!
//! Four rules shape it:
//!
//! - **One derivation, no SQLite.** A state comes from
//!   [`crate::wiring::offline::Snapshot`], the value `nz-bulk` publishes: the
//!   store is never read from the GPUI thread (`nz-cache`'s access rule — one
//!   connection behind one mutex, so a UI read queues behind the worker's own
//!   write transaction and freezes a frame).
//! - **A failure is never a bare count.** [`State::Failed`] carries the first
//!   CAUSE the server or the link gave, because a screen that says "2 échecs"
//!   makes the person guess (rule 2, rule 3).
//! - **The snapshot says what is ON the device, per track.**
//!   `Snapshot.downloaded` is every published id as one `Arc<HashSet>`, so a row
//!   answers « téléchargé » from the value the worker published — never from a
//!   SQLite read on the GPUI thread. A failed row says why, the row in flight
//!   says its bytes, and a set nobody finished says how far it got.
//! - **The keyboard and the mouse are the same.** [`State::label`] is the
//!   button's text, the menu row's label and the toast's sentence, and the
//!   keymap's `t` / `Maj+T` dispatch the actions the buttons dispatch.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{
    AnyElement, App, Context, Entity, Hsla, InteractiveElement as _, IntoElement, ParentElement,
    Render, SharedString, Styled, WeakEntity, Window, div, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};
use nz_cache::DownloadSet;

use crate::state::AppState;
use crate::theme;
use crate::wiring::offline::{self, Choice, Offline, Snapshot};

/// How often a screen asks the worker whether anything moved. A download's byte
/// count is republished every 250 ms; 300 ms of polling costs one mutex and one
/// comparison (the downloads page's own cadence).
pub const POLL: Duration = Duration::from_millis(300);

/// Where one download stands — for a collection or for a single track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Nothing asked, or a collection only partly there.
    Idle,
    /// Asked, waiting its turn (the worker writes one track at a time).
    Queued { done: usize, total: usize },
    /// Being written right now.
    Running {
        done: usize,
        total: usize,
        bytes: u64,
        size: Option<u64>,
    },
    /// Every track of the collection is on the device.
    Done { tracks: usize },
    /// At least one track gave up, with the first cause and the count.
    Failed {
        cause: String,
        failed: usize,
        done: usize,
    },
}

impl State {
    /// The button's text, the menu row's label, the toast's sentence. French,
    /// and never a number without its unit.
    pub fn label(&self) -> String {
        match self {
            State::Idle => "Télécharger".to_owned(),
            State::Queued { done, total } => format!("En file · {done} / {total}"),
            State::Running { done, total, .. } => format!("En cours · {done} / {total}"),
            State::Done { tracks } => {
                if *tracks > 1 {
                    format!("Téléchargé · {tracks} titres")
                } else {
                    "Téléchargé".to_owned()
                }
            }
            State::Failed {
                cause,
                failed,
                done,
            } => format!(
                "{} échec{} · {done} téléchargé{} · {cause}",
                failed,
                if *failed > 1 { "s" } else { "" },
                if *done > 1 { "s" } else { "" },
            ),
        }
    }

    /// The verb the control's button carries: what pressing it DOES.
    ///
    /// One button per control, always meaningful: ask, take back, stop, ask
    /// again — never a button that says a state and does nothing.
    pub fn action(&self) -> &'static str {
        match self {
            State::Idle => "Télécharger",
            State::Queued { .. } | State::Running { .. } => "Annuler",
            State::Failed { .. } => "Réessayer",
            State::Done { .. } => "Retirer",
        }
    }

    /// The compact sentence the status chip prints beside the button: a header
    /// row is a line, not a paragraph, so this drops what it can and keeps the
    /// cause of a failure.
    pub fn short(&self) -> String {
        match self {
            State::Idle => String::new(),
            State::Queued { done, total } => format!("En file · {done} / {total}"),
            State::Running { done, total, .. } => format!("En cours · {done} / {total}"),
            State::Done { tracks } => {
                if *tracks > 1 {
                    format!("Téléchargé · {tracks}")
                } else {
                    "Téléchargé".to_owned()
                }
            }
            State::Failed {
                cause,
                failed,
                done,
            } => format!(
                "{failed} échec{} · {done} sur l'appareil · {cause}",
                if *failed > 1 { "s" } else { "" }
            ),
        }
    }

    /// The line a card prints under its title: the same facts, without the verb.
    pub fn detail(&self) -> Option<String> {
        match self {
            State::Idle => None,
            State::Queued { done, total } => Some(format!("En file · {done} / {total} titres")),
            State::Running {
                done, total, bytes, ..
            } => Some(format!(
                "En cours · {done} / {total} · {}",
                offline::bytes_label(*bytes)
            )),
            State::Done { tracks } => Some(format!(
                "Sur cet appareil · {tracks} titre{}",
                if *tracks > 1 { "s" } else { "" }
            )),
            State::Failed { .. } => Some(self.label()),
        }
    }

    /// `0..=1` for a bar, when the state knows where it is.
    pub fn fraction(&self) -> Option<f32> {
        match self {
            State::Queued { done, total } => {
                (*total > 0).then(|| (*done as f32 / *total as f32).clamp(0.0, 1.0))
            }
            State::Running {
                done,
                total,
                bytes,
                size,
            } => match size {
                Some(size) if *size > 0 => Some((*bytes as f32 / *size as f32).clamp(0.0, 1.0)),
                _ => (*total > 0).then(|| (*done as f32 / *total as f32).clamp(0.0, 1.0)),
            },
            State::Done { .. } => Some(1.0),
            State::Failed { done, failed, .. } => {
                let total = done + failed;
                (total > 0).then(|| (*done as f32 / total as f32).clamp(0.0, 1.0))
            }
            State::Idle => None,
        }
    }

    /// Work is in the worker's queue for this control.
    pub fn busy(&self) -> bool {
        matches!(self, State::Queued { .. } | State::Running { .. })
    }

    /// Something of it is on the device: the control offers « Retirer » too.
    pub fn on_device(&self) -> bool {
        matches!(self, State::Done { .. } | State::Failed { .. })
    }

    /// The glyph this state paints.
    pub fn glyph(&self) -> IconName {
        match self {
            State::Idle => IconName::Download,
            State::Queued { .. } => IconName::CloudDownload,
            State::Running { .. } => IconName::LoaderCircle,
            State::Done { .. } => IconName::CircleCheckBig,
            State::Failed { .. } => IconName::TriangleAlert,
        }
    }

    /// The colour that carries the same meaning as the settings page's badges.
    fn tone(&self, cx: &App) -> Hsla {
        match self {
            State::Idle | State::Queued { .. } => theme::secondary(cx),
            State::Running { .. } | State::Done { .. } => theme::accent(cx),
            State::Failed { .. } => theme::danger(cx),
        }
    }
}

/// The set a collection was downloaded as, if it was.
pub fn set<'a>(snapshot: &'a Snapshot, kind: &str, ident: &str) -> Option<&'a DownloadSet> {
    snapshot
        .sets
        .iter()
        .find(|set| set.kind == kind && set.ident == ident)
}

/// One collection's state: a playlist, an album, a show, the favourites, the
/// queue.
///
/// `tracks` is what the SCREEN has in hand, and it is used for one thing only —
/// telling a failure of this collection from another's, because the snapshot's
/// failures are per track and a set's row carries no ids.
pub fn collection(snapshot: &Snapshot, kind: &str, ident: &str, tracks: &[String]) -> State {
    let Some(set) = set(snapshot, kind, ident) else {
        return State::Idle;
    };
    let failed: Vec<&str> = snapshot
        .failures
        .iter()
        .map(|failure| failure.track.as_str())
        .filter(|track| tracks.iter().any(|want| want == track))
        .collect();
    if !failed.is_empty() {
        let cause = snapshot
            .failures
            .iter()
            .find(|failure| failed.contains(&failure.track.as_str()))
            .map(|failure| failure.cause.clone())
            .unwrap_or_default();
        return State::Failed {
            cause,
            failed: failed.len(),
            done: set.done,
        };
    }
    if set.tracks > 0 && set.done >= set.tracks {
        return State::Done { tracks: set.done };
    }
    // The worker writes one track at a time and publishes which one: a set with
    // something in flight is RUNNING, one that is only waiting is QUEUED.
    if let Some(running) = &snapshot.running
        && tracks.iter().any(|track| track == &running.track)
    {
        return State::Running {
            done: set.done,
            total: set.tracks,
            bytes: running.bytes,
            size: running.total,
        };
    }
    State::Queued {
        done: set.done,
        total: set.tracks,
    }
}

/// One track's state, from the snapshot alone: `Snapshot.downloaded` says
/// whether the bytes are published, the failures say why one is not, and the
/// running row says which is being written.
pub fn track(snapshot: &Snapshot, track: &str) -> State {
    if let Some(failure) = snapshot
        .failures
        .iter()
        .find(|failure| failure.track == track)
    {
        return State::Failed {
            cause: failure.cause.clone(),
            failed: 1,
            done: 0,
        };
    }
    if let Some(running) = &snapshot.running
        && running.track == track
    {
        return State::Running {
            done: 0,
            total: 1,
            bytes: running.bytes,
            size: running.total,
        };
    }
    if snapshot.downloaded.contains(track) {
        return State::Done { tracks: 1 };
    }
    // A single track and an episode are each a set of one (D13), which is what
    // lets a row read « En file » without a second source of truth.
    let queued = snapshot.sets.iter().any(|set| {
        (set.kind == "track" || set.kind == "show") && set.ident == track && set.done == 0
    });
    if queued {
        return State::Queued { done: 0, total: 1 };
    }
    State::Idle
}

/// The id a track is downloaded under: what `/api/stream/<id>` takes and what
/// the engine's offline seam looks up — the queue row's reference
/// (`commands.js#playableId`), i.e. the row uuid when there is one.
pub fn key(track: &nz_model::Track) -> String {
    nz_player::track_ref(
        track.id.as_ref().map(|id| id.as_str()),
        Some(track.deezer_id.as_str()),
    )
    .unwrap_or_default()
    .to_owned()
}

/// A collection's rows, resolved ONCE per snapshot version.
///
/// A list draws one chip per visible row; asking [`track`] per row would clone
/// the whole snapshot per row (`Offline::snapshot` copies the sets and the
/// labels). This keeps only what a row can say — the failures, the one track in
/// flight, the single-track sets — in three lookups a paint can afford.
#[derive(Debug, Default)]
pub struct Rows {
    /// Every track PUBLISHED on the device, straight from the snapshot
    /// (`Snapshot.downloaded`, one `Arc` — a version clones a pointer, not four
    /// thousand ids).
    downloaded: Arc<HashSet<String>>,
    failed: HashMap<String, String>,
    queued: HashSet<String>,
    running: Option<(String, u64, Option<u64>)>,
}

impl Rows {
    pub fn build(snapshot: &Snapshot) -> Self {
        let failed: HashMap<String, String> = snapshot
            .failures
            .iter()
            .map(|failure| (failure.track.clone(), failure.cause.clone()))
            .collect();
        let queued: HashSet<String> = snapshot
            .sets
            .iter()
            .filter(|set| (set.kind == "track" || set.kind == "show") && set.done == 0)
            .map(|set| set.ident.clone())
            .collect();
        let running = snapshot
            .running
            .as_ref()
            .map(|running| (running.track.clone(), running.bytes, running.total));
        Self {
            downloaded: Arc::clone(&snapshot.downloaded),
            failed,
            queued,
            running,
        }
    }

    pub fn state(&self, track: &str) -> State {
        if let Some(cause) = self.failed.get(track) {
            return State::Failed {
                cause: cause.clone(),
                failed: 1,
                done: 0,
            };
        }
        if let Some((running, bytes, size)) = &self.running
            && running == track
        {
            return State::Running {
                done: 0,
                total: 1,
                bytes: *bytes,
                size: *size,
            };
        }
        if self.downloaded.contains(track) {
            return State::Done { tracks: 1 };
        }
        if self.queued.contains(track) {
            return State::Queued { done: 0, total: 1 };
        }
        State::Idle
    }
}

/// The row menu's one verb, decided by where the track stands: ask for it, ask
/// again, cancel it or take it back (the SPA's « Télécharger » / « Retirer le
/// téléchargement », one entry, never two that contradict each other).
///
/// Returns the sentence to toast, or `None` when the track has no id to ask for.
/// A screen's menu, its `t` / `Maj+T` keys and a page control all end here, so
/// the three paths cannot drift.
pub fn row_verb(service: &Offline, snapshot: &Snapshot, id: &str, title: &str) -> Option<String> {
    if id.is_empty() {
        return None;
    }
    let state = track(snapshot, id);
    match state {
        State::Idle | State::Failed { .. } => {
            service.choose_track(Choice::new(id, title));
            Some(format!("« {title} » ajouté aux téléchargements"))
        }
        State::Queued { .. } | State::Running { .. } | State::Done { .. } => {
            service.drop_track(id);
            Some(format!("« {title} » retiré des téléchargements"))
        }
    }
}

/// A row's download chip: a glyph, coloured by the state, and NOTHING when the
/// track says nothing (`Idle`).
///
/// No text on purpose: a row's width belongs to the title and the duration, and
/// the list builds one of these per visible row. The cause a failure carries is
/// printed by the collection's control and by Réglages → Téléchargements.
pub fn chip(state: &State, cx: &App) -> Option<AnyElement> {
    if matches!(state, State::Idle) {
        return None;
    }
    Some(
        div()
            .flex()
            .items_center()
            .justify_center()
            .w(rems(1.25))
            .flex_shrink_0()
            .text_color(state.tone(cx))
            .child(icon(state.glyph()).size(rems(0.75)))
            .into_any_element(),
    )
}

/// **The screen a global action belongs to: the one the ROUTE names.**
///
/// The 420 px queue panel is a SECOND `queue::Screen` mounted beside the route's
/// screen (`app.rs#toggle_queue`), and GPUI runs EVERY listener registered for
/// an action — so without this guard, `t` on the library with the panel open
/// would queue the favourites AND the whole queue. A screen acts only when the
/// route is its own; the panel's buttons still work, because a click reaches the
/// panel's own control and the handler it runs is the one that checked.
pub fn is_front(state: &Entity<AppState>, route: &crate::nav::Route, cx: &App) -> bool {
    state.read(cx).nav.current() == Some(route)
}

/// The page's download control: a status chip when there is something to say,
/// and ONE button that always carries the verb of the moment.
///
/// The button dispatches the app's own actions (`DownloadSelection` /
/// `RemoveDownload`) rather than calling the worker, so the mouse click, the
/// keymap's `t` / `Maj+T` and the row menu all run the screen's one handler
/// (`docs/agents/desktop.md`, *Keyboard AND mouse*). A screen that has no
/// session or no local store draws nothing: a button that cannot work is worse
/// than no button (rule 2).
pub fn control(id: &'static str, state: &State, available: bool, cx: &App) -> Option<AnyElement> {
    if !available {
        return None;
    }
    let mut row = div()
        .debug_selector(move || id.to_owned())
        .flex()
        .items_center()
        .gap(rems(0.375))
        .flex_shrink_0();
    if !matches!(state, State::Idle) {
        row = row.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.25))
                .min_w_0()
                .text_size(rems(0.6875))
                .text_color(state.tone(cx))
                .child(icon(state.glyph()).size(rems(0.75)))
                .child(div().truncate().child(state.short())),
        );
    }
    let take_back = state.on_device() || state.busy();
    row = row.child(
        button(
            SharedString::from(format!("{id}-action")),
            state.action(),
            if take_back {
                ButtonVariant::Outline
            } else {
                ButtonVariant::Secondary
            },
            cx,
        )
        .child(
            icon(if take_back {
                IconName::Trash
            } else {
                state.glyph()
            })
            .size(rems(0.75)),
        )
        .on_click(move |_, window: &mut Window, cx: &mut App| {
            if take_back {
                window.dispatch_action(Box::new(crate::actions::RemoveDownload), cx);
            } else {
                window.dispatch_action(Box::new(crate::actions::DownloadSelection), cx);
            }
        }),
    );
    Some(row.into_any_element())
}

/// A screen's view of the downloads worker: the snapshot it last read, the
/// version it painted, and the poll that redraws the screen when the worker
/// publishes something new.
///
/// Five screens hold one of these and draw their « Télécharger » control from
/// it, so a download reads the same on an album, a playlist, the favourites, a
/// show and the queue. It never touches SQLite: [`Snapshot`] is published by the
/// worker, and a read is a mutex and a clone.
///
/// The poll redraws ONLY when the version moved, so an idle app pays one mutex
/// and one comparison every 300 ms and not a frame.
pub struct Watch {
    offline: Option<Offline>,
    snapshot: Snapshot,
    painted: u64,
}

impl Watch {
    /// Point the process's worker at this session and follow it.
    ///
    /// `Offline::ensure` returns the process's own service, so a screen that
    /// opens after a login finds the worker the shell already started
    /// (`AppState::on_restored`/`on_logged_in`) and never starts a second one.
    pub fn attach<V: Render + 'static>(state: &Entity<AppState>, cx: &mut Context<V>) -> Self {
        let offline = offline::attach(state.read(cx));
        let watched = offline.clone();
        let this: WeakEntity<V> = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let mut painted = 0u64;
            loop {
                cx.background_executor().timer(POLL).await;
                let version = watched
                    .as_ref()
                    .map(|service| service.snapshot().version)
                    .unwrap_or(0);
                if version == painted {
                    continue;
                }
                painted = version;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            offline,
            snapshot: Snapshot::default(),
            painted: 0,
        }
    }

    /// The same watch around a service the caller already holds: a test draws a
    /// screen with its own worker instead of the process's, and an app that
    /// started the worker itself does not look it up twice.
    pub fn with_service<V: Render + 'static>(
        offline: Option<Offline>,
        cx: &mut Context<V>,
    ) -> Self {
        let watched = offline.clone();
        let this: WeakEntity<V> = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let mut painted = 0u64;
            loop {
                cx.background_executor().timer(POLL).await;
                let version = watched
                    .as_ref()
                    .map(|service| service.snapshot().version)
                    .unwrap_or(0);
                if version == painted {
                    continue;
                }
                painted = version;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            offline,
            snapshot: Snapshot::default(),
            painted: 0,
        }
    }

    /// The snapshot, re-read when the worker published a new version.
    pub fn get(&mut self) -> &Snapshot {
        if let Some(service) = &self.offline {
            let version = service.snapshot().version;
            if version != self.painted {
                self.snapshot = service.snapshot();
                self.painted = version;
            }
        }
        &self.snapshot
    }

    /// The service, for the screen's own click handlers.
    pub fn service(&self) -> Option<&Offline> {
        self.offline.as_ref()
    }

    /// `false` when no session or no local store: there is nothing to download
    /// into, and the screen says so rather than offering a dead button.
    pub fn available(&self) -> bool {
        self.offline.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wiring::offline::{Failure, Labels, Running};
    use nz_cache::DownloadProgress;
    use std::collections::HashMap;

    fn set_row(kind: &str, ident: &str, title: &str, tracks: usize, done: usize) -> DownloadSet {
        DownloadSet {
            kind: kind.to_owned(),
            ident: ident.to_owned(),
            title: title.to_owned(),
            added_at: 0,
            tracks,
            done,
        }
    }

    fn snap(sets: Vec<DownloadSet>) -> Snapshot {
        Snapshot {
            version: 1,
            sets,
            labels: HashMap::<String, Labels>::new(),
            progress: DownloadProgress::default(),
            ..Snapshot::default()
        }
    }

    fn ids(tracks: &[&str]) -> Vec<String> {
        tracks.iter().map(|track| (*track).to_owned()).collect()
    }

    #[test]
    fn nothing_asked_is_the_download_verb() {
        let snapshot = snap(Vec::new());
        assert_eq!(
            collection(&snapshot, "album", "42", &[]),
            State::Idle,
            "an album nobody asked for offers the verb"
        );
        assert_eq!(State::Idle.label(), "Télécharger");
        assert!(!State::Idle.busy());
        assert!(!State::Idle.on_device());
    }

    #[test]
    fn a_collection_walks_queued_running_done_failed() {
        let tracks = ids(&["a", "b", "c"]);

        let queued = snap(vec![set_row("album", "42", "Un album", 3, 1)]);
        assert_eq!(
            collection(&queued, "album", "42", &tracks),
            State::Queued { done: 1, total: 3 }
        );

        let mut running = snap(vec![set_row("album", "42", "Un album", 3, 1)]);
        running.running = Some(Running {
            track: "b".to_owned(),
            title: "B".to_owned(),
            quality: nz_api::Quality::Opus320,
            bytes: 512_000,
            total: Some(1_024_000),
        });
        assert_eq!(
            collection(&running, "album", "42", &tracks),
            State::Running {
                done: 1,
                total: 3,
                bytes: 512_000,
                size: Some(1_024_000)
            },
            "the set the track in flight belongs to is the one that reads « en cours »"
        );
        // Another collection is only WAITING, however busy the worker is.
        let other = snap(vec![set_row("playlist", "7", "Une liste", 2, 0)]);
        assert_eq!(
            collection(&other, "playlist", "7", &ids(&["x", "y"])),
            State::Queued { done: 0, total: 2 }
        );

        let done = snap(vec![set_row("album", "42", "Un album", 3, 3)]);
        assert_eq!(
            collection(&done, "album", "42", &tracks),
            State::Done { tracks: 3 }
        );

        let mut failed = snap(vec![set_row("album", "42", "Un album", 3, 2)]);
        failed.failures = vec![Failure {
            track: "c".to_owned(),
            title: "C".to_owned(),
            cause: "HTTP 404 : introuvable".to_owned(),
        }];
        let state = collection(&failed, "album", "42", &tracks);
        assert_eq!(
            state,
            State::Failed {
                cause: "HTTP 404 : introuvable".to_owned(),
                failed: 1,
                done: 2
            }
        );
        assert!(
            state.label().contains("HTTP 404"),
            "a failure prints its CAUSE, never a bare count: {}",
            state.label()
        );
        // A failure of ANOTHER collection does not blame this one.
        assert_eq!(
            collection(&failed, "album", "42", &ids(&["x"])),
            State::Queued { done: 2, total: 3 }
        );
    }

    #[test]
    fn a_track_is_its_own_set_and_says_so() {
        let mut snapshot = snap(vec![set_row("track", "a", "A", 1, 0)]);
        assert_eq!(track(&snapshot, "a"), State::Queued { done: 0, total: 1 });
        assert_eq!(track(&snapshot, "b"), State::Idle);

        snapshot.running = Some(Running {
            track: "a".to_owned(),
            title: "A".to_owned(),
            quality: nz_api::Quality::Opus64,
            bytes: 10,
            total: None,
        });
        assert!(matches!(track(&snapshot, "a"), State::Running { .. }));

        snapshot.failures = vec![Failure {
            track: "a".to_owned(),
            title: "A".to_owned(),
            cause: "disque plein".to_owned(),
        }];
        assert_eq!(
            track(&snapshot, "a"),
            State::Failed {
                cause: "disque plein".to_owned(),
                failed: 1,
                done: 0
            }
        );

        // A published track answers from `Snapshot.downloaded`, whatever set
        // brought it in — a collection's member included, which is the whole
        // point of the field.
        let mut published = snap(vec![set_row("album", "9", "Un album", 3, 3)]);
        published.downloaded = Arc::new(HashSet::from(["a".to_owned()]));
        assert_eq!(
            track(&published, "a"),
            State::Done { tracks: 1 },
            "a row of a downloaded ALBUM is downloaded too"
        );
        assert_eq!(track(&published, "b"), State::Idle);
        let episode = snap(vec![set_row("show", "e-1", "Un épisode", 1, 1)]);
        assert_eq!(track(&episode, "e-1"), State::Idle);
    }

    #[test]
    fn a_bar_only_exists_when_something_moves() {
        assert_eq!(State::Idle.fraction(), None);
        assert_eq!(State::Queued { done: 1, total: 4 }.fraction(), Some(0.25));
        assert_eq!(State::Done { tracks: 2 }.fraction(), Some(1.0));
        assert!(!State::Done { tracks: 1 }.busy());
        assert!(
            State::Running {
                done: 0,
                total: 1,
                bytes: 1,
                size: None
            }
            .busy()
        );
        // A run that ends on a failure reads how much DID land.
        assert_eq!(
            State::Failed {
                cause: "x".to_owned(),
                failed: 2,
                done: 6
            }
            .fraction(),
            Some(0.75)
        );
    }

    /// An idle row draws nothing at all; a state draws a glyph. A caller can
    /// therefore leave the chip in every row of every list.
    #[gpui::test]
    async fn an_idle_row_draws_nothing_and_a_downloaded_one_draws_a_glyph(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_omarchy::init);
        cx.update(|cx| {
            assert!(chip(&State::Idle, cx).is_none());
            for state in [
                State::Queued { done: 0, total: 1 },
                State::Running {
                    done: 0,
                    total: 1,
                    bytes: 0,
                    size: None,
                },
                State::Done { tracks: 1 },
                State::Failed {
                    cause: "x".to_owned(),
                    failed: 1,
                    done: 0,
                },
            ] {
                assert!(
                    chip(&state, cx).is_some(),
                    "« {} » paints a chip",
                    state.label()
                );
            }
        });
    }

    /// Every glyph this module draws is in the shell's registered bundle: a
    /// glyph outside it renders nothing and keeps its hit box (D7's pitfall).
    #[test]
    fn every_glyph_the_chip_draws_is_in_the_shell_bundle() {
        use gpui_kit::AssetSource as _;
        for state in [
            State::Idle,
            State::Queued { done: 0, total: 1 },
            State::Running {
                done: 0,
                total: 1,
                bytes: 0,
                size: None,
            },
            State::Done { tracks: 1 },
            State::Failed {
                cause: "x".to_owned(),
                failed: 1,
                done: 0,
            },
        ] {
            let path = state.glyph().path();
            assert!(
                crate::ShellAssets
                    .load(&path)
                    .is_ok_and(|bytes| bytes.is_some_and(|b| !b.is_empty())),
                "« {} » draws {path}, which the shell's bundle does not carry",
                state.label()
            );
        }
    }

    /// **What a list keeps is its model, and the model is three small maps.**
    ///
    /// A 4 000-row playlist pays one `Rows` per published version, and a version
    /// is published when a track lands — never per row and never per frame. The
    /// bytes are COUNTED on this thread (`testmem`), not read from RSS.
    #[test]
    fn a_four_thousand_row_list_keeps_a_few_kilobytes_and_not_its_rows() {
        use crate::testmem::Meter;
        // The common case: one collection, one track in flight, two failures.
        let mut snapshot = snap(vec![set_row("playlist", "7", "Une liste", 4_000, 1_200)]);
        snapshot.running = Some(Running {
            track: "row-7".to_owned(),
            title: "Titre 7".to_owned(),
            quality: nz_api::Quality::Opus320,
            bytes: 1,
            total: Some(2),
        });
        snapshot.failures = (0..2)
            .map(|index| Failure {
                track: format!("row-{index}"),
                title: format!("Titre {index}"),
                cause: "HTTP 404 : introuvable".to_owned(),
            })
            .collect();
        let mark = Meter::now();
        let rows = Rows::build(&snapshot);
        let collection = mark.since();
        println!("one collection of 4 000 titles, 1 in flight, 2 failures: {collection} B");
        assert!(
            collection < 4_096,
            "a collection's rows cost {collection} B, not the 4 000 rows"
        );
        assert!(matches!(rows.state("row-0"), State::Failed { .. }));
        assert!(matches!(rows.state("row-7"), State::Running { .. }));
        drop(rows);

        // The worst case the snapshot can express: 4 000 titles chosen ONE BY
        // ONE (each is its own set), which is what the queued set holds.
        let singles: Vec<DownloadSet> = (0..4_000)
            .map(|index| set_row("track", &format!("row-{index}"), "Titre", 1, 0))
            .collect();
        let snapshot = snap(singles);
        let mark = Meter::now();
        let rows = Rows::build(&snapshot);
        let bytes = mark.since();
        println!("4 000 single-track sets: {bytes} B");
        assert!(
            bytes < 400 * 1_024,
            "4 000 single-track sets cost {bytes} B of row state"
        );
        assert!(matches!(rows.state("row-3999"), State::Queued { .. }));
        drop(rows);

        // The snapshot's published set is an `Arc`: a version's `Rows` clones a
        // POINTER, whatever the device holds.
        let mut published = snap(Vec::new());
        published.downloaded = Arc::new((0..4_000).map(|i| format!("row-{i}")).collect());
        let mark = Meter::now();
        let rows = Rows::build(&published);
        let bytes = mark.since();
        println!("a Rows over 4 000 published tracks: {bytes} B");
        assert!(
            bytes < 1_024,
            "the published set must not be copied per version: {bytes} B"
        );
        assert_eq!(rows.state("row-3999"), State::Done { tracks: 1 });
    }

    /// The labels a person reads: no emoji, no bare number, French.
    #[test]
    fn the_labels_say_what_they_mean() {
        assert_eq!(State::Done { tracks: 1 }.label(), "Téléchargé");
        assert_eq!(State::Done { tracks: 3 }.label(), "Téléchargé · 3 titres");
        assert_eq!(
            State::Queued { done: 2, total: 9 }.label(),
            "En file · 2 / 9"
        );
        assert_eq!(
            State::Failed {
                cause: "réseau coupé".to_owned(),
                failed: 2,
                done: 1
            }
            .label(),
            "2 échecs · 1 téléchargé · réseau coupé"
        );
        // Every state has a sentence, and no glyph comes from an emoji: the
        // shell draws its own (rule 5).
        for state in [
            State::Idle,
            State::Queued { done: 0, total: 1 },
            State::Running {
                done: 0,
                total: 1,
                bytes: 0,
                size: None,
            },
            State::Done { tracks: 1 },
            State::Failed {
                cause: "x".to_owned(),
                failed: 1,
                done: 0,
            },
        ] {
            let label = state.label();
            assert!(!label.is_empty(), "every state has a label");
            assert!(
                !label
                    .chars()
                    .any(|c| ('\u{1F300}'..='\u{1FAFF}').contains(&c)),
                "no emoji in « {label} »"
            );
        }
    }
}
