//! The login screen: the server's address, an account, and a reason when it
//! fails.
//!
//! It is a real screen, not a gate in `main`: it validates the address before a
//! request, warns when a password would cross an unencrypted link to a public
//! host (`docs/agents/desktop.md`, *Auth and security*), keeps the person's
//! typing while the request runs, and prints the server's own cause when it
//! refuses. `Entrée` from any field submits; Tab moves between them through
//! gpui-omarchy's focus scope.

use gpui_kit::InteractiveElement as _;
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, FontWeight, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, Window, div, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, focus_scope, icon, input};

use crate::state::AppState;
use crate::theme;
use crate::wiring::net;

pub struct LoginScreen {
    state: Entity<AppState>,
    server: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    focus: FocusHandle,
    warning: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl LoginScreen {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let address = state.read(cx).server.clone();
        let server = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(address)
                .placeholder("http://192.168.1.20:5000")
        });
        let username = cx.new(|cx| InputState::new(window, cx).placeholder("identifiant"));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let focus = cx.focus_handle();

        let mut subscriptions = Vec::new();
        for field in [&server, &username, &password] {
            subscriptions.push(cx.subscribe(field, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.submit(cx);
                } else if matches!(event, InputEvent::Change) {
                    this.refresh_warning(cx);
                }
            }));
        }
        subscriptions.push(cx.observe(&state, |_, _, cx| cx.notify()));

        // The first field has the focus: a person arrives to type, not to Tab.
        server.update(cx, |state, cx| state.focus(window, cx));

        let mut this = Self {
            state,
            server,
            username,
            password,
            focus,
            warning: None,
            _subscriptions: subscriptions,
        };
        this.refresh_warning(cx);
        this
    }

    fn refresh_warning(&mut self, cx: &mut Context<Self>) {
        let address = self.server.read(cx).value();
        self.warning = net::plain_http_warning(address.as_ref()).map(SharedString::from);
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.state.read(cx).login.pending {
            return;
        }
        let server = self.server.read(cx).value().to_string();
        let username = self.username.read(cx).value().to_string();
        let password = self.password.read(cx).value().to_string();
        self.state.update(cx, |state, _| {
            state.login(server, username, password);
        });
    }
}

impl Render for LoginScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pending = self.state.read(cx).login.pending;
        let error = self.state.read(cx).login.error.clone();
        let address = self.server.read(cx).value().to_string();
        let secure_remote = net::plain_http_warning(&address).is_some();

        let secondary = theme::secondary(cx);
        let label = move |text: &'static str| {
            div()
                .text_size(rems(0.625))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(secondary)
                .child(text)
        };

        let mut card = div()
            .id("login-card")
            .flex()
            .flex_col()
            .gap(rems(1.))
            .w(rems(26.))
            .p(rems(1.75))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .font_family(theme::font(cx))
            .text_color(theme::text(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.625))
                    .child(
                        icon(IconName::AudioLines)
                            .size(rems(1.75))
                            .text_color(theme::accent(cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(rems(1.375))
                                    .font_weight(FontWeight::BOLD)
                                    .child("Neezer"),
                            )
                            .child(
                                div()
                                    .text_size(rems(0.75))
                                    .text_color(theme::secondary(cx))
                                    .child("Votre musique, votre serveur, votre lecteur."),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.375))
                    .child(label("Serveur"))
                    .child(input("login-server", &self.server, _window, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.375))
                    .child(label("Identifiant"))
                    .child(input("login-username", &self.username, _window, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.375))
                    .child(label("Mot de passe"))
                    .child(input("login-password", &self.password, _window, cx)),
            );

        if let Some(warning) = &self.warning {
            card = card.child(
                div()
                    .flex()
                    .items_start()
                    .gap(rems(0.5))
                    .text_size(rems(0.6875))
                    .text_color(theme::warning(cx))
                    .child(icon(IconName::ShieldAlert).size(rems(1.)).mt(rems(0.0625)))
                    .child(div().flex_1().child(warning.clone())),
            );
        }
        if let Some(error) = &error {
            card = card.child(
                div()
                    .id("login-error")
                    .flex()
                    .items_start()
                    .gap(rems(0.5))
                    .p(rems(0.5))
                    .border_1()
                    .border_color(theme::danger(cx))
                    .text_size(rems(0.75))
                    .text_color(theme::danger(cx))
                    .child(icon(IconName::CircleAlert).size(rems(1.)).mt(rems(0.0625)))
                    .child(div().flex_1().child(error.clone())),
            );
        }

        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    button(
                        "login-submit",
                        if pending {
                            "Connexion…"
                        } else {
                            "Se connecter"
                        },
                        ButtonVariant::Primary,
                        cx,
                    )
                    .disabled(pending)
                    .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                )
                .child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .child(if pending {
                            "Le serveur vérifie votre compte."
                        } else {
                            "Entrée pour vous connecter."
                        }),
                ),
        );

        if secure_remote {
            // The warning is not decoration: it is the only place the app says
            // a password is about to cross a public link in clear.
            card = card.border_color(theme::warning(cx));
        }

        focus_scope("login")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::shell(cx))
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &crate::actions::SubmitLogin, _, cx| this.submit(cx)))
            .child(card)
    }
}
