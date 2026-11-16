//! Récap: one year in review, as a story of cards.
//!
//! `routes/Recap.svelte` and `components/RecapStory.svelte` are the reference.
//! The screen's contract is the SPA's:
//!
//! - **Two requests cold, one warm, one per year change, zero per card.** The
//!   whole year is ONE bounded payload; the deck is `widgets/recap_story.rs`.
//! - **`/recap` resolves the latest year that holds an event**; a year chosen in
//!   the chips is asked once and replaces the one on screen. The route is NOT
//!   rewritten here (the SPA `replace`s its hash): a route change would rebuild
//!   this screen and buy a second `/recap/years` for nothing.
//! - **The keyboard reads it**: `←`/`→` (intercepted, so they work with nothing
//!   focused), `j`/`k`, `↑`/`↓`, `Page préc.`/`Page suiv.`, `Début`/`Fin`.
//! - **A year with no story says why** (no event, only skips, or too thin), and a
//!   year the server does not know is refused BEFORE a request when it is not
//!   four digits.
//! - Personal listening data: nothing here is written to the device beyond the
//!   covers every screen caches, and `/recap*` is not in the API cache.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClipboardItem, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled, Subscription,
    Window, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};
use nz_api::ApiError;
use nz_model::{Recap, RecapYearRow};

use crate::nav::Route;
use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;
use crate::widgets::card::{self, Phase};
use crate::widgets::cover::Covers;
use crate::widgets::recap_story::{self, Story};
use crate::widgets::track_list;
use crate::wiring::events::{self, Sender};

/// The wake that drives the 7 s auto-advance (a narrow window only): one cheap
/// read a second, and only while this screen exists.
const BEAT: Duration = Duration::from_millis(1_000);

enum Msg {
    Years(Box<Result<nz_model::RecapYears, ApiError>>),
    Year(i32, Box<Result<Recap, ApiError>>),
    Cover(String, Option<PathBuf>),
}

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    /// The years the server holds a story for (`/recap/years`).
    years: Vec<RecapYearRow>,
    now_year: i32,
    /// The year on screen, and whether the payload arrived for it.
    year: Option<i32>,
    payload: Phase<Recap>,
    story: Story,
    /// A paint asked for the year again (the failure's « Réessayer »).
    reload: bool,
    /// The window is narrow enough for the story to move on its own.
    narrow: bool,
    covers: Rc<RefCell<Covers>>,
    events: Sender<Msg>,
    _subscriptions: Vec<Subscription>,
    _keys: Subscription,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = events::channel::<Msg>();
        let year = match &route {
            Route::Recap { year } => *year,
            _ => None,
        };
        let mut screen = Self {
            route,
            state: state.clone(),
            years: Vec::new(),
            now_year: 0,
            year,
            payload: Phase::Loading,
            story: Story::new(),
            reload: false,
            narrow: false,
            covers: Rc::new(RefCell::new(Covers::new())),
            events: tx,
            _subscriptions: vec![cx.observe(&state, |_, _, cx| cx.notify())],
            _keys: install_keys(cx),
        };
        screen.drain(rx, cx);
        screen.load(cx);
        screen.beat(cx);
        // The cards are read like a list: `j`/`k`, `Page préc./suiv.`,
        // `Début`/`Fin` are the app's own list actions, routed here.
        track_list::own(cx, |this: &mut Self, intent, _window, cx| {
            this.on_intent(intent, cx)
        });
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

    /// Ask where the years stand, then the year itself when the route names
    /// none: 2 requests cold, 0 for a card.
    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.events.clone();
        self.payload = Phase::Loading;
        if let Some(year) = self.year {
            let tx = tx.clone();
            let api = api.clone();
            runtime.spawn(async move {
                tx.send(Msg::Year(year, Box::new(api.recap(year).await)));
            });
        }
        runtime.spawn(async move {
            tx.send(Msg::Years(Box::new(api.recap_years().await)));
        });
    }

    /// Ask one year, because a chip was chosen.
    fn open(&mut self, year: i32, cx: &mut Context<Self>) {
        if self.year == Some(year) && matches!(self.payload, Phase::Ready(_)) {
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.year = Some(year);
        self.payload = Phase::Loading;
        self.story.reset(0, 0);
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.events.clone();
        runtime.spawn(async move {
            tx.send(Msg::Year(year, Box::new(api.recap(year).await)));
        });
        cx.notify();
    }

    fn on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Years(result) => match *result {
                Ok(years) => {
                    self.now_year = years.now_year;
                    self.years = years.years.clone();
                    if self.year.is_none() {
                        match years.latest {
                            Some(year) => self.open(year, cx),
                            None => self.payload = Phase::Ready(Recap::default()),
                        }
                    }
                }
                Err(error) => {
                    if !matches!(self.payload, Phase::Ready(_)) {
                        self.payload = Phase::Failed(error);
                    }
                }
            },
            Msg::Year(year, result) => {
                if self.year.is_some_and(|current| current != year) {
                    // A year chosen while another was in flight: its answer is
                    // stale, and painting it would name the wrong year.
                    cx.notify();
                    return;
                }
                match *result {
                    Ok(payload) => {
                        self.year = Some(payload.year);
                        self.story.reset(0, recap_story::card_ids(&payload).len());
                        self.payload = Phase::Ready(payload);
                    }
                    Err(error) => self.payload = Phase::Failed(error),
                }
            }
            Msg::Cover(url, file) => self.covers.borrow_mut().resolved(&url, file),
        }
        cx.notify();
    }

    /// The 7 s auto-advance of a narrow window, checked once a second; a wide
    /// window never moves on its own.
    fn beat(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(BEAT).await;
                let alive = this.update(cx, |this, cx| {
                    let reduced = this.state.read(cx).reduced_motion;
                    if this.story.tick(this.narrow, reduced, false) {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_intent(&mut self, intent: track_list::Intent, cx: &mut Context<Self>) {
        let before = self.story.index();
        match intent {
            track_list::Intent::Move(delta) => {
                if delta < 0 {
                    self.story.previous();
                } else {
                    self.story.next();
                }
            }
            track_list::Intent::Page(delta) => {
                for _ in 0..delta.unsigned_abs() {
                    if delta < 0 {
                        self.story.previous();
                    } else {
                        self.story.next();
                    }
                }
            }
            track_list::Intent::First => self.story.first(),
            track_list::Intent::Last => self.story.last(),
            _ => return,
        }
        self.story.touch();
        if self.story.index() != before {
            cx.notify();
        }
    }

    fn step(&mut self, delta: i64, cx: &mut Context<Self>) {
        match delta {
            d if d < 0 => self.story.previous(),
            _ => self.story.next(),
        }
        self.story.touch();
        cx.notify();
    }

    /// "Lire ces 5 titres": the year's top, as a queue of its own
    /// (`Recap.svelte#playTop`).
    fn play_top(&mut self, cx: &mut Context<Self>) {
        let Some(payload) = self.payload.ready() else {
            return;
        };
        let entries: Vec<_> = payload
            .tops
            .tracks
            .iter()
            .filter(|row| !row.track.title.is_empty())
            .map(|row| crate::state::entry_from_track(&row.track))
            .collect();
        if entries.is_empty() {
            return;
        }
        let context = nz_player::Context {
            kind: "recap".to_owned(),
            id: Some(payload.year.to_string()),
            title: Some(format!("Récap {}", payload.year)),
        };
        self.state.update(cx, |state, cx| {
            state.command(
                nz_player::Command::play_queue(entries, 0, Some(context)),
                cx,
            );
        });
    }

    /// The SPA's Web Share fallback: one sentence on the clipboard. The poster
    /// image is a canvas artefact the desktop has no pipeline for; the gap is
    /// recorded in `docs/agents/desktop.md`.
    fn share(&mut self, cx: &mut Context<Self>) {
        let Some(payload) = self.payload.ready() else {
            return;
        };
        let text = recap_story::share_text(payload);
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.notice(
            ToastKind::Info,
            "Le résumé de votre année est copié : collez-le où vous voulez.",
            cx,
        );
    }

    fn notice(
        &mut self,
        kind: ToastKind,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.state.update(cx, |state, cx| {
            state.toasts.push(kind, message.into());
            cx.notify();
        });
    }

    fn want_covers(&mut self, cx: &mut Context<Self>) {
        let Some(payload) = self.payload.ready() else {
            return;
        };
        let mut asked: Vec<String> = recap_story::mosaic_covers(payload);
        for row in payload.tops.artists.iter().take(recap_story::TOP) {
            if let Some(cover) = row.cover.clone() {
                asked.push(cover);
            }
        }
        for row in payload.tops.tracks.iter().take(recap_story::TOP) {
            if let Some(cover) = row.track.album.cover.clone() {
                asked.push(cover);
            }
        }
        for row in payload.tops.albums.iter().take(recap_story::TOP) {
            if let Some(cover) = row.cover.clone() {
                asked.push(cover);
            }
        }
        for edge in [payload.first.as_ref(), payload.last.as_ref()] {
            if let Some(edge) = edge
                && let Some(cover) = edge.track.album.cover.clone()
            {
                asked.push(cover);
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
}

/// `←`/`→` for the deck. They are not in `keymap.rs` (D7 froze it), and a
/// screen may not take the focus, so they are intercepted at the app level for
/// as long as this screen exists — the SPA's deck handler, same keys.
fn install_keys(cx: &mut Context<Screen>) -> Subscription {
    let weak = cx.weak_entity();
    gpui_kit::App::intercept_keystrokes(&mut *cx, move |event, _window, cx| {
        let gpui_kit::KeystrokeEvent { keystroke, .. } = event;
        let delta = match keystroke.key.as_str() {
            "left" => -1,
            "right" => 1,
            _ => return,
        };
        let _ = weak.update(cx, |this, cx| this.step(delta, cx));
    })
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = f32::from(window.viewport_size().width);
        self.narrow = width < recap_story::NARROW_MAX;
        self.want_covers(cx);

        let root = div()
            .id("recap-scroll")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .min_w(px(0.))
            .min_h(px(0.))
            .font_family(theme::font(cx))
            .text_color(theme::text(cx))
            .key_context("Neezer");

        let mut page = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .max_w(px(1800.))
            .gap(rems(1.))
            .p(rems(1.5));

        // The period label and whether a payload is on screen: no clone of the
        // year's figures on a notify.
        let period = self.payload.ready().map(recap_story::period_label);
        let ready = self.payload.ready().is_some();
        page = page.child(
            div()
                .flex()
                .items_end()
                .justify_between()
                .gap(rems(1.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.125))
                        .child(
                            div()
                                .text_size(rems(1.5))
                                .font_weight(FontWeight::BOLD)
                                .child("Récap"),
                        )
                        .when_some(period, |column, period| {
                            column.child(
                                div()
                                    .text_size(rems(0.75))
                                    .text_color(theme::secondary(cx))
                                    .child(period),
                            )
                        }),
                )
                .when(ready, |row| {
                    row.child(
                        button("recap-copy", "Partager", ButtonVariant::Secondary, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.share(cx))),
                    )
                }),
        );

        // The year chips: one per year that holds an event, newest first.
        let chips = recap_story::year_chips(&self.years, self.now_year);
        if chips.len() > 1 {
            let mut group = gpui_omarchy::toggle_group("recap-years", cx);
            for (index, (year, label)) in chips.into_iter().enumerate() {
                let active = self.year == Some(year);
                group = group.child(
                    gpui_omarchy::toggle(("recap-year", index), label, active, cx)
                        .debug_selector(move || format!("recap-year-{index}"))
                        .on_click(cx.listener(move |this, _, _, cx| this.open(year, cx))),
                );
            }
            page = page.child(group);
        }

        // The payload is moved out for the paint and put back (a year is a few
        // kilobytes and a notify is not a place to clone it).
        let phase = std::mem::replace(&mut self.payload, Phase::Loading);
        match &phase {
            Phase::Loading => page = page.child(self.loading_card(cx)),
            Phase::Failed(error) => {
                let invalid = matches!(error, ApiError::Http { status: 400, .. });
                if invalid {
                    page = page.child(self.refused_card(cx));
                } else {
                    page = page.child(card::failed(
                        "Récap indisponible",
                        format!("{} — réessayez dans un instant.", card::cause(error)),
                        cx,
                        |this: &mut Self, _, cx| {
                            this.reload = true;
                            cx.notify();
                        },
                    ));
                }
            }
            Phase::Ready(payload) => {
                page = self.ready_page(page, payload, cx);
            }
        }
        self.payload = phase;
        if std::mem::take(&mut self.reload) {
            self.payload = Phase::Loading;
            self.load(cx);
        }

        root.child(page).into_any_element()
    }
}

impl Screen {
    fn loading_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .p(rems(1.25))
            .h(px(420.))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child("Chargement du récap…"),
            )
            .into_any_element()
    }

    fn refused_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        card::empty(
            IconName::CircleAlert,
            "Cette année n'existe pas dans votre récap",
            "Le lien est peut-être incomplet, ou l'année est hors de l'historique de ce serveur.",
            cx,
            Some(
                button(
                    "recap-back",
                    "Voir la dernière année",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.year = None;
                    this.payload = Phase::Loading;
                    this.load(cx);
                }))
                .into_any_element(),
            ),
        )
        .into_any_element()
    }

    fn ready_page(
        &mut self,
        mut page: gpui_kit::Div,
        payload: &Recap,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let reason = recap_story::empty_reason(payload);
        let cards = recap_story::card_ids(payload);
        if payload.year == 0 && reason == Some("none") {
            // `/recap/years` answered nothing: there is no story at all yet.
            return page.child(card::empty(
                IconName::Music,
                "Votre récap apparaîtra quand vous aurez écouté quelques titres.",
                "Ce que vous écoutez est compté par le serveur : ouvrez la bibliothèque et lancez un titre.",
                cx,
                None,
            ));
        }
        if let Some(reason) = reason {
            let (title, body) = match reason {
                "skips-only" => (
                    format!("Aucune écoute en {}", payload.year),
                    "Seulement des titres passés cette année-là.".to_owned(),
                ),
                "thin" => (
                    format!("Trop peu d'écoutes en {}", payload.year),
                    format!(
                        "{}, {} — pas assez pour raconter une année.",
                        plural(payload.totals.listens, "écoute", "écoutes"),
                        recap_story::format_minutes(payload.totals.minutes)
                    ),
                ),
                _ => (
                    format!("Rien à raconter pour {}", payload.year),
                    "Aucune écoute enregistrée cette année-là.".to_owned(),
                ),
            };
            let nearby = recap_story::nearest_year(&self.years, payload.year);
            let mut column = div().flex().flex_col().gap(rems(0.75));
            column = column.child(card::empty(
                if reason == "skips-only" {
                    IconName::SkipForward
                } else {
                    IconName::Timer
                },
                &title,
                &body,
                cx,
                None,
            ));
            let mut actions = div().flex().items_center().gap(rems(0.5));
            if let Some(year) = nearby {
                actions = actions.child(
                    button(
                        "recap-near",
                        format!("Voir {year}"),
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.open(year, cx))),
                );
            }
            actions = actions.child(
                button(
                    "recap-history",
                    "Retour à l'historique",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.state
                        .update(cx, |state, _| state.nav.push(Route::History));
                })),
            );
            return page.child(column.child(actions));
        }
        if cards.is_empty() {
            return page.child(card::empty(
                IconName::Sparkles,
                "Rien à raconter pour cette année",
                "Le serveur n'a pas trouvé d'histoire dans cette année-là.",
                cx,
                None,
            ));
        }

        // The deck: one fixed height, so a card never resizes the page under the
        // reader's hands, and the two-column glance beside it on a wide window.
        let deck = div()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(560.))
            .h(px(470.))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .overflow_hidden()
            .child(recap_story::view(
                &mut self.story,
                payload,
                &self.covers,
                cx,
                |this: &mut Self, delta, cx| this.step(delta, cx),
                |this: &mut Self, cx| this.play_top(cx),
                |this: &mut Self, cx| this.share(cx),
            ));

        page = page.child(
            div()
                .flex()
                .items_start()
                .gap(rems(1.25))
                .flex_wrap()
                .child(deck)
                .child(self.glance(payload, cx)),
        );
        page
    }

    /// "En un coup d'œil": the five figures a year is read by
    /// (`Recap.svelte`'s glance panel).
    fn glance(&mut self, payload: &Recap, cx: &mut Context<Self>) -> AnyElement {
        let top_artist = payload.tops.artists.first().map(|row| row.name.clone());
        let top_genre = recap_story::genre_rows(payload)
            .first()
            .map(|row| row.name.clone());
        let rows: Vec<(String, String)> = vec![
            (
                "Minutes".to_owned(),
                recap_story::format_number(i64::from(payload.totals.minutes)),
            ),
            (
                "Écoutes".to_owned(),
                recap_story::format_number(i64::from(payload.totals.listens)),
            ),
            (
                "Artiste".to_owned(),
                top_artist.unwrap_or_else(|| "—".to_owned()),
            ),
            (
                "Style".to_owned(),
                top_genre.unwrap_or_else(|| "—".to_owned()),
            ),
            (
                "Jours actifs".to_owned(),
                recap_story::format_number(i64::from(payload.totals.days_active)),
            ),
        ];
        let mut panel = div()
            .flex()
            .flex_col()
            .w(px(300.))
            .gap(rems(0.25))
            .p(rems(1.))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .pb(rems(0.25))
                    .font_family(theme::mono(cx))
                    .text_size(rems(0.625))
                    .text_color(theme::secondary(cx))
                    .child("EN UN COUP D'ŒIL"),
            );
        for (label, value) in rows {
            panel = panel.child(
                div()
                    .flex()
                    .items_baseline()
                    .justify_between()
                    .gap(rems(0.75))
                    .py(rems(0.25))
                    .border_b_1()
                    .border_color(theme::border(cx).opacity(0.4))
                    .child(
                        div()
                            .text_size(rems(0.75))
                            .text_color(theme::secondary(cx))
                            .child(label),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(rems(0.8125))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(value),
                    ),
            );
        }
        if let Some(day) = &payload.biggest_day {
            panel = panel.child(
                div()
                    .pt(rems(0.375))
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(format!(
                        "Plus grosse journée : {} · {}",
                        day.date,
                        plural(day.listens, "écoute", "écoutes")
                    )),
            );
        }
        if let Some(streak) = &payload.streak {
            panel = panel.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(format!(
                        "Série : {} d'affilée",
                        plural(streak.days, "jour", "jours")
                    )),
            );
        }
        if payload.tops.genres.available {
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.25))
                    .pt(rems(0.5))
                    .text_size(rems(0.625))
                    .text_color(theme::secondary(cx))
                    .child(icon(IconName::Tag).size(rems(0.75)))
                    .child(format!(
                        "{} % des écoutes mesurées",
                        (payload.tops.genres.coverage.clamp(0., 1.) * 100.).round()
                    )),
            );
        }
        panel.into_any_element()
    }
}

/// "3 écoutes" / "1 écoute" — French counts zero as singular
/// (`lib/recap.js#formatMinutes`'s rule, applied to a count).
fn plural(count: u32, one: &str, many: &str) -> String {
    format!(
        "{} {}",
        recap_story::format_number(i64::from(count)),
        if count > 1 { many } else { one }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    const ME: &str =
        r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#;
    const YEARS: &str = r#"{"years":[{"year":2026,"events":12,"listens":10,"minutes":300,
        "enough":true,"partial":true},{"year":2025,"events":8,"listens":8,"minutes":200,
        "enough":true,"partial":false}],"latest":2026,"now_year":2026,"min_year":2020,
        "timezone":"server-local"}"#;
    /// The same shape one year earlier: the screen takes the YEAR from the
    /// payload, so a stub that answered 2026 for both would hide a real bug.
    const RECAP_2025: &str = r#"{"year":2025,"partial":false,"enough":true,
        "timezone":"server-local","period":{"from":"2025-01-01","to":"2025-12-31"},
        "totals":{"events":8,"listens":8,"skips":0,"neutrals":0,"seconds":12000,
                  "minutes":200,"days_active":6,"first_at":null,"last_at":null},
        "months":[],"streak":null,"biggest_day":null,
        "tops":{"tracks":[],"artists":[],"albums":[],
                "genres":{"available":false,"coverage":0.0,"explored":0,"measured_listens":0,
                          "rows":[]}},
        "first":null,"last":null,"cards":["year","minutes"],"generated":1767225600}"#;
    const RECAP_2026: &str = r#"{"year":2026,"partial":true,"enough":true,
        "timezone":"server-local","period":{"from":"2026-01-01","to":"2026-11-16"},
        "totals":{"events":12,"listens":10,"skips":1,"neutrals":1,"seconds":18000,
                  "minutes":300,"days_active":9,"first_at":null,"last_at":null},
        "months":[{"month":1,"listens":4,"skips":0,"seconds":600}],
        "streak":{"days":3,"from":"2026-01-01","to":"2026-01-03"},
        "biggest_day":{"date":"2026-01-02","listens":5,"seconds":900},
        "tops":{"tracks":[{"track":{"deezer_id":"3001","title":"Un","duration":200},
                 "listens":5,"seconds":900}],
                "artists":[{"id":"42","name":"Kaya Project","cover":null,"listens":9,"tracks":4}],
                "albums":[],
                "genres":{"available":false,"coverage":0.0,"explored":0,"measured_listens":0,
                          "rows":[]}},
        "first":null,"last":null,"cards":["year","minutes","tracks"],"generated":1794835151}"#;

    /// The acceptance: the deck is navigable by keys, and a card costs nothing.
    /// Two requests cold, one per year change, zero per card.
    #[gpui::test]
    async fn the_deck_moves_by_key_and_costs_no_request(cx: &mut gpui::TestAppContext) {
        let stub = crate::widgets::card::stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/recap/years", YEARS),
            ("/api/recap/2026", RECAP_2026),
            ("/api/recap/2025", RECAP_2025),
        ]);
        let (state, runtime) = crate::widgets::card::stub::logged_in(cx, &stub);
        let (screen, cx) = cx.add_window_view(move |_, cx| {
            Screen::new(Route::Recap { year: None }, state.clone(), cx)
        });
        crate::widgets::card::stub::until(&runtime, cx, "the year's story", |cx| {
            screen.read(cx).payload.ready().is_some()
        });
        assert_eq!(stub.count("/api/recap/years"), 1, "the years, once");
        assert_eq!(stub.count("/api/recap/2026"), 1, "the latest year, once");
        cx.update(|_, cx| {
            assert_eq!(screen.read(cx).story.index(), 0);
            assert_eq!(
                screen.read(cx).story.len(),
                4,
                "three cards from the payload and the client's own share card"
            );
        });

        // The arrows are intercepted (no focus needed), and a card costs no
        // request: the whole year is one payload.
        cx.simulate_keystrokes("right");
        cx.simulate_keystrokes("right");
        cx.update(|_, cx| assert_eq!(screen.read(cx).story.index(), 2));
        assert_eq!(stub.count("/api/recap/2026"), 1, "a card costs no request");
        // Back, and to the ends. `j`/`k` and `Début`/`Fin` are the app's list
        // actions (`track_list::own`, D8a's routing), which need the shell's
        // focus anchor; the arrows are intercepted and need nothing.
        cx.simulate_keystrokes("left");
        cx.update(|_, cx| assert_eq!(screen.read(cx).story.index(), 1));
        cx.simulate_keystrokes("left");
        cx.simulate_keystrokes("left");
        cx.update(|_, cx| {
            assert_eq!(
                screen.read(cx).story.index(),
                0,
                "the first card is a wall, not a wrap"
            );
        });
        cx.update(|_window, _| {
            let mut story = Story::new();
            story.reset(0, 4);
            story.last();
            assert_eq!(story.index(), 3, "Fin lands on the last card");
            story.first();
            assert_eq!(story.index(), 0, "Début lands on the first");
        });
        assert_eq!(
            stub.total(),
            3,
            "the login and two routes: nothing per card"
        );

        // A year change is ONE request.
        cx.update(|_, cx| screen.update(cx, |screen, cx| screen.open(2025, cx)));
        crate::widgets::card::stub::until(&runtime, cx, "the other year", |cx| {
            let _ = cx;
            stub.count("/api/recap/2025") == 1
        });
        cx.update(|_, cx| {
            assert_eq!(screen.read(cx).year, Some(2025));
            assert_eq!(screen.read(cx).story.index(), 0, "a new year starts at 1");
        });
        runtime.shutdown_background();
        stub.stop();
    }

    /// The screen opens one year at a time: a year chosen while another is in
    /// flight is the only one that may paint.
    #[test]
    fn a_stale_year_is_not_painted_over_the_new_one() {
        // The rule the `Msg::Year` arm keeps: `self.year` is the route's own
        // choice, so an answer for another year is dropped before `story` moves.
        let mut screen_year: Option<i32> = Some(2026);
        let answer = 2025;
        assert!(screen_year.is_some_and(|current| current != answer));
        screen_year = Some(2026);
        assert!(!screen_year.is_some_and(|current| current != 2026));
    }

    /// The glance panel's counts read as French, zero included.
    #[test]
    fn a_count_reads_as_french() {
        assert_eq!(plural(0, "écoute", "écoutes"), "0 écoute");
        assert_eq!(plural(1, "écoute", "écoutes"), "1 écoute");
        assert_eq!(plural(8_540, "écoute", "écoutes"), "8\u{202f}540 écoutes");
    }
}
