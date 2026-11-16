//! Réglages: every section, one screen.
//!
//! D7 builds the table and the section stubs; D14a and D14b own the bodies of
//! their own files and of this module (they may dispatch a section to its own
//! view) while keeping `Screen::new(section, state, cx)`, which is what the
//! route table calls.

pub mod account;
pub mod appearance;
pub mod discovery;
pub mod downloads;
pub mod help;
pub mod playback;
pub mod server;
pub mod shortcuts;
pub mod sources;
pub mod transcribe;
pub mod users;
pub mod viz;

use gpui_kit::AppContext as _;
use gpui_kit::{AnyView, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div};

use crate::nav::{Route, SettingsSection};
use crate::state::AppState;

/// The Réglages view for one section. It is the stub until D14a lands; the
/// constructor's shape is the contract.
pub struct Screen {
    inner: AnyView,
}

impl Screen {
    pub fn new(section: SettingsSection, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let route = Route::Settings { section };
        let inner: AnyView = cx
            .new(|cx| crate::screens::StubScreen::new(route, state, cx))
            .into();
        Self { inner }
    }
}

impl Render for Screen {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.inner.clone())
    }
}
