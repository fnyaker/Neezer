//! The downloads worker, shared by the desktop app and the mobile core.
//!
//! **Empty on purpose.** SC0 creates the crate and its manifest so that no
//! later lot edits a shared one; **SC4** moves everything of
//! `desktop/app/src/wiring/offline.rs` that is not GPUI here and leaves the
//! `cx`/`Entity` updates as a desktop adapter. The surface it must expose is the
//! plan's: a `Downloader` trait (`fetch(track, quality, dest_part) ->
//! Result<u64>`, default implementation = `nz-api` on the `Bulk` lane, so iOS
//! can supply `URLSession` later), an `Offline` handle
//! (`add_set`/`remove_set`/`pause`/`resume`/`retry`/`progress`/`events()`), and
//! an `nz-bulk` thread JOINED on drop — docs/agents/desktop.md D13's pitfall: a
//! thread blocked in `Handle::block_on` on a dropped runtime panics or hangs.
//!
//! Rule 1: the worker yields through the `Bulk` lane and nothing else — no
//! second throttle, no bypass. A file is published by rename once complete and a
//! `.part` is never listed. Progress names the CAUSE of each failure, never just
//! a count (CLAUDE.md rule 2).
//!
//! No `#![forbid(unsafe_code)]`: the file to move raises its worker's niceness
//! through a hand-written `nice` binding.
//!
//! `nz-audio` is a DEV-dependency only: the moved playback tests prove that a
//! download leaves the player alone, and the library itself never opens an
//! output device.
