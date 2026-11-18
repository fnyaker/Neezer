//! The full-screen player's data: the lyrics of the playing title, the lyric
//! card's rules, and the podcast speed a show is remembered at.
//!
//! Owned by **MB11** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this directory and edits no other file of the crate. The player
//! itself — queue, transport, position, the sleep timer's countdown — is MB3b's
//! `nz-core::player`, and this area never keeps a second copy of it.
//!
//! # What a screen asks, and what it costs
//!
//! | Screen | Cold | Warm (to paint) | The SPA's file |
//! |---|---|---|---|
//! | Paroles (the lyrics view, the current line on the player) | 1 | 0 | `lib/lyrics.js#api.lyrics` (`GET /api/lyrics/<id>`) |
//! | Créer une carte (the lyric card) | 0 | 0 | `lib/lyriccard.js` (rules only, no request) |
//! | « À propos » → Titres similaires | 1 | 0 | `NowPlayingMore.svelte#api.trackRadio` (`GET /api/radio/track/<id>`) |
//! | Vitesse d'un podcast | 0 | 0 | `lib/speed.js` (a value in the device's own store) |
//!
//! **The lyrics are asked for ONCE per title, and not before the audio is under
//! way.** `lib/lyrics.js` fetches on the `playable` signal, not on the track
//! change: nobody reads along before the song starts, and a lyrics request that
//! races its own audio spends the first second of a mobile link on words the
//! person is not looking at yet. The FFI keeps both halves (`cached_lyrics`,
//! `lyrics`) and the Kotlin side decides WHEN to ask — that ordering is the
//! screen's, exactly as `ladder.js` is the SPA's.
//!
//! # What is kept, and where
//!
//! The server's own body, per account, in the CACHE directory ([`store`]): a
//! revisit and a second device paint the words with **zero requests**, and the
//! `.lrc` sidecar the server reads is the server's own business. The key is a
//! TRACK REF (a Deezer number or a row UUID), never a path, and the file name is
//! a hash of it ([`store`]).
//!
//! # The lyric card is RULES here, pixels on the device
//!
//! `lib/lyriccard.js` is the pure half of the SPA's card: which lines a card can
//! offer, which one it opens on, what a tap does at the limit, the file's name.
//! That half is ported here ([`card`]) and held by tests. The other half —
//! `LyricCard.svelte` — draws a 1080×1350 canvas with the app's own fonts and
//! saves a PNG; on the phone that is Android's `Canvas` and `Bitmap.compress`,
//! because the text metrics the layout needs are the platform's and the mobile
//! Rust workspace carries no text rasteriser (see the lot's section of
//! `docs/agents/mobile.md`: the measured reason, and what it costs).
//!
//! # Podcast speed is per show, never for music
//!
//! `lib/speed.js`: a rate is remembered under the SHOW (`channel_id`, else the
//! album's provider id), a value outside 0.5–4 is not believed, and 1 forgets
//! the entry. Music is never played faster — the player applies the rate it is
//! given, so the reconcile step only ever runs for an episode and puts 1 back
//! when music starts.

pub mod card;
mod speed;
mod store;
mod wire;

#[cfg(test)]
mod testing;

pub use speed::SPEEDS;

use crate::core::{Core, CoreError};
use crate::lists::Row;

/// The tag every request of this area is counted under (`NzCore::stats`).
pub const TAG_NOWPLAYING: &str = "nowplaying";

/// One synced line of a song's words (`parse_lrc`'s own shape: milliseconds).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LyricLine {
    pub time_ms: u32,
    pub text: String,
}

/// `/api/lyrics/<id>`: the two shapes the server answers (and both at once for a
/// `.lrc` that carries timestamps).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lyrics {
    /// Empty for plain words: the screen then prints [Lyrics::plain] as text.
    pub synced: Vec<LyricLine>,
    /// The whole song on several lines, timestamped or not.
    pub plain: String,
    /// `archive` when the words came from the track's own `.lrc` sidecar.
    pub source: String,
}

impl Lyrics {
    /// Nothing to show: the view says « Paroles indisponibles pour ce titre ».
    pub fn is_empty(&self) -> bool {
        self.synced.is_empty() && self.plain.trim().is_empty()
    }

    /// The bytes this answer holds, counted (worker rule 4).
    pub fn memory_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.plain.capacity()
            + self.source.capacity()
            + self
                .synced
                .iter()
                .map(|line| std::mem::size_of::<LyricLine>() + line.text.capacity())
                .sum::<usize>()
    }
}

/// What one tag spent, as `nz-api` counts it (SC3): requests really sent, and the
/// response bodies' bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Totals {
    requests: u64,
    bytes_in: u64,
    bytes_out: u64,
}

/// The `nowplaying` tag's running totals on this client.
fn totals(api: &nz_api::Api) -> Totals {
    api.stats()
        .into_iter()
        .find(|row| row.tag == TAG_NOWPLAYING)
        .map(|row| Totals {
            requests: row.requests,
            bytes_in: row.bytes_in,
            bytes_out: row.bytes_out,
        })
        .unwrap_or_default()
}

impl Core {
    // -- the words of a title -------------------------------------------------

    /// The lyrics as this device last saw them — disk only, **0 requests**. `None`
    /// before the first read; a title the server has no words for is kept too
    /// (an EMPTY answer is an answer: the SPA marks it tried and stops asking).
    pub fn cached_lyrics(&self, key: &str) -> Option<Lyrics> {
        wire::lyrics(&store::read(self, store::LYRICS, key)?).ok()
    }

    /// `GET /api/lyrics/<id>` — ONE request on the `User` lane, kept for the next
    /// visit. A route the server does not have yet is `Unsupported`, which the
    /// screen shows as « pas de paroles » rather than as a failure (rule 7).
    pub async fn lyrics(&self, key: &str) -> Result<Lyrics, CoreError> {
        let api = self.api()?.with_tag(TAG_NOWPLAYING);
        let before = totals(&api);
        let answer = async {
            let body = wire::fetch_lyrics(&api, key).await?;
            let lyrics = wire::lyrics(&body)?;
            keep(self, store::LYRICS, key, &body);
            Ok(lyrics)
        }
        .await;
        book(self, &api, before);
        answer
    }

    // -- what sounds like the playing title -----------------------------------

    /// `GET /api/radio/track/<id>` — one request for the « Titres similaires »
    /// section of « À propos ». The seed is left IN the list: the SPA filters the
    /// playing row out of the section itself, and a radio that arrives without
    /// its seed would answer a different question on every call.
    pub async fn similar_tracks(&self, key: &str) -> Result<Vec<Row>, CoreError> {
        let api = self.api()?.with_tag(TAG_NOWPLAYING);
        let before = totals(&api);
        let answer = async {
            let body = wire::fetch_track_radio(&api, key).await?;
            wire::track_rows(&body)
        }
        .await;
        book(self, &api, before);
        answer
    }

    // -- the lyric card -------------------------------------------------------

    /// The lines a card can offer, from either shape of an answer — the whole of
    /// `lib/lyriccard.js#lyricLines`, and the reason a blank marker between two
    /// verses must not shift the selection by one.
    pub fn card_lines(&self, lyrics: &Lyrics) -> Vec<card::CardLine> {
        card::lines(lyrics)
    }

    /// One tap on a line: the new selection, or the reason nothing changed.
    pub fn card_toggle(&self, selected: &[u32], index: u32) -> card::Selection {
        card::toggle(selected, index)
    }

    /// The text of the selected lines, in LYRIC order, at most three.
    pub fn card_texts(&self, lyrics: &Lyrics, selected: &[u32]) -> Vec<String> {
        card::choose(&card::lines(lyrics), selected)
    }

    /// `neezer-<artist>-<title>.png`, safe on every filesystem and share target.
    pub fn card_filename(&self, artist: &str, title: &str) -> String {
        card::filename(artist, title)
    }

    /// Which line the card opens on: the one being heard, else the first.
    pub fn card_default_line(&self, lyrics: &Lyrics, active_source: i64) -> u32 {
        card::default_line(&card::lines(lyrics), active_source)
    }

    // -- a podcast's rate -----------------------------------------------------

    /// The rate the show is played at: 1 unless a sane one was chosen.
    pub fn podcast_speed(&self, show: &str) -> f32 {
        speed::speed_for(self, show)
    }

    /// Remembers [rate] for [show]; 1 forgets the entry (a rate nobody chose is
    /// not a setting to keep). A rate the engine cannot play is not stored.
    pub fn set_podcast_speed(&self, show: &str, rate: f32) -> Result<(), CoreError> {
        speed::set_speed(self, show, rate)
    }

    /// Every remembered rate, as `(show, rate)` — the reconcile step's own read.
    pub fn podcast_speeds(&self) -> Vec<(String, f32)> {
        speed::all(self)
    }
}

// -- the book keeping (the entity area's two helpers, for THIS tag) -----------
//
// Free functions and not `Core` methods: an inherent `impl Core` elsewhere in
// the crate already owns `keep`/`book` for the entity area, and two inherent
// methods of one name on one type do not compile.

/// Keeps one body in the area's cache; a cache that cannot be written never
/// costs the screen (the fresh answer is already in hand).
fn keep(core: &Core, kind: &str, key: &str, body: &str) {
    if let Err(cause) = store::write(core, kind, key, body) {
        tracing::debug!(%cause, "a nowplaying cache file was not written");
    }
}

/// Books one call's counted cost, so `NzCore::stats` answers what the screen
/// spent even when the answer failed.
fn book(core: &Core, api: &nz_api::Api, before: Totals) {
    let after = totals(api);
    let requests = after.requests.saturating_sub(before.requests);
    if requests == 0 {
        return;
    }
    // `Meter::record` counts ONE request and its bytes: the whole run's bytes go
    // with the first, the remaining N−1 are zeros.
    core.meter().record(
        TAG_NOWPLAYING,
        after.bytes_in.saturating_sub(before.bytes_in),
        after.bytes_out.saturating_sub(before.bytes_out),
    );
    for _ in 1..requests {
        core.meter().record(TAG_NOWPLAYING, 0, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **One request for the words, one for the similar titles, and NOTHING on a
    /// warm open** — the screen standard's rule 1, counted (the SPA's own
    /// `lib/lyrics.js` asks once per title and paints the copy first).
    ///
    /// The bytes are the server's own answer sizes, so the PR's figures are the
    /// ones this test holds.
    #[tokio::test]
    async fn the_words_cost_one_request_and_a_warm_open_none() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();

        let lyrics = core.lyrics("9001").await.expect("the words");
        assert_eq!(server.count(), 1, "the lyrics view: ONE request");
        assert_eq!(lyrics.synced.len(), 3);
        assert_eq!(lyrics.synced[2].text, "Paris s'éveille");
        assert_eq!(lyrics.synced[2].time_ms, 24_000);
        assert_eq!(lyrics.plain, "Il est cinq heures\n♪\nParis s'éveille");
        assert_eq!(lyrics.source, "archive");
        let cold_bytes = testing::LYRICS.len();
        let row = core
            .stats()
            .into_iter()
            .find(|row| row.tag == TAG_NOWPLAYING)
            .expect("the tag is counted");
        println!(
            "MEASURE lyrics cold: {} request, {} bytes in (the answer is {cold_bytes})",
            row.requests, row.bytes_in
        );
        assert_eq!(row.requests, 1);
        assert_eq!(row.bytes_in as usize, cold_bytes);

        // Warm: the copy answers, no socket, no request.
        server.forget();
        let sockets = server.accepted();
        let again = core.cached_lyrics("9001").expect("the copy");
        assert_eq!(again, lyrics);
        assert_eq!(server.count(), 0, "a warm open asks nothing");
        assert_eq!(server.accepted(), sockets, "and opens no socket");

        // A title without words is an ANSWER, kept: the view says so once.
        server.forget();
        let none = core.lyrics("9002").await.expect("a null answer");
        assert!(none.is_empty());
        assert_eq!(server.count(), 1);
        assert!(
            core.cached_lyrics("9002").is_some(),
            "kept, like any answer"
        );

        // « Titres similaires »: one request, the seed included.
        server.forget();
        let similar = core.similar_tracks("9001").await.expect("the radio");
        assert_eq!(server.routes(), vec!["GET /api/radio/track/9001"]);
        assert_eq!(similar.len(), 2, "the seed is in the server's own list");
        assert_eq!(similar[1].title, "Blizzard");
        assert_eq!(similar[1].artists, "Fauve");
        let row = core
            .stats()
            .into_iter()
            .find(|row| row.tag == TAG_NOWPLAYING)
            .expect("the tag");
        println!("MEASURE nowplaying totals: {row:?}");
        core.shutdown();
    }

    /// A dead link fails fast, opens NO socket, and keeps what the person saw.
    #[tokio::test]
    async fn a_dead_link_opens_no_socket_and_the_copy_still_paints() {
        let (server, _dir, core) = testing::configured().await;
        core.lyrics("9001").await.expect("one good visit");
        assert!(core.cached_lyrics("9001").is_some());

        core.set_network(crate::core::NetworkState::None);
        server.forget();
        let before = server.accepted();
        let error = core.lyrics("9001").await.expect_err("offline");
        assert_eq!(error, CoreError::Offline);
        assert_eq!(
            server.accepted(),
            before,
            "the gate answered in zero sockets"
        );
        assert_eq!(server.count(), 0);
        assert!(
            core.cached_lyrics("9001").is_some(),
            "the copy stays for the view"
        );
        core.shutdown();
    }

    /// A refused route keeps its cause: the server's answer, not a sentence.
    #[tokio::test]
    async fn a_broken_answer_names_its_status() {
        let (server, _dir, core) = testing::configured().await;
        server.break_route("/api/lyrics/9001", 500);
        let error = core.lyrics("9001").await.expect_err("a 500");
        assert!(
            matches!(error, CoreError::Server { status: 500, .. }),
            "{error:?}"
        );
        assert!(core.cached_lyrics("9001").is_none(), "nothing to keep");
        core.shutdown();
    }

    /// A `.lrc` answer: the synced list and the plain text come out of ONE body,
    /// and an empty answer is a keepable answer.
    #[test]
    fn the_two_shapes_of_an_answer_are_read_from_the_same_body() {
        let body = r#"{"lyrics":{"text":"Bonjour\nBonsoir","synced":[
            {"time":0,"text":"Bonjour"},{"time":4000,"text":"Bonsoir"}],
            "source":"archive"}}"#;
        let lyrics = wire::lyrics(body).expect("the answer parses");
        assert_eq!(lyrics.synced.len(), 2);
        assert_eq!(lyrics.synced[1].time_ms, 4_000);
        assert_eq!(lyrics.plain, "Bonjour\nBonsoir");
        assert_eq!(lyrics.source, "archive");
        assert!(!lyrics.is_empty());

        let none = wire::lyrics(r#"{"lyrics":null}"#).expect("a null answer");
        assert!(none.is_empty(), "no words is a state, not a failure");
    }

    /// A hostile or truncated body is an internal fault with the route in it, not
    /// a silent empty view.
    #[test]
    fn a_broken_body_names_its_route() {
        let error = wire::lyrics("{not json").expect_err("garbage");
        assert!(error.to_string().contains("lyrics"), "{error}");
    }
}
