//! Réglages → Sources: where the sound may come from, whose accounts, and who
//! may use them.
//!
//! The SPA's `SourceOrder`, `MyAccounts`, `AccessSettings` and the admin
//! *Compte Deezer (ARL)* card of `routes/Settings.svelte`. Three panes: the
//! order of the sources (everybody reads it, an administrator changes it), the
//! person's own provider accounts (link, import, default, unlink) — plus the
//! server's Deezer credential for an administrator, which is the one field that
//! is WRITTEN here and never read back — and what may be used, with the grants
//! an administrator can hand out.
//!
//! The ARL is a full-account credential: it travels once to
//! `POST /me/accounts/deezer` (or `POST /settings` for the server's own), is
//! never stored on this device, never logged, and the server answers only a
//! hint and where it came from (`desktop.md`, *Auth and security*).

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement, StatefulInteractiveElement as _, Styled,
    Window, div, rems,
};
use gpui_omarchy::{ButtonVariant, button, input, switch};
use nz_model::{AccessAccount, AccessProvider, Account, MyAccounts, Source, Sources};

use super::{
    Field, Load, Screen, card, card_title, eid, failed, hint, loading, text_field, value_line,
};
use crate::theme;

/// Where a source stands, in the words the page uses.
pub fn place_label(source: &Source) -> String {
    match source.place.as_deref() {
        Some("main") => "Principale".to_owned(),
        Some("fallback") | Some("secondary") => {
            format!("Secours {}", source.rank.unwrap_or(0))
        }
        _ => {
            if source.configured {
                "Coupée".to_owned()
            } else {
                "Indisponible".to_owned()
            }
        }
    }
}

/// The sentence under a source's name: why it sits where it sits. Ported from
/// `SourceOrder.svelte#statusLine`, because a list of names with no reason is
/// the screen a person cannot act on.
pub fn status_line(source: &Source, before: &[&Source]) -> String {
    if !source.configured {
        return if source.setup.is_empty() {
            "Pas encore configurée sur ce serveur.".to_owned()
        } else {
            source.setup.clone()
        };
    }
    if source.resting > 0 {
        return format!(
            "A refusé ou n'a pas répondu : laissée tranquille encore {} min.",
            source.resting.div_ceil(60).max(1)
        );
    }
    match source.place.as_deref() {
        Some("main") => "Recherche, pages et musique. Toujours interrogée en première.".to_owned(),
        Some("fallback") | Some("secondary") => {
            let names = join_names(before.iter().map(|source| source.name.as_str()));
            if before.is_empty() {
                "Interrogée quand aucune autre source n'a un titre.".to_owned()
            } else if before.len() == 1 {
                format!("Interrogée quand {names} n'a pas un titre.")
            } else {
                format!("Interrogée quand {names} n'ont pas un titre.")
            }
        }
        _ => "Coupée : ce qui en est déjà archivé reste lisible.".to_owned(),
    }
    .replace("en première", "en premier")
}

fn join_names<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let names: Vec<&str> = names.collect();
    match names.len() {
        0 => String::new(),
        1 => names[0].to_owned(),
        _ => {
            let (last, rest) = names.split_last().expect("non-empty");
            format!("{} et {last}", rest.join(", "))
        }
    }
}

pub fn body(screen: &mut Screen, _window: &mut Window, cx: &mut Context<Screen>) -> AnyElement {
    let panes = screen.panes(&["Ordre", "Mes comptes", "Accès"], cx);
    let content = match screen.pane {
        1 => accounts(screen, _window, cx),
        2 => access(screen, cx),
        _ => order(screen, cx),
    };
    div()
        .flex()
        .flex_col()
        .gap(rems(1.))
        .child(panes)
        .child(content)
        .into_any_element()
}

// --- pane 1: the order -----------------------------------------------------

fn order(screen: &mut Screen, cx: &mut Context<Screen>) -> AnyElement {
    let mut card = card("settings-order", cx)
        .child(card_title("Ordre des sources", cx))
        .child(hint(
            "La source principale sert la recherche, les pages et la musique. Quand elle n'a \
             pas un titre, les sources de secours sont interrogées dans l'ordre ci-dessous. \
             Entre deux propositions aussi proches, la meilleure qualité passe devant ; à \
             qualité égale, cet ordre départage. Ce qui est déjà archivé reste lisible quoi \
             qu'il arrive.",
            cx,
        ));
    let admin = screen.is_admin(cx);
    let sources = match &screen.data.sources {
        Load::Idle | Load::Loading => None,
        Load::Failed(cause) => {
            card = card
                .child(failed("settings-order-failed", cause.clone(), cx))
                .child(
                    button(
                        "settings-order-retry",
                        "Réessayer",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.load_sources(cx))),
                );
            None
        }
        Load::Ready(sources) => Some(sources.clone()),
    };
    let Some(sources) = sources else {
        return card
            .when(
                matches!(screen.data.sources, Load::Idle | Load::Loading),
                |card| card.child(loading(cx)),
            )
            .into_any_element();
    };
    if sources.sources.is_empty() {
        return card
            .child(hint(
                "Aucune source de musique n'est installée sur ce serveur : seule la \
                 bibliothèque est lisible.",
                cx,
            ))
            .into_any_element();
    }

    let main = sources
        .sources
        .iter()
        .find(|source| source.place.as_deref() == Some("main"))
        .cloned();
    let mut fallbacks: Vec<Source> = sources
        .sources
        .iter()
        .filter(|source| {
            matches!(
                source.place.as_deref(),
                Some("fallback") | Some("secondary")
            )
        })
        .cloned()
        .collect();
    fallbacks.sort_by_key(|source| source.rank.unwrap_or(0));
    let off: Vec<Source> = sources
        .sources
        .iter()
        .filter(|source| {
            !matches!(
                source.place.as_deref(),
                Some("main") | Some("fallback") | Some("secondary")
            )
        })
        .cloned()
        .collect();

    // The main source, for an administrator who may change it.
    let leaders: Vec<Source> = sources
        .sources
        .iter()
        .filter(|source| source.can_lead && source.configured && source.enabled)
        .cloned()
        .collect();
    if admin && leaders.len() > 1 {
        let mut group = div().flex().flex_wrap().items_center().gap(rems(0.375));
        group = group.child(
            div()
                .text_size(rems(0.6875))
                .text_color(theme::secondary(cx))
                .child("Source principale"),
        );
        for leader in leaders {
            let selected = main.as_ref().is_some_and(|main| main.id == leader.id);
            group = group.child(
                gpui_omarchy::toggle(
                    format!("lead-{}", leader.id),
                    leader.name.clone(),
                    selected,
                    cx,
                )
                .when(selected, |chip| chip.text_color(theme::accent(cx)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_lead(leader.id.clone(), cx);
                })),
            );
        }
        card = card.child(group);
    }

    for (index, source) in fallbacks.iter().enumerate() {
        let before: Vec<&Source> = main.iter().chain(fallbacks.iter().take(index)).collect();
        card = card.child(source_row(
            source,
            &before,
            admin,
            index,
            fallbacks.len(),
            cx,
        ));
    }
    if let Some(main) = &main {
        card = card.child(source_row(main, &[], admin, 0, 0, cx));
    }
    for source in &off {
        card = card.child(source_row(source, &[], admin, 0, 0, cx));
    }

    // Best quality first, when the order is not already that.
    let mut by_quality = fallbacks.clone();
    by_quality.sort_by(|a, b| {
        b.quality
            .class
            .cmp(&a.quality.class)
            .then(b.quality.kbps.cmp(&a.quality.kbps))
    });
    let in_quality_order = by_quality
        .iter()
        .zip(fallbacks.iter())
        .all(|(sorted, current)| sorted.id == current.id);
    if admin && fallbacks.len() > 1 && !in_quality_order {
        card = card.child(
            button(
                "settings-order-quality",
                "Classer les secours par qualité",
                ButtonVariant::Outline,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_order(
                    by_quality.iter().map(|source| source.id.clone()).collect(),
                    cx,
                );
            })),
        );
    }
    if !admin {
        card = card.child(hint("Réglé par l'administrateur du serveur.", cx));
    }
    card.into_any_element()
}

fn source_row(
    source: &Source,
    before: &[&Source],
    admin: bool,
    index: usize,
    fallbacks: usize,
    cx: &mut Context<Screen>,
) -> impl IntoElement + use<> {
    let id = source.id.clone();
    let name = source.name.clone();
    let quality = if source.quality.label.is_empty() {
        String::new()
    } else {
        format!(
            "{}{}",
            source.quality.label,
            if source.quality.lossless {
                " · sans perte"
            } else {
                ""
            }
        )
    };
    let is_main = source.place.as_deref() == Some("main");
    let is_fallback = matches!(
        source.place.as_deref(),
        Some("fallback") | Some("secondary")
    );
    let mut controls = div().flex().items_center().gap(rems(0.25));
    if admin && is_fallback {
        let up = id.clone();
        let down = id.clone();
        controls = controls
            .child(
                button(
                    eid(format!("source-up-{id}")),
                    "Monter",
                    ButtonVariant::Secondary,
                    cx,
                )
                .disabled(index == 0)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.move_source(&up, -1, cx);
                })),
            )
            .child(
                button(
                    eid(format!("source-down-{id}")),
                    "Descendre",
                    ButtonVariant::Secondary,
                    cx,
                )
                .disabled(index + 1 >= fallbacks)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.move_source(&down, 1, cx);
                })),
            );
    }
    if admin && source.switchable && !is_main {
        let on = source.enabled;
        controls = controls.child(
            switch(eid(format!("source-on-{id}")), name.clone(), on, cx).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.toggle_source(id.clone(), !on, cx);
                },
            )),
        );
    }
    div()
        .flex()
        .items_center()
        .gap(rems(0.75))
        .py(rems(0.375))
        .border_b_1()
        .border_color(theme::border(cx))
        .child(
            div()
                .w(rems(6.))
                .text_size(rems(0.6875))
                .text_color(if is_main {
                    theme::accent(cx)
                } else {
                    theme::secondary(cx)
                })
                .child(place_label(source)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .child(div().text_size(rems(0.75)).child(name.clone()))
                        .when(!quality.is_empty(), |row| {
                            row.child(
                                div()
                                    .text_size(rems(0.625))
                                    .font_family(theme::mono(cx))
                                    .text_color(theme::secondary(cx))
                                    .child(quality),
                            )
                        }),
                )
                .child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(if source.configured && source.resting == 0 {
                            theme::secondary(cx)
                        } else {
                            theme::warning(cx)
                        })
                        .child(status_line(source, before)),
                ),
        )
        .child(controls)
}

// --- pane 2: the accounts --------------------------------------------------

fn accounts(screen: &mut Screen, window: &mut Window, cx: &mut Context<Screen>) -> AnyElement {
    let admin = screen.is_admin(cx);
    let mut column = div().flex().flex_col().gap(rems(1.));
    let mut accounts_card = card("settings-accounts", cx)
        .child(card_title("Mes comptes", cx))
        .child(hint(
            "Vos propres comptes chez les fournisseurs. Ce qu'ils contiennent (artistes \
             suivis, titres aimés, albums, playlists) peut être importé dans VOTRE \
             bibliothèque ; rien n'est jamais écrit sur le compte, et rien n'est téléchargé \
             sans que vous le demandiez.",
            cx,
        ));
    match &screen.data.accounts {
        Load::Idle | Load::Loading => accounts_card = accounts_card.child(loading(cx)),
        Load::Failed(cause) => {
            accounts_card = accounts_card
                .child(failed("settings-accounts-failed", cause.clone(), cx))
                .child(
                    button(
                        "settings-accounts-retry",
                        "Réessayer",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.load_accounts(cx))),
                );
        }
        Load::Ready(accounts) => {
            let accounts = accounts.clone();
            if accounts.accounts.is_empty() {
                accounts_card = accounts_card.child(hint("Aucun compte relié pour l'instant.", cx));
            }
            for account in &accounts.accounts {
                accounts_card = accounts_card.child(account_row(screen, account, &accounts, cx));
            }
        }
    }
    column = column.child(accounts_card);

    // Linking a Deezer account: the ARL is typed here, travels once, and is
    // never shown again (the server only keeps a hint).
    let arl = text_field(Field::Arl, window, cx);
    let typed = arl.read(cx).value().to_string();
    let mut link = card("settings-link", cx);
    link = link
        .child(card_title("Relier mon compte Deezer", cx))
        .child(hint(
            "L'ARL est la clé de session de votre compte Deezer. Il est vérifié auprès de \
             Deezer avant d'être enregistré, chiffré, sur le serveur — jamais sur cet \
             appareil, et il ne vous sera plus jamais réaffiché. Le compte du serveur ne peut \
             pas être relié une seconde fois : demandez à son titulaire de vous le prêter.",
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
                        .child(input("settings-arl", &arl, window, cx)),
                )
                .child(
                    button(
                        "settings-arl-save",
                        "Enregistrer",
                        ButtonVariant::Primary,
                        cx,
                    )
                    .disabled(typed.trim().is_empty() || screen.saving == Some("arl"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        let field = text_field(Field::Arl, window, cx);
                        let arl = field.read(cx).value().trim().to_owned();
                        if arl.is_empty() {
                            return;
                        }
                        field.update(cx, |state, cx| state.set_value("", window, cx));
                        this.link_deezer(arl, cx);
                    })),
                ),
        );
    column = column.child(link);

    if admin {
        column = column.child(deezer_credential(screen, window, cx));
    }
    column.into_any_element()
}

fn account_row(
    screen: &Screen,
    account: &Account,
    all: &MyAccounts,
    cx: &mut Context<Screen>,
) -> impl IntoElement + use<> {
    let id = account.id.clone();
    let name = if account.name.is_empty() {
        account.label.clone().unwrap_or_else(|| "Compte".to_owned())
    } else {
        account.name.clone()
    };
    let status = account_status(account);
    let is_default = all
        .default
        .as_deref()
        .map(|default| default == account.id)
        .unwrap_or(account.default);
    let kinds: Vec<String> = account.kinds.clone();
    let auto_on = kinds
        .iter()
        .any(|kind| account.auto.get(kind).and_then(|value| value.as_bool()) == Some(true));
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
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.375))
                        .child(div().text_size(rems(0.75)).child(name.clone()))
                        .when(is_default, |row| {
                            row.child(
                                div()
                                    .text_size(rems(0.625))
                                    .text_color(theme::accent(cx))
                                    .child("Par défaut"),
                            )
                        }),
                )
                .child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(if account.error.is_some() {
                            theme::warning(cx)
                        } else {
                            theme::secondary(cx)
                        })
                        .child(status),
                ),
        );
    if !kinds.is_empty() {
        let id = id.clone();
        row = row.child(
            switch(
                eid(format!("account-auto-{id}")),
                "Import quotidien",
                auto_on,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_account_auto(id.clone(), !auto_on, cx);
            })),
        );
    }
    if !is_default {
        let id = id.clone();
        row = row.child(
            button(
                eid(format!("account-default-{id}")),
                "Par défaut",
                ButtonVariant::Secondary,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_default_account(id.clone(), cx);
            })),
        );
    }
    {
        let id = id.clone();
        row = row.child(
            button(
                eid(format!("account-import-{id}")),
                "Importer",
                ButtonVariant::Primary,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.import_account(id.clone(), cx);
            })),
        );
    }
    if !account.server {
        let id = id.clone();
        let label = name.clone();
        row = row.child(
            button(
                eid(format!("account-unlink-{id}")),
                "Délier",
                ButtonVariant::Danger,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.unlink_account(id.clone(), label.clone(), cx);
            })),
        );
    }
    let _ = screen;
    row
}

/// What an account's state says in words: the SPA's `MyAccounts` statuses, plus
/// the failure KIND the server recorded (`broken` with `auth`, `unreachable`…).
pub fn account_status(account: &Account) -> String {
    if let Some(error) = &account.error {
        return match error.as_str() {
            "auth" => "Deezer refuse la clé de ce compte : reliez-le à nouveau.".to_owned(),
            "unreachable" => {
                "Le fournisseur ne répond pas : nouvelle tentative plus tard.".to_owned()
            }
            other => format!("Compte en échec ({other})."),
        };
    }
    match account.status.as_deref() {
        Some("ok") => {
            if account.lossless == Some(false) {
                "Relié · compte gratuit (le son reste en FLAC côté serveur)".to_owned()
            } else {
                "Relié".to_owned()
            }
        }
        Some("auth") => "Autorisation expirée : reliez ce compte.".to_owned(),
        Some(other) => format!("État : {other}"),
        None => "Relié".to_owned(),
    }
}

fn deezer_credential(
    screen: &mut Screen,
    window: &mut Window,
    cx: &mut Context<Screen>,
) -> AnyElement {
    let mut card = card("settings-deezer", cx)
        .child(card_title("Compte Deezer du serveur (ARL)", cx))
        .child(hint(
            "Tout le catalogue passe par cet identifiant de session. Il expire régulièrement : \
             collez-en un nouveau ici et il remplacera aussitôt celui défini dans la \
             configuration du serveur, sans redémarrage. Il est vérifié avant d'être \
             enregistré ; s'il est refusé, rien n'est modifié.",
            cx,
        ));
    match &screen.data.server {
        Load::Idle | Load::Loading => card = card.child(loading(cx)),
        Load::Failed(cause) => {
            card = card.child(failed("settings-deezer-failed", cause.clone(), cx))
        }
        Load::Ready(settings) => {
            let deezer = settings.deezer.clone();
            let state = match &screen.data.deezer {
                Load::Ready(status) => {
                    if status.ok {
                        "Connecté".to_owned()
                    } else {
                        status
                            .message
                            .clone()
                            .or_else(|| status.reason.clone())
                            .unwrap_or_else(|| "État inconnu".to_owned())
                    }
                }
                Load::Failed(cause) => cause.to_string(),
                _ => "Vérification…".to_owned(),
            };
            card = card
                .child(value_line("État", state, cx))
                .child(value_line(
                    "ARL actif",
                    if deezer.arl_set {
                        format!(
                            "{} · {}",
                            deezer.arl_hint.clone().unwrap_or_default(),
                            match deezer.arl_source.as_deref() {
                                Some("database") | Some("ui") => "saisi ici",
                                Some("env") => "variable d'environnement",
                                _ => "configuration du serveur",
                            }
                        )
                    } else {
                        "Aucun ARL configuré".to_owned()
                    },
                    cx,
                ))
                .child(
                    button(
                        "settings-deezer-test",
                        "Tester",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_deezer(cx))),
                );
            let from_ui = matches!(deezer.arl_source.as_deref(), Some("database") | Some("ui"));
            let arl = text_field(Field::Arl, window, cx);
            let typed = arl.read(cx).value().to_string();
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .flex_1()
                            .child(input("settings-deezer-arl", &arl, window, cx)),
                    )
                    .child(
                        button(
                            "settings-deezer-save",
                            "Enregistrer",
                            ButtonVariant::Primary,
                            cx,
                        )
                        .disabled(typed.trim().is_empty() || screen.saving == Some("deezer"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            let field = text_field(Field::Arl, window, cx);
                            let arl = field.read(cx).value().trim().to_owned();
                            if arl.is_empty() {
                                return;
                            }
                            field.update(cx, |state, cx| state.set_value("", window, cx));
                            this.save_server_arl(arl, cx);
                        })),
                    ),
            );
            if from_ui {
                card = card.child(
                    button(
                        "settings-deezer-clear",
                        "Revenir à l'ARL du serveur",
                        ButtonVariant::Danger,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.clear_server_arl(cx))),
                );
            }
        }
    }
    card.into_any_element()
}

// --- pane 3: the access ----------------------------------------------------

fn access(screen: &mut Screen, cx: &mut Context<Screen>) -> AnyElement {
    let mut column = div().flex().flex_col().gap(rems(1.));
    let admin = screen.is_admin(cx);
    let mut access_card = card("settings-access", cx)
        .child(card_title("Sources et accès", cx))
        .child(hint(
            "Tout ce qui est déjà sur le serveur est à tout le monde. Ici se lit le reste : \
             qui peut chercher, écouter et ouvrir des pages chez chaque service, et avec quel \
             compte. Le serveur reste le seul juge : l'interface n'offre rien qu'il refuserait.",
            cx,
        ));
    match &screen.data.access {
        Load::Idle | Load::Loading => access_card = access_card.child(loading(cx)),
        Load::Failed(cause) => {
            access_card = access_card
                .child(failed("settings-access-failed", cause.clone(), cx))
                .child(
                    button(
                        "settings-access-retry",
                        "Réessayer",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.load_access(cx))),
                );
        }
        Load::Ready(access) => {
            let providers = access.providers.clone();
            let groups = access.groups.clone();
            let users = access.users.clone().unwrap_or_default();
            for provider in &providers {
                access_card =
                    access_card.child(provider_block(screen, provider, &groups, &users, admin, cx));
            }
            column = column.child(access_card);
            if admin && !groups.is_empty() {
                let mut group_card = card("settings-groups", cx)
                    .child(card_title("Groupes", cx))
                    .child(hint(
                        "Pour prêter un compte à plusieurs personnes d'un coup (« Proches », \
                         « Famille »).",
                        cx,
                    ));
                for group in &groups {
                    let id = group.id.clone();
                    let members = group
                        .members
                        .clone()
                        .unwrap_or_default()
                        .iter()
                        .map(|member| member.name.clone())
                        .collect::<Vec<String>>()
                        .join(", ");
                    group_card = group_card.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.5))
                            .py(rems(0.25))
                            .border_b_1()
                            .border_color(theme::border(cx))
                            .child(
                                div()
                                    .w(rems(10.))
                                    .text_size(rems(0.75))
                                    .child(group.name.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(rems(0.6875))
                                    .text_color(theme::secondary(cx))
                                    .child(if members.is_empty() {
                                        "Personne pour l'instant".to_owned()
                                    } else {
                                        members
                                    }),
                            )
                            .child(
                                button(
                                    eid(format!("group-delete-{id}")),
                                    "Supprimer",
                                    ButtonVariant::Danger,
                                    cx,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.delete_group(id.clone(), cx);
                                    },
                                )),
                            ),
                    );
                }
                column = column.child(group_card);
            }
            return column.into_any_element();
        }
    }
    column.child(access_card).into_any_element()
}

fn provider_block(
    screen: &Screen,
    provider: &AccessProvider,
    groups: &[nz_model::AccessGroup],
    users: &[nz_model::AccessUser],
    admin: bool,
    cx: &mut Context<Screen>,
) -> impl IntoElement + use<> {
    let allowed: Vec<&str> = provider
        .features
        .iter()
        .filter(|feature| {
            provider
                .allowed
                .iter()
                .any(|id| id == &feature.id || id == "all")
        })
        .map(|feature| feature.label.as_str())
        .collect();
    let _ = screen;
    let mut block = div()
        .flex()
        .flex_col()
        .gap(rems(0.375))
        .py(rems(0.5))
        .border_b_1()
        .border_color(theme::border(cx))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(div().text_size(rems(0.8125)).child(provider.name.clone()))
                .when(provider.role.as_deref() == Some("info"), |row| {
                    row.child(
                        div()
                            .text_size(rems(0.625))
                            .text_color(theme::secondary(cx))
                            .child("sources de faits"),
                    )
                }),
        )
        .child(
            div()
                .text_size(rems(0.6875))
                .text_color(theme::secondary(cx))
                .child(if allowed.is_empty() {
                    "Pas accessible pour vous.".to_owned()
                } else {
                    format!("Vous pouvez : {}", allowed.join(", ").to_lowercase())
                }),
        );
    for account in &provider.accounts {
        block = block.child(account_grant(
            screen, provider, account, groups, users, admin, cx,
        ));
    }
    block
}

#[allow(clippy::too_many_arguments)]
fn account_grant(
    screen: &Screen,
    provider: &AccessProvider,
    account: &AccessAccount,
    groups: &[nz_model::AccessGroup],
    users: &[nz_model::AccessUser],
    admin: bool,
    cx: &mut Context<Screen>,
) -> impl IntoElement + use<> {
    let holder = holder_line(account);
    let mut row = div().flex().flex_col().gap(rems(0.25)).child(
        div()
            .text_size(rems(0.6875))
            .text_color(theme::text(cx))
            .child(holder),
    );
    if !admin || !account.editable {
        return row.child(
            div()
                .text_size(rems(0.625))
                .text_color(theme::secondary(cx))
                .child(grant_line(account)),
        );
    }
    // The whole-account grant, as subject chips: everyone, each group, each
    // person — the server's own spelling (`*`, `g:<id>`, `u:<id>`).
    let granted: Vec<String> = account.grants.get("*").cloned().unwrap_or_default();
    let mut chips = div().flex().flex_wrap().items_center().gap(rems(0.25));
    for (subject, label) in grant_subjects(account, groups, users) {
        let on = granted.iter().any(|value| value == &subject);
        chips = chips.child(subject_chip(
            provider.id.clone(),
            account.id.clone(),
            subject,
            label,
            on,
            cx,
        ));
    }
    let _ = screen;
    row = row.child(chips);
    row
}

/// The subjects a grant can name for this account: everybody, its holder, the
/// people who may lend it an account (groups) and every other person.
pub fn grant_subjects(
    account: &AccessAccount,
    groups: &[nz_model::AccessGroup],
    users: &[nz_model::AccessUser],
) -> Vec<(String, String)> {
    let mut subjects: Vec<(String, String)> = vec![("*".to_owned(), "Tout le monde".to_owned())];
    if let Some(owner) = &account.owner {
        subjects.push((format!("u:{}", owner.id.as_str()), owner.name.clone()));
    }
    for group in groups {
        subjects.push((format!("g:{}", group.id), group.name.clone()));
    }
    for user in users {
        if account
            .owner
            .as_ref()
            .is_some_and(|owner| owner.id == user.id)
        {
            continue;
        }
        subjects.push((format!("u:{}", user.id.as_str()), user.name.clone()));
    }
    subjects
}

fn subject_chip(
    provider: String,
    account: String,
    subject: String,
    label: String,
    on: bool,
    cx: &mut Context<Screen>,
) -> impl IntoElement + use<> {
    gpui_omarchy::toggle(
        eid(format!("grant-{provider}-{account}-{subject}")),
        label,
        on,
        cx,
    )
    .when(on, |chip| chip.text_color(theme::accent(cx)))
    .on_click(cx.listener(move |this, _, _, cx| {
        this.toggle_grant(provider.clone(), account.clone(), subject.clone(), !on, cx);
    }))
}

/// `AccessSettings.svelte#holderLine`: whose account this is, and how it is held.
pub fn holder_line(account: &AccessAccount) -> String {
    match account.mode.as_str() {
        "server" => "Compte du serveur".to_owned(),
        _ => match &account.label {
            Some(label) if !label.is_empty() => format!("Compte relié : {label}"),
            _ => match &account.owner {
                Some(owner) if account.mine => format!("Votre compte ({})", owner.name),
                Some(owner) => format!("Compte de {}", owner.name),
                None => "Compte personnel".to_owned(),
            },
        },
    }
}

/// Who may use the account, in one sentence: `*` is everybody, `u:` a person and
/// `g:` a group — the server's own spelling.
pub fn grant_line(account: &AccessAccount) -> String {
    let subjects = account.grants.get("*").cloned().unwrap_or_default();
    if subjects.is_empty() {
        return "Personne d'autre que le titulaire.".to_owned();
    }
    if subjects.iter().any(|subject| subject == "*") {
        return "Tout le monde.".to_owned();
    }
    let names: Vec<String> = subjects
        .iter()
        .map(|subject| match subject.split_once(':') {
            Some(("g", _)) => "un groupe".to_owned(),
            Some(("u", _)) => "une personne".to_owned(),
            _ => subject.clone(),
        })
        .collect();
    format!("Prêté à : {}.", names.join(", "))
}

impl Screen {
    /// The sources in the order the wire wants: main first, then the fallbacks
    /// by rank, then whatever is off (the server ignores an id it does not
    /// know, and keeps an unlisted one in its place).
    fn ordered_ids(sources: &Sources) -> Vec<String> {
        let mut ids = Vec::new();
        if let Some(main) = sources
            .sources
            .iter()
            .find(|source| source.place.as_deref() == Some("main"))
        {
            ids.push(main.id.clone());
        }
        let mut fallbacks: Vec<&Source> = sources
            .sources
            .iter()
            .filter(|source| {
                matches!(
                    source.place.as_deref(),
                    Some("fallback") | Some("secondary")
                )
            })
            .collect();
        fallbacks.sort_by_key(|source| source.rank.unwrap_or(0));
        ids.extend(fallbacks.into_iter().map(|source| source.id.clone()));
        ids.extend(
            sources
                .sources
                .iter()
                .filter(|source| {
                    !matches!(
                        source.place.as_deref(),
                        Some("main") | Some("fallback") | Some("secondary")
                    )
                })
                .map(|source| source.id.clone()),
        );
        ids
    }

    /// The server's own resolution, applied locally: the first source that is ON
    /// and can lead is the main one, the other ON ones are the fallbacks in
    /// order, the rest are off (`sourcesapi._place`).
    pub fn arrange(sources: &Sources, order: &[String]) -> Sources {
        let mut next = sources.clone();
        let mut lead: Option<String> = None;
        for id in order {
            let Some(source) = next.sources.iter().find(|source| &source.id == id).cloned() else {
                continue;
            };
            if lead.is_none() && source.enabled && source.can_lead {
                lead = Some(id.clone());
            }
        }
        let fallbacks: Vec<String> = order
            .iter()
            .filter(|id| Some(*id) != lead.as_ref())
            .filter(|id| {
                next.sources
                    .iter()
                    .find(|source| &source.id == *id)
                    .is_some_and(|source| source.enabled)
            })
            .cloned()
            .collect();
        next.main = lead.clone();
        for source in &mut next.sources {
            if Some(&source.id) == lead.as_ref() {
                source.place = Some("main".to_owned());
                source.rank = Some(0);
            } else if let Some(index) = fallbacks.iter().position(|id| id == &source.id) {
                source.place = Some("fallback".to_owned());
                source.rank = Some(index as i64 + 1);
            } else {
                source.place = Some("off".to_owned());
                source.rank = None;
            }
        }
        next
    }

    fn post_sources(&mut self, body: serde_json::Value, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.saving = Some("sources");
        let reply = self.reply();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            match api.set_sources(&body).await {
                Ok(_) => {
                    let answer = api.sources().await.map_err(cause);
                    reply.send(super::Reply::SourcesSaved(answer));
                }
                Err(error) => reply.send(super::Reply::SourcesSaved(Err(format!(
                    "Ordre non enregistré : {}",
                    error.cause_fr()
                )))),
            }
        });
        cx.notify();
    }

    /// A new order, drawn at once (rule 4), the server's answer replacing it.
    pub fn set_order(&mut self, order: Vec<String>, cx: &mut Context<Self>) {
        if let Some(sources) = self.data.sources.ready().cloned() {
            self.data.sources = Load::Ready(Self::arrange(&sources, &order));
        }
        self.post_sources(serde_json::json!({ "order": order }), cx);
    }

    /// Make `id` the main source: it goes first, the current main becomes the
    /// first fallback — it still has its catalogue.
    fn set_lead(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(sources) = self.data.sources.ready().cloned() else {
            return;
        };
        let mut order = Self::ordered_ids(&sources);
        order.retain(|other| other != &id);
        order.insert(0, id);
        self.set_order(order, cx);
    }

    fn move_source(&mut self, id: &str, delta: isize, cx: &mut Context<Self>) {
        let Some(sources) = self.data.sources.ready().cloned() else {
            return;
        };
        let mut order = Self::ordered_ids(&sources);
        let Some(index) = order.iter().position(|other| other == id) else {
            return;
        };
        let target = index as isize + delta;
        if target < 0 || target as usize >= order.len() {
            return;
        }
        order.swap(index, target as usize);
        self.set_order(order, cx);
    }

    fn toggle_source(&mut self, id: String, on: bool, cx: &mut Context<Self>) {
        if let Some(sources) = self.data.sources.ready().cloned() {
            let order = Self::ordered_ids(&sources);
            let mut preview = sources.clone();
            for source in &mut preview.sources {
                if source.id == id {
                    source.enabled = on;
                }
            }
            self.data.sources = Load::Ready(Self::arrange(&preview, &order));
        }
        self.post_sources(serde_json::json!({ "enabled": { id: on } }), cx);
    }

    /// The per-kind automatic import: the server takes `{"auto": {kind: bool}}`
    /// and only accepts the kinds the account announced (`libraryimport.KINDS`),
    /// which is what `Api::set_account_auto` now builds — the client used to post
    /// a bare boolean and had to be bypassed through `Api::send`.
    fn set_account_auto(&mut self, id: String, on: bool, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let kinds: Vec<String> = self
            .data
            .accounts
            .ready()
            .and_then(|accounts| {
                accounts
                    .accounts
                    .iter()
                    .find(|account| account.id == id)
                    .map(|account| account.kinds.clone())
            })
            .unwrap_or_default();
        if kinds.is_empty() {
            self.fail("Ce compte n'annonce aucun import automatique.", cx);
            return;
        }
        let reply = self.reply();
        let runtime = self.runtime(cx);
        self.saving = Some("auto");
        runtime.spawn(async move {
            match api.set_account_auto(&id, &kinds, on).await {
                Ok(_) => {
                    let answer = api.my_accounts().await.map_err(cause);
                    reply.send(super::Reply::AccountsSaved(answer));
                }
                Err(error) => reply.send(super::Reply::Failed(format!(
                    "Import automatique non enregistré : {}",
                    error.cause_fr()
                ))),
            }
        });
        cx.notify();
    }

    fn set_default_account(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.saving = Some("default");
        let reply = self.reply();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            match api.set_default_account(&id).await {
                Ok(_) => {
                    let answer = api.my_accounts().await.map_err(cause);
                    reply.send(super::Reply::AccountsSaved(answer));
                }
                Err(error) => reply.send(super::Reply::Failed(format!(
                    "Compte par défaut non changé : {}",
                    error.cause_fr()
                ))),
            }
        });
        cx.notify();
    }

    /// An import is a background job on the server (rule 2): the button starts
    /// it, the answer says it is running, and the account's own row shows what
    /// the server reports next time it is read.
    fn import_account(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.saving = Some("import");
        let reply = self.reply();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            match api.import_account(&id, None).await {
                Ok(_) => {
                    let answer = api.my_accounts().await.map_err(cause);
                    reply.send(super::Reply::AccountsSaved(answer));
                }
                Err(error) => reply.send(super::Reply::Failed(format!(
                    "Import impossible : {}",
                    error.cause_fr()
                ))),
            }
        });
        cx.notify();
    }

    fn unlink_account(&mut self, id: String, label: String, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.saving = Some("unlink");
        let reply = self.reply();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            match api.unlink_account(&id).await {
                Ok(()) => {
                    let answer = api.my_accounts().await.map_err(cause);
                    reply.send(super::Reply::AccountsSaved(answer));
                    let _ = label;
                }
                Err(error) => reply.send(super::Reply::Failed(format!(
                    "Compte non délié : {}",
                    error.cause_fr()
                ))),
            }
        });
        cx.notify();
    }

    /// Link this person's own Deezer account. The ARL is sent once and the
    /// server answers only the account it built.
    fn link_deezer(&mut self, arl: String, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.saving = Some("arl");
        let reply = self.reply();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            match api.link_deezer(&arl, None).await {
                Ok(_) => {
                    let answer = api.my_accounts().await.map_err(cause);
                    reply.send(super::Reply::AccountsSaved(answer));
                }
                Err(error) => reply.send(super::Reply::Failed(format!(
                    "Compte Deezer non relié : {}",
                    error.cause_fr()
                ))),
            }
        });
        cx.notify();
    }

    /// The server's own Deezer credential: written once, verified by the server
    /// before it is stored, and never read back.
    fn save_server_arl(&mut self, arl: String, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.saving = Some("deezer");
        let reply = self.reply();
        let status_reply = self.reply();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            let body = serde_json::json!({ "deezer_arl": arl });
            match api.set_settings(&body).await {
                Ok(_) => {
                    let answer = api.get_settings().await.map_err(cause);
                    reply.send(super::Reply::ServerSettings(answer));
                    match api.deezer_status(true).await {
                        Ok(status) => status_reply.send(super::Reply::Deezer(Ok(status))),
                        Err(error) => status_reply.send(super::Reply::Deezer(Err(cause(error)))),
                    }
                }
                Err(error) => reply.send(super::Reply::Failed(format!(
                    "ARL refusé : {}",
                    error.cause_fr()
                ))),
            }
        });
        cx.notify();
    }

    fn clear_server_arl(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.saving = Some("deezer");
        let reply = self.reply();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            let body = serde_json::json!({ "deezer_arl": "" });
            match api.set_settings(&body).await {
                Ok(_) => {
                    let answer = api.get_settings().await.map_err(cause);
                    reply.send(super::Reply::ServerSettings(answer));
                }
                Err(error) => reply.send(super::Reply::Failed(format!(
                    "ARL du serveur non rétabli : {}",
                    error.cause_fr()
                ))),
            }
        });
        cx.notify();
    }

    fn refresh_deezer(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let reply = self.reply();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            let answer = api.deezer_status(true).await.map_err(cause);
            reply.send(super::Reply::Deezer(answer));
        });
        cx.notify();
    }

    /// One subject of one whole-account grant, added or removed.
    fn toggle_grant(
        &mut self,
        provider: String,
        account: String,
        subject: String,
        on: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let mut subjects: Vec<String> = self
            .data
            .access
            .ready()
            .and_then(|access| {
                access
                    .providers
                    .iter()
                    .find(|other| other.id == provider)
                    .and_then(|provider| {
                        provider
                            .accounts
                            .iter()
                            .find(|other| other.id == account)
                            .map(|row| row.grants.get("*").cloned().unwrap_or_default())
                    })
            })
            .unwrap_or_default();
        if on {
            // "Tout le monde" replaces every other subject: a list holding `*`
            // and a person would grant the same thing twice.
            if subject == "*" {
                subjects.clear();
            }
            if !subjects.contains(&subject) {
                subjects.push(subject.clone());
            }
        } else {
            subjects.retain(|value| value != &subject);
        }
        self.saving = Some("grant");
        let reply = self.reply();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            match api.set_grants(&provider, &account, "*", &subjects).await {
                Ok(_) => {
                    let answer = api.access().await.map_err(cause);
                    reply.send(super::Reply::AccessSaved(answer));
                }
                Err(error) => reply.send(super::Reply::Failed(format!(
                    "Prêt non enregistré : {}",
                    error.cause_fr()
                ))),
            }
        });
        cx.notify();
    }

    fn delete_group(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        self.saving = Some("group");
        let reply = self.reply();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            match api.delete_group(&id).await {
                Ok(()) => {
                    let answer = api.access().await.map_err(cause);
                    reply.send(super::Reply::AccessSaved(answer));
                }
                Err(error) => reply.send(super::Reply::Failed(format!(
                    "Groupe non supprimé : {}",
                    error.cause_fr()
                ))),
            }
        });
        cx.notify();
    }
}

/// `encodeURIComponent`'s equivalent for one path segment (`nz-api`'s `encode`
/// is the same table, kept private there; an account id is a UUID, but a third
/// party's may not be).
fn encode(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn cause(error: nz_api::ApiError) -> String {
    error.cause_fr().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: &str, place: &str, rank: i64) -> Source {
        Source {
            id: id.to_owned(),
            name: id.to_owned(),
            enabled: true,
            configured: true,
            place: Some(place.to_owned()),
            rank: Some(rank),
            ..Source::default()
        }
    }

    #[test]
    fn the_place_names_read_like_the_page() {
        assert_eq!(place_label(&source("deezer", "main", 0)), "Principale");
        assert_eq!(place_label(&source("youtube", "fallback", 1)), "Secours 1");
        let mut off = source("youtube", "off", 0);
        off.configured = true;
        assert_eq!(place_label(&off), "Coupée");
        let mut absent = source("youtube", "off", 0);
        absent.configured = false;
        assert_eq!(place_label(&absent), "Indisponible");
    }

    #[test]
    fn a_fallback_says_which_sources_are_asked_before_it() {
        let deezer = source("Deezer", "main", 0);
        let youtube = source("YouTube", "fallback", 1);
        let local = source("Local", "fallback", 2);
        let line = status_line(&youtube, &[&deezer]);
        assert_eq!(line, "Interrogée quand Deezer n'a pas un titre.");
        let line = status_line(&local, &[&deezer, &youtube]);
        assert_eq!(
            line,
            "Interrogée quand Deezer et YouTube n'ont pas un titre."
        );
        assert_eq!(
            status_line(&deezer, &[]),
            "Recherche, pages et musique. Toujours interrogée en premier."
        );
    }

    #[test]
    fn a_resting_source_says_how_long_it_was_left_alone() {
        let mut youtube = source("YouTube", "fallback", 1);
        youtube.resting = 121;
        assert_eq!(
            status_line(&youtube, &[]),
            "A refusé ou n'a pas répondu : laissée tranquille encore 3 min."
        );
        // A source that is not configured says how to set it up, not that it
        // rests.
        let mut absent = source("YouTube", "off", 0);
        absent.configured = false;
        absent.setup = "Collez vos en-têtes YouTube Music.".into();
        assert_eq!(
            status_line(&absent, &[]),
            "Collez vos en-têtes YouTube Music."
        );
    }

    #[test]
    fn a_grant_reads_as_who_may_use_the_account() {
        let mut account = AccessAccount {
            id: "a1".into(),
            mode: "personal".into(),
            label: Some("kaya@example".into()),
            ..AccessAccount::default()
        };
        assert_eq!(holder_line(&account), "Compte relié : kaya@example");
        assert_eq!(grant_line(&account), "Personne d'autre que le titulaire.");
        account.grants.insert("*".into(), vec!["*".into()]);
        assert_eq!(grant_line(&account), "Tout le monde.");
        account
            .grants
            .insert("*".into(), vec!["u:42".into(), "g:7".into()]);
        assert_eq!(grant_line(&account), "Prêté à : une personne, un groupe.");
    }

    #[test]
    fn an_account_says_why_it_is_broken_not_just_that_it_is() {
        let mut account = Account {
            status: Some("ok".into()),
            ..Account::default()
        };
        assert_eq!(account_status(&account), "Relié");
        account.lossless = Some(false);
        assert!(account_status(&account).contains("compte gratuit"));
        account.error = Some("auth".into());
        assert!(account_status(&account).contains("reliez-le à nouveau"));
        account.error = Some("unreachable".into());
        assert!(account_status(&account).contains("ne répond pas"));
    }
}
