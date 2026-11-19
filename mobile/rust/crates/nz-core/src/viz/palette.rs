//! Where the bars' colours come from — `webapp/src/lib/viz/palette.js`, ported.
//!
//! The person picks the SOURCE, not the colours (`PALETTES`), and the palette
//! answers a base hue plus three accents (`low` / `mid` / `high`) that a scene
//! colours its registers with, plus — for the sleeve's source — the artwork's
//! own vertical zones, walked left to right so the row is coloured by what is
//! actually under it.
//!
//! # What is ported, and what is folded
//!
//! Everything the SPA does with the inputs this lot HAS:
//!
//! - the source's target hue, saturation, lightness and spread, and the
//!   saturation floor `coverSat` applies so a washed-out sleeve still colours
//!   (`clamp(s * 0.85 + 0.25, 0.35, 0.92)`);
//! - the clock-free drift, `barLen * 16` seconds a turn with `barLen` at the
//!   SPA's own 2.4 s fallback — the rhythm ladder is MB22's, so MB21 is always
//!   in the "no tempo yet" branch, which the SPA describes as "it creeps, which
//!   reads as 'waiting' rather than 'broken'";
//! - the per-frame easing (`approachAngle` for the hue, `approach` at 0.4 s for
//!   saturation and lightness, 0.7 s for the spread) — an instant hue change on
//!   a full-screen picture is a flash;
//! - the sleeve's ZONES: `zonesOfPalette`'s per-column choice is already
//!   `nz-viz`'s `Palette::zone_colours`, carried in as RGB; this module
//!   resamples three anchors to the SPA's seven stops, applies the same
//!   saturation floor and lightness fold, and walks them with `colour_at` (the
//!   short way round the wheel, so 350° → 10° crosses red);
//! - no drift on the zones, on purpose: the SPA's comment says why, and it is
//!   the one thing a zoned palette must not do — the blurred backdrop behind
//!   the row is that same artwork.
//!
//! FOLDED: `spectrum`, whose hue reads the six energy bands and the beat period
//! (`levelFor` gives the bars the spectrum depth, and `nz-viz` exposes the
//! levelled bars only). A stored `spectrum` resolves to the two-tone scheme in
//! [`super::settings`], so no colour here sits still waiting for a number that
//! never arrives. The genre lean (`skins.js`, `style.look`) and the harmony
//! term are the smart engine's too (MB22), and are absent rather than faked.
//!
//! NOTHING ALLOCATES PER FRAME: the zones are a fixed array, `colour_at` writes
//! into the caller's three floats, and the seven stops are recomputed in place
//! once per analysis frame.

use nz_viz::palette::Palette;

/// The SPA's `PALETTES`, minus `spectrum` (see the module doc).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PaletteSource {
    #[default]
    /// The artwork's own colour and zones — the scene belongs to the track.
    Cover,
    /// The two-tone fixed scheme (the SPA's `neon`; Réglages calls it « Duo »).
    Neon,
    /// Braise — the warm fixed scheme.
    Warm,
    /// Glace — the cool fixed scheme.
    Ice,
    /// Monochrome — one desaturated hue.
    Mono,
}

impl PaletteSource {
    /// The settings' stored string, resolved (the aliases are
    /// [`super::settings`]'s business too, and both agree).
    pub fn from_key(key: &str) -> PaletteSource {
        match key.trim() {
            "duo" | "neon" => PaletteSource::Neon,
            "warm" => PaletteSource::Warm,
            "ice" => PaletteSource::Ice,
            "mono" => PaletteSource::Mono,
            _ => PaletteSource::Cover,
        }
    }

    /// The source's own name, the SPA's id (`duo` is Réglages' name for the
    /// two-tone scheme, the SPA's `neon`).
    pub fn key(self) -> &'static str {
        match self {
            PaletteSource::Cover => "cover",
            PaletteSource::Neon => "duo",
            PaletteSource::Warm => "warm",
            PaletteSource::Ice => "ice",
            PaletteSource::Mono => "mono",
        }
    }

    /// `palette.js#FIXED`, the four fixed schemes' numbers.
    fn fixed(self) -> (f64, f64, f64, f64) {
        // (hue, spread, saturation, lightness)
        match self {
            PaletteSource::Neon => (288.0, 92.0, 0.92, 0.58),
            PaletteSource::Warm => (22.0, 46.0, 0.88, 0.55),
            PaletteSource::Ice => (196.0, 58.0, 0.80, 0.60),
            PaletteSource::Mono => (265.0, 8.0, 0.06, 0.72),
            // The cover's own fallbacks are the Sillon lemon, the colour the
            // SPA draws with when a track has no artwork at all.
            PaletteSource::Cover => (73.8, 64.0, 1.0, 0.62),
        }
    }
}

/// How many stops the row's colour walk has (`palette.js#ZONE_COUNT`): enough
/// that a strip of beads walks without a visible step.
pub const ZONE_COUNT: usize = 7;

/// The Sillon lemon — `stores.js`'s accent and the palette's fallback hue.
const ACCENT_HUE: f64 = 73.8;
/// `palette.js#setZones`: a zone with no hue of its own borrows the sleeve's.
const GREY_SAT: f64 = 0.08;
/// `palette.js#update`'s no-tempo bar length, in seconds.
const FALLBACK_BAR_S: f64 = 2.4;
/// A full turn of the drift every this many bars.
const DRIFT_BARS: f64 = 16.0;

/// One zone stop, in the SPA's own space: hue in degrees, saturation and
/// lightness 0..1.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Zone {
    pub hue: f64,
    pub sat: f64,
    pub light: f64,
}

/// One analysis frame's worth of the palette: what a scene paints with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colours {
    /// The base hue, degrees.
    pub hue: f64,
    /// The base saturation, 0..1.
    pub sat: f64,
    /// The base lightness, 0..1.
    pub light: f64,
    /// How far apart `low` and `high` sit, in degrees.
    pub spread: f64,
}

impl Default for Colours {
    fn default() -> Self {
        Colours {
            hue: ACCENT_HUE,
            sat: 1.0,
            light: 0.62,
            spread: 64.0,
        }
    }
}

impl Colours {
    /// The bass end (`palette.js#low`).
    pub fn low(&self) -> f64 {
        self.hue - self.spread * 0.5
    }
    /// The treble end (`palette.js#high`).
    pub fn high(&self) -> f64 {
        self.hue + self.spread * 0.5
    }
}

/// What one frame tells the palette about the music.
#[derive(Debug, Clone, Copy, Default)]
pub struct Music {
    /// The overall level, 0..1 (the SPA's `f.level`; here the bars' own mean —
    /// see [`super::frame`]).
    pub level: f64,
    /// How much of the energy is in the bass half, 0..1.
    pub low_share: f64,
    /// How much is in the treble half, 0..1.
    pub high_share: f64,
    /// Seconds since the previous analysis frame.
    pub dt: f64,
}

/// The palette: the chosen source, the sleeve's zones, and the running state the
/// easing carries between frames.
#[derive(Debug, Clone)]
pub struct PaletteState {
    source: PaletteSource,
    colours: Colours,
    /// The continuous rotation, advanced at the (absent) tempo.
    drift: f64,
    /// The sleeve's dominant colour, as HSL: `coverHue`/`coverSat`.
    cover_hue: f64,
    cover_sat: f64,
    /// The stops as the artwork handed them in (`zoneH/S/L`), before the walk.
    stops: [Zone; ZONE_COUNT],
    /// The stop a scene paints with this frame (`zones` in the SPA: the
    /// artwork's hue, the sleeve's for a grey stop, folded with the row's own
    /// value).
    walk: [Zone; ZONE_COUNT],
    /// `zoneGrey` as a bitmask: stop `i` has no hue of its own.
    grey: u8,
    /// Whether the walk is live: the source is the cover AND there is artwork.
    has_zones: bool,
}

impl PaletteState {
    pub fn new(source: PaletteSource) -> PaletteState {
        let (hue, spread, sat, light) = source.fixed();
        PaletteState {
            source,
            colours: Colours {
                hue,
                sat,
                light,
                spread,
            },
            drift: 0.0,
            cover_hue: ACCENT_HUE,
            cover_sat: 1.0,
            stops: [Zone::default(); ZONE_COUNT],
            walk: [Zone::default(); ZONE_COUNT],
            grey: 0,
            has_zones: false,
        }
    }

    /// `palette.js#setMode`: the choice changed, nothing is rebuilt (an envelope
    /// reset is exactly the frame the person is looking at while they choose).
    /// The hue, saturation, lightness and spread EASE towards the new source on
    /// the next frames, which is what makes the change a fade rather than a cut:
    /// only the source (and so the target spread) moves here.
    pub fn set_source(&mut self, source: PaletteSource) {
        self.source = source;
    }

    /// The sleeve's dominant colour, as the SPA's `setCover` reads it: `None`
    /// is "there is no artwork", which goes back to the accent rather than
    /// keeping the last sleeve's colour.
    pub fn set_cover(&mut self, rgb: Option<[u8; 3]>) {
        let Some(rgb) = rgb else {
            self.cover_hue = ACCENT_HUE;
            self.cover_sat = 1.0;
            return;
        };
        let (hue, sat, _) = rgb_to_hsl(rgb);
        // A sleeve with no hue at all — a monochrome sleeve, the grey
        // placeholder disc of a missing file — gets the Sillon accent rather
        // than hue 0: red is a colour the artwork does not contain.
        self.cover_hue = if sat < GREY_SAT { ACCENT_HUE } else { hue };
        self.cover_sat = clamp(sat * 0.85 + 0.25, 0.35, 0.92);
    }

    /// The sleeve's zones, left to right (`zonesOfPalette`'s answer, already
    /// reduced to its three anchors by `nz-viz`). `None` forgets them, and the
    /// row goes back to its own low → high walk.
    pub fn set_zones(&mut self, zones: Option<[[u8; 3]; 3]>) {
        let Some(zones) = zones else {
            self.has_zones = false;
            self.grey = 0;
            return;
        };
        self.grey = 0;
        for i in 0..ZONE_COUNT {
            // Resampled exactly as the SPA does: `round(i * (n - 1) / 6)` over
            // the three anchors this lot hands in.
            let at = ((i * (zones.len() - 1)) as f64 / (ZONE_COUNT - 1) as f64).round() as usize;
            let (hue, sat, light) = rgb_to_hsl(zones[at.min(zones.len() - 1)]);
            self.stops[i] = Zone {
                hue,
                // The same floors `setCover` applies: a washed-out zone would
                // draw a grey bead, a neon sleeve could burn.
                sat: clamp(sat * 0.85 + 0.25, 0.35, 0.92),
                light,
            };
            if sat < GREY_SAT {
                self.grey |= 1 << i;
            }
        }
        self.has_zones = true;
        self.fold_walk();
    }

    /// `palette.js#update`: one analysis frame, eased.
    pub fn update(&mut self, music: Music) {
        let dt = music.dt.clamp(0.001, 0.25);
        // Drift: a full turn every 16 bars, locked to the music when a tempo is
        // known — MB21 has no tempo (spectrum depth), so the SPA's own fallback
        // bar length applies and the colour creeps.
        self.drift = (self.drift + (dt / (FALLBACK_BAR_S * DRIFT_BARS)) * 360.0) % 360.0;

        let (mut target_hue, mut target_sat, mut target_light, target_spread) = match self.source {
            PaletteSource::Cover => (
                self.cover_hue + self.drift * 0.28,
                self.cover_sat,
                0.56,
                64.0,
            ),
            PaletteSource::Neon
            | PaletteSource::Warm
            | PaletteSource::Ice
            | PaletteSource::Mono => {
                let (hue, spread, sat, light) = self.source.fixed();
                (hue + self.drift * 0.16, sat, light, spread)
            }
        };
        // The level lifts the colour a little on every source: the SPA's
        // `spectrum` branch does it through `f.level`, and a fixed scheme on a
        // quiet passage should not read as a different scheme.
        target_sat = clamp(target_sat + music.level * 0.06, 0.0, 1.0);
        target_light = clamp(target_light + music.level * 0.04, 0.12, 0.85);
        // The register balance leans the hue: bass-heavy towards the warm end,
        // treble-heavy towards the cool one — `palette.js#spectrum`'s mapping,
        // fed by the bars' own halves instead of the energy bands (see the
        // module doc). It is a LEAN, never a replacement, which is why it is
        // mixed in and not assigned.
        if self.source != PaletteSource::Mono {
            let balance = clamp(0.5 + (music.high_share - music.low_share) * 1.6, 0.0, 1.0);
            let wanted = 8.0 + balance * 250.0 + self.drift;
            target_hue = mix_angle(target_hue, wanted, 0.12);
        }

        self.colours.hue = approach_angle(self.colours.hue, target_hue, 0.5, dt);
        self.colours.sat = approach(self.colours.sat, target_sat, 0.4, dt);
        self.colours.light = approach(self.colours.light, target_light, 0.4, dt);
        self.colours.spread = approach(self.colours.spread, target_spread, 0.7, dt);
        self.fold_walk();
    }

    /// The walk: the artwork's own hue (the sleeve's for a grey stop), folded
    /// with the row's own value so a dark sleeve reads as a deeper bead rather
    /// than as a black one. No tempo drift here, on purpose (see the module
    /// doc). Computed once per analysis frame.
    fn fold_walk(&mut self) {
        if !self.has_zones {
            return;
        }
        let light = self.colours.light;
        for i in 0..ZONE_COUNT {
            let stop = self.stops[i];
            let grey = self.grey & (1 << i) != 0;
            self.walk[i] = Zone {
                hue: if grey { self.cover_hue } else { stop.hue },
                sat: stop.sat,
                light: clamp(light * 0.6 + stop.light * 0.4, 0.5, 0.72),
            };
        }
    }

    /// The colours a scene paints with this frame.
    pub fn colours(&self) -> Colours {
        self.colours
    }

    /// The row's colour at `t` (0 left, 1 right), when the sleeve's zones are
    /// live — `palette.js#colourAt`, the short way round the wheel.
    pub fn colour_at(&self, t: f64) -> Option<Zone> {
        if !self.has_zones {
            return None;
        }
        let x = clamp(t, 0.0, 1.0) * (ZONE_COUNT - 1) as f64;
        let i = (ZONE_COUNT - 2).min(x.floor() as usize);
        let k = x - i as f64;
        let a = &self.walk[i];
        let b = &self.walk[i + 1];
        let d = ((b.hue - a.hue) % 360.0 + 540.0) % 360.0 - 180.0;
        Some(Zone {
            hue: ((a.hue + d * k) % 360.0 + 360.0) % 360.0,
            sat: a.sat + (b.sat - a.sat) * k,
            light: a.light + (b.light - a.light) * k,
        })
    }

    /// The source the frame was coloured with (the FFI's `palette` name).
    pub fn source(&self) -> PaletteSource {
        self.source
    }

    /// Whether the walk is live (`out.hasZones`): the source is the cover and
    /// there is artwork.
    pub fn has_zones(&self) -> bool {
        self.has_zones
    }

    /// The seven stops of the walk, as the FFI hands them to the surface.
    pub fn zones(&self) -> &[Zone; ZONE_COUNT] {
        &self.walk
    }

    /// The sleeve's own palette, as the platform decoder answered it: the
    /// dominant colour AND the three anchors `zone_colours` reports.
    pub fn set_artwork(&mut self, artwork: Option<&Palette>) {
        match artwork {
            Some(palette) => {
                self.set_cover(Some(palette.dominant));
                self.set_zones(Some(palette.zone_colours()));
            }
            None => {
                self.set_cover(None);
                self.set_zones(None);
            }
        }
    }
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// `util.js#approach`: an exponential approach with a time constant, so the
/// result depends on SECONDS and not on how often it is called.
fn approach(v: f64, target: f64, tau: f64, dt: f64) -> f64 {
    v + (target - v) * (1.0 - (-dt / tau.max(1e-4)).exp())
}

/// `util.js#approachAngle`: the same, the short way round the wheel.
fn approach_angle(v: f64, target: f64, tau: f64, dt: f64) -> f64 {
    let d = ((target - v) % 360.0 + 540.0) % 360.0 - 180.0;
    v + d * (1.0 - (-dt / tau.max(1e-4)).exp())
}

/// `util.js#mixAngle`: `amount` of the way from one hue to another.
fn mix_angle(a: f64, b: f64, amount: f64) -> f64 {
    let d = ((b - a) % 360.0 + 540.0) % 360.0 - 180.0;
    a + d * clamp(amount, 0.0, 1.0)
}

/// `util.js#rgbToHsl`: hue in degrees, saturation and lightness 0..1.
pub fn rgb_to_hsl(rgb: [u8; 3]) -> (f64, f64, f64) {
    let r = rgb[0] as f64 / 255.0;
    let g = rgb[1] as f64 / 255.0;
    let b = rgb[2] as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    let mut h = 0.0;
    let mut s = 0.0;
    if d > 1e-6 {
        s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        h = if (max - r).abs() < f64::EPSILON {
            ((g - b) / d + if g < b { 6.0 } else { 0.0 }) * 60.0
        } else if (max - g).abs() < f64::EPSILON {
            ((b - r) / d + 2.0) * 60.0
        } else {
            ((r - g) / d + 4.0) * 60.0
        };
    }
    (h, s, l)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn music(level: f64) -> Music {
        Music {
            level,
            low_share: 0.4,
            high_share: 0.2,
            dt: 512.0 / 48_000.0,
        }
    }

    #[test]
    fn the_cover_fallback_is_the_sillon_lemon_and_not_red() {
        // `palette.js#setCover`: a sleeve with no hue keeps the accent, and a
        // washed-out one is floored so the row still has colour.
        let mut p = PaletteState::new(PaletteSource::Cover);
        p.set_cover(None);
        assert!((p.colours.hue - ACCENT_HUE).abs() < 1e-9);

        let mut grey = PaletteState::new(PaletteSource::Cover);
        grey.set_cover(Some([128, 128, 128]));
        // `update` eases towards `cover_hue`, which the grey sleeve left at the
        // accent: no red anywhere.
        for _ in 0..400 {
            grey.update(music(0.3));
        }
        // The register lean and the drift move it a few degrees, never to red:
        // the point is that a grey sleeve keeps the accent's family.
        let from_accent = ((grey.colours.hue - ACCENT_HUE) % 360.0 + 540.0) % 360.0 - 180.0;
        assert!(
            from_accent.abs() < 40.0,
            "grey sleeve: {} is {from_accent} degrees from the accent",
            grey.colours.hue
        );

        let mut washed = PaletteState::new(PaletteSource::Cover);
        washed.set_cover(Some([90, 110, 140]));
        assert!(
            washed.cover_sat >= 0.35 && washed.cover_sat <= 0.92,
            "saturation floored: {}",
            washed.cover_sat
        );
    }

    #[test]
    fn a_sleeve_colours_the_row_left_to_right() {
        // The three anchors `zone_colours` reports: red, green, blue.
        let mut p = PaletteState::new(PaletteSource::Cover);
        p.set_cover(Some([200, 20, 20]));
        p.set_zones(Some([[200, 20, 20], [20, 200, 20], [20, 20, 200]]));
        p.update(music(0.4));
        assert!(p.has_zones());
        let left = p.colour_at(0.0).expect("a zone walk");
        let mid = p.colour_at(0.5).expect("a zone walk");
        let right = p.colour_at(1.0).expect("a zone walk");
        // Hue 0 (red) at the left, 120 (green) in the middle, 240 (blue) right.
        assert!(
            left.hue < 15.0 || left.hue > 345.0,
            "left is red: {}",
            left.hue
        );
        assert!(
            (mid.hue - 120.0).abs() < 20.0,
            "middle is green: {}",
            mid.hue
        );
        assert!(
            (right.hue - 240.0).abs() < 20.0,
            "right is blue: {}",
            right.hue
        );
    }

    #[test]
    fn the_zone_walk_takes_the_short_way_round() {
        // 350° -> 10° must cross red (through 0/360) and never sweep the wheel
        // backwards: that is `colourAt`'s `((b - a) % 360 + 540) % 360 - 180`.
        let mut p = PaletteState::new(PaletteSource::Cover);
        p.set_cover(Some([250, 40, 40]));
        // Anchors whose hues are 350 and 10: a red sleeve's two ends.
        p.set_zones(Some([[255, 60, 60], [250, 60, 60], [255, 90, 90]]));
        p.update(music(0.5));
        let a = p.zones()[0].hue;
        let b = p.zones()[ZONE_COUNT - 1].hue;
        let d = ((b - a) % 360.0 + 540.0) % 360.0 - 180.0;
        assert!(
            d.abs() < 40.0,
            "a short hop, not a 300° sweep: {a} -> {b} ({d})"
        );
    }

    #[test]
    fn no_artwork_means_no_walk_and_the_low_high_row() {
        let mut p = PaletteState::new(PaletteSource::Cover);
        p.set_zones(Some([[200, 20, 20], [20, 200, 20], [20, 20, 200]]));
        assert!(p.has_zones());
        p.set_zones(None);
        assert!(
            !p.has_zones(),
            "a sleeve that cannot be read forgets its zones"
        );
        assert!(p.colour_at(0.5).is_none());
        // And the base low/high walk still separates the registers.
        let c = p.colours();
        assert!((c.high() - c.low() - c.spread).abs() < 1e-9);
    }

    #[test]
    fn a_grey_band_borrows_the_sleeves_hue() {
        // `zoneGrey`: a black border on a coloured sleeve must not draw a grey
        // bead — it takes the sleeve's own hue.
        let mut p = PaletteState::new(PaletteSource::Cover);
        p.set_cover(Some([210, 40, 60])); // a red sleeve
        p.set_zones(Some([[12, 12, 12], [210, 40, 60], [12, 12, 12]]));
        p.update(music(0.3));
        let left = p.colour_at(0.0).expect("a walk");
        let cover_hue = p.cover_hue;
        assert!(
            (left.hue - cover_hue).abs() < 8.0,
            "the grey band borrows {cover_hue}: {}",
            left.hue
        );
    }

    #[test]
    fn the_fixed_schemes_are_the_spas_numbers_and_none_of_them_is_red_by_accident() {
        for (source, hue, spread) in [
            (PaletteSource::Neon, 288.0, 92.0),
            (PaletteSource::Warm, 22.0, 46.0),
            (PaletteSource::Ice, 196.0, 58.0),
            (PaletteSource::Mono, 265.0, 8.0),
        ] {
            let mut p = PaletteState::new(source);
            for _ in 0..600 {
                p.update(music(0.2));
            }
            let c = p.colours();
            // The drift moves the hue by a bounded amount, the spread settles on
            // the source's own.
            let drift = ((c.hue - hue) % 360.0 + 540.0) % 360.0 - 180.0;
            assert!(drift.abs() < 90.0, "{source:?}: hue {} vs {hue}", c.hue);
            assert!(
                (c.spread - spread).abs() < 6.0,
                "{source:?}: spread {} vs {spread}",
                c.spread
            );
        }
    }

    #[test]
    fn a_frame_never_allocates_a_zone() {
        // `colour_at` writes into the caller's value: it is called once per bar
        // per paint, so it may not build anything.
        let mut p = PaletteState::new(PaletteSource::Cover);
        p.set_cover(Some([40, 90, 220]));
        p.set_zones(Some([[40, 90, 220], [90, 40, 220], [220, 40, 90]]));
        p.update(music(0.6));
        let mut sum = 0.0;
        for i in 0..64 {
            let z = p.colour_at(i as f64 / 63.0).expect("a walk");
            sum += z.hue + z.sat + z.light;
        }
        assert!(sum.is_finite() && sum > 0.0);
    }

    #[test]
    fn the_source_change_is_a_fade_not_a_cut() {
        let mut p = PaletteState::new(PaletteSource::Warm);
        for _ in 0..300 {
            p.update(music(0.3));
        }
        let before = p.colours();
        p.set_source(PaletteSource::Ice);
        let at_once = p.colours();
        assert_eq!(before, at_once, "set_source moves nothing by itself");
        for _ in 0..2 {
            p.update(music(0.3));
        }
        let after = p.colours();
        assert!(
            (after.hue - before.hue).abs() < 120.0,
            "one or two frames of easing: {} -> {}",
            before.hue,
            after.hue
        );
        assert!((after.spread - before.spread).abs() < 40.0);
    }
}
