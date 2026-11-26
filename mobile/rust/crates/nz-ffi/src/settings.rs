//! Réglages: the person's own settings, read and written through the core's
//! store (the SPA's `Meta` rule: a value set in the UI overrides the file).
//! Owned by **MB16**.
//!
//! The note names what this API reaches (docs/agents/mobile.md, "Screen lots,
//! wave 3", **MB16**): the server settings the person may change, the sources'
//! order, my accounts, the Deezer ARL (MB4's save), YouTube connect, access
//! grants, groups, export services, the diagnostic log, and the app/server
//! versions; the local settings live in the `kv` table.
//!
//! # The shape of this boundary
//!
//! - **This device's settings are ONE record** ([`SettingsData`]): the theme, the
//!   accent, the animations, the quality, the prefetch and the play cache. It is
//!   read synchronously (`settings()`, disk only) because the shell needs it
//!   BEFORE the first frame — a launch in Papier must not flash Encre.
//! - **Each server section is a `cached_x()` / `x()` pair**: the device's copy
//!   paints at once, the network replaces it, and a failure keeps the copy
//!   ([`crate::core::Screens`]' rule, `screenFlow` in Kotlin).
//! - **"Je n'aime pas" crosses as a `ListHandle`**, like every long list (rule 4):
//!   the screen windows it 64 rows at a time, and the row is the shared
//!   [`TrackRow`] so a tap queues what a playlist row queues.
//! - **No JSON crosses.** The screen says what it wants (`save_sources_order`,
//!   `set_source_enabled`, `save_deezer_arl`), never a body it built itself: the
//!   payloads are the SPA's own (`lib/api.js`), and a typo in a string would only
//!   reach the server.
//! - **The ARL never comes back**, in either direction: [`DeezerCard`] carries
//!   the masked hint and where it comes from, exactly what the server sends.
//!
//! Every body converts to `nz_core::settings` and back; no rule lives here.

use std::sync::Arc;

use nz_core::core::Core;
use nz_core::settings as core_settings;

use crate::errors::NzError;
use crate::lists::{ListHandle, TrackRow};

/// One setting, as the screen names it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct SettingValue {
    /// The dotted key, the SPA's own (`playback.quality`, `appearance.theme`).
    pub key: String,
    /// A JSON scalar or object, exactly as stored.
    pub value: String,
}

/// Who is signed in, as *Compte* draws it — read from the core's own session, so
/// it costs no request and is right after a restore.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct AccountIdentity {
    pub id: String,
    pub name: String,
    /// The administrator sections exist only when true (the server enforces it).
    pub admin: bool,
}

/// The two versions Aide shows side by side.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct Versions {
    /// The app's `versionName` (`0.1.0` for MB0's skeleton).
    pub app: String,
    /// What the server's `/api/version` answered, empty when unknown.
    pub server: String,
}

/// This device's settings, as the appearance, animations and downloads sections
/// draw them (`lib/stores.js`'s persisted keys, resolved).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct SettingsData {
    /// `ui.theme` — `encre`, `papier` or `pochette`.
    pub theme: String,
    /// `ui.accent` — the person's own colour (`#RRGGBB`), `None` for the theme's.
    pub accent: Option<String>,
    /// `ui.coverAc` — the sleeve colour Pochette paints before a track is known.
    pub cover_accent: Option<String>,
    /// `viz.eco` — no animation anywhere.
    pub eco: bool,
    /// `viz.mode` — `bars` or `scope`.
    pub viz_mode: String,
    /// `viz.palette` — `cover`, `duo`, `mono`, …
    pub viz_palette: String,
    /// `viz.intensity` — 0..1.
    pub viz_intensity: f64,
    /// `viz.fps` — 30 or 60.
    pub viz_fps: u32,
    /// `viz.beat` — the beat detector feeds the visualiser.
    pub viz_beat: bool,
    /// `viz.scope.orientation` — `horizontal` or `vertical`.
    pub viz_scope_orientation: String,
    /// `viz.scope.colour` — `duo`, `mono`, …
    pub viz_scope_colour: String,
    /// The PLAYER's rung (`player.audio`'s quality, in the SPA's spelling): one
    /// truth, mirrored as `player.quality` for a reader of the SPA's keys.
    pub quality: String,
    /// `offline.quality` — what a download asks for.
    pub download_quality: String,
    /// `offline.onlyDownloaded` — an offline play queues only what is on disk.
    pub offline_only: bool,
    /// `cache.limit` — the play cache's cap, in bytes.
    pub cache_limit: u64,
    /// `cache.prefetch` — keep the next titles on the device.
    pub prefetch: bool,
    /// `cache.prefetchCount` — how many, 1..=10.
    pub prefetch_count: u32,
}

impl From<core_settings::LocalSettings> for SettingsData {
    fn from(settings: core_settings::LocalSettings) -> Self {
        SettingsData {
            theme: settings.theme,
            accent: settings.accent,
            cover_accent: settings.cover_accent,
            eco: settings.eco,
            viz_mode: settings.viz_mode,
            viz_palette: settings.viz_palette,
            viz_intensity: settings.viz_intensity,
            viz_fps: settings.viz_fps,
            viz_beat: settings.viz_beat,
            viz_scope_orientation: settings.viz_scope_orientation,
            viz_scope_colour: settings.viz_scope_colour,
            quality: settings.quality,
            download_quality: settings.download_quality,
            offline_only: settings.offline_only,
            cache_limit: settings.cache_limit,
            prefetch: settings.prefetch,
            prefetch_count: settings.prefetch_count,
        }
    }
}

/// One source of sound, in the order the server asks them.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct SourceData {
    pub id: String,
    pub name: String,
    /// `main`, `fallback` or `off` — the server's own words.
    pub place: String,
    /// The fallback's rank (1 is asked first), `None` for the main one.
    pub rank: Option<i64>,
    /// Ready to be asked at all (a missing credential says so in `setup`).
    pub enabled: bool,
    pub configured: bool,
    pub can_lead: bool,
    pub switchable: bool,
    /// `FLAC`, `Opus 160`, … — the best it delivers.
    pub quality_label: String,
    pub lossless: bool,
    /// Seconds the source is left alone after a transport failure.
    pub resting_s: u32,
    pub resting_why: String,
    /// The French sentence telling the person how to set it up.
    pub setup: String,
}

impl From<core_settings::SourceView> for SourceData {
    fn from(source: core_settings::SourceView) -> Self {
        SourceData {
            id: source.id,
            name: source.name,
            place: source.place,
            rank: source.rank,
            enabled: source.enabled,
            configured: source.configured,
            can_lead: source.can_lead,
            switchable: source.switchable,
            quality_label: source.quality_label,
            lossless: source.lossless,
            resting_s: source.resting_s,
            resting_why: source.resting_why,
            setup: source.setup,
        }
    }
}

/// `/api/sources` as the *Ordre des sources* card draws it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct SourcesData {
    /// In the server's own order: the main one, the fallbacks, the off ones.
    pub sources: Vec<SourceData>,
    /// The source leading right now, `None` when none does.
    pub main: Option<String>,
    /// Whether the person may change the order (the server enforces it).
    pub admin: bool,
}

impl From<core_settings::SourcesView> for SourcesData {
    fn from(sources: core_settings::SourcesView) -> Self {
        SourcesData {
            sources: sources.sources.into_iter().map(SourceData::from).collect(),
            main: sources.main,
            admin: sources.admin,
        }
    }
}

/// One account the person may use, as *Mes comptes* draws it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct AccountData {
    pub id: String,
    /// `deezer`, `youtube`, …
    pub provider: String,
    pub name: String,
    /// The account's own handle (`nadia`), when the server sends one.
    pub label: Option<String>,
    /// The account belongs to the server, not to this person.
    pub server: bool,
    /// `ok`, `auth`, `broken`, …
    pub status: String,
    pub is_default: bool,
    /// The import kinds this account can bring in, in the server's order.
    pub kinds: Vec<String>,
    /// The kinds that import automatically (`a.auto[kind] === true`).
    pub auto_on: Vec<String>,
    /// The running import's phase (`connect`, `list`, `import`), `None` when
    /// nothing runs.
    pub running_phase: Option<String>,
    /// What the running import is importing (`tracks`, `playlists`, …).
    pub running_kind: Option<String>,
    pub running_done: u32,
    pub running_total: u32,
    /// The last import's time (ISO-8601), `None` when it never ran.
    pub last_at: Option<String>,
    /// The last import's outcome: `network`, `auth`, or an error's own text.
    pub last_reason: Option<String>,
    pub last_error: Option<String>,
    /// What the last import added, `kind` → count (only the non-zero ones).
    pub last_added: Vec<CountData>,
    /// What was already here and left as it was.
    pub last_kept: Vec<CountData>,
    /// `false` when the account has no FLAC (a free Deezer offer).
    pub lossless: Option<bool>,
    /// The KIND of a failure only, when the account is broken.
    pub error: Option<String>,
}

/// `kind` → count, for an import's report.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct CountData {
    pub kind: String,
    pub count: u32,
}

impl From<core_settings::AccountView> for AccountData {
    fn from(account: core_settings::AccountView) -> Self {
        let (running_phase, running_kind, running_done, running_total) = match account.running {
            Some(running) => (
                Some(running.phase),
                running.kind,
                running.done,
                running.total,
            ),
            None => (None, None, 0, 0),
        };
        let (last_at, last_reason, last_error, last_added, last_kept) = match account.last {
            Some(last) => (
                last.at,
                last.reason,
                last.error,
                last.added.into_iter().map(CountData::from).collect(),
                last.kept.into_iter().map(CountData::from).collect(),
            ),
            None => (None, None, None, Vec::new(), Vec::new()),
        };
        AccountData {
            id: account.id,
            provider: account.provider,
            name: account.name,
            label: account.label,
            server: account.server,
            status: account.status,
            is_default: account.is_default,
            kinds: account.kinds,
            auto_on: account.auto_on,
            running_phase,
            running_kind,
            running_done,
            running_total,
            last_at,
            last_reason,
            last_error,
            last_added,
            last_kept,
            lossless: account.lossless,
            error: account.error,
        }
    }
}

impl From<core_settings::CountView> for CountData {
    fn from(count: core_settings::CountView) -> Self {
        CountData {
            kind: count.kind,
            count: count.count,
        }
    }
}

/// `/api/me/accounts` as *Mes comptes* draws it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct AccountsData {
    pub accounts: Vec<AccountData>,
    /// The providers this person may still link (`can_link`'s true entries).
    pub can_link: Vec<String>,
    /// The account whose favourites sync daily, `None` for "Aucune".
    pub default: Option<String>,
}

impl From<core_settings::AccountsView> for AccountsData {
    fn from(accounts: core_settings::AccountsView) -> Self {
        let core_settings::AccountsView {
            accounts,
            can_link,
            default,
        } = accounts;
        AccountsData {
            accounts: accounts.into_iter().map(AccountData::from).collect(),
            can_link,
            default,
        }
    }
}

/// One feature a provider offers, and who may use it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct FeatureData {
    pub id: String,
    /// `shared` or `personal`.
    pub scope: String,
}

/// One account a provider is reached through, with its grants.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct AccessAccountData {
    pub id: String,
    /// `server` or `personal`.
    pub mode: String,
    pub label: Option<String>,
    pub server: bool,
    /// The person who holds it, when it is not the caller.
    pub owner: Option<String>,
    pub mine: bool,
    pub editable: bool,
    /// `subject` (a user id, a group id, or `*`) → the features it may use.
    pub grants: Vec<GrantData>,
}

/// One grant: who, and which features.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct GrantData {
    pub subject: String,
    pub features: Vec<String>,
}

/// A person a grant may name.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct PersonData {
    pub id: String,
    pub name: String,
}

/// One provider, as *Sources et accès* draws it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ProviderData {
    pub id: String,
    pub name: String,
    /// `main`, `fallback`, `info`.
    pub role: Option<String>,
    /// The accounts it is reached through.
    pub accounts: Vec<AccessAccountData>,
    /// The features THIS person may use, already resolved by the server.
    pub allowed: Vec<String>,
    /// Who may use it without any account.
    pub anonymous_grants: Vec<GrantData>,
    pub anonymous_editable: bool,
    /// Every feature the provider offers.
    pub features: Vec<FeatureData>,
}

/// A group of people a grant can name.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct GroupData {
    pub id: String,
    pub name: String,
    /// Present for an administrator only (the server decides).
    pub members: Vec<PersonData>,
}

/// `/api/access` as *Sources et accès* draws it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct AccessData {
    pub providers: Vec<ProviderData>,
    pub groups: Vec<GroupData>,
    /// The people a grant may name; empty for a member who lends nothing.
    pub users: Vec<PersonData>,
    pub admin: bool,
}

impl From<core_settings::GrantView> for GrantData {
    fn from(grant: core_settings::GrantView) -> Self {
        GrantData {
            subject: grant.subject,
            features: grant.features,
        }
    }
}

impl From<core_settings::PersonView> for PersonData {
    fn from(person: core_settings::PersonView) -> Self {
        PersonData {
            id: person.id,
            name: person.name,
        }
    }
}

impl From<core_settings::AccessView> for AccessData {
    fn from(access: core_settings::AccessView) -> Self {
        AccessData {
            providers: access
                .providers
                .into_iter()
                .map(|provider| ProviderData {
                    id: provider.id,
                    name: provider.name,
                    role: provider.role,
                    accounts: provider
                        .accounts
                        .into_iter()
                        .map(|account| AccessAccountData {
                            id: account.id,
                            mode: account.mode,
                            label: account.label,
                            server: account.server,
                            owner: account.owner,
                            mine: account.mine,
                            editable: account.editable,
                            grants: account.grants.into_iter().map(GrantData::from).collect(),
                        })
                        .collect(),
                    allowed: provider.allowed,
                    anonymous_grants: provider
                        .anonymous_grants
                        .into_iter()
                        .map(GrantData::from)
                        .collect(),
                    anonymous_editable: provider.anonymous_editable,
                    features: provider
                        .features
                        .into_iter()
                        .map(|feature| FeatureData {
                            id: feature.id,
                            scope: feature.scope,
                        })
                        .collect(),
                })
                .collect(),
            groups: access
                .groups
                .into_iter()
                .map(|group| GroupData {
                    id: group.id,
                    name: group.name,
                    members: group.members.into_iter().map(PersonData::from).collect(),
                })
                .collect(),
            users: access.users.into_iter().map(PersonData::from).collect(),
            admin: access.admin,
        }
    }
}

/// One export target (or list format) the export sheet offers.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct ServiceData {
    pub id: String,
    pub name: String,
    pub linked: bool,
    /// The credential is in place, so an export can run.
    pub ready: bool,
    pub account: Option<String>,
    /// A service with no door: it exports a list FILE instead.
    pub file: bool,
    /// Spotify's own app id, empty when unset.
    pub client_id: Option<String>,
}

/// `/api/services` as *Services de streaming* draws it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct ServicesData {
    pub services: Vec<ServiceData>,
    /// The list formats a service-less export can produce (`M3U`, `CSV`).
    pub formats: Vec<String>,
}

impl From<core_settings::ServicesView> for ServicesData {
    fn from(services: core_settings::ServicesView) -> Self {
        ServicesData {
            services: services
                .services
                .into_iter()
                .map(|service| ServiceData {
                    id: service.id,
                    name: service.name,
                    linked: service.linked,
                    ready: service.ready,
                    account: service.account,
                    file: service.file,
                    client_id: service.client_id,
                })
                .collect(),
            formats: services.formats,
        }
    }
}

/// The Deezer credential's STATE — never the credential.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct DeezerCard {
    /// A credential is in force.
    pub arl_set: bool,
    /// `database` (saved from the app), `config` (the server's file), or none.
    pub arl_source: Option<String>,
    /// The last four characters, to recognise it by (`…a1b2`).
    pub arl_hint: Option<String>,
    /// The provider is running at all.
    pub enabled: bool,
    /// The server has an archive directory (an ARL cannot be saved without it).
    pub archive_dir_set: bool,
}

/// `/api/settings` as the administrator's cards draw it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ServerSettingsData {
    /// The upload quota for non-admins, in GB (0 = no limit).
    pub upload_quota_gb: f64,
    pub deezer: DeezerCard,
}

impl From<core_settings::ServerSettingsView> for ServerSettingsData {
    fn from(settings: core_settings::ServerSettingsView) -> Self {
        ServerSettingsData {
            upload_quota_gb: settings.upload_quota_gb,
            deezer: DeezerCard {
                arl_set: settings.deezer.arl_set,
                arl_source: settings.deezer.arl_source,
                arl_hint: settings.deezer.arl_hint,
                enabled: settings.deezer.enabled,
                archive_dir_set: settings.deezer.archive_dir_set,
            },
        }
    }
}

/// `/api/deezer/status` — the provider's own verdict, with its reason.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct DeezerStatusData {
    pub ok: bool,
    /// `disabled`, `no_arl`, `expired`, `unreachable`, …
    pub reason: Option<String>,
    /// The French sentence the server wrote, when it wrote one.
    pub message: Option<String>,
    pub admin: bool,
}

impl From<core_settings::DeezerStatusView> for DeezerStatusData {
    fn from(status: core_settings::DeezerStatusView) -> Self {
        DeezerStatusData {
            ok: status.ok,
            reason: status.reason,
            message: status.message,
            admin: status.admin,
        }
    }
}

/// The settings store, over the one core of the process.
///
/// It carries the `Arc<Core>` `NzCore::settings()` hands it, so a section reaches
/// the client, the caches and the meter — never a second global.
#[derive(uniffi::Object)]
pub struct SettingsApi {
    core: Arc<Core>,
}

impl SettingsApi {
    /// Built by `NzCore::settings()` over the app's one core.
    pub fn open(core: Arc<Core>) -> SettingsApi {
        SettingsApi { core }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl SettingsApi {
    // --- this device -------------------------------------------------------

    /// This device's settings, resolved. **Synchronous and disk only**: the shell
    /// reads it before the first frame, which is what keeps a launch in Papier
    /// from flashing Encre (one `SELECT` per key).
    pub fn settings(&self) -> SettingsData {
        SettingsData::from(self.core.settings())
    }

    /// Reads one key from the local `kv` table — synchronous and disk-only, so
    /// the settings screen paints at once.
    pub fn get(&self, key: String) -> Option<String> {
        self.core.setting_json(&key)
    }

    /// Every local key this app stores, with its stored JSON (only the ones that
    /// were ever written).
    pub fn all(&self) -> Vec<SettingValue> {
        core_settings::KNOWN
            .iter()
            .filter_map(|key| {
                self.core.setting_json(key).map(|value| SettingValue {
                    key: (*key).to_owned(),
                    value,
                })
            })
            .collect()
    }

    /// Writes one LOCAL key (theme, accent, animations, quality, prefetch): the
    /// next screen already reads it, and a failure has nothing to roll back.
    pub async fn set(&self, key: String, value: String) -> Result<(), NzError> {
        self.core
            .set_setting_text(&key, &value)
            .map_err(NzError::from)
    }

    /// The signed-in person, from the session the core restored (`/api/me`'s
    /// answer, kept in memory): no request, and `None` before a login.
    pub fn identity(&self) -> Option<AccountIdentity> {
        self.core.me().map(|me| AccountIdentity {
            id: me.user.id.to_string(),
            name: me.user.name,
            admin: me.user.admin,
        })
    }

    // --- Aide --------------------------------------------------------------

    /// The two versions Aide shows side by side — the app's, read from Rust so a
    /// stale constant cannot lie, and the server's, as `/api/version` answered.
    pub fn versions(&self) -> Versions {
        Versions {
            app: self.core.config().app_version.clone(),
            server: self
                .core
                .server()
                .map(|server| server.version)
                .unwrap_or_default(),
        }
    }

    /// What *Assistance* shares: versions, server, device, counted traffic and
    /// the settings in force. No cookie, no ARL, no password.
    pub fn diagnostic_report(&self) -> String {
        self.core.diagnostic_report()
    }

    // --- sources ----------------------------------------------------------

    /// The device's copy of the sources' order, for the first frame.
    pub fn cached_sources(&self) -> Option<SourcesData> {
        self.core
            .cached_settings_sources()
            .map(|sources| SourcesData::from(core_settings::sources_view(&sources)))
    }

    /// `GET /api/sources` — the order, the places and why a source rests.
    pub async fn sources(&self) -> Result<SourcesData, NzError> {
        let sources = self.core.settings_sources().await.map_err(NzError::from)?;
        Ok(SourcesData::from(core_settings::sources_view(&sources)))
    }

    /// `PUT /api/sources` with the whole order (main first) — the arrows and the
    /// "classer par qualité" button. The server's own answer comes back.
    pub async fn save_sources_order(&self, order: Vec<String>) -> Result<SourcesData, NzError> {
        let sources = self
            .core
            .save_sources_order(order)
            .await
            .map_err(NzError::from)?;
        Ok(SourcesData::from(core_settings::sources_view(&sources)))
    }

    /// `PUT /api/sources` with one switch.
    pub async fn set_source_enabled(&self, id: String, on: bool) -> Result<SourcesData, NzError> {
        let sources = self
            .core
            .set_source_enabled(&id, on)
            .await
            .map_err(NzError::from)?;
        Ok(SourcesData::from(core_settings::sources_view(&sources)))
    }

    // --- my accounts ------------------------------------------------------

    pub fn cached_accounts(&self) -> Option<AccountsData> {
        self.core
            .cached_settings_accounts()
            .map(|accounts| AccountsData::from(core_settings::accounts_view(&accounts)))
    }

    /// `GET /api/me/accounts` — the person's own accounts and their imports.
    pub async fn accounts(&self) -> Result<AccountsData, NzError> {
        let accounts = self.core.settings_accounts().await.map_err(NzError::from)?;
        Ok(AccountsData::from(core_settings::accounts_view(&accounts)))
    }

    /// `POST /api/me/accounts/deezer` — link the person's own account. `mode` is
    /// `arl` (pasted) or `browser` (MB4's own login page handed the ARL back).
    pub async fn link_deezer(
        &self,
        arl: String,
        mode: Option<String>,
    ) -> Result<AccountsData, NzError> {
        let accounts = self
            .core
            .link_deezer_account(&arl, mode.as_deref())
            .await
            .map_err(NzError::from)?;
        Ok(AccountsData::from(core_settings::accounts_view(&accounts)))
    }

    /// `DELETE /api/me/accounts/<id>` — unlink; what was imported stays.
    pub async fn unlink_account(&self, id: String) -> Result<AccountsData, NzError> {
        let accounts = self.core.unlink_account(&id).await.map_err(NzError::from)?;
        Ok(AccountsData::from(core_settings::accounts_view(&accounts)))
    }

    /// `PUT /api/me/accounts/<id>` — one import kind on or off.
    pub async fn set_account_auto(
        &self,
        id: String,
        kinds: Vec<String>,
        on: bool,
    ) -> Result<AccountsData, NzError> {
        let accounts = self
            .core
            .set_account_auto(&id, kinds, on)
            .await
            .map_err(NzError::from)?;
        Ok(AccountsData::from(core_settings::accounts_view(&accounts)))
    }

    /// `PUT /api/me/accounts/default` — which account's favourites sync daily.
    /// `None` is "Aucune".
    pub async fn set_default_account(&self, id: Option<String>) -> Result<AccountsData, NzError> {
        let accounts = self
            .core
            .set_default_account(id.as_deref())
            .await
            .map_err(NzError::from)?;
        Ok(AccountsData::from(core_settings::accounts_view(&accounts)))
    }

    /// `POST /api/me/accounts/<id>/import` — a server job (rule 2); the screen
    /// polls `accounts()` and reads `running_*`.
    pub async fn import_account(
        &self,
        id: String,
        kinds: Vec<String>,
    ) -> Result<AccountsData, NzError> {
        let kinds = (!kinds.is_empty()).then_some(kinds);
        let accounts = self
            .core
            .import_account(&id, kinds)
            .await
            .map_err(NzError::from)?;
        Ok(AccountsData::from(core_settings::accounts_view(&accounts)))
    }

    // --- who may use what -------------------------------------------------

    pub fn cached_access(&self) -> Option<AccessData> {
        self.core
            .cached_settings_access()
            .map(|access| AccessData::from(core_settings::access_view(&access)))
    }

    /// `GET /api/access` — the providers, their accounts and their grants.
    pub async fn access(&self) -> Result<AccessData, NzError> {
        let access = self.core.settings_access().await.map_err(NzError::from)?;
        Ok(AccessData::from(core_settings::access_view(&access)))
    }

    /// `PUT /api/access/grants` — grant or revoke one feature for a set of
    /// subjects. Answers the account's new grants as `[subject, features…]`.
    pub async fn set_grants(
        &self,
        provider: String,
        account: String,
        feature: String,
        subjects: Vec<String>,
    ) -> Result<Vec<GrantData>, NzError> {
        let grants = self
            .core
            .set_grants_view(&provider, &account, &feature, subjects)
            .await
            .map_err(NzError::from)?;
        Ok(grants.into_iter().map(GrantData::from).collect())
    }

    /// `POST /api/groups` — a named set of people.
    pub async fn create_group(&self, name: String) -> Result<String, NzError> {
        let answer = self.core.create_group(&name).await.map_err(NzError::from)?;
        Ok(answer
            .get("id")
            .and_then(|id| id.as_str())
            .unwrap_or_default()
            .to_owned())
    }

    /// `PUT /api/groups/<id>` — the members a grant can then name.
    pub async fn set_group_members(
        &self,
        id: String,
        members: Vec<String>,
    ) -> Result<Vec<PersonData>, NzError> {
        let members = self
            .core
            .set_group_members(&id, members)
            .await
            .map_err(NzError::from)?;
        Ok(members.into_iter().map(PersonData::from).collect())
    }

    /// `DELETE /api/groups/<id>`.
    pub async fn delete_group(&self, id: String) -> Result<(), NzError> {
        self.core.delete_group(&id).await.map_err(NzError::from)
    }

    // --- export services --------------------------------------------------

    pub fn cached_services(&self) -> Option<ServicesData> {
        self.core
            .cached_settings_services()
            .map(|services| ServicesData::from(core_settings::services_view(&services)))
    }

    /// `GET /api/services` — the export targets and the list formats.
    pub async fn services(&self) -> Result<ServicesData, NzError> {
        let services = self.core.settings_services().await.map_err(NzError::from)?;
        Ok(ServicesData::from(core_settings::services_view(&services)))
    }

    /// `DELETE /api/services/<id>` — unlink (the archive is untouched).
    pub async fn disconnect_service(&self, id: String) -> Result<ServicesData, NzError> {
        let services = self
            .core
            .disconnect_service(&id)
            .await
            .map_err(NzError::from)?;
        Ok(ServicesData::from(core_settings::services_view(&services)))
    }

    /// `PUT /api/services/spotify/app` — the operator's own Spotify app.
    pub async fn set_spotify_app(&self, client_id: String) -> Result<ServicesData, NzError> {
        let services = self
            .core
            .set_spotify_app(&client_id)
            .await
            .map_err(NzError::from)?;
        Ok(ServicesData::from(core_settings::services_view(&services)))
    }

    /// `GET /api/services/spotify/authorize` — the consent URL the platform's
    /// browser opens (the app never sees a password).
    pub async fn spotify_authorize(&self) -> Result<String, NzError> {
        self.core.spotify_authorize().await.map_err(NzError::from)
    }

    // --- the server's own settings (admin) --------------------------------

    pub fn cached_server_settings(&self) -> Option<ServerSettingsData> {
        self.core.cached_settings_server().map(|settings| {
            ServerSettingsData::from(core_settings::server_settings_view(&settings))
        })
    }

    /// `GET /api/settings` — the quota and the Deezer credential's state. A
    /// member is refused (`403`), and the screen draws no card.
    pub async fn server_settings(&self) -> Result<ServerSettingsData, NzError> {
        let settings = self.core.settings_server().await.map_err(NzError::from)?;
        Ok(ServerSettingsData::from(
            core_settings::server_settings_view(&settings),
        ))
    }

    /// `POST /api/settings` with `{upload_quota_gb}`.
    pub async fn save_upload_quota(&self, gb: f64) -> Result<ServerSettingsData, NzError> {
        let settings = self
            .core
            .save_upload_quota(gb)
            .await
            .map_err(NzError::from)?;
        Ok(ServerSettingsData::from(
            core_settings::server_settings_view(&settings),
        ))
    }

    /// `POST /api/settings` with `{deezer_arl}`. The server VERIFIES it before
    /// storing it, so a refusal leaves the previous one in place; an empty
    /// string goes back to the ARL of the server's own configuration.
    pub async fn save_deezer_arl(&self, arl: String) -> Result<ServerSettingsData, NzError> {
        // MB4's rule, reused: the shape is checked before the request, and the
        // value is stored on the SERVER only — never in this device's store.
        self.core
            .save_deezer_arl(&arl)
            .await
            .map_err(NzError::from)?;
        let settings = self.core.settings_server().await.map_err(NzError::from)?;
        Ok(ServerSettingsData::from(
            core_settings::server_settings_view(&settings),
        ))
    }

    /// `POST /api/settings` with `{deezer_arl: ""}` — back to the ARL of the
    /// server's own configuration.
    pub async fn clear_deezer_arl(&self) -> Result<ServerSettingsData, NzError> {
        let settings = self.core.clear_deezer_arl().await.map_err(NzError::from)?;
        Ok(ServerSettingsData::from(
            core_settings::server_settings_view(&settings),
        ))
    }

    pub fn cached_deezer_status(&self) -> Option<DeezerStatusData> {
        self.core
            .cached_deezer_status()
            .map(|status| DeezerStatusData::from(core_settings::deezer_status_view(&status)))
    }

    /// `GET /api/deezer/status` — is the credential still good? `force` re-tests
    /// it instead of reading the server's cached verdict (*Tester*).
    pub async fn deezer_status(&self, force: bool) -> Result<DeezerStatusData, NzError> {
        let status = self
            .core
            .deezer_status(force)
            .await
            .map_err(NzError::from)?;
        Ok(DeezerStatusData::from(core_settings::deezer_status_view(
            &status,
        )))
    }

    // --- "Je n'aime pas" --------------------------------------------------

    /// The device's copy of the list, as a windowed handle (rule 4).
    pub fn cached_dislikes(&self) -> Option<Arc<ListHandle>> {
        self.core
            .cached_disliked_tracks()
            .map(|rows| crate::lists::open_rows(&self.core, rows))
    }

    /// `GET /api/me/dislikes` — one request, then a `ListHandle`: a person with
    /// thousands of them scrolls without a second copy crossing the boundary.
    pub async fn dislikes(&self) -> Result<Arc<ListHandle>, NzError> {
        let rows = self.core.disliked_tracks().await.map_err(NzError::from)?;
        Ok(crate::lists::open_rows(&self.core, rows))
    }

    /// `POST /api/dislike` with `on = false` — the title comes back into the
    /// Flow. Optimistic on the screen, never retried by itself.
    pub async fn set_disliked(&self, row_id: String, on: bool) -> Result<(), NzError> {
        self.core
            .set_disliked(&row_id, on)
            .await
            .map_err(NzError::from)
    }
}

/// A row of a list crossing the boundary, for a caller that only wants the
/// keys (a test, a diagnostic): the same record the lists use.
pub fn row_of(handle: &ListHandle, start: u32, count: u32) -> Vec<TrackRow> {
    handle.window(start, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This device's settings cross with the SPA's own values, field by field —
    /// and never an ARL, because no record here has a field for it.
    #[test]
    fn the_local_settings_cross_with_the_spas_own_values() {
        let data = SettingsData::from(core_settings::LocalSettings::default());
        assert_eq!(data.theme, "encre");
        assert_eq!(data.accent, None);
        assert_eq!(data.viz_fps, 60);
        assert_eq!(data.download_quality, "OPUS_320");
        assert_eq!(data.cache_limit, 1024 * 1024 * 1024);
        assert!(data.prefetch);
        assert_eq!(data.prefetch_count, 1);
        // The player's own default, not the SPA's `FLAC`: the card reads the
        // player (`player.audio`), and a fresh device plays `Auto` (one truth).
        assert_eq!(data.quality, "AUTO");
    }

    /// A source crosses with the server's own words: its place, its rank, why it
    /// rests and how to set it up. The card draws them as they are.
    #[test]
    fn a_source_crosses_with_its_place_and_its_quality() {
        let data = SourceData::from(core_settings::SourceView {
            id: "youtube".into(),
            name: "YouTube".into(),
            place: "fallback".into(),
            rank: Some(2),
            enabled: true,
            configured: true,
            can_lead: false,
            switchable: true,
            quality_label: "Opus 160".into(),
            lossless: false,
            resting_s: 120,
            resting_why: "n'a pas répondu".into(),
            setup: String::new(),
        });
        assert_eq!(data.place, "fallback");
        assert_eq!(data.rank, Some(2));
        assert_eq!(data.quality_label, "Opus 160");
        assert_eq!(data.resting_s, 120, "the card says why and for how long");
    }

    /// A counts map crosses as rows, and a `None` phase means nothing runs — a
    /// zero is not a running import.
    #[test]
    fn a_quiet_account_carries_no_running_import() {
        let data = AccountData::from(core_settings::AccountView {
            id: "acc-1".into(),
            provider: "deezer".into(),
            name: "Deezer".into(),
            label: None,
            server: false,
            status: "ok".into(),
            is_default: false,
            kinds: vec!["tracks".into()],
            auto_on: vec!["tracks".into()],
            running: None,
            last: None,
            lossless: Some(true),
            error: None,
        });
        assert_eq!(data.running_phase, None);
        assert_eq!(data.running_done, 0);
        assert!(data.last_added.is_empty());
        assert_eq!(data.last_at, None);
    }
}
