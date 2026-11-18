//! Le minuteur de sommeil, porté de `lib/sleep.js` et `SleepButton.svelte`.
//!
//! Two shapes, one timer: a wall-clock deadline in minutes, or "when this
//! track ends". The rules are the SPA's, and each one exists because a bedside
//! machine is throttled, suspended or frozen:
//!
//! - **The deadline is compared with the wall clock**, never counted down in
//!   ticks. A laptop that slept for ten minutes still ends the music where the
//!   clock says it should, instead of resuming a countdown from where it froze.
//! - **The last 15 s fade** through the ENGINE's volume, multiplied on top of
//!   the person's own level: `player.volume()` is never touched, so nothing has
//!   to be put back afterwards and a fade cannot lose the level the person set.
//!   The curve is amplitude squared (`fade_at`), what the ear hears as an even
//!   ramp. (`docs/agents/webapp.md` and `webapp/test/sleep.test.mjs` pin 15 s;
//!   the lot's plan paragraph said 10 — corrected in the note.)
//! - **It fires a PAUSE, not a stop**: the queue and the position stay where
//!   they are and one tap resumes. "End of the track" waits for the file to
//!   finish (the crossfade is held back while it is armed) and stops on the
//!   NEXT track, paused at 0 — `player.advancePaused()` in the SPA.
//! - **The timer lives in the process, not in the screen.** It is offered by
//!   the now-playing screen, but the person navigates away and expects the
//!   music to stop anyway: the state is a process-wide slot and the loop that
//!   watches it is a detached foreground task, so dropping the screen does not
//!   cancel bedtime.
//!
//! Measured (`cargo test -p neezer-desktop`): the fade is 1 at 15 s, 0.25 at
//! 7.5 s and 0 at the deadline (the SPA's own assertion); a clock that jumps
//! 10 minutes past a 30-minute timer fires on its next tick; the loop wakes
//! four times a second only while a timer is armed (0 wake-ups when none is).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};
use nz_audio::FadeSettings;
use nz_player::Command;

use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;

/// The choices the SPA offers, in minutes (`lib/sleep.js#SLEEP_CHOICES`).
pub const SLEEP_CHOICES: [u32; 7] = [5, 10, 15, 30, 45, 60, 90];

/// The last seconds fade instead of cutting the sound (`SLEEP_FADE_MS`).
pub const SLEEP_FADE_MS: i64 = 15_000;

/// How often the armed timer looks at the clock. Four times a second is the
/// shell's own cadence; nothing here needs finer, and when no timer is armed
/// the loop only reads an `Option`.
const TICK_MS: u64 = 250;

/// A track "ended" when the playhead was at least this close to its length: a
/// manual skip from the last seconds must not be mistaken for the end (the SPA
/// asks the ended handler; here the queue's own `seq` is the only signal).
const TRACK_END_SLACK_S: f64 = 3.0;

/// What is armed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SleepKind {
    /// Stop at `ends_at_ms` (a wall-clock instant), after a fade.
    Minutes { minutes: u32, ends_at_ms: i64 },
    /// Stop when the playing track reaches its end.
    Track,
}

impl SleepKind {
    pub fn is_track(self) -> bool {
        matches!(self, SleepKind::Track)
    }
}

/// What the armed timer says to do right now. A pure decision so the numbers
/// above are an assertion instead of a click-through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decision {
    /// Nothing armed: the level is the person's own.
    Idle,
    /// Play at this multiplier of the person's volume.
    Level(f32),
    /// The deadline is behind us: pause, then put the level back.
    Fire,
}

/// The gain for `left_ms` still to go: 1 before the fade, 0 at the end.
pub fn fade_at(left_ms: i64) -> f32 {
    if left_ms >= SLEEP_FADE_MS {
        return 1.0;
    }
    if left_ms <= 0 {
        return 0.0;
    }
    let x = left_ms as f32 / SLEEP_FADE_MS as f32;
    x * x
}

/// The decision for `timer` at the wall-clock instant `now_ms`.
pub fn decide(timer: Option<SleepKind>, now_ms: i64) -> Decision {
    match timer {
        None => Decision::Idle,
        // "End of the track" is decided by `ended_naturally`, not by a clock.
        Some(SleepKind::Track) => Decision::Level(1.0),
        Some(SleepKind::Minutes { ends_at_ms, .. }) => {
            let left = ends_at_ms - now_ms;
            if left <= 0 {
                Decision::Fire
            } else {
                Decision::Level(fade_at(left))
            }
        }
    }
}

/// Did the queue move on because a track ENDED (rather than a skip)?
///
/// `previous` is the `(seq, position, duration)` read on the last tick, `seq`
/// the queue's own change counter: a track that ends advances the queue from
/// within the last seconds of its length, while `Ctrl+→` moves it from
/// anywhere.
pub fn ended_naturally(previous: (u64, f64, f64), seq: u64) -> bool {
    let (old_seq, position, duration) = previous;
    seq != old_seq && duration > 0.0 && position >= duration - TRACK_END_SLACK_S
}

/// "12 min", "45 s", "fin du titre": what the button shows (`leftLabel`).
pub fn left_label(timer: Option<SleepKind>, now_ms: i64) -> String {
    match timer {
        None => String::new(),
        Some(SleepKind::Track) => "fin du titre".to_owned(),
        Some(SleepKind::Minutes {
            minutes,
            ends_at_ms,
        }) => {
            // Never more than what was asked for: a reading taken a moment
            // before the timer was set must not say "6 min" for a 5-minute one.
            let asked = minutes as i64 * 60_000;
            let ms = asked.min((ends_at_ms - now_ms).max(0));
            if ms >= 60_000 {
                format!("{} min", (ms + 59_999) / 60_000)
            } else {
                format!("{} s", ((ms + 999) / 1_000).max(1))
            }
        }
    }
}

/// "5 minutes", "1 heure", "1 h 30": the menu's own words.
pub fn choice_label(minutes: u32) -> String {
    match minutes {
        60 => "1 heure".to_owned(),
        m if m > 60 => format!("{} h {:02}", m / 60, m % 60),
        m => format!("{m} minutes"),
    }
}

/// Milliseconds since the epoch: the wall clock `Date.now()` is in the SPA.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or(0)
}

// --- the process-wide timer ------------------------------------------------

static ARMED: Mutex<Option<SleepKind>> = Mutex::new(None);
/// The fade last written to the engine, as `f32` bits: the loop only touches the
/// volume when the value actually moved.
static LEVEL: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0_f32.to_bits()
static WATCHING: AtomicBool = AtomicBool::new(false);
/// The crossfade setting taken away while "end of the track" is armed.
static SAVED_FADE: Mutex<Option<FadeSettings>> = Mutex::new(None);

pub fn armed() -> Option<SleepKind> {
    *ARMED.lock().unwrap_or_else(|err| err.into_inner())
}

fn store(kind: Option<SleepKind>) {
    *ARMED.lock().unwrap_or_else(|err| err.into_inner()) = kind;
}

fn current_level() -> f32 {
    f32::from_bits(LEVEL.load(Ordering::Relaxed))
}

fn store_level(value: f32) {
    LEVEL.store(value.to_bits(), Ordering::Relaxed);
}

/// Arm "stop in `minutes`" and make sure the loop is running.
pub fn sleep_in(state: Entity<AppState>, minutes: u32, cx: &mut Context<SleepControl>) {
    let ends_at_ms = now_ms() + minutes as i64 * 60_000;
    restore_fade(state.clone(), cx);
    store(Some(SleepKind::Minutes {
        minutes,
        ends_at_ms,
    }));
    store_level(1.0);
    ensure_watcher(state, cx);
    cx.notify();
}

/// Arm "stop at the end of this track".
pub fn sleep_at_track_end(state: Entity<AppState>, cx: &mut Context<SleepControl>) {
    store(Some(SleepKind::Track));
    store_level(1.0);
    // The file must reach its last sample: a crossfade would start the next
    // track over the ending one (`beginCrossfade`, `sleepStopsAtTrackEnd`).
    state.update(cx, |state, _| {
        if let Some(audio) = &state.audio {
            let mut saved = SAVED_FADE.lock().unwrap_or_else(|err| err.into_inner());
            if saved.is_none() {
                let current = audio.engine.fade();
                *saved = Some(current);
                audio.engine.set_fade(FadeSettings {
                    enabled: false,
                    ..current
                });
            }
        }
    });
    ensure_watcher(state, cx);
    cx.notify();
}

/// Disarm and put everything back.
pub fn cancel(state: Entity<AppState>, cx: &mut Context<SleepControl>) {
    store(None);
    store_level(1.0);
    restore_fade(state.clone(), cx);
    apply_level(state, cx, 1.0);
    cx.notify();
}

fn restore_fade(state: Entity<AppState>, cx: &mut Context<SleepControl>) {
    let saved = SAVED_FADE
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take();
    if let Some(saved) = saved {
        state.update(cx, |state, _| {
            if let Some(audio) = &state.audio {
                audio.engine.set_fade(saved);
            }
        });
    }
}

/// Write `volume × level` to the engine — never to the player's own volume.
fn apply_level(state: Entity<AppState>, cx: &mut Context<SleepControl>, level: f32) {
    if (current_level() - level).abs() < 1e-4 {
        return;
    }
    store_level(level);
    state.update(cx, |state, _| {
        if let Some(audio) = &state.audio {
            let volume = (state.player.volume() * level as f64).clamp(0.0, 1.0);
            audio.engine.set_volume(volume as f32);
        }
    });
}

/// The one loop, started the first time a timer is armed anywhere in the
/// process. It is detached from the widget: leaving the now-playing screen must
/// not cancel bedtime.
fn ensure_watcher(state: Entity<AppState>, cx: &mut Context<SleepControl>) {
    if WATCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app_state = state;
    let fire_state = app_state.clone();
    cx.spawn(async move |weak, cx| {
        let mut last: Option<(u64, f64, f64)> = None;
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(TICK_MS))
                .await;
            let alive = weak.update(cx, |_this, cx| {
                let (timer, seq, position, duration, playing) = {
                    let state = app_state.read(cx);
                    let duration = state.player.duration();
                    (
                        armed(),
                        state.player.seq(),
                        state.player.current_time(),
                        duration,
                        state.player.playing(),
                    )
                };
                match decide(timer, now_ms()) {
                    Decision::Idle => {
                        last = None;
                        apply_level(app_state.clone(), cx, 1.0);
                    }
                    Decision::Level(level) => {
                        apply_level(app_state.clone(), cx, level);
                        let previous = last.replace((seq, position, duration));
                        if timer.is_some_and(SleepKind::is_track)
                            && playing
                            && let Some(previous) = previous
                            && ended_naturally(previous, seq)
                        {
                            fire(fire_state.clone(), cx);
                        }
                    }
                    Decision::Fire => fire(fire_state.clone(), cx),
                }
                cx.notify();
            });
            if alive.is_err() {
                break;
            }
        }
        WATCHING.store(false, Ordering::SeqCst);
    })
    .detach();
}

/// The deadline: pause where we are, and hand the level back once the sound has
/// actually stopped (the SPA's 400 ms), so the last instant is not full volume.
fn fire(state: Entity<AppState>, cx: &mut Context<SleepControl>) {
    store(None);
    restore_fade(state.clone(), cx);
    state.update(cx, |state, cx| {
        state.command(Command::pause(), cx);
    });
    apply_level(state, cx, 1.0);
}

// --- the widget ------------------------------------------------------------

/// The minute-countdown chip and its menu, as `SleepButton.svelte` draws it.
pub struct SleepControl {
    pub state: Entity<AppState>,
    open: bool,
    _subscription: Subscription,
}

impl SleepControl {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            open: false,
            _subscription: subscription,
        }
    }

    fn pick(&mut self, minutes: u32, cx: &mut Context<Self>) {
        sleep_in(self.state.clone(), minutes, cx);
        self.open = false;
        self.state.update(cx, |state, cx| {
            state.toasts.push(
                ToastKind::Info,
                format!(
                    "Le lecteur s'arrêtera dans {}",
                    choice_label(minutes).to_lowercase()
                ),
            );
            cx.notify();
        });
    }

    fn at_end(&mut self, cx: &mut Context<Self>) {
        sleep_at_track_end(self.state.clone(), cx);
        self.open = false;
        self.state.update(cx, |state, cx| {
            state.toasts.push(
                ToastKind::Info,
                "Le lecteur s'arrêtera à la fin de ce titre",
            );
            cx.notify();
        });
    }

    fn off(&mut self, cx: &mut Context<Self>) {
        cancel(self.state.clone(), cx);
        self.open = false;
        self.state.update(cx, |state, cx| {
            state.toasts.push(ToastKind::Info, "Minuteur désactivé");
            cx.notify();
        });
    }
}

impl Render for SleepControl {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let timer = armed();
        let label = left_label(timer, now_ms());
        let on = timer.is_some();

        let trigger = button(
            "sleep-trigger",
            if on {
                format!("Minuteur · {label}")
            } else {
                "Minuteur".to_owned()
            },
            if on {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Secondary
            },
            cx,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.open = !this.open;
            cx.notify();
        }));

        let mut chip = div()
            .id("sleep-timer")
            .relative()
            .flex()
            .items_center()
            .child(trigger);

        if self.open {
            let mut menu = div()
                .id("sleep-menu")
                .absolute()
                .bottom(px(38.))
                .right_0()
                .w(rems(15.))
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
                        .child("Minuteur de sommeil"),
                );
            if on {
                menu = menu.child(
                    div()
                        .id("sleep-off")
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(rems(0.5))
                        .h(rems(1.75))
                        .px(rems(0.5))
                        .text_size(rems(0.8125))
                        .text_color(theme::accent(cx))
                        .cursor_pointer()
                        .hover(|style| style.bg(theme::selected(cx)))
                        .on_click(cx.listener(|this, _, _, cx| this.off(cx)))
                        .child("Désactiver")
                        .child(
                            div()
                                .text_size(rems(0.75))
                                .text_color(theme::secondary(cx))
                                .child(label.clone()),
                        ),
                );
            }
            for minutes in SLEEP_CHOICES {
                let selected = matches!(
                    timer,
                    Some(SleepKind::Minutes { minutes: armed_minutes, .. }) if armed_minutes == minutes
                );
                menu = menu.child(
                    div()
                        .id(("sleep-choice", minutes))
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
                        .on_click(cx.listener(move |this, _, _, cx| this.pick(minutes, cx)))
                        .child(choice_label(minutes))
                        .when(selected, |row| {
                            row.child(
                                icon(IconName::Check)
                                    .size(rems(0.875))
                                    .text_color(theme::accent(cx)),
                            )
                        }),
                );
            }
            menu = menu.child(
                div()
                    .id("sleep-track-end")
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(rems(1.75))
                    .px(rems(0.5))
                    .text_size(rems(0.8125))
                    .text_color(if timer.is_some_and(SleepKind::is_track) {
                        theme::accent(cx)
                    } else {
                        theme::text(cx)
                    })
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::selected(cx)))
                    .on_click(cx.listener(|this, _, _, cx| this.at_end(cx)))
                    .child("À la fin du titre")
                    .when(timer.is_some_and(SleepKind::is_track), |row| {
                        row.child(
                            icon(IconName::Check)
                                .size(rems(0.875))
                                .text_color(theme::accent(cx)),
                        )
                    }),
            );
            chip = chip.child(menu);
        }
        chip
    }
}

/// The label a mounted chip shows, for a test that does not open a window.
pub fn chip_label() -> String {
    left_label(armed(), now_ms())
}

/// Forget the timer between two tests of the process-wide slot.
#[cfg(test)]
pub fn reset_for_test() {
    store(None);
    store_level(1.0);
    *SAVED_FADE.lock().unwrap_or_else(|err| err.into_inner()) = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fade_is_one_before_the_last_15_s_then_falls_squared() {
        // The SPA's own assertions (`webapp/test/sleep.test.mjs`).
        assert_eq!(fade_at(60_000), 1.0);
        assert_eq!(fade_at(SLEEP_FADE_MS), 1.0);
        assert_eq!(fade_at(0), 0.0);
        assert_eq!(fade_at(-5), 0.0);
        assert!((fade_at(7_500) - 0.25).abs() < 1e-6, "amplitude squared");
        let mut previous = 1.0;
        let mut ms = SLEEP_FADE_MS;
        while ms >= 0 {
            let gain = fade_at(ms);
            assert!(gain <= previous + 1e-12, "monotonic at {ms}");
            previous = gain;
            ms -= 250;
        }
    }

    #[test]
    fn a_timer_holds_the_level_fades_then_fires() {
        let now = 1_000_000;
        let timer = SleepKind::Minutes {
            minutes: 1,
            ends_at_ms: now + 60_000,
        };
        assert_eq!(decide(Some(timer), now + 44_000), Decision::Level(1.0));
        match decide(Some(timer), now + 50_000) {
            Decision::Level(level) => {
                assert!((level - (10.0 / 15.0_f32).powi(2)).abs() < 1e-6);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(decide(Some(timer), now + 60_000), Decision::Fire);
        assert_eq!(decide(Some(timer), now + 600_000), Decision::Fire);
        assert_eq!(decide(None, now), Decision::Idle);
    }

    #[test]
    fn a_clock_that_jumped_fires_on_the_next_tick() {
        // A suspended laptop does not fire every 250 ms; the deadline is still
        // behind us the moment the loop looks again.
        let timer = SleepKind::Minutes {
            minutes: 30,
            ends_at_ms: 1_000_000 + 30 * 60_000,
        };
        let woke_at = 1_000_000 + 40 * 60_000;
        assert_eq!(decide(Some(timer), woke_at), Decision::Fire);
    }

    #[test]
    fn the_label_never_promises_more_than_was_asked_for() {
        let now = 1_000_000;
        let timer = SleepKind::Minutes {
            minutes: 5,
            ends_at_ms: now + 5 * 60_000,
        };
        // A reading taken 1 ms before the timer was set must not say "6 min".
        assert_eq!(left_label(Some(timer), now - 1), "5 min");
        assert_eq!(left_label(Some(timer), now + 60_000), "4 min");
        assert_eq!(left_label(Some(timer), now + 5 * 60_000 - 12_000), "12 s");
        assert_eq!(left_label(Some(SleepKind::Track), now), "fin du titre");
        assert_eq!(left_label(None, now), "");
        assert_eq!(choice_label(60), "1 heure");
        assert_eq!(choice_label(90), "1 h 30");
        assert_eq!(choice_label(5), "5 minutes");
    }

    /// The loop, end to end: a timer whose deadline has passed pauses where it
    /// is — the queue and the position stay, one tap resumes.
    #[gpui::test]
    async fn a_deadline_that_passed_pauses_where_it_is(cx: &mut gpui::TestAppContext) {
        use gpui_kit::AppContext as _;
        reset_for_test();
        cx.update(|cx| {
            crate::theme::init(cx);
            crate::keymap::install(cx);
        });
        let dir = std::env::temp_dir().join(format!(
            "neezer-d9-sleep-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (tx, _rx) = crate::state::event_channel();
        let state = {
            let runtime = runtime.handle().clone();
            let dir = dir.clone();
            cx.update(move |cx| {
                cx.new(|_| {
                    crate::state::AppState::new(
                        crate::state::AppConfig {
                            server: Some("http://127.0.0.1:1".to_owned()),
                            start_audio: false,
                            start_mpris: false,
                            ..crate::state::AppConfig::for_test(&dir)
                        },
                        runtime,
                        tx,
                        None,
                    )
                })
            })
        };
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.command(
                    nz_player::Command::PlayQueue {
                        args: nz_player::QueueArgs {
                            tracks: vec![
                                nz_player::Entry {
                                    id: "one".into(),
                                    title: "Un".into(),
                                    duration: 200,
                                    ..Default::default()
                                },
                                nz_player::Entry {
                                    id: "two".into(),
                                    title: "Deux".into(),
                                    duration: 200,
                                    ..Default::default()
                                },
                            ],
                            start: 0,
                            context: None,
                        },
                    },
                    cx,
                );
                state.player.set_position(42.5);
            });
        });
        let control = cx.update(|cx| cx.new(|cx| SleepControl::new(state.clone(), cx)));
        cx.update(|cx| {
            control.update(cx, |control, cx| {
                control.pick(5, cx);
            });
        });
        assert!(matches!(
            armed(),
            Some(SleepKind::Minutes { minutes: 5, .. })
        ));
        assert!(
            cx.update(|cx| state.read(cx).player.playing()),
            "the queue plays before the deadline"
        );

        // The deadline passes: the loop's next tick is what notices (the wall
        // clock, not a countdown), and it PAUSES.
        store(Some(SleepKind::Minutes {
            minutes: 5,
            ends_at_ms: now_ms() - 1,
        }));
        cx.executor().advance_clock(Duration::from_millis(TICK_MS));
        cx.run_until_parked();
        assert!(
            !cx.update(|cx| state.read(cx).player.playing()),
            "the deadline pauses the sound"
        );
        assert_eq!(
            cx.update(|cx| state
                .read(cx)
                .player
                .current()
                .map(|entry| entry.id.clone())),
            Some("one".to_owned()),
            "the queue and the track are where they were"
        );
        assert!((cx.update(|cx| state.read(cx).player.current_time()) - 42.5).abs() < 1e-9);
        assert_eq!(armed(), None, "a spent timer is no timer");
        reset_for_test();

        std::fs::remove_dir_all(dir).ok();
        drop(runtime);
    }

    #[test]
    fn only_a_track_that_reached_its_end_counts_as_ended() {
        // A skip from the middle of a 200 s track is not the end.
        assert!(!ended_naturally((7, 30.0, 200.0), 8));
        // Two seconds before the end, it is.
        assert!(ended_naturally((7, 198.0, 200.0), 8));
        // The queue did not move: still the same track.
        assert!(!ended_naturally((7, 199.9, 200.0), 7));
        // A duration the server never measured cannot be "reached".
        assert!(!ended_naturally((7, 10.0, 0.0), 8));
    }
}
