//! The visualiser's surface: one analysed frame, the settings, and the two
//! counters the acceptance reads.
//!
//! Owned by **MB21**; the worlds' renderer is MB22's `nz-viz-gl` and
//! `worlds.rs`. The note pins `VizApi { set_visible(view, bool), frame() ->
//! VizFrame /* bars heights, scope samples, palette */ }` (docs/agents/mobile.md,
//! "Screen lots, wave 3"), so MB0 declared exactly that and this file fills it.
//!
//! # What crosses, and why in this shape
//!
//! The Kotlin side DRAWS and does not compute: `frame()` hands over the bar
//! levels and peaks, the two scope traces, and the palette's own seven HSL
//! stops — the colour language `lib/viz/scenes/bars.js` paints with. RGB, the
//! capsule geometry and the capsule gradient are the surface's (`Canvas` draws
//! them), because that is what the SPA's scene does and because a cold read of
//! a 4.5 kB record per paint is the whole cost of the boundary.
//!
//! The record GREW past MB0's three fields, exactly as MB3b's `TrackRef` did:
//! a `Vec<f32>` of heights alone cannot draw the afterglow, the oscilloscope
//! needs one `[lo, hi]` PAIR per column per channel (that is what `viz_scope`
//! computes, and collapsing it to one number would invent a picture), and the
//! palette needs its numbers and not its name. `palette` stays (the resolved
//! source's name) so the value MB0 named still exists for MB22 and the
//! diagnostics.
//!
//! # The rule of this file
//!
//! Nothing here wakes an analyser: `set_visible` is called from a view's start
//! and dispose, `frame()` reads a published `Arc`, and every setter is a local
//! write (the device's `kv` store). `stats()` exposes the counters the
//! acceptance measures — **0 analysis tick while no view is visible** is
//! `stats().ticks` before and after a hidden stretch, and `subscribes` is 0
//! whenever nothing draws (MB12's tap is refcounted and Drop unsubscribes).

use std::sync::{Arc, OnceLock};

use nz_core::core::Core;
use nz_core::viz::{self as core_viz, Viz};

use crate::errors::NzError;

/// One analysed frame, as the bars and the oscilloscope draw it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct VizFrame {
    /// A level per capsule, 0..1, left (bass) to right (treble).
    pub bars: Vec<f32>,
    /// The slow-falling peak per capsule (the afterglow), same shape.
    pub peaks: Vec<f32>,
    /// Channel A (the left): one `[lo, hi]` pair per column.
    pub scope_a: Vec<f32>,
    /// Channel B (the right), the same.
    pub scope_b: Vec<f32>,
    /// The palette's base hue, in degrees.
    pub hue: f64,
    /// The palette's base saturation, 0..1.
    pub sat: f64,
    /// The palette's base lightness, 0..1.
    pub light: f64,
    /// How far apart the bass and treble ends sit, in degrees.
    pub spread: f64,
    /// The sleeve's walk: seven stops × (hue, saturation, lightness), HSL.
    pub zones: Vec<f32>,
    /// Whether the walk is live (the source is the cover AND there is artwork).
    pub has_zones: bool,
    /// The frame's loudness, 0..1.
    pub level: f32,
    /// The scope's auto-range, for the graticule.
    pub gain: f32,
    /// How solidly the scope's trigger is locked, 0..1 (the marker's alpha).
    pub locked: f32,
    /// Whether there is a signal on the probe at all.
    pub signal: bool,
    /// A beat landed on this frame (the rhythm ladder's — MB22 fills it).
    pub beat: bool,
    /// The tempo estimate, 0 when unknown (MB22's too).
    pub bpm: f32,
    /// The resolved palette source's name (`cover`, `duo`, `mono`, …).
    pub palette: String,
}

/// What the person chose, resolved — the surface's inputs.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct VizSettingsData {
    /// What to draw: `bars`, `scope`, or `off` (eco means none).
    pub mode: String,
    /// What was chosen, before eco (`bars` or `scope`).
    pub chosen_mode: String,
    /// The palette source: `cover`, `duo`, `mono`, `warm` or `ice`.
    pub palette: String,
    /// 0..1.
    pub intensity: f64,
    /// What the surface may paint at: 30 or 60 (eco caps at 30).
    pub fps: u32,
    /// The eco switch: no animation anywhere.
    pub eco: bool,
    /// Whether this mode needs the raw waveform (`scope`).
    pub needs_wave: bool,
    /// The oscilloscope's orientation: `horizontal` or `vertical`.
    pub scope_orientation: String,
    /// The oscilloscope's colours: `duo`, `mono` or `sweep`.
    pub scope_colour: String,
}

impl From<core_viz::settings::VizSettings> for VizSettingsData {
    fn from(s: core_viz::settings::VizSettings) -> Self {
        VizSettingsData {
            needs_wave: s.needs_wave(),
            mode: s.mode,
            chosen_mode: s.chosen_mode,
            palette: s.palette,
            intensity: s.intensity,
            fps: s.fps,
            eco: s.eco,
            scope_orientation: s.scope_orientation,
            scope_colour: s.scope_colour,
        }
    }
}

/// The counters the acceptance reads: an analysis tick is work, a subscription
/// is a reader on the audio tap, and both must be 0 while nothing draws.
#[derive(uniffi::Record, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VizStats {
    /// Analysis frames produced since the process started.
    pub ticks: u64,
    /// Frames published to the surface.
    pub frames: u64,
    /// Subscriptions opened on the tap (0 while nothing draws).
    pub subscribes: u64,
    /// Tap frames this reader lost because it was behind.
    pub dropped: u64,
    /// Counted bytes of the analyser's own buffers and the published frame.
    pub memory_bytes: u64,
    /// Is a view visible right now?
    pub live: bool,
}

/// The visualiser.
#[derive(uniffi::Object)]
pub struct VizApi {
    viz: Arc<Viz>,
}

/// The ONE visualiser of the process, like the player (MB3b's rule): the media
/// service's EQ meter (MB12), the full-screen screen and the player's strip all
/// talk to the same analysis thread, so "0 tick while no view is visible" is one
/// counter and not one per caller.
static SHARED: OnceLock<Arc<VizApi>> = OnceLock::new();

impl VizApi {
    /// Builds the service over the core. `nz-core::viz::Viz` starts no thread
    /// until a view is visible, so this is cheap enough for
    /// `Application.onCreate`'s graph.
    pub fn open(core: Arc<Core>) -> Arc<VizApi> {
        // MB12's multi-reader tap (`Player::subscribe_tap`) is what the opener
        // takes, and it is merged with this lot: `NzCore::viz()` hands the
        // player's subscription in. Until then the seam answers "no tap" and the
        // surface draws its rest frame — the analysis, the gating and every rule
        // are proven against the synthetic source in `nz-core::viz`'s tests.
        let opener = core_viz::tap::no_tap();
        Arc::new(VizApi {
            viz: Viz::new(core, opener),
        })
    }

    /// The process' one instance, opened over the first core that asks.
    pub fn shared(core: Arc<Core>) -> Arc<VizApi> {
        Arc::clone(SHARED.get_or_init(|| VizApi::open(core)))
    }
}

#[uniffi::export]
impl VizApi {
    /// Turns the analysis on or off for ONE view (`bars`, `scope`, `eq`). Called
    /// from the view's lifecycle — never on a timer — so the analysis thread
    /// really does stop when the last view leaves, and the tap's subscription is
    /// released with it.
    pub fn set_visible(&self, view: String, visible: bool) {
        self.viz.set_visible(&view, visible);
    }

    /// The newest frame, or the REST frame while nothing has been analysed yet.
    /// `None` only when nothing is drawn at all (eco, or the mode is off): a
    /// screen that shows a canvas always has numbers to show.
    pub fn frame(&self) -> Option<VizFrame> {
        if !self.viz.settings().draws() {
            return None;
        }
        Some(self.viz.frame().as_ref().into())
    }

    /// The resolved settings (one local read, no disk: the store was read when
    /// the service was built and after every write).
    pub fn settings(&self) -> VizSettingsData {
        self.viz.settings().into()
    }

    /// The surface's shape: how many capsules and how many scope columns it can
    /// draw. Called on a size or mode change, never per frame (changing either
    /// resets an envelope).
    pub fn set_shape(&self, bars: u32, cols: u32) {
        self.viz.set_shape(bars, cols);
    }

    /// The sleeve, as RGBA pixels the platform decoded once (64×64 is what
    /// `coverpalette.js` reads). A zero size forgets it — the row goes back to
    /// its own low → high walk rather than keeping the last track's colours.
    pub fn set_artwork(&self, rgba: Vec<u8>, width: u32, height: u32) {
        self.viz.set_artwork_pixels(rgba, width, height);
    }

    /// Forgets the sleeve (a track with no artwork).
    pub fn clear_artwork(&self) {
        self.viz.clear_artwork();
    }

    /// `viz.mode` — `bars` or `scope`. Writes the device's own `kv` store, so
    /// the next launch already draws the choice (rule 7's "a preference is not a
    /// request").
    pub fn set_mode(&self, mode: String) -> Result<VizSettingsData, NzError> {
        self.viz
            .set_mode(&mode)
            .map(VizSettingsData::from)
            .map_err(NzError::from)
    }

    /// `viz.palette` — `cover`, `duo`, `mono`, `warm` or `ice`.
    pub fn set_palette(&self, palette: String) -> Result<VizSettingsData, NzError> {
        self.viz
            .set_palette(&palette)
            .map(VizSettingsData::from)
            .map_err(NzError::from)
    }

    /// `viz.intensity`, 0..1.
    pub fn set_intensity(&self, intensity: f64) -> Result<VizSettingsData, NzError> {
        self.viz
            .set_intensity(intensity)
            .map(VizSettingsData::from)
            .map_err(NzError::from)
    }

    /// `viz.fps` — 30 or 60.
    pub fn set_fps(&self, fps: u32) -> Result<VizSettingsData, NzError> {
        self.viz
            .set_fps(fps)
            .map(VizSettingsData::from)
            .map_err(NzError::from)
    }

    /// `viz.eco` — no animation anywhere, and the picture capped at 30.
    pub fn set_eco(&self, eco: bool) -> Result<VizSettingsData, NzError> {
        self.viz
            .set_eco(eco)
            .map(VizSettingsData::from)
            .map_err(NzError::from)
    }

    /// `viz.beat` — the tempo detector (read here, consumed by the rhythm modes).
    pub fn set_beat(&self, on: bool) -> Result<VizSettingsData, NzError> {
        self.viz
            .set_beat(on)
            .map(VizSettingsData::from)
            .map_err(NzError::from)
    }

    /// `viz.scope.orientation` — `horizontal` or `vertical`.
    pub fn set_scope_orientation(&self, orientation: String) -> Result<VizSettingsData, NzError> {
        self.viz
            .set_scope_orientation(&orientation)
            .map(VizSettingsData::from)
            .map_err(NzError::from)
    }

    /// `viz.scope.colour` — `duo`, `mono` or `sweep`.
    pub fn set_scope_colour(&self, colour: String) -> Result<VizSettingsData, NzError> {
        self.viz
            .set_scope_colour(&colour)
            .map(VizSettingsData::from)
            .map_err(NzError::from)
    }

    /// The counters of the acceptance: ticks, subscriptions, dropped frames, the
    /// counted bytes, and whether anything is visible.
    pub fn stats(&self) -> VizStats {
        VizStats {
            ticks: self.viz.ticks(),
            frames: self.viz.frames(),
            subscribes: self.viz.subscribes(),
            dropped: self.viz.dropped(),
            memory_bytes: self.viz.memory_bytes(),
            live: self.viz.is_live(),
        }
    }

    /// Joins the analysis thread. Idempotent; the process' exit path calls it
    /// (`NzCore::shutdown`), and a test calls it so no thread outlives the run
    /// (worker rule 5).
    pub fn shutdown(&self) {
        self.viz.shutdown();
    }
}

impl From<&core_viz::frame::Frame> for VizFrame {
    fn from(frame: &core_viz::frame::Frame) -> Self {
        // The seven stops, flattened to (h, s, l) triples: one allocation, and
        // the surface indexes them by the SPA's own `ZONE_COUNT`.
        let mut zones = Vec::with_capacity(core_viz::palette::ZONE_COUNT * 3);
        for zone in frame.zones.iter() {
            zones.push(zone.hue as f32);
            zones.push(zone.sat as f32);
            zones.push(zone.light as f32);
        }
        VizFrame {
            bars: frame.bars.clone(),
            peaks: frame.peaks.clone(),
            scope_a: frame.scope_a.clone(),
            scope_b: frame.scope_b.clone(),
            hue: frame.colours.hue,
            sat: frame.colours.sat,
            light: frame.colours.light,
            spread: frame.colours.spread,
            zones,
            has_zones: frame.has_zones,
            level: frame.level,
            gain: frame.gain,
            locked: frame.locked,
            signal: frame.signal,
            beat: frame.beat,
            bpm: frame.bpm,
            palette: frame.source.key().to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_crosses_with_its_palette_and_its_stops() {
        let core_frame = core_viz::frame::Frame {
            bars: vec![0.1, 0.2, 0.3],
            peaks: vec![0.2, 0.3, 0.4],
            scope_a: vec![-0.5, 0.5, -0.2, 0.2],
            has_zones: true,
            zones: [core_viz::palette::Zone {
                hue: 120.0,
                sat: 0.8,
                light: 0.6,
            }; core_viz::palette::ZONE_COUNT],
            ..core_viz::frame::Frame::default()
        };
        let frame = VizFrame::from(&core_frame);
        assert_eq!(frame.bars.len(), 3);
        assert_eq!(frame.peaks.len(), 3);
        assert_eq!(frame.scope_a.len(), 4);
        assert_eq!(frame.zones.len(), core_viz::palette::ZONE_COUNT * 3);
        assert!((frame.zones[0] - 120.0).abs() < 1e-6);
        assert!(frame.has_zones);
    }

    #[test]
    fn the_settings_cross_with_the_effective_mode_and_cap() {
        let mut settings = core_viz::settings::VizSettings {
            chosen_mode: "scope".to_owned(),
            ..core_viz::settings::VizSettings::default()
        };
        settings.settle();
        let data = VizSettingsData::from(settings.clone());
        assert_eq!(data.mode, "scope");
        assert!(data.needs_wave);
        // Eco: none, and 30.
        settings.eco = true;
        settings.settle();
        let eco = VizSettingsData::from(settings);
        assert_eq!(eco.mode, "off");
        assert_eq!(eco.fps, core_viz::settings::ECO_FPS);
        assert!(!eco.needs_wave);
    }
}
