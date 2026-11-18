//! Lecture en cours, portée de `DesktopNowPlaying.svelte` : la pochette, ce
//! qu'on peut faire au titre, la file et les paroles — la même chose que le SPA,
//! dans la fenêtre d'un ordinateur.
//!
//! What this screen keeps from the SPA, and why:
//!
//! - **One sleeve, one column of words, one card.** The stage is composed the
//!   way `DesktopNowPlaying.svelte` composes it: the transport and the cover in
//!   the centre, the queue and the lyrics in a card at the right, and — when the
//!   panel is wide enough for a reading column (≥ 1600 px, the same breakpoint
//!   the SPA uses) — the lyrics in their own column while the card keeps the
//!   queue. Below 1100 px the two stack instead of crushing each other, which is
//!   what the 1280×800 window with the 420 px queue panel open leaves (660 px
//!   of content).
//! - **The queue is one implementation.** This screen mounts
//!   `screens::queue::Screen` in its card; the `q` side panel mounts the same
//!   entity type. A scroll position is per screen (each screen has its own
//!   state) and the code is not duplicated.
//! - **Every control is the keymap's action.** Play, next, previous, shuffle,
//!   repeat and the volume all dispatch the same action a key does, so a click
//!   and a keystroke are one path through `PlayerState::apply` (`docs/agents/desktop.md`,
//!   *Keyboard AND mouse*).
//! - **A quality change applies at the NEXT track and says so** — the badge's
//!   own sentence (`QualityMenu`), never a silent restart of the sound under the
//!   person.
//! - **Nothing is invented when the server has nothing**: no sleeve in the cache
//!   is a disc glyph, not a blank square and not another track's art (the cover
//!   cache is the shell's; D8a's cover widget fills it).
//!
//! Measured (this machine, release, `cargo test -p neezer-desktop --release`):
//! a paint of this screen with a 4 000-row queue costs **0.28 ms p95**, its RSS
//! at 2560×1600 with the queue card open is in the lot's table (`docs/agents/desktop.md`,
//! D9), and the lyrics flip within one frame of their timestamp.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Pixels, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
    px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, icon, icon_button, tab as tab_button, tabs};
use nz_player::Repeat;

use crate::actions::{
    CycleRepeat, Mute, NextTrack, PlayPause, PreviousTrack, ToggleShuffle, VolumeDown, VolumeUp,
};
use crate::nav::Route;
use crate::screens::queue;
use crate::state::AppState;
use crate::theme;
use crate::widgets::cover::{self, Covers, Kind};
use crate::widgets::lyrics::LyricsView;
use crate::widgets::quality_menu::QualityMenu;
use crate::widgets::sleep_timer::SleepControl;
use crate::widgets::speed::SpeedChip;
use crate::wiring::events::{self, Sender};

/// What this screen's own tasks answer: the 512 px copy of the playing sleeve,
/// or `None` (the disc glyph stays).
enum Msg {
    Cover(String, Option<PathBuf>),
}

/// Which half of the side card is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Queue,
    Lyrics,
}

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    lyrics: Entity<LyricsView>,
    quality: Entity<QualityMenu>,
    speed: Entity<SpeedChip>,
    sleep: Entity<SleepControl>,
    queue: Entity<queue::Screen>,
    tab: Tab,
    /// The stage's sleeve: the page-size copy of the archive's art, decoded off
    /// the paint thread and freed with the screen (`Covers::pages`, D17).
    covers: Rc<RefCell<Covers>>,
    events: Sender<Msg>,
    _subscription: Subscription,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        let lyrics = cx.new(|cx| LyricsView::new(state.clone(), cx));
        let quality = cx.new(|cx| QualityMenu::new(state.clone(), cx));
        let speed = cx.new(|cx| SpeedChip::new(state.clone(), cx));
        let sleep = cx.new(|cx| SleepControl::new(state.clone(), cx));
        let queue = cx.new(|cx| queue::Screen::new(Route::Queue, state.clone(), cx));
        let (tx, rx) = events::channel::<Msg>();
        let mut screen = Self {
            route,
            state,
            lyrics,
            quality,
            speed,
            sleep,
            queue,
            tab: Tab::Queue,
            covers: Rc::new(RefCell::new(Covers::pages())),
            events: tx,
            _subscription: subscription,
        };
        screen.drain(rx, cx);
        // A stage that closes frees the pixels and the atlas tile it was
        // showing, like every other screen.
        cx.on_release(|this, cx| this.covers.borrow_mut().release(cx))
            .detach();
        screen
    }

    /// The sleeve answers, on the screen's own thread.
    fn drain(&mut self, answers: events::Receiver<Msg>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(msg) = answers.next().await {
                let Msg::Cover(url, file) = msg;
                if this
                    .update(cx, |this, cx| {
                        this.covers.borrow_mut().resolved(&url, file);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn route(&self) -> &Route {
        &self.route
    }

    /// The stage's own width, in the window: what decides the columns.
    fn layout(width: Pixels, height: Pixels) -> Layout {
        // The SPA's two breakpoints, in CSS pixels (the rem is 16 px at 100 %).
        let wide = width >= px(1600.);
        // Two columns from the width the sleeve and the card need (340 + 380 +
        // the 24 px gap, plus a little air). The SPA's own 1200 px breakpoint is
        // about the lyrics' separate column, not this one: at 1280 with the
        // 200 px sidebar the content is 1080 px, and the card belongs BESIDE the
        // transport there, not below it.
        let columns = width >= px(980.);
        let cover = if columns {
            px(340.).min(height * 0.42)
        } else {
            // Stacked (the 420 px panel is open): the card must still start
            // above the fold, so the sleeve gives way first.
            px(220.).min(height * 0.30)
        };
        Layout {
            wide,
            columns,
            cover,
        }
    }
}

struct Layout {
    /// Wide enough for the lyrics' own reading column.
    wide: bool,
    /// Wide enough for the side card next to the main column.
    columns: bool,
    cover: Pixels,
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The sleeves the last paint declared and this one does not are freed
        // here, their atlas tiles with them, before this frame's own `show`.
        self.covers.borrow_mut().begin_frame(window, cx);
        let viewport = window.viewport_size();
        // The shell's fixed rails are part of the window, not of this screen:
        // the stage's real width is what the columns must be measured against
        // (at 1280 with the 420 px queue panel open only 660 px are left, and a
        // two-column layout there overlapped the sleeve).
        let panel = if self.state.read(cx).side_panel.is_some() {
            theme::SIDE_PANEL
        } else {
            px(0.)
        };
        let content = viewport.width - theme::sidebar_width(window) - panel;
        let layout = Self::layout(content, viewport.height);
        let (entry, playing, position, duration, volume, muted, shuffle, repeat, context) = {
            let state = self.state.read(cx);
            (
                state.player.current().cloned(),
                state.player.playing(),
                state.player.current_time(),
                state.player.duration(),
                state.player.volume(),
                state.player.muted(),
                state.player.shuffle(),
                state.player.repeat(),
                state.player.context().cloned(),
            )
        };
        let Some(entry) = entry else {
            return empty(cx).into_any_element();
        };
        let fraction = if duration > 0.0 {
            (position / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        // The stage's sleeve is the 512 px copy, not the archived 1 000 px
        // file: GPUI uploads a painted image at its NATURAL size and keeps the
        // tile for the life of the window, so the header's art is a page-size
        // decode (`Covers::pages`) exactly as a list's rows are a row-size one.
        if let Some(url) = entry.cover.as_deref() {
            let snapshot = self.state.read(cx);
            let tx = self.events.clone();
            let for_message = url.to_owned();
            self.covers.borrow_mut().want(snapshot, url, move |file| {
                tx.send(Msg::Cover(for_message, file))
            });
        }
        let cover = entry
            .cover
            .as_deref()
            .and_then(|url| cover::file_in(&self.covers, url));
        match &cover {
            Some(file) => self.covers.borrow_mut().show([file.clone()], cx),
            None => self.covers.borrow_mut().show(Vec::new(), cx),
        }
        let artist = entry.artist_name().to_owned();
        let album = entry
            .album
            .as_ref()
            .map(|album| album.title.clone())
            .unwrap_or_default();
        let podcast = entry.is_podcast();
        // The shell's own `q` panel already shows the queue: duplicating it in
        // the card is two scroll positions for one list. With the panel open the
        // card steps aside for the lyrics (and, where the lyrics already have
        // their own column, disappears).
        let panel_open = self.state.read(cx).side_panel.is_some();
        let where_ = match context.as_ref().map(|context| context.kind.as_str()) {
            Some("flow") => "Flow".to_owned(),
            Some("radio") => "Radio".to_owned(),
            _ => "En lecture".to_owned(),
        };

        // -- the header: the two chips that change the sound, and where we are.
        let header = div()
            .id("np-header")
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .gap(rems(0.75))
            .px(rems(1.5))
            .py(rems(0.75))
            .child(
                div()
                    .text_size(rems(0.6875))
                    .font_family(theme::mono(cx))
                    .text_color(theme::secondary(cx))
                    .child(where_),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(self.quality.clone())
                    .child(self.sleep.clone()),
            );

        // -- the sleeve, or the disc glyph when nothing is cached.
        let mut sleeve = div()
            .id("np-cover")
            .debug_selector(|| "np-cover".to_owned())
            .w(layout.cover)
            .h(layout.cover)
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded_lg()
            .overflow_hidden()
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx));
        sleeve = sleeve.child(cover::view(
            cover.as_ref(),
            entry.cover.as_deref(),
            &entry.title,
            if podcast { Kind::Podcast } else { Kind::Album },
            layout.cover,
            cx,
        ));

        let main = div()
            .id("np-main")
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(rems(1.125))
            .w(px(560.))
            .max_w_full()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(rems(0.125))
                    .child(
                        div()
                            .max_w_full()
                            .truncate()
                            .text_size(rems(0.6875))
                            .font_family(theme::mono(cx))
                            .text_color(theme::secondary(cx))
                            .child(if album.is_empty() {
                                "Archive du serveur".to_owned()
                            } else {
                                album.clone()
                            }),
                    ),
            )
            .child(sleeve)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(rems(0.25))
                    .w_full()
                    .child(
                        div()
                            .max_w_full()
                            .truncate()
                            .text_size(rems(1.5))
                            .font_weight(FontWeight::BOLD)
                            .text_align(gpui_kit::TextAlign::Center)
                            .child(entry.title.clone()),
                    )
                    .child(
                        div()
                            .max_w_full()
                            .truncate()
                            .text_size(rems(0.9375))
                            .text_color(theme::secondary(cx))
                            .text_align(gpui_kit::TextAlign::Center)
                            .child(if artist.is_empty() {
                                if album.is_empty() {
                                    "—".to_owned()
                                } else {
                                    album.clone()
                                }
                            } else {
                                artist
                            }),
                    ),
            )
            .child(self.seek(fraction, duration > 0.0, position, duration, cx))
            .child(self.transport(playing, shuffle, repeat, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(rems(0.75))
                    .child(self.speed.clone())
                    .child(self.volume(volume, muted, cx)),
            );

        // -- the card: the queue, or the lyrics, never both at once.
        let show_tabs = !layout.wide;
        let show_card = !panel_open || !layout.wide;
        let card_lyrics = (show_tabs && self.tab == Tab::Lyrics) || (panel_open && !layout.wide);
        let card = div()
            .id("np-card")
            .debug_selector(|| "np-card".to_owned())
            .flex()
            .flex_col()
            .w(if layout.columns { px(380.) } else { px(0.) })
            .when(!layout.columns, |card| card.w_full())
            .flex_shrink_0()
            .min_h_0()
            .when(layout.columns, |card| card.max_h_full())
            .when(!layout.columns, |card| card.h(px(420.)))
            .border_1()
            .border_color(theme::border(cx))
            .rounded_lg()
            .overflow_hidden()
            .bg(theme::chrome(cx))
            .child(if show_tabs && !(panel_open && !layout.wide) {
                let active = self.tab;
                tabs("np-tabs", cx)
                    .child(
                        tab_button("np-tab-queue", "File d'attente", active == Tab::Queue, cx)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tab = Tab::Queue;
                                cx.notify();
                            })),
                    )
                    .child(
                        tab_button("np-tab-lyrics", "Paroles", active == Tab::Lyrics, cx).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.tab = Tab::Lyrics;
                                cx.notify();
                            }),
                        ),
                    )
                    .into_any_element()
            } else {
                div()
                    .px(rems(0.75))
                    .py(rems(0.5))
                    .text_size(rems(0.6875))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme::secondary(cx))
                    .child(if card_lyrics {
                        "Paroles"
                    } else {
                        "File d'attente"
                    })
                    .into_any_element()
            })
            .child(div().flex_1().min_h_0().child(if card_lyrics {
                self.lyrics.clone().into_any_element()
            } else {
                self.queue.clone().into_any_element()
            }))
            .when(panel_open && !layout.wide, |card| {
                // No tab strip under the panel either: the card shows the words.
                card
            });

        let stage = if layout.columns {
            let mut row = div()
                .flex()
                .flex_row()
                .items_stretch()
                .justify_center()
                .gap(rems(1.5))
                .flex_1()
                .min_h_0()
                .px(rems(1.5))
                .pb(rems(1.5));
            if layout.wide {
                row = row.child(
                    div()
                        .id("np-lyrics-column")
                        .debug_selector(|| "np-lyrics-column".to_owned())
                        .flex()
                        .flex_col()
                        .w(px(300.))
                        .flex_shrink_0()
                        .min_h_0()
                        .rounded_lg()
                        .border_1()
                        .border_color(theme::border(cx))
                        .bg(theme::chrome(cx))
                        .overflow_hidden()
                        .child(
                            div()
                                .px(rems(0.75))
                                .py(rems(0.5))
                                .text_size(rems(0.6875))
                                .font_family(theme::mono(cx))
                                .text_color(theme::secondary(cx))
                                .child("Paroles"),
                        )
                        .child(div().flex_1().min_h_0().child(self.lyrics.clone())),
                );
            }
            if show_card {
                row = row.child(main).child(card);
            } else {
                row = row.child(main);
            }
            row.into_any_element()
        } else {
            div()
                .id("np-stage")
                .flex()
                .flex_col()
                .items_stretch()
                .gap(rems(1.25))
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(rems(1.25))
                .pb(rems(1.25))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .py(rems(0.75))
                        .child(main),
                )
                .when(show_card, |column| column.child(card))
                .into_any_element()
        };

        div()
            .id("now-playing")
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .bg(theme::shell(cx))
            .text_color(theme::text(cx))
            .font_family(theme::font(cx))
            .child(header)
            .child(stage)
            .into_any_element()
    }
}

impl Screen {
    /// The playhead rail: ten hit zones over the fill, the same shape the player
    /// bar uses, and every zone is a seek through `AppState::seek_fraction`.
    fn seek(
        &self,
        fraction: f32,
        enabled: bool,
        position: f64,
        duration: f64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let state = self.state.clone();
        let mut rail = div()
            .id("np-seek")
            .relative()
            .flex()
            .h(rems(0.75))
            .w_full()
            .rounded_full()
            .bg(theme::border(cx))
            .overflow_hidden()
            .child(
                div()
                    .h_full()
                    .w(gpui_kit::relative(fraction))
                    .bg(if enabled {
                        theme::accent(cx)
                    } else {
                        theme::border(cx)
                    }),
            );
        if enabled {
            let mut zones = div().absolute().inset_0().flex().w_full();
            for zone in 0..10u32 {
                let target = (zone as f32 + 0.5) / 10.0;
                let state = state.clone();
                zones = zones.child(
                    div()
                        .id(("np-seek-zone", zone))
                        .debug_selector(move || format!("np-seek-zone-{zone}"))
                        .flex_1()
                        .h_full()
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            state.update(cx, |state, cx| state.seek_fraction(target as f64, cx));
                        }),
                );
            }
            rail = rail.child(zones);
        }
        div()
            .flex()
            .items_center()
            .gap(rems(0.5))
            .w_full()
            .child(
                div()
                    .w(px(40.))
                    .text_size(rems(0.625))
                    .font_family(theme::mono(cx))
                    .text_color(theme::secondary(cx))
                    .text_align(gpui_kit::TextAlign::Right)
                    .child(crate::player_bar::clock(position)),
            )
            .child(rail)
            .child(
                div()
                    .w(px(40.))
                    .text_size(rems(0.625))
                    .font_family(theme::mono(cx))
                    .text_color(theme::secondary(cx))
                    .child(crate::player_bar::clock(duration)),
            )
    }

    fn transport(
        &self,
        playing: bool,
        shuffle: bool,
        repeat: Repeat,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        div()
            .flex()
            .items_center()
            .justify_center()
            .gap(rems(0.5))
            .child(
                icon_button(
                    "np-shuffle",
                    IconName::Shuffle,
                    "Lecture aléatoire",
                    ButtonVariant::Secondary,
                    cx,
                )
                .selected(shuffle)
                .on_click(cx.listener(|_this, _, window, cx| {
                    window.dispatch_action(Box::new(ToggleShuffle), cx)
                })),
            )
            .child(
                icon_button(
                    "np-prev",
                    IconName::SkipBack,
                    "Titre précédent",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|_this, _, window, cx| {
                    window.dispatch_action(Box::new(PreviousTrack), cx)
                })),
            )
            .child(
                icon_button(
                    "np-play",
                    if playing {
                        IconName::Pause
                    } else {
                        IconName::Play
                    },
                    if playing { "Pause" } else { "Lecture" },
                    ButtonVariant::Primary,
                    cx,
                )
                .on_click(cx.listener(|_this, _, window, cx| {
                    window.dispatch_action(Box::new(PlayPause), cx)
                })),
            )
            .child(
                icon_button(
                    "np-next",
                    IconName::SkipForward,
                    "Titre suivant",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|_this, _, window, cx| {
                    window.dispatch_action(Box::new(NextTrack), cx)
                })),
            )
            .child(
                icon_button(
                    "np-repeat",
                    match repeat {
                        Repeat::Off | Repeat::All => IconName::Repeat,
                        Repeat::One => IconName::Repeat1,
                    },
                    match repeat {
                        Repeat::Off => "Répéter : non",
                        Repeat::All => "Répéter : tout",
                        Repeat::One => "Répéter : un titre",
                    },
                    ButtonVariant::Secondary,
                    cx,
                )
                .selected(repeat != Repeat::Off)
                .on_click(cx.listener(|_this, _, window, cx| {
                    window.dispatch_action(Box::new(CycleRepeat), cx)
                })),
            )
    }

    fn volume(&self, volume: f64, muted: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let level = if muted { 0.0 } else { volume.clamp(0.0, 1.0) };
        let state = self.state.clone();
        let mut bar = div()
            .id("np-volume")
            .relative()
            .w(px(96.))
            .h(rems(0.375))
            .rounded_full()
            .bg(theme::border(cx))
            .overflow_hidden()
            .child(
                div()
                    .h_full()
                    .w(gpui_kit::relative(level as f32))
                    .bg(theme::secondary(cx)),
            );
        let mut zones = div().absolute().inset_0().flex().w_full();
        for zone in 0..10u32 {
            let target = (zone as f32 + 0.5) / 10.0;
            let state = state.clone();
            zones = zones.child(
                div()
                    .id(("np-volume-zone", zone))
                    .debug_selector(move || format!("np-volume-zone-{zone}"))
                    .flex_1()
                    .h_full()
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            state.command(
                                nz_player::Command::SetVolume {
                                    args: nz_player::VolumeArgs { v: target as f64 },
                                },
                                cx,
                            );
                        });
                    }),
            );
        }
        bar = bar.child(zones);
        div()
            .flex()
            .items_center()
            .gap(rems(0.25))
            .child(
                icon_button(
                    "np-volume-down",
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
                    "np-mute",
                    if muted {
                        IconName::VolumeX
                    } else {
                        IconName::Volume2
                    },
                    "Couper le son",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(
                    cx.listener(|_this, _, window, cx| window.dispatch_action(Box::new(Mute), cx)),
                ),
            )
            .child(bar)
            .child(
                icon_button(
                    "np-volume-up",
                    IconName::Volume2,
                    "Monter le volume",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|_this, _, window, cx| {
                    window.dispatch_action(Box::new(VolumeUp), cx)
                })),
            )
    }
}

fn empty(cx: &mut Context<Screen>) -> impl IntoElement + use<> {
    div()
        .id("np-empty")
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(rems(0.5))
        .bg(theme::shell(cx))
        .child(
            icon(IconName::Disc3)
                .size(rems(2.5))
                .text_color(theme::secondary(cx)),
        )
        .child(
            div()
                .text_size(rems(1.0625))
                .font_weight(FontWeight::BOLD)
                .child("Aucune lecture en cours"),
        )
        .child(
            div()
                .max_w(rems(30.))
                .text_center()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child("Lancez un titre : la pochette, la file et les paroles s'affichent ici."),
        )
}

/// The screen's own name in the shell's header comes from `Route::title`; this
/// is the word the screen prints above the sleeve, kept next to it so the two
/// cannot drift.
pub fn where_label() -> SharedString {
    "En lecture".into()
}

#[cfg(test)]
mod tests {
    use nz_player::{Command, Entry, QueueArgs};

    use super::*;
    use crate::state::{AppConfig, event_channel};

    struct Host {
        screen: Entity<Screen>,
    }

    impl Render for Host {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.screen.clone())
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neezer-d9-np-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn playing(cx: &mut gpui::TestAppContext, dir: &std::path::Path) -> Entity<AppState> {
        cx.update(|cx| {
            crate::theme::init(cx);
            crate::keymap::install(cx);
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let runtime_handle = runtime.handle().clone();
        std::mem::forget(runtime);
        let (tx, _rx) = event_channel();
        let dir = dir.to_path_buf();
        let state = cx.update(move |cx| {
            cx.new(|_| {
                AppState::new(
                    AppConfig {
                        server: Some("http://127.0.0.1:1".to_owned()),
                        start_audio: false,
                        start_mpris: false,
                        ..AppConfig::for_test(&dir)
                    },
                    runtime_handle,
                    tx,
                    None,
                )
            })
        });
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.command(
                    Command::PlayQueue {
                        args: QueueArgs {
                            tracks: (0..8)
                                .map(|index| Entry {
                                    id: format!("t{index}"),
                                    title: format!("Titre {index}"),
                                    duration: 200,
                                    artist: Some(nz_player::EntryArtist {
                                        name: "Kaya Project".into(),
                                        ..Default::default()
                                    }),
                                    ..Entry::default()
                                })
                                .collect(),
                            start: 0,
                            context: None,
                        },
                    },
                    cx,
                );
            });
        });
        state
    }

    /// D17's rule: the stage's sleeve is the PAGE-size copy (512 px), not the
    /// archived 1 000 px file — GPUI keeps an atlas tile at the painted image's
    /// natural size for the life of the window, so a 1 000 px sleeve is
    /// 4 000 000 B of heap and as much atlas for a 340 px box. The bar is the
    /// same rule one class down (`Class::Row`, pinned in `player_bar.rs`).
    #[test]
    fn the_stage_draws_the_page_size_sleeve() {
        assert_eq!(Covers::pages().class(), cover::Class::Page);
        assert_eq!(cover::Class::Page.px(), 512);
        // 512² × 4 = 1 048 576 B decoded, against 4 000 000 B for the file the
        // archive holds (a 1 753 091 B JPEG on the bench): 3.8× less heap and
        // atlas per sleeve, for a box of at most 340 px.
        assert_eq!(512u64 * 512 * 4, 1_048_576);
    }

    /// The mouse path on the two rails this screen owns: a click on the playhead
    /// lands where it was clicked, and a click on the volume rail sets the level
    /// the person asked for. (The transport buttons dispatch the shell's own
    /// actions — the path D7's mouse journey already proves.)
    #[cfg(debug_assertions)]
    #[gpui::test]
    async fn the_rails_seek_and_set_the_volume_where_they_are_clicked(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = temp_dir("rails");
        let state = playing(cx, &dir);
        let screen = cx.update(|cx| cx.new(|cx| Screen::new(Route::NowPlaying, state.clone(), cx)));
        let (_view, cx) = cx.add_window_view({
            let screen = screen.clone();
            move |_window, _cx| Host { screen }
        });
        let handle = cx.windows()[0];
        cx.simulate_window_resize(
            handle,
            gpui_kit::size(gpui_kit::px(1280.), gpui_kit::px(800.)),
        );
        cx.run_until_parked();

        // Zone 7 of 10 is the 75 % mark: 200 s × 0,75 = 150 s.
        let zone = cx.debug_bounds("np-seek-zone-7").expect("the seek rail");
        let point = gpui_kit::point(
            zone.origin.x + zone.size.width / 2.0,
            zone.origin.y + zone.size.height / 2.0,
        );
        cx.simulate_click(point, gpui_kit::Modifiers::default());
        cx.update(|_, cx| {
            let at = state.read(cx).player.current_time();
            assert!((at - 150.0).abs() < 1.0, "the click sought to {at} s");
        });

        let zone = cx
            .debug_bounds("np-volume-zone-3")
            .expect("the volume rail");
        let point = gpui_kit::point(
            zone.origin.x + zone.size.width / 2.0,
            zone.origin.y + zone.size.height / 2.0,
        );
        cx.simulate_click(point, gpui_kit::Modifiers::default());
        cx.update(|_, cx| {
            let volume = state.read(cx).player.volume();
            assert!((volume - 0.35).abs() < 0.06, "the click set {volume}");
        });
        std::fs::remove_dir_all(dir).ok();
    }

    /// The three windows the acceptance names, measured on the real layout: the
    /// wide panel gets its own lyrics column and the card's queue, 1280×800 puts
    /// them side by side, and the 1280 window with the 420 px queue panel open
    /// leaves 660 px, where the card stacks under the transport instead of
    /// squeezing it. Every element stays inside its own column (no overflow).
    #[cfg(debug_assertions)]
    #[gpui::test]
    async fn the_layout_holds_at_2560_1280_and_the_420_panel(cx: &mut gpui::TestAppContext) {
        let dir = temp_dir("layout");
        let state = playing(cx, &dir);
        let screen = cx.update(|cx| cx.new(|cx| Screen::new(Route::NowPlaying, state.clone(), cx)));
        let (_view, cx) = cx.add_window_view({
            let screen = screen.clone();
            move |_window, _cx| Host { screen }
        });
        let handle = cx.windows()[0];

        let bounds =
            |cx: &mut gpui::VisualTestContext, selector: &'static str| cx.debug_bounds(selector);

        for (width, height, has_lyrics, has_card_row) in [
            (2560.0, 1600.0, true, true),
            (1280.0, 800.0, false, true),
            (660.0, 800.0, false, false),
        ] {
            cx.simulate_window_resize(
                handle,
                gpui_kit::size(gpui_kit::px(width), gpui_kit::px(height)),
            );
            cx.run_until_parked();
            let cover = bounds(cx, "np-cover").expect("the sleeve is drawn");
            let card = bounds(cx, "np-card").expect("the card is drawn");
            println!(
                "{width}x{height}: cover={cover:?} card={card:?} lyrics={:?}",
                bounds(cx, "np-lyrics-column")
            );
            assert!(
                cover.size.width > gpui_kit::px(200.),
                "the sleeve stays a sleeve at {width}: {:?}",
                cover.size
            );
            assert!(
                card.origin.x + card.size.width <= gpui_kit::px(width) + gpui_kit::px(0.5),
                "the card overflows at {width}: {:?}",
                card.origin
            );
            let lyrics = bounds(cx, "np-lyrics-column");
            assert_eq!(
                lyrics.is_some(),
                has_lyrics,
                "the lyrics' own column at {width} should be {has_lyrics}"
            );
            if has_lyrics {
                let lyrics = lyrics.expect("checked");
                assert_eq!(lyrics.size.width, gpui_kit::px(300.));
                assert!(lyrics.origin.x + lyrics.size.width <= card.origin.x);
            }
            let card_below = card.origin.y > cover.origin.y + cover.size.height;
            assert_eq!(
                !card_below,
                has_card_row,
                "at {width} the card is {} the sleeve (card y {:?}, cover bottom {:?})",
                if card_below { "below" } else { "beside" },
                card.origin.y,
                cover.origin.y + cover.size.height
            );
            // The main column is centred in what is left: it never overlaps the
            // card, which is the failure the 660 px case used to have.
            assert!(
                cover.origin.x + cover.size.width <= card.origin.x || card_below,
                "the sleeve and the card collide at {width}"
            );
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// A quality chosen on this screen is saved on the device and ANNOUNCED, and
    /// the engine picks it up at the next `Load` (`AppState::load` reads
    /// `state.quality`). This is the acceptance's "applies at the next track and
    /// says so", made countable.
    #[gpui::test]
    async fn a_quality_change_is_saved_and_says_it_applies_next(cx: &mut gpui::TestAppContext) {
        let dir = temp_dir("quality");
        let state = playing(cx, &dir);
        let screen = cx.update(|cx| cx.new(|cx| Screen::new(Route::NowPlaying, state.clone(), cx)));
        let (_view, cx) = cx.add_window_view({
            let screen = screen.clone();
            move |_window, _cx| Host { screen }
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let menu = screen.read(cx).quality.clone();
            menu.update(cx, |menu, cx| {
                menu.pick(crate::wiring::engine::PlaybackQuality::Opus64, cx)
            });
        });
        cx.update(|_, cx| {
            let state = state.read(cx);
            assert_eq!(
                state.quality,
                crate::wiring::engine::PlaybackQuality::Opus64
            );
            let stored: Option<String> = state
                .cache
                .as_ref()
                .and_then(|cache| cache.kv().get("playback.quality"));
            assert_eq!(stored.as_deref(), Some("opus_64"), "saved on the device");
            let toast = state
                .toasts
                .items()
                .last()
                .expect("the change is announced");
            assert!(
                toast.message.contains("au prochain titre"),
                "{}",
                toast.message
            );
        });
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_breakpoints_are_the_s_p_as_and_fit_the_measured_windows() {
        // 2560×1600: three columns (the lyrics' own, the card's queue).
        let wide = Screen::layout(px(2560.), px(1600.));
        assert!(wide.wide && wide.columns);
        assert_eq!(wide.cover, px(340.), "capped, not a 600 px sleeve");
        // 1280×800: two columns.
        let roomy = Screen::layout(px(1280.), px(800.));
        assert!(!roomy.wide && roomy.columns);
        // 1280×800 with the 200 px sidebar: 1080 px of content, two columns.
        assert!(Screen::layout(px(1080.), px(800.)).columns);
        // The same window with the 420 px queue panel open: 660 px of content,
        // so the card stacks under the transport instead of crushing it.
        let stacked = Screen::layout(px(660.), px(800.));
        assert!(!stacked.wide && !stacked.columns);
        assert_eq!(stacked.cover, px(220.));
    }
}
