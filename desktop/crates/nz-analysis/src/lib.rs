//! Client-side whole-track analysis, shared by the desktop app and the mobile
//! core.
//!
//! SC1 moved `desktop/app/src/wiring/analysis.rs` here unchanged in behaviour:
//! the same public names, the same signatures, its tests with it (11 of them,
//! before and after). That file is now `pub use nz_analysis::*;` plus the one
//! GPUI adapter it cannot lose (see [`analysis::training::families`]). The
//! measurement itself is the web app's arithmetic linked natively (`measure`,
//! `trainer`), so the desktop measures a file with the server's own numbers; the
//! loop, the spool and the training round are this crate's.
//!
//! Three rules shape it (CLAUDE.md rule 1, docs/agents/mobile.md *Rule 1 on the
//! phone*):
//!
//! - **It is background work.** The whole run lives on ONE thread named
//!   `nz-analysis` at `nice 10` ([`analysis::NICE`]), every request carries
//!   `Lane::Bulk`, and between two tracks — and between two training epochs —
//!   the thread parks while [`analysis::Control::is_yielding`] is set. It runs
//!   only while a view is visible: that gate is the screen's (`genres.rs` raises
//!   the flag), never the crate's.
//! - **It says what it is doing** (rule 2): one [`analysis::Event`] per track and
//!   one at the end, with the counts AND the counted causes.
//! - **Nothing blocks the UI thread**: the thread talks to the screen through
//!   [`events`], and a 2 268-line file of decoding stays off the paint.
//!
//! The channel is this crate's own copy of the shell's
//! (`desktop/app/src/wiring/events.rs`): Android has no GPUI and this crate may
//! not depend on the app. [`events`] says why, and why it carries no test.
//!
//! No `#![forbid(unsafe_code)]`: the analysis lowers its own thread's priority
//! through a hand-written `setpriority` binding ([`analysis::lower_priority`]).
//!
//! `nz-audio` is declared with `default-features = false`: SC2 makes `cpal` a
//! default feature, and re-enabling it here would drag PipeWire/ALSA into the
//! Android build through feature unification. `measure`, `trainer` and `rhythm`
//! are the web app's crates linked natively; `symphonia-adapter-libopus` is the
//! one dependency SC0's manifest missed (its `use` lines do not show the
//! fully-qualified `symphonia_adapter_libopus::OpusDecoder` the decoder
//! registry registers), and it was already in the workspace's pins.

pub mod events;

mod analysis;

pub use analysis::*;
