//! L'oscilloscope : le signal lui-même, une trace par canal.
//!
//! LA GÉOMÉTRIE EST CELLE DE LA PAGE. `rhythm::viz_scope` (par `nz-viz`) fait
//! le déclenchement (un passe-bas à 320 Hz, une porte de Schmitt, une chasse en
//! arrière dans la fenêtre), la réduction de chaque colonne à un couple
//! min/max, et l'auto-échelle. C'est ce qui fait qu'une trace TIENT EN PLACE
//! au lieu de glisser, et c'est déjà prouvé image pour image (V1).
//!
//! CE QUE CE MODULE DESSINE. La face de l'instrument — le cadre, les demies, les
//! divisions, la ligne de zéro —, la trace en quadrilatères (un par colonne et
//! par canal, élargi à la largeur de la colonne), et le repère de
//! déclenchement, qui s'allume quand la trace est verrouillée. Deux passes
//! seulement : un halo large et le faisceau. La page en fait trois en 2D ; ici
//! chaque passe est un `paint_quad`, donc une passe de trop se paie à chaque
//! image sur une dalle à 240 Hz.
//!
//! LES DEUX RÉGLAGES DE L'INSTRUMENT sont ceux de `stores.js` : l'orientation
//! (deux bandes ou deux colonnes) et la teinte des voies (`duo` : les deux bouts
//! de la palette, `mono` : la couleur dominante, `sweep` : la palette parcourue
//! le long de la trace, exactement comme les barres).
//!
//! EN ÉCO, le halo disparaît et le faisceau reste : une trace qu'on ne voit plus
//! ne serait pas une version économe de l'instrument, ce serait un écran noir.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::{
    Bounds, Context, Entity, Hsla, InteractiveElement as _, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, Task, Window, canvas, div, fill, px, rems,
};
use nz_viz::palette::Rgb;

use crate::screens::settings::viz::{
    FrameLog, Orientation, PaletteSource, Prefs, Sleeve, Tint, colour_at, hsla, shade,
    theme_palette, zone_stops,
};
use crate::state::AppState;
use crate::theme;

/// Le nombre de divisions de la face (`quality.js#SCOPE.divisions`).
const DIVISIONS: usize = 6;
/// La colonne la plus fine que le dessin paie : le palier « haute » de la page
/// (`quality.js#SCOPE`, 512 points). Le cœur en accepte 4 096, mais 4 096
/// quadrilatères par voie et par image coûtent le budget d'une dalle à 240 Hz
/// pour une différence invisible à l'œil — 512 colonnes sur une voie de
/// 1 548 px font déjà une colonne tous les 3 px.
pub const SCOPE_POINTS: usize = 512;
/// Le plein cadrage de la trace : on s'arrête avant le cadre, pour qu'un master
/// limité se lise « presque en haut » au lieu de coller au bord.
const DEFLECT: f32 = 0.88;

/// Dessine-t-on la trace plein cadre ou dans une bande ?
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    Strip,
    #[default]
    Full,
}

/// L'oscilloscope. Même contrat que les barres : une entité qui porte sa
/// cadence, sa pochette et son journal.
pub struct Scope {
    state: Entity<AppState>,
    prefs: Prefs,
    #[allow(dead_code)]
    layout: Layout,
    active: bool,
    last: Rc<Cell<Option<Instant>>>,
    sleeve: Rc<RefCell<Sleeve>>,
    log: Rc<RefCell<FrameLog>>,
    pacer: Option<Task<()>>,
    pacer_for: Option<u32>,
    _subscription: Subscription,
}

impl Scope {
    pub fn new(state: Entity<AppState>, prefs: Prefs, strip: bool, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            prefs,
            layout: if strip { Layout::Strip } else { Layout::Full },
            active: true,
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

    /// Le nombre d'images peintes depuis la création.
    pub fn frames(&self) -> u64 {
        self.log.borrow().paints()
    }

    pub fn frame_summary(&self) -> Option<(f32, f32, f32, usize)> {
        self.log.borrow().summary()
    }

    fn animating(&self, cx: &mut Context<Self>) -> bool {
        let state = self.state.read(cx);
        self.active && state.player.playing() && state.viz.is_some()
    }

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
                        let alive = this.update(cx, |_, cx| cx.notify());
                        if alive.is_err() {
                            break;
                        }
                    }
                }));
            }
        }
    }
}

/// Un rectangle de voie, en pixels de la fenêtre.
#[derive(Clone, Copy, Debug, Default)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// Le repère d'une voie : l'origine (temps 0, zéro volt), l'axe du temps et
/// l'axe de la tension.
#[derive(Clone, Copy, Debug)]
struct Lane {
    ox: f32,
    oy: f32,
    ax: f32,
    ay: f32,
    vx: f32,
    vy: f32,
    len: f32,
    span: f32,
}

impl Lane {
    fn of(rect: Rect, orientation: Orientation) -> Option<Self> {
        let short = rect.w.min(rect.h);
        if short < 8. {
            return None;
        }
        let pad = (short * 0.09).clamp(4., 26.).min(short * 0.35);
        let in_x = rect.w - 2. * pad;
        let in_y = rect.h - 2. * pad;
        match orientation {
            Orientation::Horizontal => Some(Self {
                ox: rect.x + pad,
                oy: rect.y + rect.h / 2.,
                ax: in_x,
                ay: 0.,
                vx: 0.,
                vy: -in_y / 2.,
                len: in_x,
                span: in_y / 2.,
            }),
            Orientation::Vertical => Some(Self {
                ox: rect.x + rect.w / 2.,
                oy: rect.y + pad,
                ax: 0.,
                ay: in_y,
                vx: in_x / 2.,
                vy: 0.,
                len: in_y,
                span: in_x / 2.,
            }),
        }
    }

    fn point(&self, t: f32, v: f32) -> (f32, f32) {
        (
            self.ox + self.ax * t + self.vx * v,
            self.oy + self.ay * t + self.vy * v,
        )
    }
}

/// Les deux voies, selon l'orientation.
fn lanes(width: f32, height: f32, orientation: Orientation) -> (Rect, Rect) {
    match orientation {
        Orientation::Horizontal => (
            Rect {
                x: 0.,
                y: 0.,
                w: width,
                h: height / 2.,
            },
            Rect {
                x: 0.,
                y: height / 2.,
                w: width,
                h: height / 2.,
            },
        ),
        Orientation::Vertical => (
            Rect {
                x: 0.,
                y: 0.,
                w: width / 2.,
                h: height,
            },
            Rect {
                x: width / 2.,
                y: 0.,
                w: width / 2.,
                h: height,
            },
        ),
    }
}

/// Un trait de la face, dans le repère de la voie : `(temps, tension)`.
fn line(window: &mut Window, lane: &Lane, from: (f32, f32), to: (f32, f32), colour: Hsla) {
    let (x0, y0) = lane.point(from.0, from.1);
    let (x1, y1) = lane.point(to.0, to.1);
    let (x, y) = (x0.min(x1), y0.min(y1));
    window.paint_quad(fill(
        Bounds {
            origin: gpui_kit::point(px(x), px(y)),
            size: gpui_kit::size(px((x1 - x0).abs().max(1.)), px((y1 - y0).abs().max(1.))),
        },
        colour,
    ));
}

/// Le nom d'une voie et son point de couleur.
fn channel_label(name: &'static str, colour: Rgb, cx: &gpui_kit::App) -> impl IntoElement + use<> {
    div()
        .flex()
        .items_center()
        .gap(rems(0.375))
        .child(div().w(rems(0.375)).h(rems(0.375)).bg(hsla(colour, 0.9)))
        .child(
            div()
                .text_size(rems(0.6875))
                .font_family(theme::mono(cx))
                .text_color(theme::secondary(cx))
                .child(SharedString::from(name)),
        )
}

impl Render for Scope {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let dt = self
            .last
            .get()
            .map(|last| (now - last).as_secs_f64().clamp(0., 0.1))
            .unwrap_or(0.);
        self.last.set(Some(now));
        let animating = self.animating(cx);
        let sleeve_path = {
            let state = self.state.read(cx);
            state
                .player
                .current()
                .and_then(|entry| state.cover_path(entry))
        };
        let prefs = self.prefs;
        let state = self.state.clone();
        let log = Rc::clone(&self.log);
        // La palette est résolue ici, une fois : le canvas et les étiquettes
        // des voies doivent porter les mêmes couleurs.
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
                // Même règle que les barres : les quadrilatères sont en
                // coordonnées de fenêtre, l'origine de l'élément s'ajoute.
                let origin = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                let width = f32::from(bounds.size.width);
                let height = f32::from(bounds.size.height);
                if width < 16. || height < 16. {
                    return;
                }
                let state = state.read(cx);
                let Some(viz) = &state.viz else {
                    return;
                };
                let (mut rect_a, mut rect_b) = lanes(width, height, prefs.orientation);
                for rect in [&mut rect_a, &mut rect_b] {
                    rect.x += origin.0;
                    rect.y += origin.1;
                }
                let (Some(lane_a), Some(lane_b)) = (
                    Lane::of(rect_a, prefs.orientation),
                    Lane::of(rect_b, prefs.orientation),
                ) else {
                    return;
                };
                let len = lane_a.len.max(lane_b.len);
                let cols = (len.round() as usize).clamp(32, SCOPE_POINTS);
                let eco = prefs.eco;
                let intensity = prefs.intensity.clamp(0., 1.);
                let colour_mode = prefs.colour;

                // La face de l'instrument, puis les traces : tout se peint sous
                // le verrou de `VizState`, court (0,08 ms p95 pour 512 colonnes).
                viz.with(|viz| {
                    let st = viz.scope_state();
                    let gain = st.gain;
                    let locked = st.locked;
                    let signal = st.signal;
                    let (a, b) = viz.scope(cols, dt);

                    let face = hsla(stops[3], 0.10);
                    for lane in [lane_a, lane_b] {
                        line(window, &lane, (0., 1.), (1., 1.), face);
                        line(window, &lane, (1., 1.), (1., -1.), face);
                        line(window, &lane, (1., -1.), (0., -1.), face);
                        line(window, &lane, (0., -1.), (0., 1.), face);
                        line(window, &lane, (0., 0.5), (1., 0.5), face.opacity(0.5));
                        line(window, &lane, (0., -0.5), (1., -0.5), face.opacity(0.5));
                        for d in 1..DIVISIONS {
                            let t = d as f32 / DIVISIONS as f32;
                            line(window, &lane, (t, -1.), (t, 1.), face.opacity(0.35));
                        }
                        line(window, &lane, (0., 0.), (1., 0.), hsla(stops[3], 0.22));
                    }

                    // La trace : un quadrilatère par colonne et par canal, de la
                    // plus petite à la plus grande valeur de la colonne — dense
                    // dans les pentes, fin quand le signal est lent.
                    let scale = gain * DEFLECT * (0.55 + intensity * 0.45);
                    let column = len / cols as f32;
                    for (channel, (lane, trace)) in
                        [(lane_a, a), (lane_b, b)].into_iter().enumerate()
                    {
                        if trace.len() < cols * 2 {
                            continue;
                        }
                        for c in 0..cols {
                            let t = c as f32 / (cols as f32 - 1.).max(1.);
                            let hi = (trace[c * 2 + 1] * scale).clamp(-1., 1.);
                            let lo = (trace[c * 2] * scale).clamp(-1., 1.);
                            let (x, y0) = lane.point(t, hi);
                            let (_, y1) = lane.point(t, lo);
                            let (top, bottom) = (y0.min(y1), y0.max(y1));
                            let height = (bottom - top).max(1.2);
                            let colour = match colour_mode {
                                Tint::Mono => stops[3],
                                Tint::Duo => {
                                    if channel == 0 {
                                        stops[0]
                                    } else {
                                        stops[6]
                                    }
                                }
                                Tint::Sweep => colour_at(&stops, t),
                            };
                            if !eco {
                                window.paint_quad(fill(
                                    Bounds {
                                        origin: gpui_kit::point(
                                            px(x - column * 1.2),
                                            px(top - height * 0.5),
                                        ),
                                        size: gpui_kit::size(
                                            px(column * 2.4 + 1.),
                                            px(height * 2.),
                                        ),
                                    },
                                    hsla(colour, 0.10),
                                ));
                            }
                            window.paint_quad(fill(
                                Bounds {
                                    origin: gpui_kit::point(px(x - column / 2.), px(top)),
                                    size: gpui_kit::size(px(column.max(1.)), px(height)),
                                },
                                hsla(colour, 0.92),
                            ));
                            let core = (bottom - top) * 0.5 + 0.6;
                            let middle = (top + bottom) / 2.;
                            window.paint_quad(fill(
                                Bounds {
                                    origin: gpui_kit::point(
                                        px(x - column / 2. - 0.5),
                                        px(middle - core),
                                    ),
                                    size: gpui_kit::size(px(column.max(1.) + 1.), px(core * 2.)),
                                },
                                hsla(shade(colour, 255, 0.55), 0.55),
                            ));
                        }
                        // Le repère de déclenchement, dans la marge de la voie :
                        // il s'allume quand la trace est verrouillée, donc « elle
                        // tient en place » et « elle glisse » se distinguent.
                        if !eco && locked > 0.02 && signal {
                            let marker = (lane.span * 0.07).clamp(3.5, 9.);
                            let (mx, my) = (lane.ox - marker * 1.4, lane.oy);
                            window.paint_quad(fill(
                                Bounds {
                                    origin: gpui_kit::point(px(mx), px(my - marker)),
                                    size: gpui_kit::size(px(marker.max(1.5)), px(marker * 2.)),
                                },
                                hsla(shade(stops[0], 255, 0.35), 0.28 + 0.55 * locked),
                            ));
                        }
                    }
                });
                log.borrow_mut()
                    .note("scope", started.elapsed().as_secs_f32() * 1e6);
            },
        )
        .size_full();

        // Les deux voies nommées, comme sur un vrai oscilloscope : deux
        // sinusoïdes sans étiquette sont une décoration, « G » et « D » sont un
        // instrument.
        let labels = div()
            .absolute()
            .top(rems(0.5))
            .left(rems(0.75))
            .flex()
            .gap(rems(2.))
            .child(channel_label("G", stops[0], cx))
            .child(channel_label("D", stops[6], cx));

        if animating {
            self.schedule(window, cx);
        }

        div()
            .id("viz-scope")
            .debug_selector(|| "viz-scope".to_owned())
            .relative()
            .size_full()
            .child(element)
            .child(labels)
    }
}

#[cfg(test)]
mod tests {
    use nz_viz::MAX_COLS;

    use super::*;

    #[test]
    fn each_orientation_puts_the_two_channels_where_it_says() {
        // Horizontal : deux bandes empilées (le haut et le bas de la fenêtre) ;
        // vertical : deux colonnes côte à côte. Les deux voies se partagent la
        // place sans se recouvrir — c'est ce que la capture vérifie à l'œil.
        let (a, b) = lanes(1000., 600., Orientation::Horizontal);
        assert_eq!((a.x, a.y, a.w, a.h), (0., 0., 1000., 300.));
        assert_eq!((b.x, b.y, b.w, b.h), (0., 300., 1000., 300.));
        let (a, b) = lanes(1000., 600., Orientation::Vertical);
        assert_eq!((a.x, a.y, a.w, a.h), (0., 0., 500., 600.));
        assert_eq!((b.x, b.y, b.w, b.h), (500., 0., 500., 600.));

        let lane = Lane::of(a, Orientation::Vertical).expect("a lane");
        assert!(lane.len > 100. && lane.span > 100.);
        // La voie va de haut en bas en vertical : le temps avance sur y.
        assert_eq!((lane.ax, lane.ay), (0., lane.len));
        assert!(
            Lane::of(
                Rect {
                    w: 4.,
                    h: 4.,
                    ..Rect::default()
                },
                Orientation::Horizontal
            )
            .is_none()
        );
    }

    #[test]
    fn the_column_count_follows_the_lane_and_stops_at_the_pages_high_tier() {
        // Une colonne par pixel de base de temps, plafonnée aux 512 points du
        // palier « haute » de la page : le cœur en accepte 4 096, mais une
        // colonne tous les 3 px est déjà la précision d'un oscilloscope, et
        // 4 096 quadrilatères par voie ne tiendraient pas les 4,2 ms d'une
        // image à 240 Hz pour un gain invisible.
        const { assert!(SCOPE_POINTS <= MAX_COLS) };
        let lane = Lane::of(
            Rect {
                x: 0.,
                y: 0.,
                w: 1548.,
                h: 300.,
            },
            Orientation::Horizontal,
        )
        .expect("a lane");
        let cols = (lane.len.round() as usize).clamp(32, SCOPE_POINTS);
        assert_eq!(cols, 512, "le plafond est celui du palier haute");
        let small = Lane::of(
            Rect {
                x: 0.,
                y: 0.,
                w: 20.,
                h: 60.,
            },
            Orientation::Horizontal,
        )
        .expect("a lane");
        assert_eq!(
            (small.len.round() as usize).clamp(32, SCOPE_POINTS),
            32,
            "une voie de 12 px de base de temps garde le plancher de 32 colonnes"
        );
    }
}
