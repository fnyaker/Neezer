//! Podcasts: the subscriptions, a show and its episodes, the person's positions
//! and markers, the chapters of an episode, its transcript and the server's
//! transcription controls. Owned by **MB13**.
//!
//! The note names what this API reaches (docs/agents/mobile.md, "Screen lots,
//! wave 3", **MB13**): subscriptions, search and subscribe by URL, a show by
//! UUID or Deezer id (the server accepts both), episodes paged by cursor,
//! progress, auto-archive, markers, chapters, and the transcript with its search
//! and the server's transcription controls. Every rule lives in
//! `nz_core::podcasts`; this file converts and nothing else.
//!
//! # The shape of this boundary
//!
//! - **A show page crosses as a header, a `ListHandle` and a cursor** — never as
//!   2 000 episodes: the screen windows the handle 64 rows at a time and asks
//!   the next cursor when it reaches the end (rule 4).
//! - **A row carries what the screen DRAWS** ([`EpisodeData`]): its title, its
//!   date, the archive and provider flags, the transcript's state, the resume
//!   position and this person's markers. The SPA's episode row shows all of that
//!   and nothing else.
//! - **Every read is a `cached_x()` / `x()` pair**, so a warm open paints the
//!   copy before the network and a dead link keeps it (rule 3).
//! - **Writes are optimistic**: subscribing, unsubscribing, the archive switch,
//!   a marker and the transcription switch all move the state first and answer
//!   `Err` with the cause — which the screen turns into a toast with
//!   « Réessayer ».
//! - **The units are the server's**: a marker's position is MILLISECONDS, a
//!   chapter's `start` is SECONDS, and both cross as such. `nz-core` owns the one
//!   conversion, so no screen can put a marker of 10:00 at ten seconds.

use std::sync::Arc;

use nz_core::podcasts as core_podcasts;
use nz_core::podcasts::{
    Chapters, EpisodeCard, Marker, Position, ShowCard, ShowPage, ShowSearch, TranscriptHit,
    TranscriptInfo, TranscriptPage, TranscriptSearch,
};

use crate::core::Screens;
use crate::errors::NzError;
use crate::lists::{ListHandle, TrackRow};

/// One show, as the subscription grid and the show page's header draw it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct ShowData {
    /// The server's row id (a UUID) — `/api/podcast/<id>` takes it, or the
    /// `deezer_id` below.
    pub id: String,
    /// The provider's id, `None` for a local feed.
    pub deezer_id: Option<String>,
    pub title: String,
    pub description: String,
    /// A server path (`/api/cover/<id>`) or `None` for the Sillon grooves.
    pub cover: Option<String>,
    /// Still followed. An unsubscribed show with archived audio keeps its page.
    pub subscribed: bool,
    /// The provider dropped it: everything archived stays playable.
    pub local: bool,
    /// `ok` or `error`.
    pub status: String,
    pub episode_count: u32,
    pub archived_count: u32,
    pub auto_archive: bool,
    pub auto_transcribe: bool,
    /// `auto` or a language code.
    pub transcribe_language: String,
    /// How many episodes have a transcript, when the server said.
    pub transcribed_count: Option<u32>,
    /// « 12 sur 40 épisodes archivés sur le serveur », ready to draw.
    pub archive_line: String,
    /// The URL `subscribe_podcast` takes for this card, `None` when it is already
    /// a row of this server.
    pub subscribe_url: Option<String>,
}

impl From<&ShowCard> for ShowData {
    fn from(show: &ShowCard) -> Self {
        ShowData {
            id: show.id.clone(),
            deezer_id: show.deezer_id.clone(),
            title: show.title.clone(),
            description: show.description.clone(),
            cover: show.cover.clone(),
            subscribed: show.subscribed,
            local: show.local,
            status: show.status.clone(),
            episode_count: show.episode_count,
            archived_count: show.archived_count,
            auto_archive: show.auto_archive,
            auto_transcribe: show.auto_transcribe,
            transcribe_language: show.transcribe_language.clone(),
            transcribed_count: show.transcribed_count,
            archive_line: show.archive_line(),
            subscribe_url: show.subscribe_url(),
        }
    }
}

/// What a show page says about one episode's transcript.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct TranscriptInfoData {
    /// `none`, `queued`, `leased`, `running`, `ready`, `error`, `stopped`.
    pub status: String,
    /// 0..1.
    pub progress: f64,
    /// Rounded, for the chip's « 42 % ».
    pub percent: u32,
    pub language: Option<String>,
    pub source: Option<String>,
    pub segment_count: u32,
    /// The failure's machine code (`too_long`, `queue_full`, …).
    pub error_code: Option<String>,
    pub active: bool,
    pub ready: bool,
}

impl From<&TranscriptInfo> for TranscriptInfoData {
    fn from(info: &TranscriptInfo) -> Self {
        TranscriptInfoData {
            status: info.status.clone(),
            progress: info.progress,
            percent: info.percent(),
            language: info.language.clone(),
            source: info.source.clone(),
            segment_count: info.segment_count,
            error_code: info.error_code.clone(),
            active: info.active(),
            ready: info.ready(),
        }
    }
}

/// Where an episode stands, for the resume bar.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct EpisodePositionData {
    pub seconds: u32,
    pub duration_s: u32,
    /// Heard to the end: « Terminé », never a resume into the credits.
    pub finished: bool,
    /// 0..1, the bar's width.
    pub fraction: f64,
    /// The seconds left, for « il reste 12 min ».
    pub remaining_s: u32,
    /// The write is still in the queue: the link was down when it happened.
    pub pending: bool,
}

impl From<&Position> for EpisodePositionData {
    fn from(position: &Position) -> Self {
        EpisodePositionData {
            seconds: position.seconds,
            duration_s: position.duration_s,
            finished: position.finished,
            fraction: position.fraction(),
            remaining_s: position.remaining_s(),
            pending: position.pending,
        }
    }
}

/// One episode's position in a map, for `/api/podcast/progress`'s merge.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct EpisodePositionEntry {
    pub episode_id: String,
    pub position: EpisodePositionData,
}

/// A personal marker inside one episode.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct EpisodeMarkerData {
    pub id: String,
    pub episode_id: String,
    /// MILLISECONDS, the server's unit for a marker.
    pub position_ms: u32,
    /// The same moment in seconds, which is what a seek takes.
    pub seconds: u32,
    /// `12:05`, ready for the chip.
    pub time_label: String,
    pub label: String,
}

impl From<&Marker> for EpisodeMarkerData {
    fn from(marker: &Marker) -> Self {
        EpisodeMarkerData {
            id: marker.id.clone(),
            episode_id: marker.episode_id.clone(),
            position_ms: marker.position_ms,
            seconds: marker.seconds(),
            time_label: marker.time_label(),
            label: marker.label.clone(),
        }
    }
}

/// One episode, as the show page's row draws it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct EpisodeData {
    /// The row's id (a UUID): the list's key and what the player queues.
    pub id: String,
    /// What `/api/stream/<id>` takes — the same UUID for an episode.
    pub deezer_id: String,
    pub title: String,
    pub description: String,
    pub duration_s: u32,
    /// The `h:mm:ss` label the row writes.
    pub duration_label: String,
    /// Unix seconds, 0 when the feed had no date.
    pub published: i64,
    /// `18 nov. 2026`.
    pub published_label: String,
    /// The audio is on the server: it plays offline and can be transcribed.
    pub archived: bool,
    /// The provider cannot serve it and it is not archived.
    pub unavailable: bool,
    pub explicit: bool,
    pub chapter_count: u32,
    /// « 3 chapitres ».
    pub chapters_label: String,
    /// The server's own state for the archived file (`completed`, …).
    pub status: String,
    pub cover: Option<String>,
    pub channel_id: String,
    /// The show's name, which is the row's artist and album.
    pub channel_title: String,
    pub transcript: Option<TranscriptInfoData>,
    /// `None` when there is nothing to resume.
    pub position: Option<EpisodePositionData>,
    /// This person's markers in this episode, in position order.
    pub markers: Vec<EpisodeMarkerData>,
    /// The episode panel has something beyond its line (a description,
    /// chapters or an archive).
    pub about: bool,
    /// The row a windowed list draws, so a tap queues exactly what a row means.
    pub row: TrackRow,
}

impl From<&EpisodeCard> for EpisodeData {
    fn from(episode: &EpisodeCard) -> Self {
        EpisodeData {
            id: episode.id.clone(),
            deezer_id: episode.deezer_id.clone(),
            title: episode.title.clone(),
            description: episode.description.clone(),
            duration_s: episode.duration_s,
            duration_label: episode.duration_label(),
            published: episode.published,
            published_label: episode.published_label(),
            archived: episode.archived,
            unavailable: episode.unavailable,
            explicit: episode.explicit,
            chapter_count: episode.chapter_count,
            chapters_label: episode.chapters_label(),
            status: episode.status.clone(),
            cover: episode.cover.clone(),
            channel_id: episode.channel_id.clone(),
            channel_title: episode.channel_title.clone(),
            transcript: episode.transcript.as_ref().map(TranscriptInfoData::from),
            position: episode.position.as_ref().map(EpisodePositionData::from),
            markers: episode.markers.iter().map(EpisodeMarkerData::from).collect(),
            about: episode.about(),
            row: TrackRow::from(episode.to_row(0)),
        }
    }
}

/// One page of a show: the header, the episodes loaded so far, and the cursor of
/// the next page.
#[derive(uniffi::Record, Clone)]
pub struct ShowPageData {
    pub show: ShowData,
    /// A windowed handle over the loaded episodes (50 a page, 2 000 in a show).
    pub episodes: Arc<ListHandle>,
    /// The opaque cursor of the next page, `None` on the last one. Pass it back
    /// to [`Screens::podcast_show_more`] — never build one.
    pub next: Option<String>,
    /// How many episodes are loaded right now (the handle's length).
    pub loaded: u32,
}

/// One chapter, in the SPA's units (seconds).
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct ChapterData {
    pub index: u32,
    pub start: u32,
    /// `None` when the feed declared no end: the chapter runs to the next one.
    pub end: Option<u32>,
    pub title: String,
    /// « Chapitre 3 · Le nom », ready for the row and the sheet.
    pub label: String,
}

/// `/api/podcast/episode/<id>/chapters`, as the list, the sheet and the ticks
/// read it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct ChaptersData {
    pub episode_id: String,
    /// Sorted by start, one per start, whole seconds.
    pub chapters: Vec<ChapterData>,
    /// `id3`, `description`, … — where they came from.
    pub source: Option<String>,
    /// The audio is on the server: chapters are only read from an archived file.
    pub archived: bool,
    /// The description was already scanned.
    pub scanned: bool,
    /// The episode's length: the last chapter's own end.
    pub duration_s: u32,
}

impl From<&Chapters> for ChaptersData {
    fn from(chapters: &Chapters) -> Self {
        ChaptersData {
            episode_id: chapters.episode_id.clone(),
            chapters: chapters
                .chapters
                .iter()
                .enumerate()
                .map(|(at, chapter)| ChapterData {
                    index: chapter.index,
                    start: chapter.start,
                    end: chapter.end,
                    title: chapter.title.clone(),
                    // « Chapitre 3 · Le nom » needs the position in the list, so
                    // the core words it and no screen can differ.
                    label: chapters.label(at),
                })
                .collect(),
            source: chapters.source.clone(),
            archived: chapters.archived,
            scanned: chapters.scanned,
            duration_s: chapters.duration_s,
        }
    }
}

/// One line of a transcript, in seconds.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct TranscriptSegmentData {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// One page of a transcript.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct TranscriptPageData {
    pub episode_id: String,
    /// `none`, `queued`, `leased`, `running`, `ready`, `error`, `stopped`.
    pub status: String,
    pub archived: bool,
    pub language: Option<String>,
    pub duration_ms: u64,
    pub done_ms: u64,
    /// 0..1.
    pub progress: f64,
    pub percent: u32,
    /// Every line this query matches (all of them without a query).
    pub total: u32,
    pub offset: u32,
    pub limit: u32,
    /// The words searched for, `None` for a plain read.
    pub query: Option<String>,
    pub segments: Vec<TranscriptSegmentData>,
    /// The next page's offset, `None` on the last one.
    pub next: Option<u32>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    /// The lines came from the device's copy: the screen says « version
    /// mémorisée » and keeps them.
    pub cached: bool,
    pub active: bool,
    /// « page 2 sur 5 », as `pageInfo` computes it.
    pub page: u32,
    pub pages: u32,
}

impl From<&TranscriptPage> for TranscriptPageData {
    fn from(page: &TranscriptPage) -> Self {
        let info = page.pages();
        TranscriptPageData {
            episode_id: page.episode_id.clone(),
            status: page.status.clone(),
            archived: page.archived,
            language: page.language.clone(),
            duration_ms: page.duration_ms,
            done_ms: page.done_ms,
            progress: page.progress,
            percent: page.percent(),
            total: page.total,
            offset: page.offset,
            limit: page.limit,
            query: page.query.clone(),
            segments: page
                .segments
                .iter()
                .map(|segment| TranscriptSegmentData {
                    start: segment.start,
                    end: segment.end,
                    text: segment.text.clone(),
                })
                .collect(),
            next: page.next,
            error_code: page.error_code.clone(),
            error_message: page.error_message.clone(),
            cached: false,
            active: page.active(),
            page: info.page,
            pages: info.pages,
        }
    }
}

impl TranscriptPageData {
    /// The same page, marked as read from the device's copy.
    fn cached(mut self) -> Self {
        self.cached = true;
        self
    }
}

/// One hit of a search inside a show's transcripts.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct TranscriptHitData {
    pub episode_id: String,
    pub episode_title: String,
    pub channel_id: String,
    pub channel_title: String,
    pub published: i64,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

impl From<&TranscriptHit> for TranscriptHitData {
    fn from(hit: &TranscriptHit) -> Self {
        TranscriptHitData {
            episode_id: hit.episode_id.clone(),
            episode_title: hit.episode_title.clone(),
            channel_id: hit.channel_id.clone(),
            channel_title: hit.channel_title.clone(),
            published: hit.published,
            start: hit.start,
            end: hit.end,
            text: hit.text.clone(),
        }
    }
}

/// A search inside one show's transcripts (`/podcast/<id>/transcript/search`).
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct TranscriptSearchData {
    pub q: String,
    pub hits: Vec<TranscriptHitData>,
    pub total: u32,
    pub more: bool,
    /// How many episodes were scanned.
    pub episodes: u32,
    /// The scan stopped at its budget: the screen says so.
    pub truncated: bool,
}

impl From<&TranscriptSearch> for TranscriptSearchData {
    fn from(search: &TranscriptSearch) -> Self {
        TranscriptSearchData {
            q: search.q.clone(),
            hits: search.hits.iter().map(TranscriptHitData::from).collect(),
            total: search.total,
            more: search.more,
            episodes: search.episodes,
            truncated: search.truncated,
        }
    }
}

impl From<&ShowSearch> for TranscriptSearchData {
    fn from(search: &ShowSearch) -> Self {
        TranscriptSearchData {
            q: search.query.clone(),
            hits: search.hits.iter().map(TranscriptHitData::from).collect(),
            total: search.total,
            more: false,
            episodes: search.episodes,
            truncated: search.truncated,
        }
    }
}

/// What `/api/podcast/<id>/auto_archive` answered.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct ShowArchiveData {
    /// The show as it now stands.
    pub show: ShowData,
    /// How many episodes the server queued for archiving.
    pub queued: u32,
}

/// What `/api/podcast/<id>/transcribe` answered.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct ShowTranscribeData {
    pub show: Option<ShowData>,
    pub queued: u32,
    /// Episodes the server refused (too long, over budget).
    pub skipped: u32,
    /// `all`, `recent` or `none`.
    pub backlog: String,
}

/// `GET /api/transcribe/settings`, as the show page's card reads it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct TranscribeSettingsData {
    pub available: bool,
    pub enabled: bool,
    /// A machine code (`engine_missing`, …); the screen turns it into a sentence
    /// with [`super::nz_error_text`]'s table.
    pub reason: Option<String>,
    pub language: String,
    /// The picker's languages, `auto` first.
    pub languages: Vec<String>,
    pub daily_minutes: u32,
    pub max_minutes: u32,
    pub queued: u32,
    pub running: u32,
    pub ready: u32,
    pub error: u32,
    pub engine: Option<String>,
    pub model: Option<String>,
}

impl From<&core_podcasts::TranscribeSettingsCard> for TranscribeSettingsData {
    fn from(settings: &core_podcasts::TranscribeSettingsCard) -> Self {
        TranscribeSettingsData {
            available: settings.available,
            enabled: settings.enabled,
            reason: settings.reason.clone(),
            language: settings.language.clone(),
            languages: settings.languages.clone(),
            daily_minutes: settings.daily_minutes,
            max_minutes: settings.max_minutes,
            queued: settings.queued,
            running: settings.running,
            ready: settings.ready,
            error: settings.error,
            engine: settings.engine.clone(),
            model: settings.model.clone(),
        }
    }
}

/// `GET /api/transcribe/status`, as the panel's status card reads it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Default)]
pub struct TranscribeStatusData {
    pub available: bool,
    pub enabled: bool,
    pub reason: Option<String>,
    pub queued: u32,
    pub running: u32,
    pub ready: u32,
    pub error: u32,
    pub budget_limit_ms: u64,
    pub budget_used_ms: u64,
}

impl From<&core_podcasts::TranscribeStatusCard> for TranscribeStatusData {
    fn from(status: &core_podcasts::TranscribeStatusCard) -> Self {
        TranscribeStatusData {
            available: status.available,
            enabled: status.enabled,
            reason: status.reason.clone(),
            queued: status.queued,
            running: status.running,
            ready: status.ready,
            error: status.error,
            budget_limit_ms: status.budget_limit_ms,
            budget_used_ms: status.budget_used_ms,
        }
    }
}

/// What the device's transcript cache holds (`transcriptcache.js#cacheStats`).
#[derive(uniffi::Record, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TranscriptCacheData {
    pub pages: u32,
    pub bytes: u64,
    pub episodes: u32,
}

/// The windowed handle over a show's loaded episodes, built over the core's own
/// list registry so a finished download or a like patches the row in place.
fn page_data(core: &Arc<nz_core::core::Core>, page: &ShowPage) -> ShowPageData {
    let rows = page.rows();
    let loaded = rows.len() as u32;
    ShowPageData {
        show: ShowData::from(&page.show),
        episodes: crate::lists::open_rows(core, rows),
        next: page.next.clone(),
        loaded,
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Screens {
    // -- the subscriptions --------------------------------------------------

    /// The shows this device kept — disk only, no request. This is what a warm
    /// open of « Podcasts » paints before the network answers.
    pub fn cached_podcasts(&self) -> Option<Vec<ShowData>> {
        self.core()
            .cached_podcasts()
            .map(|shows| shows.iter().map(ShowData::from).collect())
    }

    /// `GET /api/podcasts` — every show the server holds, unsubscribed ones
    /// included (their audio is on the server and stays reachable).
    pub async fn podcasts(&self) -> Result<Vec<ShowData>, NzError> {
        let shows = self.core().podcasts().await.map_err(NzError::from)?;
        Ok(shows.iter().map(ShowData::from).collect())
    }

    /// `GET /api/search/podcasts?q=` — the provider's catalogue, one request.
    pub async fn search_podcasts(&self, q: String) -> Result<Vec<ShowData>, NzError> {
        let shows = self.core().search_podcasts(&q).await.map_err(NzError::from)?;
        Ok(shows.iter().map(ShowData::from).collect())
    }

    /// `POST /api/podcasts {url}` — subscribe by a Deezer show URL (a catalogue
    /// card builds it, and the screen can also pass a pasted link).
    pub async fn subscribe_podcast(&self, url: String) -> Result<ShowData, NzError> {
        let card = self
            .core()
            .subscribe_podcast(&url)
            .await
            .map_err(NzError::from)?;
        Ok(ShowData::from(&card))
    }

    /// `DELETE /api/podcast/<id>` — unsubscribe. The archived audio stays.
    pub async fn unsubscribe_podcast(&self, id: String) -> Result<(), NzError> {
        self.core()
            .unsubscribe_podcast(&id)
            .await
            .map_err(NzError::from)
    }

    // -- one show -----------------------------------------------------------

    /// The show this device kept: its header, the episodes loaded so far and the
    /// next cursor — disk only, no request.
    pub fn cached_podcast_show(&self, id: String) -> Option<ShowPageData> {
        self.core()
            .cached_show(&id)
            .map(|page| page_data(self.core(), &page))
    }

    /// `GET /api/podcast/<id>?limit=50` — the first page and the show's markers
    /// (two requests, the SPA's own count), by UUID or by the provider's id.
    pub async fn podcast_show(&self, id: String) -> Result<ShowPageData, NzError> {
        let page = self.core().show(&id).await.map_err(NzError::from)?;
        Ok(page_data(self.core(), &page))
    }

    /// The NEXT page of episodes, appended to what the device holds (one
    /// request). A show of 2 000 episodes is 40 of these, never one big answer.
    pub async fn podcast_show_more(&self, id: String) -> Result<ShowPageData, NzError> {
        let page = self.core().show_more(&id).await.map_err(NzError::from)?;
        Ok(page_data(self.core(), &page))
    }

    /// The episodes `start..start+count` this device holds, for the rows a
    /// windowed list is about to draw. **No request**: the page is already in.
    pub fn podcast_episode_cards(
        &self,
        id: String,
        start: u32,
        count: u32,
    ) -> Vec<EpisodeData> {
        self.core()
            .episode_cards(&id, start, count)
            .iter()
            .map(EpisodeData::from)
            .collect()
    }

    /// `POST /api/podcast/<id>/auto_archive` — archive this show's new episodes
    /// and queue the ones it lists. Optimistic: a refusal puts the switch back.
    pub async fn podcast_auto_archive(
        &self,
        id: String,
        on: bool,
    ) -> Result<ShowArchiveData, NzError> {
        let answer = self
            .core()
            .auto_archive(&id, on)
            .await
            .map_err(NzError::from)?;
        Ok(ShowArchiveData {
            show: ShowData::from(&answer.show),
            queued: answer.queued,
        })
    }

    /// `POST /api/podcast/<id>/transcribe` — the show's auto-transcription
    /// switch, its language and its backlog mode (`all`, `recent`, `none`).
    /// The answer says what the server queued and what it skipped (rule 2).
    pub async fn podcast_set_show_transcribe(
        &self,
        id: String,
        auto: Option<bool>,
        backlog: Option<String>,
        language: Option<String>,
    ) -> Result<ShowTranscribeData, NzError> {
        let answer = self
            .core()
            .set_show_transcribe(&id, auto, backlog.as_deref(), language.as_deref())
            .await
            .map_err(NzError::from)?;
        Ok(ShowTranscribeData {
            show: answer.show.as_ref().map(ShowData::from),
            queued: answer.queued,
            skipped: answer.skipped,
            backlog: answer.backlog,
        })
    }

    // -- positions ----------------------------------------------------------

    /// `GET /api/podcast/progress` — every episode this person has started, the
    /// device's mirror merged in (newest wins). One request.
    pub async fn podcast_progress(&self) -> Result<Vec<EpisodePositionEntry>, NzError> {
        let positions = self.core().progress().await.map_err(NzError::from)?;
        Ok(entries(positions))
    }

    /// The merged positions, disk only: an episode resumes at its position with
    /// the link down, and this is what the show page's bars read.
    pub fn podcast_positions(&self) -> Vec<EpisodePositionEntry> {
        entries(self.core().positions())
    }

    /// One episode's position, or `None` when there is nothing worth resuming.
    pub fn podcast_episode_position(&self, episode_id: String) -> Option<EpisodePositionData> {
        self.core()
            .position(&episode_id)
            .as_ref()
            .map(EpisodePositionData::from)
    }

    // -- markers ------------------------------------------------------------

    /// `GET /api/podcast/<id>/markers` — the show's markers, kept for the rows
    /// (one request; a row's own chips are then read with
    /// [`Screens::podcast_episode_markers`], which costs nothing). Fail-soft in
    /// the app: the SPA ignores a refusal here (`markers.js`).
    pub async fn podcast_load_show_markers(&self, id: String) -> Result<(), NzError> {
        self.core().show_markers(&id).await.map_err(NzError::from)
    }

    /// `GET /api/podcast/episode/<id>/markers` — one episode's markers, for a
    /// panel opened from the player instead of from the show page.
    pub async fn podcast_load_episode_markers(
        &self,
        episode_id: String,
    ) -> Result<Vec<EpisodeMarkerData>, NzError> {
        let markers = self
            .core()
            .load_episode_markers(&episode_id)
            .await
            .map_err(NzError::from)?;
        Ok(markers.iter().map(EpisodeMarkerData::from).collect())
    }

    /// One episode's markers, disk only — what the chips under a row read.
    pub fn podcast_episode_markers(&self, episode_id: String) -> Vec<EpisodeMarkerData> {
        self.core()
            .episode_markers(&episode_id)
            .iter()
            .map(EpisodeMarkerData::from)
            .collect()
    }

    /// `POST /api/podcast/episode/<id>/markers` — a marker at `position_ms`
    /// (MILLISECONDS). Optimistic: a refusal removes the chip it just added.
    pub async fn podcast_add_marker(
        &self,
        episode_id: String,
        position_ms: u32,
        label: Option<String>,
    ) -> Result<EpisodeMarkerData, NzError> {
        let marker = self
            .core()
            .add_marker(&episode_id, position_ms, label.as_deref())
            .await
            .map_err(NzError::from)?;
        Ok(EpisodeMarkerData::from(&marker))
    }

    /// `DELETE /api/podcast/marker/<id>` — optimistic, rolled back on a refusal.
    pub async fn podcast_remove_marker(&self, marker_id: String) -> Result<(), NzError> {
        self.core()
            .remove_marker(&marker_id)
            .await
            .map_err(NzError::from)
    }

    // -- chapters -----------------------------------------------------------

    /// The chapter list this device kept, or `None` the first time — disk only.
    pub fn cached_podcast_chapters(&self, episode_id: String) -> Option<ChaptersData> {
        self.core()
            .cached_chapters(&episode_id)
            .as_ref()
            .map(ChaptersData::from)
    }

    /// `GET /api/podcast/episode/<id>/chapters` — seconds, from the archived
    /// file; one request, and the copy is kept for the next open.
    pub async fn podcast_chapters(&self, episode_id: String) -> Result<ChaptersData, NzError> {
        let chapters = self
            .core()
            .chapters(&episode_id)
            .await
            .map_err(NzError::from)?;
        Ok(ChaptersData::from(&chapters))
    }

    // -- transcripts --------------------------------------------------------

    /// The transcript page this device kept — disk only, 0 requests. A ready
    /// transcript therefore costs nothing while listening.
    pub fn cached_podcast_transcript(
        &self,
        episode_id: String,
        limit: u32,
        offset: u32,
        at: Option<f64>,
    ) -> Option<TranscriptPageData> {
        self.core()
            .cached_transcript(&episode_id, limit, offset, at)
            .as_ref()
            .map(|page| TranscriptPageData::from(page).cached())
    }

    /// `GET /api/podcast/episode/<id>/transcript` — one page: by `offset`, by
    /// `at` (the page that covers that moment), or the lines matching `q`.
    pub async fn podcast_transcript(
        &self,
        episode_id: String,
        limit: u32,
        offset: Option<u32>,
        at: Option<f64>,
        q: Option<String>,
    ) -> Result<TranscriptPageData, NzError> {
        let page = self
            .core()
            .transcript(&episode_id, limit, offset, at, q.as_deref())
            .await
            .map_err(NzError::from)?;
        Ok(TranscriptPageData::from(&page))
    }

    /// A search inside the pages this device HOLDS for one episode — what the
    /// panel answers with when the link is dead (`localSearch`).
    pub fn podcast_local_search(
        &self,
        episode_id: String,
        q: String,
        limit: u32,
    ) -> Vec<TranscriptSegmentData> {
        self.core()
            .local_transcript_search(&episode_id, &q, limit)
            .iter()
            .map(|segment| TranscriptSegmentData {
                start: segment.start,
                end: segment.end,
                text: segment.text.clone(),
            })
            .collect()
    }

    /// `GET /api/podcast/<id>/transcript/search?q=` — inside one show. Two
    /// characters minimum, exactly as the SPA's field requires.
    pub async fn podcast_show_search(
        &self,
        id: String,
        q: String,
        limit: u32,
    ) -> Result<TranscriptSearchData, NzError> {
        let search = self
            .core()
            .show_transcript_search(&id, &q, limit)
            .await
            .map_err(NzError::from)?;
        Ok(TranscriptSearchData::from(&search))
    }

    /// Whether this server has the transcription routes at all (rule 7). `None`
    /// is a doubt, and a doubt hides nothing.
    pub async fn podcast_transcription_supported(&self) -> Option<bool> {
        self.core().transcription_supported().await
    }

    /// `GET /api/transcribe/settings` — availability, languages, the queue.
    pub async fn podcast_transcribe_settings(
        &self,
        probe: bool,
    ) -> Result<TranscribeSettingsData, NzError> {
        let settings = self
            .core()
            .transcribe_settings(probe)
            .await
            .map_err(NzError::from)?;
        Ok(TranscribeSettingsData::from(&settings))
    }

    /// `GET /api/transcribe/status` — the queue and the daily budget.
    pub async fn podcast_transcribe_status(&self) -> Result<TranscribeStatusData, NzError> {
        let status = self
            .core()
            .transcribe_status()
            .await
            .map_err(NzError::from)?;
        Ok(TranscribeStatusData::from(&status))
    }

    /// `POST /api/podcast/episode/<id>/transcribe` — a server JOB: it answers at
    /// once and the panel polls [`Screens::podcast_transcript`] for its state.
    pub async fn podcast_start_transcript(
        &self,
        episode_id: String,
        language: Option<String>,
        force: bool,
    ) -> Result<(), NzError> {
        self.core()
            .start_transcript(&episode_id, language.as_deref(), force)
            .await
            .map_err(NzError::from)
    }

    /// `POST /api/podcast/episode/<id>/transcribe/stop`.
    pub async fn podcast_stop_transcript(&self, episode_id: String) -> Result<(), NzError> {
        self.core()
            .stop_transcript(&episode_id)
            .await
            .map_err(NzError::from)
    }

    /// `DELETE /api/podcast/episode/<id>/transcript` — the server's text and
    /// this device's copy of its pages.
    pub async fn podcast_delete_transcript(&self, episode_id: String) -> Result<(), NzError> {
        self.core()
            .delete_transcript(&episode_id)
            .await
            .map_err(NzError::from)
    }

    /// What the device holds of transcripts: pages, bytes and episodes. The
    /// panel says it, and the acceptance reads the cap from here.
    pub fn podcast_transcript_cache(&self) -> TranscriptCacheData {
        let stats = self.core().transcript_cache_stats();
        TranscriptCacheData {
            pages: stats.pages,
            bytes: stats.bytes,
            episodes: stats.episodes,
        }
    }
}

/// The merged positions as the record list the screen draws.
fn entries(positions: std::collections::BTreeMap<String, Position>) -> Vec<EpisodePositionEntry> {
    positions
        .into_iter()
        .map(|(episode_id, position)| EpisodePositionEntry {
            episode_id,
            position: EpisodePositionData::from(&position),
        })
        .collect()
}

/// The error a screen shows for a transcript that the server refuses with a
/// machine code: `too_long`, `not_archived`, `no_duration`, `queue_full`,
/// `already_running` (`Show.svelte#transcribeError`).
#[uniffi::export]
pub fn nz_podcast_transcription_cause(code: String) -> String {
    transcription_cause(&code)
}

/// The same table, for this crate's own callers and tests.
pub fn transcription_cause(code: &str) -> String {
    match code {
        "too_long" => "L'épisode dépasse la durée maximale de transcription.".to_owned(),
        "not_archived" => "L'épisode doit d'abord être archivé sur le serveur.".to_owned(),
        "no_duration" => "La durée de l'épisode est inconnue ; impossible de la borner.".to_owned(),
        "queue_full" => "La file de transcription est pleine.".to_owned(),
        "already_running" => "Cet épisode est déjà en cours de transcription.".to_owned(),
        _ => "La transcription n'a pas pu démarrer.".to_owned(),
    }
}

/// One piece of a line with the search's words marked (`highlightParts`), so the
/// panel draws a hit's highlights without re-implementing the merge.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq, Default)]
pub struct TextPartData {
    pub text: String,
    pub marked: bool,
}

/// « Français », « Détection automatique », or the code in capitals.
#[uniffi::export]
pub fn nz_podcast_language_label(code: String) -> String {
    core_podcasts::language_label(&code)
}

/// Why the server cannot transcribe, in French — never a machine code.
#[uniffi::export]
pub fn nz_podcast_reason_text(reason: String) -> String {
    core_podcasts::reason_text(&reason)
}

/// The batch sizes the transcription card offers for `eligible` episodes
/// (5, then 10 and 20).
#[uniffi::export]
pub fn nz_podcast_batch_options(eligible: u32) -> Vec<u32> {
    core_podcasts::batch_options(eligible)
}

/// `highlightParts`: the whole query first, then each word, the ranges merged —
/// a line never gets two overlapping marks.
#[uniffi::export]
pub fn nz_podcast_highlight_parts(text: String, query: String) -> Vec<TextPartData> {
    core_podcasts::highlight_parts(&text, &query)
        .into_iter()
        .map(|part| TextPartData {
            text: part.text,
            marked: part.marked,
        })
        .collect()
}

/// `pageInfo(offset, limit, total)`: which page of a paged read, and the rows it
/// covers. Returned as a record so the panel's footer is the core's arithmetic.
#[derive(uniffi::Record, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PageInfoData {
    pub page: u32,
    pub pages: u32,
    pub from: u32,
    pub to: u32,
}

/// One page's numbers (`transcript.js#pageInfo`).
#[uniffi::export]
pub fn nz_podcast_page_info(offset: u32, limit: u32, total: u32) -> PageInfoData {
    let info: core_podcasts::PageInfo = core_podcasts::page_info(offset, limit, total);
    PageInfoData {
        page: info.page,
        pages: info.pages,
        from: info.from,
        to: info.to,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The units of this boundary: a marker's milliseconds become seconds AND a
    /// usual label, so no screen does the arithmetic.
    #[test]
    fn a_marker_crosses_with_both_its_units() {
        let marker = Marker {
            id: "m-1".to_owned(),
            episode_id: "ep".to_owned(),
            position_ms: 600_000,
            label: "La maison".to_owned(),
        };
        let data = EpisodeMarkerData::from(&marker);
        assert_eq!(data.position_ms, 600_000);
        assert_eq!(data.seconds, 600);
        assert_eq!(data.time_label, "10:00");
    }

    /// A catalogue card knows the URL that subscribes it, and a row of this
    /// server does not (it is already subscribed).
    #[test]
    fn a_show_carries_its_own_subscribe_url() {
        let card = ShowCard {
            deezer_id: Some("9400001".to_owned()),
            title: "Les Histoires".to_owned(),
            episode_count: 40,
            archived_count: 12,
            ..ShowCard::default()
        };
        let data = ShowData::from(&card);
        assert_eq!(
            data.subscribe_url.as_deref(),
            Some("https://www.deezer.com/show/9400001")
        );
        assert_eq!(
            data.archive_line,
            "12 sur 40 épisodes archivés sur le serveur"
        );
    }

    /// Every code the server can refuse a transcription with has a French
    /// sentence, and none of them leaks the code to the person.
    #[test]
    fn every_transcription_code_has_a_sentence() {
        for code in [
            "too_long",
            "not_archived",
            "no_duration",
            "queue_full",
            "already_running",
            "something_else",
        ] {
            let text = transcription_cause(code);
            assert!(text.ends_with('.'), "{code}: {text}");
            assert!(!text.contains('_'), "{code} leaked: {text}");
        }
    }

    /// A transcript page crosses with its own page numbers and its cached flag,
    /// which is what the panel's footer says (« page 3 sur 5 »).
    #[test]
    fn a_transcript_page_crosses_with_its_page_numbers() {
        let page = TranscriptPage {
            status: "ready".to_owned(),
            total: 450,
            offset: 400,
            limit: 200,
            progress: 1.0,
            ..TranscriptPage::default()
        };
        let data = TranscriptPageData::from(&page);
        assert_eq!(data.page, 3);
        assert_eq!(data.pages, 3);
        assert_eq!(data.percent, 100);
        assert!(!data.cached);
        assert!(TranscriptPageData::from(&page).cached().cached);
    }
}
