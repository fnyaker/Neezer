//! The supervisor half of a run: two processes, a SIGSTOP for a pause, a resume
//! from the last chunk — owned by lot T3.
//!
//! The engine is a child *of the supervisor* (`neezer-transcribe engine`), not a
//! thread: a pause is a `SIGSTOP` of the child's process group, so a stalled
//! whisper cannot slow the person's music (rule 1), and the lease keeps being
//! heartbeated while it is stopped. T1 pins the one timing that makes the pause
//! usable instead of a flicker.

/// How long every pause reason must be gone before the engine resumes.
///
/// A `User`-lane request, a buffer low-water mark or one slow second of frames
/// are all *bursty*: resuming on the first quiet millisecond would start and
/// stop whisper dozens of times per track, and each start costs the first GPU
/// call's pipeline compilation. Three seconds is the note's measured value.
pub const RESUME_AFTER_MS: u64 = 3_000;

/// The audio engine's buffered-ahead time under which the engine is paused.
///
/// Below two seconds of decoded audio a stalled decode thread would be audible,
/// so a transcription that needs the CPU or the GPU yields before it is (rule 1).
pub const AUDIO_LOW_WATER_MS: u64 = 2_000;

/// How long a `User`-lane request may be in flight before the engine pauses.
pub const USER_LANE_GRACE_MS: u64 = 300;

#[cfg(test)]
mod tests {
    use super::{AUDIO_LOW_WATER_MS, RESUME_AFTER_MS, USER_LANE_GRACE_MS};

    /// Rule 1 on the device: the pause triggers are shorter than the resume
    /// delay, so a pause is a real pause and not a per-frame toggle.
    #[test]
    fn a_pause_is_a_real_pause() {
        assert_eq!(RESUME_AFTER_MS, 3_000);
        const {
            assert!(RESUME_AFTER_MS > AUDIO_LOW_WATER_MS);
            assert!(AUDIO_LOW_WATER_MS > USER_LANE_GRACE_MS);
        };
    }
}
