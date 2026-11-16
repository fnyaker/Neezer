//! Réglages → Compte: who is signed in, and where their lists can be sent.
//!
//! The SPA's *Compte* card and `ServicesSettings.svelte`: the account itself
//! (name, role, sign out), the streaming services a playlist can be exported
//! to, and — for an administrator — the server's Spotify application id. The
//! export itself lives on the playlist screen (`ExportSheet`, D8b); this page
//! only says which services are ready and with which account.
//!
//! Linking Spotify opens the system browser on Spotify's own consent page: the
//! callback lands on the server, and the app finds the result by re-reading
//! `/api/services` — the token never passes through this window.

use gpui_kit::{AnyElement, Context, IntoElement, ParentElement, Styled, Window, div, rems};
use gpui_omarchy::{ButtonVariant, button, input, textarea};
use nz_model::{Service, Services};

use super::{
    Field, Load, Screen, card, card_title, eid, failed, hint, loading, text_area, text_field,
    value_line,
};
use crate::actions::Logout;
use crate::theme;

pub fn body(screen: &mut Screen, window: &mut Window, cx: &mut Context<Screen>) -> AnyElement {
    let (name, admin) = {
        let state = screen.state.read(cx);
        (
            state
                .user_name()
                .map(str::to_owned)
                .unwrap_or_else(|| "Compte".to_owned()),
            state.auth.user().is_some_and(|me| me.user.admin),
        )
    };
    let account = card("settings-account", cx)
        .child(card_title("Compte", cx))
        .child(value_line("Identifiant", name, cx))
        .child(value_line(
            "Rôle",
            if admin {
                "Administrateur"
            } else {
                "Utilisateur"
            },
            cx,
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    button("settings-logout", "Déconnexion", ButtonVariant::Danger, cx).on_click(
                        cx.listener(|_, _, window, cx| {
                            window.dispatch_action(Box::new(Logout), cx);
                        }),
                    ),
                )
                .child(hint(
                    "La file en cours est oubliée sur cet appareil ; la musique déjà \
                     téléchargée reste.",
                    cx,
                )),
        );

    let mut column = div().flex().flex_col().gap(rems(1.)).child(account);
    column = column.child(services(screen, window, cx));
    if admin {
        column = column.child(spotify_app(screen, window, cx));
    }
    column.into_any_element()
}

fn services(screen: &mut Screen, window: &mut Window, cx: &mut Context<Screen>) -> AnyElement {
    let mut card = card("settings-services", cx)
        .child(card_title("Services de streaming", cx))
        .child(hint(
            "Pour envoyer une playlist vers vos propres comptes : chaque titre y est recherché \
             (code ISRC d'abord, puis titre et artiste) et ceux qui manquent vous sont listés. \
             Rien n'est envoyé sans votre geste depuis une playlist.",
            cx,
        ));
    match &screen.data.services {
        Load::Idle | Load::Loading => card = card.child(loading(cx)),
        Load::Failed(cause) => {
            card = card
                .child(failed("settings-services-failed", cause.clone(), cx))
                .child(
                    button(
                        "settings-services-retry",
                        "Réessayer",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.load_services(cx))),
                );
        }
        Load::Ready(services) => {
            let services = services.clone();
            let list: Vec<Service> = services
                .services
                .iter()
                .filter(|service| service.id != "apple")
                .cloned()
                .collect();
            if list.is_empty() {
                card = card.child(hint("Aucun service disponible sur ce serveur.", cx));
            }
            for service in &list {
                card = card.child(service_row(screen, service, cx));
            }
            if !services.formats.is_empty() {
                card = card.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.25))
                        .child(hint(
                            "Sans compte relié, une playlist s'exporte aussi en fichier — c'est \
                             ce que proposent les playlists :",
                            cx,
                        ))
                        .child(div().flex().flex_wrap().gap(rems(0.5)).children(
                            services.formats.iter().map(|format| {
                                div()
                                    .text_size(rems(0.6875))
                                    .font_family(theme::mono(cx))
                                    .text_color(theme::secondary(cx))
                                    .child(format.label.clone())
                            }),
                        )),
                );
            }
            card = card.child(hint(
                "Apple Music, Tidal, Amazon Music… n'ouvrent leur API qu'aux développeurs sous \
                 contrat : exportez la liste en CSV depuis la playlist et importez-la avec \
                 TuneMyMusic ou Soundiiz.",
                cx,
            ));
        }
    }

    // Linking YouTube Music: the browser's own request headers, pasted once.
    let youtube_linked = screen
        .data
        .services
        .ready()
        .map(|services| {
            services
                .services
                .iter()
                .any(|service| service.id == "youtube" && service.linked)
        })
        .unwrap_or(false);
    if !youtube_linked {
        let headers = text_area(Field::YoutubeHeaders, window, cx);
        let typed = headers.read(cx).value().to_string();
        card = card
            .child(div().text_size(rems(0.75)).child("Relier YouTube Music"))
            .child(hint(
                "1. Sur un ordinateur, ouvrez music.youtube.com connecté à votre compte. 2. \
                 Outils de développement (F12) → onglet Réseau, puis cliquez sur \
                 « Bibliothèque ». 3. Choisissez une requête browse (POST) → En-têtes de la \
                 requête → copiez-les tous. Ces en-têtes valent une session : ils sont \
                 chiffrés sur le serveur et ne servent qu'à créer vos playlists.",
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.5))
                    .child(textarea("settings-yt-headers", &headers, window, cx))
                    .child(
                        button(
                            "settings-yt-link",
                            if screen.saving == Some("youtube") {
                                "Vérification…"
                            } else {
                                "Relier YouTube Music"
                            },
                            ButtonVariant::Primary,
                            cx,
                        )
                        .disabled(typed.trim().is_empty() || screen.saving == Some("youtube"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            let field = text_area(Field::YoutubeHeaders, window, cx);
                            let raw = field.read(cx).value().to_string();
                            if raw.trim().is_empty() {
                                return;
                            }
                            field.update(cx, |state, cx| state.set_value("", window, cx));
                            this.link_youtube(raw, cx);
                        })),
                    ),
            );
    }
    card.into_any_element()
}

fn service_row(
    screen: &Screen,
    service: &Service,
    cx: &mut Context<Screen>,
) -> impl IntoElement + use<> {
    let id = service.id.clone();
    let name = service.name.clone();
    let status = service_status(service);
    let linked = service.linked;
    let ready = service.ready;
    let is_deezer = service.id == "deezer";
    let mut row = div()
        .flex()
        .items_center()
        .gap(rems(0.75))
        .py(rems(0.375))
        .border_b_1()
        .border_color(theme::border(cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_size(rems(0.75)).child(name))
                .child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .child(status),
                ),
        );
    if !is_deezer {
        if linked {
            let id = id.clone();
            row = row.child(
                button(
                    eid(format!("service-unlink-{id}")),
                    "Délier",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.unlink_service(id.clone(), cx);
                })),
            );
        } else if ready {
            row = row.child(
                button(
                    eid(format!("service-link-{id}")),
                    "Relier",
                    ButtonVariant::Primary,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if id == "spotify" {
                        this.authorize_spotify(cx);
                    }
                })),
            );
        }
    }
    let _ = screen;
    row
}

/// The sentence a service row shows: the SPA's `ServicesSettings#status`.
pub fn service_status(service: &Service) -> String {
    if service.id == "deezer" {
        return if service.ready {
            "Le compte Deezer du serveur".to_owned()
        } else {
            "Proxy Deezer désactivé".to_owned()
        };
    }
    if service.linked {
        return match &service.account {
            Some(account) if !account.is_empty() => format!("Relié : {account}"),
            _ => "Relié".to_owned(),
        };
    }
    if !service.ready {
        return if service.id == "spotify" {
            "Pas encore configuré sur ce serveur".to_owned()
        } else {
            "Indisponible sur ce serveur".to_owned()
        };
    }
    "Non relié".to_owned()
}

fn spotify_app(screen: &mut Screen, window: &mut Window, cx: &mut Context<Screen>) -> AnyElement {
    let client_id = screen
        .data
        .services
        .ready()
        .and_then(|services| {
            services
                .services
                .iter()
                .find(|service| service.id == "spotify")
                .cloned()
        })
        .and_then(|service| service.client_id);
    let Some(current) = client_id else {
        return div().into_any_element();
    };
    let field = text_field(Field::ClientId, window, cx);
    if field.read(cx).value().is_empty() && !current.is_empty() {
        // Seeded once with what the server holds; the field is editable after.
        let value = current.clone();
        field.update(cx, |state, cx| state.set_value(value, window, cx));
    }
    let typed = field.read(cx).value().to_string();
    card("settings-spotify-app", cx)
        .child(card_title("Application Spotify du serveur", cx))
        .child(hint(
            "Spotify n'accepte que des applications enregistrées. Sur \
             developer.spotify.com → Dashboard → Create app (« Web API »), ajoutez l'adresse \
             de retour affichée par le serveur, puis collez ici l'identifiant du client. Laissez \
             vide pour désactiver Spotify.",
            cx,
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    div()
                        .flex_1()
                        .child(input("settings-spotify-id", &field, window, cx)),
                )
                .child(
                    button(
                        "settings-spotify-save",
                        "Enregistrer",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(typed == current)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let field = text_field(Field::ClientId, window, cx);
                        let id = field.read(cx).value().trim().to_owned();
                        this.save_spotify_app(id, cx);
                    })),
                ),
        )
        .into_any_element()
}

/// A redirect address, without the query: what to paste into Spotify's
/// dashboard.
pub fn callback_hint(services: &Services, server: &str) -> String {
    let redirect = services
        .services
        .iter()
        .find(|service| service.id == "spotify")
        .and_then(|service| service.redirect_uri.clone())
        .unwrap_or_default();
    if redirect.is_empty() {
        format!("{server}/api/services/spotify/callback")
    } else {
        redirect
    }
}

impl Screen {
    /// Spotify's consent page opens in the system browser: the token never
    /// passes through this window, and the callback lands on the server. The
    /// app finds the result by re-reading `/api/services` (the Relier button
    /// does), because the browser is a different process.
    fn authorize_spotify(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.saving = Some("spotify");
        let (tx, rx) = crate::wiring::events::channel::<Result<String, String>>();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            let answer = api.spotify_authorize().await;
            let url = answer.ok().and_then(|value| {
                value
                    .get("url")
                    .and_then(|url| url.as_str())
                    .map(str::to_owned)
            });
            tx.send(url.ok_or_else(|| {
                "Spotify n'a pas donné d'adresse d'autorisation : vérifiez son application."
                    .to_owned()
            }));
        });
        cx.spawn(async move |this, cx| {
            let Some(result) = rx.next().await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.saving = None;
                match result {
                    Ok(url) => {
                        cx.open_url(&url);
                        this.toast(
                            crate::toast::ToastKind::Info,
                            "Autorisez Spotify dans le navigateur, puis revenez ici",
                            cx,
                        );
                    }
                    Err(cause) => this.fail(cause, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn link_youtube(&mut self, headers: String, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.saving = Some("youtube");
        let reply = self.reply();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            let value = serde_json::Value::String(headers);
            match api.connect_youtube(&value).await {
                Ok(_) => {
                    let answer = api
                        .services()
                        .await
                        .map_err(|error| error.cause_fr().to_owned());
                    reply.send(super::Reply::ServicesSaved(answer));
                }
                Err(error) => reply.send(super::Reply::Failed(format!(
                    "YouTube Music non relié : {}",
                    error.cause_fr()
                ))),
            }
        });
        cx.notify();
    }

    fn unlink_service(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.saving = Some("service");
        let reply = self.reply();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            match api.disconnect_service(&id).await {
                Ok(()) => {
                    let answer = api
                        .services()
                        .await
                        .map_err(|error| error.cause_fr().to_owned());
                    reply.send(super::Reply::ServicesSaved(answer));
                }
                Err(error) => reply.send(super::Reply::Failed(format!(
                    "Service non délié : {}",
                    error.cause_fr()
                ))),
            }
        });
        cx.notify();
    }

    fn save_spotify_app(&mut self, client_id: String, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.saving = Some("spotify-app");
        let reply = self.reply();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            match api.set_spotify_app(&client_id).await {
                Ok(_) => {
                    let answer = api
                        .services()
                        .await
                        .map_err(|error| error.cause_fr().to_owned());
                    reply.send(super::Reply::ServicesSaved(answer));
                }
                Err(error) => reply.send(super::Reply::Failed(format!(
                    "Application Spotify non enregistrée : {}",
                    error.cause_fr()
                ))),
            }
        });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_says_where_it_stands_not_just_that_it_exists() {
        let ready = Service {
            id: "spotify".into(),
            name: "Spotify".into(),
            ready: true,
            ..Service::default()
        };
        assert_eq!(service_status(&ready), "Non relié");
        let linked = Service {
            account: Some("kaya".into()),
            linked: true,
            ..ready.clone()
        };
        assert_eq!(service_status(&linked), "Relié : kaya");
        let unconfigured = Service {
            id: "spotify".into(),
            name: "Spotify".into(),
            ..Service::default()
        };
        assert_eq!(
            service_status(&unconfigured),
            "Pas encore configuré sur ce serveur"
        );
        let youtube = Service {
            id: "youtube".into(),
            name: "YouTube Music".into(),
            ..Service::default()
        };
        assert_eq!(service_status(&youtube), "Indisponible sur ce serveur");
        let deezer = Service {
            id: "deezer".into(),
            name: "Deezer".into(),
            ready: true,
            ..Service::default()
        };
        assert_eq!(service_status(&deezer), "Le compte Deezer du serveur");
    }

    #[test]
    fn the_callback_hint_falls_back_to_the_servers_own_route() {
        let services = Services::default();
        assert_eq!(
            callback_hint(&services, "https://neezer.example"),
            "https://neezer.example/api/services/spotify/callback"
        );
        let services = Services {
            services: vec![Service {
                id: "spotify".into(),
                redirect_uri: Some("http://127.0.0.1:5000/api/services/spotify/callback".into()),
                ..Service::default()
            }],
            ..Services::default()
        };
        assert_eq!(
            callback_hint(&services, "https://neezer.example"),
            "http://127.0.0.1:5000/api/services/spotify/callback"
        );
    }
}
