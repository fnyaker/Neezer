//! The player bar: always visible, always the same transport.
//!
//! Every control dispatches the action the keymap binds, so a click and a
//! keystroke go through one handler and one `PlayerState::apply`. The bar reads
//! the state it is given and owns no clock: the playhead comes from
//! `AppState::tick`, which reads the mixer's own position four times a second.
//!
//! The progress bar is twenty transparent hit zones over the fill. A zone seeks
//! to its fifth of the track through [`AppState::seek_fraction`] — the same
//! command `Shift+→` produces — rather than inventing a second seek path.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{
    Context, Entity, InteractiveElement, IntoElement, MouseButton, ParentElement, Styled, Window,
    div, px, relative, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon_button};
use nz_player::Repeat;

use crate::actions::{
    CycleRepeat, Mute, NextTrack, PlayPause, PreviousTrack, ToggleFullscreen, ToggleQueue,
    ToggleShuffle, ToggleViz, VolumeDown, VolumeUp,
};
use crate::app::RootView;
use crate::state::AppState;
use crate::theme;
use crate::widgets::cover::{self, Covers, Kind};
use crate::wiring::events::Sender;

/// What the bar's sleeve fetch answers: the 128 px copy of the playing art, or
/// `None` (the slot keeps its disc glyph).
pub enum BarMsg {
    Cover(String, Option<PathBuf>),
}

/// `mm:ss` for a playhead or a length. A duration of zero is `—`, never `0:00`
/// on a row the server has not measured.
pub fn clock(seconds: f64) -> String {
    if !seconds.is_finite() || seconds <= 0.0 {
        return "—".to_owned();
    }
    let total = seconds.round() as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

pub fn render(
    state: Entity<AppState>,
    window: &mut Window,
    covers: &Rc<RefCell<Covers>>,
    cover_events: &Sender<BarMsg>,
    cx: &mut Context<RootView>,
) -> impl IntoElement + use<> {
    // The bar is on every screen, so its sleeve is freed the same way a
    // screen's is: what the last paint declared and this one does not goes
    // here, tile included.
    covers.borrow_mut().begin_frame(window, cx);
    // The playing sleeve, at the size the 44 px slot draws it (`Class::Row`):
    // the archived art is a 1 000 px file, and GPUI keeps a painted image's
    // atlas tile for the life of the window, so the bar asks for the 128 px
    // copy exactly as a queue row does. The `AppState` borrow is scoped: the
    // decode is announced to the executor, which needs `cx` mutably.
    let (cover_url, cover_file) = {
        let snapshot = state.read(cx);
        let url = snapshot
            .player
            .current()
            .and_then(|entry| entry.cover.clone());
        if let Some(url) = url.as_deref() {
            let tx = cover_events.clone();
            let for_message = url.to_owned();
            covers.borrow_mut().want(snapshot, url, move |file| {
                tx.send(BarMsg::Cover(for_message, file));
            });
        }
        let file = url.as_deref().and_then(|url| cover::file_in(covers, url));
        (url, file)
    };
    match &cover_file {
        Some(file) => covers.borrow_mut().show([file.clone()], cx),
        None => covers.borrow_mut().show(Vec::new(), cx),
    }
    let state = state.read(cx);
    let entry = state.player.current();
    let has_track = entry.is_some();
    let title = entry.map(|entry| entry.title.clone()).unwrap_or_default();
    let artist = entry
        .map(|entry| entry.artist_name().to_owned())
        .unwrap_or_default();
    let album = entry
        .and_then(|entry| entry.album.as_ref())
        .map(|album| album.title.clone())
        .unwrap_or_default();
    let playing = state.player.playing();
    let position = state.player.current_time();
    let duration = state.player.duration();
    let fraction = if duration > 0.0 {
        (position / duration).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };

    let mut controls = div().flex().items_center().gap(rems(0.25));
    controls =
        controls
            .child(
                icon_button(
                    "bar-prev",
                    IconName::SkipBack,
                    "Titre précédent",
                    ButtonVariant::Secondary,
                    cx,
                )
                .disabled(!has_track)
                .on_click(cx.listener(|_this, _, window, cx| {
                    window.dispatch_action(Box::new(PreviousTrack), cx)
                })),
            )
            .child(
                icon_button(
                    "bar-play",
                    if playing {
                        IconName::Pause
                    } else {
                        IconName::Play
                    },
                    if playing { "Pause" } else { "Lecture" },
                    ButtonVariant::Primary,
                    cx,
                )
                .disabled(!has_track)
                .on_click(cx.listener(|_this, _, window, cx| {
                    window.dispatch_action(Box::new(PlayPause), cx)
                })),
            )
            .child(
                icon_button(
                    "bar-next",
                    IconName::SkipForward,
                    "Titre suivant",
                    ButtonVariant::Secondary,
                    cx,
                )
                .disabled(!has_track)
                .on_click(cx.listener(|_this, _, window, cx| {
                    window.dispatch_action(Box::new(NextTrack), cx)
                })),
            )
            .child(
                icon_button(
                    "bar-shuffle",
                    IconName::Shuffle,
                    "Lecture aléatoire",
                    ButtonVariant::Secondary,
                    cx,
                )
                .selected(state.player.shuffle())
                .on_click(cx.listener(|_this, _, window, cx| {
                    window.dispatch_action(Box::new(ToggleShuffle), cx)
                })),
            )
            .child(
                icon_button(
                    "bar-repeat",
                    match state.player.repeat() {
                        Repeat::Off => IconName::Repeat,
                        Repeat::All => IconName::Repeat,
                        Repeat::One => IconName::Repeat1,
                    },
                    match state.player.repeat() {
                        Repeat::Off => "Répéter : non",
                        Repeat::All => "Répéter : tout",
                        Repeat::One => "Répéter : un titre",
                    },
                    ButtonVariant::Secondary,
                    cx,
                )
                .selected(state.player.repeat() != Repeat::Off)
                .on_click(cx.listener(|_this, _, window, cx| {
                    window.dispatch_action(Box::new(CycleRepeat), cx)
                })),
            );

    let mut bar = div()
        .id("player-bar")
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(rems(1.))
        .h(rems(4.5))
        .px(rems(1.25))
        .border_t_1()
        .border_color(theme::border(cx))
        .bg(theme::chrome(cx))
        .text_color(theme::text(cx))
        .font_family(theme::font(cx))
        // The sleeve slot: the 128 px copy of the playing art, or the glyph
        // when nothing is cached (D17 replaced D7's disc-only placeholder).
        .child(
            div()
                .id("bar-cover")
                .debug_selector(|| "bar-cover".to_owned())
                .size(px(44.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(theme::border(cx))
                .bg(theme::sidebar(cx))
                .overflow_hidden()
                .child(cover::view(
                    cover_file.as_ref(),
                    cover_url.as_deref(),
                    &title,
                    if entry.is_some_and(|entry| entry.is_podcast()) {
                        Kind::Podcast
                    } else {
                        Kind::Album
                    },
                    px(44.),
                    cx,
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .w(px(260.))
                .min_w(px(140.))
                .flex_shrink_1()
                .gap(rems(0.125))
                .child(
                    div()
                        .text_size(rems(0.8125))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .truncate()
                        .child(if has_track {
                            title
                        } else {
                            "Rien en lecture".to_owned()
                        }),
                )
                .child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .truncate()
                        .child(if has_track {
                            if album.is_empty() {
                                artist
                            } else {
                                format!("{artist} · {album}")
                            }
                        } else {
                            "Choisissez un titre pour commencer".to_owned()
                        }),
                ),
        )
        .child(controls)
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .flex_1()
                .min_w(px(120.))
                .child(
                    div()
                        .w(px(40.))
                        .text_size(rems(0.625))
                        .text_color(theme::secondary(cx))
                        .text_align(gpui_kit::TextAlign::Right)
                        .child(clock(position)),
                )
                .child(progress(fraction, has_track, cx))
                .child(
                    div()
                        .w(px(40.))
                        .text_size(rems(0.625))
                        .text_color(theme::secondary(cx))
                        .child(clock(duration)),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.125))
                .child(
                    icon_button(
                        "bar-volume",
                        if state.player.muted() {
                            IconName::VolumeX
                        } else {
                            IconName::Volume2
                        },
                        "Couper le son",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(|_this, _, window, cx| {
                        window.dispatch_action(Box::new(Mute), cx)
                    })),
                )
                .child(
                    icon_button(
                        "bar-volume-down",
                        IconName::Volume1,
                        "Baisser le volume",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(|_this, _, window, cx| {
                        window.dispatch_action(Box::new(VolumeDown), cx)
                    })),
                )
                .child(
                    icon_button(
                        "bar-volume-up",
                        IconName::Volume2,
                        "Monter le volume",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(|_this, _, window, cx| {
                        window.dispatch_action(Box::new(VolumeUp), cx)
                    })),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.125))
                .child(
                    icon_button(
                        "bar-queue",
                        IconName::ListMusic,
                        "File d'attente",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .selected(state.side_panel.is_some())
                    .on_click(cx.listener(|_this, _, window, cx| {
                        window.dispatch_action(Box::new(ToggleQueue), cx)
                    })),
                )
                .child(
                    icon_button(
                        "bar-viz",
                        IconName::AudioLines,
                        "Visualiseur",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(|_this, _, window, cx| {
                        window.dispatch_action(Box::new(ToggleViz), cx)
                    })),
                )
                .child(
                    icon_button(
                        "bar-fullscreen",
                        IconName::Maximize2,
                        "Lecteur plein écran",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(|_this, _, window, cx| {
                        window.dispatch_action(Box::new(ToggleFullscreen), cx)
                    })),
                ),
        );
    bar = bar.child(
        button(
            "bar-quality",
            crate::state::quality_label(state.quality),
            ButtonVariant::Secondary,
            cx,
        )
        .on_click(cx.listener(|this, _, _, cx| this.open_quality_menu(cx))),
    );
    bar
}

/// The line of the bar: a filled track with twenty seek zones over it.
fn progress(fraction: f32, enabled: bool, cx: &Context<RootView>) -> impl IntoElement + use<> {
    let mut zones = div()
        .absolute()
        .inset_0()
        .flex()
        .children((0..20).map(|index| {
            let target = (index as f64 + 0.5) / 20.0;
            div()
                .id(gpui_kit::ElementId::Name(
                    format!("seek-zone-{index}").into(),
                ))
                .flex_1()
                .h_full()
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| this.seek_to(target, cx)),
                )
        }));
    if !enabled {
        zones = zones.opacity(0.4);
    }
    div()
        .id("bar-progress")
        .relative()
        .flex_1()
        .h(rems(0.25))
        .bg(theme::border(cx))
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .w(relative(fraction))
                .bg(theme::accent(cx)),
        )
        .child(zones)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D17: the 44 px slot paints the ROW-size copy (128 px), so the bar costs
    /// 65 536 B of decoded sleeve and as much atlas — not the 4 MB of the
    /// archived file D7 drew as a glyph placeholder.
    #[test]
    fn the_bar_slot_paints_the_row_size_sleeve() {
        assert_eq!(Covers::rows().class(), cover::Class::Row);
        assert_eq!(cover::Class::Row.px(), 128);
        assert_eq!(128u64 * 128 * 4, 65_536);
    }

    #[test]
    fn a_playhead_reads_as_minutes_and_seconds_never_as_zero_for_unknown() {
        assert_eq!(clock(0.0), "—");
        assert_eq!(clock(f64::NAN), "—");
        assert_eq!(clock(9.4), "0:09");
        assert_eq!(clock(61.0), "1:01");
        assert_eq!(clock(3_725.6), "62:06");
    }
}
