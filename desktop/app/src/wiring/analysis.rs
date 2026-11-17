//! Client-side whole-track analysis — the shell's side of it.
//!
//! The work moved to the `nz-analysis` crate (SC1, `docs/agents/mobile.md`
//! *SC1*): the tap → decode → `measure` → POST loop, the training round and their
//! 11 tests live there now, because the phone runs the same analysis and this
//! module never needed GPUI. The crate's doc is the contract; this file is the
//! re-export the shell's screens keep using (`crate::wiring::analysis::…`), plus
//! the ONE adapter that cannot move: [`training::families`] reads the studio's
//! genre tree, which is a GPUI widget.
//!
//! Rules the moved code keeps: one thread named `nz-analysis` at `nice 10`, every
//! request on `Lane::Bulk`, and the thread parks while the screen says a track is
//! starting (rule 1). It still runs only while the genre studio's analysis card is
//! on screen — `genres.rs` raises the flag, this module never decides.

pub use nz_analysis::*;

pub mod training {
    //! The training half as the studio uses it: everything is `nz-analysis`'s,
    //! except the family map, which needs the shell's tree.

    pub use nz_analysis::training::*;

    use std::collections::BTreeMap;

    /// The `families` map a head ships with (`metrics.families`), from the
    /// studio's tree — the signature `genres.rs` has always called. The mapping
    /// itself is [`nz_analysis::training::families`], which takes the lookup
    /// instead of a GPUI widget: `widgets::genre_tree::Tree` is a `gpui_kit`
    /// type and this crate is compiled for Android.
    pub fn families(
        labels: &[String],
        tree: &crate::widgets::genre_tree::Tree,
    ) -> BTreeMap<String, String> {
        nz_analysis::training::families(labels, |label| tree.family_of(label))
    }
}
