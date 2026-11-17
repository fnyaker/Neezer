//! download_badge: the compact « hors ligne » chip (lot `D13`).
//!
//! One chip, three facts, in the order a person reads them: whether work is
//! running, how much of it is left, and how much is on the device. It is the
//! badge the queue header, a library row or the player bar drops in — the SPA
//! shows the same state in its « Téléchargements » card and on a downloaded
//! row.
//!
//! Two entry points, because two callers need different lifetimes:
//!
//! - [`chip`] is pure: it takes a [`Snapshot`] and draws it. A row that already
//!   redraws for other reasons uses it and owns no timer.
//! - [`Badge`] is an entity with its own 1 s tick, for a place nobody else
//!   repaints (the player bar while nothing plays). It renders the same chip.
//!
//! It draws NOTHING when the person has no downloads and no work — the shell
//! includes it unconditionally, and an idle app shows no badge.
//!
//! Every glyph comes from the shell's registered bundle (`main.rs`'s
//! `icon_assets!`); a glyph outside it renders nothing and keeps its hit box, so
//! `every_glyph_the_badge_draws_is_in_the_shell_bundle` fails CI instead.

use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, div, rems,
};
use gpui_omarchy::{IconName, icon};

use crate::state::AppState;
use crate::theme;
use crate::wiring::offline::{self, Offline, Snapshot, WorkerState, bytes_label};

/// How often the entity form re-reads the published snapshot.
const POLL: Duration = Duration::from_secs(1);

/// The chip for one snapshot: `None` when there is nothing to say.
pub fn chip(snapshot: &Snapshot, cx: &App) -> Option<AnyElement> {
    let downloads = snapshot.progress.done;
    let remaining = snapshot.remaining();
    if downloads == 0 && remaining == 0 {
        return None;
    }

    let (glyph, colour, text) = match &snapshot.state {
        WorkerState::Running | WorkerState::Pausing => (
            IconName::CloudDownload,
            theme::accent(cx),
            format!("{} · {} restants", snapshot.progress.done, remaining),
        ),
        WorkerState::Paused => (
            IconName::CirclePause,
            theme::secondary(cx),
            format!("{} · {remaining} en attente", snapshot.progress.done),
        ),
        WorkerState::Offline => (
            IconName::CloudOff,
            theme::warning(cx),
            format!("{remaining} en attente du réseau"),
        ),
        WorkerState::Blocked(_) => (
            IconName::TriangleAlert,
            theme::danger(cx),
            snapshot.state.label().to_owned(),
        ),
        WorkerState::Unavailable | WorkerState::Idle => (
            IconName::CircleCheck,
            theme::secondary(cx),
            format!(
                "Hors ligne · {} titre{} · {}",
                downloads,
                if downloads > 1 { "s" } else { "" },
                bytes_label(snapshot.progress.bytes.max(0) as u64)
            ),
        ),
    };

    let idle = matches!(snapshot.state, WorkerState::Idle | WorkerState::Unavailable);
    let busy = remaining > 0;
    let element = div()
        .id("download-badge")
        .flex()
        .items_center()
        .gap(rems(0.375))
        .px(rems(0.375))
        .py(rems(0.125))
        // An idle badge is a state, not a control: no border, no chrome, and it
        // never competes with the transport for the eye.
        .when(busy || !idle, |chip| {
            chip.border_1().border_color(theme::border(cx))
        })
        .text_size(rems(0.625))
        .font_family(theme::font(cx))
        .text_color(colour)
        .child(icon(glyph).size(rems(0.75)))
        .child(div().child(SharedString::from(text)));
    Some(element.into_any_element())
}

/// The ticking form: the same chip, refreshed once a second on its own.
pub struct Badge {
    offline: Option<Offline>,
    snapshot: Snapshot,
    _state: Subscription,
}

impl Badge {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let offline = offline::attach(state.read(cx));
        let snapshot = offline.as_ref().map(Offline::snapshot).unwrap_or_default();
        let _state = cx.observe(&state, |this, state, cx| {
            this.offline = offline::attach(state.read(cx));
            cx.notify();
        });
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let alive = this.update(cx, |this, cx| {
                    let Some(offline) = &this.offline else {
                        return;
                    };
                    let snapshot = offline.snapshot();
                    if snapshot.version != this.snapshot.version {
                        this.snapshot = snapshot;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            offline,
            snapshot,
            _state,
        }
    }
}

impl Render for Badge {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        match chip(&self.snapshot, cx) {
            Some(chip) => chip,
            None => div().id("download-badge-empty").into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_cache::DownloadProgress;

    fn snapshot(done: usize, pending: usize, state: WorkerState) -> Snapshot {
        Snapshot {
            state,
            progress: DownloadProgress {
                done,
                pending,
                ..DownloadProgress::default()
            },
            ..Snapshot::default()
        }
    }

    /// Nothing downloaded, nothing queued: the badge draws nothing at all, so a
    /// caller can leave it in the layout for ever. A finished download is a
    /// state sentence, not a progress count.
    #[gpui::test]
    async fn an_idle_app_shows_a_badge_only_when_something_is_on_the_device(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(crate::theme::init);
        cx.update(|cx| {
            assert!(
                chip(&snapshot(0, 0, WorkerState::Idle), cx).is_none(),
                "rien à dire : pas de badge"
            );
            assert!(
                chip(&snapshot(3, 0, WorkerState::Idle), cx).is_some(),
                "trois titres hors ligne : le badge le dit"
            );
            assert!(
                chip(&snapshot(2, 40, WorkerState::Running), cx).is_some(),
                "40 titres en attente : le badge les compte"
            );
        });
    }

    /// Every glyph is in the shell's bundle (D7's measured pitfall: a glyph
    /// outside it renders nothing and keeps its hit box).
    #[test]
    fn every_glyph_the_badge_draws_is_in_the_shell_bundle() {
        use gpui_kit::AssetSource as _;
        for icon in [
            IconName::CloudDownload,
            IconName::CirclePause,
            IconName::CloudOff,
            IconName::TriangleAlert,
            IconName::CircleCheck,
        ] {
            let path = icon.path();
            assert!(
                crate::ShellAssets
                    .load(&path)
                    .is_ok_and(|bytes| bytes.is_some_and(|b| !b.is_empty())),
                "{path} is missing from the shell's icon bundle"
            );
        }
    }
}
