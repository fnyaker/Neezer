//! The GUEST side on the phone: the landing card's read, the seat, and the two
//! causes a person can actually meet.
//!
//! The session itself — the clock probe, the once-a-second poll (four times
//! faster near a handover), the six-second chunk fetches, the scheduler that
//! places them on the output clock — is `nz-party`'s `guest.rs` (SC6), started
//! from [`super::Party::join`]. This file is only what that crate asks the APP
//! for: the client to talk with, the player's channel to play through, and the
//! two refusals the SPA gives its own words to.

use nz_api::{Api, ApiError};
use nz_model::PartyState;
use nz_party::Guest;

use crate::core::CoreError;

/// `GET /api/party/<id>` — the landing card. `None` when the party does not
/// exist, which is the SPA's `missing` phase, not a failure.
pub async fn peek(api: &Api, id: &str) -> Result<Option<PartyState>, CoreError> {
    Guest::peek(api, id).await.map_err(CoreError::from)
}

/// `POST /api/party/<id>/join` — take a seat. The listener id it answers
/// authorises the guest's own `leave` and nothing else.
pub async fn take_seat(api: &Api, id: &str, name: &str) -> Result<String, ApiError> {
    Guest::join(api, id, name).await
}

/// The French sentence for a join the server refused.
///
/// `429` is the SPA's `party full` (`MAX_LISTENERS`) and deserves its own words:
/// « réessayez » would be a lie, another link is what the person needs. Anything
/// else keeps `nz-api`'s cause, which is the register every screen prints.
pub fn join_cause(error: &ApiError) -> String {
    match error {
        ApiError::Http { status: 429, .. } => {
            "Cette party est complète : demandez un autre lien.".to_owned()
        }
        ApiError::Http { status: 404, .. } => "Cette party n'existe plus.".to_owned(),
        other => other.cause_fr(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two refusals a person meets have the SPA's own register, and every
    /// other failure keeps `nz-api`'s cause instead of a code.
    #[test]
    fn the_two_refusals_say_what_to_do() {
        assert_eq!(
            join_cause(&ApiError::Http {
                status: 429,
                message: String::new(),
                code: None,
            }),
            "Cette party est complète : demandez un autre lien."
        );
        assert_eq!(
            join_cause(&ApiError::Http {
                status: 404,
                message: String::new(),
                code: None,
            }),
            "Cette party n'existe plus."
        );
        let cause = join_cause(&ApiError::Offline);
        assert!(!cause.is_empty());
        assert!(!cause.contains("429") && !cause.contains("HTTP"), "{cause}");
    }
}
