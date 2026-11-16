//! Toasts: what failed, why, and the one gesture that retries it.
//!
//! Rule 2 and rule 3 in the small: an action that cannot finish says so in a
//! surface that does not steal focus, names the cause the server gave
//! (`ApiError::cause_fr`), and offers « Réessayer » when retrying is meaningful.
//! A toast is never the only report of a screen that cannot load — that is
//! [`crate::load_failed`] — it is the report of a *command* that failed.
//!
//! Stacking and timing are owned here: errors stay ten seconds, information
//! five, and a prune runs on the shell's own 500 ms tick (the same task that
//! refreshes the player bar), so nothing sleeps in a request thread.

use std::time::{Duration, Instant};

use gpui_kit::{
    Context, ElementId, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    Styled, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};

use crate::app::RootView;
use crate::theme;

/// What a toast is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    /// A command succeeded, or information the person asked for.
    Info,
    /// A command failed. The message is the cause, in French.
    Error,
}

/// What « Réessayer » should repeat. The shell owns the three retries a toast
/// can offer; a screen with its own retry uses [`crate::load_failed`] instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retry {
    /// Ask the server again whether it is reachable.
    Probe,
    /// Try `GET /api/me` again (a session that could not be restored).
    Restore,
    /// Re-open the track the engine could not load.
    Play,
}

/// One message.
#[derive(Clone, Debug)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub message: SharedString,
    pub retry: Option<Retry>,
    pub created: Instant,
    pub life: Duration,
}

impl Toast {
    fn expired(&self, now: Instant) -> bool {
        now.duration_since(self.created) >= self.life
    }
}

/// The stack, owned by `AppState`.
#[derive(Debug, Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next: u64,
}

impl Toasts {
    /// Errors live longer: a cause worth reading is worth keeping on screen.
    pub fn push(&mut self, kind: ToastKind, message: impl Into<SharedString>) -> u64 {
        self.push_with(kind, message, None)
    }

    pub fn push_with(
        &mut self,
        kind: ToastKind,
        message: impl Into<SharedString>,
        retry: Option<Retry>,
    ) -> u64 {
        let message = message.into();
        // The same failure twice (a dead link, two taps) is one toast: a stack
        // of identical messages hides the one that differs.
        if let Some(existing) = self
            .items
            .iter_mut()
            .find(|toast| toast.message == message && toast.kind == kind)
        {
            existing.created = Instant::now();
            existing.retry = retry.or(existing.retry);
            return existing.id;
        }
        self.next += 1;
        let life = match kind {
            ToastKind::Error => Duration::from_secs(10),
            ToastKind::Info => Duration::from_secs(5),
        };
        self.items.push(Toast {
            id: self.next,
            kind,
            message,
            retry,
            created: Instant::now(),
            life,
        });
        self.next
    }

    pub fn dismiss(&mut self, id: u64) {
        self.items.retain(|toast| toast.id != id);
    }

    /// Drop what outlived its life. Returns whether anything went, which is
    /// what tells the shell to repaint.
    pub fn prune(&mut self, now: Instant) -> bool {
        let before = self.items.len();
        self.items.retain(|toast| !toast.expired(now));
        self.items.len() != before
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn items(&self) -> &[Toast] {
        &self.items
    }

    /// The retry of the newest failed command, for the `Retry` action.
    pub fn top_retry(&self) -> Option<Retry> {
        self.items.iter().rev().find_map(|toast| toast.retry)
    }
}

/// Draw the stack in the bottom-right corner, above the player bar.
pub fn render(items: &[Toast], cx: &mut Context<RootView>) -> impl IntoElement + use<> {
    let mut column = div()
        .id("toasts")
        .absolute()
        .right(rems(1.5))
        .bottom(rems(6.))
        .flex()
        .flex_col()
        .items_end()
        .gap(rems(0.5))
        .max_w(px(420.));
    for toast in items {
        let accent = match toast.kind {
            ToastKind::Error => theme::danger(cx),
            ToastKind::Info => theme::accent(cx),
        };
        let id = toast.id;
        let mut card = div()
            .id(ElementId::Name(format!("toast-{id}").into()))
            .flex()
            .items_center()
            .gap(rems(0.75))
            .px(rems(0.875))
            .py(rems(0.625))
            .border_1()
            .border_color(accent)
            .bg(theme::chrome(cx))
            .text_color(theme::text(cx))
            .text_size(rems(0.75))
            .font_family(theme::font(cx))
            .child(
                icon(match toast.kind {
                    ToastKind::Error => IconName::CircleAlert,
                    ToastKind::Info => IconName::Info,
                })
                .size(rems(1.))
                .text_color(accent),
            )
            .child(div().flex_1().min_w_0().child(toast.message.clone()))
            .child(
                div()
                    .id(ElementId::Name(format!("toast-close-{id}").into()))
                    .cursor_pointer()
                    .text_color(theme::secondary(cx))
                    .hover(|style| style.text_color(theme::text(cx)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.state().update(cx, |state, cx| {
                                state.toasts.dismiss(id);
                                cx.notify();
                            });
                        }),
                    )
                    .child(icon(IconName::Close).size(rems(0.75))),
            );
        if toast.retry.is_some() {
            card = card.child(
                button(("toast-retry", id), "Réessayer", ButtonVariant::Outline, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
            );
        }
        column = column.child(card);
    }
    column
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identical_failure_is_one_toast_not_a_stack() {
        let mut toasts = Toasts::default();
        let a = toasts.push(ToastKind::Error, "Serveur injoignable");
        let b = toasts.push(ToastKind::Error, "Serveur injoignable");
        assert_eq!(a, b);
        assert_eq!(toasts.len(), 1);
        toasts.push(ToastKind::Info, "Serveur injoignable");
        assert_eq!(toasts.len(), 2, "a different kind is a different message");
    }

    #[test]
    fn errors_outlive_information_and_pruning_is_explicit() {
        let mut toasts = Toasts::default();
        toasts.push(ToastKind::Error, "échec");
        toasts.push(ToastKind::Info, "fait");
        assert_eq!(toasts.items()[0].life, Duration::from_secs(10));
        assert_eq!(toasts.items()[1].life, Duration::from_secs(5));
        // The clock is passed in; nothing sleeps in a test.
        assert!(!toasts.prune(Instant::now()));
        assert!(toasts.prune(Instant::now() + Duration::from_secs(11)));
        assert!(toasts.is_empty());
    }

    #[test]
    fn the_newest_retry_is_the_one_the_retry_action_uses() {
        let mut toasts = Toasts::default();
        assert_eq!(toasts.top_retry(), None);
        toasts.push_with(ToastKind::Error, "hors ligne", Some(Retry::Probe));
        toasts.push_with(ToastKind::Error, "lecture", Some(Retry::Play));
        assert_eq!(toasts.top_retry(), Some(Retry::Play));
        let id = toasts.items()[1].id;
        toasts.dismiss(id);
        assert_eq!(toasts.top_retry(), Some(Retry::Probe));
    }
}
