//! Enrolment, renewal and the trust sync with the server
//! (docs/agents/lan-nodes.md, *Trust chain*, *Enrolment*).
//!
//! **LN0 declares the four functions and `Enrolled`; LN7 fills them.**
//!
//! The whole point is that the person never sees it: at login, and every 24 h
//! while the server is reachable, the device creates its key if absent, then
//! `POST /api/lan/enroll` with a possession proof, and receives its credential, its
//! roster and the revocation list. A credential with fewer than 23 of its 30 days
//! left is renewed, and the renewal ROTATES the key. Everything runs `Bulk`, so it
//! never sits in front of a screen (rule 2).
//!
//! A device that already holds a valid credential starts offline: [`load_cached`]
//! answers it without any request, which is what makes the LAN work on a trip where
//! the server is out of reach.

use std::path::Path;
use std::sync::Arc;

use crate::store::Store;
use crate::transport::{Identity, SharedTrust, TrustState};
use crate::types::LanError;

/// What a device holds once it is enrolled: the identity it authenticates with,
/// the trust it verifies peers against, and when the credential ends.
#[derive(Debug)]
pub struct Enrolled {
    pub identity: Arc<Identity>,
    pub trust: TrustState,
    /// `not_after` of the credential, unix seconds: Réglages shows it
    /// (« Clé valable jusqu'au 18 décembre ») and the renewal is scheduled from it.
    pub until: i64,
}

/// Makes sure this device can be a node: a key on disk, a credential valid for
/// more than 23 days, a fresh roster and the newest revocation list.
///
/// `now` is passed in rather than read here, so a test drives the clock instead of
/// sleeping (the rule: events, never the wall clock).
pub async fn ensure_enrolled(
    api: &nz_api::Api,
    dir: &Path,
    name: &str,
    platform: &str,
    now: i64,
) -> Result<Enrolled, LanError> {
    let _ = (api, dir, name, platform, now);
    Err(LanError::Unsupported)
}

/// Re-reads the roster and the revocation list, conditionally (`ETag`, `304` =
/// nothing changed and 0 body bytes), and answers whether anything moved.
pub async fn sync_trust(
    api: &nz_api::Api,
    store: &Store,
    trust: &SharedTrust,
) -> Result<bool, LanError> {
    let _ = (api, store, trust);
    Err(LanError::Unsupported)
}

/// Forgets everything at logout: the key, the credential, the roster, and the
/// device row on the server when it is reachable (`DELETE /api/lan/devices/self`).
///
/// `api` is `None` when the person logs out offline: the local wipe still happens,
/// because a session that ends must not leave a usable credential behind.
pub async fn forget(api: Option<&nz_api::Api>, dir: &Path, store: &Store) {
    let _ = (api, dir, store);
}

/// The enrolled state a previous run left on disk, if it is still valid at `now`.
///
/// This is the OFFLINE start: no request, no screen, and the LAN path works on a
/// trip where the server is unreachable.
pub fn load_cached(dir: &Path, store: &Store, now: i64) -> Option<Enrolled> {
    let _ = (dir, store, now);
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub answers `None`: an unimplemented device behaves like a device that
    /// never enrolled, which is the case the app already handles (nothing starts).
    #[test]
    fn an_unimplemented_cache_holds_nothing() {
        assert!(load_cached(Path::new("/nonexistent"), &Store, 0).is_none());
    }
}
