//! The network service: one `Api` per server, the session file, reachability.
//!
//! Everything here is the app's policy and none of it is HTTP: `nz-api` owns
//! the request, the retry, the lanes and the cookie jar. This module decides
//! *which* server the app speaks to, warns before a password crosses an
//! unencrypted link to a public address, and turns `Api::online()` into the
//! events the network chip and `LoadFailed` read.

use std::sync::Arc;

use nz_api::{Api, ApiError, SessionJar};
use nz_cache::Cache;
use reqwest::Url;

use crate::state::AppEvent;
use crate::wiring::events::Sender;

/// The default when nothing is stored yet: the server a self-hosted Neezer
/// listens on. The login screen shows it as the prefilled address, and it is
/// editable there (rule 6: the server address is a setting, in the UI).
pub const DEFAULT_SERVER: &str = "http://127.0.0.1:5000";

/// The key the chosen server lives under in the local `kv` store.
pub const SERVER_KEY: &str = "server.url";

/// Clean up what a person typed into the address field.
pub fn normalise(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("indiquez l'adresse du serveur Neezer".to_owned());
    }
    let with_scheme = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("http://{raw}")
    };
    let url = Url::parse(&with_scheme)
        .map_err(|error| format!("adresse du serveur invalide : {error}"))?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("protocole non pris en charge : {other}")),
    }
    if url.host_str().is_none() {
        return Err("adresse du serveur sans nom d'hôte".to_owned());
    }
    Ok(with_scheme.trim_end_matches('/').to_owned())
}

/// The warning the login screen shows for a plain-HTTP address that is not on
/// the local network. `None` when the link is TLS or the host is private.
pub fn plain_http_warning(server: &str) -> Option<&'static str> {
    let url = Url::parse(server).ok()?;
    if url.scheme() != "http" {
        return None;
    }
    let host = url.host_str()?;
    (!is_private_host(host)).then_some(
        "Connexion non chiffrée vers une adresse distante : le mot de passe circulera en clair \
         sur le réseau. Préférez https:// pour un serveur exposé.",
    )
}

/// Loopback, RFC 1918, link-local, a `.local` name or `localhost`: the LAN
/// installs the plan allows over plain HTTP.
pub fn is_private_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".local") || host.ends_with(".lan") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(std::net::IpAddr::V6(ip)) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || (ip.segments()[0] & 0xffc0) == 0xfe80
                || (ip.segments()[0] & 0xfe00) == 0xfc00
        }
        Err(_) => false,
    }
}

/// One client for `server`, with the shared cookie jar and offline cache.
pub fn build_api(server: &str, cache: &Cache, session: Arc<SessionJar>) -> Result<Api, ApiError> {
    let store: Arc<dyn nz_api::ResponseCache> = Arc::new(cache.clone());
    Api::builder(server).cache(store).session(session).build()
}

/// Forward every change of the link's verdict to the shell.
pub fn spawn_online_watch(api: &Api, runtime: &tokio::runtime::Handle, events: Sender<AppEvent>) {
    let mut online = api.online();
    events.send(AppEvent::Online(*online.borrow_and_update()));
    runtime.spawn(async move {
        loop {
            if online.changed().await.is_err() {
                break;
            }
            let verdict = *online.borrow_and_update();
            events.send(AppEvent::Online(verdict));
        }
    });
}

/// Probe the server once: `/api/version` is cheap, answers before a session is
/// restored, and is exactly the request the person's « Réessayer » makes.
pub fn spawn_probe(api: Arc<Api>, runtime: &tokio::runtime::Handle, events: Sender<AppEvent>) {
    runtime.spawn(async move {
        let reachable = match api.version().await {
            Ok(_) => true,
            Err(ApiError::Unauthorized) => true,
            Err(_) => false,
        };
        api.set_online(reachable);
        events.send(AppEvent::Probed(reachable));
    });
}

/// `GET /api/me` for a stored session: what reopens the app on the right
/// screen, and what turns a stored cookie into an account.
pub fn spawn_restore(api: Arc<Api>, runtime: &tokio::runtime::Handle, events: Sender<AppEvent>) {
    runtime.spawn(async move {
        let result = api.me().await;
        if let Ok(me) = &result {
            api.set_owner(Some(me.user.id.as_str().to_owned()));
        }
        events.send(AppEvent::Restored(result.map(Box::new)));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_typed_without_a_scheme_is_http_and_has_no_trailing_slash() {
        assert_eq!(
            normalise("192.168.1.20:5000").unwrap(),
            "http://192.168.1.20:5000"
        );
        assert_eq!(
            normalise(" https://neezer.example/ ").unwrap(),
            "https://neezer.example"
        );
        assert!(normalise("").is_err());
        assert!(normalise("ftp://example.org").is_err());
        assert!(normalise("http://").is_err());
    }

    #[test]
    fn only_a_public_plain_http_address_warns() {
        assert_eq!(plain_http_warning("https://neezer.example"), None);
        assert_eq!(plain_http_warning("http://192.168.1.20:5000"), None);
        assert_eq!(plain_http_warning("http://127.0.0.1:5000"), None);
        assert_eq!(plain_http_warning("http://localhost:5000"), None);
        assert_eq!(plain_http_warning("http://[::1]:5000"), None);
        assert!(plain_http_warning("http://neezer.example").is_some());
        assert!(plain_http_warning("http://203.0.113.7").is_some());
    }

    #[test]
    fn the_private_ranges_are_the_ones_a_lan_install_uses() {
        for host in [
            "10.0.0.7",
            "172.16.4.9",
            "172.31.255.1",
            "192.168.0.2",
            "169.254.1.1",
            "fd00::1",
            "fe80::1",
            "nas.local",
        ] {
            assert!(is_private_host(host), "{host} should be private");
        }
        for host in ["8.8.8.8", "203.0.113.7", "neezer.example", "172.32.0.1"] {
            assert!(!is_private_host(host), "{host} should be public");
        }
    }
}
