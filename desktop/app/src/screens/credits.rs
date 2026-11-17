//! Crédits: the people behind one track, and everything one person wrote,
//! composed or produced in the library.
//!
//! `routes/Credits.svelte`, `components/TrackCredits.svelte` and
//! `components/PersonCredits.svelte` are the reference. Two shapes share one
//! route ([`Route::Credits`]):
//!
//! - [`CreditKind::Track`] — `GET /track/<id>/credits`: one row per role, in the
//!   order a sleeve lists them, then the release line (album, date, label,
//!   ISRC). A name OPENS what the library has of that person: their artist page
//!   when there is one, their credits page when they are credited elsewhere, and
//!   plain text when a link would lead nowhere — a link that does nothing is a
//!   bug, and the SPA says so too.
//! - [`CreditKind::Person`] — `GET /credits/person?name=`: a chip per role
//!   narrows the list, and the list is the same windowed [`super::super::widgets::track_list`],
//!   so a songwriter credited on hundreds of titles costs the rows on screen.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{
    Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement, Render,
    StatefulInteractiveElement as _, Styled, Subscription, Window, div, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};
use nz_api::ApiError;
use nz_model::{CreditPerson, PersonCredits, TrackCredits};

use crate::nav::{CreditKind, Route};
use crate::state::AppState;
use crate::theme;
use crate::widgets::card::{self, Phase};
use crate::widgets::context_menu::{Item, Menu};
use crate::widgets::page_scroll::{self, PageScroll};
use crate::widgets::track_list::{self, Intent, Subtitle, TrackList as List};
use crate::wiring::events::{self, Sender};

enum Msg {
    Track(Box<Result<TrackCredits, ApiError>>),
    Person(Box<Result<PersonCredits, ApiError>>),
    Cover(String, Option<PathBuf>),
}

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    kind: CreditKind,
    id: String,
    track: Phase<TrackCredits>,
    person: Option<Phase<PersonCredits>>,
    /// The role chip that narrows a person's list (`""` = every role).
    role: String,
    list: List,
    covers: Rc<RefCell<crate::widgets::cover::Covers>>,
    menu: Menu,
    /// The page of a TRACK's credits: a role list is as tall as the sleeve's
    /// small print. A person's credits are a list (see `person_body`), and the
    /// list is the scroller there, so this page owns no keys on that shape.
    page: PageScroll,
    /// The keystroke interceptor that owns the keyboard while a menu is open.
    menu_keys: Option<Subscription>,
    events: Sender<Msg>,
    _subscriptions: Vec<Subscription>,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let (kind, id) = match &route {
            Route::Credits { kind, id } => (*kind, id.clone()),
            _ => (CreditKind::Person, String::new()),
        };
        let (tx, rx) = events::channel::<Msg>();
        let mut screen = Self {
            route,
            state: state.clone(),
            kind,
            id: id.clone(),
            track: Phase::Loading,
            person: None,
            role: String::new(),
            list: List::new().subtitle(Subtitle::Roles),
            covers: Rc::new(RefCell::new(crate::widgets::cover::Covers::rows())),
            menu: Menu::new(),
            page: PageScroll::new(),
            menu_keys: None,
            events: tx,
            _subscriptions: vec![cx.observe(&state, |_, _, cx| cx.notify())],
        };
        match kind {
            CreditKind::Track => {
                let path = format!("/track/{}/credits", card::encode(&id));
                let cached = card::cached::<TrackCredits>(state.read(cx), &path);
                if let Some(cached) = cached {
                    screen.track = Phase::Ready(cached);
                }
            }
            CreditKind::Person => {
                let path = crate::widgets::card::person_credits_path(&id);
                let cached = card::cached::<PersonCredits>(state.read(cx), &path);
                if let Some(cached) = cached {
                    screen.apply_person(cached, cx);
                }
            }
        }
        screen.drain(rx, cx);
        screen.load(cx);
        track_list::own(cx, |this: &mut Self, intent, window, cx| {
            this.on_intent(intent, window, cx)
        });
        // A track's credits have no list and no text field: the page takes
        // `↑`/`↓`, `Page préc./suiv.` and `Début`/`Fin` and CONSUMES them (D8a's
        // rule). A person's credits are a list, whose own keys stay the list's;
        // that page scrolls with the wheel.
        if kind == CreditKind::Track {
            let keys = screen.page.own(cx);
            screen._subscriptions.push(keys);
        }
        screen
    }

    fn drain(&mut self, rx: events::Receiver<Msg>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                if this.update(cx, |this, cx| this.on_msg(msg, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.events.clone();
        let id = self.id.clone();
        let kind = self.kind;
        runtime.spawn(async move {
            match kind {
                CreditKind::Track => tx.send(Msg::Track(Box::new(api.track_credits(&id).await))),
                CreditKind::Person => tx.send(Msg::Person(Box::new(api.person_credits(&id).await))),
            }
        });
    }

    fn apply_person(&mut self, credits: PersonCredits, cx: &mut Context<Self>) {
        let role = self.role.clone();
        let rows: Vec<_> = credits
            .tracks
            .iter()
            .filter(|track| role.is_empty() || track.credit_roles.contains(&role))
            .cloned()
            .collect();
        self.list.set_context(Some(nz_player::Context {
            kind: "credits".to_owned(),
            id: Some(credits.name.clone()),
            title: Some(credits.name.clone()),
        }));
        self.list.set_rows(rows);
        let _ = cx;
        self.person = Some(Phase::Ready(credits));
    }

    fn on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Track(result) => {
                self.track = match *result {
                    Ok(credits) => Phase::Ready(credits),
                    Err(error) => Phase::Failed(error),
                }
            }
            Msg::Person(result) => match *result {
                Ok(credits) => self.apply_person(credits, cx),
                Err(error) => self.person = Some(Phase::Failed(error)),
            },
            Msg::Cover(url, file) => self.covers.borrow_mut().resolved(&url, file),
        }
        cx.notify();
    }

    fn covers_for_visible(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let mut asked: Vec<String> = Vec::new();
        for index in range {
            if let Some(url) = self
                .list
                .row(index)
                .and_then(|track| track.album.cover.clone())
            {
                asked.push(url);
            }
        }
        let snapshot = self.state.read(cx);
        let mut covers = self.covers.borrow_mut();
        for url in asked {
            let for_request = url.clone();
            let for_message = url.clone();
            let tx = self.events.clone();
            covers.want(snapshot, &for_request, move |file| {
                tx.send(Msg::Cover(for_message, file))
            });
        }
    }

    fn on_intent(&mut self, intent: Intent, window: &mut Window, cx: &mut Context<Self>) {
        let page = List::page_rows(window.viewport_size().height);
        match intent {
            Intent::Menu(_) | Intent::Like(_) | Intent::Dislike(_) => {}
            other => {
                if let Some(request) = self.list.apply(other, page) {
                    let command = request.command();
                    self.state
                        .update(cx, |state, cx| state.command(command, cx));
                }
                cx.notify();
            }
        }
    }

    /// A person's name: their artist page when the library holds one, their
    /// credits page when they are credited elsewhere, nothing otherwise.
    fn open_person(&mut self, person: &CreditPerson, cx: &mut Context<Self>) {
        let route = if let Some(id) = person
            .deezer_id
            .as_ref()
            .map(|id| id.as_str().to_owned())
            .filter(|id| !id.is_empty())
            .or_else(|| person.id.clone())
        {
            Route::Artist { id }
        } else if person.credited.is_some_and(|count| count > 0) {
            Route::Credits {
                kind: CreditKind::Person,
                id: person.name.clone(),
            }
        } else {
            return;
        };
        self.state.update(cx, |state, _| state.nav.push(route));
    }

    fn open_album(&mut self, cx: &mut Context<Self>) {
        let Some(album) = self
            .list
            .selected()
            .and_then(|index| self.list.row(index))
            .map(|track| track.album.deezer_id.as_str().to_owned())
            .filter(|id| !id.is_empty())
        else {
            return;
        };
        self.state
            .update(cx, |state, _| state.nav.push(Route::Album { id: album }));
    }

    fn play_all(&mut self, cx: &mut Context<Self>) {
        let entries = self.list.entries().to_vec();
        if entries.is_empty() {
            return;
        }
        let context = self.list_context();
        self.state.update(cx, |state, cx| {
            state.command(nz_player::Command::play_queue(entries, 0, context), cx)
        });
    }

    fn list_context(&self) -> Option<nz_player::Context> {
        let credits = self.person.as_ref()?.ready()?;
        Some(nz_player::Context {
            kind: "credits".to_owned(),
            id: Some(credits.name.clone()),
            title: Some(credits.name.clone()),
        })
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if self.kind == CreditKind::Track {
            self.track = Phase::Loading;
        } else {
            self.person = None;
        }
        self.covers.borrow_mut().retry_failed();
        self.load(cx);
        cx.notify();
    }
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .font_family(theme::font(cx))
            .text_color(theme::text(cx))
            // The shell's own context (`app.rs`), restated so a screen is a
            // whole page on its own: a test that renders it without the shell
            // still gets the keymap's bindings, and no second context name can
            // drift from the shell's.
            .key_context("Neezer");
        let content = match self.kind {
            CreditKind::Track => self.track_body(cx),
            CreditKind::Person => self.person_body(window, cx),
        };
        root.child(content)
    }
}

impl Screen {
    fn track_body(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let title = self
            .track
            .ready()
            .and_then(|credits| {
                credits
                    .info
                    .get("album")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "Crédits du titre".to_owned());
        // A track's credits are one row per role, then the release line: as tall
        // as there are people, and taller than a 640 px pane for a fully
        // credited title. The page scroller is what makes the last role and the
        // ISRC reachable (D8a's rule: a content-sized column in a bounded
        // container, never a `flex_1` one).
        let mut body = page_scroll::column().gap(rems(0.75)).p(rems(1.75));
        match self.track.ready() {
            Some(credits) => {
                let credits = credits.clone();
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.75))
                        .child(
                            icon(IconName::Menu)
                                .size(rems(1.5))
                                .text_color(theme::accent(cx)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(rems(0.6875))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(theme::secondary(cx))
                                        .child("Crédits"),
                                )
                                .child(
                                    div()
                                        .text_size(rems(1.5))
                                        .font_weight(FontWeight::BOLD)
                                        .truncate()
                                        .child(title.clone()),
                                ),
                        ),
                );
                if credits.credits.is_empty() {
                    body = body.child(card::empty(
                        IconName::Info,
                        "Aucun crédit pour ce titre",
                        "Le serveur n'a pas encore lu les crédits de ce fichier.",
                        cx,
                        None,
                    ));
                } else {
                    for group in credits.credits.clone() {
                        let mut people = div().flex().flex_wrap().items_center().gap(rems(0.25));
                        for (index, person) in group.people.iter().enumerate() {
                            if index > 0 {
                                people = people.child(
                                    div()
                                        .text_size(rems(0.8125))
                                        .text_color(theme::secondary(cx))
                                        .child(","),
                                );
                            }
                            people = people.child(self.person_chip(person, cx));
                        }
                        body = body.child(
                            div()
                                .flex()
                                .items_start()
                                .gap(rems(1.))
                                .py(rems(0.5))
                                .border_b_1()
                                .border_color(theme::border(cx).opacity(0.5))
                                .child(
                                    div()
                                        .w(rems(11.))
                                        .flex_shrink_0()
                                        .text_size(rems(0.75))
                                        .text_color(theme::secondary(cx))
                                        .child(group.label.clone()),
                                )
                                .child(div().flex_1().child(people)),
                        );
                    }
                }
                let release = release_line(&credits.info);
                if !release.is_empty() {
                    body = body.child(
                        div()
                            .pt(rems(0.5))
                            .text_size(rems(0.75))
                            .text_color(theme::secondary(cx))
                            .child(release),
                    );
                }
            }
            None => match &self.track {
                Phase::Failed(error) => {
                    let cause = card::cause(error);
                    body = body.child(card::failed(
                        "Crédits indisponibles",
                        cause,
                        cx,
                        |this: &mut Self, _, cx| this.reload(cx),
                    ));
                }
                _ => {
                    body = body.child(
                        div()
                            .text_size(rems(0.8125))
                            .text_color(theme::secondary(cx))
                            .child("Chargement des crédits…"),
                    )
                }
            },
        }
        self.page
            .container("credits-track")
            .child(body)
            .into_any_element()
    }

    /// One name of one role: a link when there is somewhere to go, plain text
    /// otherwise — never a control that does nothing.
    fn person_chip(
        &mut self,
        person: &CreditPerson,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let navigable = person
            .deezer_id
            .as_ref()
            .is_some_and(|id| !id.as_str().is_empty())
            || person.id.is_some()
            || person.credited.is_some_and(|count| count > 0);
        let copy = person.clone();
        let mut chip = div()
            .id(format!("credit-person-{}", person.name))
            .text_size(rems(0.8125))
            .font_weight(FontWeight::SEMIBOLD)
            .child(person.name.clone());
        if navigable {
            chip = chip
                .cursor_pointer()
                .text_color(theme::text(cx))
                .hover(|style| style.text_color(theme::accent(cx)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.open_person(&copy, cx);
                    cx.notify();
                }));
        } else {
            chip = chip.text_color(theme::secondary(cx));
        }
        chip.into_any_element()
    }

    fn person_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let visible = self.list.visible();
        self.covers_for_visible(visible, cx);
        let mut body = div().size_full().flex().flex_col().min_h_0().p(rems(1.5));
        let Some(phase) = self.person.clone() else {
            return body
                .child(
                    div()
                        .text_size(rems(0.8125))
                        .text_color(theme::secondary(cx))
                        .child("Chargement des crédits…"),
                )
                .into_any_element();
        };
        let credits = match phase {
            Phase::Ready(credits) => credits,
            Phase::Failed(error) => {
                let cause = card::cause(&error);
                return body
                    .child(card::failed(
                        "Crédits indisponibles",
                        cause,
                        cx,
                        |this: &mut Self, _, cx| this.reload(cx),
                    ))
                    .into_any_element();
            }
            Phase::Loading => return body.into_any_element(),
        };
        body = body
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.75))
                    .child(
                        icon(IconName::Music)
                            .size(rems(1.5))
                            .text_color(theme::accent(cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(rems(0.6875))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::secondary(cx))
                                    .child("Crédits"),
                            )
                            .child(
                                div()
                                    .text_size(rems(1.5))
                                    .font_weight(FontWeight::BOLD)
                                    .child(credits.name.clone()),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        button("credits-play", "Tout lire", ButtonVariant::Primary, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.play_all(cx))),
                    ),
            )
            .child(
                div()
                    .py(rems(0.625))
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(format!(
                        "{} titre{} de la bibliothèque",
                        credits.tracks.len(),
                        if credits.tracks.len() > 1 { "s" } else { "" }
                    )),
            );
        if credits.roles.len() > 1 {
            let mut chips: Vec<gpui_kit::AnyElement> = Vec::new();
            chips.push(card::chip(
                "role-all".into(),
                &format!("Tout {}", credits.tracks.len()),
                self.role.is_empty(),
                cx,
                |this: &mut Self, _, cx| {
                    this.role.clear();
                    if let Some(Phase::Ready(credits)) = this.person.clone() {
                        this.apply_person(credits, cx);
                    }
                    cx.notify();
                },
            ));
            for role in &credits.roles {
                let label = format!("{} {}", role.label, role.count);
                let value = role.role.clone();
                chips.push(card::chip(
                    format!("role-{value}").into(),
                    &label,
                    self.role == value,
                    cx,
                    move |this: &mut Self, _, cx| {
                        this.role = value.clone();
                        if let Some(Phase::Ready(credits)) = this.person.clone() {
                            this.apply_person(credits, cx);
                        }
                        cx.notify();
                    },
                ));
            }
            body = body.child(card::chip_row(chips).pb(rems(0.5)));
        }
        let _ = &credits;
        let element = track_list::element(
            cx.entity(),
            "credits-list",
            &mut self.list,
            &self.state,
            Rc::clone(&self.covers),
            &self.menu,
            window,
            cx,
            |this: &mut Self, intent, window, cx| this.on_intent(intent, window, cx),
            |_, _, _, _| None,
            |_: &mut Self, _: Item, _: &mut Context<Self>| {},
            |_: &mut Self, _: &mut Context<Self>| {},
        );
        body.child(div().flex_1().min_h_0().child(element))
            .into_any_element()
    }
}

/// The release line a track's credits carry: the album, the date, the label, the
/// copyright and the ISRC, joined as the sleeve prints them.
fn release_line(info: &BTreeMap<String, serde_json::Value>) -> String {
    let text = |key: &str| {
        info.get(key)
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let mut parts: Vec<String> = Vec::new();
    if let Some(album) = text("album") {
        parts.push(album);
    }
    if let Some(released) = text("released") {
        parts.push(format_date(&released));
    }
    if let Some(label) = text("label") {
        parts.push(label);
    }
    let mut line = parts.join(" · ");
    if let Some(isrc) = text("isrc") {
        if !line.is_empty() {
            line.push_str(" · ");
        }
        line.push_str(&format!("ISRC {isrc}"));
    }
    line
}

/// `2001-03-12` → `12 mars 2001`, the server's own dates in French.
fn format_date(raw: &str) -> String {
    let parts: Vec<&str> = raw.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return raw.to_owned();
    };
    let (Ok(year), Ok(month), Ok(day)) = (
        year.parse::<u32>(),
        month.parse::<usize>(),
        day.parse::<u32>(),
    ) else {
        return raw.to_owned();
    };
    const MONTHS: [&str; 12] = [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ];
    match MONTHS.get(month.saturating_sub(1)) {
        Some(name) => format!("{day} {name} {year}"),
        None => raw.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{ListFirst, ListLast, ListPageDown};
    use crate::widgets::card::stub;

    const ME: &str =
        r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#;

    /// A track's credits as a sleeve prints them: one group per role, several
    /// people per group — a fully credited title, which is taller than a pane.
    fn track_credits_body(groups: usize) -> String {
        let groups: Vec<String> = (0..groups)
            .map(|index| {
                format!(
                    r#"{{"role":"role_{index}","label":"Rôle {index}","people":[
                        {{"name":"Personne A{index}","credited":12}},
                        {{"name":"Personne B{index}","deezer_id":"77"}},
                        {{"name":"Personne C{index}","id":"uuid-{index}"}}]}}"#
                )
            })
            .collect();
        format!(
            r#"{{"credits":[{}],"info":{{"album":"Songs from the Inside",
                "released":"2004-03-12","label":"Interchill","isrc":"GBABC0400012"}}}}"#,
            groups.join(",")
        )
    }

    fn person_credits_body(tracks: usize) -> String {
        let roles = r#"[{"role":"composer","label":"Composition","count":3},
                        {"role":"author","label":"Paroles","count":2}]"#;
        let rows: Vec<String> = (0..tracks)
            .map(|index| {
                format!(
                    r#"{{"id":"row-{index}","deezer_id":"5000{index}","title":"Titre {index}",
                        "duration":190,"credit_roles":["composer"],
                        "artist":{{"deezer_id":"42","name":"Kaya Project"}},
                        "album":{{"deezer_id":"9","title":"Songs from the Inside",
                        "cover":"/api/cover/9"}}}}"#
                )
            })
            .collect();
        format!(
            r#"{{"name":"Aiyana","labels":{{"composer":"Composition","author":"Paroles"}},
                "roles":{roles},"tracks":[{}]}}"#,
            rows.join(",")
        )
    }

    /// **A track's credits scroll** (v1.6.1). The page had NO scroll container
    /// at all: a title credited on twelve roles (three people each) painted its
    /// last roles and its release line below the window, unreachable — no wheel,
    /// no touchpad, no key. Now the page is a content-sized column in a bounded
    /// scroller (`widgets/page_scroll`), so: a wheel notch **76 px**,
    /// `Page suivante` **640 px** (0.8 of a 800 px window), `Fin` the bottom and
    /// `Début` back to the top, at 1280×800 and in the 660 px pane.
    #[gpui::test]
    async fn the_track_credits_page_scrolls_with_the_wheel_and_the_keys(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TouchPhase, point, px, size};

        let body = track_credits_body(30);
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/track/3000/credits", body.as_str()),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (screen, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| {
                Screen::new(
                    Route::Credits {
                        kind: CreditKind::Track,
                        id: "3000".into(),
                    },
                    state,
                    cx,
                )
            }
        });
        stub::until(&runtime, cx, "the track's credits", |cx| {
            screen.read(cx).track.ready().is_some()
        });

        let paint = |cx: &mut gpui::VisualTestContext| {
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            })
        };
        let offset =
            |cx: &mut gpui::VisualTestContext| cx.update(|_, cx| screen.read(cx).page.offset().y);
        let range =
            |cx: &mut gpui::VisualTestContext| cx.update(|_, cx| screen.read(cx).page.range());

        for (width, what) in [(1280., "1280×800"), (660., "the 660 px pane")] {
            cx.simulate_resize(size(px(width), px(800.)));
            paint(cx);
            let travel = range(cx);
            assert!(
                travel > px(200.),
                "{what}: thirty roles do not fit a 800 px window; measured {travel:?}"
            );
            assert_eq!(offset(cx), px(0.), "{what}: a fresh page starts at the top");

            cx.simulate_mouse_move(point(px(width / 2.), px(400.)), None, Modifiers::default());
            cx.simulate_event(ScrollWheelEvent {
                position: point(px(width / 2.), px(400.)),
                delta: ScrollDelta::Lines(point(0., -3.)),
                modifiers: Modifiers::default(),
                touch_phase: TouchPhase::Moved,
            });
            paint(cx);
            let wheeled = offset(cx);
            assert!(
                wheeled < px(-10.),
                "{what}: the wheel scrolls the credits; measured {wheeled:?}"
            );

            cx.simulate_keystrokes("pagedown");
            paint(cx);
            let paged = offset(cx);
            assert!(
                paged < wheeled - px(200.),
                "{what}: Page suivante moves most of the window; measured {paged:?}"
            );
            cx.simulate_keystrokes("end");
            paint(cx);
            assert_eq!(offset(cx), -travel, "{what}: Fin reaches the bottom");
            cx.simulate_keystrokes("home");
            paint(cx);
            assert_eq!(offset(cx), px(0.), "{what}: Début returns to the top");
            println!(
                "credits_page {what}: range={travel:?} wheel={:?} pagedown={:?}",
                wheeled,
                paged - wheeled
            );
        }
        runtime.shutdown_background();
        stub.stop();
    }

    /// **A person's credits scroll through their titles** — the shared windowed
    /// list, whose own keys and wheel are what the page has. Pinned here because
    /// this page's list has no surrounding scroller: the list IS the scroller,
    /// and a 400-row person still builds a screenful.
    #[gpui::test]
    async fn the_person_credits_list_scrolls_with_the_wheel_and_the_keys(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TouchPhase, point, px, size};

        let body = person_credits_body(400);
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/credits/person", body.as_str()),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (screen, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| {
                Screen::new(
                    Route::Credits {
                        kind: CreditKind::Person,
                        id: "Aiyana".into(),
                    },
                    state,
                    cx,
                )
            }
        });
        stub::until(&runtime, cx, "the person's credits", |cx| {
            screen.read(cx).list.len() == 400
        });

        let paint = |cx: &mut gpui::VisualTestContext| {
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            })
        };
        let seen = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, cx| {
                let list = &screen.read(cx).list;
                (list.scroll_y(), list.visible(), list.selected())
            })
        };

        for (width, what) in [(1280., "1280×800"), (660., "the 660 px pane")] {
            cx.simulate_resize(size(px(width), px(800.)));
            paint(cx);
            let (offset, visible, selected) = seen(cx);
            assert_eq!(offset, px(0.), "{what}: a fresh list starts at the top");
            assert_eq!(selected, Some(0));
            assert!(
                visible.len() < 60,
                "{what}: 400 titles build a screenful ({visible:?})"
            );

            cx.simulate_mouse_move(point(px(width / 2.), px(500.)), None, Modifiers::default());
            cx.simulate_event(ScrollWheelEvent {
                position: point(px(width / 2.), px(500.)),
                delta: ScrollDelta::Lines(point(0., -3.)),
                modifiers: Modifiers::default(),
                touch_phase: TouchPhase::Moved,
            });
            paint(cx);
            let (after_wheel, wheeled, _) = seen(cx);
            assert!(
                after_wheel < px(-10.),
                "{what}: the wheel scrolled the person's titles; measured {after_wheel:?}"
            );
            assert!(wheeled.start > 0, "{what}: the rows moved; {wheeled:?}");

            cx.update(|_, cx| cx.dispatch_action(&ListPageDown));
            cx.run_until_parked();
            paint(cx);
            let (after_page, paged, row) = seen(cx);
            assert!(
                after_page < after_wheel - px(200.),
                "{what}: Page suivante moves most of the window; measured {after_page:?}"
            );
            assert!(
                paged.contains(&row.expect("a selected row")),
                "{what}: the selected row is painted; {row:?} not in {paged:?}"
            );

            cx.update(|_, cx| cx.dispatch_action(&ListLast));
            cx.run_until_parked();
            paint(cx);
            let (bottom, last, row) = seen(cx);
            assert_eq!(row, Some(399), "{what}: Fin reaches the last title");
            assert!(last.contains(&399), "{what}: it is painted; {last:?}");
            assert!(
                bottom < px(-1_000.),
                "{what}: the list went down; {bottom:?}"
            );

            cx.update(|_, cx| cx.dispatch_action(&ListFirst));
            cx.run_until_parked();
            paint(cx);
            assert_eq!(seen(cx).0, px(0.), "{what}: Début returns to the top");
            println!(
                "credits_list {what}: wheel={after_wheel:?} page={:?} bottom={bottom:?}",
                after_page - after_wheel
            );
        }
        runtime.shutdown_background();
        stub.stop();
    }
}
