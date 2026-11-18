//! The party's tests, moved here with the code (SC6).
//!
//! The same 21 tests the desktop app ran, unchanged in what they assert: 9 for
//! the host's decision ([`host`]), 12 for the guest — the timeline's arithmetic,
//! the correction rule, the normalisation gain, the published state, the
//! planner's ten minutes and its recovery after a cut, a real server's chunks,
//! and two real engines end to end ([`guest`], [`end_to_end`]).
//!
//! They run in the desktop workspace (`cargo test -p nz-party`) because they
//! need ffmpeg and, for two of them, a Python that can `import neezer` — the
//! mobile CI's job builds this crate for Android and runs none of it.

mod end_to_end;
mod guest;
mod host;
