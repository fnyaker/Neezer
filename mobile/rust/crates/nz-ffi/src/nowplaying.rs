//! The full-screen player's data — the FFI surface of lot MB11.
//!
//! Owned by **MB11** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this file — its own `#[uniffi::export] impl` blocks, its own records
//! — and edits no other file of the crate. UniFFI merges the `impl Screens` block
//! below into the ONE Kotlin `Screens` class, so nothing shared is touched.
//!
//! # What crosses, and what does not
//!
//! - **The words**: `cached_lyrics()` (disk only, what a warm open paints in its
//!   first frame) then `lyrics()` (one request on the `User` lane). The screen
//!   decides WHEN to ask — the SPA asks once the audio is under way
//!   (`lib/lyrics.js`), never on the track change.
//! - **The lyric card's RULES**: which lines it can offer, which one it opens on,
//!   what a tap does at the limit, the selected texts, the file's name. Ported
//!   from `lib/lyriccard.js` and held by Rust tests.
//! - **The card's pixels do NOT cross**: they are drawn by Android's `Canvas`
//!   with the design system's own fonts, because the text metrics the layout
//!   needs are the platform's (the lot's section of `docs/agents/mobile.md`
//!   carries the measured reason). What Rust owns is every DECISION the person
//!   can feel, which is what `lib/lyriccard.js` owns in the SPA.
//! - **« Titres similaires »**: `similar_tracks()` is one request
//!   (`GET /api/radio/track/<id>`) and crosses as a `ListHandle`, so a radio of
//!   sixty rows never crosses whole.
//! - **A podcast's rate**: `podcast_speed()`/`set_podcast_speed()` keep it per
//!   SHOW in the device's store (`lib/speed.js`); music is never affected.
//!
//! # What is NOT here, on purpose
//!
//! - **The credits**: MB7 already answers `/api/track/<id>/credits`
//!   (`track_credits`) and owns `TrackCreditsHost`, the sheet MB11's « À propos »
//!   opens. A second FFI method would be a second spelling of one page.
//! - **The transport, the queue, the position and the sleep timer**: MB3b's
//!   `PlayerApi` (`command`, `queue()`, `position()`, `set_sleep`). The sleep
//!   menu reads `snapshot().sleep` and sends `set_sleep`; a local countdown here
//!   would disagree with the engine that actually fades.

use std::sync::Arc;

use nz_core::nowplaying as core;

use crate::core::Screens;
use crate::errors::NzError;
use crate::lists::{ListHandle, open_rows};

/// One synced line of a song's words.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    /// Milliseconds from the start of the track, as the server's `.lrc` says.
    pub time_ms: u32,
    /// Empty for an instrumental marker (`♪`): the view draws « • • • ».
    pub text: String,
}

impl From<core::LyricLine> for LyricLine {
    fn from(line: core::LyricLine) -> Self {
        LyricLine {
            time_ms: line.time_ms,
            text: line.text,
        }
    }
}

impl From<LyricLine> for core::LyricLine {
    fn from(line: LyricLine) -> Self {
        core::LyricLine {
            time_ms: line.time_ms,
            text: line.text,
        }
    }
}

/// `/api/lyrics/<id>`: the two shapes the server answers, and whether there is
/// anything at all.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct LyricsData {
    /// Empty for plain words: the view prints [LyricsData::plain] as text.
    pub synced: Vec<LyricLine>,
    pub plain: String,
    /// `archive` when the words came from the track's own `.lrc` sidecar.
    pub source: String,
    /// Nothing to show: « Paroles indisponibles pour ce titre ».
    pub empty: bool,
    /// The counted bytes of the answer (worker rule 4), for the screen's own
    /// journal line.
    pub bytes: u64,
}

impl From<core::Lyrics> for LyricsData {
    fn from(lyrics: core::Lyrics) -> Self {
        LyricsData {
            empty: lyrics.is_empty(),
            bytes: lyrics.memory_bytes() as u64,
            synced: lyrics.synced.into_iter().map(LyricLine::from).collect(),
            plain: lyrics.plain,
            source: lyrics.source,
        }
    }
}

impl From<LyricsData> for core::Lyrics {
    fn from(lyrics: LyricsData) -> Self {
        core::Lyrics {
            synced: lyrics
                .synced
                .into_iter()
                .map(core::LyricLine::from)
                .collect(),
            plain: lyrics.plain,
            source: lyrics.source,
        }
    }
}

/// One line the lyric card can offer.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct CardLineData {
    pub text: String,
    /// Milliseconds, for a synced answer; `None` for plain words.
    pub time_ms: Option<u32>,
    /// The index this line had in the ORIGINAL array — what the playing line is
    /// compared with, so a marker between two verses cannot shift the choice.
    pub source: u32,
}

impl From<core::card::CardLine> for CardLineData {
    fn from(line: core::card::CardLine) -> Self {
        CardLineData {
            text: line.text,
            time_ms: line.time_ms,
            source: line.source,
        }
    }
}

/// One tap's answer on the card: the selection (lyric order) and, when nothing
/// changed, why — `min` or `max`. The French sentence is the screen's.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct CardSelectionData {
    pub indices: Vec<u32>,
    pub reason: Option<String>,
}

impl From<core::card::Selection> for CardSelectionData {
    fn from(selection: core::card::Selection) -> Self {
        CardSelectionData {
            indices: selection.indices,
            reason: selection.reason,
        }
    }
}

#[uniffi::export]
impl Screens {
    // -- the words of a title -------------------------------------------------

    /// The lyrics as this device last saw them — disk only, **0 requests**.
    pub fn cached_lyrics(&self, key: String) -> Option<LyricsData> {
        self.core().cached_lyrics(&key).map(LyricsData::from)
    }

    /// `GET /api/lyrics/<id>` — ONE request on the `User` lane, kept for the next
    /// visit. A title the server has no words for is an empty answer, not an
    /// error; a dead link is `NzError::Offline` in zero sockets.
    pub async fn lyrics(&self, key: String) -> Result<LyricsData, NzError> {
        self.core()
            .lyrics(&key)
            .await
            .map(LyricsData::from)
            .map_err(NzError::from)
    }

    /// `GET /api/radio/track/<id>` — « Titres similaires », one request, windowed
    /// by the screen (the seed is in the list, as the server sends it).
    pub async fn similar_tracks(&self, key: String) -> Result<Arc<ListHandle>, NzError> {
        let rows = self
            .core()
            .similar_tracks(&key)
            .await
            .map_err(NzError::from)?;
        Ok(open_rows(self.core(), rows))
    }

    // -- the lyric card's rules -----------------------------------------------

    /// The lines a card can offer, marker lines dropped.
    pub fn card_lines(&self, lyrics: LyricsData) -> Vec<CardLineData> {
        self.core()
            .card_lines(&core::Lyrics::from(lyrics))
            .into_iter()
            .map(CardLineData::from)
            .collect()
    }

    /// Which line the card opens on: the one being heard (`active_source`, the
    /// index in the sy`nced array, `-1` for none), else the first.
    pub fn card_default_line(&self, lyrics: LyricsData, active_source: i64) -> u32 {
        self.core()
            .card_default_line(&core::Lyrics::from(lyrics), active_source)
    }

    /// One tap on `index`: the new selection, or the reason it did not change.
    pub fn card_toggle(&self, selected: Vec<u32>, index: u32) -> CardSelectionData {
        self.core().card_toggle(&selected, index).into()
    }

    /// The selected lines' text, in LYRIC order, at most three.
    pub fn card_texts(&self, lyrics: LyricsData, selected: Vec<u32>) -> Vec<String> {
        self.core()
            .card_texts(&core::Lyrics::from(lyrics), &selected)
    }

    /// `neezer-<artist>-<title>.png`, safe on every filesystem and share target.
    pub fn card_filename(&self, artist: String, title: String) -> String {
        self.core().card_filename(&artist, &title)
    }

    // -- a podcast's rate -----------------------------------------------------

    /// The rates the speed menu offers (`lib/speed.js#SPEEDS`).
    pub fn podcast_speed_choices(&self) -> Vec<f32> {
        core::SPEEDS.to_vec()
    }

    /// The rate a show is played at (1 for an episode nobody has chosen one for,
    /// and for anything that is not an episode: the screen only asks for one).
    pub fn podcast_speed(&self, show: String) -> f32 {
        self.core().podcast_speed(&show)
    }

    /// Remembers a rate for a show; 1 forgets it. Never fails the screen: a store
    /// that cannot be written keeps the current rate for this session.
    pub fn set_podcast_speed(&self, show: String, rate: f32) -> Result<(), NzError> {
        self.core()
            .set_podcast_speed(&show, rate)
            .map_err(NzError::from)
    }

    /// Every remembered rate, as `show=rate` — what the reconcile step compares
    /// against (the SPA's `podcastSpeeds` map).
    pub fn podcast_speeds(&self) -> Vec<String> {
        self.core()
            .podcast_speeds()
            .into_iter()
            .map(|(show, rate)| format!("{show}={rate}"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every record survives the boundary in both directions: a `LyricsData`
    /// handed BACK (the card's calls take one) is the same lyrics.
    #[test]
    fn the_words_round_trip_through_the_boundary() {
        let lyrics = LyricsData {
            synced: vec![
                LyricLine {
                    time_ms: 0,
                    text: "Il est cinq heures".into(),
                },
                LyricLine {
                    time_ms: 12_000,
                    text: "♪".into(),
                },
            ],
            plain: "Il est cinq heures\n♪".into(),
            source: "archive".into(),
            empty: false,
            bytes: 120,
        };
        let back = LyricsData::from(core::Lyrics::from(lyrics.clone()));
        assert_eq!(back.synced, lyrics.synced);
        assert_eq!(back.plain, lyrics.plain);
        assert_eq!(back.source, lyrics.source);
        assert!(!back.empty);
    }

    /// An empty answer says so, and its own `bytes` is what the record holds (not
    /// a process RSS: worker rule 4).
    #[test]
    fn an_empty_answer_is_flagged_and_counted() {
        let data = LyricsData::from(core::Lyrics::default());
        assert!(data.empty);
        assert!(data.bytes > 0, "the record itself is counted bytes");
    }

    /// The card's rules cross unchanged: the marker between two verses keeps its
    /// index, and the tap at the limit answers its reason.
    #[test]
    fn the_card_rules_survive_the_crossing() {
        let core_lyrics = core::Lyrics {
            synced: vec![
                core::LyricLine {
                    time_ms: 0,
                    text: "Premier".into(),
                },
                core::LyricLine {
                    time_ms: 1_000,
                    text: "♪".into(),
                },
                core::LyricLine {
                    time_ms: 2_000,
                    text: "Second".into(),
                },
            ],
            plain: String::new(),
            source: "archive".into(),
        };
        let data = LyricsData::from(core_lyrics.clone());
        let lines = core::card::lines(&core::Lyrics::from(data.clone()));
        assert_eq!(lines.len(), 2);
        assert_eq!(CardLineData::from(lines[1].clone()).source, 2);

        let selection = CardSelectionData::from(core::card::toggle(&[0, 1, 2], 3));
        assert_eq!(selection.indices, vec![0, 1, 2]);
        assert_eq!(selection.reason.as_deref(), Some("max"));
    }

    /// The filename rule and the speed choices are the SPA's.
    #[test]
    fn the_names_and_the_rates_are_the_spas() {
        assert_eq!(
            core::card::filename("Beyoncé", "Déjà Vu"),
            "neezer-beyonce-deja-vu.png"
        );
        assert_eq!(
            core::SPEEDS.to_vec(),
            vec![0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0]
        );
    }
}
