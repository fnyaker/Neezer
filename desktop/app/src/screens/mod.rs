//! The screen table: one module per route, one constructor for the shell.
//!
//! D7 creates every module here and the stub each one starts as. A later lot
//! replaces the body of its own file and keeps the shape this module calls:
//! `Screen::new(route, state, cx)` (settings sections go through
//! `settings/mod.rs`). That is the whole reason this file can be frozen after
//! D7 — a new screen is a new file declared here once, not an edit to the
//! shell.
//!
//! The stub renders the route's own title and « Bientôt », so every navigation
//! target is reachable, measurable and screenshot-able before its screen lands.

pub mod album;
pub mod artist;
pub mod credits;
pub mod genres;
pub mod history;
pub mod home;
pub mod import;
pub mod library;
pub mod now_playing;
pub mod onboarding;
pub mod party;
pub mod peer_sets;
pub mod playlist;
pub mod podcasts;
pub mod queue;
pub mod recap;
pub mod releases;
pub mod remote;
pub mod search;
pub mod settings;
pub mod show;
pub mod viz;

use gpui_kit::{
    AnyView, App, AppContext as _, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Window, div, rems,
};
use gpui_omarchy::{IconName, icon};

use crate::nav::{Route, SettingsSection};
use crate::state::AppState;
use crate::theme;

/// The stub every screen starts as, and the one the route table can build.
pub struct StubScreen {
    route: Route,
    #[allow(dead_code)]
    state: Entity<AppState>,
    _subscription: Subscription,
}

impl StubScreen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        // A stub redraws when the session changes (login/logout, offline), like
        // every real screen will.
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            route,
            state,
            _subscription: subscription,
        }
    }

    pub fn route(&self) -> &Route {
        &self.route
    }
}

impl Render for StubScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        stub(&self.route, cx)
    }
}

/// What a screen that does not exist yet shows: its own title, « Bientôt », and
/// a sentence that says the shell is not broken.
pub fn stub(route: &Route, cx: &App) -> impl IntoElement {
    div()
        .id("stub-screen")
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(rems(0.75))
        .p(rems(2.))
        .font_family(theme::font(cx))
        .text_color(theme::text(cx))
        .child(
            icon(IconName::Construction)
                .size(rems(2.))
                .text_color(theme::accent(cx)),
        )
        .child(
            div()
                .text_size(rems(1.5))
                .font_weight(FontWeight::BOLD)
                .child(route.title()),
        )
        .child(
            div()
                .px(rems(0.5))
                .py(rems(0.125))
                .border_1()
                .border_color(theme::border(cx))
                .text_size(rems(0.6875))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::secondary(cx))
                .child("Bientôt"),
        )
        .child(
            div()
                .max_w(rems(34.))
                .text_center()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child(
                    "Cet écran arrive dans une prochaine version. \
                     La navigation, la lecture et la file fonctionnent déjà.",
                ),
        )
        .child(div().h(rems(0.125)).w(rems(4.)).bg(theme::accent(cx)))
}

/// Build the view for `route`. Only called when the route changes; a screen
/// keeps its own state and observes `AppState` afterwards.
pub fn build(route: &Route, state: Entity<AppState>, cx: &mut App) -> AnyView {
    match route {
        Route::Login => cx
            .new(|cx| StubScreen::new(route.clone(), state, cx))
            .into(),
        Route::Onboarding => cx
            .new(|cx| onboarding::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Home => cx
            .new(|cx| home::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Search { .. } => cx
            .new(|cx| search::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Artist { .. } => cx
            .new(|cx| artist::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Album { .. } => cx
            .new(|cx| album::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Playlist { .. } => cx
            .new(|cx| playlist::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Library { .. } => cx
            .new(|cx| library::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Podcasts => cx
            .new(|cx| podcasts::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Show { .. } => cx
            .new(|cx| show::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Releases => cx
            .new(|cx| releases::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Recap { .. } => cx
            .new(|cx| recap::Screen::new(route.clone(), state, cx))
            .into(),
        Route::History => cx
            .new(|cx| history::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Genres => cx
            .new(|cx| genres::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Credits { .. } => cx
            .new(|cx| credits::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Import { .. } => cx
            .new(|cx| import::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Party => cx
            .new(|cx| party::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Remote => cx
            .new(|cx| remote::Screen::new(route.clone(), state, cx))
            .into(),
        Route::NowPlaying => cx
            .new(|cx| now_playing::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Queue => cx
            .new(|cx| queue::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Viz => cx
            .new(|cx| viz::Screen::new(route.clone(), state, cx))
            .into(),
        Route::Settings { section } => cx
            .new(|cx| settings::Screen::new(*section, state, cx))
            .into(),
    }
}

/// Every route the shell can show, in the order the inventory lists them. The
/// test walks it through [`build`], so a route without a screen cannot compile
/// past this list.
pub fn all_routes() -> Vec<Route> {
    use crate::nav::{CreditKind, LibraryTab};
    vec![
        Route::Login,
        Route::Onboarding,
        Route::Home,
        Route::Search {
            query: "kaya".into(),
        },
        Route::Artist { id: "1".into() },
        Route::Album { id: "2".into() },
        Route::Playlist { id: "3".into() },
        Route::Library {
            tab: LibraryTab::Playlists,
        },
        Route::Podcasts,
        Route::Show { id: "4".into() },
        Route::Releases,
        Route::Recap { year: None },
        Route::History,
        Route::Genres,
        Route::Credits {
            kind: CreditKind::Track,
            id: "5".into(),
        },
        Route::Import { url: None },
        Route::Party,
        Route::Remote,
        Route::NowPlaying,
        Route::Queue,
        Route::Viz,
    ]
    .into_iter()
    .chain(
        SettingsSection::USER
            .iter()
            .chain(SettingsSection::ADMIN.iter())
            .map(|section| Route::Settings { section: *section }),
    )
    .collect()
}

/// The shared "this screen is a stub" title, for a test that reads the label
/// without a renderer.
pub fn stub_label() -> SharedString {
    "Bientôt".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_of_the_inventory_has_a_screen() {
        // One variant per route the plan's inventory names. If a route were
        // added to `Route` without a match arm, `build` would not compile; this
        // catches the other direction (a screen the app cannot reach).
        let routes = all_routes();
        assert!(routes.iter().any(|route| matches!(route, Route::Home)));
        assert!(routes.iter().any(|route| matches!(route, Route::Genres)));
        assert!(routes.iter().any(|route| matches!(
            route,
            Route::Settings {
                section: SettingsSection::Transcribe
            }
        )));
        assert_eq!(
            routes
                .iter()
                .filter(|route| matches!(route, Route::Settings { .. }))
                .count(),
            SettingsSection::USER.len() + SettingsSection::ADMIN.len()
        );
    }

    /// D17's access-path audit: every screen the shell can build must be
    /// reachable by someone who has never seen it — a sidebar row, a keystroke
    /// the `?` overlay prints, or a link on another screen. `Route::Queue` was
    /// the one hole (the `q` panel drew the same screen, so nothing noticed),
    /// and `g q` is its way in.
    #[test]
    fn every_route_of_the_table_has_a_way_in() {
        use crate::nav::{Access, Route};

        for route in all_routes() {
            match route.access() {
                Access::Automatic => assert!(
                    matches!(route, Route::Login | Route::Onboarding),
                    "{route:?} has no way in: a route is only Automatic when the app \
                     lands on it by itself"
                ),
                Access::Shortcut(keys) => assert!(
                    crate::keymap::SHORTCUTS.iter().any(|row| row.keys == keys),
                    "{route:?} is reached by « {keys} », which the ? overlay does not list"
                ),
                Access::Sidebar => {}
                Access::Link(where_) => assert!(!where_.is_empty(), "{route:?} names no link"),
            }
        }
        assert_eq!(
            Route::Queue.access(),
            Access::Shortcut("g q"),
            "the full-screen queue is reachable, not only as a side panel"
        );
    }

    #[test]
    fn a_stub_route_names_itself() {
        assert_eq!(stub_label(), "Bientôt");
        assert_eq!(Route::Home.title(), "Accueil");
        assert_eq!(
            Route::Library {
                tab: crate::nav::LibraryTab::Favorites
            }
            .title(),
            "Bibliothèque · Favoris"
        );
    }
}
