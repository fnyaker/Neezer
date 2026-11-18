//! The types every module shares: who a device is, who a person is on the LAN,
//! which qualities are tolerated, where a track came from, and the one error
//! this crate answers with.

use std::collections::BTreeSet;
use std::path::PathBuf;

pub use crate::proto::{Urgency, WireQuality};

/// A device's Ed25519 public key, the only identifier another device trusts.
///
/// Not a name and not a hash of one: the key IS the identity (Syncthing's rule),
/// and it rotates at every credential renewal, which is what bounds how long a
/// stranger who saw it can follow it.
pub type DevicePub = [u8; 32];

/// A person, named on the LAN by an opaque 16-byte value.
///
/// `HMAC-SHA256(get_secret_key("lan_subject"), "neezer-lan/1 subject" ‖ user_id)[0..16]`
/// — stable, unguessable, no column anywhere, and never the account name. Two
/// devices of one person compare this to know they are "own" devices.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Subject(pub [u8; 16]);

/// The id of a server LAN key: `SHA-256(public key)[0..8]`.
///
/// A credential names the key that signed it, so a server can rotate its key and
/// still validate what the previous one signed until that expires.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Kid(pub [u8; 8]);

/// The qualities the person accepts from a PEER while the server is reachable
/// (`Réglages` → « Qualités tolérées »).
///
/// It is a set, not a ranking: [`crate::decide`] asks a peer for the wanted
/// quality when it has it, and otherwise for the best of these it does have.
/// The default is `{FLAC, OPUS_320, OPUS_128}` — Opus 64 is what the Auto ladder
/// picks on a bad link, and taking it from a peer while the server could send
/// better would be a regression. With the server unreachable the peer's quality
/// is always accepted: a worse copy beats silence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tolerance(pub BTreeSet<nz_api::Quality>);

impl Tolerance {
    /// `{FLAC, OPUS_320, OPUS_128}` — the note's default, and the one
    /// `Réglages` starts from.
    pub fn default_set() -> Self {
        Tolerance(
            [
                nz_api::Quality::Flac,
                nz_api::Quality::Opus320,
                nz_api::Quality::Opus128,
            ]
            .into_iter()
            .collect(),
        )
    }

    /// Whether a quality a peer offers may be taken while the server answers.
    pub fn contains(&self, q: nz_api::Quality) -> bool {
        self.0.contains(&q)
    }

    /// The best tolerated quality out of the ones offered, if any — the peer
    /// answer [`crate::decide`] ranks its candidates with.
    ///
    /// `nz_api::Quality`'s `Ord` IS the quality order (ascending, FLAC last), so
    /// this cannot disagree with a second table.
    pub fn best_of(
        &self,
        offered: impl IntoIterator<Item = nz_api::Quality>,
    ) -> Option<nz_api::Quality> {
        offered.into_iter().filter(|q| self.contains(*q)).max()
    }
}

impl Default for Tolerance {
    fn default() -> Self {
        Self::default_set()
    }
}

/// How another node is related to this person — the answer authentication gives,
/// and what the screens show beside a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Relation {
    /// The same subject: one of my own devices.
    Own,
    /// A person in my roster who shares with me: what they are called is only
    /// known from the signed roster, never from the wire.
    Friend { name: String },
}

/// Where a track will be taken from. The decision is local and instant
/// ([`crate::decide`]); this is its answer, and what the UI reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A complete file on this device (a download, then the play cache).
    Local(PathBuf),
    /// A peer that has it, at the quality it will serve.
    Peer {
        peer: DevicePub,
        quality: nz_api::Quality,
    },
    /// The server, at the quality asked.
    Server(nz_api::Quality),
}

/// Everything this crate can fail with, in the words a screen can show.
///
/// The variants are deliberately coarse where a peer could learn something from
/// a finer answer: [`LanError::Refused`] is what EVERY failed authentication
/// answers, whatever the real cause was (unknown key, expired credential,
/// revoked device, a proof signed over another connection), and the precise
/// cause is only ever in this device's own log and its own tests.
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum LanError {
    /// « Échanger avec mes appareils à proximité » is off (per device).
    #[error("l'échange entre appareils proches est désactivé")]
    Disabled,
    /// The server predates the feature, or an admin switched it off: nothing is
    /// started and no screen mentions it (rule 7).
    #[error("ce serveur n'a pas l'échange entre appareils proches")]
    Unsupported,
    /// This device holds no credential yet (never enrolled, or logged out).
    #[error("cet appareil n'est pas encore inscrit auprès du serveur")]
    NotEnrolled,
    /// The credential is past its end, and no server answered to renew it.
    #[error("l'inscription de cet appareil a expiré : reconnectez-vous au serveur")]
    Expired,
    /// Authentication failed. One variant for every cause, on purpose.
    #[error("appareil refusé")]
    Refused,
    /// The connection went away, or the peer stopped answering.
    #[error("l'appareil ne répond plus")]
    PeerGone,
    /// The peer does not have the track, or it is outside what it shares with
    /// us: the SAME answer for both, built by the same function.
    #[error("ce titre n'est pas disponible sur cet appareil")]
    NotFound,
    /// The responder is already serving as much as it will (rule 1 on its own
    /// resources): wait exactly this long.
    #[error("appareil occupé, réessayez dans {retry_in_ms} ms")]
    Busy { retry_in_ms: u32 },
    /// The bytes received are not the file that was announced (length, SHA-256,
    /// or the decoder probe).
    #[error("le fichier reçu n'est pas celui qui a été annoncé")]
    Mismatch,
    /// A file the peer offered is unusable, with the cause kept for the log.
    #[error("{0}")]
    BadFile(String),
    /// The local disk refused the write (full, read-only, a vanished directory).
    #[error("{0}")]
    Disk(String),
    /// A frame or a message this build cannot make sense of (a newer major,
    /// a size over the cap).
    #[error("{0}")]
    Proto(String),
    /// The server's answer was a refusal, with the server's own cause.
    #[error("{0}")]
    Server(String),
    /// A bug, or a state this crate did not expect. Logged with its cause.
    #[error("{0}")]
    Internal(String),
}

impl LanError {
    /// The failure in the one French sentence a screen shows (rule 3: a screen
    /// that cannot do the thing says why; rule 5: the copy is French).
    ///
    /// It is `Display`, spelled out variant by variant, so a caller that wants
    /// the string does not have to format an error it then throws away.
    pub fn cause_fr(&self) -> String {
        match self {
            LanError::Disabled => {
                "L'échange entre appareils proches est désactivé dans les réglages.".to_owned()
            }
            LanError::Unsupported => {
                "Ce serveur n'a pas l'échange entre appareils proches.".to_owned()
            }
            LanError::NotEnrolled => {
                "Cet appareil n'est pas encore inscrit : reconnectez-vous au serveur.".to_owned()
            }
            LanError::Expired => {
                "L'inscription de cet appareil a expiré : reconnectez-vous au serveur.".to_owned()
            }
            LanError::Refused => "Cet appareil a refusé la connexion.".to_owned(),
            LanError::PeerGone => "L'appareil ne répond plus.".to_owned(),
            LanError::NotFound => "Ce titre n'est pas disponible sur cet appareil.".to_owned(),
            LanError::Busy { retry_in_ms } => {
                format!(
                    "L'appareil est occupé, réessayez dans {} s.",
                    retry_in_ms.div_ceil(1_000)
                )
            }
            LanError::Mismatch => "Le fichier reçu ne correspond pas à celui annoncé.".to_owned(),
            LanError::BadFile(cause) => format!("Fichier inutilisable : {cause}"),
            LanError::Disk(cause) => format!("Espace disque insuffisant : {cause}"),
            LanError::Proto(cause) => format!("Échange incompréhensible avec l'appareil : {cause}"),
            LanError::Server(cause) => format!("Le serveur a refusé : {cause}"),
            LanError::Internal(cause) => format!("Erreur interne de l'échange : {cause}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default tolerance is the note's three rungs, and Opus 64 is NOT one of
    /// them: taking it from a peer while the server could send better is the
    /// regression the setting exists to prevent.
    #[test]
    fn the_tolerated_set_is_flac_320_and_128() {
        let tolerance = Tolerance::default_set();
        assert!(tolerance.contains(nz_api::Quality::Flac));
        assert!(tolerance.contains(nz_api::Quality::Opus320));
        assert!(tolerance.contains(nz_api::Quality::Opus128));
        assert!(!tolerance.contains(nz_api::Quality::Opus64));
        assert_eq!(tolerance.0.len(), 3);
        assert_eq!(Tolerance::default(), tolerance);
    }

    /// "The best tolerated copy a peer has" comes from `Quality`'s own `Ord`
    /// (see `nz-model`'s `the_declaration_order_is_the_quality_order`): FLAC wins,
    /// and an Opus 64-only peer offers nothing the default set accepts.
    #[test]
    fn the_best_offered_copy_is_the_highest_tolerated_one() {
        let tolerance = Tolerance::default_set();
        assert_eq!(
            tolerance.best_of([nz_api::Quality::Opus128, nz_api::Quality::Flac]),
            Some(nz_api::Quality::Flac)
        );
        assert_eq!(
            tolerance.best_of([nz_api::Quality::Opus64]),
            None,
            "Opus 64 is outside the default set"
        );
        assert_eq!(
            tolerance.best_of([nz_api::Quality::Opus256, nz_api::Quality::Opus320]),
            Some(nz_api::Quality::Opus320)
        );
    }

    /// Every variant has a French sentence, and none of them is the Debug
    /// spelling or an empty string: rule 3 is that a failure always says why.
    #[test]
    fn every_failure_has_a_french_sentence() {
        let all = [
            LanError::Disabled,
            LanError::Unsupported,
            LanError::NotEnrolled,
            LanError::Expired,
            LanError::Refused,
            LanError::PeerGone,
            LanError::NotFound,
            LanError::Busy { retry_in_ms: 1_500 },
            LanError::Mismatch,
            LanError::BadFile("codec".into()),
            LanError::Disk("plein".into()),
            LanError::Proto("version".into()),
            LanError::Server("409".into()),
            LanError::Internal("bug".into()),
        ];
        for error in all {
            let sentence = error.cause_fr();
            assert!(!sentence.is_empty(), "{error:?}");
            assert!(
                sentence.contains(' ') && !sentence.contains("LanError"),
                "{error:?}: {sentence}"
            );
            assert_ne!(sentence, error.to_string(), "{error:?} is not just Display");
        }
        // The busy sentence rounds UP: 1 500 ms is "2 s", never "1 s", because a
        // retry before the peer is ready is a wasted request.
        assert_eq!(
            LanError::Busy { retry_in_ms: 1_500 }.cause_fr(),
            "L'appareil est occupé, réessayez dans 2 s."
        );
    }
}
