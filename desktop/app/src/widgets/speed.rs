//! La vitesse d'un podcast, portée de `lib/speed.js` et `SpeedButton.svelte`.
//!
//! The rules the SPA pinned, kept here because each one was a bug before it:
//!
//! - **Per show, not per episode.** The key is the episode's `channel_id`, with
//!   the album's Deezer id as the fallback an old queue row still needs
//!   (`speed_key`). A stored value is believed only when it is a rate the
//!   engine can play (`sane`), so a hand-edited setting cannot make the sound
//!   invalid.
//! - **Music is never touched.** `speed_for` answers 1 for anything that is not
//!   an episode, and `nz-audio` re-checks it on its own (`TrackMeta::podcast`
//!   decides, not the app's current setting): nobody wants a song at 1.5×.
//! - **Pitch is kept.** `signalsmith-stretch` inside the engine is a
//!   phase-vocoder, not a resampler (`docs/agents/playback.md`).
//! - **The memory is the device's**: `podcast.speeds` in `nz-cache`'s `kv`, the
//!   SPA's own store name. It is applied when an episode becomes current, and
//!   a listen party would lock it at 1× (the SPA's `partyHost` guard) — the
//!   party lots call [`set_rate`] with the lock in place.
//!
//! Measured (`cargo test -p neezer-desktop`): the label is French (`1,25×`),
//! `1` is stored by *deleting* the show's key (the SPA's rule, so a map never
//! grows with 1.0 entries), and an unknown rate reads as 1.

use std::collections::BTreeMap;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px, rems,
};
use gpui_omarchy::{IconName, icon};
use nz_player::Entry;

use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;

/// The rates the SPA offers (`lib/speed.js#SPEEDS`).
pub const SPEEDS: [f64; 8] = [0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];

/// The `kv` key the map lives under — the SPA's store name.
pub const SPEEDS_KEY: &str = "podcast.speeds";

/// The key a show's speed is kept under: `channel_id`, else the album's Deezer
/// id (an old queue row may lack the channel), and `None` for music.
pub fn speed_key(entry: &Entry) -> Option<String> {
    if !entry.is_podcast() {
        return None;
    }
    entry
        .channel_id
        .clone()
        .filter(|key| !key.is_empty())
        .or_else(|| {
            entry
                .album
                .as_ref()
                .and_then(|album| album.deezer_id.clone())
                .filter(|key| !key.is_empty())
        })
}

/// A stored value is only believed when the engine can play it (`sane`).
pub fn sane(rate: f64) -> f64 {
    if rate.is_finite() && (0.5..=4.0).contains(&rate) {
        rate
    } else {
        1.0
    }
}

/// The rate for `entry`, given the stored map.
pub fn speed_for(entry: &Entry, map: &BTreeMap<String, f64>) -> f64 {
    match speed_key(entry) {
        None => 1.0,
        Some(key) => sane(map.get(&key).copied().unwrap_or(1.0)),
    }
}

/// "1,25×", "1×": French decimal comma, the SPA's `speedLabel`.
pub fn label(rate: f64) -> String {
    let trimmed = (rate * 100.0).round() / 100.0;
    format!("{}×", format!("{trimmed}").replace('.', ","))
}

/// Load the map from the device.
pub fn stored(state: &AppState) -> BTreeMap<String, f64> {
    state
        .cache
        .as_ref()
        .and_then(|cache| cache.kv().get::<BTreeMap<String, f64>>(SPEEDS_KEY))
        .unwrap_or_default()
}

fn save(state: &AppState, map: &BTreeMap<String, f64>) {
    if let Some(cache) = &state.cache
        && let Err(error) = cache.kv().set(SPEEDS_KEY, map)
    {
        tracing::warn!(%error, "la vitesse du podcast n'a pas pu être enregistrée");
    }
}

/// Remember `rate` for the show `entry` belongs to. `1` forgets it, as the SPA
/// does: a map of neutral entries is a map that never stops growing.
pub fn set_rate(state: &mut AppState, entry: &Entry, rate: f64) {
    let Some(key) = speed_key(entry) else {
        return;
    };
    let rate = sane(rate);
    let mut map = stored(state);
    if (rate - 1.0).abs() < f64::EPSILON {
        map.remove(&key);
    } else {
        map.insert(key, rate);
    }
    save(state, &map);
}

/// The rate the current entry should play at.
pub fn rate_for(state: &AppState, entry: &Entry) -> f64 {
    speed_for(entry, &stored(state))
}

/// Hand the rate to the engine. `nz-audio` still ignores it for music, so this
/// is safe to call on every track.
pub fn apply(state: &AppState, entry: Option<&Entry>) {
    let Some(audio) = &state.audio else {
        return;
    };
    let rate = entry.map(|entry| rate_for(state, entry)).unwrap_or(1.0);
    audio.engine.set_speed(rate as f32);
}

/// Watch the queue's current episode and keep the engine's rate on the show's
/// own setting. Started once per process from the now-playing screen; detached,
/// so an episode that starts after the person left that screen still gets its
/// show's speed.
pub fn follow(state: Entity<AppState>, cx: &mut Context<SpeedChip>) {
    if FOLLOWING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    cx.spawn(async move |weak, cx| {
        let mut last: Option<String> = None;
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(500))
                .await;
            let alive = weak.update(cx, |_this, cx| {
                let (reference, entry) = {
                    let state = state.read(cx);
                    (
                        state
                            .player
                            .current()
                            .and_then(|entry| entry.reference().map(str::to_owned)),
                        state.player.current().cloned(),
                    )
                };
                if reference == last {
                    return;
                }
                last = reference;
                let state = state.read(cx);
                apply(state, entry.as_ref());
            });
            if alive.is_err() {
                break;
            }
        }
    })
    .detach();
}

static FOLLOWING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The speed chip and its drop-up list, as `SpeedButton.svelte` draws it.
pub struct SpeedChip {
    pub state: Entity<AppState>,
    open: bool,
    _subscription: Subscription,
}

impl SpeedChip {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        let this = Self {
            state,
            open: false,
            _subscription: subscription,
        };
        follow(this.state.clone(), cx);
        this
    }

    fn pick(&mut self, rate: f64, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            let Some(entry) = state.player.current().cloned() else {
                return;
            };
            set_rate(state, &entry, rate);
            apply(state, Some(&entry));
            state
                .toasts
                .push(ToastKind::Info, format!("Vitesse : {}", label(sane(rate))));
            cx.notify();
        });
        self.open = false;
        cx.notify();
    }
}

impl Render for SpeedChip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Only an episode has a speed: nobody wants a song at 1.5×.
        let (episode, current) = {
            let state = self.state.read(cx);
            let entry = state.player.current().cloned();
            match entry {
                Some(entry) if entry.is_podcast() => {
                    let rate = rate_for(state, &entry);
                    (true, rate)
                }
                _ => (false, 1.0),
            }
        };
        let mut chip = div().id("speed-chip").relative().flex().items_center();
        if !episode {
            return chip;
        }
        let trigger = div()
            .id("speed-trigger")
            .flex()
            .items_center()
            .justify_center()
            .h(px(28.))
            .px(rems(0.5))
            .border_1()
            .border_color(if (current - 1.0).abs() > f64::EPSILON {
                theme::accent(cx)
            } else {
                theme::border(cx)
            })
            .text_size(rems(0.75))
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .text_color(theme::text(cx))
            .cursor_pointer()
            .hover(|style| style.bg(theme::selected(cx)))
            .on_click(cx.listener(|this, _, _, cx| {
                this.open = !this.open;
                cx.notify();
            }))
            .child(label(current));
        chip = chip.child(trigger);
        if self.open {
            let mut menu = div()
                .id("speed-list")
                .absolute()
                .bottom(px(34.))
                .right_0()
                .w(rems(11.))
                .flex()
                .flex_col()
                .gap(rems(0.125))
                .p(rems(0.375))
                .border_1()
                .border_color(theme::border(cx))
                .bg(theme::chrome(cx))
                .shadow_lg()
                .child(
                    div()
                        .px(rems(0.5))
                        .py(rems(0.25))
                        .text_size(rems(0.6875))
                        .font_weight(gpui_kit::FontWeight::BOLD)
                        .text_color(theme::secondary(cx))
                        .child("Vitesse de lecture"),
                );
            for rate in SPEEDS {
                let selected = (rate - current).abs() < 1e-9;
                menu = menu.child(
                    div()
                        .id(("speed", (rate * 100.0) as usize))
                        .flex()
                        .items_center()
                        .justify_between()
                        .h(rems(1.75))
                        .px(rems(0.5))
                        .text_size(rems(0.8125))
                        .text_color(if selected {
                            theme::accent(cx)
                        } else {
                            theme::text(cx)
                        })
                        .cursor_pointer()
                        .hover(|style| style.bg(theme::selected(cx)))
                        .on_click(cx.listener(move |this, _, _, cx| this.pick(rate, cx)))
                        .child(if (rate - 1.0).abs() < f64::EPSILON {
                            "Normale".to_owned()
                        } else {
                            label(rate)
                        })
                        .when(selected, |row| {
                            row.child(
                                icon(IconName::Check)
                                    .size(rems(0.875))
                                    .text_color(theme::accent(cx)),
                            )
                        }),
                );
            }
            chip = chip.child(menu);
        }
        chip
    }
}

/// Forget the watcher between two tests of the process-wide flag.
#[cfg(test)]
pub fn reset_for_test() {
    FOLLOWING.store(false, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_player::EntryAlbum;

    fn episode(channel: Option<&str>, album: Option<&str>) -> Entry {
        Entry {
            id: "ep".into(),
            title: "Épisode".into(),
            podcast: Some(true),
            channel_id: channel.map(str::to_owned),
            album: album.map(|id| EntryAlbum {
                id: None,
                deezer_id: Some(id.to_owned()),
                title: "Émission".into(),
                cover: None,
            }),
            ..Entry::default()
        }
    }

    #[test]
    fn the_show_is_the_key_and_music_has_none() {
        assert_eq!(
            speed_key(&episode(Some("42"), Some("9"))),
            Some("42".into())
        );
        // An old queue row without a channel still finds its show.
        assert_eq!(speed_key(&episode(None, Some("9"))), Some("9".into()));
        assert_eq!(speed_key(&episode(None, None)), None);
        let song = Entry {
            id: "t".into(),
            title: "Titre".into(),
            ..Entry::default()
        };
        assert_eq!(speed_key(&song), None);
        assert_eq!(speed_for(&song, &BTreeMap::new()), 1.0);
    }

    #[test]
    fn only_a_rate_the_engine_can_play_is_believed() {
        assert_eq!(sane(1.5), 1.5);
        assert_eq!(sane(0.4), 1.0);
        assert_eq!(sane(4.1), 1.0);
        assert_eq!(sane(f64::NAN), 1.0);
        let mut map = BTreeMap::new();
        map.insert("42".to_owned(), 9.0);
        assert_eq!(speed_for(&episode(Some("42"), None), &map), 1.0);
        map.insert("42".to_owned(), 1.25);
        assert_eq!(speed_for(&episode(Some("42"), None), &map), 1.25);
    }

    #[test]
    fn the_label_is_french() {
        assert_eq!(label(1.0), "1×");
        assert_eq!(label(1.25), "1,25×");
        assert_eq!(label(0.75), "0,75×");
        assert_eq!(label(2.0), "2×");
    }
}
