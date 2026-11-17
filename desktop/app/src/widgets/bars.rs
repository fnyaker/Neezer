//! Les barres : le spectre du titre en capsules de lumière.
//!
//! LA GÉOMÉTRIE N'EST PAS ICI. `rhythm::viz_bars` (par `nz-viz`) calcule le
//! nivellement par trois zones, le gamma, l'enveloppe attaque/relâche et les
//! crêtes qui tombent : c'est l'arithmétique que la page exécute, et elle est
//! déjà prouvée image pour image contre le `rhythm.wasm` livré (V1). Ce module
//! est le DESSIN : une capsule par bande, la couleur de la zone de pochette que
//! la bande occupe, et rien d'autre.
//!
//! CE QU'IL AJOUTE À LA PAGE, ET POURQUOI. Le SPA peint sur un `<canvas>` 2D à
//! la fréquence de l'écran ; ici chaque capsule est un quadrilatère GPU
//! (`Window::paint_quad`), donc une image coûte 64 à 190 appels et aucun chemin
//! tessellé. Le coût par image et la fréquence réelle sont mesurés dans le PR
//! et dans `docs/agents/desktop.md` (D10b).
//!
//! LE PLAFOND D'IMAGES. À « Écran » (le défaut), chaque image en demande une
//! autre (`request_animation_frame`) : la dalle mène, 240 Hz sur cette machine.
//! À 30/45/60 — et en mode éco, qui impose 30 — une tâche de cadence endormie
//! entre deux images les espace : c'est toute la différence entre « dessiner
//! moins vite » et « ne pas dessiner ».
//!
//! RIEN QUAND RIEN NE JOUE. En pause, le widget peint son dernier état une fois
//! et ne demande plus d'image ; la barre du lecteur continue de battre à 4 Hz
//! par la coquille, pas ici.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::{
    Bounds, Context, Entity, IntoElement, Render, Styled, Subscription, Task, Window, canvas, fill,
    px,
};
use nz_viz::palette::Rgb;

use crate::screens::settings::viz::{
    FrameLog, PaletteSource, Prefs, Sleeve, colour_at, hsla, mix, shade, theme_palette, zone_stops,
};
use crate::state::AppState;

/// La bande passante d'une barre : une capsule tous les 20,5 px en plein écran,
/// tous les 16 px dans une bande — les deux chiffres de `scenes/bars.js`.
const PITCH_FULL: f32 = 20.5;
const PITCH_STRIP: f32 = 16.;
/// Le nombre de barres qu'une rangée dessine, borné comme la page (18 au moins,
/// 64 au plus : au-delà, une bande fait 4 px et ne se lit plus).
const MIN_BARS: usize = 18;
pub const MAX_BARS: usize = 64;
/// Une respiration au repos : une pulsation lente, sans battement puisqu'on
/// n'analyse qu'au niveau SPECTRUM (celui que la page donne à ces deux scènes).
const BREATH_HZ: f64 = 0.45;

/// Dessine-t-on la rangée plein cadre ou dans une bande (le lecteur) ?
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// Une bande de la hauteur de son parent, l'axe au centre (le lecteur).
    Strip,
    /// Le plein écran du visualiseur : l'axe au centre, la rangée au sol.
    #[default]
    Full,
}

/// La rangée de barres. Une entité : elle porte sa cadence, son journal et sa
/// pochette décodée, et la scène qui l'affiche se contente de la monter.
pub struct Bars {
    state: Entity<AppState>,
    prefs: Prefs,
    #[allow(dead_code)]
    layout: Layout,
    /// La scène est celle qu'on regarde. Une scène inactive ne peint rien et ne
    /// demande aucune image.
    active: bool,
    /// La phase de la respiration au repos, en secondes.
    breath: f64,
    /// Temps de la dernière image peinte, pour le `dt` de `nz-viz`.
    last: Rc<Cell<Option<Instant>>>,
    sleeve: Rc<RefCell<Sleeve>>,
    log: Rc<RefCell<FrameLog>>,
    pacer: Option<Task<()>>,
    pacer_for: Option<u32>,
    _subscription: Subscription,
}

impl Bars {
    pub fn new(state: Entity<AppState>, prefs: Prefs, strip: bool, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            prefs,
            layout: if strip { Layout::Strip } else { Layout::Full },
            active: true,
            breath: 0.,
            last: Rc::new(Cell::new(None)),
            sleeve: Rc::new(RefCell::new(Sleeve::new())),
            log: Rc::new(RefCell::new(FrameLog::new())),
            pacer: None,
            pacer_for: None,
            _subscription: subscription,
        }
    }

    pub fn set_prefs(&mut self, prefs: Prefs, cx: &mut Context<Self>) {
        if prefs == self.prefs {
            return;
        }
        self.prefs = prefs;
        self.pacer = None;
        self.pacer_for = None;
        cx.notify();
    }

    /// La scène est (ou n'est plus) celle qu'on regarde.
    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.active == active {
            return;
        }
        self.active = active;
        if !active {
            self.pacer = None;
            self.pacer_for = None;
            self.log.borrow_mut().reset_clock();
        }
        cx.notify();
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Le nombre d'images peintes depuis la création, pour les tests et la
    /// mesure (`nz_frame_log viz=bars`).
    pub fn frames(&self) -> u64 {
        self.log.borrow().paints()
    }

    pub fn frame_summary(&self) -> Option<(f32, f32, f32, usize)> {
        self.log.borrow().summary()
    }

    /// Vrai quand une image doit être peinte : la scène est visible, un titre
    /// joue, et l'analyse existe (sans sortie audio il n'y a rien à dessiner).
    fn animating(&self, cx: &mut Context<Self>) -> bool {
        let state = self.state.read(cx);
        self.active && state.player.playing() && state.viz.is_some()
    }

    /// La cadence. À l'écran, l'image suivante se demande à la dalle ; à 30, 45
    /// ou 60, une tâche endormie entre deux images les espace — une seule tâche
    /// par plafond, jamais une par image.
    fn schedule(&mut self, window: &Window, cx: &mut Context<Self>) {
        match self.prefs.interval() {
            None => {
                self.pacer = None;
                self.pacer_for = None;
                window.request_animation_frame();
            }
            Some(interval) => {
                let fps = self.prefs.effective_fps().0;
                if self.pacer.is_some() && self.pacer_for == Some(fps) {
                    return;
                }
                self.pacer_for = Some(fps);
                self.pacer = Some(cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor().timer(interval).await;
                        if this.update(cx, |this, cx| this.animating(cx)).is_err() {
                            break;
                        }
                        if this.update(cx, |_, cx| cx.notify()).is_err() {
                            break;
                        }
                    }
                }));
            }
        }
    }
}

impl Render for Bars {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let dt = self
            .last
            .get()
            .map(|last| (now - last).as_secs_f64().clamp(0., 0.1))
            .unwrap_or(0.);
        self.last.set(Some(now));
        if !self.prefs.eco {
            self.breath += dt;
        }
        let animating = self.animating(cx);
        // La pochette du titre qui joue : une lecture de la carte de la
        // coquille, aucune requête (le widget de vignette s'en charge, D8a).
        let sleeve_path = {
            let state = self.state.read(cx);
            state
                .player
                .current()
                .and_then(|entry| state.cover_path(entry))
        };
        let prefs = self.prefs;
        let layout = self.layout;
        let breath = self.breath;
        let state = self.state.clone();
        let log = Rc::clone(&self.log);
        // La palette est résolue une fois par image, avant la peinture : la
        // pochette quand elle est choisie ET décodée, le thème sinon (le temps
        // qu'elle arrive, ou si le titre n'en a pas).
        let palette = match prefs.palette {
            PaletteSource::Cover => self
                .sleeve
                .borrow_mut()
                .palette(sleeve_path.as_deref(), window, cx)
                .unwrap_or_else(|| theme_palette(cx)),
            PaletteSource::Theme => theme_palette(cx),
        };
        let stops = zone_stops(&palette);

        let element = canvas(
            move |bounds, _window, _cx| bounds.size,
            move |bounds, _size, window, cx| {
                let started = Instant::now();
                // `paint_quad` peint en coordonnées de FENÊTRE : sans l'origine
                // de l'élément, la rangée se dessinerait au coin de la fenêtre,
                // par-dessus la barre latérale (vu à la première exécution).
                let origin = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                let width = f32::from(bounds.size.width);
                let height = f32::from(bounds.size.height);
                if width < 8. || height < 8. {
                    return;
                }
                let state = state.read(cx);
                let Some(viz) = &state.viz else {
                    return;
                };
                let reduced = state.reduced_motion;
                let n = bar_count(width, layout);
                let axis = height / 2.;
                // L'intensité règle l'ampleur du mouvement, pas la luminosité.
                let reach = (match layout {
                    Layout::Full => height * 0.31,
                    Layout::Strip => height / 2. - 1.,
                }) * (0.55 + 0.45 * prefs.intensity.clamp(0., 1.));
                let bw = width / n as f32;
                let body = (bw * 0.64).min(height * 0.55).max(2.);
                let r = body / 2.;
                let eco = prefs.eco;
                let breath_on = !eco && !reduced;

                viz.with(|viz| {
                    let out = viz.bars(n, dt);
                    let (levels, peaks) = out.split_at(n);
                    // Le niveau global de la page est l'enveloppe lente que le
                    // cœur de `viz_bars` garde pour ses appelants canvas ;
                    // `nz-viz` ne l'expose pas encore (V1), donc on lit la
                    // moyenne des bandes : même rôle, même lenteur, aucun
                    // aller-retour de plus.
                    let loud = if levels.is_empty() {
                        0.3
                    } else {
                        let mean = levels.iter().sum::<f32>() / levels.len() as f32;
                        0.3 + 0.7 * (mean * 1.5).clamp(0., 1.)
                    };
                    for (i, &level) in levels.iter().enumerate() {
                        let t = i as f32 / (n as f32 - 1.).max(1.);
                        let v = (level * loud).clamp(0., 1.);
                        // La basse est un losange, l'aigu une aiguille ; une
                        // bande forte s'épaissit.
                        let taper = 1.06 - 0.26 * t;
                        let weight = (1. - t * 2.5).max(0.) * (v * 1.4).min(1.);
                        let half_w =
                            (r * taper * (0.88 + 0.22 * (v / 0.9).min(1.) + 0.18 * weight))
                                .min(bw * 0.46);
                        let rest = 1. - 0.65 * ((v - 0.05) / 0.65).clamp(0., 1.);
                        let swing = if breath_on {
                            (std::f64::consts::TAU * (breath * BREATH_HZ + f64::from(t) * 0.55))
                                .sin() as f32
                        } else {
                            0.
                        };
                        let min_half = half_w * (1. + 0.46 * (0.5 + 0.5 * swing) * rest);
                        let half = min_half.max(v * reach);
                        let x = origin.0 + i as f32 * bw + (bw - half_w * 2.) / 2.;
                        let colour = colour_at(&stops, t);
                        let peak = peaks.get(i).copied().unwrap_or(0.) * reach * loud;
                        if layout == Layout::Full && !eco && peak > half + 1.5 {
                            window.paint_quad(
                                fill(
                                    Bounds {
                                        origin: gpui_kit::point(px(x), px(origin.1 + axis - peak)),
                                        size: gpui_kit::size(px(half_w * 2.), px(peak * 2.)),
                                    },
                                    hsla(colour, 0.14),
                                )
                                .corner_radii(px(half_w)),
                            );
                        }
                        let rect = Bounds {
                            origin: gpui_kit::point(px(x), px(origin.1 + axis - half)),
                            size: gpui_kit::size(px(half_w * 2.), px(half * 2.)),
                        };
                        window.paint_quad(fill(rect, hsla(colour, 0.92)).corner_radii(px(half_w)));
                        if eco {
                            // Éco : une seule teinte plate par barre, et rien
                            // d'autre — c'est le grade que le mode demande.
                            continue;
                        }
                        let inset = (half_w * 0.3).min(1.4);
                        let inner_w = half_w * 2. - inset * 2.;
                        let inner_h = half * 2. - inset * 2.;
                        if inner_w > 0.6 && inner_h > 0.6 {
                            let inner = mix(colour, shade(colour, 255, 0.45), 0.35 + 0.35 * v);
                            window.paint_quad(
                                fill(
                                    Bounds {
                                        origin: gpui_kit::point(
                                            px(x + inset),
                                            px(origin.1 + axis - half + inset),
                                        ),
                                        size: gpui_kit::size(px(inner_w), px(inner_h)),
                                    },
                                    hsla(inner, 0.45 + 0.35 * v),
                                )
                                .corner_radii(px((half_w - inset).max(0.5))),
                            );
                        }
                        // Le cœur chaud sur l'axe, comme la perle de la page.
                        let core_h = half.min(half_w * 1.7).max(1.);
                        window.paint_quad(
                            fill(
                                Bounds {
                                    origin: gpui_kit::point(
                                        px(x + inset),
                                        px(origin.1 + axis - core_h),
                                    ),
                                    size: gpui_kit::size(
                                        px((half_w * 2. - inset * 2.).max(0.6)),
                                        px(core_h * 2.),
                                    ),
                                },
                                hsla(shade(colour, 255, 0.72), 0.22 + 0.28 * v),
                            )
                            .corner_radii(px((half_w - inset).max(0.5))),
                        );
                    }
                });
                log.borrow_mut()
                    .note("bars", started.elapsed().as_secs_f32() * 1e6);
            },
        )
        .size_full();

        if animating {
            self.schedule(window, cx);
        }
        element
    }
}

/// Combien de barres une rangée de `width` px porte : le pas de la page, borné.
pub fn bar_count(width: f32, layout: Layout) -> usize {
    let pitch = match layout {
        Layout::Full => PITCH_FULL,
        Layout::Strip => PITCH_STRIP,
    };
    ((width / pitch).floor() as usize).clamp(MIN_BARS, MAX_BARS)
}

/// La couleur d'une bande à la position `t`, exposée pour que le test de
/// palette dise la même chose que le dessin.
pub fn band_colour(stops: &[Rgb; 7], t: f32) -> Rgb {
    colour_at(stops, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_row_has_one_capsule_per_two_centimetres_and_never_more_than_64() {
        // Le pas de `scenes/bars.js` : une capsule tous les 20,5 px en plein
        // écran, tous les 16 px dans une bande. 2 560 px → 64 (le plafond),
        // 1 280 px → 62, une bande de 560 px → 35, une bande étroite → 18.
        assert_eq!(bar_count(2560., Layout::Full), MAX_BARS);
        assert_eq!(bar_count(1280., Layout::Full), 62);
        assert_eq!(bar_count(560., Layout::Strip), 35);
        assert_eq!(bar_count(100., Layout::Strip), MIN_BARS);
        // Un écran de 240 Hz n'a pas plus de barres qu'un autre : le plafond
        // borne le coût par image.
        assert!(bar_count(3840., Layout::Full) <= MAX_BARS);
    }

    #[test]
    fn a_bar_takes_the_colour_of_the_sleeve_zone_under_it() {
        // Sept arrêts, du bord gauche au bord droit de la pochette : la
        // première barre est la zone gauche, la dernière la zone droite.
        let mut stops = [[0u8; 3]; 7];
        for (i, stop) in stops.iter_mut().enumerate() {
            stop[0] = (i * 30) as u8;
        }
        assert_eq!(band_colour(&stops, 0.), stops[0]);
        assert_eq!(band_colour(&stops, 1.), stops[6]);
        // Au milieu : entre le centre et le suivant, jamais un saut.
        let middle = band_colour(&stops, 0.5);
        assert!(middle[0] >= 90 && middle[0] <= 120, "{middle:?}");
    }
}
