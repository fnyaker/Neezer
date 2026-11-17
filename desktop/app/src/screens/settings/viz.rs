//! Réglages → Animations, et tout ce que la visualisation partage.
//!
//! Ce que le SPA met dans `VisualizerSettings.svelte` + `stores.js`, pour les
//! deux seules scènes du bureau (`docs/agents/desktop.md`, *The need* : les
//! barres et l'oscilloscope, rien d'autre) : la scène, la provenance des
//! couleurs, l'intensité, le plafond d'images, le mode éco, l'orientation et la
//! teinte de l'oscilloscope. Les clés sont celles de la page (`viz.mode`,
//! `viz.palette`, `viz.intensity`, `viz.fps`, `viz.eco`, `viz.scope.*`) : une
//! personne qui utilise les deux clients retrouve les mêmes valeurs.
//!
//! LE MODE ÉCO EST DIFFÉRENT DE CELUI DE LA PAGE, ET C'EST VOULU. Le SPA coupe
//! toute animation quand `viz.eco` est vrai (`effectiveMode` renvoie « off »).
//! Le lot D10b du bureau mesure au contraire « éco : plafond à 30 images/s,
//! < 5 % d'un cœur » : ici l'éco garde l'image et retire ce qui coûte — le
//! plafond tombe à 30, et les scènes dessinent leur version plate (une seule
//! teinte par barre, pas de halo, pas de respiration, pas de fantôme de crête).
//! Un visualiseur qu'on ne voit plus ne se règle pas ; un visualiseur à 30
//! images/s qui ne touche pas la batterie, si.
//!
//! CE MODULE POSSÈDE AUSSI LES PIÈCES QUE LES DEUX WIDGETS PARTAGENT : la
//! résolution de la palette (le thème, ou la pochette décodée une fois par
//! titre), la conversion `Rgb` de `nz-viz` vers `Hsla` de GPUI, et le journal
//! d'images (`NZ_FRAME_LOG=1`). Les mettre ici évite que les barres et
//! l'oscilloscope en aient chacun une copie, et c'est le fichier que le lot
//! possède déjà.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui_kit::base::slider::{SliderEvent, SliderState, SliderValue};
use gpui_kit::{
    App, AppContext as _, Context, Entity, Hsla, ImageCacheItem, InteractiveElement as _,
    IntoElement, ParentElement, Render, Resource, Rgba, StatefulInteractiveElement as _, Styled,
    Subscription, Window, div, rems,
};
use gpui_omarchy::{ActiveTheme, switch, toggle, toggle_group};
use nz_cache::Cache;
use nz_viz::palette::{self, Palette};
use serde::{Deserialize, Serialize};

use super::{card, card_title, hint};
use crate::state::AppState;
use crate::theme;
use crate::widgets::bars::Bars;
use crate::widgets::scope::Scope;

/// Les clés de l'appareil : celles de `stores.js`, une par réglage.
pub mod keys {
    /// `stores.js#vizMode` — ici seulement `bars` et `scope`.
    pub const MODE: &str = "viz.mode";
    /// `stores.js#vizPalette` — ici `cover` et `theme`.
    pub const PALETTE: &str = "viz.palette";
    /// `stores.js#vizIntensity`, 0…1.
    pub const INTENSITY: &str = "viz.intensity";
    /// `stores.js#vizFps` : 0 = la fréquence de l'écran.
    pub const FPS: &str = "viz.fps";
    /// `stores.js#ecoMode`.
    pub const ECO: &str = "viz.eco";
    /// `stores.js#vizScopeOrientation`.
    pub const SCOPE_ORIENTATION: &str = "viz.scope.orientation";
    /// `stores.js#vizScopeColour`.
    pub const SCOPE_COLOUR: &str = "viz.scope.colour";
}

/// Les deux scènes que le bureau dessine (`docs/agents/desktop.md`, *The need*).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Bars,
    Scope,
}

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::Bars, Mode::Scope];

    pub fn id(self) -> &'static str {
        match self {
            Mode::Bars => "bars",
            Mode::Scope => "scope",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Bars => "Barres",
            Mode::Scope => "Oscilloscope",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Mode::Bars => "Le spectre, avec une vraie résolution dans les graves",
            Mode::Scope => "Le signal lui-même, une trace par canal, déclenchée",
        }
    }

    /// Ce que la page stockait : toute autre scène (pulse, aurora, smart) n'a
    /// pas d'équivalent ici et retombe sur les barres.
    pub fn parse(value: &str) -> Mode {
        match value {
            "scope" => Mode::Scope,
            _ => Mode::Bars,
        }
    }
}

/// D'où viennent les couleurs. Deux sources, celles que `nz-viz` sait produire :
/// la pochette (`coverpalette.js#analyse`, porté) ou le thème Omarchy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaletteSource {
    #[default]
    Cover,
    Theme,
}

impl PaletteSource {
    pub const ALL: [PaletteSource; 2] = [PaletteSource::Cover, PaletteSource::Theme];

    pub fn id(self) -> &'static str {
        match self {
            PaletteSource::Cover => "cover",
            PaletteSource::Theme => "theme",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PaletteSource::Cover => "Pochette",
            PaletteSource::Theme => "Thème",
        }
    }

    pub fn parse(value: &str) -> PaletteSource {
        match value {
            "theme" => PaletteSource::Theme,
            _ => PaletteSource::Cover,
        }
    }
}

/// L'orientation de l'instrument : deux bandes empilées, ou deux colonnes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    #[default]
    Horizontal,
    Vertical,
}

impl Orientation {
    pub const ALL: [Orientation; 2] = [Orientation::Horizontal, Orientation::Vertical];

    pub fn id(self) -> &'static str {
        match self {
            Orientation::Horizontal => "horizontal",
            Orientation::Vertical => "vertical",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Orientation::Horizontal => "Horizontal",
            Orientation::Vertical => "Vertical",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Orientation::Horizontal => "Deux bandes, l'une sous l'autre",
            Orientation::Vertical => "Deux colonnes, côte à côte",
        }
    }

    pub fn parse(value: &str) -> Orientation {
        match value {
            "vertical" => Orientation::Vertical,
            _ => Orientation::Horizontal,
        }
    }
}

/// La teinte des deux voies, par-dessus la source de palette.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tint {
    #[default]
    Duo,
    Mono,
    Sweep,
}

impl Tint {
    pub const ALL: [Tint; 3] = [Tint::Duo, Tint::Mono, Tint::Sweep];

    pub fn id(self) -> &'static str {
        match self {
            Tint::Duo => "duo",
            Tint::Mono => "mono",
            Tint::Sweep => "sweep",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tint::Duo => "Deux teintes",
            Tint::Mono => "Une teinte",
            Tint::Sweep => "Dégradé",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Tint::Duo => "Gauche et droite aux deux bouts de la palette",
            Tint::Mono => "Les deux voies sur la couleur dominante",
            Tint::Sweep => "La teinte parcourt la trace, comme les barres",
        }
    }

    pub fn parse(value: &str) -> Tint {
        match value {
            "mono" => Tint::Mono,
            "sweep" => Tint::Sweep,
            _ => Tint::Duo,
        }
    }
}

/// Le plafond d'images. `Display` suit l'écran (240 Hz ici, la cible du lot) ;
/// les autres valeurs sont celles que le SPA offre.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FpsCap(pub u32);

impl FpsCap {
    /// 0 = la fréquence de l'écran, exactement ce que `stores.js` stocke.
    pub const DISPLAY: FpsCap = FpsCap(0);
    pub const ALL: [FpsCap; 4] = [FpsCap(0), FpsCap(30), FpsCap(45), FpsCap(60)];

    pub fn label(self) -> &'static str {
        match self.0 {
            0 => "Écran",
            30 => "30",
            45 => "45",
            _ => "60",
        }
    }

    pub fn hint(self) -> &'static str {
        match self.0 {
            0 => "Sans plafond : la fréquence de l'écran",
            30 => "Le plus économe",
            45 => "Bon compromis",
            _ => "Fluide",
        }
    }

    /// La période entre deux images, ou `None` quand c'est l'écran qui décide.
    pub fn interval(self) -> Option<Duration> {
        (self.0 > 0).then(|| Duration::from_secs_f64(1.0 / f64::from(self.0)))
    }

    /// Le nombre d'images visé, pour la phrase de la carte.
    pub fn effective(self) -> u32 {
        if self.0 == 0 { 240 } else { self.0 }
    }
}

/// Tout ce que la visualisation mémorise sur l'appareil.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    pub mode: Mode,
    pub palette: PaletteSource,
    pub intensity: f32,
    /// 0 = la fréquence de l'écran (`stores.js#vizFps`, valeur par défaut 60
    /// dans la page ; ici l'écran, parce que le bureau vise ses 240 Hz).
    pub fps: FpsCap,
    pub eco: bool,
    pub orientation: Orientation,
    pub colour: Tint,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            mode: Mode::default(),
            palette: PaletteSource::default(),
            intensity: 0.7,
            fps: FpsCap::DISPLAY,
            eco: false,
            orientation: Orientation::default(),
            colour: Tint::default(),
        }
    }
}

impl Prefs {
    /// Lit les sept clés, une par réglage ; une clé absente ou d'un autre type
    /// garde la valeur par défaut (c'est la réparation d'une montée de version,
    /// règle 7).
    pub fn load(cache: Option<&Cache>) -> Self {
        let default = Self::default();
        let Some(cache) = cache else {
            return default;
        };
        let kv = cache.kv();
        let mode = kv
            .get::<String>(keys::MODE)
            .map(|value| Mode::parse(&value))
            .unwrap_or(default.mode);
        let palette = kv
            .get::<String>(keys::PALETTE)
            .map(|value| PaletteSource::parse(&value))
            .unwrap_or(default.palette);
        let intensity = kv
            .get::<f64>(keys::INTENSITY)
            .map(|value| (value as f32).clamp(0., 1.))
            .unwrap_or(default.intensity);
        let fps = kv
            .get::<u32>(keys::FPS)
            .map(|value| match value {
                30 | 45 | 60 => FpsCap(value),
                _ => FpsCap::DISPLAY,
            })
            .unwrap_or(default.fps);
        let eco = kv.get::<bool>(keys::ECO).unwrap_or(default.eco);
        let orientation = kv
            .get::<String>(keys::SCOPE_ORIENTATION)
            .map(|value| Orientation::parse(&value))
            .unwrap_or(default.orientation);
        let colour = kv
            .get::<String>(keys::SCOPE_COLOUR)
            .map(|value| Tint::parse(&value))
            .unwrap_or(default.colour);
        Self {
            mode,
            palette,
            intensity,
            fps,
            eco,
            orientation,
            colour,
        }
    }

    /// Écrit les sept clés. Un échec d'écriture ne casse pas le réglage en
    /// cours : l'appareil garde la valeur pour cette session, la prochaine
    /// relit l'ancienne (et le journal dit pourquoi).
    pub fn save(&self, cache: Option<&Cache>) {
        let Some(cache) = cache else {
            return;
        };
        let kv = cache.kv();
        let writes: [(&str, Result<(), nz_cache::Error>); 7] = [
            (keys::MODE, kv.set(keys::MODE, &self.mode.id())),
            (keys::PALETTE, kv.set(keys::PALETTE, &self.palette.id())),
            (
                keys::INTENSITY,
                kv.set(keys::INTENSITY, &f64::from(self.intensity)),
            ),
            (keys::FPS, kv.set(keys::FPS, &self.fps.0)),
            (keys::ECO, kv.set(keys::ECO, &self.eco)),
            (
                keys::SCOPE_ORIENTATION,
                kv.set(keys::SCOPE_ORIENTATION, &self.orientation.id()),
            ),
            (
                keys::SCOPE_COLOUR,
                kv.set(keys::SCOPE_COLOUR, &self.colour.id()),
            ),
        ];
        for (key, result) in writes {
            if let Err(error) = result {
                tracing::debug!(%error, key, "le réglage de visualisation n'a pas été écrit");
            }
        }
    }

    /// Le plafond effectif : l'éco impose 30, quel que soit le réglage.
    pub fn effective_fps(&self) -> FpsCap {
        if self.eco { FpsCap(30) } else { self.fps }
    }

    pub fn interval(&self) -> Option<Duration> {
        self.effective_fps().interval()
    }
}

/// Le thème, comme palette : l'accent de la bande Omarchy, sa surface, et le
/// drapeau clair de `Palette::light` (le fond au-dessus du gris moyen).
pub fn theme_palette(cx: &App) -> Palette {
    let theme = cx.omarchy();
    Palette::from_theme(
        rgb(theme.accent),
        rgb(theme.surface),
        theme.background.l > 0.5,
    )
}

/// `Hsla` de GPUI vers les octets que `nz-viz` attend.
pub fn rgb(colour: Hsla) -> palette::Rgb {
    let colour: Rgba = colour.to_rgb();
    [
        (colour.r.clamp(0., 1.) * 255.).round() as u8,
        (colour.g.clamp(0., 1.) * 255.).round() as u8,
        (colour.b.clamp(0., 1.) * 255.).round() as u8,
    ]
}

/// Les octets de `nz-viz` vers une couleur de GPUI, à l'alpha demandé.
pub fn hsla(colour: palette::Rgb, alpha: f32) -> Hsla {
    Rgba {
        r: f32::from(colour[0]) / 255.,
        g: f32::from(colour[1]) / 255.,
        b: f32::from(colour[2]) / 255.,
        a: alpha.clamp(0., 1.),
    }
    .into()
}

/// Mélange deux couleurs de palette, `t` de 0 (a) à 1 (b).
pub fn mix(a: palette::Rgb, b: palette::Rgb, t: f32) -> palette::Rgb {
    let t = t.clamp(0., 1.);
    let channel = |i: usize| {
        let value = f32::from(a[i]) * (1. - t) + f32::from(b[i]) * t;
        value.round().clamp(0., 255.) as u8
    };
    [channel(0), channel(1), channel(2)]
}

/// Éclaircit ou assombrit vers `target` (255 = blanc, 0 = noir).
pub fn shade(colour: palette::Rgb, target: u8, amount: f32) -> palette::Rgb {
    let amount = amount.clamp(0., 1.);
    let channel = |i: usize| {
        let value = f32::from(colour[i]) * (1. - amount) + f32::from(target) * amount;
        value.round().clamp(0., 255.) as u8
    };
    [channel(0), channel(1), channel(2)]
}

/// Les sept arrêts de la palette, dérivés des trois zones de la pochette : la
/// même idée que `ZONE_COUNT` de la page — une barre prend la couleur de
/// l'endroit de la pochette qu'elle occupe.
pub fn zone_stops(palette: &Palette) -> [palette::Rgb; 7] {
    let zones = palette.zone_colours();
    // Chaque arrêt est une position sur la pochette : 0 = le bord gauche,
    // 3 = le centre, 6 = le bord droit, les autres interpolés.
    std::array::from_fn(|i| match i {
        0 => zones[0],
        1 => mix(zones[0], zones[1], 1. / 3.),
        2 => mix(zones[0], zones[1], 2. / 3.),
        3 => zones[1],
        4 => mix(zones[1], zones[2], 1. / 3.),
        5 => mix(zones[1], zones[2], 2. / 3.),
        _ => zones[2],
    })
}

/// La couleur d'un endroit de la palette, `t` de 0 (gauche) à 1 (droite), sur
/// les sept arrêts de [`zone_stops`].
pub fn colour_at(stops: &[palette::Rgb; 7], t: f32) -> palette::Rgb {
    let x = t.clamp(0., 1.) * 6.;
    let i = (x.floor() as usize).min(5);
    mix(stops[i], stops[i + 1], x - i as f32)
}

/// La pochette du titre, décodée une fois par fichier, et sa palette.
///
/// `nz-viz::palette::analyse` veut des octets RGBA ; GPUI décode déjà les
/// images pour les peindre (`ImageCacheItem`), et `RenderImage::as_bytes`
/// rend ses pixels en BGRA. On échantillonne 64 × 64 avant d'analyser — la
/// page décode sa vignette à cette taille, et une analyse d'une pochette de
/// 1 000 px coûterait des millisecondes sur le fil de l'interface pour un
/// résultat identique à l'arrondi près.
#[derive(Default)]
pub struct Sleeve {
    path: Option<PathBuf>,
    item: Option<ImageCacheItem>,
    palette: Option<Palette>,
    /// Combien de décodages ce widget a demandés (mesuré par un test : un seul
    /// par titre, pas un par image).
    pub decodes: usize,
}

impl Sleeve {
    pub fn new() -> Self {
        Self::default()
    }

    /// La palette de la pochette `path`, ou `None` tant qu'elle est en route.
    ///
    /// Appelée depuis `prepaint` : la première image après un changement de
    /// titre démarre le décodage, les suivantes lisent la réponse. Jeter
    /// l'entrée quand le titre change libère l'image décodée (D9 a mesuré
    /// +291 MB pour vingt pochettes gardées).
    pub fn palette(
        &mut self,
        path: Option<&Path>,
        window: &Window,
        cx: &mut App,
    ) -> Option<Palette> {
        match path {
            None => {
                if self.path.is_some() || self.item.is_some() {
                    self.path = None;
                    self.item = None;
                    self.palette = None;
                }
                None
            }
            Some(path) => {
                if self.path.as_deref() != Some(path) {
                    self.path = Some(path.to_owned());
                    self.item = Some(ImageCacheItem::new(
                        &Resource::Path(path.to_owned().into()),
                        cx,
                    ));
                    self.palette = None;
                    self.decodes += 1;
                }
                if self.palette.is_none()
                    && let Some(Ok(image)) =
                        self.item.as_ref().and_then(|item| item.use_image(window))
                {
                    self.palette = image_palette(&image);
                }
                self.palette.clone()
            }
        }
    }

    /// La palette déjà analysée, sans toucher au disque.
    pub fn cached(&self) -> Option<&Palette> {
        self.palette.as_ref()
    }
}

/// La palette d'une image décodée par GPUI : BGRA, échantillonnée à 64 × 64.
pub fn image_palette(image: &gpui_kit::RenderImage) -> Option<Palette> {
    let bytes = image.as_bytes(0)?;
    let size = image.size(0);
    let (width, height) = (u32::from(size.width), u32::from(size.height));
    if width == 0 || height == 0 {
        return None;
    }
    let thumb = thumbnail_bgra(bytes, width, height, 64);
    palette::analyse(&thumb, 64, 64)
}

/// Réduit une image BGRA à `n × n` pixels RGBA par échantillonnage du centre de
/// chaque case. Le plus proche suffit : `analyse` regroupe en 3 × 3 cellules et
/// en 4 096 seaux, donc un lissage ne changerait aucun arrondi.
pub fn thumbnail_bgra(bytes: &[u8], width: u32, height: u32, n: u32) -> Vec<u8> {
    let n = n.max(1);
    let mut out = Vec::with_capacity((n * n * 4) as usize);
    let w = width.max(1);
    let h = height.max(1);
    for y in 0..n {
        for x in 0..n {
            let sx = ((x * w) / n).min(w - 1);
            let sy = ((y * h) / n).min(h - 1);
            let at = ((sy * w + sx) * 4) as usize;
            if at + 3 >= bytes.len() {
                out.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            // BGRA → RGBA, l'ordre que `coverpalette.js#analyse` lit.
            out.extend_from_slice(&[bytes[at + 2], bytes[at + 1], bytes[at], bytes[at + 3]]);
        }
    }
    out
}

/// Le coût des images d'une scène, et sa fréquence réelle.
///
/// `NZ_FRAME_LOG=1` allume la publication : toutes les 120 images, une ligne
/// `nz_frame_log viz=… fps=… paint_us p50=… p95=… max=…` sur la sortie
/// standard — la même forme que le journal des listes (`track_list.rs`), pour
/// que la mesure d'un écran et celle d'une liste se lisent pareil.
pub struct FrameLog {
    samples: Vec<f32>,
    enabled: bool,
    started: Instant,
    first: Option<Instant>,
    paints: u64,
}

impl Default for FrameLog {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameLog {
    pub fn new() -> Self {
        Self {
            samples: Vec::new(),
            enabled: std::env::var("NZ_FRAME_LOG").is_ok_and(|value| value != "0"),
            started: Instant::now(),
            first: None,
            paints: 0,
        }
    }

    pub fn note(&mut self, label: &str, micros: f32) {
        self.paints += 1;
        self.first.get_or_insert_with(Instant::now);
        if !self.enabled {
            return;
        }
        self.samples.push(micros);
        if self.samples.len().is_multiple_of(120) {
            let fps = self.fps();
            if let Some((p50, p95, max, n)) = self.summary() {
                println!(
                    "nz_frame_log viz={label} fps={fps:.1} paint_us p50={p50:.0} p95={p95:.0} \
                     max={max:.0} n={n}"
                );
            }
        }
    }

    /// Les images peintes depuis la naissance du widget.
    pub fn paints(&self) -> u64 {
        self.paints
    }

    /// La fréquence réelle, mesurée sur la première image et la dernière.
    pub fn fps(&self) -> f64 {
        let Some(first) = self.first else {
            return 0.;
        };
        let elapsed = first.elapsed().as_secs_f64().max(1e-6);
        // La première image compte : `paints - 1` intervalles sur `elapsed`
        // sous-estime d'une image, et c'est la borne basse qu'on veut.
        (self.paints.max(1) as f64 - 1.) / elapsed
    }

    pub fn summary(&self) -> Option<(f32, f32, f32, usize)> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let at = |q: f32| sorted[((sorted.len() - 1) as f32 * q) as usize];
        Some((
            at(0.5),
            at(0.95),
            *sorted.last().unwrap_or(&0.),
            sorted.len(),
        ))
    }

    /// Le temps écoulé depuis la première image, pour un débit moyen.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Repart de zéro : une scène qu'on quitte puis qu'on rouvre ne doit pas
    /// afficher la fréquence de la visite précédente.
    pub fn reset_clock(&mut self) {
        self.first = None;
        self.paints = 0;
        self.samples.clear();
    }
}

/// Ce qu'un écran qui affiche les réglages doit savoir faire : porter les
/// préférences et réagir à un changement. Le panneau du visualiseur et cette
/// section partagent donc les mêmes contrôles, une seule fois écrits.
pub trait Host: Render + Sized + 'static {
    fn viz_prefs(&self) -> &Prefs;

    fn viz_prefs_mut(&mut self) -> &mut Prefs;

    /// Appelé après chaque changement : l'hôte enregistre et repeint.
    fn viz_changed(&mut self, cx: &mut Context<Self>);
}

/// Les contrôles de la visualisation, dans l'ordre où on les règle.
pub fn controls<H: Host>(
    prefs: &Prefs,
    intensity: &Entity<SliderState>,
    window: &mut Window,
    cx: &mut Context<H>,
) -> gpui_kit::AnyElement {
    let mut scene = toggle_group("viz-mode", cx);
    for mode in Mode::ALL {
        let selected = prefs.mode == mode;
        scene = scene.child(
            toggle(
                format!("viz-mode-{}", mode.id()),
                mode.label(),
                selected,
                cx,
            )
            .debug_selector(move || format!("viz-mode-{}", mode.id()))
            .on_click(cx.listener(move |this: &mut H, _, _, cx| {
                this.viz_prefs_mut().mode = mode;
                this.viz_changed(cx);
            })),
        );
    }

    let mut palette = toggle_group("viz-palette", cx);
    for source in PaletteSource::ALL {
        let selected = prefs.palette == source;
        palette = palette.child(
            toggle(
                format!("viz-palette-{}", source.id()),
                source.label(),
                selected,
                cx,
            )
            .debug_selector(move || format!("viz-palette-{}", source.id()))
            .on_click(cx.listener(move |this: &mut H, _, _, cx| {
                this.viz_prefs_mut().palette = source;
                this.viz_changed(cx);
            })),
        );
    }

    let mut fps = toggle_group("viz-fps", cx);
    for cap in FpsCap::ALL {
        let selected = prefs.fps == cap;
        fps = fps.child(
            toggle(format!("viz-fps-{}", cap.0), cap.label(), selected, cx)
                .debug_selector(move || format!("viz-fps-{}", cap.0))
                .on_click(cx.listener(move |this: &mut H, _, _, cx| {
                    this.viz_prefs_mut().fps = cap;
                    this.viz_changed(cx);
                })),
        );
    }

    let mut orientation = toggle_group("viz-scope-orientation", cx);
    for axis in Orientation::ALL {
        let selected = prefs.orientation == axis;
        orientation = orientation.child(
            toggle(
                format!("viz-scope-{}", axis.id()),
                axis.label(),
                selected,
                cx,
            )
            .debug_selector(move || format!("viz-scope-{}", axis.id()))
            .on_click(cx.listener(move |this: &mut H, _, _, cx| {
                this.viz_prefs_mut().orientation = axis;
                this.viz_changed(cx);
            })),
        );
    }

    let mut tint = toggle_group("viz-scope-colour", cx);
    for colour in Tint::ALL {
        let selected = prefs.colour == colour;
        tint = tint.child(
            toggle(
                format!("viz-tint-{}", colour.id()),
                colour.label(),
                selected,
                cx,
            )
            .debug_selector(move || format!("viz-tint-{}", colour.id()))
            .on_click(cx.listener(move |this: &mut H, _, _, cx| {
                this.viz_prefs_mut().colour = colour;
                this.viz_changed(cx);
            })),
        );
    }

    let slider = gpui_omarchy::slider(intensity, false, window, cx);
    let percent = (prefs.intensity * 100.).round();

    div()
        .flex()
        .flex_col()
        .gap(rems(1.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(0.5))
                .child(card_title("Scène", cx))
                .child(hint(
                    "Deux animations, celles que le bureau dessine : le spectre en barres et \
                     l'oscilloscope. Les deux lisent la même analyse, au même rythme.",
                    cx,
                ))
                .child(scene),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(0.5))
                .child(card_title("Couleurs", cx))
                .child(hint(
                    "« Pochette » prend les couleurs de la jaquette du titre, zone par zone ; \
                     « Thème » prend l'accent et la surface d'Omarchy. Une pochette encore en \
                     route garde celles du thème.",
                    cx,
                ))
                .child(palette),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(0.5))
                .child(card_title("Images par seconde", cx))
                .child(hint(
                    "« Écran » suit la dalle (240 Hz ici). L'éco impose 30 images/s et le \
                     dessin plat, sans halo ni fantôme de crête.",
                    cx,
                ))
                .child(fps)
                .child(
                    div()
                        .text_size(rems(0.6875))
                        .font_family(theme::mono(cx))
                        .text_color(theme::secondary(cx))
                        .child(if prefs.eco {
                            "éco : 30 images/s".to_owned()
                        } else {
                            format!("{} images/s", prefs.effective_fps().effective())
                        }),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(0.5))
                .child(card_title("Intensité", cx))
                .child(hint(
                    "L'ampleur du mouvement, pas la luminosité : la hauteur atteinte par les \
                     barres et par la trace.",
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .child(slider)
                        .child(
                            div()
                                .w(rems(3.5))
                                .text_size(rems(0.6875))
                                .font_family(theme::mono(cx))
                                .child(format!("{percent} %")),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(0.5))
                .child(card_title("Oscilloscope", cx))
                .child(hint(
                    "Une trace par canal, déclenchée sur le grave comme un vrai oscilloscope : \
                     l'image tient en place au lieu de glisser. La précision est toujours \
                     maximale sur un ordinateur.",
                    cx,
                ))
                .child(orientation)
                .child(tint),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(0.5))
                .child(card_title("Mode éco", cx))
                .child(hint(
                    "Un plafond à 30 images/s et le dessin plat : une seule teinte par barre, \
                     pas de halo, pas de respiration. Les réglages restent enregistrés.",
                    cx,
                ))
                .child(
                    switch("viz-eco", "Économiser la batterie", prefs.eco, cx)
                        .debug_selector(|| "viz-eco".to_owned())
                        .on_click(cx.listener(|this: &mut H, _, _, cx| {
                            let eco = !this.viz_prefs().eco;
                            this.viz_prefs_mut().eco = eco;
                            this.viz_changed(cx);
                        })),
                ),
        )
        .into_any_element()
}

/// Réglages → Animations. Le même [`controls`] que le panneau du visualiseur,
/// plus un aperçu vivant de la scène choisie, piloté par le titre qui joue.
///
/// Cette section est la forme que `settings/mod.rs` dispatche pour un lot
/// livré (`Screen::new(section, state, cx)` + `body(screen, window, cx)`) ; le
/// panneau du visualiseur, lui, monte les mêmes contrôles dans une feuille.
pub struct Section {
    section: crate::nav::SettingsSection,
    state: Entity<AppState>,
    prefs: Prefs,
    intensity: Entity<SliderState>,
    bars: Entity<Bars>,
    scope: Entity<Scope>,
    _subscriptions: Vec<Subscription>,
}

impl Section {
    pub fn new(
        section: crate::nav::SettingsSection,
        state: Entity<AppState>,
        cx: &mut Context<Self>,
    ) -> Self {
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
        let mut subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&intensity, |this: &mut Self, _, event: &SliderEvent, cx| {
                let value = match event {
                    SliderEvent::Change(value) | SliderEvent::Release(value) => match value {
                        SliderValue::Single(value) => *value,
                        SliderValue::Range(start, _) => *start,
                    },
                };
                this.prefs.intensity = value.clamp(0., 1.);
                this.viz_changed(cx);
            }),
            // L'analyse s'arrête avec l'aperçu : sans visualiseur visible, le
            // fil `nz-analysis` ne fait rien (c'est son critère d'acceptation).
            cx.on_release(|this: &mut Self, cx| {
                this.state.update(cx, |state, _| {
                    if let Some(viz) = &state.viz {
                        viz.set_visible(false);
                    }
                });
            }),
        ];
        let this = Self {
            section,
            state,
            prefs,
            intensity,
            bars,
            scope,
            _subscriptions: std::mem::take(&mut subscriptions),
        };
        this.state.update(cx, |state, _| {
            if let Some(viz) = &state.viz {
                viz.set_visible(true);
            }
        });
        this
    }

    pub fn body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        self.state.update(cx, |state, _| {
            if let Some(viz) = &state.viz {
                viz.set_visible(true);
            }
        });
        let scene = match self.prefs.mode {
            Mode::Bars => self.bars.clone().into_any_element(),
            Mode::Scope => self.scope.clone().into_any_element(),
        };
        let controls = controls(&self.prefs, &self.intensity, window, cx);
        let playing = self.state.read(cx).player.playing();
        let has_analysis = self.state.read(cx).viz.is_some();
        div()
            .flex()
            .flex_col()
            .gap(rems(1.))
            .child(
                card("settings-viz-preview", cx)
                    .debug_selector(|| "settings-viz-preview".to_owned())
                    .child(card_title("Aperçu", cx))
                    .child(hint(
                        "La scène choisie, sur le titre qui joue. Elle suit les réglages pendant \
                         qu'on les bouge.",
                        cx,
                    ))
                    .child(
                        div()
                            .w_full()
                            .h(rems(14.))
                            .border_1()
                            .border_color(theme::border(cx))
                            .bg(theme::shell(cx))
                            .child(div().size_full().p(rems(0.75)).child(scene)),
                    )
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .text_color(theme::secondary(cx))
                            .child(if !has_analysis {
                                "Analyse indisponible : la sortie audio n'est pas ouverte. \
                                 Lancez un titre, l'aperçu s'anime."
                            } else if playing {
                                "En lecture : l'aperçu est l'analyse réelle du titre."
                            } else {
                                "En pause : l'aperçu reprend avec la lecture."
                            }),
                    ),
            )
            .child(card("settings-viz-controls", cx).child(controls))
            .into_any_element()
    }
}

impl Host for Section {
    fn viz_prefs(&self) -> &Prefs {
        &self.prefs
    }

    fn viz_prefs_mut(&mut self) -> &mut Prefs {
        &mut self.prefs
    }

    fn viz_changed(&mut self, cx: &mut Context<Self>) {
        let (prefs, cache) = (self.prefs, self.state.read(cx).cache.clone());
        prefs.save(cache.as_ref());
        if let Some(viz) = &self.state.read(cx).viz {
            viz.set_visible(true);
        }
        self.bars.update(cx, |bars, cx| bars.set_prefs(prefs, cx));
        self.scope
            .update(cx, |scope, cx| scope.set_prefs(prefs, cx));
        cx.notify();
    }
}

impl Render for Section {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.body(window, cx);
        div()
            .id("settings-viz")
            .debug_selector(|| "settings-viz".to_owned())
            .size_full()
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_viz::palette::Rgb;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neezer-d10b-viz-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn the_defaults_are_the_pages_saved_choices() {
        // `stores.js` : la scène en barres, la pochette, 0,7 d'intensité, deux
        // bandes, deux teintes. Une seule différence, voulue et mesurée : le
        // bureau vise la dalle (240 Hz), donc « Écran » et non 60.
        let prefs = Prefs::default();
        assert_eq!(prefs.mode, Mode::Bars);
        assert_eq!(prefs.palette, PaletteSource::Cover);
        assert!((prefs.intensity - 0.7).abs() < 1e-6);
        assert_eq!(prefs.fps, FpsCap(0));
        assert!(!prefs.eco);
        assert_eq!(prefs.orientation, Orientation::Horizontal);
        assert_eq!(prefs.colour, Tint::Duo);
    }

    #[test]
    fn the_saved_preferences_come_back_whole() {
        let dir = temp_dir("keys");
        let cache = Cache::open_with_cache_dir(&dir, &dir).expect("cache");
        let prefs = Prefs {
            mode: Mode::Scope,
            palette: PaletteSource::Theme,
            intensity: 0.35,
            fps: FpsCap(45),
            eco: true,
            orientation: Orientation::Vertical,
            colour: Tint::Sweep,
        };
        prefs.save(Some(&cache));
        let read = Prefs::load(Some(&cache));
        assert_eq!(read, prefs, "every key round-trips");
        // Les clés sont celles de la page, pas des nôtres.
        for key in [
            keys::MODE,
            keys::PALETTE,
            keys::INTENSITY,
            keys::FPS,
            keys::ECO,
            keys::SCOPE_ORIENTATION,
            keys::SCOPE_COLOUR,
        ] {
            assert!(
                cache.kv().get::<serde_json::Value>(key).is_some(),
                "{key} was not written"
            );
        }
        // Une valeur d'une autre version ne casse pas l'écran : elle retombe
        // sur le défaut, clé par clé.
        cache.kv().set(keys::MODE, &"aurora").unwrap();
        cache.kv().set(keys::FPS, &17u32).unwrap();
        let read = Prefs::load(Some(&cache));
        assert_eq!(read.mode, Mode::Bars);
        assert_eq!(read.fps, FpsCap::DISPLAY);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn eco_is_thirty_images_a_second_whatever_the_cap() {
        // Le critère du lot : « éco : plafond à 30 images/s ». Quel que soit le
        // réglage, l'éco gagne — sans l'écrire dans la clé, pour que le retour
        // à la normale retrouve exactement le choix précédent.
        let prefs = Prefs {
            fps: FpsCap(60),
            eco: true,
            ..Prefs::default()
        };
        assert_eq!(prefs.effective_fps(), FpsCap(30));
        assert_eq!(prefs.interval(), Some(Duration::from_secs_f64(1. / 30.)));
        let off = Prefs {
            fps: FpsCap(60),
            eco: false,
            ..Prefs::default()
        };
        assert_eq!(off.interval(), Some(Duration::from_secs_f64(1. / 60.)));
        assert_eq!(Prefs::default().interval(), None, "l'écran mène");
    }

    #[test]
    fn a_sleeve_thumbnail_is_rgba_at_the_pages_scale() {
        // Une image BGRA 2 × 2 : le coin haut-gauche rouge vif, le coin
        // haut-droit bleu, la moitié basse sombre. Après échantillonnage, les
        // pixels sont en RGBA (l'ordre que `analyse` lit) et les zones de la
        // pochette se retrouvent à leur place.
        let mut bgra = Vec::new();
        for y in 0..64u32 {
            for x in 0..64u32 {
                let (r, g, b) = if y >= 32 {
                    (10, 10, 10)
                } else if x < 32 {
                    (220, 20, 20)
                } else {
                    (20, 20, 220)
                };
                bgra.extend_from_slice(&[b as u8, g as u8, r as u8, 255]);
            }
        }
        let rgba = thumbnail_bgra(&bgra, 64, 64, 64);
        assert_eq!(rgba.len(), 64 * 64 * 4);
        assert_eq!(&rgba[0..4], &[220, 20, 20, 255], "BGRA → RGBA");
        let palette = palette::analyse(&rgba, 64, 64).expect("une palette");
        let zones = palette.zone_colours();
        println!("zones de la pochette de test : {zones:?}");
        assert!(zones[0][0] > zones[0][2], "la zone gauche est rouge");
        assert!(zones[2][2] > zones[2][0], "la zone droite est bleue");
    }

    #[test]
    fn the_bar_row_walks_the_sleeve_from_left_to_right() {
        let palette = Palette::from_theme([200, 40, 40], [18, 18, 20], false);
        let stops = zone_stops(&palette);
        assert_eq!(stops[0], [200, 40, 40]);
        assert_eq!(stops[6], [200, 40, 40]);
        let mixed: Rgb = mix([200, 0, 0], [0, 0, 200], 0.5);
        assert_eq!(mixed, [100, 0, 100]);
        // `colour_at` est continue : deux positions voisines ne sautent pas.
        let a = colour_at(&stops, 0.5);
        let b = colour_at(&stops, 0.51);
        assert!(a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 4));
    }

    #[test]
    fn the_theme_palette_is_the_strip_s_one_flat_colour() {
        // Sans pochette, les trois zones sont l'accent du thème : c'est ce que
        // la bande du lecteur dessine, et ce que `Palette::from_theme` promet.
        let palette = Palette::from_theme([240, 200, 90], [18, 18, 20], false);
        assert_eq!(zone_stops(&palette), [[240, 200, 90]; 7]);
        assert!(!palette.light);
    }

    #[test]
    fn the_cap_choices_are_the_pages_four() {
        assert_eq!(
            FpsCap::ALL.map(FpsCap::label),
            ["Écran", "30", "45", "60"],
            "les mêmes choix que `VisualizerSettings.svelte#FPS_CHOICES`"
        );
        assert_eq!(
            FpsCap::ALL[1].interval(),
            Some(Duration::from_secs_f64(1. / 30.))
        );
    }
}
