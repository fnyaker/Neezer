//! Réglages, the person's own sections. Owned by **MB16** (docs/agents/mobile.md,
//! "Module and file ownership", and its **MB16** paragraph): the lot fills this
//! directory and edits no other file of the crate.
//!
//! The behaviour is `webapp/src/routes/Settings.svelte` and the components it
//! draws, PORTED — never re-invented. What that means, section by section:
//!
//! | This module | The SPA |
//! |---|---|
//! | [`LocalSettings`], [`Core::set_local_setting`] | `lib/stores.js`'s `persisted(...)`, one key each |
//! | [`Core::settings_sources`], [`Core::save_sources`] | `components/SourceOrder.svelte`, `lib/sources.js` |
//! | [`Core::settings_accounts`], [`Core::import_account`] | `components/MyAccounts.svelte` |
//! | [`Core::settings_access`], [`Core::set_grants`] | `components/AccessSettings.svelte` |
//! | [`Core::settings_services`], [`Core::disconnect_service`] | `components/ServicesSettings.svelte` |
//! | [`Core::settings_server`], [`Core::save_server_settings`] | `Settings.svelte`'s ARL card + `api.getSettings` |
//! | [`Core::disliked_tracks`], [`Core::set_disliked`] | `components/DislikedTracks.svelte` |
//! | [`Core::diagnostic_report`] | `lib/log.js` + `Settings.svelte`'s *Assistance* |
//!
//! # The request budget (rule 3)
//!
//! Each section is ONE screen-open, and it asks what the SPA's own component asks
//! on mount and nothing else: `Sources` is `/sources` + `/me/accounts` +
//! `/access` (three, plus `/settings` and `/deezer/status` for an administrator —
//! exactly the SPA's two admin cards), `Compte` is `/services`, `Découverte` is
//! `/me/dislikes` and the discovery level MB5 already owns, and the appearance,
//! animations, downloads and assistance sections ask NOTHING: they are this
//! device's own preferences. All of it runs on the `User` lane, counted under
//! [`TAG_SETTINGS`].
//!
//! A WARM open of a section asks zero: [`wire::Snapshot`] holds each section as
//! it was last read, so the screen paints at once and refreshes behind the
//! first frame.
//!
//! # What is a "local" setting and what is the server's
//!
//! The theme, the accent, the animations, the download quality, the prefetch and
//! the play cache are THIS device's, stored in the `kv` table exactly as the SPA
//! stored them in `localStorage` ([`local`]). The sources' order, the accounts,
//! the grants, the services and the Deezer credential are the SERVER's: they are
//! read back after every write (rule 6 — a value set in the UI overrides the
//! file, and what the screen shows is what the server holds).

mod local;
mod view;
mod wire;

#[cfg(test)]
mod testing;

use nz_api::{Api, Lane, Method};
use nz_model::{Access, DeezerStatus, MyAccounts, ServerSettings, Services, Sources, Track};
use serde_json::Value;

pub use local::{
    CACHE_LIMITS, DEFAULT_THEME, KNOWN, LocalSettings, PREFETCH_MAX, QUALITIES, THEMES, VIZ_FPS,
    VIZ_MODES,
};
pub use view::{
    AccessAccountView, AccessView, AccountView, AccountsView, CountView, DeezerCardView,
    DeezerStatusView, FeatureView, GrantView, GroupView, LastView, PersonView, ProviderView,
    RunningView, ServerSettingsView, ServiceView, ServicesView, SourceView, SourcesView,
    access_view, account_view, accounts_view, deezer_status_view, server_settings_view,
    service_view, services_view, source_view, sources_view,
};

use crate::core::{Core, CoreError};
use crate::lists::Row;

/// The tag every request of this screen is counted under (`NzCore::stats`).
pub const TAG_SETTINGS: &str = "settings";

/// The place a source holds, as `/api/sources` spells it (`providers/registry.py`):
/// the one asked first, one of the fallbacks in their order, or switched off.
pub const PLACE_MAIN: &str = "main";
pub const PLACE_FALLBACK: &str = "fallback";
pub const PLACE_OFF: &str = "off";

impl Core {
    // --- local settings ----------------------------------------------------

    /// This device's settings (see [`local`]). Disk only: the first frame of a
    /// launch can already be Papier when the person chose Papier.
    pub fn settings(&self) -> LocalSettings {
        self.local_settings()
    }

    /// One local key as its JSON text, for the Aide dump and for a lot that
    /// reads a key of its own.
    pub fn setting_json(&self, key: &str) -> Option<String> {
        self.local_setting(key)
    }

    /// Writes one local key (validated against [`KNOWN`] and its own shape).
    pub fn set_setting_json(&self, key: &str, value: Value) -> Result<Value, CoreError> {
        self.set_local_setting(key, value)
    }

    /// Writes one local key from the JSON TEXT a screen sent (the FFI carries a
    /// `String`; the parse and the validation happen here, once).
    pub fn set_setting_text(&self, key: &str, text: &str) -> Result<(), CoreError> {
        let parsed: Value = serde_json::from_str(text).map_err(|error| CoreError::Invalid {
            detail: format!("{key} : {error}"),
        })?;
        self.set_local_setting(key, parsed).map(|_| ())
    }

    // --- sources -----------------------------------------------------------

    /// The device's copy of `/api/sources`, for the first frame.
    pub fn cached_settings_sources(&self) -> Option<Sources> {
        wire::Snapshot::read(self).sources
    }

    /// `GET /api/sources` — the order, the places and why a source rests.
    pub async fn settings_sources(&self) -> Result<Sources, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_sources(&api).await;
        self.note_settings_traffic(&api, before);
        let sources = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.sources = Some(sources.clone());
        });
        Ok(sources)
    }

    /// `PUT /api/sources` — the admin's order and switches, then the server's
    /// own answer (never a guess: a refused order puts the old one back).
    pub async fn save_sources(&self, change: Value) -> Result<Sources, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        let answer = wire::save_sources(&api, &change).await;
        self.note_settings_traffic(&api, before);
        let sources = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.sources = Some(sources.clone());
        });
        Ok(sources)
    }

    // --- the person's accounts ---------------------------------------------

    pub fn cached_settings_accounts(&self) -> Option<MyAccounts> {
        wire::Snapshot::read(self).accounts
    }

    /// `GET /api/me/accounts` — the person's own accounts and their imports.
    pub async fn settings_accounts(&self) -> Result<MyAccounts, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_accounts(&api).await;
        self.note_settings_traffic(&api, before);
        let accounts = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.accounts = Some(accounts.clone());
        });
        Ok(accounts)
    }

    /// `POST /api/me/accounts/deezer` — link the person's own Deezer account.
    pub async fn link_deezer_account(
        &self,
        arl: &str,
        mode: Option<&str>,
    ) -> Result<MyAccounts, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        api.link_deezer(arl, mode).await?;
        self.note_settings_traffic(&api, before);
        self.refresh_accounts(&api).await
    }

    /// `DELETE /api/me/accounts/<id>` — unlink (the imported library stays).
    pub async fn unlink_account(&self, id: &str) -> Result<MyAccounts, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        api.unlink_account(id).await?;
        self.note_settings_traffic(&api, before);
        self.refresh_accounts(&api).await
    }

    /// `PUT /api/me/accounts/<id>` — what one kind imports automatically.
    pub async fn set_account_auto(
        &self,
        id: &str,
        kinds: Vec<String>,
        on: bool,
    ) -> Result<MyAccounts, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        api.set_account_auto(id, &kinds, on).await?;
        self.note_settings_traffic(&api, before);
        self.refresh_accounts(&api).await
    }

    /// `PUT /api/me/accounts/default` — which account leads. `None` is the
    /// SPA's "Aucune" (`{account: null}`), which `set_default_account`'s typed
    /// form cannot spell.
    pub async fn set_default_account(&self, id: Option<&str>) -> Result<MyAccounts, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        let body = serde_json::json!({ "account": id });
        api.send::<Value, Value>(Lane::User, Method::PUT, "/me/accounts/default", Some(&body))
            .await?;
        self.note_settings_traffic(&api, before);
        self.refresh_accounts(&api).await
    }

    /// `POST /api/me/accounts/<id>/import` — a background job on the server
    /// (rule 2): this answers at once, the screen polls `settings_accounts`.
    pub async fn import_account(
        &self,
        id: &str,
        kinds: Option<Vec<String>>,
    ) -> Result<MyAccounts, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        api.import_account(id, kinds.as_deref()).await?;
        self.note_settings_traffic(&api, before);
        self.refresh_accounts(&api).await
    }

    /// The accounts after a write, and the device's copy refreshed with them.
    async fn refresh_accounts(&self, api: &Api) -> Result<MyAccounts, CoreError> {
        let accounts = wire::fetch_accounts(api).await?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.accounts = Some(accounts.clone());
        });
        Ok(accounts)
    }

    /// `PUT /api/sources` with the whole order, main first — the arrows and
    /// *classer par qualité*.
    pub async fn save_sources_order(&self, order: Vec<String>) -> Result<Sources, CoreError> {
        self.save_sources(serde_json::json!({ "order": order }))
            .await
    }

    /// `PUT /api/sources` with one switch.
    pub async fn set_source_enabled(&self, id: &str, on: bool) -> Result<Sources, CoreError> {
        let mut enabled = serde_json::Map::new();
        enabled.insert(id.to_owned(), Value::Bool(on));
        self.save_sources(serde_json::json!({ "enabled": enabled }))
            .await
    }

    // --- who may use what ---------------------------------------------------

    pub fn cached_settings_access(&self) -> Option<Access> {
        wire::Snapshot::read(self).access
    }

    /// `GET /api/access` — every provider, its accounts, and the groups a grant
    /// can name.
    pub async fn settings_access(&self) -> Result<Access, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_access(&api).await;
        self.note_settings_traffic(&api, before);
        let access = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.access = Some(access.clone());
        });
        Ok(access)
    }

    /// `PUT /api/access/grants` — grant or revoke one feature for a set of
    /// subjects. Answers the account's new grants, as the SPA patches its row.
    pub async fn set_grants(
        &self,
        provider: &str,
        account: &str,
        feature: &str,
        subjects: Vec<String>,
    ) -> Result<Value, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        let answer = api.set_grants(provider, account, feature, &subjects).await;
        self.note_settings_traffic(&api, before);
        Ok(answer?)
    }

    /// `POST /api/groups` — a named set of people.
    pub async fn create_group(&self, name: &str) -> Result<Value, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        let answer = api.create_group(name, &[]).await;
        self.note_settings_traffic(&api, before);
        Ok(answer?)
    }

    /// `PUT /api/groups/<id>` — rename, or change the members.
    pub async fn update_group(&self, id: &str, change: Value) -> Result<Value, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        let answer = api.update_group(id, &change).await;
        self.note_settings_traffic(&api, before);
        Ok(answer?)
    }

    /// `DELETE /api/groups/<id>`.
    pub async fn delete_group(&self, id: &str) -> Result<(), CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        let answer = api.delete_group(id).await;
        self.note_settings_traffic(&api, before);
        Ok(answer?)
    }

    /// `PUT /api/access/grants`, answering the account's new grants as rows.
    pub async fn set_grants_view(
        &self,
        provider: &str,
        account: &str,
        feature: &str,
        subjects: Vec<String>,
    ) -> Result<Vec<GrantView>, CoreError> {
        let answer = self
            .set_grants(provider, account, feature, subjects)
            .await?;
        let grants: std::collections::BTreeMap<String, Vec<String>> = answer
            .get("grants")
            .and_then(|grants| serde_json::from_value(grants.clone()).ok())
            .unwrap_or_default();
        Ok(view::grants(&grants))
    }

    /// `PUT /api/groups/<id>` with the members a grant can then name.
    pub async fn set_group_members(
        &self,
        id: &str,
        members: Vec<String>,
    ) -> Result<Vec<PersonView>, CoreError> {
        let answer = self
            .update_group(id, serde_json::json!({ "members": members }))
            .await?;
        Ok(answer
            .get("members")
            .and_then(|members| members.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|member| {
                Some(PersonView {
                    id: member.get("id")?.as_str()?.to_owned(),
                    name: member.get("name")?.as_str()?.to_owned(),
                })
            })
            .collect())
    }

    // --- the services a playlist can be exported to -------------------------

    pub fn cached_settings_services(&self) -> Option<Services> {
        wire::Snapshot::read(self).services
    }

    /// `GET /api/services` — the export targets and the list formats.
    pub async fn settings_services(&self) -> Result<Services, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_services(&api).await;
        self.note_settings_traffic(&api, before);
        let services = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.services = Some(services.clone());
        });
        Ok(services)
    }

    /// `DELETE /api/services/<id>` — unlink a service. It cannot touch the
    /// archive (nothing here ever does).
    pub async fn disconnect_service(&self, id: &str) -> Result<Services, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        api.disconnect_service(id).await?;
        self.note_settings_traffic(&api, before);
        let services = wire::fetch_services(&api).await?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.services = Some(services.clone());
        });
        Ok(services)
    }

    /// `PUT /api/services/spotify/app` — the operator's own Spotify app.
    pub async fn set_spotify_app(&self, client_id: &str) -> Result<Services, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        api.set_spotify_app(client_id).await?;
        self.note_settings_traffic(&api, before);
        let services = wire::fetch_services(&api).await?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.services = Some(services.clone());
        });
        Ok(services)
    }

    /// `GET /api/services/spotify/authorize` — the consent URL to open in the
    /// platform's browser (an app never types a password for somebody else).
    pub async fn spotify_authorize(&self) -> Result<String, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        let answer = api.spotify_authorize().await;
        self.note_settings_traffic(&api, before);
        let answer = answer?;
        Ok(answer
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned())
    }

    // --- the server's own settings (admin) ---------------------------------

    pub fn cached_settings_server(&self) -> Option<ServerSettings> {
        wire::Snapshot::read(self).server
    }

    /// `GET /api/settings` — the upload quota and the Deezer credential's state.
    /// A member is refused by the server (`403`); the screen shows the cause and
    /// draws no card.
    pub async fn settings_server(&self) -> Result<ServerSettings, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_server_settings(&api).await;
        self.note_settings_traffic(&api, before);
        let server = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.server = Some(server.clone());
        });
        Ok(server)
    }

    /// `POST /api/settings` — the ARL travels here ONCE and is verified by the
    /// server before it is stored; it never comes back, and never touches this
    /// device (CLAUDE.md, *Config & secrets*).
    pub async fn save_server_settings(&self, fields: Value) -> Result<ServerSettings, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        api.set_settings(&fields).await?;
        self.note_settings_traffic(&api, before);
        let server = wire::fetch_server_settings(&api).await?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.server = Some(server.clone());
        });
        Ok(server)
    }

    /// `POST /api/settings` with `{upload_quota_gb}` (0 = no limit).
    pub async fn save_upload_quota(&self, gb: f64) -> Result<ServerSettings, CoreError> {
        self.save_server_settings(serde_json::json!({ "upload_quota_gb": gb }))
            .await
    }

    /// `POST /api/settings` with `{deezer_arl: ""}` — back to the ARL the
    /// server's own configuration carries (what the SPA's *Revenir à l'ARL du
    /// serveur* does). A NEW ARL travels through MB4's `save_deezer_arl`, which
    /// checks its shape and never stores it on this device.
    pub async fn clear_deezer_arl(&self) -> Result<ServerSettings, CoreError> {
        self.save_server_settings(serde_json::json!({ "deezer_arl": "" }))
            .await
    }

    pub fn cached_deezer_status(&self) -> Option<DeezerStatus> {
        wire::Snapshot::read(self).deezer
    }

    /// `GET /api/deezer/status` — is the credential still good? `force` re-tests
    /// the login instead of reading the server's cached verdict.
    pub async fn deezer_status(&self, force: bool) -> Result<DeezerStatus, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_deezer_status(&api, force).await;
        self.note_settings_traffic(&api, before);
        let status = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.deezer = Some(status.clone());
        });
        Ok(status)
    }

    // --- "Je n'aime pas" ----------------------------------------------------

    /// The rows of the person's dislike list, as the list draws them. The CACHE
    /// copy is used by [`Core::cached_disliked_tracks`].
    pub fn cached_disliked_tracks(&self) -> Option<Vec<Row>> {
        wire::Snapshot::read(self).dislikes.map(|tracks| {
            tracks
                .iter()
                .enumerate()
                .map(|(i, t)| track_row(t, i as u32))
                .collect()
        })
    }

    /// `GET /api/me/dislikes` — the titles the person said they do not like.
    pub async fn disliked_tracks(&self) -> Result<Vec<Row>, CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        let answer = wire::fetch_dislikes(&api).await;
        self.note_settings_traffic(&api, before);
        let tracks = answer?;
        wire::Snapshot::write(self, |snapshot| {
            snapshot.dislikes = Some(tracks.clone());
        });
        Ok(tracks
            .iter()
            .enumerate()
            .map(|(index, track)| track_row(track, index as u32))
            .collect())
    }

    /// `POST /api/dislike` with `on = false` — the row comes back into the Flow.
    pub async fn set_disliked(&self, id: &str, on: bool) -> Result<(), CoreError> {
        let api = self.settings_api()?;
        let before = wire::totals(&api);
        let answer = api.dislike(id, on).await;
        self.note_settings_traffic(&api, before);
        Ok(answer?)
    }

    // --- Assistance --------------------------------------------------------

    /// What *Assistance* shows and shares: the two versions, the server, the
    /// device, the counters of what this app spent, and the settings in force.
    ///
    /// The SPA's `lib/log.js` keeps a ring buffer in the page; a native app has
    /// no page, so the journal is logcat (`adb logcat -s neezer`, installed by
    /// MB2) and this report is the part a person can attach to a bug report
    /// without a cable. Nothing secret: no cookie, no ARL, no password.
    pub fn diagnostic_report(&self) -> String {
        let settings = self.settings();
        let mut out = String::new();
        out.push_str("Neezer — diagnostic\n");
        out.push_str(&format!("application : {}\n", self.config().app_version));
        out.push_str(&format!("noyau Rust : {}\n", crate::version()));
        out.push_str(&format!("appareil : {}\n", self.config().device_name));
        out.push_str(&format!("API Android : {}\n", self.config().sdk_int));
        match self.server() {
            Some(server) => {
                out.push_str(&format!("serveur : {}\n", server.origin.as_str()));
                out.push_str(&format!(
                    "version du serveur : {}\n",
                    if server.version.is_empty() {
                        "inconnue"
                    } else {
                        &server.version
                    }
                ));
                out.push_str(&format!(
                    "certificat épinglé : {}\n",
                    server.pinned_sha256.as_deref().unwrap_or("aucun")
                ));
            }
            None => out.push_str("serveur : aucun\n"),
        }
        out.push_str(&format!(
            "réseau : {:?}\n",
            if self.network().is_online() {
                "en ligne"
            } else {
                "hors ligne"
            }
        ));
        let memory = self.memory();
        out.push_str(&format!(
            "mémoire comptée : file {} o, listes {} o\n",
            memory.queue_bytes, memory.lists_bytes
        ));
        out.push_str(&format!("rebuffers : {}\n", self.rebuffers()));
        out.push_str("requêtes par écran :\n");
        for stats in self.stats() {
            out.push_str(&format!(
                "  {} : {} requêtes, {} o reçus, {} o envoyés\n",
                stats.tag, stats.requests, stats.bytes_in, stats.bytes_out
            ));
        }
        out.push_str(&format!(
            "réglages : thème {}, accent {}, éco {}, qualité {}, téléchargement {}, préchargement {} ×{}\n",
            settings.theme,
            settings.accent.as_deref().unwrap_or("thème"),
            if settings.eco { "oui" } else { "non" },
            settings.quality,
            settings.download_quality,
            if settings.prefetch { "oui" } else { "non" },
            settings.prefetch_count
        ));
        out
    }
}

/// One title of "Je n'aime pas", as the shared list draws it. Marked
/// `disliked`, so the row can say it without asking the server again.
fn track_row(track: &Track, position: u32) -> Row {
    let row_id = track.id.as_ref().map(ToString::to_string);
    let provider_id = track.deezer_id.to_string();
    let source = match track.source.as_deref() {
        Some("youtube") => "youtube",
        _ if track.local => "upload",
        _ => "deezer",
    };
    Row {
        key: row_id.clone().unwrap_or_else(|| provider_id.clone()),
        id: row_id.unwrap_or(provider_id),
        title: track.title.clone(),
        artists: artist_line(track),
        album: track.album.title.clone(),
        album_id: (!track.album.deezer_id.to_string().is_empty())
            .then(|| track.album.deezer_id.to_string()),
        duration_s: track.duration,
        cover: track.album.cover.clone(),
        explicit: track.explicit,
        liked: false,
        disliked: true,
        unavailable: track.unavailable,
        downloaded: track.is_archived(),
        source: source.to_owned(),
        position,
        played_at: 0,
        added_at: 0,
    }
}

/// `format.js#artistLine`: the credited line when the server sent one, else the
/// main artist's name.
fn artist_line(track: &Track) -> String {
    if let Some(line) = track
        .display_artist
        .as_deref()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        return line.to_owned();
    }
    let credited: Vec<&str> = track
        .artists
        .iter()
        .map(|credit| credit.name.trim())
        .filter(|name| !name.is_empty())
        .collect();
    if !credited.is_empty() {
        return credited.join(", ");
    }
    track.artist.name.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::NetworkState;

    /// The number the acceptance is held to: opening *Sources* costs the SPA's
    /// own three reads (`SourceOrder`, `MyAccounts`, `AccessSettings` each fire
    /// theirs on mount), a WARM open costs ZERO, and the core's meter says so.
    #[tokio::test]
    async fn a_cold_sources_section_costs_three_and_a_warm_one_none() {
        let (server, _dir, core) = testing::configured().await;

        // --- cold, the SPA's three -------------------------------------------
        core.settings_sources().await.expect("sources");
        core.settings_accounts().await.expect("accounts");
        core.settings_access().await.expect("access");
        assert_eq!(
            server.routes(),
            vec![
                "GET /api/sources",
                "GET /api/me/accounts",
                "GET /api/access",
            ],
            "one request per section, and nothing else"
        );
        let counted = core
            .stats()
            .into_iter()
            .find(|row| row.tag == TAG_SETTINGS)
            .expect("the tag is counted");
        assert_eq!(counted.requests, 3, "NzCore::stats() says what it cost");
        assert!(counted.bytes_in > 0);

        // --- warm, the device's copy -----------------------------------------
        server.forget();
        assert!(core.cached_settings_sources().is_some());
        assert!(core.cached_settings_accounts().is_some());
        assert!(core.cached_settings_access().is_some());
        assert_eq!(server.count(), 0, "a warm open asks nothing");
    }

    /// The administrator's cards are the SPA's two: the quota/ARL state and the
    /// provider's own verdict.
    #[tokio::test]
    async fn the_admin_card_asks_the_spas_two_routes() {
        let (server, _dir, core) = testing::configured().await;
        core.settings_server().await.expect("the server's settings");
        let status = core.deezer_status(true).await.expect("the status");
        assert!(status.ok);
        assert_eq!(
            server.routes(),
            vec!["GET /api/settings", "GET /api/deezer/status"]
        );
        assert!(server.hits()[1].path.contains("force=1"), "Tester re-tests");
    }

    /// A dead link keeps the copy the person already saw, and names the cause
    /// instead of emptying the section (rule 3).
    #[tokio::test]
    async fn a_failure_keeps_the_copy_and_names_its_cause() {
        let (server, _dir, core) = testing::configured().await;
        let first = core.settings_sources().await.expect("the first read");
        assert_eq!(first.main.as_deref(), Some("deezer"));

        server.break_route("/api/sources", 503);
        let failed = core.settings_sources().await.expect_err("a 503");
        assert!(
            matches!(
                failed,
                CoreError::NotReady { .. } | CoreError::Server { .. }
            ),
            "{failed:?}"
        );
        let kept = core.cached_settings_sources().expect("the copy stays");
        assert_eq!(kept.sources.len(), 3);
        assert_eq!(kept.main.as_deref(), Some("deezer"));

        // And with no link at all, the gate answers before a socket is opened.
        core.set_network(NetworkState::None);
        server.forget();
        assert_eq!(
            core.settings_sources().await.expect_err("offline"),
            CoreError::Offline
        );
        assert_eq!(server.count(), 0, "not one socket while offline");
        assert!(core.cached_settings_sources().is_some());
    }

    /// Rule 6: the order shown after a save is the SERVER's answer, read back —
    /// never the body we hoped it would take.
    #[tokio::test]
    async fn a_saved_order_is_read_back_from_the_server() {
        let (server, _dir, core) = testing::configured().await;
        let saved = core
            .save_sources(serde_json::json!({ "order": ["youtube", "deezer"] }))
            .await
            .expect("saved");
        assert_eq!(
            server.routes(),
            vec!["PUT /api/sources", "GET /api/sources"],
            "the write, then the server's own state"
        );
        assert!(
            server
                .last_body_of("PUT", "/api/sources")
                .is_some_and(|body| body.contains("youtube")),
            "the order really travelled"
        );
        // The fake answers its own order; the screen draws THAT, not the request.
        assert_eq!(saved.main.as_deref(), Some("deezer"));
        assert_eq!(
            core.cached_settings_sources()
                .expect("cached")
                .main
                .as_deref(),
            Some("deezer")
        );
    }

    /// The ARL travels ONCE to the server and is never stored on this device,
    /// never echoed back: the screen only ever shows the masked hint.
    #[tokio::test]
    async fn the_arl_is_posted_once_and_never_kept_here() {
        let (server, _dir, core) = testing::configured().await;
        let server_settings = core
            .save_server_settings(serde_json::json!({ "deezer_arl": "super-secret-arl" }))
            .await
            .expect("saved");
        assert_eq!(
            server.last_body_of("POST", "/api/settings").as_deref(),
            Some(r#"{"deezer_arl":"super-secret-arl"}"#),
            "the credential is in the request, and only there"
        );
        assert_eq!(
            server_settings.deezer.arl_hint.as_deref(),
            Some("…a1b2"),
            "what comes back is the masked hint"
        );
        assert_eq!(server_settings.upload_quota_gb, 12.5);
        // The device's copy holds what the server answered — which cannot carry
        // the credential, because `nz-model`'s record has no field for it.
        let body = std::fs::read_to_string(wire::snapshot_path_for_test(&core))
            .expect("the copy is on disk");
        assert!(!body.contains("super-secret-arl"), "{body}");
        assert!(body.contains("…a1b2"), "{body}");
    }

    /// "Je n'aime pas" crosses as rows the shared list draws, each marked
    /// disliked so the row can say it without asking again.
    #[tokio::test]
    async fn the_dislikes_are_rows_the_list_draws() {
        let (server, _dir, core) = testing::configured().await;
        let rows = core.disliked_tracks().await.expect("the list");
        assert_eq!(server.routes(), vec!["GET /api/me/dislikes"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title, "Bof");
        assert_eq!(rows[0].artists, "Ascenseur");
        assert_eq!(rows[0].album, "Le matin");
        assert!(rows[0].disliked);
        assert_eq!(rows[0].key, "22222222-2222-4222-8222-111111111111");
        assert_eq!(
            core.cached_disliked_tracks().expect("the copy").len(),
            2,
            "a warm open paints the same two rows"
        );

        core.set_disliked(&rows[0].id, false).await.expect("undone");
        assert_eq!(
            server.last_body("/api/dislike").as_deref(),
            Some(r#"{"id":"22222222-2222-4222-8222-111111111111","on":false}"#),
            "the SPA's own body (`api.dislike(trackRef(t), false)`)"
        );
    }
}
