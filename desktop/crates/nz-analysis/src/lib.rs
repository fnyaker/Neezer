//! Client-side whole-track analysis, shared by the desktop app and the mobile
//! core.
//!
//! **Empty on purpose.** SC0 creates the crate and its manifest so that no
//! later lot edits a shared one; **SC1** moves
//! `desktop/app/src/wiring/analysis.rs` here unchanged in behaviour — same
//! public names, same signatures, its tests with it — and leaves a re-export
//! plus any GPUI glue in the desktop file. The manifest already carries every
//! dependency that file imports (`measure` and `trainer`, the web app's own
//! arithmetic linked natively; `nz-api`, `nz-model`; symphonia, rubato, tokio,
//! serde, serde_json) plus the plan's `rhythm` and `nz-audio`, so the extraction
//! declares none.
//!
//! The rules the moved code keeps (CLAUDE.md rule 1, docs/agents/mobile.md
//! *Rule 1 on the phone*): the whole run lives on ONE `nz-analysis` thread at
//! `nice 10`, every request carries the `Bulk` lane, and between two tracks the
//! thread parks while the screen says a track is starting — a person waiting for
//! sound never waits on an analysis. It runs only while a view asks for it.
//!
//! No `#![forbid(unsafe_code)]`: the file to move lowers its own thread's
//! priority through a hand-written `setpriority` binding (`mod sys`), and the
//! attribute would refuse SC1's extraction.
//!
//! `nz-audio` is declared with `default-features = false`: SC2 turns `cpal` into
//! a default feature, and a shared crate that re-enabled it would drag PipeWire,
//! ALSA and the desktop output stack into the Android build through feature
//! unification. That is why `cargo ndk -t arm64-v8a -P 26 check -p nz-analysis`
//! is part of SC0's acceptance.
