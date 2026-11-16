//! Discovery: the Flow, the radios' answers, the discovery/energy settings, the
//! listening history, the year in review and the mood mixes.

use serde::{Deserialize, Serialize};

use crate::album::Album;
use crate::artist::Artist;
use crate::track::Track;

/// One genre a Flow can be set to (`/api/flow/styles`).
///
/// `tracks` counts the titles whose OWN style is this node, `branch` every title
/// under it, so the picker can show a family with the size of all its children.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FlowStyle {
    #[serde(default)]
    pub name: String,
    /// The node's first parent in the genre tree, `null` at a root.
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub tracks: u32,
    #[serde(default)]
    pub branch: u32,
}

/// `/api/flow/styles`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FlowStyles {
    #[serde(default)]
    pub styles: Vec<FlowStyle>,
}

/// `/api/recommend?d=&seed=` — the home's "for you" rows at a discovery level.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Recommend {
    /// 0 = the closest to what the person knows, 1 = an adventure.
    #[serde(default)]
    pub level: f64,
    /// The server's own French name for that notch (`"Voisin"`, …).
    #[serde(default)]
    pub step: String,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub albums: Vec<Album>,
    #[serde(default)]
    pub artists: Vec<Artist>,
}

/// One notch of the discovery slider.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiscoverStep {
    #[serde(default)]
    pub name: String,
    /// The level this notch starts at.
    #[serde(default, rename = "from")]
    pub from_level: f64,
}

/// One notch of the Flow's starting energy.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EnergyStep {
    #[serde(default)]
    pub name: String,
    /// `null` for "Libre" (no aim at all).
    #[serde(default)]
    pub value: Option<f64>,
}

/// `/api/discover/prefs` — the two sliders, with the notches to draw.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiscoverPrefs {
    #[serde(default)]
    pub level: f64,
    #[serde(default)]
    pub step: String,
    #[serde(default)]
    pub steps: Vec<DiscoverStep>,
    /// The Flow's starting energy, `null` for "Libre".
    #[serde(default)]
    pub energy: Option<f64>,
    #[serde(default)]
    pub energy_step: String,
    #[serde(default)]
    pub energy_steps: Vec<EnergyStep>,
}

/// One entry of the listening history (`neezer/listening.py`'s rows).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryEvent {
    #[serde(default)]
    pub id: i64,
    /// ISO date-time, server local time.
    #[serde(default)]
    pub at: Option<String>,
    /// Seconds actually played.
    #[serde(default)]
    pub listened: u32,
    /// `1` a listen, `-1` a skip, `0` neither.
    #[serde(default)]
    pub outcome: i32,
    /// Where the play came from, when the client said.
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub track: Track,
}

/// One artist in the 30-day summary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryArtist {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub count: u32,
}

/// One title in the 30-day summary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryTrack {
    #[serde(default)]
    pub track: Track,
    #[serde(default)]
    pub count: u32,
}

/// The last 30 days, in figures.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HistorySummary {
    /// The window the summary covers.
    #[serde(default)]
    pub days: u32,
    #[serde(default)]
    pub listens: u32,
    #[serde(default)]
    pub skips: u32,
    /// Minutes actually spent, skips included.
    #[serde(default)]
    pub minutes: u32,
    #[serde(default)]
    pub top_artists: Vec<HistoryArtist>,
    #[serde(default)]
    pub top_tracks: Vec<HistoryTrack>,
}

/// The keyset cursor of the next history page: pass both halves back as
/// `before` / `before_id` (one without the other is refused).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryCursor {
    #[serde(default)]
    pub before: String,
    #[serde(default)]
    pub before_id: i64,
}

/// `/api/history?before=&before_id=&limit=&outcome=&days=`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct History {
    #[serde(default)]
    pub events: Vec<HistoryEvent>,
    #[serde(default)]
    pub next: Option<HistoryCursor>,
    #[serde(default)]
    pub summary: HistorySummary,
}

/// One year in `/api/recap/years`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecapYearRow {
    #[serde(default)]
    pub year: i32,
    /// Events that year (listens plus skips).
    #[serde(default)]
    pub events: u32,
    #[serde(default)]
    pub listens: u32,
    #[serde(default)]
    pub minutes: u32,
    /// Enough history for the year to tell a story (the `enough` of its page).
    #[serde(default)]
    pub enough: bool,
    /// The running year (the numbers grow until 31 December).
    #[serde(default)]
    pub partial: bool,
}

/// `/api/recap/years` — the years that hold at least one event, newest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecapYears {
    #[serde(default)]
    pub years: Vec<RecapYearRow>,
    /// `null` when there is no history at all.
    #[serde(default)]
    pub latest: Option<i32>,
    #[serde(default)]
    pub now_year: i32,
    /// The earliest year the screen offers to open.
    #[serde(default)]
    pub min_year: i32,
    /// `"server-local"`: the moments are the server's local time.
    #[serde(default)]
    pub timezone: String,
}

/// The period a year covers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecapPeriod {
    /// `YYYY-MM-DD`.
    #[serde(default, rename = "from")]
    pub from_date: String,
    #[serde(default)]
    pub to: String,
}

/// The year's figures.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecapTotals {
    #[serde(default)]
    pub events: u32,
    #[serde(default)]
    pub listens: u32,
    #[serde(default)]
    pub skips: u32,
    #[serde(default)]
    pub neutrals: u32,
    #[serde(default)]
    pub seconds: u64,
    #[serde(default)]
    pub minutes: u32,
    #[serde(default)]
    pub days_active: u32,
    #[serde(default)]
    pub first_at: Option<String>,
    #[serde(default)]
    pub last_at: Option<String>,
}

/// One month of the year, always all twelve.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecapMonth {
    /// 1..12.
    #[serde(default)]
    pub month: u32,
    #[serde(default)]
    pub listens: u32,
    #[serde(default)]
    pub skips: u32,
    #[serde(default)]
    pub seconds: u64,
}

/// The longest run of consecutive listening days.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecapStreak {
    #[serde(default)]
    pub days: u32,
    /// `YYYY-MM-DD`.
    #[serde(default, rename = "from")]
    pub from_date: String,
    #[serde(default)]
    pub to: String,
}

/// The day with the most listens.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecapDay {
    /// `YYYY-MM-DD`.
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub listens: u32,
    #[serde(default)]
    pub seconds: u64,
}

/// The first or the last title heard in the year.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecapEdge {
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default)]
    pub track: Track,
}

/// One title of the year's top.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecapTrackRow {
    #[serde(default)]
    pub track: Track,
    #[serde(default)]
    pub listens: u32,
    #[serde(default)]
    pub seconds: u64,
}

/// One artist of the year's top.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecapArtistRow {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub listens: u32,
    /// How many distinct titles of theirs were played.
    #[serde(default)]
    pub tracks: u32,
}

/// One album of the year's top.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecapAlbumRow {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub listens: u32,
}

/// One style of the year's styles card.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecapGenreRow {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub listens: u32,
    /// The share of the MEASURED listens, 0..1.
    #[serde(default)]
    pub share: f64,
}

/// The styles card: how much of the year the measurement could explain.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecapGenres {
    #[serde(default)]
    pub available: bool,
    /// Measured listens / all listens, 0..1.
    #[serde(default)]
    pub coverage: f64,
    /// How many distinct styles were heard.
    #[serde(default)]
    pub explored: u32,
    #[serde(default)]
    pub measured_listens: u32,
    #[serde(default)]
    pub rows: Vec<RecapGenreRow>,
}

/// The four tops of the year.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecapTops {
    #[serde(default)]
    pub tracks: Vec<RecapTrackRow>,
    #[serde(default)]
    pub artists: Vec<RecapArtistRow>,
    #[serde(default)]
    pub albums: Vec<RecapAlbumRow>,
    #[serde(default)]
    pub genres: RecapGenres,
}

/// `/api/recap/<year>` — one year in review, bounded and cached server-side.
///
/// `cards` is the ORDER the screen shows the stories in; `enough` is `false` for
/// a year too thin to tell a story about (the screen then says so instead).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Recap {
    #[serde(default)]
    pub year: i32,
    #[serde(default)]
    pub partial: bool,
    #[serde(default)]
    pub enough: bool,
    #[serde(default)]
    pub timezone: String,
    #[serde(default)]
    pub period: RecapPeriod,
    #[serde(default)]
    pub totals: RecapTotals,
    #[serde(default)]
    pub months: Vec<RecapMonth>,
    #[serde(default)]
    pub streak: Option<RecapStreak>,
    #[serde(default)]
    pub biggest_day: Option<RecapDay>,
    #[serde(default)]
    pub tops: RecapTops,
    #[serde(default)]
    pub first: Option<RecapEdge>,
    #[serde(default)]
    pub last: Option<RecapEdge>,
    #[serde(default)]
    pub cards: Vec<String>,
    /// Unix seconds the payload was aggregated.
    #[serde(default)]
    pub generated: i64,
}

/// One mood's card in `/api/moods`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MoodCard {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub subtitle: String,
    /// How many titles match — a COUNT here, not tracks (see [`Mood`]).
    #[serde(default)]
    pub tracks: u32,
    /// Enough titles to offer the card at all.
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub cover: Option<String>,
}

/// `/api/moods` — the six moods.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Moods {
    #[serde(default)]
    pub moods: Vec<MoodCard>,
}

/// `/api/moods/<id>` — one mix.
///
/// `tracks` is a LIST here and a COUNT in [`MoodCard`]: the fixture proves both,
/// and a model that shared one type would silently read 0 titles every time.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Mood {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub ready: bool,
    /// `"not-enough"` or `"no-tracks"` when the mix is empty, `null` otherwise.
    #[serde(default)]
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mood_card_counts_its_titles_and_a_mood_lists_them() {
        let moods: Moods = serde_json::from_str(
            r#"{"moods":[{"cover":null,"id":"calme","name":"Calme",
                "ready":false,"subtitle":"Souffle lent","tracks":0}]}"#,
        )
        .expect("moods");
        assert_eq!(moods.moods[0].tracks, 0);

        let mood: Mood = serde_json::from_str(
            r#"{"id":"energie","name":"Énergie","ready":false,"reason":"not-enough",
                "subtitle":"Le tempo qui pousse","tracks":[]}"#,
        )
        .expect("mood");
        assert!(mood.tracks.is_empty());
        assert_eq!(mood.reason.as_deref(), Some("not-enough"));
    }

    #[test]
    fn an_empty_recap_year_is_a_full_twelve_month_shape() {
        // The empty year still carries all twelve months: a screen that assumed
        // a sparse list would draw a broken chart.
        let recap: Recap = serde_json::from_str(
            r#"{"year":2025,"partial":false,"enough":false,"timezone":"server-local",
                "period":{"from":"2025-01-01","to":"2025-12-31"},
                "totals":{"events":0,"listens":0,"skips":0,"neutrals":0,"seconds":0,
                          "minutes":0,"days_active":0,"first_at":null,"last_at":null},
                "months":[{"month":1,"listens":0,"skips":0,"seconds":0}],
                "streak":null,"biggest_day":null,
                "tops":{"tracks":[],"artists":[],"albums":[],
                        "genres":{"available":false,"coverage":0.0,"explored":0,
                                  "measured_listens":0,"rows":[]}},
                "first":null,"last":null,"cards":[],"generated":1794835151}"#,
        )
        .expect("empty recap");
        assert_eq!(recap.months.len(), 1);
        assert_eq!(recap.period.from_date, "2025-01-01");
    }
}
