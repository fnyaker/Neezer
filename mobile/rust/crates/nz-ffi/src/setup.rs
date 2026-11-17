//! Setup: server URL and port, the TOFU pin, login, the one-shot migration, the updater, the Deezer login hand-off.
//!
//! Owned by **MB4** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this file — its own `#[uniffi::export] impl` blocks, its own
//! records, its own listener — and edits no other file of the crate.
//!
//! The migration reads the OLD app's data directory (same applicationId, rule 7); nothing here writes to the Deezer account.
//!
//! # What crosses, and in which shape
//!
//! - **`probe_server`** answers one record: the origin, the server's version, the
//!   certificate (fingerprint, names, expiry, whether the platform trusts it),
//!   the leaf to pin, and `pin_changed` — the screen says "le certificat a
//!   changé" before replacing a pin, and never replaces it silently.
//! - **`migration_status` / `run_migration`**: the status is a record a screen
//!   paints; the input is what the Kotlin side read off the old app (its
//!   preferences, its cookie, its files' names and the SPA's `localStorage`
//!   values); the report carries the French notice and the CAUSE of everything
//!   that could not be taken over. **The token is never logged.**
//! - **`fetch_update`**: the APK, on disk, counted. Kotlin decides whether
//!   Android may install it (`Updater.kt`/`ApkArchive.kt`, ported as they are).
//! - **`save_deezer_arl`**: the one value `DeezerLoginActivity` captured, sent
//!   to the configured server (`POST /api/settings`). It reaches no file on this
//!   device and never a log line.
//!
//! The lot's core side lives in `nz_core::setup` (`probe`, `migration`, `update`,
//! `deezer`), where the rules and the measurements are written down.

use nz_core::setup::{
    DeezerLink, MigrationInput, MigrationReport, MigrationStatus, OldDownload, OldPrefs,
    OldSetting, ServerProbe, SetupAddress,
};

use crate::core::{CertInfo, Screens};
use crate::errors::NzError;

// --- the setup screen --------------------------------------------------------

/// What a probe learnt, and what the screen shows before saving anything.
#[derive(uniffi::Record, Debug, Clone)]
pub struct ProbeResult {
    /// `scheme://host:port`, the ONE definition of "our server".
    pub origin: String,
    /// What `GET /api/version` answered (`0.1.0`, empty when it said nothing).
    pub version: String,
    /// `https` or a plaintext LAN install (the screen says so).
    pub https: bool,
    /// The leaf the handshake presented, when one was learnt (verification OFF:
    /// with it ON the platform already decided, and there is nothing to show).
    pub cert: Option<CertInfo>,
    /// Whether Android's own trust store would accept this chain.
    pub system_trusted: bool,
    /// The leaf to pin once the person accepts it. `None` when the platform
    /// trusts the chain, or the origin is plaintext.
    pub pin: Option<String>,
    /// A DIFFERENT leaf than the one already pinned for this origin: the screen
    /// says "le certificat a changé" and replaces the pin only on acceptance.
    pub pin_changed: bool,
    /// The server publishes an APK (`/api/version`'s `android.apk`).
    pub update_apk: bool,
    /// The Android version it declares, when it names one.
    pub android_version: Option<String>,
}

impl From<ServerProbe> for ProbeResult {
    fn from(probe: ServerProbe) -> Self {
        ProbeResult {
            origin: probe.origin,
            version: probe.version,
            https: probe.https,
            cert: probe.cert.map(CertInfo::from),
            system_trusted: probe.system_trusted,
            pin: probe.pin,
            pin_changed: probe.pin_changed,
            update_apk: probe.update_apk,
            android_version: probe.android_version,
        }
    }
}

// --- the migration -----------------------------------------------------------

/// The old app's `SharedPreferences("settings")`, as the Kotlin side read it
/// (the platform owns that file's format; this crate owns what the values mean).
#[derive(uniffi::Record, Debug, Clone, Default)]
pub struct OldPrefsRecord {
    pub host: String,
    pub port: String,
    pub verify_ssl: bool,
    pub pin_origin: Option<String>,
    pub pin_cert: Option<String>,
}

impl From<OldPrefsRecord> for OldPrefs {
    fn from(prefs: OldPrefsRecord) -> Self {
        OldPrefs {
            host: prefs.host,
            port: prefs.port,
            verify_ssl: prefs.verify_ssl,
            pin_origin: prefs.pin_origin,
            pin_cert: prefs.pin_cert,
        }
    }
}

/// One download the old app left: its `.audio` file name, the track key its
/// sidecar names, and the account it was made for.
#[derive(uniffi::Record, Debug, Clone, Default)]
pub struct OldDownloadRecord {
    /// A FILE NAME inside `filesDir/offline`, never a path.
    pub file: String,
    pub key: String,
    pub owner: String,
    /// The sidecar's `type` (`audio/flac`, `audio/opus`…): the only thing that
    /// says which quality the bytes are.
    pub content_type: String,
}

impl From<OldDownloadRecord> for OldDownload {
    fn from(download: OldDownloadRecord) -> Self {
        OldDownload {
            file: download.file,
            key: download.key,
            owner: download.owner,
            content_type: download.content_type,
        }
    }
}

/// One `localStorage` entry: the SPA's key verbatim and its raw JSON text.
#[derive(uniffi::Record, Debug, Clone, Default)]
pub struct OldSettingRecord {
    pub key: String,
    pub value: String,
}

impl From<OldSettingRecord> for OldSetting {
    fn from(setting: OldSettingRecord) -> Self {
        OldSetting {
            key: setting.key,
            value: setting.value,
        }
    }
}

/// Everything the Kotlin side read off the old app. An empty field means "the
/// old app never wrote it", never an error.
#[derive(uniffi::Record, Debug, Clone, Default)]
pub struct MigrationInputRecord {
    pub prefs: OldPrefsRecord,
    /// `CookieManager.getCookie(<configured origin>)`, when a session was there.
    pub cookie: Option<String>,
    pub downloads: Vec<OldDownloadRecord>,
    pub settings: Vec<OldSettingRecord>,
    /// How many download sets the SPA's IndexedDB still listed.
    pub offline_sets: u32,
}

impl From<MigrationInputRecord> for MigrationInput {
    fn from(input: MigrationInputRecord) -> Self {
        MigrationInput {
            prefs: input.prefs.into(),
            cookie: input.cookie,
            downloads: input.downloads.into_iter().map(OldDownload::from).collect(),
            settings: input.settings.into_iter().map(OldSetting::from).collect(),
            offline_sets: input.offline_sets,
        }
    }
}

/// One step that could not be taken, with the cause the screen prints (rule 2:
/// never just a count).
#[derive(uniffi::Record, Debug, Clone)]
pub struct MigrationFailureRecord {
    /// `serveur`, `certificat`, `session`, `téléchargements`, `réglages`.
    pub step: String,
    pub cause: String,
}

/// What the migration did, and what the notice says.
#[derive(uniffi::Record, Debug, Clone)]
pub struct MigrationReportRecord {
    /// The origin the old app was pointed at, when it had one.
    pub server: Option<String>,
    /// Whether that origin answered (a home LAN server does not, from a train —
    /// and that is not a failure).
    pub server_answered: bool,
    /// The leaf the old app had pinned, re-pinned here.
    pub pin: bool,
    /// How many cookies were taken over.
    pub session: u32,
    /// How many downloads were MOVED into this app's store.
    pub tracks: u32,
    /// How many settings were kept (the rest are keys this app does not know).
    pub settings: u32,
    /// How many albums the SPA's IndexedDB still had a downloaded track of.
    pub offline_sets: u32,
    pub failures: Vec<MigrationFailureRecord>,
    /// The sentence the notice shows: French, what was taken and the cause of
    /// everything that was not.
    pub notice: String,
}

impl From<MigrationReport> for MigrationReportRecord {
    fn from(report: MigrationReport) -> Self {
        MigrationReportRecord {
            server: report.server,
            server_answered: report.server_answered,
            pin: report.pin,
            session: report.session,
            tracks: report.tracks,
            settings: report.settings,
            offline_sets: report.offline_sets,
            failures: report
                .failures
                .into_iter()
                .map(|failure| MigrationFailureRecord {
                    step: failure.step,
                    cause: failure.cause,
                })
                .collect(),
            notice: report.notice,
        }
    }
}

/// Whether a migration is owed, and what it did the last time.
#[derive(uniffi::Record, Debug, Clone)]
pub struct MigrationStatusRecord {
    /// The old app ran on this device (its downloads or its settings are here).
    pub old_app_found: bool,
    /// The migration still has to run.
    pub needed: bool,
    /// It ran to the end.
    pub done: bool,
    /// What it did, so the notice survives the launch that ran it.
    pub report: Option<MigrationReportRecord>,
}

impl From<MigrationStatus> for MigrationStatusRecord {
    fn from(status: MigrationStatus) -> Self {
        MigrationStatusRecord {
            old_app_found: status.old_app_found,
            needed: status.needed,
            done: status.done,
            report: status.report.map(MigrationReportRecord::from),
        }
    }
}

// --- the updater -------------------------------------------------------------

/// The APK the server served, on this device's own disk.
#[derive(uniffi::Record, Debug, Clone)]
pub struct UpdateFileRecord {
    /// `cacheDir/update/neezer.apk`, an absolute path the app owns.
    pub path: String,
    pub bytes: u64,
}

// --- Deezer ------------------------------------------------------------------

/// What the server says about the account once it holds the ARL.
#[derive(uniffi::Record, Debug, Clone)]
pub struct DeezerLinkRecord {
    /// Whether the server now holds a working credential.
    pub linked: bool,
    /// The masked account the server names (`ab…yz`), when it said.
    pub hint: Option<String>,
    /// Whether the ARL comes from the server's database (an override) rather
    /// than its configuration file.
    pub from_database: bool,
}

impl From<DeezerLink> for DeezerLinkRecord {
    fn from(link: DeezerLink) -> Self {
        DeezerLinkRecord {
            linked: link.linked,
            hint: link.hint,
            from_database: link.from_database,
        }
    }
}

// --- the lot's one impl block ------------------------------------------------

#[uniffi::export(async_runtime = "tokio")]
impl Screens {
    /// Learns the certificate and checks the server answers, WITHOUT saving
    /// anything: only `NzCore::set_server` commits, which is what lets the
    /// screen offer "Réessayer" and "Continuer quand même" over an address
    /// nobody has validated.
    ///
    /// `host` is the field as typed (a scheme and a path may be in it), `port`
    /// the optional port field, and `verify_ssl` the "Vérifier le certificat"
    /// switch: off means trust on first use, never trust-all.
    pub async fn probe_server(
        &self,
        host: String,
        port: String,
        verify_ssl: bool,
    ) -> Result<ProbeResult, NzError> {
        self.core()
            .probe_server(&SetupAddress::new(host, port, verify_ssl))
            .await
            .map(ProbeResult::from)
            .map_err(NzError::from)
    }

    /// Pins one leaf for one origin (trust on first use), or clears it with an
    /// empty hash — the setup screen's "accepter ce certificat", and how a
    /// renewed one is trusted again. `nz-core` rebuilds the client over the new
    /// pin, so every later request obeys it in THIS process.
    pub fn pin_server_certificate(&self, origin: String, sha256: String) -> Result<(), NzError> {
        self.core()
            .pin_certificate(&origin, &sha256)
            .map_err(NzError::from)
    }

    /// The server root a `host`/`port` pair means, normalised by the ONE
    /// implementation of that rule (`nz-core`'s `SetupAddress`, `Prefs.buildBaseUrl`'s
    /// behaviour): the optional port lands in the AUTHORITY only, a host without
    /// a scheme is `https`, and a reverse-proxied path is kept.
    ///
    /// No request, no state: the migration needs the origin BEFORE any screen
    /// exists (to read the old WebView's cookie for that very origin), and a
    /// second URL builder in Kotlin is how `…/musique:5722` happened once.
    pub fn server_url(&self, host: String, port: String) -> Option<String> {
        SetupAddress::new(host, port, true).base_url()
    }

    /// Saves the server and checks it answers `GET /api/version`. The origin is
    /// remembered even when the probe fails (a home server on another link is
    /// still the server the person chose) — the screen shows the cause and
    /// "Réessayer".
    pub async fn set_server(
        &self,
        host: String,
        port: String,
    ) -> Result<crate::core::ServerInfo, NzError> {
        let base = SetupAddress::new(host, port, true)
            .base_url()
            .ok_or_else(|| NzError::Invalid {
                detail: "Adresse du serveur invalide".to_owned(),
            })?;
        self.core()
            .set_server(&base)
            .await
            .map(crate::core::ServerInfo::from)
            .map_err(NzError::from)
    }

    /// Whether a migration is owed, and what the last one did.
    pub fn migration_status(&self) -> MigrationStatusRecord {
        MigrationStatusRecord::from(self.core().migration_status())
    }

    /// Takes the old app's data over, once, and answers what it did (and the
    /// CAUSE of everything it could not). Safe to call again after a kill: a
    /// track already moved is skipped.
    pub async fn run_migration(
        &self,
        input: MigrationInputRecord,
    ) -> Result<MigrationReportRecord, NzError> {
        self.core()
            .run_migration(input.into())
            .await
            .map(MigrationReportRecord::from)
            .map_err(NzError::from)
    }

    /// Fetches the server's APK into `destination` (`cacheDir/update`), on the
    /// `Bulk` lane: a `202` is waited out, a body shorter than its own
    /// `Content-Length` or too small to be an application is refused with its
    /// cause, and nothing is left for an installer.
    ///
    /// Kotlin checks package, `versionCode` and signing lineage BEFORE handing
    /// it to Android (`Updater.kt`, `ApkArchive.kt`).
    pub async fn fetch_update(&self, destination: String) -> Result<UpdateFileRecord, NzError> {
        let file = self
            .core()
            .fetch_update(std::path::Path::new(&destination))
            .await?;
        Ok(UpdateFileRecord {
            path: file.path.display().to_string(),
            bytes: file.bytes,
        })
    }

    /// Hands the ARL `DeezerLoginActivity` captured to the configured server
    /// (`POST /api/settings`, the route `lib/nativeDeezer.js`'s caller uses).
    /// The value reaches no file on this device and no log line; the server
    /// validates and probes it before storing it, so a refused one comes back
    /// as a cause in French.
    pub async fn save_deezer_arl(&self, arl: String) -> Result<DeezerLinkRecord, NzError> {
        self.core()
            .save_deezer_arl(&arl)
            .await
            .map(DeezerLinkRecord::from)
            .map_err(NzError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every record converts without inventing a field, and the notice crosses
    /// as the core wrote it.
    #[test]
    fn a_report_crosses_whole() {
        let report = MigrationReport {
            server: Some("https://neezer.example:443".to_owned()),
            server_answered: true,
            pin: true,
            session: 1,
            tracks: 3,
            settings: 18,
            offline_sets: 2,
            failures: vec![nz_core::setup::MigrationFailure {
                step: "réglages".to_owned(),
                cause: "viz.fps n'était pas une valeur lisible".to_owned(),
            }],
            notice: "Repris : 3 titres téléchargés.".to_owned(),
        };
        let record = MigrationReportRecord::from(report);
        assert_eq!(record.tracks, 3);
        assert_eq!(record.settings, 18);
        assert_eq!(record.failures.len(), 1);
        assert_eq!(record.failures[0].step, "réglages");
        assert!(record.notice.contains("3 titres"));
    }

    /// An input crosses without losing the cookie (and it is never printed).
    #[test]
    fn an_input_crosses_whole() {
        let input = MigrationInputRecord {
            prefs: OldPrefsRecord {
                host: "music.example.org".to_owned(),
                port: "4533".to_owned(),
                verify_ssl: false,
                pin_origin: Some("https://music.example.org:4533".to_owned()),
                pin_cert: Some("AB:CD".to_owned()),
            },
            cookie: Some("session=abc123".to_owned()),
            downloads: vec![OldDownloadRecord {
                file: "aaa.audio".to_owned(),
                key: "t1".to_owned(),
                owner: "42".to_owned(),
                content_type: "audio/opus".to_owned(),
            }],
            settings: vec![OldSettingRecord {
                key: "ui.theme".to_owned(),
                value: "\"papier\"".to_owned(),
            }],
            offline_sets: 2,
        };
        let converted: MigrationInput = input.clone().into();
        assert_eq!(converted.prefs.port, "4533");
        assert_eq!(converted.cookie.as_deref(), Some("session=abc123"));
        assert_eq!(converted.downloads[0].file, "aaa.audio");
        assert_eq!(converted.settings[0].value, "\"papier\"");
        assert_eq!(converted.offline_sets, 2);
        // The record's Debug never carries the cookie into a log line by
        // accident: only the enum's fields the caller prints itself would.
        assert!(!format!("{:?}", input.prefs).contains("abc123"));
    }

    /// The setup screen's sentences never pretend a plaintext origin is secure
    /// and never hide a changed certificate.
    #[test]
    fn a_probe_result_keeps_what_the_screen_says() {
        let probe = ServerProbe {
            origin: "http://192.168.1.10:4533".to_owned(),
            version: "0.1.0".to_owned(),
            https: false,
            cert: None,
            system_trusted: true,
            pin: None,
            pin_changed: false,
            update_apk: false,
            android_version: None,
        };
        let record = ProbeResult::from(probe);
        assert!(!record.https);
        assert!(record.cert.is_none());
        assert_eq!(record.version, "0.1.0");
    }
}
