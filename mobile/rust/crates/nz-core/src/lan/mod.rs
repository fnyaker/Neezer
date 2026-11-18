//! The LAN exchange between the person's devices: `Lan`, the adapters that make
//! playback and downloads ask a peer first, and the discovery the platform feeds
//! (docs/agents/lan-nodes.md).
//!
//! Owned by **LN13** (the lot plan's row for `mobile/rust/crates/nz-core/src/lan/**`):
//! the lot fills this directory and edits no other file of the crate. LN0 created
//! the module — and the `nz-lan` dependency above it — so that no later lot edits
//! `lib.rs` or a `Cargo.toml`.
//!
//! What LN13 puts here, and why the shapes are already fixed elsewhere:
//!
//! - `LanCore` owns the one [`nz_lan::Lan`] of the process, built with
//!   `DiscoveryKind::External` (Kotlin's `NsdManager` resolves and calls
//!   `LanApi::found`/`lost`).
//! - It wraps the player's opener (`PeerFirstOpener`) and the offline worker's
//!   downloader (`PeerFirstDownloader`), so a tap and a download both go to the
//!   LAN first without the Kotlin side knowing.
//! - It follows the app's activity (`set_active`) for the battery, and tells the
//!   LAN that nothing is happening when the phone sleeps.
//!
//! Every contract LN13 consumes is already declared: `nz_lan::service::Lan`,
//! `nz_lan::opener::PeerFirstOpener`, `nz_lan::downloader::PeerFirstDownloader`
//! and the FFI surface in `nz-ffi/src/lan.rs`.

// LN13: nothing yet — see `docs/agents/lan-nodes.md`, lot LN13.
