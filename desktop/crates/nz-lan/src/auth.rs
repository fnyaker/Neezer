//! The exporter-bound proof exchange, and the authorisation decision that
//! follows it (docs/agents/lan-nodes.md, *Transport and authentication*).
//!
//! **LN0 declares the two application close codes — the rest is LN6.** The shape
//! of the exchange is fixed here so every lot reads the same rules:
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
}
