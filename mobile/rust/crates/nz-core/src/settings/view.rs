//! The screen-shaped views of the server's answers.
//!
//! `nz-model` decodes the routes faithfully, which means a few fields are still
//! `serde_json::Value`: an import's `running`/`last` report, a grant map. Reading
//! INSIDE them here — once, beside the rules — is what lets `nz-ffi` stay a
//! converter with no JSON of its own (`nz-ffi` carries no `serde_json`
//! dependency, and a lot may not add one to a shared manifest).
//!
//! Nothing here invents a field the server does not send: an absent key is
//! `None`, a zero count is not news.

use std::collections::BTreeMap;

use nz_model::{
    Access, Account, DeezerStatus, MyAccounts, ServerSettings, Service, Services, Source, Sources,
};

/// One `{kind: count}` entry of an import's report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountView {
    pub kind: String,
    pub count: u32,
}

/// What a running import is doing (`MyAccounts.svelte#progress`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningView {
    /// `connect`, `list` or `import`.
    pub phase: String,
    /// The kind being imported, when the phase says.
    pub kind: Option<String>,
    pub done: u32,
    pub total: u32,
}

/// The last import's outcome (`MyAccounts.svelte#statusLine`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LastView {
    /// ISO-8601, the server's own clock.
    pub at: Option<String>,
    /// `network` or `auth` when it was a known failure.
    pub reason: Option<String>,
    pub error: Option<String>,
    /// The kinds that came in, `None` when nothing did.
    pub added: Vec<CountView>,
    /// The kinds already here, left as they were.
    pub kept: Vec<CountView>,
}

/// One account, as *Mes comptes* draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountView {
    pub id: String,
    pub provider: String,
    pub name: String,
    pub label: Option<String>,
    pub server: bool,
    /// `ok`, `auth`, `broken`, … (empty when the server said nothing).
    pub status: String,
    pub is_default: bool,
    pub kinds: Vec<String>,
    /// The kinds that import automatically.
    pub auto_on: Vec<String>,
    pub running: Option<RunningView>,
    pub last: Option<LastView>,
    /// `false` when the account has no FLAC (a free Deezer offer).
    pub lossless: Option<bool>,
    /// The KIND of a failure only, when the account is broken.
    pub error: Option<String>,
}

/// One grant: who, and which features.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantView {
    pub subject: String,
    pub features: Vec<String>,
}

/// A person a grant may name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonView {
    pub id: String,
    pub name: String,
}

/// One feature a provider offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureView {
    pub id: String,
    pub scope: String,
}

/// One account a provider is reached through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessAccountView {
    pub id: String,
    pub mode: String,
    pub label: Option<String>,
    pub server: bool,
    pub owner: Option<String>,
    pub mine: bool,
    pub editable: bool,
    pub grants: Vec<GrantView>,
}

/// One provider, with its accounts and its features.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderView {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    pub accounts: Vec<AccessAccountView>,
    pub allowed: Vec<String>,
    pub anonymous_grants: Vec<GrantView>,
    pub anonymous_editable: bool,
    pub features: Vec<FeatureView>,
}

/// A group of people a grant can name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupView {
    pub id: String,
    pub name: String,
    /// Present for an administrator only (the server decides).
    pub members: Vec<PersonView>,
}

/// `/api/access`, screen-shaped.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessView {
    pub providers: Vec<ProviderView>,
    pub groups: Vec<GroupView>,
    pub users: Vec<PersonView>,
    pub admin: bool,
}

/// One export target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceView {
    pub id: String,
    pub name: String,
    pub linked: bool,
    pub ready: bool,
    pub account: Option<String>,
    pub file: bool,
    pub client_id: Option<String>,
}

/// `/api/services`, screen-shaped (the formats as their names).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServicesView {
    pub services: Vec<ServiceView>,
    pub formats: Vec<String>,
}

/// `{kind: count}` with the zeroes dropped: a report of nothing is no news.
fn counts(map: Option<&serde_json::Value>) -> Vec<CountView> {
    let Some(map) = map.and_then(|value| value.as_object()) else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(kind, value)| {
            let count = value.as_u64()?;
            (count > 0).then(|| CountView {
                kind: kind.clone(),
                count: count.try_into().unwrap_or(u32::MAX),
            })
        })
        .collect()
}

fn text(value: Option<&serde_json::Value>, field: &str) -> Option<String> {
    value
        .and_then(|value| value.get(field))
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

fn number(value: Option<&serde_json::Value>, field: &str) -> u32 {
    value
        .and_then(|value| value.get(field))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
        .try_into()
        .unwrap_or(u32::MAX)
}

/// One account, screen-shaped.
pub fn account_view(account: &Account) -> AccountView {
    let running = account.running.as_ref();
    let last = account.last.as_ref();
    AccountView {
        id: account.id.clone(),
        provider: account.provider.clone(),
        name: account.name.clone(),
        label: account.label.clone(),
        server: account.server,
        status: account.status.clone().unwrap_or_default(),
        is_default: account.default,
        kinds: account.kinds.clone(),
        auto_on: account
            .auto
            .iter()
            .filter(|(_, value)| value.as_bool() == Some(true))
            .map(|(kind, _)| kind.clone())
            .collect(),
        running: running.map(|value| RunningView {
            phase: text(Some(value), "phase").unwrap_or_else(|| "connect".to_owned()),
            kind: text(Some(value), "kind"),
            done: number(Some(value), "done"),
            total: number(Some(value), "total"),
        }),
        last: last.map(|value| LastView {
            at: text(Some(value), "at"),
            reason: text(Some(value), "reason"),
            error: text(Some(value), "error"),
            added: counts(value.get("added")),
            kept: counts(value.get("kept")),
        }),
        lossless: account.lossless,
        error: account.error.clone(),
    }
}

/// `/api/me/accounts`, screen-shaped: the accounts, the providers still
/// linkable, and the default one.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountsView {
    pub accounts: Vec<AccountView>,
    pub can_link: Vec<String>,
    pub default: Option<String>,
}

/// `/api/me/accounts`, screen-shaped.
pub fn accounts_view(accounts: &MyAccounts) -> AccountsView {
    AccountsView {
        accounts: accounts.accounts.iter().map(account_view).collect(),
        can_link: accounts
            .can_link
            .iter()
            .filter(|(_, allowed)| **allowed)
            .map(|(provider, _)| provider.clone())
            .collect(),
        default: accounts.default.clone(),
    }
}

/// One grant map, screen-shaped, in the subject's alphabetical order.
pub fn grants(map: &BTreeMap<String, Vec<String>>) -> Vec<GrantView> {
    map.iter()
        .map(|(subject, features)| GrantView {
            subject: subject.clone(),
            features: features.clone(),
        })
        .collect()
}

/// `/api/access`, screen-shaped.
pub fn access_view(access: &Access) -> AccessView {
    AccessView {
        providers: access
            .providers
            .iter()
            .map(|provider| ProviderView {
                id: provider.id.clone(),
                name: provider.name.clone(),
                role: provider.role.clone(),
                accounts: provider
                    .accounts
                    .iter()
                    .map(|account| AccessAccountView {
                        id: account.id.clone(),
                        mode: account.mode.clone(),
                        label: account.label.clone(),
                        server: account.server,
                        owner: account.owner.as_ref().map(|user| user.name.clone()),
                        mine: account.mine,
                        editable: account.editable,
                        grants: grants(&account.grants),
                    })
                    .collect(),
                allowed: provider.allowed.clone(),
                anonymous_grants: provider
                    .anonymous_use
                    .as_ref()
                    .map(|anonymous| grants(&anonymous.grants))
                    .unwrap_or_default(),
                anonymous_editable: provider
                    .anonymous_use
                    .as_ref()
                    .map(|anonymous| anonymous.editable)
                    .unwrap_or(false),
                features: provider
                    .features
                    .iter()
                    .map(|feature| FeatureView {
                        id: feature.id.clone(),
                        scope: feature.scope.clone(),
                    })
                    .collect(),
            })
            .collect(),
        groups: access
            .groups
            .iter()
            .map(|group| GroupView {
                id: group.id.clone(),
                name: group.name.clone(),
                members: group
                    .members
                    .as_ref()
                    .map(|members| members.iter().map(person_view).collect())
                    .unwrap_or_default(),
            })
            .collect(),
        users: access
            .users
            .as_ref()
            .map(|users| users.iter().map(person_view).collect())
            .unwrap_or_default(),
        admin: access.admin,
    }
}

fn person_view(user: &nz_model::AccessUser) -> PersonView {
    PersonView {
        id: user.id.to_string(),
        name: user.name.clone(),
    }
}

/// One export target, screen-shaped.
pub fn service_view(service: &Service) -> ServiceView {
    ServiceView {
        id: service.id.clone(),
        name: service.name.clone(),
        linked: service.linked,
        ready: service.ready,
        account: service.account.clone(),
        file: service.file.unwrap_or(false),
        client_id: service.client_id.clone(),
    }
}

/// `/api/services`, screen-shaped.
pub fn services_view(services: &Services) -> ServicesView {
    ServicesView {
        services: services.services.iter().map(service_view).collect(),
        formats: services
            .formats
            .iter()
            .map(|format| format.label.clone())
            .collect(),
    }
}

/// One source of sound, in the order the server asks them.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceView {
    pub id: String,
    pub name: String,
    /// `main`, `fallback` or `off` — never empty: a server that predates the
    /// field reads as `off`, so it cannot look like the leading source.
    pub place: String,
    pub rank: Option<i64>,
    pub enabled: bool,
    pub configured: bool,
    pub can_lead: bool,
    pub switchable: bool,
    pub quality_label: String,
    pub lossless: bool,
    /// Seconds the source is left alone after a transport failure.
    pub resting_s: u32,
    pub resting_why: String,
    pub setup: String,
}

/// `/api/sources`, screen-shaped.
#[derive(Debug, Clone, PartialEq)]
pub struct SourcesView {
    pub sources: Vec<SourceView>,
    pub main: Option<String>,
    pub admin: bool,
}

/// One source, screen-shaped.
pub fn source_view(source: &Source) -> SourceView {
    SourceView {
        id: source.id.clone(),
        name: source.name.clone(),
        place: source.place.clone().unwrap_or_else(|| "off".to_owned()),
        rank: source.rank,
        enabled: source.enabled,
        configured: source.configured,
        can_lead: source.can_lead,
        switchable: source.switchable,
        quality_label: source.quality.label.clone(),
        lossless: source.quality.lossless,
        resting_s: source.resting,
        resting_why: source.resting_why.clone(),
        setup: source.setup.clone(),
    }
}

/// `/api/sources`, screen-shaped.
pub fn sources_view(sources: &Sources) -> SourcesView {
    SourcesView {
        sources: sources.sources.iter().map(source_view).collect(),
        main: sources.main.clone(),
        admin: sources.admin,
    }
}

/// The Deezer credential's STATE — never the credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeezerCardView {
    pub arl_set: bool,
    pub arl_source: Option<String>,
    pub arl_hint: Option<String>,
    pub enabled: bool,
    pub archive_dir_set: bool,
}

/// `/api/settings`, screen-shaped.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerSettingsView {
    /// The upload quota for non-admins, in GB (0 = no limit).
    pub upload_quota_gb: f64,
    pub deezer: DeezerCardView,
}

/// `/api/settings`, screen-shaped.
pub fn server_settings_view(settings: &ServerSettings) -> ServerSettingsView {
    ServerSettingsView {
        upload_quota_gb: settings.upload_quota_gb,
        deezer: DeezerCardView {
            arl_set: settings.deezer.arl_set,
            arl_source: settings.deezer.arl_source.clone(),
            arl_hint: settings.deezer.arl_hint.clone(),
            enabled: settings.deezer.enabled,
            archive_dir_set: settings.deezer.archive_dir_set,
        },
    }
}

/// `/api/deezer/status`, screen-shaped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeezerStatusView {
    pub ok: bool,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub admin: bool,
}

/// `/api/deezer/status`, screen-shaped.
pub fn deezer_status_view(status: &DeezerStatus) -> DeezerStatusView {
    DeezerStatusView {
        ok: status.ok,
        reason: status.reason.clone(),
        message: status.message.clone(),
        admin: status.admin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An import's report: the zeroes are not news, a missing key is `None`, and
    /// the running job keeps its own phase and counts.
    #[test]
    fn an_imports_report_is_read_once() {
        let account = Account {
            id: "acc-1".into(),
            provider: "deezer".into(),
            name: "Deezer".into(),
            kinds: vec!["tracks".into(), "albums".into()],
            auto: BTreeMap::from([
                ("tracks".to_owned(), serde_json::json!(true)),
                ("albums".to_owned(), serde_json::json!(false)),
            ]),
            running: Some(
                serde_json::json!({"phase":"import","kind":"tracks","done":40,"total":120}),
            ),
            last: Some(serde_json::json!({
                "at":"2026-11-17T10:00:00Z",
                "added":{"tracks":12,"playlists":0},
                "kept":{"albums":3}
            })),
            ..Account::default()
        };
        let view = account_view(&account);
        assert_eq!(view.auto_on, vec!["tracks"]);
        let running = view.running.expect("the running job");
        assert_eq!(running.phase, "import");
        assert_eq!((running.done, running.total), (40, 120));
        let last = view.last.expect("the last import");
        assert_eq!(last.at.as_deref(), Some("2026-11-17T10:00:00Z"));
        assert_eq!(
            last.added,
            vec![CountView {
                kind: "tracks".into(),
                count: 12
            }],
            "a zero is not news"
        );
        assert_eq!(last.kept.len(), 1);
        assert_eq!(view.lossless, None, "the server said nothing about FLAC");
    }

    /// A grant map becomes rows in the subjects' own order, and a group's
    /// members are `None` for a member (the server never sends them).
    #[test]
    fn grants_and_groups_cross_as_rows() {
        let access = Access {
            admin: true,
            groups: vec![nz_model::AccessGroup {
                id: "g1".into(),
                name: "Maison".into(),
                members: Some(vec![nz_model::AccessUser {
                    id: nz_model::UserId::new("u2"),
                    name: "Léo".into(),
                }]),
            }],
            providers: vec![nz_model::AccessProvider {
                id: "deezer".into(),
                name: "Deezer".into(),
                features: vec![nz_model::ProviderFeature {
                    id: "search".into(),
                    scope: "shared".into(),
                    ..nz_model::ProviderFeature::default()
                }],
                ..nz_model::AccessProvider::default()
            }],
            ..Access::default()
        };
        let view = access_view(&access);
        assert_eq!(view.groups[0].members[0].name, "Léo");
        assert_eq!(view.providers[0].features[0].scope, "shared");
        assert!(view.users.is_empty(), "a member lends nothing");
    }
}
