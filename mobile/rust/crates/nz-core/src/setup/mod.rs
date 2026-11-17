//! Setup: server URL and port, the TOFU pin, login, the one-shot migration from the old app's data, the updater, the Deezer login hand-off.
//!
//! Owned by **MB4** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this directory and edits no other file of the crate. MB0 created
//! the module empty so that no later lot ever edits `lib.rs`.
//!
//! # The lot, in one paragraph
//!
//! The first minutes of the app: choose the server (host, port, and whether the
//! certificate is verified — off means trust on first use, never trust-all),
//! sign in, and, for a person updating from the WebView app, find everything
//! where they left it. Then two things that are not the first minutes: the app
//! updates itself from ITS server, and Réglages can link the Deezer account.
//!
//! # What each file owns
//!
//! - [`probe`] — the setup screen's one call: normalise the address, learn the
//!   certificate with a handshake that sends nothing, read `GET /api/version`
//!   through it, and say when the certificate is not the one that was pinned.
//! - [`migration`] — the one-shot takeover of the old app's data: preferences,
//!   the session cookie, the downloaded audio (RENAMED, never copied, never
//!   deleted) and the SPA's `localStorage`, with a report whose notice names
//!   what was taken and the CAUSE of everything that was not.
//! - [`update`] — the APK fetch from `/api/android/apk`, `202` waited out, a
//!   truncated body refused. The checks that decide whether Android may install
//!   it (package, `versionCode`, signing lineage, the zip's binary manifest) are
//!   Kotlin's, ported as they are from `android/`.
//! - [`deezer`] — the ARL the login WebView captured, handed to the server
//!   (`POST /api/settings`), never stored on the device.
//!
//! # The rules these files keep
//!
//! - **Rule 1**: nothing here fans out. The probe is one request, the version
//!   read is one, the update is one `Bulk` transfer, the ARL is one mutation.
//! - **Rule 3**: every failure is a cause in French, printed by the screen; no
//!   call waits for a timeout to learn the link is down (the offline gate is
//!   `Core::api`'s, and the probe asks it first).
//! - **Rule 7**: a migration interrupted by a kill resumes (the flag is written
//!   last, every step is idempotent), the old data is never deleted, and the new
//!   app reads the old app's own formats.
//! - **Security**: the pin is enforced on every request (`core::server`), the
//!   session cookie is seeded for the configured origin only, the ARL never
//!   reaches the disk, and no cookie, password or ARL is ever logged.

pub mod deezer;
pub mod migration;
pub mod probe;
pub mod update;

#[cfg(test)]
mod testing;

pub use deezer::DeezerLink;
pub use migration::{
    KEY_DONE, KEY_REPORT, KNOWN_SETTINGS, MIGRATED_SET, MIGRATED_TITLE, MigrationFailure,
    MigrationInput, MigrationReport, MigrationStatus, OldDownload, OldPrefs, OldSetting,
};
pub use probe::{ServerProbe, SetupAddress};
pub use update::{MAX_APK_BYTES, MIN_APK_BYTES, PREPARING_WAIT_S, UpdateFile, update_dir};
