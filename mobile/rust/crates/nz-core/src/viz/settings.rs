//! The visualiser's settings: what the person chose, resolved, plus the two
//! rules that decide what actually runs.
//!
//! The keys are `stores.js`'s own and they live in the device's `kv` store,
//! written by Réglages (MB16) through the same whitelist
//! (`nz-core::settings::local`): this module never invents a key, it reads the
//! ones the store already validates and turns a stored string into a value.
//!
//! THE TWO RULES ARE THE ACCEPTANCE'S. **Eco means none** (`stores.js#ecoMode`:
//! "not a smaller one, not a slower one — none"), so eco resolves the mode to
//! `off` and caps the frame rate at 30 — the number the note pins for the eco
//! grade, kept here so the Kotlin side has one place to ask. And **the FPS cap
//! is a ceiling on PAINTING, never on analysis**: the analysis runs at the
//! analyser's own cadence (~94 Hz at 48 kHz) whatever the cap, so a beat or an
//! onset is never missed because the screen was capped to 30 (`Visualizer.svelte`
//! says so: "a scene never misses an event and never paints more often than the
//! screen (or the user) wants").
//!
//! # The vocabulary, and where it comes from
//!
//! `viz.mode` is `bars` or `scope`: those are the two modes MB21 owns, and the
//! two Réglages offers (`settings::local::VIZ_MODES`). `smart`, `pulse`,
//! `aurora` and the worlds are MB22's, and a value stored by a browser that had
//! them resolves here to the nearest mode this lot can draw — never to a dead
//! canvas.
//!
//! `viz.palette` is the SPA's `lib/viz/palette.js#PALETTES` MINUS `spectrum`,
//! whose hue needs the raw energy bands and a locked tempo (the rhythm ladder);
//! MB21 draws at the spectrum depth (`modes.js#levelFor("bars")`), so the two
//! sources it can colour with are the sleeve's zones and the fixed schemes. A
//! stored `spectrum` therefore resolves to the two-tone scheme rather than to a
//! colour that would never move. Réglages offers three of them (Pochette, Duo,
//! Monochrome); the other fixed schemes are read too, so a value written by a
//! browser or by a later screen is honoured instead of reset.

use crate::core::{Core, CoreError};
use crate::settings::LocalSettings;

/// The modes this lot draws, in the order the segmented control shows them.
pub const MODES: &[&str] = &["bars", "scope"];

/// `viz.palette` values, the SPA's ids plus the alias Réglages uses for the
/// two-tone scheme (`duo`, the SPA's `neon`: one hue with a wide spread, the
/// palette's two ends).
pub const PALETTES: &[&str] = &["cover", "duo", "mono", "neon", "warm", "ice"];

/// The oscilloscope's two knobs (`lib/viz/scenes/scope.js#SCOPE_ORIENTATIONS`
/// and `#SCOPE_COLOURS`); `sweep` is the third colour mode the SPA has.
pub const SCOPE_ORIENTATIONS: &[&str] = &["horizontal", "vertical"];
/// How the two channels are coloured.
pub const SCOPE_COLOURS: &[&str] = &["duo", "mono", "sweep"];

/// The caps `stores.js#vizFps` offers, and the eco grade's own.
pub const FPS_CHOICES: &[u32] = &[30, 60];
/// What eco caps the picture at, wherever it is drawn (`animations.md`: the eco
/// grade is 30 fps and no post pass).
pub const ECO_FPS: u32 = 30;

/// The resolved settings: every field a real value, the SPA's default when the
/// person never chose one.
#[derive(Debug, Clone, PartialEq)]
pub struct VizSettings {
    /// `viz.eco` — the switch that means NO animation anywhere.
    pub eco: bool,
    /// `viz.mode` as stored (`bars`, `scope`, or a value another build wrote).
    pub chosen_mode: String,
    /// What to draw after eco and the vocabulary resolution: `bars`, `scope` or
    /// `off`.
    pub mode: String,
    /// `viz.palette`, resolved to one of [`PALETTES`].
    pub palette: String,
    /// `viz.intensity`, 0..1.
    pub intensity: f64,
    /// `viz.fps` as stored (30 or 60).
    pub chosen_fps: u32,
    /// What the surface may paint at, after eco.
    pub fps: u32,
    /// `viz.beat` — the tempo detector feeds the visualiser. READ here and
    /// consumed by the rhythm modes (MB22): the bars and the scope need the
    /// spectrum only, which is exactly the SPA's `levelFor`.
    pub beat_detect: bool,
    /// `viz.scope.orientation`, resolved.
    pub scope_orientation: String,
    /// `viz.scope.colour`, resolved.
    pub scope_colour: String,
}

impl Default for VizSettings {
    /// `stores.js`'s own initial values, field for field.
    fn default() -> Self {
        VizSettings {
            eco: false,
            chosen_mode: "bars".to_owned(),
            mode: "bars".to_owned(),
            palette: "cover".to_owned(),
            intensity: 0.7,
            chosen_fps: 60,
            fps: 60,
            beat_detect: true,
            scope_orientation: "horizontal".to_owned(),
            scope_colour: "duo".to_owned(),
        }
    }
}

impl VizSettings {
    /// Reads every key once from the device's store (disk only, no request —
    /// `LocalSettings::read` is one indexed `SELECT` per key).
    pub fn read(core: &Core) -> VizSettings {
        VizSettings::from_local(&core.local_settings())
    }

    /// The visualiser's own fields out of the whole local settings record.
    pub fn from_local(local: &LocalSettings) -> VizSettings {
        let chosen_mode = resolve(
            &local.viz_mode,
            MODES,
            // A mode another build wrote (`smart`, `pulse`, `aurora`) is a mode
            // this screen cannot draw: the nearest one it can is the bars.
            "bars",
        );
        let palette = resolve_palette(&local.viz_palette);
        let chosen_fps = if FPS_CHOICES.contains(&local.viz_fps) {
            local.viz_fps
        } else {
            VizSettings::default().chosen_fps
        };
        let mut settings = VizSettings {
            eco: local.eco,
            chosen_mode: chosen_mode.clone(),
            mode: chosen_mode,
            palette,
            intensity: local.viz_intensity.clamp(0.0, 1.0),
            chosen_fps,
            fps: chosen_fps,
            beat_detect: local.viz_beat,
            scope_orientation: resolve(
                &local.viz_scope_orientation,
                SCOPE_ORIENTATIONS,
                "horizontal",
            ),
            scope_colour: resolve(&local.viz_scope_colour, SCOPE_COLOURS, "duo"),
        };
        settings.settle();
        settings
    }

    /// Applies the two rules: eco means NO animation (mode `off`) and caps the
    /// picture at [`ECO_FPS`]. Idempotent, so it is safe after every change.
    pub fn settle(&mut self) {
        self.mode = if self.eco {
            "off".to_owned()
        } else {
            self.chosen_mode.clone()
        };
        self.fps = if self.eco {
            self.chosen_fps.min(ECO_FPS)
        } else {
            self.chosen_fps
        };
    }

    /// Whether anything is drawn at all — the condition the analysis thread and
    /// the tap subscription both hang on.
    pub fn draws(&self) -> bool {
        self.mode != "off"
    }

    /// Whether this mode needs the raw waveform (`modes.js#needsWave`).
    pub fn needs_wave(&self) -> bool {
        self.mode == "scope"
    }

    /// The palette source, as the palette module's own enum.
    pub fn palette_source(&self) -> super::palette::PaletteSource {
        super::palette::PaletteSource::from_key(&self.palette)
    }

    /// Writes one key through the core's validating store, then re-reads the
    /// resolved settings. Optimistic by construction: the store is local, so
    /// there is nothing to roll back — a refused value is the only failure and
    /// it changes nothing.
    pub fn write(
        core: &Core,
        key: &str,
        value: serde_json::Value,
    ) -> Result<VizSettings, CoreError> {
        core.set_local_setting(key, value)?;
        Ok(VizSettings::read(core))
    }
}

/// A stored string, or the default when it names something this lot does not
/// know — one rule for every open vocabulary here.
fn resolve(stored: &str, known: &[&str], fallback: &str) -> String {
    let trimmed = stored.trim();
    if known.contains(&trimmed) {
        trimmed.to_owned()
    } else {
        fallback.to_owned()
    }
}

/// `viz.palette`, with `spectrum` folded onto the two-tone scheme (see the
/// module doc: its hue needs the rhythm ladder, which MB21 does not run).
fn resolve_palette(stored: &str) -> String {
    match stored.trim() {
        "cover" => "cover".to_owned(),
        "mono" => "mono".to_owned(),
        "neon" | "duo" => "duo".to_owned(),
        // The one source whose hue reads the six energy bands and a locked
        // tempo: folded onto the two-tone scheme, never left as a colour that
        // could not move.
        "spectrum" => "duo".to_owned(),
        "warm" => "warm".to_owned(),
        "ice" => "ice".to_owned(),
        _ => "cover".to_owned(),
    }
}

// No path ever arrives here: the visualiser asks `Screens::cover_path`, never a
// URL, so rule 3's "paths never come from the server" holds by construction.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_spas_own() {
        let s = VizSettings::default();
        assert_eq!(s.chosen_mode, "bars");
        assert_eq!(s.palette, "cover");
        assert!((s.intensity - 0.7).abs() < 1e-9);
        assert_eq!(s.chosen_fps, 60);
        assert_eq!(s.scope_orientation, "horizontal");
        assert_eq!(s.scope_colour, "duo");
        assert!(s.beat_detect);
        assert!(s.draws());
    }

    #[test]
    fn eco_means_none_and_caps_at_thirty() {
        // `stores.js#ecoMode` is not "a smaller animation": it is none, and the
        // note's eco grade is 30 fps.
        let mut s = VizSettings {
            eco: true,
            ..VizSettings::default()
        };
        s.settle();
        assert_eq!(s.mode, "off");
        assert!(!s.draws());
        assert_eq!(s.fps, 30);
        // And the chosen values survive, so turning eco off restores them.
        assert_eq!(s.chosen_mode, "bars");
        assert_eq!(s.chosen_fps, 60);
    }

    #[test]
    fn a_mode_this_lot_cannot_draw_falls_back_to_the_bars() {
        let local = LocalSettings {
            viz_mode: "smart".to_owned(), // MB22's
            ..LocalSettings::default()
        };
        let s = VizSettings::from_local(&local);
        assert_eq!(s.chosen_mode, "bars");
        assert_eq!(s.mode, "bars");
    }

    #[test]
    fn a_spectrum_palette_resolves_to_the_two_tone_scheme() {
        // The only source whose hue needs the rhythm ladder: folded, never left
        // as a colour that would not move.
        let local = LocalSettings {
            viz_palette: "spectrum".to_owned(),
            ..LocalSettings::default()
        };
        assert_eq!(VizSettings::from_local(&local).palette, "duo");
        let duo = LocalSettings {
            viz_palette: "duo".to_owned(),
            ..LocalSettings::default()
        };
        assert_eq!(VizSettings::from_local(&duo).palette, "duo");
        assert_eq!(
            VizSettings::from_local(&duo).palette_source(),
            super::super::palette::PaletteSource::Neon
        );
    }

    #[test]
    fn an_unknown_orientation_or_colour_is_the_default_not_a_blank_scope() {
        let local = LocalSettings {
            viz_scope_orientation: "diagonal".to_owned(),
            viz_scope_colour: "rainbow".to_owned(),
            ..LocalSettings::default()
        };
        let s = VizSettings::from_local(&local);
        assert_eq!(s.scope_orientation, "horizontal");
        assert_eq!(s.scope_colour, "duo");
    }

    #[test]
    fn the_scope_is_the_mode_that_needs_the_waveform() {
        let bars = VizSettings::default();
        assert!(!bars.needs_wave());
        let scope = VizSettings {
            chosen_mode: "scope".to_owned(),
            mode: "scope".to_owned(),
            ..VizSettings::default()
        };
        assert!(scope.needs_wave(), "modes.js#needsWave");
    }
}
