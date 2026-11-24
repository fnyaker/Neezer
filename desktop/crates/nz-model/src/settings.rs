//! Who the person is, what they may use, the sources of sound, and the server's
//! own settings/status payloads (Réglages).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::ids::UserId;

/// One source of sound as `/api/me` lists it (a small summary).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UserSource {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// `"lead"`, `"off"`, `"resting"`…
    #[serde(default)]
    pub place: Option<String>,
    /// The quality label it serves (`"FLAC"`, `"Opus 160k"`).
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub lossless: bool,
    #[serde(default)]
    pub rank: Option<i64>,
}

/// The signed-in person, as `/api/me` describes them.
///
/// `access` is `{provider: [feature ids]}`, already resolved by the server: the
/// UI offers nothing this map refuses, and the server stays the enforcement.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub name: String,
    #[serde(default)]
    pub admin: bool,
    /// `"done"`, `"artists"`, `"account"`… — where the onboarding stands.
    #[serde(default)]
    pub onboarding: Option<String>,
    #[serde(default)]
    pub access: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub sources: Vec<UserSource>,
}

/// The grant a remote-control session runs under (`/api/me`'s `remote`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoteGrant {
    /// `"queue"`, `"read"`, `"full"` or `"admin"` — the ceilings the server
    /// enforces; the UI hides what is above it.
    #[serde(default)]
    pub level: String,
    /// The person whose player is being driven.
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub expires: Option<String>,
}

/// `/api/me` — the session's own answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Me {
    pub user: User,
    /// Present only when the session is a remote-control LOAN, not a login.
    #[serde(default)]
    pub remote: Option<RemoteGrant>,
}

impl Me {
    /// Whether this person may use one feature of one provider.
    pub fn allows(&self, provider: &str, feature: &str) -> bool {
        self.user
            .access
            .get(provider)
            .is_some_and(|features| features.iter().any(|f| f == feature || f == "all"))
    }
}

/// One field a provider's feature needs (an `isrc`, a title, …).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FeatureField {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
}

/// One capability a provider offers (search, stream, browse, …).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderFeature {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    /// `"none"` or `"account"`: whether it needs the person's account.
    #[serde(default)]
    pub auth: String,
    /// `"shared"` or `"personal"`.
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub help: String,
    #[serde(default)]
    pub fields: Vec<FeatureField>,
}

/// One way to sign a provider in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AuthMode {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub help: String,
    /// Only the provider's own app can do it (no password through the server).
    #[serde(default)]
    pub app_only: bool,
}

/// One account lent to (or held by) the person, as `/api/access` lists it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AccessAccount {
    #[serde(default)]
    pub id: String,
    /// `"server"` or `"personal"`.
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub server: bool,
    #[serde(default)]
    pub owner: Option<AccessUser>,
    #[serde(default)]
    pub mine: bool,
    #[serde(default)]
    pub editable: bool,
    /// `{user or group id, or "*": [feature ids or "all"]}`.
    #[serde(default)]
    pub grants: BTreeMap<String, Vec<String>>,
}

/// A person named in a grant.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AccessUser {
    pub id: UserId,
    #[serde(default)]
    pub name: String,
}

/// The anonymous-use block of a provider: who may use it without an account.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnonymousUse {
    #[serde(default)]
    pub editable: bool,
    #[serde(default)]
    pub grants: BTreeMap<String, Vec<String>>,
}

/// One provider in `/api/access`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AccessProvider {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// `"main"`, `"fallback"`, `"info"`.
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub account: bool,
    #[serde(default)]
    pub accounts: Vec<AccessAccount>,
    /// The features THIS person may use, already resolved.
    #[serde(default)]
    pub allowed: Vec<String>,
    #[serde(default)]
    pub anonymous: bool,
    #[serde(default)]
    pub anonymous_use: Option<AnonymousUse>,
    #[serde(default)]
    pub auth_modes: Vec<AuthMode>,
    #[serde(default)]
    pub features: Vec<ProviderFeature>,
}

/// A group of people a grant can name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AccessGroup {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Present for an admin only.
    #[serde(default)]
    pub members: Option<Vec<AccessUser>>,
}

/// `/api/access` — who may use which provider, and who a grant may name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Access {
    #[serde(default)]
    pub me: UserId,
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub providers: Vec<AccessProvider>,
    #[serde(default)]
    pub groups: Vec<AccessGroup>,
    /// Present only when the caller has something to lend.
    #[serde(default)]
    pub users: Option<Vec<AccessUser>>,
}

/// One personal account in `/api/me/accounts`.
///
/// `auto` / `last` / `running` are the import scheduler's own little objects;
/// they stay untyped until a screen reads inside them (D14a).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub server: bool,
    /// `"ok"`, `"broken"`, …
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub auto: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub last: Option<serde_json::Value>,
    #[serde(default)]
    pub lossless: Option<bool>,
    #[serde(default)]
    pub running: Option<serde_json::Value>,
    #[serde(default)]
    pub default: bool,
    /// The import kinds this account can bring in (`"playlists"`, `"tracks"`, …).
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub pickable: Vec<String>,
    /// The KIND of a failure only, when the account is broken.
    #[serde(default)]
    pub error: Option<String>,
}

/// `/api/me/accounts` — this person's own linked accounts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MyAccounts {
    #[serde(default)]
    pub accounts: Vec<Account>,
    /// `{provider: can link}` — a provider whose server account already holds
    /// the credential is not linkable a second time.
    #[serde(default)]
    pub can_link: BTreeMap<String, bool>,
    #[serde(default)]
    pub default: Option<String>,
}

/// The quality a source serves, as `/api/sources` spells it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceQuality {
    /// Deezer's own quality class (40 = FLAC, 30 = 320 kbps, …).
    #[serde(default, rename = "class")]
    pub class: u32,
    #[serde(default)]
    pub kbps: u32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub lossless: bool,
}

/// One source of sound in `/api/sources` (the order, and why one rests).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Source {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    /// The credential/setup is complete.
    #[serde(default)]
    pub configured: bool,
    /// Can be the main source at all.
    #[serde(default)]
    pub can_lead: bool,
    /// The person may move it in the order.
    #[serde(default)]
    pub switchable: bool,
    /// `"lead"`, `"off"`, `"resting"`.
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub rank: Option<i64>,
    #[serde(default)]
    pub quality: SourceQuality,
    /// A countdown the server is serving ("resting" after a transport failure).
    #[serde(default)]
    pub resting: u32,
    #[serde(default)]
    pub resting_why: String,
    /// The French sentence telling the person how to set it up.
    #[serde(default)]
    pub setup: String,
}

/// `/api/sources` — the sources in their order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Sources {
    #[serde(default)]
    pub sources: Vec<Source>,
    /// The source leading right now, `null` when none does.
    #[serde(default)]
    pub main: Option<String>,
    #[serde(default)]
    pub admin: bool,
}

/// The Deezer block of `/api/settings`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerDeezerSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub arl_set: bool,
    /// `"config"`, `"env"` or `"ui"` — where the ARL came from.
    #[serde(default)]
    pub arl_source: Option<String>,
    /// A masked hint of the ARL, never the ARL itself.
    #[serde(default)]
    pub arl_hint: Option<String>,
    #[serde(default)]
    pub archive_dir_set: bool,
}

/// `/api/settings` — the admin's server-wide settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerSettings {
    #[serde(default)]
    pub deezer: ServerDeezerSettings,
    /// The upload quota in GB; `0` means no limit.
    #[serde(default)]
    pub upload_quota_gb: f64,
}

/// `/api/storage` — what the archive costs and what is left.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Storage {
    #[serde(default)]
    pub archive_dir: String,
    #[serde(default)]
    pub archive_bytes: u64,
    #[serde(default)]
    pub cache_bytes: u64,
    #[serde(default)]
    pub cache_limit: u64,
    #[serde(default)]
    pub transcode_bytes: u64,
    #[serde(default)]
    pub transcode_limit: u64,
    #[serde(default)]
    pub disk_free: u64,
    #[serde(default)]
    pub disk_total: u64,
    #[serde(default)]
    pub tracks_archived: u32,
    #[serde(default)]
    pub tracks_total: u32,
}

/// `/api/deezer/status` — whether the account credential still works.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeezerStatus {
    #[serde(default)]
    pub ok: bool,
    /// `"disabled"`, `"no_arl"`, `"expired"`, `"unreachable"`…
    #[serde(default)]
    pub reason: Option<String>,
    /// The French sentence the settings screen shows.
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub admin: bool,
}

/// The Android release the server knows about (`/api/version`'s `android`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AndroidRelease {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub url: String,
    /// An APK is published for that version.
    #[serde(default)]
    pub apk: bool,
    /// The NATIVE app's own channel (`fr.neezer.mobile`). A server that predates
    /// it sends no key at all: the native app then has no update to offer and
    /// must never fall back to the WebView `apk` above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<AndroidNativeRelease>,
}

/// The native Android app's channel (`/api/version`'s `android.native`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AndroidNativeRelease {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub url: String,
    /// `/api/android/native-apk` serves it.
    #[serde(default)]
    pub apk: bool,
    /// The ABIs the release carries a split for.
    #[serde(default)]
    pub abis: Vec<String>,
}

/// `/api/version` — the server's build identity.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Version {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub build: Option<String>,
    #[serde(default)]
    pub android: AndroidRelease,
}

/// One export format (`/api/export/formats`, `/api/services`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExportFormat {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub ext: String,
}

/// `/api/export/formats` — the ZIP export's formats.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExportFormats {
    /// The format the sheet starts on.
    #[serde(default)]
    pub default: String,
    #[serde(default)]
    pub formats: Vec<ExportFormat>,
}

/// One streaming service `/api/services` can export to (or list for).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Service {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub linked: bool,
    /// The credential is in place, so an export can run.
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub account: Option<String>,
    /// Spotify only: the app's client id, empty when unset.
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub redirect_here: Option<String>,
    /// A service with no door: it exports a list FILE instead.
    #[serde(default)]
    pub file: Option<bool>,
}

/// `/api/services` — the export targets and the list formats.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Services {
    #[serde(default)]
    pub services: Vec<Service>,
    #[serde(default)]
    pub formats: Vec<ExportFormat>,
}

/// `/api/upload/usage` — this person's uploads against the quota.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UploadUsage {
    #[serde(default)]
    pub used: u64,
    #[serde(default)]
    pub quota: u64,
    #[serde(default)]
    pub unlimited: bool,
}

/// `/api/sync/status` — is the Deezer sync running?
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncStatus {
    #[serde(default)]
    pub running: bool,
}

/// `/api/archive/status` — the archive backfill's progress.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArchiveStatus {
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub done: u32,
    #[serde(default)]
    pub archived: u32,
    #[serde(default)]
    pub failed: u32,
    #[serde(default)]
    pub unavailable: u32,
    #[serde(default)]
    pub error: Option<String>,
}

/// The archiving/cleanup rule set (identical shape for `rules` and `defaults`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArchiveRuleSet {
    #[serde(default)]
    pub on_fav_track: bool,
    #[serde(default)]
    pub on_fav_album: bool,
    #[serde(default)]
    pub on_fav_artist: bool,
    #[serde(default)]
    pub on_fav_playlist: bool,
    #[serde(default)]
    pub on_playlist_add: bool,
    #[serde(default)]
    pub on_play_context: bool,
    #[serde(default)]
    pub on_podcast: bool,
    #[serde(default)]
    pub artist_scope: String,
    #[serde(default)]
    pub artist_limit: u32,
    #[serde(default)]
    pub chapters: bool,
    #[serde(default)]
    pub chapter_urls: bool,
    #[serde(default)]
    pub clean_on: bool,
    #[serde(default)]
    pub clean_stale_days: u32,
    /// Gigabytes free below which a cleanup may run — a FLOAT on the wire.
    #[serde(default)]
    pub clean_min_free_gb: f64,
    #[serde(default)]
    pub clean_order: String,
    #[serde(default)]
    pub clean_keep_fav: bool,
    #[serde(default)]
    pub clean_keep_playlist: bool,
    #[serde(default)]
    pub clean_keep_podcast: bool,
}

/// `/api/archive/rules` — the rules, their defaults, and the choices to draw.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArchiveRules {
    #[serde(default)]
    pub rules: ArchiveRuleSet,
    #[serde(default)]
    pub defaults: ArchiveRuleSet,
    /// The event rule ids, in the screen's order.
    #[serde(default)]
    pub events: Vec<String>,
    /// The artist scopes the picker offers.
    #[serde(default)]
    pub artist_scopes: Vec<String>,
    /// The cleanup orders the picker offers.
    #[serde(default)]
    pub cleanup_orders: Vec<String>,
    /// Whether "archive my whole library" is on.
    #[serde(default)]
    pub archive_library: bool,
}

/// `/api/archive/cleanup/preview` — what a cleanup would delete right now.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CleanupPreview {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub running: bool,
    /// How many archived tracks match the policy.
    #[serde(default)]
    pub eligible: u32,
    #[serde(default)]
    pub would_free: u64,
    #[serde(default)]
    pub free_bytes: u64,
    #[serde(default)]
    pub needed_bytes: u64,
    /// The first rows of the list; the screen shows them, and their shape is
    /// the archive screen's own (D14b).
    #[serde(default)]
    pub tracks: Vec<serde_json::Value>,
}

/// `/api/archive/migrate` — the archived files' format migration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MigrateStatus {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub done: u32,
    #[serde(default)]
    pub failed: u32,
    #[serde(default)]
    pub error: Option<String>,
    /// Per-step counters (`tags`, `sidecar`, `bpm`, `bpm_unreachable`, `unchanged`).
    #[serde(default)]
    pub counts: BTreeMap<String, u32>,
    /// The failures' own rows; the archive screen names them (D14b).
    #[serde(default)]
    pub failures: Vec<serde_json::Value>,
    #[serde(default)]
    pub failure_reasons: BTreeMap<String, u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_carries_the_resolved_access_map() {
        let me: Me = serde_json::from_str(
            r#"{"user":{"id":"fab4c584","name":"bench","admin":true,"onboarding":"done",
                "access":{"youtube":["search","stream"]},
                "sources":[{"id":"deezer","lossless":true,"name":"Deezer",
                            "place":"off","quality":"FLAC","rank":null}]}}"#,
        )
        .expect("me");
        assert!(me.allows("youtube", "search"));
        assert!(me.allows("youtube", "stream"));
        assert!(
            !me.allows("youtube", "browse"),
            "a feature the map omits is refused"
        );
        assert!(
            !me.allows("deezer", "stream"),
            "a provider absent from the map is refused"
        );
        assert!(me.remote.is_none(), "an ordinary login is not a loan");
    }

    /// A server that predates the native channel sends no `native` key: parsing
    /// must leave it absent so the app can say "mise à jour indisponible" —
    /// never read the WebView `android.apk` beside it.
    #[test]
    fn a_version_without_the_native_channel_keeps_it_absent() {
        let old: Version = serde_json::from_str(
            r#"{"version":"1.5.0","android":{"version":"1.5.0","url":"https://x/","apk":true}}"#,
        )
        .expect("old server");
        assert!(old.android.native.is_none());
        assert!(old.android.apk, "the WebView channel is still there");

        let new: Version = serde_json::from_str(
            r#"{"version":"1.6.3","android":{"version":"1.6.3","url":"https://x/","apk":true,
                "native":{"version":"1.6.3","url":"https://x/","apk":true,
                          "abis":["arm64-v8a","x86_64"]}}}"#,
        )
        .expect("new server");
        let native = new.android.native.expect("the native channel");
        assert_eq!(native.version.as_deref(), Some("1.6.3"));
        assert!(native.apk);
        assert_eq!(native.abis, ["arm64-v8a", "x86_64"]);
    }

    #[test]
    fn a_source_carries_its_quality_and_why_it_is_not_leading() {
        let sources: Sources = serde_json::from_str(
            r#"{"admin":true,"main":null,"sources":[{"can_lead":true,"configured":false,
                "enabled":false,"id":"deezer","name":"Deezer","place":"off",
                "quality":{"class":40,"kbps":0,"label":"FLAC","lossless":true},
                "rank":null,"resting":0,"resting_why":"","setup":"Aucun ARL.","switchable":false}]}"#,
        )
        .expect("sources");
        assert_eq!(sources.sources[0].quality.class, 40);
        assert!(sources.sources[0].quality.lossless);
    }
}

// -- onboarding -------------------------------------------------------------
// The first-run flow: the artists feed, the search beside it, and the account
// step (link or import). Everything here is read only by `/api/onboarding*`.

/// What the onboarding says about one provider's account step.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardingProvider {
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub linked: bool,
    #[serde(default)]
    pub auth_modes: Vec<AuthMode>,
    /// The provider's own account, when one is linked.
    #[serde(default)]
    pub account: Option<String>,
    /// `/api/onboarding/account/options` only.
    #[serde(default)]
    pub can_link: Option<bool>,
    #[serde(default)]
    pub denied: Option<bool>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub installed: Option<bool>,
    #[serde(default)]
    pub desktop: Option<bool>,
    /// Why the step cannot be offered (`"absent"`, `"deezer_disabled"`, …).
    #[serde(default)]
    pub reason: Option<String>,
}

/// One way to bring a library in during onboarding (a link, or a list file).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardingImport {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub label: String,
    /// `"link"` or `"public_link"`.
    #[serde(default)]
    pub mode: String,
    /// `/api/onboarding/account/options` only: whether it would work now.
    #[serde(default)]
    pub available: Option<bool>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// The account step's summary on `/api/onboarding`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardingAccount {
    #[serde(default)]
    pub can_save_imports: bool,
    #[serde(default)]
    pub deezer: OnboardingProvider,
    #[serde(default)]
    pub youtube: OnboardingProvider,
    #[serde(default)]
    pub imports: Vec<OnboardingImport>,
}

/// The artists feed's own state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardingFeed {
    /// `"library"`, `"deezer"`, …
    #[serde(default)]
    pub source: String,
    /// `"ready"`, `"building"`, `"stale"`, …
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub building: bool,
    #[serde(default)]
    pub stale: bool,
    /// Kept apart from the pay-to-read keys: the person's own library artists.
    #[serde(default)]
    pub kept: u32,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub build: Option<serde_json::Value>,
    #[serde(default)]
    pub progress: Option<f64>,
}

/// The mix's diversity bounds (how many of one family the feed may show).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardingDiversity {
    #[serde(default)]
    pub window: u32,
    #[serde(default)]
    pub max_same_genre: u32,
    #[serde(default)]
    pub max_streak: u32,
    #[serde(default)]
    pub chart_max: u32,
    #[serde(default)]
    pub country_max: u32,
    #[serde(default)]
    pub country_window: u32,
    #[serde(default)]
    pub region_max: u32,
    #[serde(default)]
    pub region_window: u32,
}

/// `/api/onboarding` — where the first-run flow stands.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardingState {
    /// `"artists"`, `"account"`, `"done"`.
    #[serde(default)]
    pub step: String,
    #[serde(default)]
    pub liked: u32,
    #[serde(default)]
    pub min_artists: u32,
    /// What the feed fell back to (`"library"`).
    #[serde(default)]
    pub fallback: String,
    #[serde(default)]
    pub feed: OnboardingFeed,
    #[serde(default)]
    pub diversity: OnboardingDiversity,
    #[serde(default)]
    pub account: OnboardingAccount,
    /// The YouTube step's own state.
    #[serde(default)]
    pub youtube: OnboardingYouTube,
}

/// The YouTube step of the onboarding.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardingYouTube {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub denied: bool,
    #[serde(default)]
    pub held: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub retry_in: u32,
}

/// One artist tile of the feed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardingArtist {
    /// The provider ref the like button sends (`deezer_id` for a catalogue row).
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub deezer_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub picture: Option<String>,
    #[serde(default)]
    pub picture_small: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    /// `"deezer"` or `"local"`.
    #[serde(default)]
    pub provider: String,
    /// Which source the tile came from (`"library"`, `"related"`, …).
    #[serde(default)]
    pub via: Option<String>,
    #[serde(default)]
    pub liked: bool,
    /// The artist has a page the library can open.
    #[serde(default)]
    pub openable: bool,
    /// The tile this one is a neighbour of.
    #[serde(default)]
    pub parent: Option<String>,
    /// Why this tile is here, in the server's words.
    #[serde(default)]
    pub hint: Option<String>,
}

/// `/api/onboarding/artists?after=&limit=` — one page of the feed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardingArtists {
    #[serde(default)]
    pub items: Vec<OnboardingArtist>,
    /// The cursor of the next page, `null` on the last one.
    #[serde(default)]
    pub next: Option<String>,
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub seed: Option<String>,
    #[serde(default)]
    pub partial: bool,
    #[serde(default)]
    pub degraded: bool,
    #[serde(default)]
    pub building: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub build: Option<serde_json::Value>,
}

/// `/api/onboarding/search?q=` — the feed's own search box.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardingSearch {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub items: Vec<OnboardingArtist>,
    /// How many hits the LIBRARY itself had.
    #[serde(default)]
    pub local: u32,
    #[serde(default)]
    pub partial: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// `/api/onboarding/account/options` — what the account step may offer now.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardingAccountOptions {
    #[serde(default)]
    pub step: String,
    #[serde(default)]
    pub can_save_imports: bool,
    #[serde(default)]
    pub deezer: OnboardingProvider,
    #[serde(default)]
    pub youtube: OnboardingProvider,
    #[serde(default)]
    pub imports: Vec<OnboardingImport>,
    /// The import job the step started, when one is running.
    #[serde(default)]
    pub job: Option<serde_json::Value>,
}
