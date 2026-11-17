//! The worlds' GLES 3 renderer on an `ANativeWindow` (`glow` + `khronos-egl`).
//! Owned by **MB22**.
//!
//! **MB0 created the crate and nothing else**: no context, no program, no
//! surface. The 50 worlds, the 226 skins and the quality tiers arrive with
//! MB22/MB23, extracted from `webapp/src/lib/viz/` by
//! `mobile/tools/extract-worlds.mjs`.
//!
//! What the crate must keep, because the SPA's bench measured it:
//!
//! - one GL context, on the `SurfaceView`'s window, frame-capped (30 fps in the
//!   eco grade), and NOTHING renders while the surface is gone or the screen is
//!   behind another app;
//! - the uniforms come from `rhythm`'s `viz_scene`, the same numbers the page's
//!   own scenes use, so a world looks the same on both;
//! - a shader that fails to compile loses ITS world, never the app.

/// Whether a GLES 3 context can exist on this build and this surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Support {
    pub gles3: bool,
    /// A one-line reason when `gles3` is false, for the diagnostic log.
    pub why: &'static str,
}

/// What this build can do. MB0 answers honestly: nothing yet.
pub fn support() -> Support {
    Support {
        gles3: false,
        why: "MB0: le rendu GLES arrive avec MB22",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The skeleton draws nothing and says why — a screen that cannot render
    /// must name its cause (rule 3), never spin.
    #[test]
    fn nothing_renders_yet_and_it_says_why() {
        let s = support();
        assert!(!s.gles3);
        assert!(!s.why.is_empty());
    }
}
