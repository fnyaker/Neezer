//! The shell: one view owns the window, the navigation, the header, the
//! sidebar, the player bar and the layers (queue panel, toasts, shortcut list).
//!
//! It is the only place that listens to application actions, so every key and
//! every click dispatches the same handler. Screens are entities built from
//! [`crate::screens::build`] and keep their own state; the shell rebuilds one
//! only when the route changes and otherwise leaves it alone.
//!
//! Two tasks run for the process's life and both are event-driven: one drains
//! [`crate::state::AppEvent`] (audio, MPRIS, network, session), one refreshes
//! the playhead and the toasts four times a second. A paused, idle app wakes for
//! neither the bar nor a repaint.

use std::time::Duration;

use gpui_kit::{
    Animation, AnimationExt, AnyView, AppContext as _, Context, ElementId, Entity, FocusHandle,
    FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px, relative, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, focus_scope, icon, icon_button, keycaps};
use nz_player::Command;

use crate::actions::*;
use crate::keymap::SHORTCUTS;
use crate::load_failed;
use crate::login::LoginScreen;
use crate::nav::{Route, Section, SettingsSection};
use crate::net_indicator;
use crate::player_bar;
use crate::screens;
use crate::state::{AppEvent, AppState, Auth, SidePanel};
use crate::theme;
use crate::toast;
use crate::wiring::events::Receiver;

pub struct RootView {
    state: Entity<AppState>,
    focus: FocusHandle,
    /// The current screen, and the route it was built for.
    screen: Option<(Route, AnyView)>,
    /// The queue panel, when open.
    side: Option<AnyView>,
    /// The login screen while it is the screen (its fields hold the typing).
    login: Option<AnyView>,
    shortcuts_open: bool,
    /// The next paint must give the keyboard anchor back to the shell (the
    /// login screen's field, now gone, used to hold it).
    refocus: bool,
    _subscriptions: Vec<Subscription>,
}

impl RootView {
    pub fn new(
        state: Entity<AppState>,
        events: Receiver<AppEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let mut this = Self {
            state: state.clone(),
            focus: focus.clone(),
            screen: None,
            side: None,
            login: None,
            shortcuts_open: false,
            refocus: false,
            _subscriptions: Vec::new(),
        };
        this._subscriptions.push(cx.observe(&state, |this, _, cx| {
            this.sync_screen(cx);
            cx.notify();
        }));
        window.focus(&focus, cx);
        this.spawn_events(events, cx);
        this.spawn_ticker(cx);
        this
    }

    pub fn state(&self) -> &Entity<AppState> {
        &self.state
    }

    fn spawn_events(&self, events: Receiver<AppEvent>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                let alive = this.update(cx, |this, cx| {
                    this.state.update(cx, |state, cx| state.on_event(event, cx));
                    this.sync_screen(cx);
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn spawn_ticker(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let alive = this.update(cx, |this, cx| {
                    this.state.update(cx, |state, cx| state.tick(cx));
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Rebuild the screen when the route changed. Called from the state
    /// observer and at the top of every render.
    fn sync_screen(&mut self, cx: &mut Context<Self>) {
        let (route, logged_in) = {
            let state = self.state.read(cx);
            (state.nav.current().cloned(), state.auth.is_logged_in())
        };
        if logged_in {
            // A new session gets a fresh login screen next time.
            self.login = None;
        }
        if !logged_in {
            self.screen = None;
            return;
        }
        if self.screen.is_none() {
            self.refocus = true;
        }
        match route {
            Some(route) => {
                let stale = self
                    .screen
                    .as_ref()
                    .map(|(current, _)| current != &route)
                    .unwrap_or(true);
                if stale {
                    let view = screens::build(&route, self.state.clone(), cx);
                    self.screen = Some((route, view));
                    self.shortcuts_open = false;
                    // A screen that had put the focus on one of its own nodes
                    // (the search field, a list's results) takes that focus with
                    // it when it is replaced: measured with D8a, `g l` then
                    // `Alt+←` did nothing, because the focused handle belonged to
                    // the screen that no longer existed and GPUI then dispatches
                    // above this view. The anchor goes back to the shell on every
                    // screen change, so a key NEVER depends on which screen the
                    // person came from.
                    self.refocus = true;
                }
            }
            None => self.screen = None,
        }
    }

    pub fn go(&mut self, route: Route, cx: &mut Context<Self>) {
        self.state.update(cx, |state, _| state.nav.push(route));
        self.sync_screen(cx);
        cx.notify();
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, _| {
            state.nav.back();
        });
        self.sync_screen(cx);
        cx.notify();
    }

    pub fn forward(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, _| {
            state.nav.forward();
        });
        self.sync_screen(cx);
        cx.notify();
    }

    pub fn close_top(&mut self, cx: &mut Context<Self>) {
        if self.shortcuts_open {
            self.shortcuts_open = false;
            cx.notify();
        } else if self.state.read(cx).side_panel.is_some() {
            self.toggle_queue(cx);
        } else {
            self.back(cx);
        }
    }

    pub fn command(&mut self, command: Command, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.command(command, cx));
        cx.notify();
    }

    pub fn seek_by(&mut self, delta: f64, cx: &mut Context<Self>) {
        let at = {
            let state = self.state.read(cx);
            let duration = state.player.duration();
            let target = state.player.current_time() + delta;
            if duration > 0.0 {
                target.clamp(0.0, duration)
            } else {
                target.max(0.0)
            }
        };
        self.command(
            Command::Seek {
                args: nz_player::SeekArgs { t: at },
            },
            cx,
        );
    }

    pub fn seek_to(&mut self, fraction: f64, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.seek_fraction(fraction, cx));
        cx.notify();
    }

    pub fn volume_by(&mut self, delta: f64, cx: &mut Context<Self>) {
        let volume = (self.state.read(cx).player.volume() + delta).clamp(0.0, 1.0);
        self.command(
            Command::SetVolume {
                args: nz_player::VolumeArgs { v: volume },
            },
            cx,
        );
    }

    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.retry(cx));
        cx.notify();
    }

    pub fn probe(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.probe(cx));
        cx.notify();
    }

    pub fn open_quality_menu(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.cycle_quality(cx));
        cx.notify();
    }

    pub fn toggle_queue(&mut self, cx: &mut Context<Self>) {
        if self.state.read(cx).side_panel.is_some() {
            self.side = None;
            self.state.update(cx, |state, _| state.side_panel = None);
        } else {
            let state = self.state.clone();
            let view: AnyView = cx
                .new(|cx| screens::queue::Screen::new(Route::Queue, state, cx))
                .into();
            self.side = Some(view);
            self.state
                .update(cx, |state, _| state.side_panel = Some(SidePanel::Queue));
        }
        cx.notify();
    }

    fn zoom(&mut self, factor: f32, window: &mut Window, cx: &mut Context<Self>) {
        let zoom = (self.state.read(cx).zoom * factor).clamp(0.75, 1.875);
        window.set_rem_size(px(16. * zoom));
        self.state.update(cx, |state, _| state.zoom = zoom);
        cx.notify();
    }

    fn zoom_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.set_rem_size(px(16.));
        self.state.update(cx, |state, _| state.zoom = 1.0);
        cx.notify();
    }

    fn login_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyView {
        if let Some(view) = &self.login {
            return view.clone();
        }
        let view: AnyView = cx
            .new(|cx| LoginScreen::new(self.state.clone(), window, cx))
            .into();
        self.login = Some(view.clone());
        view
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_screen(cx);
        // The shell owns the focus anchor: a screen change (the login screen
        // leaving, a screen that dropped its field) must not leave the window
        // with nothing focused, or a `Neezer`-scoped binding is not found and
        // the keyboard silently stops working.
        if std::mem::take(&mut self.refocus) {
            window.focus(&self.focus, cx);
        }

        let (auth, session_cause, login_pending, raise, route, nav_version) = {
            let state = self.state.read(cx);
            (
                state.auth.clone(),
                state.session_cause.clone(),
                state.login.pending,
                state.raise_requested,
                state.nav.current().cloned(),
                state.nav.version(),
            )
        };
        if raise {
            window.activate_window();
            cx.activate(true);
            self.state
                .update(cx, |state, _| state.raise_requested = false);
        }

        // A person is either signing in or using the app; there is no shell
        // around the login screen.
        if matches!(auth, Auth::LoggedOut) || (matches!(auth, Auth::Restoring) && login_pending) {
            let view = self.login_view(window, cx);
            return focus_scope("app")
                .size_full()
                .track_focus(&self.focus)
                .key_context("Neezer")
                .on_action(cx.listener(|_, _: &Quit, _, cx| cx.quit()))
                .on_action(cx.listener(|_, _: &ShowShortcuts, _, cx| {
                    let _ = cx;
                }))
                .child(view)
                .into_any_element();
        }
        if matches!(auth, Auth::Restoring) {
            return restoring(cx).into_any_element();
        }
        if matches!(auth, Auth::Failed) {
            let cause = session_cause.unwrap_or_else(|| "Le serveur n'a pas répondu.".into());
            return div()
                .size_full()
                .track_focus(&self.focus)
                .key_context("Neezer")
                .child(load_failed::view("Session non confirmée", cause, cx))
                .into_any_element();
        }

        let (online, probing, user, is_admin, side_open, toasts) = {
            let state = self.state.read(cx);
            (
                state.online,
                state.probing,
                state.user_name().map(|name| name.to_owned()),
                state.is_admin(),
                state.side_panel.is_some(),
                state.toasts.items().to_vec(),
            )
        };

        let mut root = focus_scope("app")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .key_context("Neezer")
            .bg(theme::shell(cx))
            .text_color(theme::text(cx))
            .font_family(theme::font(cx));

        root = root
            .on_action(cx.listener(|_, _: &Quit, _, cx| cx.quit()))
            .on_action(cx.listener(|this, _: &Logout, _, cx| {
                this.state.update(cx, |state, cx| state.logout(cx));
                this.login = None;
                this.sync_screen(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Retry, _, cx| this.retry(cx)))
            .on_action(cx.listener(|this, _: &ShowShortcuts, _, cx| {
                this.shortcuts_open = !this.shortcuts_open;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CloseTop, _, cx| this.close_top(cx)))
            .on_action(cx.listener(|this, _: &ZoomIn, window, cx| this.zoom(1.1, window, cx)))
            .on_action(
                cx.listener(|this, _: &ZoomOut, window, cx| this.zoom(1.0 / 1.1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &ZoomReset, window, cx| this.zoom_reset(window, cx)))
            .on_action(cx.listener(|this, _: &Back, _, cx| this.back(cx)))
            .on_action(cx.listener(|this, _: &Forward, _, cx| this.forward(cx)))
            .on_action(cx.listener(|this, _: &GoHome, _, cx| this.go(Route::Home, cx)))
            .on_action(cx.listener(|this, _: &GoSearch, _, cx| {
                this.go(
                    Route::Search {
                        query: String::new(),
                    },
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &GoLibrary, _, cx| {
                this.go(
                    Route::Library {
                        tab: crate::nav::LibraryTab::default(),
                    },
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &GoPodcasts, _, cx| this.go(Route::Podcasts, cx)))
            .on_action(cx.listener(|this, _: &GoReleases, _, cx| this.go(Route::Releases, cx)))
            .on_action(cx.listener(|this, _: &GoNowPlaying, _, cx| this.go(Route::NowPlaying, cx)))
            .on_action(cx.listener(|this, _: &GoSettings, _, cx| {
                this.go(
                    Route::Settings {
                        section: SettingsSection::default(),
                    },
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &PlayPause, _, cx| {
                this.command(
                    Command::Toggle {
                        args: Default::default(),
                    },
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &NextTrack, _, cx| {
                this.command(
                    Command::Next {
                        args: Default::default(),
                    },
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &PreviousTrack, _, cx| {
                this.command(
                    Command::Prev {
                        args: Default::default(),
                    },
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &SeekForward, _, cx| this.seek_by(10.0, cx)))
            .on_action(cx.listener(|this, _: &SeekBackward, _, cx| this.seek_by(-10.0, cx)))
            .on_action(cx.listener(|this, _: &VolumeUp, _, cx| this.volume_by(0.05, cx)))
            .on_action(cx.listener(|this, _: &VolumeDown, _, cx| this.volume_by(-0.05, cx)))
            .on_action(cx.listener(|this, _: &Mute, _, cx| {
                this.command(
                    Command::ToggleMute {
                        args: Default::default(),
                    },
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &ToggleShuffle, _, cx| {
                this.command(
                    Command::ToggleShuffle {
                        args: Default::default(),
                    },
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &CycleRepeat, _, cx| {
                this.command(
                    Command::CycleRepeat {
                        args: Default::default(),
                    },
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &ToggleQueue, _, cx| this.toggle_queue(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleFullscreen, _, cx| this.go(Route::NowPlaying, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleViz, _, cx| this.go(Route::Viz, cx)))
            .on_action(cx.listener(|this, _: &DismissToast, _, cx| {
                this.state.update(cx, |state, cx| {
                    if let Some(toast) = state.toasts.items().last() {
                        let id = toast.id;
                        state.toasts.dismiss(id);
                        cx.notify();
                    }
                });
            }));

        // Header: back/forward, the page's name, search, the link, help.
        let can_back = self.state.read(cx).nav.can_go_back();
        let can_forward = self.state.read(cx).nav.can_go_forward();
        let title = route
            .as_ref()
            .map(Route::title)
            .unwrap_or_else(|| "Neezer".to_owned());
        let header = div()
            .id("app-header")
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(rems(0.5))
            .h(theme::header_height(window))
            .px(rems(1.))
            .border_b_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                icon_button(
                    "nav-back",
                    IconName::ChevronLeft,
                    "Retour",
                    ButtonVariant::Secondary,
                    cx,
                )
                .disabled(!can_back)
                .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
            )
            .child(
                icon_button(
                    "nav-forward",
                    IconName::ChevronRight,
                    "Avancer",
                    ButtonVariant::Secondary,
                    cx,
                )
                .disabled(!can_forward)
                .on_click(cx.listener(|this, _, _, cx| this.forward(cx))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(div().w(rems(0.375)).h(rems(1.)).bg(theme::accent(cx)))
                    .child(
                        div()
                            .text_size(rems(0.875))
                            .font_weight(FontWeight::BOLD)
                            .child(title),
                    ),
            )
            .child(
                div()
                    .id("header-search")
                    .flex()
                    .flex_1()
                    .max_w(rems(26.))
                    .items_center()
                    .gap(rems(0.5))
                    .px(rems(0.625))
                    .py(rems(0.25))
                    .border_1()
                    .border_color(theme::border(cx))
                    .bg(theme::sidebar(cx))
                    .text_color(theme::secondary(cx))
                    .text_size(rems(0.75))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.go(
                            Route::Search {
                                query: String::new(),
                            },
                            cx,
                        )
                    }))
                    .child(icon(IconName::Search).size(rems(0.875)))
                    .child(
                        div()
                            .flex_1()
                            .child("Rechercher un titre, un album, un artiste"),
                    )
                    .child(keycaps(["Ctrl", "K"], cx)),
            )
            .child(div().flex_1())
            .child(net_indicator::render(online, probing, cx))
            .child(
                icon_button(
                    "header-help",
                    IconName::Keyboard,
                    "Raccourcis",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.shortcuts_open = !this.shortcuts_open;
                    cx.notify();
                })),
            )
            .child(
                icon_button(
                    "header-settings",
                    IconName::Settings,
                    "Réglages",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.go(
                        Route::Settings {
                            section: if is_admin {
                                SettingsSection::Server
                            } else {
                                SettingsSection::Playback
                            },
                        },
                        cx,
                    )
                })),
            );

        let mut header_right = div().flex().items_center().gap(rems(0.375));
        if let Some(user) = user {
            header_right = header_right.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(user),
            );
        }
        header_right = header_right.child(
            icon_button(
                "header-logout",
                IconName::LogOut,
                "Se déconnecter",
                ButtonVariant::Secondary,
                cx,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.state.update(cx, |state, cx| state.logout(cx));
                this.login = None;
                this.sync_screen(cx);
                cx.notify();
            })),
        );
        let header = header.child(header_right);

        let active = self.state.read(cx).current_route().and_then(Route::section);
        let sidebar = sidebar(active, cx).into_any_element();

        let content = match &self.screen {
            Some((_route, view)) => {
                let reduced = self.state.read(cx).reduced_motion;
                let duration = theme::motion_ms(theme::SCREEN_MS, reduced);
                div().relative().size_full().min_w_0().min_h_0().child(
                    div().size_full().child(view.clone()).with_animation(
                        ("screen", nav_version),
                        Animation::new(duration),
                        move |element, delta| element.opacity(delta).ml(px(24.0 * (1.0 - delta))),
                    ),
                )
            }
            None => div().size_full(),
        };

        let mut body = div()
            .flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(sidebar)
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .child(content),
            );
        if side_open && let Some(view) = &self.side {
            body = body.child(
                div()
                    .id("side-panel")
                    .flex()
                    .flex_col()
                    .w(theme::SIDE_PANEL)
                    .flex_shrink_0()
                    .border_l_1()
                    .border_color(theme::border(cx))
                    .bg(theme::chrome(cx))
                    .child(view.clone()),
            );
        }

        root = root
            .child(header)
            .child(body)
            .child(player_bar::render(self.state.clone(), cx));

        if !toasts.is_empty() {
            root = root.child(toast::render(&toasts, cx));
        }
        if self.shortcuts_open {
            root = root.child(shortcuts_overlay(cx));
        }
        root.into_any_element()
    }
}

fn sidebar(active: Option<Section>, cx: &mut Context<RootView>) -> impl IntoElement + use<> {
    let entries = [
        Section::Home,
        Section::Search,
        Section::Library,
        Section::Podcasts,
        Section::Releases,
        Section::NowPlaying,
    ];
    let mut column = div()
        .id("app-sidebar")
        .flex()
        .flex_col()
        .flex_shrink_0()
        .w(rems(12.5))
        .h_full()
        .py(rems(0.75))
        .px(rems(0.5))
        .gap(rems(0.125))
        .border_r_1()
        .border_color(theme::border(cx))
        .bg(theme::sidebar(cx));
    for section in entries {
        let selected = active == Some(section);
        let icon_name = match section {
            Section::Home => IconName::House,
            Section::Search => IconName::Search,
            Section::Library => IconName::LibraryBig,
            Section::Podcasts => IconName::Mic,
            Section::Releases => IconName::Sparkles,
            Section::NowPlaying => IconName::Disc3,
            Section::Settings => IconName::Settings,
        };
        column = column.child(
            div()
                .id(ElementId::Name(SharedString::from(format!(
                    "nav-{}",
                    section.title()
                ))))
                .flex()
                .items_center()
                .gap(rems(0.625))
                .h(rems(2.))
                .px(rems(0.625))
                .border_l_1()
                .border_color(if selected {
                    theme::accent(cx)
                } else {
                    theme::sidebar(cx)
                })
                .bg(if selected {
                    theme::selected(cx)
                } else {
                    theme::sidebar(cx)
                })
                .text_color(if selected {
                    theme::text(cx)
                } else {
                    theme::secondary(cx)
                })
                .text_size(rems(0.8125))
                .cursor_pointer()
                .hover(|style| style.bg(theme::selected(cx)).text_color(theme::text(cx)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.go(section.route(), cx);
                }))
                .child(icon(icon_name).size(rems(1.125)))
                .child(div().child(section.title())),
        );
    }
    column = column.child(div().flex_1());
    column.child(
        div()
            .id("nav-settings")
            .flex()
            .items_center()
            .gap(rems(0.625))
            .h(rems(2.))
            .px(rems(0.625))
            .text_size(rems(0.8125))
            .text_color(if active == Some(Section::Settings) {
                theme::text(cx)
            } else {
                theme::secondary(cx)
            })
            .cursor_pointer()
            .hover(|style| style.text_color(theme::text(cx)))
            .on_click(cx.listener(|this, _, _, cx| {
                this.go(
                    Route::Settings {
                        section: SettingsSection::default(),
                    },
                    cx,
                )
            }))
            .child(icon(IconName::Settings).size(rems(1.125)))
            .child(div().child(Section::Settings.title())),
    )
}

fn restoring(cx: &mut Context<RootView>) -> impl IntoElement + use<> {
    div()
        .id("restoring")
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(rems(0.75))
        .bg(theme::shell(cx))
        .text_color(theme::text(cx))
        .font_family(theme::font(cx))
        .child(
            icon(IconName::AudioLines)
                .size(rems(2.))
                .text_color(theme::accent(cx)),
        )
        .child(
            div()
                .text_size(rems(1.125))
                .font_weight(FontWeight::BOLD)
                .child("Connexion à Neezer"),
        )
        .child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child("Vérification de la session…"),
        )
}

fn shortcuts_overlay(cx: &mut Context<RootView>) -> impl IntoElement + use<> {
    let columns: Vec<Vec<crate::keymap::Shortcut>> =
        SHORTCUTS.chunks(13).map(|chunk| chunk.to_vec()).collect();
    let mut card = div()
        .id("shortcuts")
        .flex()
        .flex_col()
        .gap(rems(1.))
        .max_w(rems(64.))
        .max_h(relative(0.86))
        .p(rems(1.5))
        .border_1()
        .border_color(theme::border(cx))
        .bg(theme::chrome(cx))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    icon(IconName::Keyboard)
                        .size(rems(1.25))
                        .text_color(theme::accent(cx)),
                )
                .child(
                    div()
                        .text_size(rems(1.125))
                        .font_weight(FontWeight::BOLD)
                        .child("Raccourcis"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .child("Échap pour fermer"),
                ),
        );
    let mut grid = div().flex().gap(rems(1.5)).items_start();
    for column in columns {
        let mut stack = div().flex().flex_col().gap(rems(0.375)).flex_1();
        for row in column {
            stack = stack.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(rems(0.75))
                    .text_size(rems(0.6875))
                    .child(div().text_color(theme::secondary(cx)).child(row.label))
                    .child(keycaps(
                        row.keys
                            .split(" / ")
                            .flat_map(|part| part.split(' '))
                            .map(str::to_owned),
                        cx,
                    )),
            );
        }
        grid = grid.child(stack);
    }
    card = card.child(grid);
    div()
        .id("shortcuts-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::shell(cx).opacity(0.94))
        .on_mouse_down(
            gpui_kit::MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.shortcuts_open = false;
                cx.notify();
            }),
        )
        .child(card)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    use nz_player::{Command, Entry, QueueArgs};

    use super::*;
    use crate::state::{AppConfig, AppState, Auth, event_channel};

    /// The server the journey talks to: `/api/login`, `/api/me` and
    /// `/api/version`, nothing else. A real socket, so the async client, the
    /// cookie jar and the session restore are exercised; the assertion is on
    /// the state transition, never on a duration.
    fn stub_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("local address");
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0u8; 4096];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let body = if request.starts_with("GET /api/version") {
                    r#"{"version":"9.9.9"}"#.to_string()
                } else {
                    // `/api/login` answers `/me`'s own body.
                    r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#
                        .to_string()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Set-Cookie: session=abc; Path=/\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{address}")
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neezer-d7-app-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn track(id: &str, title: &str) -> Entry {
        Entry {
            id: id.to_owned(),
            title: title.to_owned(),
            duration: 120,
            ..Entry::default()
        }
    }

    /// Wait, bounded, for an async answer the app is expecting. This is an
    /// end-to-end test of the request path; it asserts the event, and the
    /// deadline only turns a hang into a failure.
    fn wait_until(
        runtime: &tokio::runtime::Runtime,
        cx: &mut gpui::VisualTestContext,
        state: &Entity<AppState>,
        what: &str,
        mut done: impl FnMut(&AppState) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            // The test thread is the network thread: driving the runtime here
            // means every wake (the HTTP answer, the event channel) happens on
            // GPUI's own thread, which is what the deterministic test scheduler
            // requires. The deadline only turns a hang into a failure.
            runtime.block_on(async {
                tokio::time::sleep(Duration::from_millis(1)).await;
            });
            cx.run_until_parked();
            if cx.update(|_, cx| done(state.read(cx))) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
        }
    }

    /// The acceptance's keyboard-only journey: the login screen is typed into,
    /// `Entrée` connects, `g l` navigates, `Space` plays and `Ctrl+→` moves to
    /// the next row — every step a keystroke through the real keymap.
    #[gpui::test]
    async fn the_keyboard_journey_reaches_a_playing_track_and_the_next_one(
        cx: &mut gpui::TestAppContext,
    ) {
        let server = stub_server();
        let dir = temp_dir("journey");
        cx.update(|cx| {
            gpui_omarchy::init(cx);
            crate::keymap::install(cx);
        });

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (events_tx, events_rx) = event_channel();
        let config = AppConfig {
            server: Some(server),
            start_audio: false,
            start_mpris: false,
            ..AppConfig::for_test(&dir)
        };
        let state = {
            let runtime = runtime.handle().clone();
            cx.update(|cx| cx.new(move |_| AppState::new(config, runtime, events_tx, None)))
        };
        let (_view, cx) = cx.add_window_view({
            let state = state.clone();
            move |window, cx| RootView::new(state, events_rx, window, cx)
        });

        // The server field is prefilled and focused; Tab reaches the account.
        cx.simulate_keystrokes("tab");
        cx.simulate_input("bench");
        cx.simulate_keystrokes("tab");
        cx.simulate_input("secret");
        cx.simulate_keystrokes("enter");

        wait_until(&runtime, cx, &state, "the session", |state| {
            state.auth.is_logged_in()
        });
        cx.update(|_, cx| {
            assert_eq!(
                state.read(cx).nav.current(),
                Some(&Route::Home),
                "a login lands on the home screen"
            );
        });

        // D8a's Home lists the tracks; until then the queue is seeded, which is
        // exactly what D8a's row will hand D7's transport.
        cx.update(|_, cx| {
            state.update(cx, |state, cx| {
                state.command(
                    Command::PlayQueue {
                        args: QueueArgs {
                            tracks: vec![track("one", "Premier"), track("two", "Deuxième")],
                            start: 0,
                            context: None,
                        },
                    },
                    cx,
                );
            });
        });

        // Seeded paused: the keyboard is what starts it.
        cx.update(|_, cx| {
            state.update(cx, |state, cx| {
                state.command(
                    Command::Pause {
                        args: Default::default(),
                    },
                    cx,
                );
            });
        });
        cx.simulate_keystrokes("space");
        cx.update(|_, cx| {
            let state = state.read(cx);
            assert!(state.player.playing(), "Espace starts the queue");
            let pending = state
                .pending_load
                .as_ref()
                .expect("the engine boundary was crossed");
            assert_eq!(pending.entry.id, "one");
            assert!(pending.autoplay);
        });

        cx.simulate_keystrokes("ctrl-right");
        cx.update(|_, cx| {
            let state = state.read(cx);
            assert_eq!(
                state.player.current().map(|entry| entry.id.as_str()),
                Some("two")
            );
            let pending = state
                .pending_load
                .as_ref()
                .expect("the next row was opened");
            assert_eq!(pending.entry.id, "two");
        });

        // Navigation, same keymap.
        cx.simulate_keystrokes("g l");
        cx.update(|_, cx| {
            let state = state.read(cx);
            assert!(matches!(
                state.nav.current(),
                Some(Route::Library {
                    tab: crate::nav::LibraryTab::Playlists
                })
            ));
        });
        cx.simulate_keystrokes("alt-left");
        cx.update(|_, cx| {
            assert_eq!(state.read(cx).nav.current(), Some(&Route::Home));
        });

        // The queue panel is the shell's own layer, at its fixed 420 px.
        cx.simulate_keystrokes("q");
        cx.update(|_, cx| {
            assert!(
                state.read(cx).side_panel.is_some(),
                "q ouvre le panneau de la file"
            );
        });
        cx.simulate_keystrokes("q");
        cx.update(|_, cx| {
            assert!(state.read(cx).side_panel.is_none());
        });

        std::fs::remove_dir_all(dir).ok();
        drop(runtime);
    }

    /// A session that cannot be confirmed is a `LoadFailed`, not an endless
    /// spinner: the server is gone and the cause is the client's own sentence.
    #[gpui::test]
    async fn a_stored_session_with_a_dead_server_shows_load_failed(cx: &mut gpui::TestAppContext) {
        let dir = temp_dir("dead");
        // A session file with a cookie for a port nothing listens on.
        let session = dir.join("session.json");
        std::fs::write(&session, r#"{"http://127.0.0.1:1":[["session","abc"]]}"#)
            .expect("session file");
        cx.update(|cx| {
            gpui_omarchy::init(cx);
            crate::keymap::install(cx);
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (events_tx, events_rx) = event_channel();
        let config = AppConfig {
            server: Some("http://127.0.0.1:1".to_owned()),
            start_audio: false,
            start_mpris: false,
            session_file: Some(session.clone()),
            ..AppConfig::for_test(&dir)
        };
        let state = {
            let runtime = runtime.handle().clone();
            cx.update(|cx| cx.new(move |_| AppState::new(config, runtime, events_tx, None)))
        };
        {
            let jar = nz_api::SessionJar::at(&session);
            let url = reqwest::Url::parse("http://127.0.0.1:1").expect("url");
            assert_eq!(jar.len(&url), 1, "the session file loads for its origin");
        }
        cx.update(|cx| {
            let state = state.read(cx);
            let api = state.api.as_ref().expect("an api");
            assert!(
                api.has_session(),
                "the written session file is for this origin ({api:?})"
            );
            assert_eq!(state.auth, Auth::Restoring);
        });
        let (_view, cx) = cx.add_window_view({
            let state = state.clone();
            move |window, cx| RootView::new(state, events_rx, window, cx)
        });

        wait_until(&runtime, cx, &state, "the failed restore", |state| {
            matches!(state.auth, Auth::Failed)
        });
        cx.update(|_, cx| {
            let state = state.read(cx);
            let cause = state
                .session_cause
                .clone()
                .expect("a cause a person can read");
            assert!(
                cause.contains("réseau") || cause.contains("serveur"),
                "{cause}"
            );
            assert_eq!(state.toasts.len(), 1, "the failure is also a toast");
        });

        std::fs::remove_dir_all(dir).ok();
        drop(runtime);
    }
}

#[cfg(test)]
mod theme_tests {
    use gpui_omarchy::ActiveTheme;

    /// The shell reads every colour through `cx.omarchy()`; a theme applied at
    /// runtime must reach those tokens, and gpui-omarchy's `apply_palette` is
    /// what refreshes the windows (`omarchy-theme-set` rewrites the palette and
    /// the watcher calls exactly this).
    #[gpui::test]
    async fn a_theme_change_reaches_the_shell_tokens(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_omarchy::init);
        let before = cx.update(|cx| {
            (
                crate::theme::shell(cx),
                crate::theme::chrome(cx),
                crate::theme::text(cx),
                crate::theme::accent(cx),
            )
        });
        cx.update(|cx| gpui_omarchy::Theme::flexoki_light().apply(cx));
        let after = cx.update(|cx| {
            (
                crate::theme::shell(cx),
                crate::theme::chrome(cx),
                crate::theme::text(cx),
                crate::theme::accent(cx),
            )
        });
        assert_ne!(before, after, "the shell tokens follow the live theme");
        assert_eq!(
            cx.update(|cx| cx.omarchy().name.to_string()),
            "Flexoki Light"
        );
    }
}

#[cfg(test)]
mod mouse {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    use nz_player::{Command, Entry, QueueArgs};

    use super::*;
    use crate::state::{AppConfig, AppState, Auth, event_channel};

    fn stub_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("local address");
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0u8; 4096];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let body = if request.starts_with("GET /api/version") {
                    r#"{"version":"9.9.9"}"#.to_string()
                } else {
                    r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#
                        .to_string()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Set-Cookie: session=abc; Path=/\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{address}")
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neezer-d7-mouse-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn entry(id: &str, title: &str) -> Entry {
        Entry {
            id: id.to_owned(),
            title: title.to_owned(),
            duration: 120,
            ..Entry::default()
        }
    }

    fn wait_until(
        runtime: &tokio::runtime::Runtime,
        cx: &mut gpui::VisualTestContext,
        state: &Entity<AppState>,
        mut done: impl FnMut(&AppState) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            runtime.block_on(async {
                tokio::time::sleep(Duration::from_millis(1)).await;
            });
            cx.run_until_parked();
            if cx.update(|_, cx| done(state.read(cx))) {
                return;
            }
            assert!(Instant::now() < deadline, "the session did not arrive");
        }
    }

    /// The mouse-only journey after the credentials: the play button and the
    /// next button are real hit tests in the real layout, and the transport is
    /// the same `PlayerState::apply` path a key uses.
    #[gpui::test]
    async fn the_mouse_journey_plays_and_skips(cx: &mut gpui::TestAppContext) {
        let server = stub_server();
        let dir = temp_dir("journey");
        cx.update(|cx| {
            gpui_omarchy::init(cx);
            crate::keymap::install(cx);
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (events_tx, events_rx) = event_channel();
        let config = AppConfig {
            server: Some(server),
            start_audio: false,
            start_mpris: false,
            ..AppConfig::for_test(&dir)
        };
        let state = {
            let runtime = runtime.handle().clone();
            cx.update(|cx| cx.new(move |_| AppState::new(config, runtime, events_tx, None)))
        };
        let (_view, cx) = cx.add_window_view({
            let state = state.clone();
            move |window, cx| RootView::new(state, events_rx, window, cx)
        });

        cx.update(|_, cx| {
            state.update(cx, |state, _| {
                state.login(state.server.clone(), "bench".into(), "Bench1".into());
            });
        });
        wait_until(&runtime, cx, &state, |state| {
            matches!(state.auth, Auth::LoggedIn { .. })
        });
        cx.update(|_, cx| {
            state.update(cx, |state, cx| {
                state.command(
                    Command::PlayQueue {
                        args: QueueArgs {
                            tracks: vec![entry("one", "Premier"), entry("two", "Deuxième")],
                            start: 0,
                            context: None,
                        },
                    },
                    cx,
                );
                state.command(
                    Command::Pause {
                        args: Default::default(),
                    },
                    cx,
                );
            });
        });

        // The window the acceptance describes, so the coordinates below are the
        // layout's and not the test platform's default size.
        let handle = cx.windows()[0];
        cx.simulate_window_resize(
            handle,
            gpui_kit::size(gpui_kit::px(1280.), gpui_kit::px(800.)),
        );
        cx.run_until_parked();

        // The player bar's geometry: the control cluster starts after the cover
        // and the 260 px title block, and the bar is 72 px tall at the bottom.
        let height = cx.update(|window, _| window.viewport_size().height);
        let y = height - gpui_kit::px(30.);
        cx.simulate_click(
            gpui_kit::point(gpui_kit::px(402.), y),
            gpui_kit::Modifiers::default(),
        );
        cx.update(|_, cx| {
            let state = state.read(cx);
            assert!(
                state.player.playing(),
                "un clic sur lecture démarre la file"
            );
            assert_eq!(
                state.player.current().map(|entry| entry.id.as_str()),
                Some("one")
            );
        });
        cx.simulate_click(
            gpui_kit::point(gpui_kit::px(434.), y),
            gpui_kit::Modifiers::default(),
        );
        cx.update(|_, cx| {
            assert_eq!(
                state
                    .read(cx)
                    .player
                    .current()
                    .map(|entry| entry.id.as_str()),
                Some("two"),
                "un clic sur suivant avance la file"
            );
        });

        std::fs::remove_dir_all(dir).ok();
        drop(runtime);
    }
}
