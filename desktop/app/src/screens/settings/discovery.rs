//! Réglages → Découverte: how far the Flow reaches, what was pushed away, and
//! (for an administrator) what the releases check watches.
//!
//! The SPA's `DiscoverySlider`, `DislikedTracks` and the *Nouveautés* card of
//! `routes/Settings.svelte`. The level is the person's and lives on the SERVER
//! (`/discover/prefs`), so the phone and the desktop agree; a copy on the device
//! paints the control before the server answers, and a save is one request per
//! click — never one per pixel, because there is no drag: the track is twenty
//! hit zones and four named steps, so a click is a value.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement as _, Styled, Window, div, rems,
};
use gpui_omarchy::{ButtonVariant, button, switch};
use nz_model::{DiscoverStep, ReleasesSettings, Track, TrackList};

use super::{Load, Screen, card, card_title, eid, failed, hint, loading, value_line};
use crate::theme;

/// The device's copy of the notch (`lib/discovery.js#discoveryLevel`), read
/// before the server answers so the slider never starts at zero.
pub const LEVEL_KEY: &str = "discover.level";
/// `lib/discovery.js#DEFAULT_LEVEL`.
pub const DEFAULT_LEVEL: f32 = 0.35;

/// `lib/discovery.js#STEPS`: the names drawn before the server's own list
/// arrives. Every value here is a start level, not a label.
pub const STEPS: [(&str, f32); 4] = [
    ("Proche", 0.),
    ("Voisin", 0.25),
    ("Exploration", 0.5),
    ("Aventure", 0.75),
];

/// The step a level falls in: the highest start it reaches. The server answers
/// its own list (`/discover/prefs`), so this is only the local copy's answer —
/// the two agree on the four names, and a server that renamed them is drawn
/// with its names.
pub fn step_of(level: f32, steps: &[DiscoverStep]) -> String {
    let level = if level.is_finite() {
        level.clamp(0., 1.)
    } else {
        0.
    };
    let mut current = String::new();
    if steps.is_empty() {
        for (name, from) in STEPS {
            if level >= from {
                current = name.to_owned();
            }
        }
        return current;
    }
    for step in steps {
        if level as f64 >= step.from_level {
            current = step.name.clone();
        }
    }
    current
}

pub fn body(screen: &mut Screen, _window: &mut Window, cx: &mut Context<Screen>) -> AnyElement {
    let mut column = div().flex().flex_col().gap(rems(1.));
    column = column.child(notch(screen, cx));
    column = column.child(dislikes(screen, cx));
    if screen.is_admin(cx) {
        column = column.child(releases(screen, cx));
    }
    column.into_any_element()
}

// --- the notch -------------------------------------------------------------

fn notch(screen: &mut Screen, cx: &mut Context<Screen>) -> AnyElement {
    let level = screen.discover_level.clamp(0., 1.);
    let steps = screen
        .data
        .prefs
        .ready()
        .map(|prefs| prefs.steps.clone())
        .unwrap_or_default();
    let current = step_of(level, &steps);

    let mut card = card("settings-discovery", cx)
        .child(card_title("Niveau de découverte", cx))
        .child(hint(
            "Jusqu'où le Flow et les suggestions « Pour vous » s'éloignent de ce que vous \
             écoutez. Le réglage vous suit sur tous vos appareils, et le titre suivant est \
             choisi au nouveau niveau : ce qui joue n'est jamais coupé.",
            cx,
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.75))
                .child(div().text_size(rems(0.875)).child(current.clone()))
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(rems(0.75))
                        .font_family(theme::mono(cx))
                        .text_color(theme::secondary(cx))
                        .child(format!("{} %", (level * 100.).round())),
                ),
        )
        .child(track_metre("settings-discovery-track", level, cx))
        .child(steps_row(level, &steps, cx));

    if let Some(load) = match &screen.data.prefs {
        Load::Failed(cause) => Some(cause.clone()),
        _ => None,
    } {
        card = card
            .child(failed("settings-discovery-failed", load, cx))
            .child(
                button(
                    "settings-discovery-retry",
                    "Réessayer",
                    ButtonVariant::Outline,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.load_prefs(cx))),
            );
    }
    card.into_any_element()
}

/// A line of twenty zones over a filled track: clicking one sets that level
/// and saves it. The fill is the level itself, so the control says where the
/// person stands at a glance.
fn track_metre(id: &'static str, level: f32, cx: &mut Context<Screen>) -> impl IntoElement + use<> {
    let zones = div()
        .absolute()
        .inset_0()
        .flex()
        .children((0..20).map(|index| {
            let target = (index as f32 + 0.5) / 20.;
            div()
                .id(("discovery-zone", index as usize))
                .flex_1()
                .h_full()
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.set_discovery(target, cx);
                    }),
                )
        }));
    div()
        .id(id)
        .relative()
        .h(rems(0.625))
        .w_full()
        .max_w(rems(30.))
        .bg(theme::border(cx))
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .w(rems(0.625))
                .bg(theme::accent(cx))
                .ml(gpui_kit::relative(level)),
        )
        .child(zones)
}

/// The four named steps. A click lands a little past the step's start
/// (`lib/discovery.js`: `s.from + 0.125`), so the name stays chosen instead of
/// sitting on the boundary.
fn steps_row(
    level: f32,
    steps: &[DiscoverStep],
    cx: &mut Context<Screen>,
) -> impl IntoElement + use<> {
    let names: Vec<(String, f32)> = if steps.is_empty() {
        STEPS
            .iter()
            .map(|(name, from)| ((*name).to_owned(), *from))
            .collect()
    } else {
        steps
            .iter()
            .map(|step| (step.name.clone(), step.from_level as f32))
            .collect()
    };
    let current = step_of(level, steps);
    let mut row = div().flex().items_center().gap(rems(0.25)).max_w(rems(30.));
    for (name, from) in names {
        let target = if from <= 0. { 0. } else { from + 0.125 };
        let selected = name == current;
        row = row.child(
            gpui_omarchy::toggle(format!("discovery-step-{name}"), name, selected, cx)
                .when(selected, |chip| chip.text_color(theme::accent(cx)))
                .on_click(cx.listener(move |this, _, _, cx| this.set_discovery(target, cx))),
        );
    }
    row
}

// --- the disliked titles ---------------------------------------------------

fn dislikes(screen: &mut Screen, cx: &mut Context<Screen>) -> AnyElement {
    const PAGE: usize = 12;
    let mut card = card("settings-dislikes", cx)
        .child(card_title("Titres que vous n'aimez pas", cx))
        .child(hint(
            "Ils ne reviennent plus dans le Flow, les radios et les mixes, ni les titres qui \
             sonnent presque pareil. Vous pouvez toujours les chercher et les écouter vous-même.",
            cx,
        ));
    match &screen.data.dislikes {
        Load::Idle | Load::Loading => card = card.child(loading(cx)),
        Load::Failed(cause) => {
            card = card
                .child(failed("settings-dislikes-failed", cause.clone(), cx))
                .child(
                    button(
                        "settings-dislikes-retry",
                        "Réessayer",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.reload_dislikes(cx))),
                );
        }
        Load::Ready(list) => {
            let tracks: &[Track] = &list.tracks;
            if tracks.is_empty() {
                card = card.child(hint(
                    "Aucun pour l'instant : « Je n'aime pas » dans le lecteur ou le menu d'un \
                     titre l'ajoute ici.",
                    cx,
                ));
            } else {
                card = card.child(value_line("Titres écartés", tracks.len().to_string(), cx));
                for track in tracks.iter().take(screen.shown.max(PAGE)) {
                    card = card.child(disliked_row(track, cx));
                }
                if tracks.len() > screen.shown.max(PAGE) {
                    card = card.child(
                        button(
                            "settings-dislikes-more",
                            format!("Afficher {} de plus", PAGE),
                            ButtonVariant::Outline,
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.shown += PAGE;
                            cx.notify();
                        })),
                    );
                }
            }
        }
    }
    card.into_any_element()
}

fn disliked_row(track: &Track, cx: &mut Context<Screen>) -> impl IntoElement + use<> {
    let reference = dislike_reference(track);
    let title = track.title.clone();
    let artist = track.display_artist.clone();
    div()
        .flex()
        .items_center()
        .gap(rems(0.75))
        .py(rems(0.25))
        .border_b_1()
        .border_color(theme::border(cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_size(rems(0.75)).child(title))
                .child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .child(artist.unwrap_or_default()),
                ),
        )
        .child(
            button(
                "dislike-restore",
                "Réintégrer",
                ButtonVariant::Secondary,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.restore_dislike(reference.clone(), cx);
            })),
        )
}

/// `trackkey.js#trackRef`: the row's own id when the library holds it, else
/// the catalogue's — the same spelling `/api/favorite` and `/api/dislike` take.
pub fn dislike_reference(track: &Track) -> String {
    match &track.id {
        Some(id) => id.as_str().to_owned(),
        None => track.deezer_id.as_str().to_owned(),
    }
}

// --- the releases check (admin) -------------------------------------------

fn releases(screen: &mut Screen, cx: &mut Context<Screen>) -> AnyElement {
    let mut card = card("settings-releases", cx)
        .child(card_title("Nouveautés", cx))
        .child(hint(
            "Le serveur vérifie les sorties de vos artistes favoris, des artistes que chacun \
             écoute le plus et de quelques artistes très proches. Rien n'est téléchargé : les \
             sorties apparaissent dans « Nouveautés ».",
            cx,
        ));
    match &screen.data.releases {
        Load::Idle | Load::Loading => card = card.child(loading(cx)),
        Load::Failed(cause) => {
            card = card
                .child(failed("settings-releases-failed", cause.clone(), cx))
                .child(
                    button(
                        "settings-releases-retry",
                        "Réessayer",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.load_releases(cx))),
                );
        }
        Load::Ready(settings) => {
            let settings = settings.clone();
            card = card
                .child(
                    switch(
                        "settings-releases-on",
                        "Vérifier les nouveautés automatiquement",
                        settings.on,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.save_releases("on", !settings.on, cx);
                    })),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(rems(0.75))
                        .child(stepper(
                            "every",
                            "Vérifier toutes les",
                            settings.every,
                            "h",
                            1,
                            168,
                            1,
                            cx,
                        ))
                        .child(stepper(
                            "listened",
                            "Plus écoutés suivis",
                            settings.listened,
                            "artistes",
                            0,
                            200,
                            5,
                            cx,
                        ))
                        .child(stepper(
                            "close",
                            "Artistes proches par favori",
                            settings.close,
                            "",
                            0,
                            5,
                            1,
                            cx,
                        ))
                        .child(stepper(
                            "window",
                            "Sorties des derniers",
                            settings.window,
                            "jours",
                            7,
                            365,
                            7,
                            cx,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .child(
                            button(
                                "settings-releases-check",
                                if settings.running {
                                    "Vérification…"
                                } else {
                                    "Vérifier maintenant"
                                },
                                ButtonVariant::Primary,
                                cx,
                            )
                            .disabled(settings.running)
                            .on_click(cx.listener(|this, _, _, cx| this.check_releases(cx))),
                        )
                        .child(hint(
                            format!(
                                "{} artistes suivis, dont {} favoris · {} sorties connues",
                                settings.watched, settings.favourites, settings.releases
                            ),
                            cx,
                        )),
                );
            if let Some(last) = &settings.last_run {
                card = card.child(value_line("Dernière vérification", readable_date(last), cx));
            }
            if let Some(error) = &settings.last_error {
                card = card.child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::warning(cx))
                        .child(error.clone()),
                );
            }
        }
    }
    card.into_any_element()
}

/// A number with a − and a + beside it: the whole control is buttons, so it
/// needs no text editing and cannot be typed into an invalid value.
#[allow(clippy::too_many_arguments)]
fn stepper(
    key: &'static str,
    label: &'static str,
    value: u32,
    unit: &'static str,
    min: u32,
    max: u32,
    step: u32,
    cx: &mut Context<Screen>,
) -> impl IntoElement + use<> {
    let down = value.saturating_sub(step).max(min);
    let up = (value + step).min(max);
    div()
        .flex()
        .flex_col()
        .gap(rems(0.25))
        .child(
            div()
                .text_size(rems(0.6875))
                .text_color(theme::secondary(cx))
                .child(label),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.25))
                .child(
                    button(
                        eid(format!("step-down-{key}")),
                        "−",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(value <= min)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.save_releases(key, down, cx);
                    })),
                )
                .child(
                    div()
                        .w(rems(4.))
                        .text_size(rems(0.75))
                        .font_family(theme::mono(cx))
                        .child(if unit.is_empty() {
                            value.to_string()
                        } else {
                            format!("{value} {unit}")
                        }),
                )
                .child(
                    button(
                        eid(format!("step-up-{key}")),
                        "+",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(value >= max)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.save_releases(key, up, cx);
                    })),
                ),
        )
}

impl Screen {
    /// Move the notch: the device keeps the value at once (the SPA does), the
    /// server is told once, and a refusal puts the old value back.
    pub fn set_discovery(&mut self, level: f32, cx: &mut Context<Self>) {
        let level = level.clamp(0., 1.);
        if (self.discover_level - level).abs() < f32::EPSILON {
            return;
        }
        let before = self.discover_level;
        self.discover_level = level;
        self.save(LEVEL_KEY, &f64::from(level), cx);
        if let Some(api) = self.api(cx) {
            let reply = self.reply();
            let runtime = self.runtime(cx);
            runtime.spawn(async move {
                match api.set_discover_level(f64::from(level)).await {
                    Ok(_) => {
                        let prefs = api.discover_prefs().await.map_err(cause);
                        reply.send(super::Reply::Prefs(prefs));
                    }
                    // A refusal is a cause the person must see, and the notch
                    // goes back to what the server actually holds.
                    Err(error) => reply.send(super::Reply::DiscoveryFailed {
                        previous: before,
                        cause: error.cause_fr().to_owned(),
                    }),
                }
            });
        }
        cx.notify();
    }

    fn reload_dislikes(&mut self, cx: &mut Context<Self>) {
        if let Some(api) = self.api(cx) {
            let reply = self.reply();
            let runtime = self.runtime(cx);
            runtime.spawn(async move {
                let answer = api.my_dislikes().await.map_err(cause);
                reply.send(super::Reply::Dislikes(answer));
            });
        }
    }

    /// Put a title back in the Flow: optimistic (the row leaves at once) and
    /// rolled back with a cause when the server refuses.
    fn restore_dislike(&mut self, reference: String, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let removed = match &mut self.data.dislikes {
            Load::Ready(list) => {
                let before = list.clone();
                list.tracks
                    .retain(|track| dislike_reference(track) != reference);
                Some(before)
            }
            _ => None,
        };
        let reply = self.reply();
        let runtime = self.runtime(cx);
        let restored = reference.clone();
        runtime.spawn(async move {
            match api.dislike(&reference, false).await {
                Ok(()) => reply.send(super::Reply::Saved("Titre réintégré")),
                Err(error) => {
                    let _ = restored;
                    reply.send(super::Reply::Failed(error.cause_fr().to_owned()))
                }
            }
        });
        if removed.is_none() {
            self.fail(
                "La liste des titres écartés n'a pas pu être mise à jour.",
                cx,
            );
        }
        cx.notify();
    }

    /// Save one releases field, then re-read the settings so the counts and
    /// the last-run line stay the server's.
    pub fn save_releases(
        &mut self,
        key: &'static str,
        value: impl serde::Serialize + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let body = serde_json::json!({ key: value });
        let reply = self.reply();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            match api.save_releases_settings(&body).await {
                Ok(_) => {
                    let settings = api.releases_settings().await.map_err(cause);
                    reply.send(super::Reply::Releases(settings));
                }
                Err(error) => reply.send(super::Reply::Failed(format!(
                    "Réglage refusé : {}",
                    error.cause_fr()
                ))),
            }
        });
        cx.notify();
    }

    fn check_releases(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let reply = self.reply();
        let runtime = self.runtime(cx);
        runtime.spawn(async move {
            match api.releases_check().await {
                Ok(()) => reply.send(super::Reply::Saved(
                    "Vérification lancée : les sorties arriveront dans « Nouveautés »",
                )),
                Err(error) => reply.send(super::Reply::Failed(format!(
                    "Vérification impossible : {}",
                    error.cause_fr()
                ))),
            }
        });
        cx.notify();
    }
}

fn cause(error: nz_api::ApiError) -> String {
    error.cause_fr().to_owned()
}

/// The server sends an ISO date-time; a person reads `16/11/2026 14:05`. No
/// date crate is pinned, so the fields are cut out of the string itself —
/// `2026-11-16T14:05:00` is the only shape the server writes.
pub fn readable_date(raw: &str) -> String {
    let date = raw.get(0..10).unwrap_or(raw);
    let time = raw.get(11..16).unwrap_or("");
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() == 3 {
        format!("{}/{}/{} {time}", parts[2], parts[1], parts[0])
            .trim_end()
            .to_owned()
    } else {
        raw.to_owned()
    }
}

/// The count of a list, for the line beside the card's title.
pub fn count(list: &TrackList) -> usize {
    list.tracks.len()
}

/// The settings a `Loaded` releases card draws, for a test that does not need
/// the network.
pub fn releases_lines(settings: &ReleasesSettings) -> [String; 3] {
    [
        format!("{} h", settings.every),
        format!("{} jours", settings.window),
        format!("{} artistes suivis", settings.watched),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notch_falls_in_the_named_step_a_person_sees() {
        let steps = [
            DiscoverStep {
                name: "Proche".into(),
                from_level: 0.,
            },
            DiscoverStep {
                name: "Voisin".into(),
                from_level: 0.25,
            },
            DiscoverStep {
                name: "Exploration".into(),
                from_level: 0.5,
            },
            DiscoverStep {
                name: "Aventure".into(),
                from_level: 0.75,
            },
        ];
        assert_eq!(step_of(0., &steps), "Proche");
        assert_eq!(step_of(0.24, &steps), "Proche");
        assert_eq!(step_of(0.25, &steps), "Voisin");
        assert_eq!(step_of(0.51, &steps), "Exploration");
        assert_eq!(step_of(0.75, &steps), "Aventure");
        assert_eq!(step_of(1., &steps), "Aventure");
        // A stale value outside 0…1 still names a step, and the local table is
        // the page's own four names.
        assert_eq!(step_of(-2., &steps), "Proche");
        assert_eq!(step_of(9., &steps), "Aventure");
        assert_eq!(step_of(0.6, &[]), "Exploration");
        assert_eq!(step_of(0.1, &[]), "Proche");
    }

    #[test]
    fn a_step_click_lands_just_inside_its_name() {
        // `lib/discovery.js`: `s.from + 0.125`, except the first step (0).
        let steps = [
            DiscoverStep {
                name: "Proche".into(),
                from_level: 0.,
            },
            DiscoverStep {
                name: "Voisin".into(),
                from_level: 0.25,
            },
        ];
        let target = 0.25 + 0.125;
        assert_eq!(step_of(target, &steps), "Voisin");
        // The name a click on the zone between two steps lands in: never on the
        // boundary, where rounding could flip it.
        assert_eq!(step_of(0.125, &steps), "Proche");
    }

    #[test]
    fn the_dislike_reference_prefers_the_library_row() {
        let library: Track = serde_json::from_str(
            r#"{"id":"3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f","deezer_id":"3000001",
                "title":"T","duration":100,"artist":{"name":"A"},"album":{"title":"B"}}"#,
        )
        .expect("library row");
        assert_eq!(
            dislike_reference(&library),
            "3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f"
        );
        let catalogue: Track = serde_json::from_str(
            r#"{"deezer_id":"3000001","title":"T","duration":100,
                "artist":{"name":"A"},"album":{"title":"B"}}"#,
        )
        .expect("catalogue row");
        assert_eq!(dislike_reference(&catalogue), "3000001");
    }

    #[test]
    fn the_last_run_reads_as_a_french_date_and_time() {
        assert_eq!(readable_date("2026-11-16T14:05:00"), "16/11/2026 14:05");
        assert_eq!(readable_date("2026-11-16"), "16/11/2026");
        assert_eq!(readable_date("hier"), "hier");
    }

    #[test]
    fn the_default_notch_is_the_pages_own() {
        assert_eq!(DEFAULT_LEVEL, 0.35);
        assert!(DEFAULT_LEVEL >= STEPS[1].1 && DEFAULT_LEVEL < STEPS[2].1);
        assert_eq!(step_of(DEFAULT_LEVEL, &[]), "Voisin");
    }
}
