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

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{
    Animation, AnimationExt, AnyView, AppContext as _, Context, ElementId, Entity, FocusHandle,
    FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px, relative, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, focus_scope, icon, icon_button, keycaps};
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
use crate::widgets::cover::Covers;
use crate::wiring::events::{Receiver, Sender};
use crate::wiring::remote_host;

/// The genre studio's own action: D7 froze `actions.rs` after the shell landed,
/// and D16 may not add a variant there, so the verb lives next to the one screen
/// that answers it (the keyboard table and the help overlay read it from here).
#[derive(Clone, Copy, Debug, PartialEq, gpui_kit::Action)]
#[action(namespace = neezer, no_json)]
pub struct GoGenres;

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
    /// The bar's sleeve: the 128 px copy of what plays, so a 44 px slot never
    /// uploads a 1 000 px tile (`Covers::rows`, D17).
    bar_covers: Rc<RefCell<Covers>>,
    bar_events: Sender<player_bar::BarMsg>,
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
        let (bar_events, bar_answers) = crate::wiring::events::channel::<player_bar::BarMsg>();
        let mut this = Self {
            state: state.clone(),
            focus: focus.clone(),
            screen: None,
            side: None,
            login: None,
            shortcuts_open: false,
            refocus: false,
            bar_covers: Rc::new(RefCell::new(Covers::rows())),
            bar_events: bar_events.clone(),
            _subscriptions: Vec::new(),
        };
        this._subscriptions.push(cx.observe(&state, |this, _, cx| {
            this.sync_screen(cx);
            cx.notify();
        }));
        window.focus(&focus, cx);
        this.spawn_events(events, cx);
        this.spawn_ticker(cx);
        this.spawn_bar_covers(bar_answers, cx);
        this
    }

    /// The bar's sleeve answers, on the shell's own thread: the bar is not a
    /// screen, so it owns its cache and its channel here.
    fn spawn_bar_covers(&self, answers: Receiver<player_bar::BarMsg>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(msg) = answers.next().await {
                let player_bar::BarMsg::Cover(url, file) = msg;
                if this
                    .update(cx, |this, cx| {
                        this.bar_covers.borrow_mut().resolved(&url, file);
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
        let render_started = std::time::Instant::now();
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
            .on_action(cx.listener(|this, _: &GoGenres, _, cx| this.go(Route::Genres, cx)))
            .on_action(cx.listener(|this, _: &GoNowPlaying, _, cx| this.go(Route::NowPlaying, cx)))
            .on_action(cx.listener(|this, _: &GoQueue, _, cx| this.go(Route::Queue, cx)))
            .on_action(cx.listener(|this, _: &RemoteClaim, _, cx| this.go(Route::Remote, cx)))
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

        let genres_active = matches!(self.state.read(cx).current_route(), Some(Route::Genres));
        let active = self.state.read(cx).current_route().and_then(Route::section);
        let on_remote = route.as_ref() == Some(&Route::Remote);
        let on_party = route.as_ref() == Some(&Route::Party);
        // D17: a server that does not have a feature does not get its entry —
        // an older server's 404 is what says so (`nz_api::features`).
        let features = self.state.read(cx).features.clone();
        let sidebar = sidebar(
            active,
            is_admin,
            genres_active,
            on_remote,
            on_party,
            &features,
            cx,
        )
        .into_any_element();

        // A screen reached by a link (a remote-control URL, a folder in the
        // store) on a server that does not have its feature says so, with the
        // cause and the way back — not an endless spinner (rule 3).
        let absent = route.as_ref().and_then(|route| features.absent(route));
        let content = match (&self.screen, absent) {
            (_, Some(label)) => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(rems(0.75))
                .p(rems(2.))
                .child(
                    icon(IconName::Info)
                        .size(rems(1.75))
                        .text_color(theme::secondary(cx)),
                )
                .child(
                    div()
                        .text_size(rems(1.125))
                        .font_weight(gpui_kit::FontWeight::BOLD)
                        .child(format!("{label} n'est pas sur ce serveur")),
                )
                .child(
                    div()
                        .max_w(rems(34.))
                        .text_center()
                        .text_size(rems(0.8125))
                        .text_color(theme::secondary(cx))
                        .child(
                            "Ce serveur est plus ancien que cette fonction. Les autres \
                             écrans continuent de marcher ; la lecture, la bibliothèque \
                             et la file ne dépendent pas de lui.",
                        ),
                )
                .into_any_element(),
            (Some((_route, view)), None) => {
                let reduced = self.state.read(cx).reduced_motion;
                let duration = theme::motion_ms(theme::SCREEN_MS, reduced);
                div()
                    .relative()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .child(div().size_full().child(view.clone()).with_animation(
                        ("screen", nav_version),
                        Animation::new(duration),
                        move |element, delta| element.opacity(delta).ml(px(24.0 * (1.0 - delta))),
                    ))
                    .into_any_element()
            }
            (None, None) => div().size_full().into_any_element(),
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

        root = root.child(header).child(body).child(player_bar::render(
            self.state.clone(),
            window,
            &self.bar_covers,
            &self.bar_events,
            cx,
        ));

        // « Contrôlé par … » — always on screen while somebody drives this
        // player, with the one tap that takes it back (the SPA's
        // `RemoteHostChip`). Above the bar, never over the transport.
        if let Some(chip) = remote_chip(window, cx) {
            root = root.child(chip);
        }

        if !toasts.is_empty() {
            root = root.child(toast::render(&toasts, cx));
        }
        if self.shortcuts_open {
            root = root.child(shortcuts_overlay(cx));
        }
        // The measurement seam: one line per painted frame, at 0.4 µs.
        crate::bench::frame(render_started);
        root.into_any_element()
    }
}

/// « Contrôlé par … » over the player bar: who is driving this player, and the
/// one tap that takes it back (`RemoteHostChip.svelte`). Drawn by the shell
/// because it must be on screen from every route, and never over the transport
/// itself: it sits just above the bar.
fn remote_chip(
    window: &mut Window,
    cx: &mut Context<RootView>,
) -> Option<gpui_kit::Stateful<gpui_kit::Div>> {
    let host = remote_host::RemoteHost::get()?;
    let status = host.status();
    if status.live() == 0 {
        return None;
    }
    let who = status.who();
    let level = status.level().unwrap_or_default();
    let bottom = theme::player_height(window) + px(12.);
    Some(
        div()
            .id("remote-chip")
            .debug_selector(|| "remote-chip".to_owned())
            .absolute()
            .left_0()
            .right_0()
            .bottom(bottom)
            .flex()
            .justify_center()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.25))
                    .p(rems(0.25))
                    .border_1()
                    .border_color(theme::accent(cx))
                    .bg(theme::chrome(cx))
                    .child(
                        button("remote-chip-who", "", ButtonVariant::Secondary, cx)
                            .flex()
                            .items_center()
                            .gap(rems(0.5))
                            .pr(rems(0.75))
                            .on_click(cx.listener(|this, _, _, cx| this.go(Route::Remote, cx)))
                            .child(div().size(rems(0.5)).rounded_full().bg(theme::success(cx)))
                            .child(icon(IconName::Cast).size(rems(0.875)))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .items_start()
                                    .child(
                                        div()
                                            .text_size(rems(0.75))
                                            .font_weight(FontWeight::BOLD)
                                            .child(who),
                                    )
                                    .child(
                                        div()
                                            .text_size(rems(0.625))
                                            .text_color(theme::secondary(cx))
                                            .child(level.to_owned()),
                                    ),
                            ),
                    )
                    .child(
                        button("remote-chip-cut", "Couper", ButtonVariant::Danger, cx)
                            .child(icon(IconName::Close).size(rems(0.75)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(host) = remote_host::RemoteHost::get() {
                                    host.cut_all();
                                }
                                this.state.update(cx, |state, cx| {
                                    state.toasts.push(
                                        crate::toast::ToastKind::Info,
                                        "Contrôle à distance coupé.",
                                    );
                                    cx.notify();
                                });
                            })),
                    ),
            ),
    )
}

fn sidebar(
    active: Option<Section>,
    is_admin: bool,
    genres_active: bool,
    on_remote: bool,
    on_party: bool,
    features: &crate::state::Features,
    cx: &mut Context<RootView>,
) -> impl IntoElement + use<> {
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
        // Genres is not one of `Section`'s entries (that enum predates D16), so
        // its own row carries the active state and Releases steps back.
        let selected = active == Some(section) && !(genres_active && section == Section::Releases);
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
        if section == Section::Releases
            && is_admin
            && !features.hidden(crate::state::Feature::Genres)
        {
            column = column.child(
                div()
                    .id("nav-Genres")
                    .flex()
                    .items_center()
                    .gap(rems(0.625))
                    .h(rems(2.))
                    .px(rems(0.625))
                    .border_l_1()
                    .border_color(if genres_active {
                        theme::accent(cx)
                    } else {
                        theme::sidebar(cx)
                    })
                    .bg(if genres_active {
                        theme::selected(cx)
                    } else {
                        theme::sidebar(cx)
                    })
                    .text_color(if genres_active {
                        theme::text(cx)
                    } else {
                        theme::secondary(cx)
                    })
                    .text_size(rems(0.8125))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::selected(cx)).text_color(theme::text(cx)))
                    .on_click(cx.listener(|this, _, _, cx| this.go(Route::Genres, cx)))
                    .child(icon(IconName::Tag).size(rems(1.125)))
                    .child(div().child("Genres")),
            );
        }
    }
    // Contrôle distant (lot D15a): a link lends THIS player, so the way in is the
    // sidebar, next to Réglages, and the glyph lights up when a controller is on.
    // D17: an entry whose feature the server lacks is not drawn at all.
    if !features.hidden(crate::state::Feature::Remote) {
        let remote_live = remote_host::RemoteHost::get()
            .map(|host| host.status().live())
            .unwrap_or(0);
        column = column.child(
            div()
                .id("nav-remote")
                .flex()
                .items_center()
                .gap(rems(0.625))
                .h(rems(2.))
                .px(rems(0.625))
                .text_size(rems(0.8125))
                .text_color(if on_remote {
                    theme::text(cx)
                } else {
                    theme::secondary(cx)
                })
                .cursor_pointer()
                .hover(|style| style.text_color(theme::text(cx)))
                .on_click(cx.listener(|this, _, _, cx| this.go(Route::Remote, cx)))
                .child(
                    icon(IconName::Cast)
                        .size(rems(1.125))
                        .text_color(if remote_live > 0 {
                            theme::accent(cx)
                        } else if on_remote {
                            theme::text(cx)
                        } else {
                            theme::secondary(cx)
                        }),
                )
                .child(div().child("Contrôle distant")),
        );
    }
    // Écoute partagée (lot D15b): the same place as Contrôle distant — both are
    // about the room this player is in, and both light up while they are live.
    if !features.hidden(crate::state::Feature::Party) {
        let party_live = crate::wiring::party_host::PartyHost::get()
            .map(|host| !host.id().is_empty())
            .unwrap_or(false)
            || crate::wiring::party_guest::Guest::get()
                .map(|guest| {
                    matches!(
                        guest.view().phase,
                        crate::wiring::party_guest::Phase::Joining
                            | crate::wiring::party_guest::Phase::Live
                    )
                })
                .unwrap_or(false);
        column =
            column.child(
                div()
                    .id("nav-party")
                    .flex()
                    .items_center()
                    .gap(rems(0.625))
                    .h(rems(2.))
                    .px(rems(0.625))
                    .text_size(rems(0.8125))
                    .text_color(if on_party {
                        theme::text(cx)
                    } else {
                        theme::secondary(cx)
                    })
                    .cursor_pointer()
                    .hover(|style| style.text_color(theme::text(cx)))
                    .on_click(cx.listener(|this, _, _, cx| this.go(Route::Party, cx)))
                    .child(icon(IconName::PartyPopper).size(rems(1.125)).text_color(
                        if party_live {
                            theme::accent(cx)
                        } else if on_party {
                            theme::text(cx)
                        } else {
                            theme::secondary(cx)
                        },
                    ))
                    .child(div().child("Écoute partagée")),
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
            crate::theme::init(cx);
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
            crate::theme::init(cx);
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
    use std::path::{Path, PathBuf};

    use gpui_kit::{Hsla, rgb};
    use gpui_omarchy::ActiveTheme;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neezer-theme-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// One Omarchy palette, the shape `omarchy-theme-set` writes.
    fn write_colors(root: &Path, accent: &str) {
        std::fs::write(
            root.join("colors.toml"),
            format!(
                "mode = 'dark'\nbackground = '#1a1b26'\nforeground = '#a9b1d6'\n\
                 accent = '{accent}'\nred = '#f7768e'\ngreen = '#9ece6a'\n\
                 yellow = '#e0af68'\nselection = '#292e42'\n"
            ),
        )
        .expect("write the temporary palette");
    }

    /// The shell reads every colour through `cx.omarchy()`; a theme applied at
    /// runtime must reach those tokens, and gpui-omarchy's `apply_palette` is
    /// what refreshes the windows (`omarchy-theme-set` rewrites the palette and
    /// the watcher calls exactly this).
    #[gpui::test]
    async fn a_theme_change_reaches_the_shell_tokens(cx: &mut gpui::TestAppContext) {
        cx.update(crate::theme::init);
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

    /// **A theme file rewritten while a GPUI test runs reaches the shell, and
    /// no observer points at the person's configuration.**
    ///
    /// This is the shape of the flake SC4 reported: the person's shell writes
    /// `~/.config/omarchy/shell.json` beside their theme, and a test whose
    /// theme init watched that directory was aborted at its end by
    /// « Detected activity on thread Some("notify-rs inotify loop") » — the
    /// deterministic scheduler cannot be woken from notify's thread. The palette
    /// here comes from a temporary file the test owns, and the `shell.json`
    /// beside it is rewritten with it; a wake from that writer would be recorded
    /// and fail this test, and the palette the file ends on must reach
    /// `cx.omarchy()` when the file is read.
    ///
    /// **The write is an EVENT the test drives and waits for, never a rate it
    /// hopes for.** This test used to let a thread rewrite the file every 2 ms
    /// and then require ten rewrites in fifty pumps: that floor measured the
    /// runner's scheduler, not the theme, and it is what turned `cargo test +
    /// window` red on a loaded machine — CI runs 44219296421 and 44216181175
    /// failed it with **9** and **3** rewrites while every theme assertion in it
    /// held. Now the test asks the writer for one named palette at a time and
    /// pumps the deterministic scheduler until the writer reports the file
    /// written, so the bytes are landing *while* the scheduler runs on any
    /// machine, and the assertion is the behaviour: the shell paints the
    /// palette the file holds, twice, with both accents.
    #[gpui::test]
    async fn a_theme_file_written_while_the_test_runs_reaches_the_shell(
        cx: &mut gpui::TestAppContext,
    ) {
        // Colours no Omarchy palette carries, so a stale or wrong palette
        // fails the accent assertion itself instead of matching by accident
        // (Tokyo Night's accent IS `#7aa2f7`, which the old pair used).
        const FIRST: &str = "#123456";
        const SECOND: &str = "#abcdef";
        let root = temp_dir("written");
        write_colors(&root, FIRST);
        std::fs::write(root.join("shell.json"), "{}\n").expect("write the temporary shell.json");
        cx.update(|cx| crate::theme::init_at(cx, &root).expect("the temporary palette"));
        assert_eq!(
            cx.update(|cx| crate::theme::accent(cx)),
            Hsla::from(rgb(0x123456)),
            "the shell starts on the temporary palette"
        );

        // The writer owns the file, the test owns the rhythm: one channel asks
        // it for a palette, the other says the bytes are on disk. No sleep and
        // no count decides the outcome.
        let (orders, asked) = std::sync::mpsc::channel::<&'static str>();
        let (wrote, written) = std::sync::mpsc::channel::<&'static str>();
        let writer = {
            let root = root.clone();
            std::thread::spawn(move || {
                for accent in asked {
                    write_colors(&root, accent);
                    std::fs::write(
                        root.join("shell.json"),
                        format!("{{\"accent\":\"{accent}\"}}\n"),
                    )
                    .ok();
                    if wrote.send(accent).is_err() {
                        break;
                    }
                }
            })
        };

        for accent in [SECOND, FIRST] {
            orders.send(accent).expect("the writer thread");
            // Pump until the write LANDS: the scheduler therefore runs while
            // the theme file is being rewritten, which is the race this test is
            // about, whatever the machine's speed. The pump budget is a hang
            // guard, far above a write (a parked scheduler pumps in microseconds
            // and the writer needs one write per round).
            let mut pumps = 0u32;
            let landed = loop {
                cx.run_until_parked();
                match written.try_recv() {
                    Ok(landed) => break landed,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        pumps += 1;
                        assert!(
                            pumps < 1_000_000,
                            "the writer thread never finished one palette"
                        );
                        std::thread::yield_now();
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        panic!("the writer thread died before writing {accent}")
                    }
                }
            };
            assert_eq!(landed, accent, "the writer landed the palette asked for");

            // The shell paints with the file, not with a copy taken at start-up.
            let (painted, shell, name) = cx.update(|cx| {
                crate::theme::apply_from(cx, &root).expect("the temporary palette");
                (
                    crate::theme::accent(cx),
                    crate::theme::shell(cx),
                    cx.omarchy().name.to_string(),
                )
            });
            assert_eq!(
                painted,
                Hsla::from(rgb(if accent == SECOND { 0xabcdef } else { 0x123456 })),
                "the shell's accent is the palette the file holds ({accent})"
            );
            assert_ne!(
                shell, painted,
                "the palette is a whole theme, not one token"
            );
            assert_eq!(name, "Test");
        }
        drop(orders);
        writer.join().expect("the writer thread");

        std::fs::remove_dir_all(&root).ok();
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
            crate::theme::init(cx);
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

    /// A screen built and dropped must not be KEPT by its own reply channel.
    ///
    /// Every screen owns one `events::Sender` and one foreground task awaiting
    /// the other half. Until the channel closed when its last sender dropped,
    /// nothing ever woke that task: GPUI kept it — and the screen state the
    /// future had captured — for the life of the process, one screen per
    /// navigation. Measured in the release app on the isolated display, 1 000
    /// round trips between Accueil and Bibliothèque: **+321 700 KB of RSS**
    /// (+161 KB per screen built); after the fix: +4 100 KB (+4 KB), which is
    /// the shell's own atlas, not the screens.
    ///
    /// The number is BYTES the calling thread still holds (`testmem`'s counting
    /// allocator), never `VmRSS`: the suite runs its tests in parallel in one
    /// process, so the process's resident set moves with its neighbours (PR
    /// #236: the same page read +8 404 KB under load). A budget in bytes
    /// cannot move with a neighbour, and a leak of 161 KB per screen still
    /// fails it 40 times over.
    #[gpui::test]
    async fn building_a_screen_a_thousand_times_keeps_nothing(cx: &mut gpui::TestAppContext) {
        use crate::testmem::{Meter, retained_bytes};

        cx.update(|cx| {
            crate::theme::init(cx);
            crate::keymap::install(cx);
        });
        let dir = temp_dir("nav-guard");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (events_tx, _events_rx) = event_channel();
        let config = AppConfig {
            start_audio: false,
            start_mpris: false,
            ..AppConfig::for_test(&dir)
        };
        let runtime_handle = runtime.handle().clone();
        let state =
            cx.update(|cx| cx.new(move |_| AppState::new(config, runtime_handle, events_tx, None)));
        let routes = [
            Route::Home,
            Route::Library {
                tab: crate::nav::LibraryTab::Playlists,
            },
        ];
        // A screen's requests run on the runtime; without driving it they stay
        // parked, and a parked task holds its own clone of the screen's reply
        // sender — which is the app's real case (the runtime runs) and must be
        // the test's too, or the measurement is of a frozen process.
        let settle = |runtime: &tokio::runtime::Runtime, cx: &mut gpui::TestAppContext| {
            runtime.block_on(async {
                tokio::time::sleep(Duration::from_millis(1)).await;
            });
            cx.run_until_parked();
        };
        // A first pass warms what the shell allocates once (the text system's
        // font handles): a screen's own weight is what the second pass measures.
        for route in routes.iter() {
            let view = cx.update(|cx| crate::screens::build(route, state.clone(), cx));
            drop(view);
            settle(&runtime, cx);
        }
        let before = Meter::now();
        for _ in 0..1_000 {
            for route in routes.iter() {
                let view = cx.update(|cx| crate::screens::build(route, state.clone(), cx));
                drop(view);
                settle(&runtime, cx);
            }
        }
        let kept = before.since();
        println!(
            "navigation: 2 000 screens kept {kept} B (thread holds {})",
            retained_bytes()
        );
        // Measured 892 936 B for 2 000 screens (446 B each: GPUI's own task
        // arena settles a little later than this test can see). The leak it
        // guards was 161 KB per screen in the release app — 2 000 of them held
        // 322 MB — so the budget is 2 MiB: 2.3× the measured weight and 160×
        // tighter than the leak. A screen that starts keeping a list, a decoded
        // image or a response fails here at once.
        const BUDGET_BYTES: u64 = 2 * 1024 * 1024;
        assert!(
            kept < BUDGET_BYTES,
            "2 000 screen builds must keep under {BUDGET_BYTES} B, they kept {kept} B"
        );
        std::fs::remove_dir_all(dir).ok();
        drop(runtime);
    }
}
