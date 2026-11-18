//! What this device REMEMBERS about the party, in `nz-cache`'s `kv`.
//!
//! Three values are the listener's own and the SPA keeps them in
//! `localStorage` (`guest.js`): their name (`party.name`), the party's volume
//! (`party.volume`) and the output trim they tuned by ear (`party.latency`, the
//! *Décalage* chip of `Party.svelte`). They are re-read on the next join, so a
//! person's phone sounds the same in the next room.
//!
//! The fourth is this lot's own and it is rule 3's: the landing card's last
//! answer, kept so that **opening a party you already opened costs no blocking
//! request** — the card paints from disk at once, then the peek refreshes it. One
//! row only (`party.peek`): a person opens one party per evening, and a store
//! that grew one row per scanned link would be a cache with no bound.

use nz_cache::Cache;
use nz_model::PartyState;

/// The listener's name (`guest.js#NAME_KEY`).
pub const NAME_KEY: &str = "party.name";
/// The party's own volume (`guest.js#VOLUME_KEY`).
pub const VOLUME_KEY: &str = "party.volume";
/// The device's output trim, milliseconds (`guest.js#LATENCY_KEY`).
pub const TRIM_KEY: &str = "party.latency";
/// The landing card's last answer, `{id, at, state}`.
pub const PEEK_KEY: &str = "party.peek";

/// How long a remembered card may be shown: past this, a link that died while
/// the app was closed would still look alive. The server's own `PARTY_TTL` is
/// 30 minutes of silence; a day is the generous outer bound of "I scanned this
/// tonight".
pub const PEEK_TTL_MS: u64 = 24 * 60 * 60 * 1000;

/// The three values the listener owns.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub name: String,
    pub volume: f64,
    pub trim_ms: f64,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            // The SPA's placeholder is « Invité » (`Party.svelte`), and 0 dB of
            // trim is "the OS already reports its latency".
            name: String::new(),
            volume: 1.0,
            trim_ms: 0.0,
        }
    }
}

impl Prefs {
    /// The name the host will see: the person's, or the SPA's own fallback.
    pub fn display_name(&self) -> String {
        let trimmed = self.name.trim();
        if trimmed.is_empty() {
            "Invité".to_owned()
        } else {
            trimmed.chars().take(32).collect()
        }
    }
}

/// Read the three values. Never fails: a store that cannot answer is a person
/// who never set them, which the defaults say.
pub fn load(cache: &Cache) -> Prefs {
    let kv = cache.kv();
    Prefs {
        name: kv
            .get::<String>(NAME_KEY)
            .unwrap_or_default()
            .chars()
            .take(32)
            .collect(),
        volume: kv
            .get::<f64>(VOLUME_KEY)
            .map(|volume| volume.clamp(0.0, 1.0))
            .unwrap_or(1.0),
        trim_ms: kv
            .get::<f64>(TRIM_KEY)
            .map(|ms| ms.clamp(-500.0, 1000.0).round())
            .unwrap_or(0.0),
    }
}

pub fn save_name(cache: &Cache, name: &str) {
    let _ = cache
        .kv()
        .set(NAME_KEY, &name.chars().take(32).collect::<String>());
}

pub fn save_volume(cache: &Cache, volume: f64) {
    let _ = cache.kv().set(VOLUME_KEY, &volume.clamp(0.0, 1.0));
}

pub fn save_trim(cache: &Cache, ms: f64) {
    let _ = cache.kv().set(TRIM_KEY, &ms.clamp(-500.0, 1000.0).round());
}

/// The last answer the landing card painted, if it is still the same party and
/// still recent enough to show.
pub fn cached_peek(cache: &Cache, id: &str) -> Option<PartyState> {
    let value = cache.kv().get::<serde_json::Value>(PEEK_KEY)?;
    let cached = value.get("id").and_then(serde_json::Value::as_str)?;
    if cached != id {
        return None;
    }
    let at = value.get("at").and_then(serde_json::Value::as_u64)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    if now.saturating_sub(at) > PEEK_TTL_MS {
        return None;
    }
    serde_json::from_value(value.get("state")?.clone()).ok()
}

/// Keep one answer for the next open.
pub fn save_peek(cache: &Cache, state: &PartyState) {
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    let value = serde_json::json!({ "id": state.id, "at": at, "state": state });
    let _ = cache.kv().set(PEEK_KEY, &value);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache() -> crate::party::tests::TempCore {
        crate::party::tests::TempCore::new("party-store")
    }

    /// A person's three values survive the app being killed: the store is
    /// SQLite, and the next join reads them back unchanged.
    #[test]
    fn the_listeners_three_values_come_back() {
        let dir = cache();
        let cache = dir.core().cache();
        let fresh = load(cache);
        assert_eq!(fresh, Prefs::default());
        assert_eq!(fresh.display_name(), "Invité");

        save_name(cache, "  Camille de la pièce du fond, vraiment  ");
        save_volume(cache, 0.42);
        save_trim(cache, 149.6);
        let back = load(cache);
        // A name is bounded to the server's 32 characters (`_text(…, 32)`) and
        // the person's own spaces go: 32 kept, 30 shown.
        assert_eq!(back.name.chars().count(), 32);
        assert_eq!(
            back.display_name(),
            "Camille de la pièce du fond, vraiment"
                .chars()
                .take(30)
                .collect::<String>()
        );
        assert_eq!(back.volume, 0.42);
        // A trim is a whole millisecond: the SPA's chip prints an integer.
        assert_eq!(back.trim_ms, 150.0);
    }

    /// Hostile values neither crash nor escape their range (the store is a file
    /// on the device, and a person can edit it).
    #[test]
    fn a_doctored_store_is_clamped_not_trusted() {
        let dir = cache();
        let cache = dir.core().cache();
        let _ = cache.kv().set(VOLUME_KEY, &9.0_f64);
        let _ = cache.kv().set(TRIM_KEY, &-9000.0_f64);
        let prefs = load(cache);
        assert_eq!(prefs.volume, 1.0);
        assert_eq!(prefs.trim_ms, -500.0);
    }

    /// The card's copy is per party: another link never shows this one's track.
    #[test]
    fn the_remembered_card_is_that_party_and_nothing_else() {
        let dir = cache();
        let cache = dir.core().cache();
        assert!(cached_peek(cache, "p1").is_none());
        let state = PartyState {
            id: "p1".to_owned(),
            host: "Camille".to_owned(),
            v: 4,
            ..PartyState::default()
        };
        save_peek(cache, &state);
        let back = cached_peek(cache, "p1").expect("the remembered card");
        assert_eq!(back.host, "Camille");
        assert_eq!(back.v, 4);
        assert!(
            cached_peek(cache, "p2").is_none(),
            "another id is another party"
        );
    }
}
