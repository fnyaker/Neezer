//! The queue and the transport as a pure state machine, with the web player's
//! own rules.
//!
//! Owned by lot D4 (`docs/agents/desktop.md`): `PlayerState::apply(Command) ->
//! Vec<Effect>`, no I/O, no clock reading, so every rule is a unit test and
//! remote control maps 1:1 onto the SPA's `commands.js`.

/// Placeholder for D4's transport rules. Kept out of the way of the lot: the
/// module is empty on purpose and D4 replaces this file's body.
pub const CRATE: &str = "nz-player";

#[cfg(test)]
mod tests {
    use super::CRATE;

    #[test]
    fn the_crate_is_named_after_the_player() {
        assert_eq!(CRATE, "nz-player");
    }
}
