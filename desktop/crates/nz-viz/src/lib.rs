//! Bars and oscilloscope geometry, pure: levels and peaks for the bars, a
//! min/max pair per column for the scope.
//!
//! Owned by lot V1 (`docs/agents/desktop.md`), which adds the path dependency
//! on `webapp/rhythm` (`viz_bars::Bars`, `viz_scope::Scope`) and the palette.
//! D0 ships the crate and its manifest so the workspace is complete.

/// Placeholder for V1's geometry; the lot replaces this file's body.
pub const CRATE: &str = "nz-viz";

#[cfg(test)]
mod tests {
    use super::CRATE;

    #[test]
    fn the_crate_is_named_after_the_visualiser() {
        assert_eq!(CRATE, "nz-viz");
    }
}
