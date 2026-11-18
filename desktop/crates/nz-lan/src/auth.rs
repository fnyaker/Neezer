//! The exporter-bound proof exchange, and the authorisation decision that
//! follows it (docs/agents/lan-nodes.md, *Transport and authentication*).
//!
//! ```text
//! exporter_r = TLS exporter("EXPORTER-neezer-lan/1", context = role, 32 bytes)
//! proof_r    = Ed25519(device key, b"neezer-lan/1 auth\0" ‖ role ‖ exporter_r)
//!
//! I → R  Auth   { cred, proof_initiator, revocation_serial, proto }
//! R      verify cred (R's known kids + revocations), proof over R's own
//!        exporter("initiator"), authorise: == R's subject → Own ;
//!        in R's roster.shares_to → Friend(scope) ; else close
//! R → I  AuthOk { cred, proof_responder, revocation_serial, proto }
//! I      verify the same; authorise from ITS roster.trusted_by ; else close
//! both   Hello { name, platform, app_version, inventory_version, sets_version }
//! ```
//!
//! Two properties make this safe on a hostile Wi-Fi, and both are structural:
//! the proof signs THIS connection's exporter (a replay is useless on another
//! connection, and a MITM that terminates two TLS sessions holds two different
//! exporters, so a relayed proof fails), and the INITIATOR authenticates first,
//! so a stranger who connects learns nothing — not even the responder's
//! credential (T2).
//!
//! # The two contexts, and why a reflection cannot work
//!
//! Each side signs the exporter whose **context is its own role** and verifies the
//! peer's proof against the same (label, context) pair. Both sides derive the same
//! bytes for one pair (RFC 8446 §7.5), and the two pairs differ, so a responder's
//! `AuthOk` proof — signed over `"responder" ‖ exporter` — does not verify as an
//! initiator's `Auth` proof over `"initiator" ‖ exporter`, even on the *same*
//! connection. The role is in the signed message ([`crate::cred::auth_message`]),
//! the context is in the exporter, and a test covers both halves.
//!
//! # One answer for every refusal
//!
//! Every failure — an unknown server key, an expired or revoked credential, a
//! proof over another connection, a subject that is neither mine nor a friend —
//! leaves through [`CLOSE_REFUSED`] with an **empty reason**, writes **no byte**
//! on the stream, and closes the same way. The precise cause lives in
//! [`AuthError`] for this device's own log and its tests, never on the wire.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_lc_rs::signature::{ED25519, UnparsedPublicKey};

use crate::cred::{
    CredError, Credential, Revocations, Role, auth_message, base64_decode, base64_encode,
};
use crate::proto;
use crate::scope::Scope;
use crate::transport::{Identity, TrustState};
use crate::types::{DevicePub, LanError, Relation, Subject};

/// The application close code of every refused authentication.
///
/// ONE code for every cause (unknown key, expired credential, revoked device, a
/// proof over another connection, an unauthorised subject): the peer must not be
/// able to tell them apart, and neither must anybody watching the connection.
pub const CLOSE_REFUSED: u32 = 0x10;

/// The application close code of a protocol MAJOR this build cannot speak.
///
/// Separate from [`CLOSE_REFUSED`] on purpose: this one is not a security event,
/// it is a version mismatch, and the screens say « mettez à jour l'app sur
/// <appareil> » instead of "refused".
pub const CLOSE_WRONG_MAJOR: u32 = 0x11;

/// The protocol major this build speaks. A peer that sends another one is closed
/// with [`CLOSE_WRONG_MAJOR`].
pub const PROTO_MAJOR: u32 = 1;

/// How much a responder reads before it has an authenticated peer, and how long
/// it waits: one stream, 2 KiB, 2 s.
///
/// Beyond either, the connection is closed with [`CLOSE_REFUSED`] and nothing was
/// written. Cheap for the responder, and it means an unauthenticated peer cannot
/// make the node allocate or wait.
pub const PREAUTH_MAX_BYTES: usize = 2 * 1024;

/// How long a responder waits for the initiator's `Auth` frame.
pub const PREAUTH_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

/// The TLS exporter's label, exactly as the note writes it. Both sides pass it
/// verbatim: a label that differs by one character yields 32 unrelated bytes and
/// every proof fails, which is why it is one constant and not two strings.
pub const EXPORTER_LABEL: &[u8] = b"EXPORTER-neezer-lan/1";

/// How many bytes the exporter produces (the note's 32).
pub const EXPORTER_BYTES: usize = 32;

/// How long an authenticated peer has to send its `Hello`: one frame, on a
/// connection whose TLS handshake and both proofs have already completed, so a
/// peer that says nothing this long is gone rather than slow.
pub const HELLO_DEADLINE: Duration = Duration::from_secs(10);

/// My side of one connection.
///
/// It decides two things at once, and that is why it is one parameter: which frame
/// I send ([`proto::Control::Auth`] or [`proto::Control::AuthOk`]), and WHICH list
/// of my roster authorises the peer — the responder serves whom it shares to
/// (`shares_to`), the initiator may ask whom it trusts (`trusted_by`). A friend's
/// connection is therefore one-directional in content, and two friends who share
/// with each other each authorise the other on their own side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The peer dialled me: I answer `AuthOk`, and the peer may fetch what I share.
    Responder,
    /// I dialled the peer: I send `Auth`, and I may fetch from whom I trust.
    Initiator,
}

impl Side {
    /// The role my own proof is signed with.
    pub fn role(self) -> Role {
        match self {
            Side::Responder => Role::Responder,
            Side::Initiator => Role::Initiator,
        }
    }

    /// The role the peer's proof is signed with: the other one.
    pub fn peer_role(self) -> Role {
        match self {
            Side::Responder => Role::Initiator,
            Side::Initiator => Role::Responder,
        }
    }
}

/// What one authenticated peer is: its credential, how it relates to me, and what
/// it may fetch from me.
#[derive(Debug, Clone)]
pub struct Authorised {
    pub cred: Credential,
    pub relation: Relation,
    /// What the peer may fetch from THIS device, built from MY roster. A peer that
    /// may fetch nothing gets the empty friend scope, whose `allows` answers
    /// `false` for every track — so every request it makes is answered exactly
    /// like an absent file (T7), never with a refusal that tells it more.
    pub scope: Scope,
}

/// Derives this connection's exporter for one role.
///
/// Both peers get the same 32 bytes for the same role — that is what RFC 8446
/// §7.5 promises — and a different 32 bytes for the other role, which is what
/// makes a reflected proof fail.
pub fn exporter(conn: &quinn::Connection, role: Role) -> Result<[u8; EXPORTER_BYTES], LanError> {
    let mut exporter = [0u8; EXPORTER_BYTES];
    conn.export_keying_material(&mut exporter, EXPORTER_LABEL, role.as_bytes())
        .map_err(|_| {
            // The only way this fails is a session without keying material — a
            // local bug, never a peer's doing.
            LanError::Internal("l'exporteur TLS de la connexion est indisponible".to_owned())
        })?;
    Ok(exporter)
}

/// The proof this device sends: `Ed25519(device key, auth_message(my role, my
/// exporter))`.
pub fn proof(
    key: &crate::keys::DeviceKey,
    conn: &quinn::Connection,
    side: Side,
) -> Result<[u8; 64], LanError> {
    let exporter = exporter(conn, side.role())?;
    Ok(key.sign(&auth_message(side.role(), &exporter)))
}

/// Verifies a peer's proof against the exporter of THIS connection.
///
/// The message is rebuilt from the connection, never taken from the wire: a proof
/// that was recorded on another connection names 32 bytes this connection does not
/// have, and one replayed from the other direction names the other role.
pub fn verify_proof(
    public: &DevicePub,
    conn: &quinn::Connection,
    role: Role,
    signature: &[u8],
) -> Result<(), LanError> {
    let exporter = exporter(conn, role)?;
    let message = auth_message(role, &exporter);
    UnparsedPublicKey::new(&ED25519, public)
        .verify(&message, signature)
        .map_err(|_| LanError::Refused)
}

/// The frame this device sends for its side: `Auth` when it dialled, `AuthOk` when
/// it was dialled. Both carry the same four fields, so a peer reads one shape.
pub fn proof_frame(
    id: &Identity,
    conn: &quinn::Connection,
    side: Side,
    revocation_serial: u64,
) -> Result<proto::Control, LanError> {
    let proof = proof(&id.key, conn, side)?;
    let fields = (
        base64_encode(id.cred.raw()),
        base64_encode(&proof),
        revocation_serial,
        PROTO_MAJOR,
    );
    Ok(match side {
        Side::Initiator => proto::Control::Auth {
            cred: fields.0,
            proof: fields.1,
            revocation_serial: fields.2,
            proto: fields.3,
        },
        Side::Responder => proto::Control::AuthOk {
            cred: fields.0,
            proof: fields.1,
            revocation_serial: fields.2,
            proto: fields.3,
        },
    })
}

/// Why a peer was not let in. Every variant is for this device's log and its tests:
/// the peer is closed with one code and no reason (see [`AuthError::close_code`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// The peer speaks another protocol major.
    WrongMajor { peer: u32 },
    /// The frame is not the one this side expects, or a field is not the base64
    /// the layout requires.
    Malformed(String),
    /// The credential does not decode, does not verify, is expired or is revoked.
    Credential(CredError),
    /// The proof does not verify over this connection and this role.
    Proof,
    /// The credential is valid, but the subject is neither mine nor authorised by
    /// my roster.
    Unauthorised,
}

impl AuthError {
    /// The application code the connection is closed with: the version mismatch
    /// keeps its own code so the screens can say « mettez à jour », and EVERY other
    /// cause shares [`CLOSE_REFUSED`].
    pub fn close_code(&self) -> u32 {
        match self {
            AuthError::WrongMajor { .. } => CLOSE_WRONG_MAJOR,
            _ => CLOSE_REFUSED,
        }
    }

    /// The refusal as this device answers it: coarse for the peer, precise for the
    /// log and the tests.
    pub fn as_lan_error(&self) -> LanError {
        match self {
            AuthError::WrongMajor { peer } => {
                LanError::Proto(format!("protocole {peer} non pris en charge"))
            }
            _ => LanError::Refused,
        }
    }

    /// The cause in the words this device's log and its tests use.
    pub fn cause_fr(&self) -> String {
        match self {
            AuthError::WrongMajor { peer } => {
                format!("protocole {peer} : cette version ne le parle pas")
            }
            AuthError::Malformed(why) => format!("trame d'authentification inattendue : {why}"),
            AuthError::Credential(error) => format!("attestation refusée : {error}"),
            AuthError::Proof => "preuve de clé invalide sur cette connexion".to_owned(),
            AuthError::Unauthorised => "sujet ni le mien ni dans mon roster".to_owned(),
        }
    }
}

/// Verifies one authentication frame and authorises the peer it names.
///
/// `side` is MY side: it fixes the expected variant, the role the peer's proof must
/// carry, and the list of my roster that authorises it. `now` is unix seconds, so
/// the credential's window is checked against a clock the caller can pin (the
/// tests do, and no clock is read twice in one decision).
pub fn verify_peer(
    frame: &proto::Control,
    side: Side,
    trust: &TrustState,
    conn: &quinn::Connection,
    now: i64,
) -> Result<Authorised, AuthError> {
    let (cred, proof, proto) = match (side, frame) {
        (
            Side::Responder,
            proto::Control::Auth {
                cred, proof, proto, ..
            },
        )
        | (
            Side::Initiator,
            proto::Control::AuthOk {
                cred, proof, proto, ..
            },
        ) => (cred, proof, *proto),
        (Side::Responder, other) => {
            return Err(AuthError::Malformed(format!(
                "attendu Auth, reçu {}",
                control_tag(other)
            )));
        }
        (Side::Initiator, other) => {
            return Err(AuthError::Malformed(format!(
                "attendu AuthOk, reçu {}",
                control_tag(other)
            )));
        }
    };

    // The version first: it is not a security decision, it is "your app is too
    // old", and it costs one comparison.
    if proto != PROTO_MAJOR {
        return Err(AuthError::WrongMajor { peer: proto });
    }

    let raw = base64_decode(cred)
        .ok_or_else(|| AuthError::Malformed("attestation non base64".to_owned()))?;
    let cred = Credential::decode(&raw).map_err(AuthError::Credential)?;

    // The revocation list of the credential's own server key; a device revoked
    // under one key is refused even if the list was signed under another.
    let revoked = trust
        .revocations
        .get(&cred.kid)
        .cloned()
        .unwrap_or_else(|| Revocations::empty(cred.kid));
    cred.verify(&trust.keys, &revoked, now)
        .map_err(AuthError::Credential)?;

    // The proof is over THIS connection, in the role the peer claims, with the
    // public key the (verified) credential names.
    let signature =
        base64_decode(proof).ok_or_else(|| AuthError::Malformed("preuve non base64".to_owned()))?;
    verify_proof(&cred.device_pub, conn, side.peer_role(), &signature)
        .map_err(|_| AuthError::Proof)?;

    let (relation, scope) =
        relation_and_scope(&cred.subject, side, trust).ok_or(AuthError::Unauthorised)?;
    Ok(Authorised {
        cred,
        relation,
        scope,
    })
}

/// The authorisation decision, in one place.
///
/// - the peer is one of my own devices (`subject == mine`) → [`Relation::Own`], and
///   it sees everything I have;
/// - I am the responder and my roster has a `shares_to` entry for it → a friend,
///   with exactly the scope the server signed;
/// - I am the initiator and my roster lists it in `trusted_by` → a friend I may ask
///   (the scope is what it may fetch from ME, which my `shares_to` decides and which
///   may legitimately be nothing: it gets the empty friend scope, whose every answer
///   is `NotFound`);
/// - anything else → `None`, and the connection is closed before a single request
///   could exist.
pub fn relation_and_scope(
    peer: &Subject,
    side: Side,
    trust: &TrustState,
) -> Option<(Relation, Scope)> {
    let own = peer == &trust.subject;
    // What the peer may fetch from me, from MY roster: `Own` sees everything, a
    // friend exactly the union of what I share, anybody else nothing.
    let serving = Scope::for_peer(peer, &trust.subject, trust.roster.as_ref());
    if own {
        // My own device never needed the roster, and keeps seeing everything
        // while the roster is being refreshed.
        return Some((Relation::Own, serving?));
    }
    match side {
        Side::Responder => {
            // May this peer fetch from me? `Scope::for_peer` IS `shares_to`, and it
            // answered `None` above when the entry is absent.
            let scope = serving?;
            let name = trust
                .roster
                .as_ref()
                .and_then(|roster| roster.truster(peer))
                .map(|truster| truster.name.clone())
                .unwrap_or_default();
            Some((Relation::Friend { name }, scope))
        }
        Side::Initiator => {
            // May I fetch from this peer? Only if the server says it shares with
            // me. Its display name travels in `trusted_by`, never on the wire
            // before `Hello`.
            let truster = trust.roster.as_ref()?.truster(peer)?;
            Some((
                Relation::Friend {
                    name: truster.name.clone(),
                },
                serving.unwrap_or_else(nothing),
            ))
        }
    }
}

/// The scope of a peer that may fetch nothing from me: `Scope` has no third
/// variant, and the empty friend scope is exactly "not one of these tracks".
fn nothing() -> Scope {
    Scope::Friend {
        all: false,
        tracks: Arc::new(HashSet::new()),
    }
}

/// This instant, in unix seconds. One read per decision; a credential is valid for
/// days, so a second of drift between two decisions changes nothing.
pub fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

/// The tag of a control frame, for a refusal that names what arrived without
/// serialising the frame (which could carry a credential into a log).
fn control_tag(control: &proto::Control) -> &'static str {
    match control {
        proto::Control::Auth { .. } => "Auth",
        proto::Control::AuthOk { .. } => "AuthOk",
        proto::Control::Hello { .. } => "Hello",
        proto::Control::Revocations { .. } => "Revocations",
        proto::Control::Roster { .. } => "Roster",
        proto::Control::InventoryChanged { .. } => "InventoryChanged",
        proto::Control::SetsChanged { .. } => "SetsChanged",
        proto::Control::Bye => "Bye",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The note's numbers, and the two codes are distinct: one is a refusal, the
    /// other is "your app is too old", and a UI that mixed them would tell
    /// somebody to retry a version mismatch for ever.
    #[test]
    fn the_pre_auth_budget_and_the_two_close_codes() {
        assert_eq!(CLOSE_REFUSED, 0x10);
        assert_eq!(CLOSE_WRONG_MAJOR, 0x11);
        assert_ne!(CLOSE_REFUSED, CLOSE_WRONG_MAJOR);
        assert_eq!(PREAUTH_MAX_BYTES, 2048);
        assert_eq!(PREAUTH_DEADLINE.as_secs(), 2);
        assert_eq!(PROTO_MAJOR, 1);
    }

    /// One exporter label, 32 bytes, and the two roles spelled exactly as the
    /// signed message spells them: the label and the contexts are the whole channel
    /// binding.
    ///
    /// The message is 59 bytes, not the 58 the note's prose counts: the domain it
    /// writes as "17 of domain" is `neezer-lan/1 auth\0`, which is **18** bytes once
    /// its NUL terminator is counted (17 characters + 1). LN2's `cred.rs` is the
    /// implementation, and its vector is what LN1 signs against, so the number that
    /// matters is 18 + 9 + 32 = 59 — the prose is off by one, not the code.
    #[test]
    fn the_exporter_label_and_the_two_contexts_are_the_notes() {
        assert_eq!(EXPORTER_LABEL, b"EXPORTER-neezer-lan/1");
        assert_eq!(EXPORTER_BYTES, 32);
        assert_eq!(Role::Initiator.as_bytes(), b"initiator");
        assert_eq!(Role::Responder.as_bytes(), b"responder");
        assert_eq!(auth_message(Role::Initiator, &[0xAA; 32]).len(), 59);
    }

    /// Every refusal has a close code, and exactly one of them is not
    /// [`CLOSE_REFUSED`]: that is what makes "a stranger learns nothing" checkable
    /// — the same code, whichever cause, on the wire.
    #[test]
    fn every_refusal_shares_one_close_code_except_the_version_mismatch() {
        let causes = [
            AuthError::WrongMajor { peer: 2 },
            AuthError::Malformed("rien".to_owned()),
            AuthError::Credential(CredError::Expired),
            AuthError::Credential(CredError::Revoked),
            AuthError::Credential(CredError::UnknownKid),
            AuthError::Credential(CredError::Signature),
            AuthError::Proof,
            AuthError::Unauthorised,
        ];
        for cause in &causes {
            assert!(!cause.cause_fr().is_empty(), "{cause:?}");
            match cause {
                AuthError::WrongMajor { .. } => {
                    assert_eq!(cause.close_code(), CLOSE_WRONG_MAJOR, "{cause:?}")
                }
                _ => assert_eq!(cause.close_code(), CLOSE_REFUSED, "{cause:?}"),
            }
        }
        // The version mismatch keeps a cause a screen can act on; a refusal is
        // one word and nothing else.
        assert_eq!(
            AuthError::WrongMajor { peer: 2 }.as_lan_error(),
            LanError::Proto("protocole 2 non pris en charge".to_owned())
        );
        assert_eq!(AuthError::Proof.as_lan_error(), LanError::Refused);
    }

    /// The two roles are opposite, so a `Side` can never sign and verify with the
    /// same context.
    #[test]
    fn the_two_sides_sign_and_check_opposite_roles() {
        for side in [Side::Initiator, Side::Responder] {
            assert_ne!(side.role(), side.peer_role(), "{side:?}");
            assert_eq!(
                side.role(),
                match side {
                    Side::Initiator => Role::Initiator,
                    Side::Responder => Role::Responder,
                }
            );
        }
    }
}
