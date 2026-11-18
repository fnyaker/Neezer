//! Enrolment, renewal and the trust sync with the server
//! (docs/agents/lan-nodes.md, *Trust chain*, *Enrolment*).
//!
//! **LN7 filled the four functions and `Enrolled`.**
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
//!
//! Four properties, each measured in `tests/enroll.rs`:
//!
//! - **Nothing is written before the server said yes.** The status answer comes
//!   first (it carries the subject the proof is built over and the keys the
//!   credential is verified against); a server that answers `404` leaves the data
//!   directory exactly as it was — no key, no database, no directory.
//! - **Nothing unverified is stored.** The credential, the roster and the
//!   revocation list are decoded and verified *before* the first `put_trust`, so a
//!   broken or hostile server cannot leave the device holding a blob it would
//!   refuse to use.
//! - **A cached credential costs zero requests.** A start with more than
//!   [`RENEW_BELOW_DAYS`] of validity left reads the four trust rows and the key
//!   file and answers, with no HTTP at all.
//! - **The ETag is derived, never guessed.** A fresh enrolment already carries both
//!   numbers the server's `ETag` is made of (the roster serial and the revocation
//!   serial), so the FIRST daily sync is conditional — one request, `304`, 0 bytes
//!   — instead of paying one full roster to learn a validator it could compute.
//!
//! The store's shapes, all of them `store.rs`'s `trust` table (`key` → blob):
//!
//! | Key | Blob |
//! |---|---|
//! | `credential` | the 154 raw bytes |
//! | `roster` | the `{"body","sig"}` envelope, JSON (what gossip relays) |
//! | `roster.etag` | the server's validator, UTF-8 |
//! | `revocations:<kid hex>` | the signed list, raw (what gossip relays) |
//! | `ca:<kid hex>` | the server's 32-byte Ed25519 public key |

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::cred::{
    CredError, Credential, Revocations, Roster, TrustKeys, base64_decode, base64_encode, hex_encode,
};
use crate::gossip::{ROSTER_KEY, revocations_key};
use crate::keys::{DeviceKey, KEY_FILE};
use crate::store::{DB_FILE, Store};
use crate::transport::{Identity, SharedTrust, TrustState};
use crate::types::{Kid, LanError, Subject};

/// A credential with less than this much validity left is renewed: 23 of the
/// default 30 days, so a device that reaches the server even once a week always
/// renews before it expires.
pub const RENEW_BELOW_DAYS: i64 = 23;
/// The same threshold in seconds.
pub const RENEW_BELOW_SECONDS: i64 = RENEW_BELOW_DAYS * 86_400;

/// The `trust` row the credential is kept under.
const CREDENTIAL_KEY: &str = "credential";
/// The `trust` row the server's `ETag` for the roster is kept under.
///
/// One key beside the four the note lists: the route answers a validator and a
/// conditional sync has to hand it back, and there is nowhere else on a device
/// to put it. It is a blob key, not a schema change, and [`forget`] wipes it
/// with every other trust row.
const ROSTER_ETAG_KEY: &str = "roster.etag";
/// The `ca:<kid>` prefix: one row per server LAN key this device trusts.
const CA_PREFIX: &str = "ca:";
/// The `revocations:<kid>` prefix (spelled by [`revocations_key`]).
const REVOCATIONS_PREFIX: &str = "revocations:";
/// Where a renewal writes its next key until the server has answered (`dir`'s
/// subdirectory, so the rename that promotes it stays inside one filesystem).
pub const STAGED_DIR: &str = "staged-key";

/// A renewal's next key, written in its own directory and **removed as soon as it
/// goes out of scope unless it was promoted**: a refusal must not leave a second
/// private seed on the device (T11), and the credential it was renewing stays the
/// only one that works.
struct StagedKey {
    dir: std::path::PathBuf,
}

impl StagedKey {
    /// A fresh key under `dir/staged-key`, replacing a seed a crash left there.
    fn create(dir: &Path) -> Result<(DeviceKey, StagedKey), LanError> {
        let staged = StagedKey {
            dir: dir.join(STAGED_DIR),
        };
        let _ = std::fs::remove_dir_all(&staged.dir);
        let key =
            DeviceKey::rotate(&staged.dir).map_err(|error| LanError::Disk(error.to_string()))?;
        Ok((key, staged))
    }

    /// One `rename` inside `dir`: the device's key becomes the one the server just
    /// attested. A crash before it keeps the old key (contacted again with the same
    /// `device_id`); a crash after it leaves the new one — never half a seed.
    fn promote(&self, dir: &Path) -> Result<(), LanError> {
        std::fs::rename(self.dir.join(KEY_FILE), dir.join(KEY_FILE))
            .map_err(|error| LanError::Disk(error.to_string()))
    }
}

impl Drop for StagedKey {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A 154-byte credential is 208 base64 characters; a longer text is refused
/// BEFORE it is decoded (a hostile server must not size an allocation).
const CREDENTIAL_B64_MAX: usize = 208;
/// A revocation list at its 4 096-key cap: 131 169 bytes → 174 892 characters.
const REVOCATIONS_B64_MAX: usize = 174_892;
/// A roster body is capped at 4 MiB by `cred.rs`; its base64 form is this long.
const ROSTER_B64_MAX: usize = (crate::cred::ROSTER_MAX_BYTES / 3 + 1) * 4;

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
///
/// The order is the one that keeps a refusal harmless: a credential this device
/// already holds and that is good for more than [`RENEW_BELOW_DAYS`] answers with
/// **no request at all**; otherwise `GET /api/lan/status` (subject + keys) →
/// the key → `POST /api/lan/enroll`. A server without the route (`404`) or with
/// the feature off answers `Unsupported`/`Disabled` and **writes nothing**: the
/// data directory is not even created.
pub async fn ensure_enrolled(
    api: &nz_api::Api,
    dir: &Path,
    name: &str,
    platform: &str,
    now: i64,
) -> Result<Enrolled, LanError> {
    // The offline half first: no request when the device already holds a good
    // credential. `Store` is opened only if a database exists, so a device that
    // never enrolled does not create one (and a 404 below leaves no trace).
    let existing = open_if_present(dir)?;
    if let Some(store) = existing.as_ref()
        && let Some(enrolled) = load_cached(dir, store, now)
        && enrolled.until.saturating_sub(now) > RENEW_BELOW_SECONDS
    {
        tracing::debug!(
            until = enrolled.until,
            "LAN: credential still good, no request"
        );
        return Ok(enrolled);
    }

    let status = api.lan_status().await.map_err(status_error)?;
    if !status.enabled {
        return Err(LanError::Disabled);
    }
    let subject = parse_subject(&status.subject)?;
    let pairs = parse_keys(&status.keys)?;
    if pairs.is_empty() {
        return Err(LanError::Proto(
            "le serveur n'a annoncé aucune clé LAN".to_owned(),
        ));
    }
    let keys = TrustKeys::from_status(&pairs);

    // Only now may anything be written: the server said yes and named its keys.
    let store = match existing {
        Some(store) => store,
        None => Store::open(dir)?,
    };
    // A stored credential names the device this key REPLACES, so a renewal keeps
    // one device row (and its place under the 20-device cap) instead of enrolling
    // a new one every 30 days.
    //
    // A RENEWAL STAGES ITS KEY. Rotating `device.key` in place before the POST
    // would cost the device the credential it still holds whenever the server
    // refuses or the answer is lost (rotate → 403 → the stored credential no
    // longer matches the key on disk, `load_cached` answers `None`, and the node
    // disappears for days). The new key is written in its own directory and only
    // replaces `device.key` once everything the server sent has verified — the
    // same "nothing is kept before it is proven" rule as the credential itself.
    let previous = stored_credential(&store);
    let (key, staged) = match previous.as_ref() {
        Some(_) => {
            let (key, staged) = StagedKey::create(dir)?;
            (key, Some(staged))
        }
        None => (
            DeviceKey::load_or_create(dir).map_err(|error| LanError::Disk(error.to_string()))?,
            None,
        ),
    };

    let at = now;
    let request = nz_model::lan::LanEnrollRequest {
        pubkey: hex_encode(&key.public()),
        name: name.to_owned(),
        platform: platform.to_owned(),
        at,
        proof: base64_encode(&key.enroll_proof(&status.subject, at)),
        device_id: previous
            .as_ref()
            .map(|credential| hex_encode(&credential.device_id)),
    };
    let answer = api.lan_enroll(&request).await.map_err(enrol_error)?;

    let credential = decode_credential(&answer.credential)?;
    let revocations = decode_revocations(&answer.revocations, &keys)?;
    let roster = decode_roster(&answer.roster, &keys)?;
    if credential.subject != subject {
        // The server signed a credential for another person: nothing about this
        // is usable, and the device stays un-enrolled rather than acting as
        // somebody else.
        tracing::warn!("LAN: le serveur a signé une attestation pour un autre sujet");
        return Err(LanError::Mismatch);
    }
    if credential.device_pub != key.public() {
        tracing::warn!("LAN: l'attestation ne porte pas la clé de cet appareil");
        return Err(LanError::Mismatch);
    }
    if roster.subject != subject {
        tracing::warn!("LAN: le roster reçu ne concerne pas ce sujet");
        return Err(LanError::Mismatch);
    }
    // The revocation list the credential is checked against is the one this
    // answer just carried — the newest the server has (`verify` uses it for the
    // device key and for its own `kid`).
    credential
        .verify(&keys, &revocations, now)
        .map_err(credential_error)?;

    // Everything is verified: from here on the device commits. The staged key
    // becomes `device.key` FIRST (a rename inside one directory), so a credential
    // is never stored for a key that is not on disk.
    if let Some(staged) = staged.as_ref() {
        staged.promote(dir)?;
    }
    store_keys(&store, &pairs)?;
    store.put_trust(CREDENTIAL_KEY, credential.raw())?;
    store.put_trust(ROSTER_KEY, &roster.raw_signed())?;
    store.put_trust(&revocations_key(&revocations.kid), revocations.raw())?;
    // The route's own validator is `"<roster serial>-<revocation serial>"`, and
    // both numbers just arrived: the first daily sync is conditional without a
    // request to learn it.
    store.put_trust(
        ROSTER_ETAG_KEY,
        roster_etag(roster.serial, revocations.serial).as_bytes(),
    )?;

    Ok(Enrolled {
        until: credential.not_after,
        identity: Arc::new(Identity {
            key,
            cred: credential,
        }),
        trust: TrustState {
            subject,
            keys,
            revocations: HashMap::from([(revocations.kid, revocations)]),
            roster: Some(roster),
        },
    })
}

/// Re-reads the roster and the revocation list, conditionally (`ETag`, `304` =
/// nothing changed and 0 body bytes), and answers whether anything moved.
///
/// One request on the `Bulk` lane, whatever the answer: a `304` costs a status
/// line and 0 bytes, which is why this can run once a day for years.
pub async fn sync_trust(
    api: &nz_api::Api,
    store: &Store,
    trust: &SharedTrust,
) -> Result<bool, LanError> {
    let etag = store
        .trust(ROSTER_ETAG_KEY)
        .and_then(|blob| String::from_utf8(blob).ok());
    let answer = api
        .lan_roster(etag.as_deref())
        .await
        .map_err(status_error)?;
    let Some(answer) = answer else {
        // `304`: the validator still matches, so the device already holds this
        // exact roster and revocation list.
        return Ok(false);
    };
    let (keys, subject) = {
        let state = read(trust);
        let keys = if state.keys.is_empty() {
            stored_keys(store)
        } else {
            state.keys.clone()
        };
        (keys, state.subject)
    };
    let roster = decode_roster(&answer.roster, &keys)?;
    let revocations = decode_revocations(&answer.revocations, &keys)?;
    if roster.subject != subject {
        // Signed, verified, and not about me: only my own roster may become the
        // one my devices trust.
        tracing::warn!("LAN: le roster synchronisé ne concerne pas ce sujet");
        return Err(LanError::Mismatch);
    }

    // The server is authoritative, but an answer is never allowed to move the
    // device BACKWARDS: a peer's gossip can only relay rosters the server signed,
    // so a serial below what this device holds means the answer is not newer than
    // what it already trusts. One write lock makes the decision and the store
    // agree (a single upsert each, and the bytes are at most a few hundred KB).
    let mut state = write(trust);
    let roster_moved = state
        .roster
        .as_ref()
        .is_none_or(|held| roster.serial > held.serial);
    if roster_moved {
        store.put_trust(ROSTER_KEY, &roster.raw_signed())?;
    }
    let revocations_moved = state
        .revocations
        .get(&revocations.kid)
        .is_none_or(|held| revocations.serial > held.serial);
    if revocations_moved {
        store.put_trust(&revocations_key(&revocations.kid), revocations.raw())?;
    }
    if !answer.etag.is_empty() {
        store.put_trust(ROSTER_ETAG_KEY, answer.etag.as_bytes())?;
    }
    if roster_moved {
        state.roster = Some(roster);
    }
    if revocations_moved {
        state.revocations.insert(revocations.kid, revocations);
    }
    Ok(roster_moved || revocations_moved)
}

/// Forgets everything at logout: the key, the credential, the roster, and the
/// device row on the server when it is reachable (`DELETE /api/lan/devices/self`).
///
/// `api` is `None` when the person logs out offline: the local wipe still happens,
/// because a session that ends must not leave a usable credential behind.
pub async fn forget(api: Option<&nz_api::Api>, dir: &Path, store: &Store) {
    // The trust rows go FIRST: `load_cached` refuses to read a key for a device
    // that holds no credential, so a reader racing this function can never see a
    // key without one.
    if let Err(error) = store.forget_trust() {
        tracing::warn!(%error, "LAN: les données de confiance n'ont pas été effacées");
    }
    if let Err(error) = DeviceKey::wipe(dir) {
        tracing::warn!(%error, "LAN: la clé d'appareil n'a pas été effacée");
    }
    // A renewal that crashed between its two renames leaves a staged seed, which
    // is as good as the key itself: the wipe removes it too.
    let _ = std::fs::remove_dir_all(dir.join(STAGED_DIR));
    let Some(api) = api else {
        return;
    };
    if !api.is_online() {
        // Offline: the local wipe is what matters. The device stays enrolled on
        // the server and is revoked from Réglages on another device, which is
        // exactly what the revocation list is for.
        return;
    }
    if let Err(error) = api.lan_revoke("self").await {
        // A failure here is not the person's logout: the local key is gone, so
        // this device can no longer authenticate on the LAN whatever the server
        // still lists.
        tracing::debug!(error = %error, "LAN: la révocation en ligne a échoué");
    }
}

/// The enrolled state a previous run left on disk, if it is still valid at `now`.
///
/// This is the OFFLINE start: no request, no screen, and the LAN path works on a
/// trip where the server is unreachable. It never writes: no key is created, no
/// row is added, and a device that never enrolled answers `None`.
pub fn load_cached(dir: &Path, store: &Store, now: i64) -> Option<Enrolled> {
    // `credential` first: a device without one is not enrolled, and must not even
    // have its key file read.
    let credential = stored_credential(store)?;
    if !dir.join(KEY_FILE).is_file() {
        return None;
    }
    let key = match DeviceKey::load_or_create(dir) {
        Ok(key) => key,
        Err(error) => {
            tracing::debug!(%error, "LAN: clé d'appareil illisible");
            return None;
        }
    };
    if key.public() != credential.device_pub {
        // A renewal rotated the key but never got its credential: fail closed and
        // let the next contact re-enrol on the same device row.
        tracing::debug!("LAN: la clé d'appareil ne correspond plus à l'attestation");
        return None;
    }
    let keys = stored_keys(store);
    let revocations = stored_revocations(store, &keys);
    let revoked = revocations
        .get(&credential.kid)
        .cloned()
        .unwrap_or_else(|| Revocations::empty(credential.kid));
    if let Err(error) = credential.verify(&keys, &revoked, now) {
        tracing::debug!(%error, "LAN: attestation inutilisable hors ligne");
        return None;
    }
    let roster = stored_roster(store, &keys).filter(|roster| roster.subject == credential.subject);
    Some(Enrolled {
        until: credential.not_after,
        identity: Arc::new(Identity {
            key,
            cred: credential.clone(),
        }),
        trust: TrustState {
            subject: credential.subject,
            keys,
            revocations,
            roster,
        },
    })
}

// -- the server's answers ------------------------------------------------------

/// `status`'s failure, said in this crate's words: a server without the route is
/// `Unsupported` (rule 7 — no screen, no setting, nothing changes), a server with
/// the feature off is `Disabled`, and anything else keeps the server's cause.
fn status_error(error: nz_api::ApiError) -> LanError {
    match error {
        nz_api::ApiError::Http { status: 404, .. } => LanError::Unsupported,
        other => LanError::Server(other.cause_fr()),
    }
}

/// `enroll`'s failure. Every cause the server names is kept, and the one that
/// needs a person's action gets its own sentence: a proof refused for its date
/// means the DEVICE's clock is wrong, which no retry can fix.
fn enrol_error(error: nz_api::ApiError) -> LanError {
    if error.code() == Some("stale_proof") {
        return LanError::Server(
            "l'heure de l'appareil est décalée : corrigez-la, puis réessayez".to_owned(),
        );
    }
    status_error(error)
}

/// A `CredError` as the wire-visible refusal. The precise cause goes to the log,
/// never to a peer (the note's T2/T10: one variant for every cause).
fn credential_error(error: CredError) -> LanError {
    match error {
        CredError::Expired => LanError::Expired,
        CredError::Revoked => LanError::Refused,
        other => {
            tracing::debug!(%other, "LAN: attestation refusée");
            LanError::Refused
        }
    }
}

fn decode_credential(text: &str) -> Result<Credential, LanError> {
    if text.len() > CREDENTIAL_B64_MAX {
        return Err(LanError::Proto("attestation trop longue".to_owned()));
    }
    let raw = base64_decode(text)
        .ok_or_else(|| LanError::Proto("attestation : base64 invalide".to_owned()))?;
    Credential::decode(&raw).map_err(|error| {
        tracing::debug!(%error, "LAN: attestation illisible");
        LanError::Refused
    })
}

fn decode_revocations(text: &str, keys: &TrustKeys) -> Result<Revocations, LanError> {
    if text.len() > REVOCATIONS_B64_MAX {
        return Err(LanError::Proto(
            "liste de révocation trop longue".to_owned(),
        ));
    }
    let raw = base64_decode(text)
        .ok_or_else(|| LanError::Proto("liste de révocation : base64 invalide".to_owned()))?;
    Revocations::decode_verify(&raw, keys).map_err(|error| {
        tracing::debug!(%error, "LAN: liste de révocation refusée");
        LanError::Refused
    })
}

fn decode_roster(signed: &nz_model::lan::LanSigned, keys: &TrustKeys) -> Result<Roster, LanError> {
    if signed.body.len() > ROSTER_B64_MAX || signed.sig.len() > 128 {
        return Err(LanError::Proto("roster trop long".to_owned()));
    }
    // The envelope is the `{"body","sig"}` shape `cred.rs` verifies: only the
    // DECODED body is signed, so re-wrapping the server's own base64 strings is
    // exact — and the signature is checked over the bytes they decode to.
    let envelope = serde_json::json!({ "body": signed.body, "sig": signed.sig });
    let bytes = serde_json::to_vec(&envelope)
        .map_err(|error| LanError::Proto(format!("roster : {error}")))?;
    Roster::decode_verify(&bytes, keys).map_err(|error| {
        tracing::debug!(%error, "LAN: roster refusé");
        LanError::Refused
    })
}

// -- what the store holds ------------------------------------------------------

/// Opens `lan.sqlite` **only if it is there**: an un-enrolled device must not
/// create a database just to discover that the server has no such feature.
fn open_if_present(dir: &Path) -> Result<Option<Store>, LanError> {
    if !dir.join(DB_FILE).is_file() {
        return Ok(None);
    }
    Store::open(dir).map(Some)
}

/// The stored credential, decoded but not verified — the renewal needs the
/// `device_id` it carries, and a credential whose window is closing is still the
/// one that names this device.
fn stored_credential(store: &Store) -> Option<Credential> {
    let raw = store.trust(CREDENTIAL_KEY)?;
    match Credential::decode(&raw) {
        Ok(credential) => Some(credential),
        Err(error) => {
            tracing::debug!(%error, "LAN: attestation stockée illisible");
            None
        }
    }
}

fn stored_keys(store: &Store) -> TrustKeys {
    let pairs: Vec<(Kid, [u8; 32])> = store
        .trust_keys()
        .iter()
        .filter_map(|key| key.strip_prefix(CA_PREFIX))
        .filter_map(|hex| {
            let kid = hex_array::<8>(hex)?;
            let blob = store.trust(&format!("{CA_PREFIX}{hex}"))?;
            let public: [u8; 32] = blob.as_slice().try_into().ok()?;
            Some((Kid(kid), public))
        })
        .collect();
    TrustKeys::from_status(&pairs)
}

fn stored_revocations(store: &Store, keys: &TrustKeys) -> HashMap<Kid, Revocations> {
    let mut out = HashMap::new();
    for key in store.trust_keys() {
        let Some(hex) = key.strip_prefix(REVOCATIONS_PREFIX) else {
            continue;
        };
        let Some(blob) = store.trust(&key) else {
            continue;
        };
        match Revocations::decode_verify(&blob, keys) {
            Ok(list) => {
                // One row per `kid`; if a build ever wrote two, the newest wins.
                if out
                    .get(&list.kid)
                    .is_none_or(|held: &Revocations| list.serial > held.serial)
                {
                    out.insert(list.kid, list);
                }
            }
            Err(error) => tracing::debug!(%error, hex, "LAN: liste de révocation stockée refusée"),
        }
    }
    out
}

fn stored_roster(store: &Store, keys: &TrustKeys) -> Option<Roster> {
    let blob = store.trust(ROSTER_KEY)?;
    match Roster::decode_verify(&blob, keys) {
        Ok(roster) => Some(roster),
        Err(error) => {
            tracing::debug!(%error, "LAN: roster stocké refusé");
            None
        }
    }
}

fn store_keys(store: &Store, pairs: &[(Kid, [u8; 32])]) -> Result<(), LanError> {
    for (kid, public) in pairs {
        store.put_trust(&format!("{CA_PREFIX}{}", hex_encode(&kid.0)), public)?;
    }
    Ok(())
}

// -- the spellings -------------------------------------------------------------

/// `"<roster serial>-<revocation serial>"`, quoted — the server's own
/// `roster_etag` (`neezer/lan.py`), reproduced from the two numbers an enrolment
/// carries. A `304` needs this string to be byte-identical, which is why it is
/// written once here and asserted against the Python formula in the tests.
pub(crate) fn roster_etag(roster_serial: u64, revocation_serial: u64) -> String {
    format!("\"{roster_serial}-{revocation_serial}\"")
}

/// The caller's subject: 32 hex characters, nothing else.
fn parse_subject(hex: &str) -> Result<Subject, LanError> {
    hex_array::<16>(hex)
        .map(Subject)
        .ok_or_else(|| LanError::Proto("sujet LAN invalide".to_owned()))
}

/// `status.keys` as `(Kid, public)`: a malformed entry is dropped, because a
/// single unreadable key must not keep a device from enrolling with the others.
fn parse_keys(keys: &[nz_model::lan::LanKey]) -> Result<Vec<(Kid, [u8; 32])>, LanError> {
    Ok(keys
        .iter()
        .filter_map(|key| {
            let kid = hex_array::<8>(&key.kid)?;
            let public: [u8; 32] = hex_array::<32>(&key.pub_key)?;
            Some((Kid(kid), public))
        })
        .collect())
}

/// `N` bytes of lowercase hex, and nothing else.
fn hex_array<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 || !text.is_ascii() {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = [0u8; N];
    for (index, byte) in out.iter_mut().enumerate() {
        let high = (bytes[index * 2] as char).to_digit(16)?;
        let low = (bytes[index * 2 + 1] as char).to_digit(16)?;
        *byte = ((high << 4) | low) as u8;
    }
    Some(out)
}

fn read(trust: &SharedTrust) -> std::sync::RwLockReadGuard<'_, TrustState> {
    trust
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write(trust: &SharedTrust) -> std::sync::RwLockWriteGuard<'_, TrustState> {
    trust
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The validator is the server's `roster_etag` reproduced: quoted, two
    /// numbers, a hyphen. The Python side builds
    /// `f'"{serial}-"{revocation_serial}"'`, so a change here is a `304` that
    /// never happens (a silent full-roster download every day).
    #[test]
    fn the_etag_is_the_servers_own_formula() {
        assert_eq!(roster_etag(42, 7), "\"42-7\"");
        assert_eq!(roster_etag(0, 0), "\"0-0\"");
        assert_eq!(roster_etag(u64::MAX, 1), "\"18446744073709551615-1\"");
    }

    /// Hex is exact in LENGTH and strict in digits, and both cases are accepted
    /// the way `cred.rs`'s own decoder accepts them (one decoder, one meaning):
    /// the row keys this file writes are always re-spelled lowercase, so an
    /// uppercase answer cannot create a second `ca:` row.
    #[test]
    fn hex_is_exact_length_and_strict_in_digits() {
        assert_eq!(hex_array::<2>("00ff").expect("lowercase hex"), [0, 255]);
        assert_eq!(hex_array::<2>("00FF").expect("uppercase hex"), [0, 255]);
        assert!(hex_array::<2>("00f").is_none(), "short");
        assert!(hex_array::<2>("00ff0").is_none(), "long");
        assert!(hex_array::<2>("00zz").is_none(), "not hex");
        assert!(
            hex_array::<16>("00112233445566778899aabbccddeeff").is_some(),
            "the note's subject"
        );
    }

    /// A device that never enrolled answers `None` and writes nothing: this is
    /// the offline start, and it must not create a key file or a database.
    #[test]
    fn a_device_that_never_enrolled_loads_nothing() {
        let dir = temp_dir("never");
        let store = Store::open(&dir).expect("store");
        assert!(load_cached(&dir, &store, 1_800_000_000).is_none());
        assert!(!dir.join(KEY_FILE).exists(), "no key is created by a read");
        assert!(
            open_if_present(&dir)
                .expect("a database that exists")
                .is_some()
        );
        drop(store);
        std::fs::remove_dir_all(&dir).ok();

        // A directory with no database at all: nothing is opened, created, or
        // read.
        let absent = dir.join("absent");
        assert!(
            open_if_present(&absent)
                .expect("no database, no error")
                .is_none()
        );
        assert!(!absent.exists(), "the directory is not even created");
    }

    /// A test directory under the workspace's `target/`, never `/tmp`.
    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nz-lan-tests")
            .join(format!("enroll-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test dir");
        dir
    }

    /// The note's credential vector (154 bytes), as hex — decoded, not verified,
    /// which is all this file's guard test needs.
    const CREDENTIAL_HEX: &str = concat!(
        "4e5a4c430156475aa75463474c00112233445566778899aabbccddeeff010101010101010101010101",
        "29acbae141bccaf0b22e1a94d34d0bc7361e526d0bfe12c89794bc9322966dd7000000006b36ec80",
        "000000006b5e798003a839e52b155515b9f88b70ab9844585cb0ed07a9a685618cce2243fedfce50",
        "968c222fb9c69068b1ded70f7eff5f1d66acc9419e4f4b524add9fa8bf7efcde09",
    );

    /// **A read never writes.** With a credential in the store but no key file, the
    /// offline start answers `None` and leaves the directory as it was: calling
    /// `load_or_create` there would mint a key on a screen's start path, and a
    /// device whose key vanished must re-enrol instead.
    ///
    /// Mutant that must fail here: remove the `KEY_FILE.is_file()` guard.
    #[test]
    fn a_read_never_creates_a_key() {
        let dir = temp_dir("no-key");
        let store = Store::open(&dir).expect("store");
        let credential: Vec<u8> = (0..CREDENTIAL_HEX.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&CREDENTIAL_HEX[at..at + 2], 16).expect("hex"))
            .collect();
        assert_eq!(credential.len(), 154);
        store
            .put_trust("credential", &credential)
            .expect("a stored credential");
        assert!(load_cached(&dir, &store, 1_798_761_600).is_none());
        assert!(
            !dir.join(KEY_FILE).exists(),
            "aucune clé n'est créée par une lecture"
        );
        drop(store);
        std::fs::remove_dir_all(&dir).ok();
    }
}
