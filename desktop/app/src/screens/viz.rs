//! Le visualiseur : les barres et l'oscilloscope, plein cadre.
//!
//! Ce que le SPA met dans son lecteur plein écran (`DesktopNowPlaying.svelte`,
//! `Visualizer.svelte` en disposition `full`) et dans sa page d'animation, pour
//! les deux scènes que le bureau dessine : la scène en fond, la carte du titre
//! en bas à gauche, la barre de contrôle en haut à droite, et la feuille de
//! réglages — les mêmes contrôles que Réglages → Animations, une seule fois
//! écrits (`screens::settings::viz::controls`).
//!
//! RIEN NE PREND LE FOCUS. L'écran ne réclame jamais le clavier : ses verbes
//! sont des actions de l'application, enregistrées une fois et portées par un
//! `WeakEntity` (D8b), donc `Espace`, `q`, `g h` et `v` continuent de marcher
//! ici comme partout. Les touches propres au visualiseur sont installées par
//! [`install`], une seule fois par processus, et annoncées en bas de la carte
//! (« 1 Barres · 2 Oscilloscope · c Réglages · Maj+F Plein écran »), puisque
//! l'overlay `?` est figé par D7.
//!
//! LA BARRE S'EFFACE. Comme sur un projecteur, la carte et les contrôles
//! disparaissent après trois secondes sans geste et reviennent au premier
//! mouvement : l'écran doit être l'animation, pas ses boutons.
//!
//! L'ANALYSE S'ARRÊTE AVEC L'ÉCRAN. `AppState::viz` est un service unique ; cet
//! écran le dit visible tant qu'il peint, et le rend au moment où il est
//! détruit. C'est ce qui rend mesurable « zéro appel d'analyseur quand aucun
//! visualiseur n'est visible ».

use std::time::{Duration, Instant};

use gpui_kit::StyledImage as _;
use gpui_kit::base::slider::{SliderEvent, SliderState, SliderValue};
use gpui_kit::{
    App, AppContext as _, Context, Entity, Global, InteractiveElement as _, IntoElement,
    KeyBinding, ParentElement, Render, StatefulInteractiveElement as _, Styled, Subscription, Task,
    WeakEntity, Window, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, icon, icon_button, toggle, toggle_group};

use crate::nav::Route;
use crate::screens::settings::viz::{Host, Mode, Prefs, controls};
use crate::state::AppState;
use crate::theme;
use crate::widgets::bars::Bars;
use crate::widgets::scope::Scope;

gpui_kit::actions!(viz, [ShowBars, ShowScope, ToggleVizSettings, VizFullscreen]);

/// Combien de temps la carte et les contrôles restent après le dernier geste.
const CHROME_MS: u64 = 3_000;
/// La largeur de la feuille de réglages : celle du panneau latéral du plan.
const SHEET: gpui_kit::Pixels = px(420.);

/// L'écran vivant, pour les actions globales. Un seul écran de visualisation
/// existe à la fois (une route à la fois) ; les handlers sont donc installés
/// une fois et lisent cette case, au lieu qu'un handler s'accumule par visite.
/// Le jeton dit à une libération TARDIVE de ne pas effacer l'écran qui l'a
/// remplacé.
#[derive(Default)]
struct Active {
    token: u64,
    screen: Option<WeakEntity<Screen>>,
}

impl Global for Active {}

/// Marque l'installation des touches : une fois par processus.
struct Installed;

impl Global for Installed {}

/// Installe les touches du visualiseur, une seule fois, dans le contexte du
/// shell (`Neezer && !Input`) : elles ne tirent jamais dans un champ de texte,
/// et une action sans écran vivant ne fait rien.
pub fn install(cx: &mut App) {
    if cx.has_global::<Installed>() {
        return;
    }
    cx.set_global(Installed);
    let context = Some(crate::keymap::CONTEXT);
    cx.bind_keys([
        KeyBinding::new("1", ShowBars, context),
        KeyBinding::new("2", ShowScope, context),
        KeyBinding::new("c", ToggleVizSettings, context),
        KeyBinding::new("shift-f", VizFullscreen, context),
    ]);
    cx.on_action(|_: &ShowBars, cx: &mut App| {
        with_active(cx, |screen, cx| screen.set_mode(Mode::Bars, cx));
    });
    cx.on_action(|_: &ShowScope, cx: &mut App| {
        with_active(cx, |screen, cx| screen.set_mode(Mode::Scope, cx));
    });
    cx.on_action(|_: &ToggleVizSettings, cx: &mut App| {
        with_active(cx, |screen, cx| screen.toggle_sheet(cx));
    });
    cx.on_action(|_: &VizFullscreen, cx: &mut App| {
        if let Some(window) = cx.windows().first().copied() {
            let _ = window.update(cx, |_, window, _| window.toggle_fullscreen());
        }
    });
}

fn with_active(cx: &mut App, action: impl FnOnce(&mut Screen, &mut Context<Screen>)) {
    let Some(slot) = cx.try_global::<Active>() else {
        return;
    };
    let Some(screen) = slot.screen.clone() else {
        return;
    };
    let _ = screen.update(cx, |screen, cx| action(screen, cx));
}

/// Le numéro de cet écran, pour que sa libération ne referme pas celui d'après.
fn next_token() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// La part de l'écran qui se met en veilleuse, et celle qui reste.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Chrome {
    #[default]
    Visible,
    Hidden,
}

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    prefs: Prefs,
    intensity: Entity<SliderState>,
    bars: Entity<Bars>,
    scope: Entity<Scope>,
    sheet: bool,
    chrome: Chrome,
    idle: Option<Task<()>>,
    /// L'instant où le pointeur a bougé pour la dernière fois (le masquage ne
    /// reprogramme pas une tâche par mouvement).
    shown_at: Instant,
    /// Le numéro de cet écran dans [`Active`].
    token: u64,
    _subscriptions: Vec<Subscription>,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        install(cx);
        let prefs = Prefs::load(state.read(cx).cache.as_ref());
        let intensity = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(1.)
                .step(0.05)
                .default_value(prefs.intensity)
        });
        let bars = cx.new(|cx| Bars::new(state.clone(), prefs, false, cx));
        let scope = cx.new(|cx| Scope::new(state.clone(), prefs, false, cx));
        bars.update(cx, |bars, cx| bars.set_active(prefs.mode == Mode::Bars, cx));
        scope.update(cx, |scope, cx| {
            scope.set_active(prefs.mode == Mode::Scope, cx)
        });
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        let slider = cx.subscribe(&intensity, |this: &mut Self, _, event: &SliderEvent, cx| {
            let value = match event {
                SliderEvent::Change(value) | SliderEvent::Release(value) => match value {
                    SliderValue::Single(value) => *value,
                    SliderValue::Range(start, _) => *start,
                },
            };
            this.prefs.intensity = value.clamp(0., 1.);
            this.viz_changed(cx);
        });
        let token = next_token();
        let release = cx.on_release(move |this: &mut Self, cx| {
            this.state.update(cx, |state, _| {
                if let Some(viz) = &state.viz {
                    viz.set_visible(false);
                }
            });
            if cx
                .try_global::<Active>()
                .is_some_and(|slot| slot.token == token)
            {
                cx.set_global(Active::default());
            }
        });
        let mut this = Self {
            route,
            state,
            prefs,
            intensity,
            bars,
            scope,
            sheet: false,
            chrome: Chrome::Visible,
            idle: None,
            shown_at: Instant::now(),
            token,
            _subscriptions: vec![subscription, slider, release],
        };
        let mine = cx.weak_entity();
        cx.set_global(Active {
            token,
            screen: Some(mine),
        });
        this.schedule_idle(cx);
        this
    }

    pub fn route(&self) -> &Route {
        &self.route
    }

    /// La scène affichée, pour un test et pour le `?`.
    pub fn mode(&self) -> Mode {
        self.prefs.mode
    }

    pub fn prefs(&self) -> Prefs {
        self.prefs
    }

    pub fn sheet_open(&self) -> bool {
        self.sheet
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.prefs.mode == mode {
            return;
        }
        self.prefs.mode = mode;
        self.bars
            .update(cx, |bars, cx| bars.set_active(mode == Mode::Bars, cx));
        self.scope
            .update(cx, |scope, cx| scope.set_active(mode == Mode::Scope, cx));
        self.viz_changed(cx);
    }

    fn toggle_sheet(&mut self, cx: &mut Context<Self>) {
        self.sheet = !self.sheet;
        self.show_chrome(cx);
        cx.notify();
    }

    fn show_chrome(&mut self, cx: &mut Context<Self>) {
        self.shown_at = Instant::now();
        self.chrome = Chrome::Visible;
        self.schedule_idle(cx);
        cx.notify();
    }

    /// Trois secondes, puis la veilleuse — sauf feuille ouverte : on ne cache
    /// pas les contrôles qu'on est en train d'utiliser.
    fn schedule_idle(&mut self, cx: &mut Context<Self>) {
        self.idle = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(CHROME_MS))
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.sheet && this.chrome == Chrome::Visible {
                    this.chrome = Chrome::Hidden;
                    cx.notify();
                }
            });
        }));
    }
}

impl Host for Screen {
    fn viz_prefs(&self) -> &Prefs {
        &self.prefs
    }

    fn viz_prefs_mut(&mut self) -> &mut Prefs {
        &mut self.prefs
    }

    fn viz_changed(&mut self, cx: &mut Context<Self>) {
        let (prefs, cache) = (self.prefs, self.state.read(cx).cache.clone());
        prefs.save(cache.as_ref());
        self.bars.update(cx, |bars, cx| bars.set_prefs(prefs, cx));
        self.scope
            .update(cx, |scope, cx| scope.set_prefs(prefs, cx));
        cx.notify();
    }
}

/// Ce que l'écran dit quand il n'y a rien à voir : plein cadre pour ce qui
/// manque (aucun titre, analyse indisponible), discret en bas pour une pause.
enum Notice {
    Full(&'static str, &'static str),
    Quiet(&'static str, &'static str),
}

/// La carte du titre, en bas à gauche : la pochette, le titre, l'artiste, et
/// les touches du visualiseur.
fn now_card(
    state: &Entity<AppState>,
    prefs: Prefs,
    accent: gpui_kit::Hsla,
    cx: &mut Context<Screen>,
) -> impl IntoElement + use<> {
    let (entry, playing) = {
        let state = state.read(cx);
        (state.player.current().cloned(), state.player.playing())
    };
    let mut card = div()
        .id("viz-card")
        .debug_selector(|| "viz-card".to_owned())
        .flex()
        .flex_col()
        .gap(rems(0.5))
        .p(rems(1.))
        .min_w(rems(16.))
        .max_w(rems(28.))
        .border_1()
        .border_color(theme::border(cx))
        .bg(theme::chrome(cx).opacity(0.9))
        .rounded_lg();

    if let Some(entry) = &entry {
        let cover = state.read(cx).cover_path(entry);
        let mut line = div().flex().items_center().gap(rems(0.75));
        if let Some(path) = cover {
            line = line.child(
                gpui_kit::img(path)
                    .w(rems(3.))
                    .h(rems(3.))
                    .rounded_md()
                    .object_fit(gpui_kit::ObjectFit::Cover),
            );
        } else {
            line = line.child(
                div()
                    .w(rems(3.))
                    .h(rems(3.))
                    .rounded_md()
                    .border_1()
                    .border_color(theme::border(cx))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(IconName::Disc3).size(rems(1.25)).text_color(accent)),
            );
        }
        let album = entry
            .album
            .as_ref()
            .map(|album| album.title.clone())
            .unwrap_or_default();
        let mut words = div()
            .flex()
            .flex_col()
            .gap(rems(0.125))
            .min_w_0()
            .child(
                div()
                    .truncate()
                    .text_size(rems(1.))
                    .font_weight(gpui_kit::FontWeight::BOLD)
                    .text_color(theme::text(cx))
                    .child(entry.title.clone()),
            )
            .child(
                div()
                    .truncate()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(if entry.artist_name().is_empty() {
                        "—".to_owned()
                    } else {
                        entry.artist_name().to_owned()
                    }),
            );
        if !album.is_empty() {
            words = words.child(
                div()
                    .truncate()
                    .text_size(rems(0.6875))
                    .font_family(theme::mono(cx))
                    .text_color(theme::secondary(cx))
                    .child(album),
            );
        }
        card = card.child(line.child(words));
    } else {
        card = card
            .child(
                div()
                    .text_size(rems(0.875))
                    .font_weight(gpui_kit::FontWeight::BOLD)
                    .child("Rien en lecture"),
            )
            .child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child("Lancez un titre : l'animation suit ce que vous entendez."),
            );
    }

    let scene = match prefs.mode {
        Mode::Bars => "Barres",
        Mode::Scope => "Oscilloscope",
    };
    let rate = if prefs.eco {
        "éco · 30 images/s".to_owned()
    } else {
        format!("{} images/s", prefs.effective_fps().effective())
    };
    card.child(
        div()
            .flex()
            .items_center()
            .gap(rems(0.5))
            .text_size(rems(0.6875))
            .font_family(theme::mono(cx))
            .text_color(theme::secondary(cx))
            .child(format!(
                "{scene} · {rate}{}",
                if !playing && entry.is_some() {
                    " · en pause"
                } else {
                    ""
                }
            )),
    )
    .child(
        div()
            .text_size(rems(0.625))
            .text_color(theme::secondary(cx).opacity(0.75))
            .child("1 Barres · 2 Oscilloscope · c Réglages · Maj+F Plein écran"),
    )
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // L'analyse tourne tant que cet écran peint.
        self.state.update(cx, |state, _| {
            if let Some(viz) = &state.viz {
                viz.set_visible(true);
            }
        });
        let playing = self.state.read(cx).player.playing();
        let has_track = self.state.read(cx).player.current().is_some();
        let has_analysis = self.state.read(cx).viz.is_some();
        let prefs = self.prefs;
        // La carte prend l'accent du thème : la scène, elle, a sa palette, et
        // décoder la pochette deux fois pour colorer deux carrés serait un
        // décodage de trop par titre.
        let accent = theme::accent(cx);
        let scene = match prefs.mode {
            Mode::Bars => self.bars.clone().into_any_element(),
            Mode::Scope => self.scope.clone().into_any_element(),
        };
        let card = now_card(&self.state, prefs, accent, cx);

        // La barre de contrôle : la scène, la feuille, le plein écran.
        let mut bar = toggle_group("viz-scene", cx);
        for mode in Mode::ALL {
            bar = bar.child(
                toggle(
                    format!("viz-scene-{}", mode.id()),
                    mode.label(),
                    prefs.mode == mode,
                    cx,
                )
                .debug_selector(move || format!("viz-scene-{}", mode.id()))
                .on_click(cx.listener(move |this, _, _, cx| this.set_mode(mode, cx))),
            );
        }
        let chrome = div()
            .id("viz-bar")
            .debug_selector(|| "viz-bar".to_owned())
            .absolute()
            .top(rems(1.))
            .right(rems(1.))
            .flex()
            .items_center()
            .gap(rems(0.5))
            .p(rems(0.5))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx).opacity(0.9))
            .rounded_lg()
            .child(bar)
            .child(
                div()
                    .text_size(rems(0.6875))
                    .font_family(theme::mono(cx))
                    .text_color(theme::secondary(cx))
                    .child(if prefs.eco {
                        "éco".to_owned()
                    } else {
                        format!("{} fps", prefs.effective_fps().effective())
                    }),
            )
            .child(
                icon_button(
                    "viz-settings",
                    IconName::SlidersHorizontal,
                    "Réglages de l'animation",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_sheet(cx))),
            )
            .child(
                icon_button(
                    "viz-fullscreen",
                    if window.is_fullscreen() {
                        IconName::Minimize2
                    } else {
                        IconName::Maximize2
                    },
                    "Plein écran",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    window.toggle_fullscreen();
                    this.show_chrome(cx);
                })),
            );

        // La feuille de réglages : les mêmes contrôles que Réglages → Animations.
        let sheet = self.sheet.then(|| {
            let controls = controls(&self.prefs, &self.intensity, window, cx);
            div()
                .id("viz-sheet")
                .debug_selector(|| "viz-sheet".to_owned())
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .w(SHEET)
                .flex()
                .flex_col()
                .gap(rems(0.75))
                .p(rems(1.25))
                .border_l_1()
                .border_color(theme::border(cx))
                .bg(theme::chrome(cx))
                .overflow_y_scroll()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .child(
                            div()
                                .text_size(rems(0.9375))
                                .font_weight(gpui_kit::FontWeight::BOLD)
                                .child("Animation"),
                        )
                        .child(div().flex_1())
                        .child(
                            icon_button(
                                "viz-sheet-close",
                                IconName::Close,
                                "Fermer les réglages",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_sheet(cx))),
                        ),
                )
                .child(controls)
        });

        // Ce que l'écran dit quand il n'y a rien à voir (règle 3) : la cause,
        // pas un spinner.
        let notice = if !has_track {
            Some(Notice::Full(
                "Rien en lecture",
                "Lancez un titre : la scène suit ce que vous entendez, pas une horloge.",
            ))
        } else if !has_analysis {
            Some(Notice::Full(
                "Analyse indisponible",
                "La sortie audio n'est pas ouverte : aucun échantillon n'arrive à l'analyseur.",
            ))
        } else if !playing {
            Some(Notice::Quiet(
                "En pause",
                "L'animation reprend dès que la lecture repart.",
            ))
        } else {
            None
        };

        let mut stage = div()
            .id("viz-stage")
            .debug_selector(|| "viz-stage".to_owned())
            .relative()
            .size_full()
            .bg(theme::shell(cx))
            .font_family(theme::font(cx))
            .text_color(theme::text(cx))
            .on_mouse_move(cx.listener(|this, _, _, cx| {
                if this.chrome == Chrome::Hidden {
                    this.show_chrome(cx);
                }
            }))
            .child(div().absolute().inset_0().p(rems(0.5)).child(scene));

        match notice {
            Some(Notice::Full(title, cause)) => {
                stage = stage.child(
                    div()
                        .id("viz-notice")
                        .debug_selector(|| "viz-notice".to_owned())
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(rems(0.5))
                        .child(
                            div()
                                .text_size(rems(1.5))
                                .font_weight(gpui_kit::FontWeight::BOLD)
                                .child(title),
                        )
                        .child(
                            div()
                                .max_w(rems(38.))
                                .text_center()
                                .text_size(rems(0.8125))
                                .text_color(theme::secondary(cx))
                                .child(cause),
                        ),
                );
            }
            Some(Notice::Quiet(title, cause)) => {
                stage = stage.child(
                    div()
                        .id("viz-notice")
                        .debug_selector(|| "viz-notice".to_owned())
                        .absolute()
                        .bottom(rems(1.5))
                        .left_0()
                        .right_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(rems(0.125))
                        .child(
                            div()
                                .text_size(rems(0.8125))
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(theme::text(cx).opacity(0.75))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::secondary(cx).opacity(0.8))
                                .child(cause),
                        ),
                );
            }
            None => {}
        }

        // La carte et la barre : visibles au premier geste, effacées après
        // trois secondes, jamais pendant qu'on règle.
        if self.chrome == Chrome::Visible || self.sheet {
            stage = stage.child(div().absolute().bottom(rems(1.)).left(rems(1.)).child(card));
            stage = stage.child(chrome);
        }
        if let Some(sheet) = sheet {
            stage = stage.child(sheet);
        }
        stage
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use gpui_kit::{IntoElement, ParentElement, Styled, div};

    use super::*;
    use crate::screens::settings::viz::keys;
    use crate::state::{AppConfig, event_channel};
    use crate::wiring::viz::VizService;

    /// Un hôte pour que la fenêtre ait quelque chose à peindre. Le champ est
    /// optionnel pour qu'un test puisse lâcher l'écran comme le fait un
    /// changement de route dans la coquille.
    struct Host {
        screen: Option<Entity<Screen>>,
    }

    impl Render for Host {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().children(self.screen.clone())
        }
    }

    /// Aucune source : ces tests ne jouent rien, ils veulent un moteur qui a un
    /// tap (le silence du mélangeur suffit à alimenter l'analyseur).
    struct Silent;

    impl nz_audio::StreamOpener for Silent {
        fn open(
            &self,
            _id: &str,
            _quality: nz_audio::Quality,
            _start: Option<f64>,
            _from_byte: Option<u64>,
            _lane: nz_audio::Lane,
        ) -> Result<Box<dyn nz_audio::ByteStream>, nz_audio::SourceError> {
            Err(nz_audio::SourceError::Message("aucune source".into()))
        }

        fn ready(
            &self,
            _id: &str,
            _quality: nz_audio::Quality,
        ) -> Result<nz_audio::StreamReady, nz_audio::SourceError> {
            Err(nz_audio::SourceError::Message("aucune source".into()))
        }

        fn local(&self, _id: &str, _quality: nz_audio::Quality) -> Option<std::path::PathBuf> {
            None
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neezer-d10b-screen-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn app_state(cx: &mut gpui::TestAppContext, dir: &std::path::Path) -> Entity<AppState> {
        cx.update(|cx| {
            crate::theme::init(cx);
            crate::keymap::install(cx);
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let handle = runtime.handle().clone();
        std::mem::forget(runtime);
        let (tx, _rx) = event_channel();
        let dir = dir.to_path_buf();
        cx.update(move |cx| {
            cx.new(|_| {
                AppState::new(
                    AppConfig {
                        server: Some("http://127.0.0.1:1".to_owned()),
                        start_audio: false,
                        start_mpris: false,
                        ..AppConfig::for_test(&dir)
                    },
                    handle,
                    tx,
                    None,
                )
            })
        })
    }

    #[gpui::test]
    async fn the_scene_is_chosen_by_key_and_remembered(cx: &mut gpui::TestAppContext) {
        let dir = temp_dir("scene");
        let state = app_state(cx, &dir);
        let screen = cx.update(|cx| cx.new(|cx| Screen::new(Route::Viz, state.clone(), cx)));
        cx.update(|cx| assert_eq!(screen.read(cx).mode(), Mode::Bars));
        cx.update(|cx| cx.dispatch_action(&ShowScope));
        cx.update(|cx| assert_eq!(screen.read(cx).mode(), Mode::Scope));
        // La clé de la page, pas une clé à nous : « scope ».
        let stored = cx.update(|cx| {
            state
                .read(cx)
                .cache
                .as_ref()
                .and_then(|cache| cache.kv().get::<String>(keys::MODE))
        });
        assert_eq!(stored.as_deref(), Some("scope"));
        cx.update(|cx| cx.dispatch_action(&ShowBars));
        cx.update(|cx| assert_eq!(screen.read(cx).mode(), Mode::Bars));
        std::fs::remove_dir_all(dir).ok();
    }

    #[gpui::test]
    async fn the_settings_sheet_opens_and_closes_from_the_keyboard(cx: &mut gpui::TestAppContext) {
        let dir = temp_dir("sheet");
        let state = app_state(cx, &dir);
        let screen = cx.update(|cx| cx.new(|cx| Screen::new(Route::Viz, state.clone(), cx)));
        cx.update(|cx| assert!(!screen.read(cx).sheet_open()));
        cx.update(|cx| cx.dispatch_action(&ToggleVizSettings));
        cx.update(|cx| assert!(screen.read(cx).sheet_open()));
        cx.update(|cx| cx.dispatch_action(&ToggleVizSettings));
        cx.update(|cx| assert!(!screen.read(cx).sheet_open()));
        std::fs::remove_dir_all(dir).ok();
    }

    /// **Sans visualiseur à l'écran, l'analyseur ne fait rien.**
    ///
    /// Le critère du lot, mesuré sur le vrai service : l'écran dit « visible »
    /// tant qu'il peint, et le rend au moment où il est détruit ; le fil
    /// `nz-analysis` vide le tap et dort, donc `frames()` ne bouge plus même
    /// quand le moteur rend du silence.
    #[gpui::test]
    async fn the_analyser_does_no_work_when_no_visualiser_is_on_screen(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = temp_dir("analysis");
        let state = app_state(cx, &dir);
        let config = nz_audio::EngineConfig {
            sample_rate: 48_000,
            spool_dir: dir.join("spool"),
            ..nz_audio::EngineConfig::default()
        };
        let (engine, mut sink) = nz_audio::Engine::new_offline(config, Arc::new(Silent));
        let viz = VizService::start(&engine, 48_000.).expect("le moteur hors ligne a un tap");
        // Le mélangeur ne pousse dans le tap que s'il n'est pas en pause.
        engine.play();
        viz.set_visible(false);
        cx.update(|cx| state.update(cx, |state, _| state.viz = Some(viz)));

        let state_for_view = state.clone();
        let (host, vcx) = cx.add_window_view(move |_window, cx| Host {
            screen: Some(cx.new(|cx| Screen::new(Route::Viz, state_for_view, cx))),
        });
        vcx.run_until_parked();
        // Peindre l'écran allume l'analyse.
        let visible = vcx.update(|_, cx| state.read(cx).viz.as_ref().unwrap().is_visible());
        assert!(visible, "un écran de visualisation rend l'analyse visible");

        // On quitte l'écran, comme le fait la coquille quand la route change :
        // plus rien ne doit être analysé.
        vcx.update(|_, cx| {
            host.update(cx, |host, cx| {
                host.screen = None;
                cx.notify();
            })
        });
        vcx.run_until_parked();
        let gone = vcx.update(|_, cx| state.read(cx).viz.as_ref().unwrap().is_visible());
        assert!(!gone, "l'écran détruit rend l'analyse invisible");
        let before = vcx.update(|_, cx| state.read(cx).viz.as_ref().unwrap().frames());
        let mut buffer = vec![0.0f32; 960];
        for _ in 0..40 {
            sink.render(&mut buffer);
        }
        std::thread::sleep(Duration::from_millis(120));
        let after = vcx.update(|_, cx| state.read(cx).viz.as_ref().unwrap().frames());
        assert_eq!(
            after, before,
            "0 échantillon analysé quand rien n'est visible"
        );

        // Et elle repart dès qu'un écran la redemande.
        vcx.update(|_, cx| state.read(cx).viz.as_ref().unwrap().set_visible(true));
        let deadline = std::time::Instant::now() + crate::testwait::WAIT;
        loop {
            for _ in 0..4 {
                sink.render(&mut buffer);
            }
            let frames = vcx.update(|_, cx| state.read(cx).viz.as_ref().unwrap().frames());
            if frames > 0 {
                println!("analyse : {frames} blocs après 40 tampons");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "l'analyse n'a jamais repris"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// **L'écran se peint à 1 280 × 800, à 2 560 × 1 600 et à 660 × 800** — la
    /// même passe que D9 et D13 : le cadre remplit la fenêtre et la feuille de
    /// réglages fait 420 px, la largeur du panneau du plan.
    #[gpui::test]
    async fn the_stage_fills_the_window_at_every_size(cx: &mut gpui::TestAppContext) {
        let dir = temp_dir("sizes");
        let state = app_state(cx, &dir);
        let (host, vcx) = cx.add_window_view(move |_window, cx| Host {
            screen: Some(cx.new(|cx| Screen::new(Route::Viz, state.clone(), cx))),
        });
        let handle = vcx.windows()[0];
        for (width, height) in [(1280., 800.), (2560., 1600.), (660., 800.)] {
            vcx.simulate_window_resize(
                handle,
                gpui_kit::size(gpui_kit::px(width), gpui_kit::px(height)),
            );
            vcx.run_until_parked();
            let viewport = vcx.update(|window, _| window.viewport_size());
            assert_eq!(
                f32::from(viewport.width),
                width,
                "la fenêtre fait {width} px"
            );
            let stage = vcx
                .debug_bounds("viz-stage")
                .unwrap_or_else(|| panic!("pas de scène à {width} × {height}"));
            assert!(
                (f32::from(stage.size.width) - width).abs() < 1.,
                "la scène fait {} px dans {width}",
                stage.size.width
            );
            assert!((f32::from(stage.size.height) - height).abs() < 1.);
            // La carte du titre est dessinée, à sa taille de lecture.
            let card = vcx.debug_bounds("viz-card").expect("la carte du titre");
            assert!(
                f32::from(card.size.width) > 200. && f32::from(card.size.height) > 60.,
                "la carte fait {} × {}",
                card.size.width,
                card.size.height
            );
        }
        // La feuille ouverte : 420 px, la largeur du panneau du plan.
        vcx.update(|_, cx| cx.dispatch_action(&ToggleVizSettings));
        vcx.run_until_parked();
        let sheet = vcx
            .debug_bounds("viz-sheet")
            .expect("la feuille est ouverte");
        assert!(
            (f32::from(sheet.size.width) - f32::from(SHEET)).abs() < 1.,
            "la feuille fait {} px au lieu de 420",
            sheet.size.width
        );
        let _ = host;
        std::fs::remove_dir_all(dir).ok();
    }
}
