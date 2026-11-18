//! The screen-shaped records of this area, and the ONE converter from the
//! server's JSON to them.
//!
//! The FFI rule (`docs/agents/mobile.md`, *The FFI boundary*) is that a record
//! crossing to Kotlin carries what a screen DRAWS and nothing else, and that a
//! long list never crosses whole. So the conversions live here, `mod.rs` keeps
//! the rules and the requests, and `nz-ffi/src/podcasts.rs` only renames fields.
//!
//! Two decisions worth knowing before reading a field:
//!
//! - **A marker's position is MILLISECONDS and a chapter's `start` is SECONDS.**
//!   That is the server's own split (`webui`'s `podcast.py` and
//!   `podcast_chapters.py`), and getting it wrong puts a marker of 10:00 at ten
//!   seconds. [`Marker::seconds`] is the only conversion, and it exists so a
//!   screen never does the arithmetic itself.
//! - **A resume position is not the server's**: it is the player's local mirror
//!   (`playerProgress`), merged with `/api/podcast/progress` at the session's
//!   start. An episode carries `position: None` when there is nothing to resume,
//!   which is what makes « Reprendre · il reste 12 min » appear only when it is
//!   true.

use serde::{Deserialize, Serialize};

use nz_model::{
    Chapter, EpisodeTranscript, EpisodeTranscriptInfo, Marker as WireMarker, PodcastCard,
    PodcastShow, TranscriptHit as WireHit,
};

use super::text;
use crate::lists::Row;

/// What a show page says about one episode's transcript, without the segments.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscriptInfo {
    /// `none`, `queued`, `leased`, `running`, `ready`, `error`, `stopped`.
    pub status: String,
    /// 0..1.
    pub progress: f64,
    /// The language it was transcribed in, `None` when unknown.
    pub language: Option<String>,
    /// `server` or `client`.
    pub source: Option<String>,
    pub segment_count: u32,
    /// The failure's machine code (`too_long`, `queue_full`, …), never a message
    /// only an administrator may read.
    pub error_code: Option<String>,
}

impl TranscriptInfo {
    /// Whether the screen draws a progress ring for it (`isActiveStatus`).
    pub fn active(&self) -> bool {
        text::is_active_status(&self.status)
    }

    /// Whether the episode has a transcript to open.
    pub fn ready(&self) -> bool {
        self.status == "ready"
    }

    /// The percentage the chip writes, as the SPA rounds it.
    pub fn percent(&self) -> u32 {
        (self.progress.clamp(0.0, 1.0) * 100.0).round() as u32
    }
}

impl From<&EpisodeTranscriptInfo> for TranscriptInfo {
    fn from(info: &EpisodeTranscriptInfo) -> Self {
        TranscriptInfo {
            status: info.status.clone(),
            progress: info.progress.clamp(0.0, 1.0),
            language: info.language.clone(),
            source: info.source.clone(),
            segment_count: info.segment_count,
            error_code: info.error.as_ref().map(|error| error.code.clone()),
        }
    }
}

/// Where an episode stands, as the show page's resume bar draws it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Position {
    /// Seconds into the episode.
    pub seconds: u32,
    pub duration_s: u32,
    /// Heard to the end: never offer to resume.
    pub finished: bool,
    /// When it was written (Unix milliseconds) — the merge's "newest wins".
    pub at_ms: u64,
    /// The write is still in `pending_progress`: the link was dead when it
    /// happened, so the server has not seen it yet.
    pub pending: bool,
}

impl Position {
    /// The fraction played, clamped (`resumeOf`'s `pct`).
    pub fn fraction(&self) -> f64 {
        if self.duration_s == 0 {
            return 0.0;
        }
        (f64::from(self.seconds) / f64::from(self.duration_s)).clamp(0.0, 1.0)
    }

    /// The seconds left (`resumeOf`'s `left`).
    pub fn remaining_s(&self) -> u32 {
        self.duration_s.saturating_sub(self.seconds)
    }
}

/// A personal marker inside one episode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub id: String,
    pub episode_id: String,
    /// MILLISECONDS, the server's unit for a marker.
    pub position_ms: u32,
    pub label: String,
}

impl Marker {
    /// The same moment in seconds — what a seek takes.
    pub fn seconds(&self) -> u32 {
        self.position_ms / 1_000
    }

    /// The moment as `12:05`, so a chip reads like the SPA's.
    pub fn time_label(&self) -> String {
        text::duration_label(self.seconds())
    }
}

impl From<&WireMarker> for Marker {
    fn from(marker: &WireMarker) -> Self {
        Marker {
            id: marker.id.clone(),
            episode_id: marker.episode_id.to_string(),
            position_ms: marker.position,
            label: marker.label.clone(),
        }
    }
}

/// One show, as a grid card and as a page's header draw it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShowCard {
    pub id: String,
    /// The provider's show id — what a catalogue card carries and a page accepts.
    pub deezer_id: Option<String>,
    pub title: String,
    pub description: String,
    /// A server path (`/api/cover/<id>`) or `None` for the Sillon grooves.
    pub cover: Option<String>,
    /// Still followed: an unsubscribed show with audio keeps its episodes.
    pub subscribed: bool,
    /// The provider dropped it: everything archived stays playable.
    pub local: bool,
    /// `ok` or `error` (the provider's last verdict).
    pub status: String,
    pub episode_count: u32,
    pub archived_count: u32,
    /// Archive new episodes of this show as they appear.
    pub auto_archive: bool,
    pub auto_transcribe: bool,
    /// `auto` or a language code.
    pub transcribe_language: String,
    /// How many episodes have a transcript (`/api/podcast/<id>` only).
    pub transcribed_count: Option<u32>,
}

impl From<&PodcastShow> for ShowCard {
    fn from(show: &PodcastShow) -> Self {
        ShowCard {
            id: show.id.to_string(),
            deezer_id: show.deezer_id.clone().filter(|id| !id.trim().is_empty()),
            title: show.title.clone(),
            description: show.description.clone().unwrap_or_default(),
            cover: show.cover.clone(),
            subscribed: show.subscribed,
            local: show.local,
            status: show
                .status
                .clone()
                .filter(|status| !status.is_empty())
                .unwrap_or_else(|| "ok".to_owned()),
            episode_count: show.episode_count,
            archived_count: show.archived_count,
            auto_archive: show.auto_archive,
            auto_transcribe: show.auto_transcribe,
            transcribe_language: show
                .transcribe_language
                .clone()
                .filter(|language| !language.is_empty())
                .unwrap_or_else(|| "auto".to_owned()),
            transcribed_count: show.transcribed_count,
        }
    }
}

impl ShowCard {
    /// A catalogue search result, as `Podcasts.svelte#PodcastCard` draws it: no
    /// row yet, a provider id, and the sentence the server wrote.
    pub fn from_card(card: &PodcastCard) -> Self {
        ShowCard {
            id: String::new(),
            deezer_id: Some(card.deezer_id.clone()).filter(|id| !id.is_empty()),
            title: card.title.clone(),
            description: card.description.clone(),
            cover: card.cover.clone(),
            subscribed: false,
            local: false,
            status: "ok".to_owned(),
            episode_count: 0,
            archived_count: 0,
            auto_archive: false,
            auto_transcribe: false,
            transcribe_language: "auto".to_owned(),
            transcribed_count: None,
        }
    }

    /// The URL `POST /api/podcasts` takes for this card (`actions.js#subscribe`
    /// wraps a provider id exactly this way).
    pub fn subscribe_url(&self) -> Option<String> {
        self.deezer_id
            .as_ref()
            .map(|id| format!("https://www.deezer.com/show/{id}"))
    }

    /// « 12 sur 40 épisodes archivés » — the line under the archive switch.
    pub fn archive_line(&self) -> String {
        format!(
            "{} sur {} épisode{} archivé{} sur le serveur",
            group_digits(self.archived_count.min(self.episode_count)),
            group_digits(self.episode_count),
            if self.episode_count > 1 { "s" } else { "" },
            if self.archived_count > 1 { "s" } else { "" },
        )
    }

    /// Counted bytes of this record, for the lot's own measure (worker rule 4).
    pub fn memory_bytes(&self) -> u64 {
        std::mem::size_of::<ShowCard>() as u64
            + (self.id.len()
                + self.deezer_id.as_deref().map_or(0, str::len)
                + self.title.len()
                + self.description.len()
                + self.cover.as_deref().map_or(0, str::len)
                + self.status.len()
                + self.transcribe_language.len()) as u64
    }
}

/// `2 000`: the NARROW NO-BREAK SPACE French typography puts between digit
/// groups (the SPA's `toLocaleString("fr-FR")`). A plain space would let a
/// line break inside a number, which is why it is U+202F and not `' '`.
pub fn group_digits(count: u32) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push('\u{202F}');
        }
        out.push(character);
    }
    out
}

/// One episode, as a row of the show page.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EpisodeCard {
    /// The episode's row id (a UUID) — the key a list and the player use.
    pub id: String,
    /// What `/api/stream/<id>` takes; for an episode it is the same UUID.
    pub deezer_id: String,
    pub title: String,
    pub description: String,
    pub duration_s: u32,
    /// Unix seconds, 0 when the feed had no date.
    pub published: i64,
    /// The audio is on the server, so it plays offline and can be transcribed.
    pub archived: bool,
    /// The provider cannot serve it and it is not archived: shown, not playable.
    pub unavailable: bool,
    pub explicit: bool,
    pub chapter_count: u32,
    /// The server's own state for the archived file (`completed`, …).
    pub status: String,
    pub cover: Option<String>,
    pub channel_id: String,
    /// The show's name, which is the episode row's "artist".
    pub channel_title: String,
    /// `None` when this episode has no transcript at all.
    pub transcript: Option<TranscriptInfo>,
    /// `None` when there is nothing to resume.
    pub position: Option<Position>,
    /// This person's markers in this episode, in position order.
    pub markers: Vec<Marker>,
}

impl EpisodeCard {
    /// The row a windowed list draws, and the reference a tap queues: an episode
    /// IS a track to the queue (`Player.svelte`'s `playQueue(episodes, …)`), with
    /// the show as its album and the `podcast` source chip.
    pub fn to_row(&self, position: u32) -> Row {
        Row {
            key: self.id.clone(),
            id: self.deezer_id.clone(),
            title: self.title.clone(),
            artists: self.channel_title.clone(),
            album: self.channel_title.clone(),
            album_id: (!self.channel_id.is_empty()).then(|| self.channel_id.clone()),
            duration_s: self.duration_s,
            cover: self.cover.clone(),
            explicit: self.explicit,
            liked: false,
            disliked: false,
            unavailable: self.unavailable,
            downloaded: self.archived,
            source: "podcast".to_owned(),
            position,
            played_at: 0,
            added_at: 0,
        }
    }

    /// The published date, as the show page writes it.
    pub fn published_label(&self) -> String {
        text::date_label(self.published)
    }

    /// The length, as the row writes it.
    pub fn duration_label(&self) -> String {
        text::duration_label(self.duration_s)
    }

    /// `chapter_count_label`, for the chip.
    pub fn chapters_label(&self) -> String {
        text::chapter_count_label(self.chapter_count)
    }

    /// The `podcastAbout` verdict: the episode panel has something to show.
    pub fn about(&self) -> bool {
        text::podcast_about(&self.description, self.chapter_count, self.archived)
    }

    /// Counted bytes, for the lot's own measure (worker rule 4).
    pub fn memory_bytes(&self) -> u64 {
        std::mem::size_of::<EpisodeCard>() as u64
            + (self.id.len()
                + self.deezer_id.len()
                + self.title.len()
                + self.description.len()
                + self.status.len()
                + self.cover.as_deref().map_or(0, str::len)
                + self.channel_id.len()
                + self.channel_title.len()) as u64
            + self
                .markers
                .iter()
                .map(|marker| (marker.id.len() + marker.label.len()) as u64 + 40)
                .sum::<u64>()
    }
}

/// One page of a show: the header, the episodes loaded so far, and the cursor of
/// the next page (`PodcastPage`'s `next`, opaque).
///
/// The loaded episodes ARE the page: opening a show reads 50 (`EPISODE_PAGE`),
/// scrolling to the end reads the next cursor, and a 2 000-episode show is 40
/// requests of 50 rows — the SPA's own paging, which never asks for the whole
/// archive at once (rule 3: nothing fans out unbounded).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShowPage {
    pub show: ShowCard,
    /// In the server's order (newest first).
    pub episodes: Vec<EpisodeCard>,
    /// The keyset cursor of the next page, `None` when this was the last one.
    pub next: Option<String>,
}

impl ShowPage {
    /// The rows a `ListHandle` windows over, in the page's own order.
    pub fn rows(&self) -> Vec<Row> {
        self.episodes
            .iter()
            .enumerate()
            .map(|(position, episode)| episode.to_row(position as u32))
            .collect()
    }

    /// The episode with `id`, when it is loaded.
    pub fn episode(&self, id: &str) -> Option<&EpisodeCard> {
        self.episodes.iter().find(|episode| episode.id == id)
    }

    /// Counted bytes of everything this page holds (worker rule 4).
    pub fn memory_bytes(&self) -> u64 {
        self.show.memory_bytes()
            + self
                .next
                .as_deref()
                .map_or(0, |next| next.len() as u64 + 24)
            + self
                .episodes
                .iter()
                .map(EpisodeCard::memory_bytes)
                .sum::<u64>()
    }
}

/// One chapter, in the SPA's units (seconds), with the label it draws.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterCard {
    pub index: u32,
    pub start: u32,
    /// `None` when the server sent no end, or one before the start.
    pub end: Option<u32>,
    pub title: String,
}

impl From<&Chapter> for ChapterCard {
    fn from(chapter: &Chapter) -> Self {
        ChapterCard {
            index: chapter.index,
            start: chapter.start.max(0.0).round() as u32,
            end: (chapter.end.is_finite() && chapter.end >= chapter.start.max(0.0))
                .then(|| chapter.end.round().max(0.0) as u32),
            title: chapter.title.trim().to_owned(),
        }
    }
}

/// `GET /api/podcast/episode/<id>/chapters`, as the chapter list and the ticks
/// read it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Chapters {
    pub episode_id: String,
    pub chapters: Vec<ChapterCard>,
    /// `id3`, `description`, … — where they came from.
    pub source: Option<String>,
    /// The audio is on disk: chapters are only read from an archived file.
    pub archived: bool,
    /// The description was already scanned.
    pub scanned: bool,
    /// The episode's length in seconds (the last chapter's end fallback).
    pub duration_s: u32,
}

impl Chapters {
    /// `chapterAt`, for the card above the list.
    pub fn at(&self, seconds: f64) -> Option<(usize, &ChapterCard)> {
        let normalized = self.text_cards();
        // `chapter_at` needs the same normalisation the server's rows already
        // have (sorted by start), so the slice is searched directly.
        if normalized.is_empty() || !seconds.is_finite() {
            return None;
        }
        let at = text::chapter_at(&normalized, seconds)?;
        self.chapters.get(at.0).map(|chapter| (at.0, chapter))
    }

    /// The chapter after `seconds`, for « dans … ».
    pub fn next(&self, seconds: f64) -> Option<(usize, &ChapterCard)> {
        let normalized = self.text_cards();
        let at = text::next_chapter(&normalized, seconds)?;
        self.chapters.get(at.0).map(|chapter| (at.0, chapter))
    }

    /// « Chapitre 3 · Le nom » for the chapter at `index`, from the whole list
    /// (`chapterLabel` needs the position, not just the row).
    pub fn label(&self, index: usize) -> String {
        text::chapter_label(&self.text_cards(), index)
    }

    /// « 3 chapitres ».
    pub fn count_label(&self) -> String {
        text::chapter_count_label(self.chapters.len() as u32)
    }

    /// The same list in the pure functions' own shape.
    fn text_cards(&self) -> Vec<text::ChapterCard> {
        self.chapters
            .iter()
            .map(|chapter| text::ChapterCard {
                index: chapter.index,
                start: chapter.start,
                end: chapter.end,
                title: chapter.title.clone(),
            })
            .collect()
    }

    /// Where chapter `index` ends, with the episode's length as the fallback.
    pub fn end_of(&self, index: usize) -> u32 {
        let Some(chapter) = self.chapters.get(index) else {
            return 0;
        };
        if let Some(end) = chapter.end {
            return end;
        }
        if let Some(after) = self.chapters.get(index + 1) {
            return after.start;
        }
        self.duration_s.max(chapter.start)
    }
}

/// One line of a transcript, in seconds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// One page of a transcript (`GET /api/podcast/episode/<id>/transcript`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TranscriptPage {
    pub episode_id: String,
    /// `none`, `queued`, `leased`, `running`, `ready`, `error`, `stopped`.
    pub status: String,
    pub archived: bool,
    pub language: Option<String>,
    pub duration_ms: u64,
    pub done_ms: u64,
    /// 0..1.
    pub progress: f64,
    /// The segments this query matches (all of them when there is no query).
    pub total: u32,
    pub offset: u32,
    pub limit: u32,
    /// The words the server searched for, `None` for a plain read.
    pub query: Option<String>,
    pub segments: Vec<Segment>,
    /// The offset of the next page, `None` on the last one.
    pub next: Option<u32>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

impl From<&EpisodeTranscript> for TranscriptPage {
    fn from(page: &EpisodeTranscript) -> Self {
        TranscriptPage {
            episode_id: page.episode_id.to_string(),
            status: page.status.clone(),
            archived: page.archived,
            language: page.language.clone(),
            duration_ms: page.duration_ms,
            done_ms: page.done_ms,
            progress: page.progress.clamp(0.0, 1.0),
            total: page.total,
            offset: page.offset,
            limit: page.limit,
            query: page.query.clone().filter(|query| !query.trim().is_empty()),
            segments: page
                .segments
                .iter()
                .map(|segment| Segment {
                    start: segment.start,
                    end: segment.end,
                    text: segment.text.clone(),
                })
                .collect(),
            next: page.next,
            error_code: page.error.as_ref().map(|error| error.code.clone()),
            error_message: page.error.as_ref().and_then(|error| error.message.clone()),
        }
    }
}

impl TranscriptPage {
    /// The starts, for a binary search on the playing line.
    pub fn starts(&self) -> Vec<f64> {
        self.segments.iter().map(|segment| segment.start).collect()
    }

    /// The line playing at `seconds`, as `segmentAt` finds it.
    pub fn segment_at(&self, seconds: f64) -> Option<usize> {
        text::segment_at(&self.starts(), seconds)
    }

    /// Whether the panel keeps polling for it.
    pub fn active(&self) -> bool {
        text::is_active_status(&self.status)
    }

    /// The percentage the progress bar writes.
    pub fn percent(&self) -> u32 {
        (self.progress.clamp(0.0, 1.0) * 100.0).round() as u32
    }

    /// `pageInfo(offset, limit, total)`, for « page 2 sur 5 ».
    pub fn pages(&self) -> text::PageInfo {
        text::page_info(self.offset, self.limit, self.total)
    }

    /// Counted bytes, for the transcript cache's cap and the lot's measure.
    pub fn memory_bytes(&self) -> u64 {
        std::mem::size_of::<TranscriptPage>() as u64
            + (self.episode_id.len()
                + self.status.len()
                + self.language.as_deref().map_or(0, str::len)
                + self.query.as_deref().map_or(0, str::len)
                + self.error_code.as_deref().map_or(0, str::len)
                + self.error_message.as_deref().map_or(0, str::len)) as u64
            + self
                .segments
                .iter()
                .map(|segment| segment.text.len() as u64 + 20)
                .sum::<u64>()
    }
}

/// One hit of a search inside a show's transcripts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TranscriptHit {
    pub episode_id: String,
    pub episode_title: String,
    pub channel_id: String,
    pub channel_title: String,
    /// Unix seconds.
    pub published: i64,
    pub start: f64,
    pub end: f64,
    /// The excerpt, with the server's own `[…]` marks around the matches.
    pub text: String,
}

impl From<&WireHit> for TranscriptHit {
    fn from(hit: &WireHit) -> Self {
        TranscriptHit {
            episode_id: hit
                .episode_id
                .as_ref()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            episode_title: hit
                .episode
                .as_ref()
                .map(|episode| episode.title.clone())
                .unwrap_or_default(),
            channel_id: hit
                .channel_id
                .as_ref()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            channel_title: hit.channel_title.clone().unwrap_or_default(),
            published: hit
                .episode
                .as_ref()
                .and_then(|episode| episode.published)
                .unwrap_or(0),
            start: hit.start,
            end: hit.end,
            text: hit.text.clone(),
        }
    }
}

/// A search inside one show's transcripts (`/podcast/<id>/transcript/search`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TranscriptSearch {
    pub q: String,
    pub hits: Vec<TranscriptHit>,
    pub total: u32,
    /// More hits exist than this answer holds.
    pub more: bool,
    /// How many episodes were scanned.
    pub episodes: u32,
    /// The scan stopped at its budget: the screen says so.
    pub truncated: bool,
}

/// The show-level free-text search on the show page, as `Show.svelte` runs it:
/// 2 characters minimum, 300 ms after the last keystroke, one request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShowSearch {
    pub query: String,
    pub hits: Vec<TranscriptHit>,
    pub total: u32,
    pub truncated: bool,
    pub episodes: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn episode(id: &str, chapters: u32) -> EpisodeCard {
        EpisodeCard {
            id: id.to_owned(),
            deezer_id: id.to_owned(),
            title: format!("Épisode {id}"),
            description: "Un épisode".to_owned(),
            duration_s: 3_600,
            published: 1_794_960_000,
            archived: true,
            chapter_count: chapters,
            channel_id: "show-1".to_owned(),
            channel_title: "Les Histoires".to_owned(),
            ..EpisodeCard::default()
        }
    }

    /// An episode becomes an ordinary row: the show names it, the source says
    /// `podcast`, and the key is the episode's UUID (what the player queues).
    #[test]
    fn an_episode_is_a_track_row_to_the_queue() {
        let row = episode("abc", 2).to_row(7);
        assert_eq!(row.key, "abc");
        assert_eq!(row.id, "abc");
        assert_eq!(row.artists, "Les Histoires");
        assert_eq!(row.album, "Les Histoires");
        assert_eq!(row.album_id.as_deref(), Some("show-1"));
        assert_eq!(row.source, "podcast");
        assert_eq!(row.position, 7);
        assert!(row.downloaded, "an archived episode is on the server");
        assert_eq!(row.duration_s, 3_600);
    }

    /// A marker's unit is milliseconds and a chapter's is seconds: the one
    /// conversion lives here, so a chip cannot put 10:00 at ten seconds.
    #[test]
    fn a_marker_is_milliseconds_and_a_chapter_is_seconds() {
        let marker = Marker {
            id: "m1".to_owned(),
            episode_id: "abc".to_owned(),
            position_ms: 600_000,
            label: "La maison".to_owned(),
        };
        assert_eq!(marker.seconds(), 600);
        assert_eq!(marker.time_label(), "10:00");
    }

    /// The chips and lines the show page writes, from the record itself.
    #[test]
    fn a_show_states_its_counters_and_an_episode_its_labels() {
        let show = ShowCard {
            episode_count: 40,
            archived_count: 12,
            ..ShowCard::default()
        };
        assert_eq!(
            show.archive_line(),
            "12 sur 40 épisodes archivés sur le serveur"
        );
        let big = ShowCard {
            episode_count: 2_000,
            archived_count: 1_200,
            ..ShowCard::default()
        };
        assert_eq!(
            big.archive_line(),
            "1\u{202F}200 sur 2\u{202F}000 épisodes archivés sur le serveur"
        );
        let card = episode("abc", 3);
        assert_eq!(card.published_label(), "18 nov. 2026");
        assert_eq!(card.duration_label(), "1:00:00");
        assert_eq!(card.chapters_label(), "3 chapitres");
        assert!(card.about());
        let bare = EpisodeCard {
            id: "abc".to_owned(),
            title: "Épisode".to_owned(),
            archived: false,
            ..EpisodeCard::default()
        };
        assert!(!bare.about(), "nothing but the row itself");
        let counting = ShowCard {
            deezer_id: Some("9400001".to_owned()),
            ..ShowCard::default()
        };
        assert_eq!(
            counting.subscribe_url().as_deref(),
            Some("https://www.deezer.com/show/9400001")
        );
        assert!(ShowCard::default().subscribe_url().is_none());
    }

    /// The latest chapter and the one after it, from the same list.
    #[test]
    fn a_chapters_list_answers_the_two_questions_the_card_asks() {
        let chapters = Chapters {
            episode_id: "abc".to_owned(),
            chapters: vec![
                ChapterCard {
                    index: 0,
                    start: 0,
                    end: Some(60),
                    title: "Intro".to_owned(),
                },
                ChapterCard {
                    index: 1,
                    start: 60,
                    end: None,
                    title: String::new(),
                },
            ],
            archived: true,
            duration_s: 3_600,
            ..Chapters::default()
        };
        assert_eq!(chapters.at(30.0).map(|(at, _)| at), Some(0));
        assert_eq!(chapters.next(30.0).map(|(at, _)| at), Some(1));
        assert_eq!(chapters.end_of(1), 3_600);
        assert_eq!(chapters.end_of(0), 60);
    }

    /// A transcript page answers its own units: seconds for a line, a percentage
    /// for the bar, and the page a clamped subtraction gives.
    #[test]
    fn a_transcript_page_carries_its_own_times_and_page() {
        let page = TranscriptPage {
            status: "ready".to_owned(),
            progress: 0.456,
            total: 450,
            offset: 400,
            limit: 200,
            segments: vec![
                Segment {
                    start: 0.0,
                    end: 4.0,
                    text: "Un".to_owned(),
                },
                Segment {
                    start: 4.0,
                    end: 9.0,
                    text: "Deux".to_owned(),
                },
            ],
            ..TranscriptPage::default()
        };
        assert_eq!(page.segment_at(5.0), Some(1));
        assert_eq!(page.percent(), 46);
        assert_eq!(page.pages().page, 3);
        assert!(!page.active(), "a ready page is not polled");
    }
}
