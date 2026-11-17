//! The one-shot migration from the WebView app (lot MB4, rule 7).
//!
//! The new app keeps the old one's `applicationId`, signing key and data
//! directory, so an update installs OVER it and everything it wrote is still
//! there. This file takes it over — **and never deletes anything of it**:
//! downloads are RENAMED (same filesystem), the old WebView's own directory is
//! left exactly where it is (cleaning it is a later release's decision), and a
//! step that fails records its CAUSE while the rest carries on.
//!
//! The five steps, in the order `docs/agents/mobile.md` (MB4) names them:
//!
//! 1. **the server**: `SharedPreferences("settings")`'s `host`, `port`, the SSL
//!    toggle and `pinOrigin`/`pinCert`, read by the Kotlin side (the platform
//!    owns that format) and handed here; [`Core::set_server`] remembers the
//!    origin even when the probe fails, exactly as setup does.
//! 2. **the pin**: the old app's TOFU leaf, re-pinned for the same origin.
//! 3. **the session**: `CookieManager.getCookie(origin)` — the cookie the old
//!    WebView still holds — seeded into this app's jar, for the CONFIGURED
//!    origin only. The person is not asked to sign in again.
//! 4. **the downloads**: every `filesDir/offline/<hash>.audio` with its sidecar
//!    `{id, type}` becomes a row of `nz-cache`'s downloads store under the set
//!    "Repris de l'ancienne app" ([`MIGRATED_SET`]), its bytes MOVED into the
//!    new offline directory by `rename` — never copied, never re-fetched (the
//!    person is quite possibly on the train this happens on). A track's
//!    metadata is fetched lazily, when the link is back.
//! 5. **the SPA's own storage**: `localStorage`'s known keys (the mapping table
//!    below is `stores.js`, `theme.js` and `offline.js`'s own names) come in as
//!    raw JSON text and go into `kv`, which is what the new Réglages reads.
//!    Unknown keys are ignored: a build from another fork must not be able to
//!    plant a setting this app never had.
//!
//! Every step is guarded by [`KEY_DONE`], so a migration interrupted by a kill
//! resumes instead of starting over (rule 2), and the report is kept in `kv`
//! ([`KEY_REPORT`]) so the notice can be shown again on the next launch.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::core::{Core, CoreError, Origin, ServerInfo};

use super::probe::SetupAddress;

/// `kv` key: the migration happened, once (the flag rule 7 is proven by).
pub const KEY_DONE: &str = "migration.from_webview";
/// `kv` key: the report, so the notice survives the launch that ran it.
pub const KEY_REPORT: &str = "migration.report";
/// The one set the renamed downloads land in.
pub const MIGRATED_SET: &str = "old-app";
/// What that set is called on screen.
pub const MIGRATED_TITLE: &str = "Repris de l'ancienne app";

/// The old app's `SharedPreferences("settings")`, read by the Kotlin side and
/// handed over as values (the platform owns that file's format, this crate owns
/// what they mean).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OldPrefs {
    /// `host`, as the person typed it (a scheme and a path may be in there).
    pub host: String,
    /// `port`, empty for the scheme's default.
    pub port: String,
    /// "Vérifier le certificat" (the old app's `verifySsl`, default true).
    pub verify_ssl: bool,
    /// `pinOrigin`/`pinCert`: the leaf the old app trusted on first use.
    pub pin_origin: Option<String>,
    pub pin_cert: Option<String>,
}

/// One download of the old app: the `.audio` file it left, the track key its
/// sidecar names, and the account it was made for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OldDownload {
    /// The file name inside the old `filesDir/offline` — never a path: the
    /// Kotlin side lists that directory and this crate joins the name to it.
    pub file: String,
    /// The page's track key, from the sidecar `.json`'s `id`.
    pub key: String,
    /// The account the old app downloaded it for (from IndexedDB's `meta`), or
    /// empty when the record predates accounts.
    pub owner: String,
    /// The sidecar's `type` (`audio/flac`, `audio/opus`, …), which is the only
    /// thing that says which quality the bytes are.
    pub content_type: String,
}

/// One `localStorage` entry: the SPA's key verbatim, and its raw JSON text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OldSetting {
    pub key: String,
    /// `JSON.stringify`'s output, exactly as the SPA wrote it.
    pub value: String,
}

/// Everything the Kotlin side read off the old app. Empty fields mean "the old
/// app never wrote that", never an error: a person who only ever opened it once
/// still migrates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationInput {
    pub prefs: OldPrefs,
    /// `CookieManager.getCookie(<configured origin>)`, when the old WebView
    /// still held a session.
    pub cookie: Option<String>,
    pub downloads: Vec<OldDownload>,
    pub settings: Vec<OldSetting>,
    /// How many albums the SPA's IndexedDB still had a downloaded track of.
    pub offline_sets: u32,
    /// Why the SPA's own storage could not be read, when it could not. The page
    /// has a deadline and a WebView that may refuse to open at all: a settings
    /// step that fails must name its cause (rule 2), never just count 0.
    pub settings_cause: Option<String>,
}

/// One step that could not be taken, with the cause the screen prints (rule 2:
/// never just a count).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationFailure {
    /// The step, in the person's language: `serveur`, `session`,
    /// `téléchargements`, `réglages`.
    pub step: String,
    pub cause: String,
}

/// What the migration did — the notice, and the numbers behind it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationReport {
    /// The origin the old app was pointed at, when it had one.
    pub server: Option<String>,
    /// Whether that origin answered (a server only reachable on the home LAN
    /// answers nothing on the train, and that is not a failure).
    pub server_answered: bool,
    /// The leaf the old app had pinned, re-pinned here.
    pub pin: bool,
    /// How many cookies were taken over (0 or 1 in practice).
    pub session: u32,
    /// How many downloads were MOVED into this app's store.
    pub tracks: u32,
    /// How many of `settings` were kept (the rest are keys this app does not
    /// know).
    pub settings: u32,
    /// How many ALBUMS the SPA's IndexedDB still had a downloaded track of (it
    /// keeps per-track records and no set table: the page counts distinct
    /// `track.album.id`s).
    pub offline_sets: u32,
    pub failures: Vec<MigrationFailure>,
    /// The sentence the notice shows: what was taken over, or the cause of each
    /// failure. French, no count without a reason.
    pub notice: String,
}

/// Whether a migration is owed, and what it did the last time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationStatus {
    /// The old app ran on this device (its downloads or its settings are here).
    pub old_app_found: bool,
    /// The migration still has to run.
    pub needed: bool,
    /// It ran to the end.
    pub done: bool,
    /// What it did, kept in `kv` so the notice outlives the launch.
    pub report: Option<MigrationReport>,
}

/// The `localStorage` keys this app TAKES OVER — the SPA's own names, one per
/// setting the new app has a home for (`stores.js`, `theme.js`, `offline.js`).
/// A key outside this table is ignored, and that is the point: the page is a
/// file this crate did not write.
pub const KNOWN_SETTINGS: &[&str] = &[
    // playback, crossfade, trim, normalisation, EQ, presets
    "player.quality",
    "player.volume",
    "player.muted",
    "player.shuffle",
    "player.repeat",
    "fx.eq.enabled",
    "fx.eq.bands",
    "fx.bass",
    "fx.normalize",
    "fx.presets",
    "fade.enabled",
    "fade.seconds",
    "fade.onSkip",
    "fade.skipSeconds",
    "fade.trim",
    "fade.trimDb",
    // where the sound goes
    "audio.output",
    "audio.outputTrim",
    // apparence
    "ui.theme",
    "ui.accent",
    "ui.coverAc",
    // animations and the visualiser
    "viz.eco",
    "viz.mode",
    "viz.quality",
    "viz.palette",
    "viz.intensity",
    "viz.beat",
    "viz.showStyle",
    "viz.lookahead",
    "viz.lookahead.mode",
    "viz.fps",
    "viz.fullBleed",
    "viz.scope.orientation",
    "viz.scope.colour",
    "viz.world",
    "viz.screen.mode",
    "viz.screen.quality",
    "viz.flash",
    "viz.flash.ack",
    // offline and cache
    "offline.quality",
    "offline.onlyDownloaded",
    "cache.limit",
    "cache.prefetch",
    "cache.prefetchCount",
    // bibliothèque
    "playlistfolders.open",
    "playlistfolders.closed",
    "podcast.speeds",
];

impl Core {
    /// The old app's own directory: `filesDir/offline`, beside this app's
    /// `filesDir/neezer`.
    pub fn old_offline_dir(&self) -> Option<PathBuf> {
        self.config()
            .data_dir
            .parent()
            .map(|root| root.join("offline"))
    }

    /// The old app's `shared_prefs` directory, for the "did it ever run here?"
    /// question — its CONTENT is Android's format and is read by Kotlin.
    fn old_prefs_file(&self) -> Option<PathBuf> {
        self.config()
            .data_dir
            .parent()
            .map(|root| root.join("shared_prefs").join("settings.xml"))
    }

    /// Whether a migration is owed, what it did last time, and whether the old
    /// app left anything here at all.
    pub fn migration_status(&self) -> MigrationStatus {
        let done = self.cache().kv().get::<bool>(KEY_DONE).unwrap_or(false);
        let report = self
            .cache()
            .kv()
            .get::<serde_json::Value>(KEY_REPORT)
            .and_then(|value| serde_json::from_value(value).ok());
        let offline = self
            .old_offline_dir()
            .map(|dir| {
                std::fs::read_dir(&dir)
                    .map(|entries| entries.flatten().any(|entry| entry.path().is_file()))
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        let prefs = self
            .old_prefs_file()
            .map(|path| path.is_file())
            .unwrap_or(false);
        MigrationStatus {
            old_app_found: offline || prefs,
            needed: !done,
            done,
            report,
        }
    }

    /// Takes the old app's data over, once. Safe to call again after a kill: a
    /// track already moved is skipped, the set is reused, and only what is
    /// missing is done (rule 7, and rule 2's "resumes instead of starting
    /// over").
    ///
    /// On the `Bulk` lane's side of rule 1: the only request it may make is the
    /// server probe, which is short and happens before any screen exists.
    pub async fn run_migration(&self, input: MigrationInput) -> Result<MigrationReport, CoreError> {
        let mut report = MigrationReport::default();
        let mut failures: Vec<MigrationFailure> = Vec::new();

        // 1. The pin FIRST: the old app trusted this leaf on first use, and the
        // server's own probe below must already obey it. In the other order the
        // probe met the certificate unpinned and the cause came out as the weaker
        // "n'est pas accepté" instead of "a changé" — measured on a TLS server
        // whose certificate had changed (the old app's setup passed its fingerprint
        // to its probe for the same reason).
        if let (Some(pin_origin), Some(pin_cert)) = (
            input.prefs.pin_origin.as_ref(),
            input.prefs.pin_cert.as_ref(),
        ) && !pin_cert.trim().is_empty()
        {
            match self.pin_certificate(pin_origin, pin_cert) {
                Ok(()) => report.pin = true,
                Err(error) => failures.push(MigrationFailure {
                    step: "certificat".to_owned(),
                    cause: format!(
                        "le certificat épinglé n'a pas pu être repris ({})",
                        cause_of(&error)
                    ),
                }),
            }
        }

        // 2. The server, remembered even when it does not answer (a home server
        // on another link is still the server the person chose).
        let mut origin: Option<Origin> = None;
        if !input.prefs.host.trim().is_empty() {
            let address = SetupAddress::new(
                input.prefs.host.clone(),
                input.prefs.port.clone(),
                input.prefs.verify_ssl,
            );
            match address.base_url() {
                Some(base) => match self.set_server(&base).await {
                    Ok(ServerInfo { origin: set, .. }) => {
                        report.server = Some(set.as_str());
                        report.server_answered = true;
                        origin = Some(set);
                    }
                    // The origin is stored before the probe, so this is "the
                    // person is on another link", not a lost setup.
                    Err(CoreError::Offline) => {
                        report.server = self.origin().map(|set| set.as_str());
                        origin = self.origin();
                    }
                    Err(error) => {
                        report.server = self.origin().map(|set| set.as_str());
                        origin = self.origin();
                        failures.push(MigrationFailure {
                            step: "serveur".to_owned(),
                            cause: format!("le serveur n'a pas répondu ({})", cause_of(&error)),
                        });
                    }
                },
                None => failures.push(MigrationFailure {
                    step: "serveur".to_owned(),
                    cause: "l'adresse enregistrée par l'ancienne application est illisible"
                        .to_owned(),
                }),
            }
        }

        // 3. The session: the old WebView's cookie, seeded into this app's jar,
        // for the CONFIGURED origin only — a cookie lifted from one host must
        // never be presented to another.
        if let Some(cookie) = input
            .cookie
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            match origin.or_else(|| self.origin()) {
                Some(configured) => {
                    let kept = self.jar().seed(&configured.as_str(), cookie);
                    report.session = kept as u32;
                    if kept == 0 {
                        failures.push(MigrationFailure {
                            step: "session".to_owned(),
                            cause: "le cookie de l'ancienne application est illisible".to_owned(),
                        });
                    }
                }
                None => failures.push(MigrationFailure {
                    step: "session".to_owned(),
                    cause: "aucun serveur n'est configuré pour recevoir la session".to_owned(),
                }),
            }
        }

        // 4. The downloads: MOVED, never copied and never re-fetched.
        let (moved, mut moved_failures) = self.take_downloads(&input.downloads)?;
        report.tracks = moved;
        failures.append(&mut moved_failures);
        report.offline_sets = input.offline_sets;

        // 5. The SPA's own storage.
        report.settings = self.take_settings(&input.settings, &mut failures)?;
        if let Some(cause) = input
            .settings_cause
            .as_deref()
            .filter(|cause| !cause.trim().is_empty())
        {
            failures.push(MigrationFailure {
                step: "réglages".to_owned(),
                cause: cause.to_owned(),
            });
        }

        report.failures = failures;
        report.notice = notice(&report);

        self.cache().kv().set(KEY_REPORT, &report)?;
        self.cache().kv().set(KEY_DONE, &true)?;
        tracing::info!(
            tracks = report.tracks,
            settings = report.settings,
            session = report.session,
            failures = report.failures.len(),
            "migration from the old app done"
        );
        Ok(report)
    }

    /// Renames every download the old app left into this app's offline store,
    /// one row per track under [`MIGRATED_SET`]. Nothing is deleted: a file
    /// that cannot be moved stays where it is, with its cause in the report.
    fn take_downloads(
        &self,
        downloads: &[OldDownload],
    ) -> Result<(u32, Vec<MigrationFailure>), CoreError> {
        let Some(old_dir) = self.old_offline_dir() else {
            return Ok((0, Vec::new()));
        };
        let mut failures = Vec::new();
        let mut moved = 0u32;
        // The account each track belongs to; an empty owner is the old record's
        // own "before accounts" answer and is stored as such.
        let mut by_owner: BTreeMap<String, Vec<(nz_model::TrackId, nz_model::Quality)>> =
            BTreeMap::new();
        let mut sources: Vec<(String, String, nz_model::TrackId, nz_model::Quality)> = Vec::new();

        for download in downloads {
            // A file name, never a path: a hostile sidecar cannot walk out of
            // the directory.
            if download.file.is_empty()
                || download.file.contains('/')
                || download.file.contains('\\')
                || download.file.starts_with('.')
            {
                failures.push(MigrationFailure {
                    step: "téléchargements".to_owned(),
                    cause: "un fichier de l'ancienne application a un nom inattendu".to_owned(),
                });
                continue;
            }
            if !old_dir.join(&download.file).is_file() {
                failures.push(MigrationFailure {
                    step: "téléchargements".to_owned(),
                    cause: format!("{} n'est plus sur le disque", download.file),
                });
                continue;
            }
            if download.key.trim().is_empty() {
                failures.push(MigrationFailure {
                    step: "téléchargements".to_owned(),
                    cause: format!("{} n'a pas de titre associé", download.file),
                });
                continue;
            }
            let track = nz_model::TrackId::new(download.key.clone());
            let quality = quality_of(&download.content_type);
            match self.cache().downloads().local(&download.owner, &track) {
                Ok(Some(_)) => continue, // already taken over by a first run
                Ok(None) => {}
                Err(error) => {
                    failures.push(MigrationFailure {
                        step: "téléchargements".to_owned(),
                        cause: format!("le registre des téléchargements est illisible ({error})"),
                    });
                    continue;
                }
            }
            by_owner
                .entry(download.owner.clone())
                .or_default()
                .push((track.clone(), quality));
            sources.push((
                download.file.clone(),
                download.owner.clone(),
                track,
                quality,
            ));
        }

        // The set comes first: a kill between the two leaves rows the next run
        // reuses (`ON CONFLICT DO NOTHING`) instead of a set nobody listed.
        for (owner, tracks) in &by_owner {
            let wanted: Vec<nz_cache::DownloadTrack> = tracks
                .iter()
                .map(|(track, quality)| nz_cache::DownloadTrack {
                    track: track.clone(),
                    quality: *quality,
                })
                .collect();
            if let Err(error) = self.cache().downloads().add_set(
                owner,
                "migrated",
                MIGRATED_SET,
                MIGRATED_TITLE,
                &wanted,
            ) {
                failures.push(MigrationFailure {
                    step: "téléchargements".to_owned(),
                    cause: format!("la liste reprise n'a pas pu être créée ({error})"),
                });
            }
        }

        for (file, owner, track, quality) in sources {
            let source = old_dir.join(&file);
            let partial = match self.cache().downloads().reserve(&owner, &track, quality) {
                Ok(partial) => partial,
                Err(error) => {
                    failures.push(MigrationFailure {
                        step: "téléchargements".to_owned(),
                        cause: format!("{file} n'a pas pu être réservé ({error})"),
                    });
                    continue;
                }
            };
            // SAME filesystem (`filesDir/offline` → `filesDir/neezer/offline`),
            // so this is a rename: no byte is read, written or re-fetched.
            let length = std::fs::metadata(&source)
                .map(|meta| meta.len())
                .unwrap_or(0);
            if let Err(error) = std::fs::rename(&source, &partial) {
                let _ = std::fs::remove_file(&partial);
                let _ = self.cache().downloads().mark(
                    &owner,
                    &track,
                    nz_cache::DownloadState::Failed,
                    Some("le fichier de l'ancienne application n'a pas pu être déplacé"),
                );
                failures.push(MigrationFailure {
                    step: "téléchargements".to_owned(),
                    cause: format!("{file} n'a pas pu être déplacé ({error})"),
                });
                continue;
            }
            match self.cache().downloads().complete(&owner, &track, length) {
                Ok(_) => moved += 1,
                Err(error) => failures.push(MigrationFailure {
                    step: "téléchargements".to_owned(),
                    cause: format!("{file} est déplacé mais n'a pas pu être publié ({error})"),
                }),
            }
        }

        Ok((moved, failures))
    }

    /// The known `localStorage` keys, raw JSON in and raw JSON out, so the value
    /// the new app reads is the value the SPA wrote.
    fn take_settings(
        &self,
        settings: &[OldSetting],
        failures: &mut Vec<MigrationFailure>,
    ) -> Result<u32, CoreError> {
        let mut kept = 0u32;
        for setting in settings {
            if !KNOWN_SETTINGS.contains(&setting.key.as_str()) {
                continue;
            }
            match serde_json::from_str::<serde_json::Value>(&setting.value) {
                Ok(value) => match self.cache().kv().set(&setting.key, &value) {
                    Ok(()) => kept += 1,
                    Err(error) => failures.push(MigrationFailure {
                        step: "réglages".to_owned(),
                        cause: format!("{} n'a pas pu être repris ({error})", setting.key),
                    }),
                },
                Err(_) => failures.push(MigrationFailure {
                    step: "réglages".to_owned(),
                    cause: format!("{} n'était pas une valeur lisible", setting.key),
                }),
            }
        }
        Ok(kept)
    }
}

/// The quality the old bytes are, from the sidecar's content type. The file is
/// played as it is: this only names what is on disk (a label, never a re-encode).
fn quality_of(content_type: &str) -> nz_model::Quality {
    let kind = content_type.to_ascii_lowercase();
    if kind.contains("flac") {
        nz_model::Quality::Flac
    } else if kind.contains("mpeg") || kind.contains("mp3") {
        nz_model::Quality::Opus320
    } else {
        // Opus (the SPA's default download quality) and anything unknown: the
        // best Opus label, because the file is what it is.
        nz_model::Quality::Opus320
    }
}

/// A failure's cause in a few words, never a code and never a cookie.
fn cause_of(error: &CoreError) -> String {
    match error {
        CoreError::Offline => "hors ligne".to_owned(),
        CoreError::Timeout => "délai dépassé".to_owned(),
        CoreError::Session => "session expirée".to_owned(),
        CoreError::Server { status, .. } => format!("le serveur a répondu {status}"),
        CoreError::Certificate { changed, .. } => {
            if *changed {
                "le certificat du serveur a changé".to_owned()
            } else {
                "le certificat du serveur n'est pas accepté".to_owned()
            }
        }
        CoreError::Invalid { detail } => detail.clone(),
        CoreError::Internal { detail } => detail.clone(),
        other => other.kind().replace('_', " "),
    }
}

/// "Repris : 3 titres téléchargés, 1 liste hors ligne, 18 réglages." — then the
/// cause of each failure, one clause each (rule 2: never just a count).
fn notice(report: &MigrationReport) -> String {
    let mut taken: Vec<String> = Vec::new();
    if report.tracks > 0 {
        taken.push(format!(
            "{} titre{} téléchargé{}",
            report.tracks,
            plural(report.tracks),
            plural(report.tracks)
        ));
    }
    if report.offline_sets > 0 {
        // What the SPA's IndexedDB really knows: the ALBUMS a downloaded track
        // belongs to (it keeps per-track records, never a "set"); counted in the
        // hidden WebView, one entry per `track.album.id`.
        taken.push(format!(
            "{} album{} hors ligne",
            report.offline_sets,
            plural(report.offline_sets)
        ));
    }
    if report.settings > 0 {
        taken.push(format!(
            "{} réglage{}",
            report.settings,
            plural(report.settings)
        ));
    }
    if report.session > 0 {
        taken.push("la session".to_owned());
    }
    let mut text = if taken.is_empty() {
        "Rien à reprendre de l'ancienne application.".to_owned()
    } else {
        format!("Repris : {}.", taken.join(", "))
    };
    // A server that is simply not reachable from here (a home LAN server, a
    // train) is NOT a failure: the origin is stored and the next connection
    // finds it. Saying so is rule 2's "a button that seems to do nothing".
    if report.server.is_some() && !report.server_answered {
        text.push_str(" Le serveur n'a pas répondu ; il sera joint dès que possible.");
    }
    if !report.failures.is_empty() {
        let causes: Vec<String> = report
            .failures
            .iter()
            .map(|failure| format!("{} : {}", failure.step, failure.cause))
            .collect();
        text.push_str(&format!(" Non repris — {}.", causes.join(" ; ")));
    }
    text
}

/// French plural, the SPA's own rule: one, then everything else.
fn plural(count: u32) -> &'static str {
    if count > 1 { "s" } else { "" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::testing::TempDir;

    fn download(file: &str, key: &str) -> OldDownload {
        OldDownload {
            file: file.to_owned(),
            key: key.to_owned(),
            owner: "42".to_owned(),
            content_type: "audio/opus".to_owned(),
        }
    }

    /// A fresh directory that LOOKS like the old app's: `filesDir/offline` with
    /// two audio files and their sidecars.
    fn with_old_app(dir: &TempDir) -> PathBuf {
        let offline = dir.path().join("offline");
        std::fs::create_dir_all(&offline).expect("the old offline dir");
        std::fs::write(offline.join("aaa.audio"), vec![7u8; 4_096]).expect("audio a");
        std::fs::write(offline.join("bbb.audio"), vec![9u8; 2_048]).expect("audio b");
        std::fs::write(
            offline.join("aaa.json"),
            br#"{"id":"t1","type":"audio/opus"}"#,
        )
        .expect("sidecar a");
        offline
    }

    /// The acceptance's heart: the files are MOVED (present in the new store,
    /// gone from the old one — a rename, not a copy) and registered under the
    /// set the note names; the old directory keeps everything else.
    #[tokio::test]
    async fn downloads_are_renamed_into_the_new_store_and_never_deleted() {
        let dir = TempDir::new("migrate-downloads");
        let old = with_old_app(&dir);
        std::fs::write(old.join("notes.txt"), b"keep me").expect("a foreign file");
        let core = dir.core("test");

        let report = core
            .run_migration(MigrationInput {
                downloads: vec![download("aaa.audio", "t1"), download("bbb.audio", "t2")],
                offline_sets: 2,
                ..MigrationInput::default()
            })
            .await
            .expect("the migration runs");

        assert_eq!(report.tracks, 2, "{report:?}");
        assert!(!old.join("aaa.audio").exists(), "moved, not copied");
        assert!(!old.join("bbb.audio").exists(), "moved, not copied");
        assert!(old.join("notes.txt").is_file(), "nothing else was touched");
        for key in ["t1", "t2"] {
            let track = nz_model::TrackId::new(key);
            let (path, _quality) = core
                .cache()
                .downloads()
                .local("42", &track)
                .expect("the store answers")
                .unwrap_or_else(|| panic!("{key} is in the new store"));
            assert!(path.is_file(), "{key}: {}", path.display());
            assert_eq!(
                path.parent(),
                Some(core.cache().offline_dir().as_path()),
                "{key} lives in THIS app's offline directory"
            );
        }
        let sets = core.cache().downloads().sets("42").expect("its set");
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].title, MIGRATED_TITLE);
        assert_eq!(sets[0].tracks, 2);
        assert_eq!(sets[0].done, 2, "both are playable, not pending");
        // The bytes are the old ones, byte for byte.
        let (path, _) = core
            .cache()
            .downloads()
            .local("42", &nz_model::TrackId::new("t1"))
            .expect("answers")
            .expect("present");
        assert_eq!(std::fs::read(path).expect("read"), vec![7u8; 4_096]);
        assert!(
            report.notice.contains("2 titres téléchargés"),
            "{}",
            report.notice
        );
    }

    /// The settings: known keys are taken over with their exact JSON, unknown
    /// ones are ignored (a fork's key cannot plant a setting here), and a value
    /// that is not JSON is a cause, not a crash.
    #[tokio::test]
    async fn known_settings_are_taken_over_and_unknown_ones_ignored() {
        let dir = TempDir::new("migrate-settings");
        let core = dir.core("test");
        let report = core
            .run_migration(MigrationInput {
                settings: vec![
                    OldSetting {
                        key: "ui.theme".to_owned(),
                        value: "\"papier\"".to_owned(),
                    },
                    OldSetting {
                        key: "fx.eq.bands".to_owned(),
                        value: "[1,-2,3,0,0,0,4,0,-1,2]".to_owned(),
                    },
                    OldSetting {
                        key: "nz.unknown".to_owned(),
                        value: "\"whatever\"".to_owned(),
                    },
                    OldSetting {
                        key: "viz.fps".to_owned(),
                        value: "not json".to_owned(),
                    },
                ],
                ..MigrationInput::default()
            })
            .await
            .expect("the migration runs");

        assert_eq!(report.settings, 2, "{report:?}");
        assert_eq!(
            core.cache().kv().get::<String>("ui.theme"),
            Some("papier".to_owned())
        );
        assert_eq!(
            core.cache().kv().get::<Vec<i32>>("fx.eq.bands"),
            Some(vec![1, -2, 3, 0, 0, 0, 4, 0, -1, 2])
        );
        assert!(
            core.cache()
                .kv()
                .get::<serde_json::Value>("nz.unknown")
                .is_none(),
            "an unknown key is not written"
        );
        assert_eq!(report.failures.len(), 1, "{report:?}");
        assert_eq!(report.failures[0].step, "réglages");
        assert!(report.notice.contains("viz.fps"), "{}", report.notice);
    }

    /// A settings step that could not run at all names its cause instead of
    /// counting 0 (rule 2): the hidden WebView has a deadline, and a page that
    /// never answers must cost the settings and say why.
    #[tokio::test]
    async fn an_unreadable_spa_storage_keeps_its_cause() {
        let dir = TempDir::new("migrate-settings-cause");
        let core = dir.core("test");
        let report = core
            .run_migration(MigrationInput {
                settings_cause: Some("la page de migration n'a pas répondu".to_owned()),
                ..MigrationInput::default()
            })
            .await
            .expect("the migration runs");
        assert_eq!(report.settings, 0);
        assert_eq!(report.failures.len(), 1, "{report:?}");
        assert_eq!(report.failures[0].step, "réglages");
        assert!(report.failures[0].cause.contains("pas répondu"));
        assert!(report.notice.contains("Non repris"), "{}", report.notice);
    }

    /// The session: the old WebView's cookie is seeded for the CONFIGURED
    /// origin, and the next request carries it — no relaunch. A cookie with no
    /// server to belong to is a cause, not a leak.
    #[tokio::test]
    async fn the_session_cookie_is_seeded_for_the_configured_origin_only() {
        let dir = TempDir::new("migrate-session");
        let core = dir.core("test");
        core.set_network(crate::core::NetworkState::None);
        let report = core
            .run_migration(MigrationInput {
                prefs: OldPrefs {
                    host: "music.example.org".to_owned(),
                    port: String::new(),
                    verify_ssl: true,
                    pin_origin: None,
                    pin_cert: None,
                },
                cookie: Some("session=abc123".to_owned()),
                ..MigrationInput::default()
            })
            .await
            .expect("the migration runs");
        assert_eq!(report.session, 1, "{report:?}");
        core.set_network(crate::core::NetworkState::Wifi { metered: false });
        assert_eq!(
            report.server.as_deref(),
            Some("https://music.example.org:443"),
            "the origin was remembered even with no link"
        );
        let client = core.api().expect("a client");
        assert!(client.has_session(), "the next request carries the cookie");

        // A migration with a cookie but no server at all says why.
        let bare = TempDir::new("migrate-session-bare");
        let core = bare.core("test");
        let report = core
            .run_migration(MigrationInput {
                cookie: Some("session=abc123".to_owned()),
                ..MigrationInput::default()
            })
            .await
            .expect("the migration runs");
        assert_eq!(report.session, 0);
        assert!(
            report
                .failures
                .iter()
                .any(|failure| failure.step == "session"),
            "{report:?}"
        );
    }

    /// It runs ONCE (rule 7's flag), and a second run does not move anything
    /// twice nor lose a track.
    #[tokio::test]
    async fn the_flag_makes_it_run_once_and_a_rerun_loses_nothing() {
        let dir = TempDir::new("migrate-once");
        with_old_app(&dir);
        let core = dir.core("test");
        assert!(core.migration_status().needed);
        assert!(core.migration_status().old_app_found);

        let first = core
            .run_migration(MigrationInput {
                downloads: vec![download("aaa.audio", "t1"), download("bbb.audio", "t2")],
                settings: vec![OldSetting {
                    key: "ui.theme".to_owned(),
                    value: "\"encre\"".to_owned(),
                }],
                ..MigrationInput::default()
            })
            .await
            .expect("the migration runs");
        assert_eq!(first.tracks, 2);

        let status = core.migration_status();
        assert!(!status.needed, "the flag is set");
        assert!(status.done);
        assert_eq!(
            status.report.as_ref().map(|report| report.tracks),
            Some(2),
            "the report survives for the notice"
        );

        // A second run (a kill between the flag and the end, say) finds its
        // files gone and says so, without inventing a third track.
        let again = crate::core::Core::new(crate::core::CoreConfig {
            data_dir: dir.path().join("data"),
            cache_dir: dir.path().join("cache"),
            app_version: "0.0.0-test".to_owned(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".to_owned(),
        })
        .expect("a core over the same directories");
        let second = again
            .run_migration(MigrationInput {
                downloads: vec![download("aaa.audio", "t1")],
                ..MigrationInput::default()
            })
            .await
            .expect("a second run answers");
        assert_eq!(second.tracks, 0, "already taken over, never moved twice");
        assert_eq!(
            again
                .cache()
                .downloads()
                .progress("42")
                .expect("progress")
                .done,
            2,
            "the two tracks of the first run are still there"
        );
    }

    /// A hostile sidecar cannot walk out of the directory, and an ownerless
    /// record still works (the old app wrote some before accounts existed).
    #[tokio::test]
    async fn a_file_name_is_never_a_path() {
        let dir = TempDir::new("migrate-hostile");
        let core = dir.core("test");
        let report = core
            .run_migration(MigrationInput {
                downloads: vec![
                    download("../../session.json", "t1"),
                    download("/etc/passwd", "t2"),
                    OldDownload {
                        owner: String::new(),
                        ..download("gone.audio", "t3")
                    },
                ],
                ..MigrationInput::default()
            })
            .await
            .expect("the migration runs");
        assert_eq!(report.tracks, 0);
        assert_eq!(report.failures.len(), 3, "{report:?}");
        assert!(core.cache().downloads().sets("").expect("sets").is_empty());
    }

    /// A server that does not answer is NOT a failure: the origin is stored (the
    /// person is on another link) and the notice says the server will be joined
    /// as soon as possible. The cause of a REFUSAL is in the notice, in French.
    #[tokio::test]
    async fn an_unreachable_old_server_is_a_cause_not_a_lost_setup() {
        let dir = TempDir::new("migrate-server");
        let core = dir.core("test");
        let report = core
            .run_migration(MigrationInput {
                prefs: OldPrefs {
                    // Port 1 on the loopback: nothing listens, and no name is
                    // resolved — the test never leaves the machine.
                    host: "http://127.0.0.1:1".to_owned(),
                    port: String::new(),
                    verify_ssl: false,
                    pin_origin: None,
                    pin_cert: None,
                },
                ..MigrationInput::default()
            })
            .await
            .expect("the migration runs");
        assert_eq!(report.server.as_deref(), Some("http://127.0.0.1:1"));
        assert_eq!(
            core.origin().map(|origin| origin.as_str()).as_deref(),
            Some("http://127.0.0.1:1"),
            "the server the person chose is remembered anyway"
        );
        assert!(!report.server_answered);
        assert!(
            report.failures.is_empty(),
            "an unreachable server is not a failed takeover: {report:?}"
        );
        assert!(
            report.notice.contains("n'a pas répondu"),
            "{}",
            report.notice
        );
    }

    /// Every cause a step can fail with is a French sentence, never a code and
    /// never a cookie — the notice prints these verbatim.
    #[test]
    fn a_cause_is_a_sentence() {
        assert_eq!(cause_of(&CoreError::Offline), "hors ligne");
        assert_eq!(
            cause_of(&CoreError::Server {
                status: 503,
                detail: "busy".to_owned()
            }),
            "le serveur a répondu 503"
        );
        assert_eq!(
            cause_of(&CoreError::Certificate {
                sha256: "AB".to_owned(),
                changed: true
            }),
            "le certificat du serveur a changé"
        );
        assert_eq!(
            cause_of(&CoreError::Invalid {
                detail: "Adresse du serveur invalide".to_owned()
            }),
            "Adresse du serveur invalide"
        );
    }
}
