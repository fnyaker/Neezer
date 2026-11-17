//! The visualiser's data: one analysed frame, and what the settings ask for.
//! Owned by **MB21**; the worlds' renderer is MB22's `nz-viz-gl` and
//! `worlds.rs`.
//!
//! The note pins `VizApi { set_visible(view, bool), frame() -> VizFrame /* bars
//! heights, scope samples, palette */ }` (docs/agents/mobile.md, "Screen lots,
//! wave 3"), so MB0 declared exactly that. Rust pushes a frame ONLY while a view
//! is visible: the analysis thread runs at `nice 5` and stops when nothing looks
//! at it (0 counted ticks with no view — MB21's acceptance).

/// One analysed frame, as the bars and the oscilloscope draw it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct VizFrame {
    /// The bar heights, already normalised to 0..1 (`viz_bars.rs`).
    pub bars: Vec<f32>,
    /// The scope's waveform, downsampled to what the surface draws (`viz_scope.rs`).
    pub scope: Vec<f32>,
    /// The palette the frame was rendered with (`palette.js`'s rules, in Rust).
    pub palette: String,
    /// The overall level, already smoothed the way `rhythm` reports it.
    pub level: f32,
    /// A beat landed on this frame.
    pub beat: bool,
    /// The current tempo estimate, 0 when unknown.
    pub bpm: f32,
}

/// The visualiser. MB21 fills it; MB0 answers an empty frame.
#[derive(uniffi::Object)]
pub struct VizApi;

#[uniffi::export]
impl VizApi {
    /// Turns the tap on or off for ONE view (`bars`, `scope`, `eq`). Called from
    /// the view's `onStart`/`onDispose` — never on a timer, so the analysis
    /// thread really does stop when the card leaves the screen.
    pub fn set_visible(&self, view: String, visible: bool) {
        tracing::debug!(%view, visible, "viz set_visible (MB0 skeleton)");
    }

    /// The latest frame, or `None` while nothing has been analysed yet.
    pub fn frame(&self) -> Option<VizFrame> {
        None
    }
}
