//! One podcast show (lot D11): its sleeve and facts, its two switches, the
//! server's transcription card, the search inside its transcripts, and its
//! episodes — paged by cursor, windowed, with the selected episode's panel.
//!
//! `webapp/src/routes/Show.svelte` is the reference; each rule below was a bug
//! there before it was a rule here:
//!
//! - **The episode list is PAGED BY CURSOR**: `GET /api/podcast/<id>?limit=50`
//!   answers a page plus an opaque `next`, and « Charger plus d'épisodes » sends
//!   it back. Nothing ever asks for the whole archive (rule 3), and the rows are
//!   windowed at one 64 px size (a page of 50 paints the ~12 a window shows).
//! - **The archive switch says what it did.** Turning it on queues every episode
//!   the show is missing on the server's background archiver; the line under it
//!   counts the files in and the page re-reads ONE page every 6 s while the count
//!   moves, stopping after five stalls or when the show is complete.
//! - **The transcription card is a job's panel**: engine availability with its
//!   cause, the language, the backlog, the batch with its audio/CPU estimate, the
//!   running job with its progress and « Arrêter », and every refusal in French
//!   (rule 2: a failure names its cause, never a count).
//! - **The progress follows the person** — [`progress`] is the keeper: pause,
//!   every 30 s while playing, episode change, quit; offline the write waits in
//!   `nz-cache`'s `pending_progress` and goes up when the link returns.
//! - **The show's speed is the show's** (`podcast.speeds`, D9's map, keyed by
//!   `channel_id`): the chip writes the key the player reads.
//! - **Background work yields** (rule 1): "Tout télécharger" hands the episodes to
//!   D13's `nz-bulk` worker (`Offline::choose`), one at a time, behind playback.
//!
//! The screen takes no focus (D8b). Its verbs are the app-level list intents and
//! two bindings this file installs once (`Ctrl+T` / `Ctrl+Maj+T`: start / stop the
//! panel episode's transcription — the actions D7 declared and left unbound).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use gpui_kit::base::VirtualListScrollHandle;
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, FontWeight,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement, Render, SharedString, Size,
    StatefulInteractiveElement as _, Styled, Subscription, Task, Window, div, px, relative, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon, icon_button, switch, virtual_list};
use nz_api::{Api, ApiError, Lane};
use nz_model::{
    Episode, EpisodeChapters, EpisodeTranscript, EpisodeTranscriptInfo, Marker, PodcastPage,
    PodcastPosition, PodcastShow, ShowMarkers, TranscribeSettings, TranscriptHit, TranscriptSearch,
};
use nz_player::Entry;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::actions::{DownloadSelection, RemoveDownload, StartTranscription, StopTranscription};
use crate::nav::Route;
use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;
use crate::widgets::card::{self, Phase};
use crate::widgets::chapters::Chapters;
use crate::widgets::cover::{self, Covers, Kind};
use crate::widgets::download_control::{
    self, Rows, State as DownloadState, Watch as DownloadWatch,
};
use crate::widgets::episode_panel::{self, Act, EpisodePanel, Resume};
use crate::widgets::markers::{self, Markers};
use crate::widgets::speed;
use crate::widgets::track_list::{self, Intent, duration_label};
use crate::widgets::transcript_panel::{self as transcript, Ask as TranscriptAsk, Confirm, Tone};
use crate::wiring::events::{self, Sender};

/// Episodes per page: the server's own default (`PODCAST_EPISODES_PAGE`).
pub const PAGE: u32 = 50;
/// The archive/transcript poll beat (`Show.svelte`'s `POLL_MS`).
const POLL_MS: u64 = 6_000;
/// Five polls without a new file and the page stops asking (`STALL_POLLS`).
const STALL_POLLS: u32 = 5;
/// The transcript search's quiet time (the SPA's 300 ms).
const SEARCH_DEBOUNCE_MS: u64 = 300;
/// The shortest term worth a request.
const MIN_QUERY: usize = 2;
/// How often the keeper writes a playing episode.
pub const PROGRESS_EVERY: Duration = Duration::from_secs(30);
/// Below this, a position is a mis-tap, not a resume point (the SPA's `MIN_SAVE`).
pub const PROGRESS_MIN: f64 = 8.0;
/// Within this many seconds of the end an episode is finished
/// (`_EPISODE_DONE_TAIL` in `neezer/webui/__init__.py`).
pub const DONE_TAIL: u32 = 15;
/// One episode row's height.
pub const ROW_HEIGHT: f32 = 64.;

/// The head never takes more than this share of the page: the episode list is
/// what the page is for.
const HEAD_SHARE: f32 = 0.45;
/// The page's reading width: the same 1800 px cap D12's screens use, so a
/// 2560 px window is margins rather than a two-metre episode row.
const PAGE_MAX: f32 = 1_800.;
/// The shell's header and player bar, which the screen never sees.
const SHELL_CHROME: f32 = 144.;
/// Under this page height the hero loses its sleeve size and its description:
/// at 1280×800 the full hero plus the admin block left the list 38 px.
const COMPACT_BELOW: f32 = 900.;
/// Below this much content the panel cannot sit beside the list.
const PANEL_MIN_CONTENT: f32 = 1_040.;
/// The panel's width when it is a column.
const PANEL_WIDTH: f32 = 460.;
/// The shell's fixed rails (D7): a 200 px sidebar and a 420 px queue panel.
const SIDEBAR: f32 = 200.;
const QUEUE_PANEL: f32 = 420.;

/// What the show page's head does at a given page height.
///
/// The head is a fixed region above a windowed list, capped at [`HEAD_SHARE`] of
/// the page: with the admin's transcription card it is taller than the cap at
/// every window under ~1300 px, and the person then saw a switch cut in half
/// above the first episode (1280×800, measured). So the hero goes compact when
/// the page is short, and the admin block FOLDS (one summary row that opens it)
/// whenever the head with it would not fit the cap — the person can still open
/// it, deliberately, and its body scrolls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeadFit {
    /// A 96 px sleeve, no description, a smaller title.
    pub compact: bool,
    /// The admin block is expanded, not folded into its summary row.
    pub settings: bool,
}

/// The heights the decision compares, in pixels (they are the real ones at the
/// default rem size: the hero's 180 px sleeve plus its 28 px padding, a 44 px
/// action row, a 34 px folded row, the 220 px unfolded admin block and the 40 px
/// transcript search row).
const HERO_FULL: f32 = 236.;
const HERO_COMPACT: f32 = 152.;
const ACTIONS_ROW: f32 = 44.;
const FOLDED_ROW: f32 = 34.;
const ADMIN_OPEN: f32 = 220.;
const SEARCH_ROW: f32 = 40.;

pub fn head_fit(page_height: f32) -> HeadFit {
    let cap = page_height * HEAD_SHARE;
    let compact = page_height < COMPACT_BELOW;
    let hero = if compact { HERO_COMPACT } else { HERO_FULL };
    HeadFit {
        compact,
        settings: hero + ACTIONS_ROW + ADMIN_OPEN + SEARCH_ROW <= cap,
    }
}

/// The page height a window gives the screen: the shell's header and player bar
/// are not the screen's.
fn page_height(window: &Window) -> f32 {
    (f32::from(window.viewport_size().height) - SHELL_CHROME).max(0.)
}

/// The backlog a show's transcription switch applies when turned on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backlog {
    #[default]
    All,
    Recent,
    None,
}

impl Backlog {
    pub fn wire(self) -> &'static str {
        match self {
            Backlog::All => "all",
            Backlog::Recent => "recent",
            Backlog::None => "none",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Backlog::All => "Tout l'historique archivé",
            Backlog::Recent => "Les 10 plus récents",
            Backlog::None => "Aucun rattrapage",
        }
    }

    const ALL: [Backlog; 3] = [Backlog::All, Backlog::Recent, Backlog::None];
}

/// Which job verb was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Job {
    Started,
    Stopped,
    Relaunched,
    Deleted,
}

/// What a background thread tells this screen.
enum Msg {
    Page(Box<Result<PodcastPage, ApiError>>),
    More(Box<Result<PodcastPage, ApiError>>),
    Poll(Box<Result<PodcastPage, ApiError>>),
    Markers(String, Box<Result<ShowMarkers, ApiError>>),
    Chapters(String, Box<Result<EpisodeChapters, ApiError>>),
    Transcribe(Box<Result<TranscribeSettings, ApiError>>),
    AutoArchive(Box<Result<nz_api::PodcastAutoArchive, ApiError>>),
    ShowTranscribe(Box<Result<serde_json::Value, ApiError>>),
    Batch {
        queued: u32,
        skipped: u32,
        cause: Option<SharedString>,
    },
    MarkerAdded(String, String, Box<Result<Marker, ApiError>>),
    MarkerRemoved(Marker, Box<Result<(), ApiError>>),
    Transcript {
        episode: String,
        tone: Tone,
        answer: Box<Result<EpisodeTranscript, ApiError>>,
    },
    TranscriptSearch {
        episode: String,
        term: String,
        answer: Box<Result<EpisodeTranscript, ApiError>>,
    },
    ShowSearch(Box<Result<TranscriptSearch, ApiError>>),
    Job {
        episode: String,
        done: Job,
        answer: Box<Result<(), ApiError>>,
    },
    Cover(String, Option<PathBuf>),
}

/// The row menu: what `Menu`/`Maj+F10` or the row's `⋯` opens. Drawn here, not
/// with `context_menu`'s track items, because an episode's verbs are not a
/// track's (that menu offers Radio and Credits, which mean nothing here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowItem {
    Play,
    PlayNext,
    Queue,
    Download,
    Marker,
    Details,
}

impl RowItem {
    const ALL: [RowItem; 6] = [
        RowItem::Play,
        RowItem::PlayNext,
        RowItem::Queue,
        RowItem::Download,
        RowItem::Marker,
        RowItem::Details,
    ];

    /// The download entry is the ONE row whose verb depends on where the
    /// episode stands (lot D18): ask for it, stop it, take it back, ask again.
    fn label_for(self, download: &download_control::State) -> &'static str {
        match self {
            RowItem::Download => match download {
                download_control::State::Idle => "Télécharger sur cet appareil",
                download_control::State::Queued { .. }
                | download_control::State::Running { .. } => "Annuler le téléchargement",
                download_control::State::Failed { .. } => "Réessayer le téléchargement",
                download_control::State::Done { .. } => "Retirer le téléchargement",
            },
            other => other.label(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            RowItem::Play => "Lire l'épisode",
            RowItem::PlayNext => "Lire ensuite",
            RowItem::Queue => "Ajouter à la file",
            RowItem::Download => "Télécharger sur cet appareil",
            RowItem::Marker => "Poser un marqueur ici",
            RowItem::Details => "Chapitres et transcription",
        }
    }

    fn glyph(self) -> IconName {
        match self {
            RowItem::Play => IconName::Play,
            RowItem::PlayNext => IconName::SkipForward,
            RowItem::Queue => IconName::ListMusic,
            RowItem::Download => IconName::Download,
            RowItem::Marker => IconName::BookmarkPlus,
            RowItem::Details => IconName::ScrollText,
        }
    }
}

struct RowMenu {
    index: usize,
    cursor: usize,
}

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    id: String,
    /// The downloads worker, as this page reads it (lot D18): one state for the
    /// show's own download, one per episode.
    downloads: DownloadWatch,
    rows: Rc<Rows>,
    rows_version: u64,
    page: Phase<PodcastPage>,
    /// Page one plus every "Charger plus".
    episodes: Vec<Episode>,
    next: Option<String>,
    loading_more: bool,
    more_failed: Option<ApiError>,
    covers: Rc<RefCell<Covers>>,
    chapters: Chapters,
    markers: Markers,
    positions: Vec<(String, PodcastPosition)>,
    panel: Option<EpisodePanel>,
    row_menu: Option<RowMenu>,
    /// The interceptor that owns Échap while a panel or a menu is open.
    keys: Option<Subscription>,
    cursor: usize,
    sizes: Rc<Vec<Size<gpui_kit::Pixels>>>,
    scroll: VirtualListScrollHandle,
    /// Rows the last paint built (the windowing assertion).
    pub rendered_rows: usize,
    auto_busy: bool,
    archiving: bool,
    stalls: u32,
    poll: Option<Task<()>>,
    transcribe: Phase<TranscribeSettings>,
    backlog: Backlog,
    language: String,
    batch: usize,
    batch_busy: bool,
    search_field: Option<Entity<InputState>>,
    search_query: String,
    search_pending: Option<String>,
    search_term: String,
    search_hits: Vec<TranscriptHit>,
    search_total: u32,
    search_loading: bool,
    search_failed: Option<ApiError>,
    search_cleared: bool,
    transcript_field: Option<Entity<InputState>>,
    transcript_query: String,
    transcript_pending: Option<String>,
    download_busy: bool,
    unsubscribe_confirm: bool,
    speed_open: bool,
    /// The admin block, as the person left it (`None`: `head_fit` decides).
    settings_open: Option<bool>,
    /// What the last frame actually showed, so the row's own click flips it.
    settings_shown: bool,
    events: Sender<Msg>,
    _subscriptions: Vec<Subscription>,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let id = match &route {
            Route::Show { id } => id.clone(),
            _ => String::new(),
        };
        let path = format!("/podcast/{}?limit={PAGE}", card::encode(&id));
        let page = match card::cached::<PodcastPage>(state.read(cx), &path) {
            Some(page) => Phase::Ready(page),
            None => Phase::Loading,
        };
        let episodes: Vec<Episode> = page
            .ready()
            .map(|page| page.episodes.clone())
            .unwrap_or_default();
        let next = page.ready().and_then(|page| page.next.clone());
        let language = page
            .ready()
            .and_then(|page| page.show.transcribe_language.clone())
            .filter(|code| !code.is_empty())
            .unwrap_or_else(|| "auto".to_owned());
        let sizes = Rc::new(vec![
            Size {
                width: px(0.),
                height: px(ROW_HEIGHT),
            };
            episodes.len()
        ]);
        let (tx, rx) = events::channel::<Msg>();
        let downloads = DownloadWatch::attach(&state, cx);
        let mut screen = Self {
            route,
            state: state.clone(),
            id,
            downloads,
            rows: Rc::new(Rows::default()),
            rows_version: u64::MAX,
            page,
            episodes,
            next,
            loading_more: false,
            more_failed: None,
            covers: Rc::new(RefCell::new(Covers::new())),
            chapters: Chapters::new(),
            markers: Markers::new(),
            positions: positions(cx),
            panel: None,
            row_menu: None,
            keys: None,
            cursor: 0,
            sizes,
            scroll: VirtualListScrollHandle::new(),
            rendered_rows: 0,
            auto_busy: false,
            archiving: false,
            stalls: 0,
            poll: None,
            transcribe: Phase::Loading,
            backlog: Backlog::default(),
            language,
            batch: 5,
            batch_busy: false,
            search_field: None,
            search_query: String::new(),
            search_pending: None,
            search_term: String::new(),
            search_hits: Vec::new(),
            search_total: 0,
            search_loading: false,
            search_failed: None,
            search_cleared: false,
            transcript_field: None,
            transcript_query: String::new(),
            transcript_pending: None,
            download_busy: false,
            unsubscribe_confirm: false,
            speed_open: false,
            settings_open: None,
            settings_shown: false,
            events: tx,
            _subscriptions: vec![cx.observe(&state, |this, _, cx| {
                this.positions = positions(cx);
                cx.notify();
            })],
        };
        attach_keeper(state, cx);
        install_transcribe_keys(cx);
        register_transcribe_actions(cx);
        track_list::own(cx, |this: &mut Self, intent, window, cx| {
            this.on_intent(intent, window, cx)
        });
        screen.drain(rx, cx);
        screen.bind_download(cx);
        screen.load(cx);
        if screen.is_admin(cx) {
            screen.load_transcribe_settings(cx);
        }
        screen.load_markers(cx);
        screen
    }

    pub fn route(&self) -> &Route {
        &self.route
    }

    fn is_admin(&self, cx: &App) -> bool {
        self.state.read(cx).is_admin()
    }

    fn drain(&mut self, rx: events::Receiver<Msg>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                if this.update(cx, |this, cx| this.on_msg(msg, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    // -- requests ----------------------------------------------------------

    fn page_path(&self, cursor: Option<&str>) -> String {
        match cursor {
            Some(cursor) => format!(
                "/podcast/{}?limit={PAGE}&cursor={}",
                card::encode(&self.id),
                card::encode(cursor)
            ),
            None => format!("/podcast/{}?limit={PAGE}", card::encode(&self.id)),
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let (Some(api), runtime) = (
            self.state.read(cx).api.clone(),
            self.state.read(cx).runtime.clone(),
        ) else {
            return;
        };
        let path = self.page_path(None);
        let tx = self.events.clone();
        runtime.spawn(async move {
            let answer = get::<PodcastPage>(&api, Lane::User, &path).await;
            tx.send(Msg::Page(Box::new(answer)));
        });
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.loading_more {
            return;
        }
        let Some(cursor) = self.next.clone() else {
            return;
        };
        let (Some(api), runtime) = (
            self.state.read(cx).api.clone(),
            self.state.read(cx).runtime.clone(),
        ) else {
            return;
        };
        self.loading_more = true;
        self.more_failed = None;
        let path = self.page_path(Some(&cursor));
        let tx = self.events.clone();
        runtime.spawn(async move {
            let answer = get::<PodcastPage>(&api, Lane::User, &path).await;
            tx.send(Msg::More(Box::new(answer)));
        });
        cx.notify();
    }

    fn load_markers(&mut self, cx: &mut Context<Self>) {
        if !self.markers.wants(&self.id) {
            return;
        }
        let snapshot = self.state.read(cx);
        let tx = self.events.clone();
        let channel = self.id.clone();
        self.markers
            .load(snapshot, &channel, move |channel, answer| {
                tx.send(Msg::Markers(channel, Box::new(answer)));
            });
        cx.notify();
    }

    fn load_transcribe_settings(&mut self, cx: &mut Context<Self>) {
        let (Some(api), runtime) = (
            self.state.read(cx).api.clone(),
            self.state.read(cx).runtime.clone(),
        ) else {
            return;
        };
        self.transcribe = Phase::Loading;
        let tx = self.events.clone();
        runtime.spawn(async move {
            let answer = api.transcribe_settings(false).await;
            tx.send(Msg::Transcribe(Box::new(answer)));
        });
    }

    fn arm_poll(&mut self, cx: &mut Context<Self>) {
        if self.poll.is_some() {
            return;
        }
        self.poll = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(POLL_MS))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.poll = None;
                this.poll_now(cx);
            });
        }));
    }

    fn poll_now(&mut self, cx: &mut Context<Self>) {
        let (Some(api), runtime) = (
            self.state.read(cx).api.clone(),
            self.state.read(cx).runtime.clone(),
        ) else {
            return;
        };
        let path = self.page_path(None);
        let tx = self.events.clone();
        runtime.spawn(async move {
            let answer = get::<PodcastPage>(&api, Lane::Prefetch, &path).await;
            tx.send(Msg::Poll(Box::new(answer)));
        });
    }

    fn any_transcript_active(&self) -> bool {
        self.episodes.iter().any(|episode| {
            episode
                .transcript
                .as_ref()
                .is_some_and(|info| transcript::is_active(&info.status))
        }) || self
            .panel
            .as_ref()
            .is_some_and(|panel| transcript::is_active(&panel.transcript.status))
    }

    // -- answers -----------------------------------------------------------

    fn on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Page(result) => match *result {
                Ok(page) => self.apply_page(page, true),
                Err(error) => {
                    if self.page.ready().is_none() {
                        self.page = Phase::Failed(error);
                    }
                }
            },
            Msg::More(result) => {
                self.loading_more = false;
                match *result {
                    Ok(page) => {
                        let known: std::collections::HashSet<String> = self
                            .episodes
                            .iter()
                            .map(|episode| episode.id.as_str().to_owned())
                            .collect();
                        let fresh: Vec<Episode> = page
                            .episodes
                            .into_iter()
                            .filter(|episode| !known.contains(episode.id.as_str()))
                            .collect();
                        self.episodes.extend(fresh);
                        self.next = page.next;
                        self.rebuild_sizes();
                    }
                    Err(error) => self.more_failed = Some(error),
                }
            }
            Msg::Poll(result) => match *result {
                Ok(page) => {
                    let was = self.show().map(|show| show.archived_count).unwrap_or(0);
                    let now = page.show.archived_count;
                    self.stalls = if now > was { 0 } else { self.stalls + 1 };
                    self.apply_page(page, false);
                    let reachable = self.reachable();
                    let complete = self
                        .show()
                        .is_some_and(|show| show.archived_count >= reachable);
                    let auto = self.show().is_some_and(|show| show.auto_archive);
                    if !auto || complete || self.stalls >= STALL_POLLS {
                        self.archiving = false;
                    }
                    if self.archiving || self.any_transcript_active() {
                        self.arm_poll(cx);
                    }
                }
                Err(_) => {
                    self.stalls += 1;
                    if self.stalls < STALL_POLLS && (self.archiving || self.any_transcript_active())
                    {
                        self.arm_poll(cx);
                    } else {
                        self.archiving = false;
                    }
                }
            },
            Msg::Markers(channel, answer) => self.markers.resolved(&channel, *answer),
            Msg::Chapters(episode, answer) => self.chapters.resolved(&episode, *answer),
            Msg::Transcribe(result) => match *result {
                Ok(settings) => {
                    if let Some(language) = settings
                        .languages
                        .iter()
                        .find(|code| **code == self.language)
                        .cloned()
                    {
                        self.language = language;
                    }
                    self.transcribe = Phase::Ready(settings);
                }
                Err(error) => self.transcribe = Phase::Failed(error),
            },
            Msg::AutoArchive(result) => {
                self.auto_busy = false;
                match *result {
                    Ok(answer) => {
                        if self.show().is_some_and(|show| show.auto_archive) {
                            // The answer IS the count (`{**canal, "queued": n}`,
                            // `backfill.archive_show`): the SPA says how many
                            // episodes went into the queue, and a generic
                            // « Archivage lancé » next to a re-read was D11's
                            // workaround for a body `nz-api` used to drop.
                            self.notice(
                                ToastKind::Info,
                                match answer.queued {
                                    0 => "Archivage activé : rien à mettre en file".to_owned(),
                                    1 => "Archivage activé : 1 épisode en file".to_owned(),
                                    n => format!("Archivage activé : {n} épisodes en file"),
                                },
                                cx,
                            );
                            self.archiving = answer.queued > 0;
                            self.stalls = 0;
                            if self.archiving {
                                self.arm_poll(cx);
                            }
                        } else {
                            self.archiving = false;
                        }
                        // The channel payload comes with the count, so the
                        // switch and the counters are already right; no re-read.
                        if let Some(page) = self.page.ready().cloned()
                            && let Some(auto) = answer
                                .show
                                .get("auto_archive")
                                .and_then(|value| value.as_bool())
                        {
                            let mut show = page.show.clone();
                            show.auto_archive = auto;
                            self.page = Phase::Ready(PodcastPage { show, ..page });
                        }
                    }
                    Err(error) => {
                        if let Some(show) = self.show().cloned()
                            && let Some(page) = self.page.ready().cloned()
                        {
                            let mut show = show;
                            show.auto_archive = !show.auto_archive;
                            self.page = Phase::Ready(PodcastPage { show, ..page });
                        }
                        self.notice(
                            ToastKind::Error,
                            format!("Réglage non enregistré : {}", card::cause(&error)),
                            cx,
                        );
                    }
                }
            }
            Msg::ShowTranscribe(result) => {
                self.batch_busy = false;
                match *result {
                    Ok(answer) => {
                        if let Some(page) = self.page.ready().cloned() {
                            let mut show = page.show.clone();
                            if let Some(auto) = answer
                                .get("auto_transcribe")
                                .and_then(|value| value.as_bool())
                            {
                                show.auto_transcribe = auto;
                            }
                            if let Some(language) = answer
                                .get("transcribe_language")
                                .and_then(|value| value.as_str())
                                .filter(|code| !code.is_empty())
                            {
                                show.transcribe_language = Some(language.to_owned());
                                self.language = language.to_owned();
                            }
                            self.page = Phase::Ready(PodcastPage {
                                show,
                                episodes: page.episodes,
                                next: page.next,
                            });
                        }
                        let queued = answer
                            .get("queued")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0);
                        let skipped = answer
                            .get("skipped")
                            .map(|value| match value {
                                serde_json::Value::Number(number) => number.as_u64().unwrap_or(0),
                                serde_json::Value::Object(map) => {
                                    map.values().filter_map(|value| value.as_u64()).sum()
                                }
                                _ => 0,
                            })
                            .unwrap_or(0);
                        if queued > 0 {
                            self.notice(
                                ToastKind::Info,
                                format!(
                                    "{queued} épisode{} en file de transcription",
                                    plural(queued as usize)
                                ),
                                cx,
                            );
                            self.arm_poll(cx);
                        } else if skipped > 0 {
                            self.notice(
                                ToastKind::Error,
                                format!(
                                    "{skipped} épisode{} ignoré{} (trop long ou hors budget)",
                                    plural(skipped as usize),
                                    if skipped > 1 { "s" } else { "" }
                                ),
                                cx,
                            );
                        }
                        self.load(cx);
                    }
                    Err(error) => {
                        self.notice(ToastKind::Error, transcribe_cause(&error), cx);
                        self.load(cx);
                    }
                }
            }
            Msg::Batch {
                queued,
                skipped,
                cause,
            } => {
                self.batch_busy = false;
                if queued > 0 {
                    self.notice(
                        ToastKind::Info,
                        format!(
                            "{queued} épisode{} en file de transcription",
                            plural(queued as usize)
                        ),
                        cx,
                    );
                    self.arm_poll(cx);
                }
                if skipped > 0 {
                    self.notice(
                        ToastKind::Error,
                        match cause {
                            Some(cause) => format!(
                                "{skipped} épisode{} non envoyé{} : {cause}",
                                plural(skipped as usize),
                                if skipped > 1 { "s" } else { "" }
                            ),
                            None => format!(
                                "{skipped} épisode{} ignoré{}",
                                plural(skipped as usize),
                                if skipped > 1 { "s" } else { "" }
                            ),
                        },
                        cx,
                    );
                }
                if queued == 0 && skipped == 0 {
                    self.notice(ToastKind::Info, "Aucun épisode à transcrire", cx);
                }
            }
            Msg::MarkerAdded(episode, temporary, answer) => {
                self.markers.added(&episode, &temporary, *answer);
            }
            Msg::MarkerRemoved(marker, answer) => {
                if let Err(error) = *answer {
                    self.markers.restore(marker);
                    self.notice(
                        ToastKind::Error,
                        format!("Marqueur non supprimé : {}", card::cause(&error)),
                        cx,
                    );
                }
            }
            Msg::Transcript {
                episode,
                tone,
                answer,
            } => self.on_transcript(episode, tone, *answer, cx),
            Msg::TranscriptSearch {
                episode,
                term,
                answer,
            } => {
                if let Some(panel) = self.panel.as_mut()
                    && panel.key() == episode
                {
                    match *answer {
                        Ok(page) => panel.transcript.apply_search(page, &term),
                        Err(error) => {
                            panel.transcript.search.busy = false;
                            panel.transcript.search.error = Some(error);
                        }
                    }
                }
            }
            Msg::ShowSearch(result) => {
                self.search_loading = false;
                match *result {
                    Ok(answer) => {
                        self.search_hits = answer.hits;
                        self.search_total = answer.total.max(self.search_hits.len() as u32);
                        self.search_failed = None;
                    }
                    Err(error) => {
                        self.search_hits.clear();
                        self.search_total = 0;
                        self.search_failed = Some(error);
                    }
                }
            }
            Msg::Job {
                episode,
                done,
                answer,
            } => self.on_job(episode, done, *answer, cx),
            Msg::Cover(url, file) => self.covers.borrow_mut().resolved(&url, file),
        }
        cx.notify();
    }

    /// Adopt a page. `replace` is the first load; a poll refreshes the show's own
    /// facts and the episodes already on screen (it must not drop the pages
    /// "Charger plus" added).
    fn apply_page(&mut self, page: PodcastPage, replace: bool) {
        if replace {
            self.episodes = page.episodes.clone();
            self.next = page.next.clone();
            self.chapters.clear();
        } else {
            let fresh: BTreeMap<String, Episode> = page
                .episodes
                .iter()
                .map(|episode| (episode.id.as_str().to_owned(), episode.clone()))
                .collect();
            for episode in self.episodes.iter_mut() {
                if let Some(copy) = fresh.get(episode.id.as_str()) {
                    *episode = copy.clone();
                }
            }
            for episode in page.episodes.iter() {
                if !self.episodes.iter().any(|held| held.id == episode.id) {
                    self.episodes.push(episode.clone());
                }
            }
        }
        self.page = Phase::Ready(page);
        self.rebuild_sizes();
        if self.cursor >= self.episodes.len() {
            self.cursor = self.episodes.len().saturating_sub(1);
        }
    }

    fn show(&self) -> Option<&PodcastShow> {
        self.page.ready().map(|page| &page.show)
    }

    fn eligible(&self) -> usize {
        self.episodes
            .iter()
            .filter(|episode| {
                episode.archived
                    && episode.transcript.as_ref().is_none_or(|info| {
                        matches!(info.status.as_str(), "none" | "" | "error" | "stopped")
                    })
            })
            .count()
    }

    fn ready_transcripts(&self) -> usize {
        self.episodes
            .iter()
            .filter(|episode| {
                episode
                    .transcript
                    .as_ref()
                    .is_some_and(|info| info.status == "ready")
            })
            .count()
    }

    fn archived_episodes(&self) -> usize {
        self.episodes
            .iter()
            .filter(|episode| episode.archived)
            .count()
    }

    /// The episodes the provider still serves: one it dropped cannot be
    /// archived, so it does not hold the count back (the SPA's `reachable`).
    fn reachable(&self) -> u32 {
        let gone = self
            .episodes
            .iter()
            .filter(|episode| episode.unavailable && !episode.archived)
            .count() as u32;
        self.show()
            .map(|show| show.episode_count.saturating_sub(gone))
            .unwrap_or(0)
    }

    fn notice(
        &mut self,
        kind: ToastKind,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.state.update(cx, |state, cx| {
            state.toasts.push(kind, message.into());
            cx.notify();
        });
    }

    fn rebuild_sizes(&mut self) {
        if self.sizes.len() == self.episodes.len() {
            return;
        }
        self.sizes = Rc::new(vec![
            Size {
                width: px(0.),
                height: px(ROW_HEIGHT),
            };
            self.episodes.len()
        ]);
    }

    // -- transport ---------------------------------------------------------

    fn context(&self) -> Option<nz_player::Context> {
        let show = self.show()?;
        Some(nz_player::Context {
            kind: "podcast".to_owned(),
            id: Some(self.id.clone()),
            title: (!show.title.is_empty()).then(|| show.title.clone()),
        })
    }

    fn entries(&self) -> Vec<Entry> {
        self.episodes.iter().map(episode_panel::entry).collect()
    }

    fn command(&mut self, command: nz_player::Command, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.command(command, cx));
    }

    fn play_from(&mut self, index: usize, cx: &mut Context<Self>) {
        let entries = self.entries();
        if index >= entries.len() {
            return;
        }
        self.cursor = index;
        self.command(
            nz_player::Command::play_queue(entries, index, self.context()),
            cx,
        );
    }

    fn play_all(&mut self, cx: &mut Context<Self>) {
        if !self.episodes.is_empty() {
            self.play_from(0, cx);
        }
    }

    fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(episode) = self.episodes.get(index) else {
            return;
        };
        let key = episode_panel::key(episode);
        let current = self
            .state
            .read(cx)
            .player
            .current()
            .map(|entry| entry.id.clone());
        if current.as_deref() == Some(key.as_str()) {
            self.command(
                nz_player::Command::Toggle {
                    args: Default::default(),
                },
                cx,
            );
            return;
        }
        self.play_from(index, cx);
    }

    fn seek(&mut self, at: f64, cx: &mut Context<Self>) {
        self.command(
            nz_player::Command::Seek {
                args: nz_player::SeekArgs { t: at.max(0.0) },
            },
            cx,
        );
    }

    /// Play from `at` seconds of the episode at `index`: the playing one seeks,
    /// another one records the target as its resume point first (the SPA's
    /// `playAt`), so the load lands exactly there.
    fn play_at(&mut self, index: usize, at: f64, cx: &mut Context<Self>) {
        let Some(episode) = self.episodes.get(index).cloned() else {
            return;
        };
        let key = episode_panel::key(&episode);
        let current = self
            .state
            .read(cx)
            .player
            .current()
            .map(|entry| entry.id.clone());
        if current.as_deref() == Some(key.as_str()) {
            self.seek(at, cx);
            return;
        }
        set_resume(cx, &key, at, episode.duration);
        self.play_from(index, cx);
    }

    fn playhead_of(&self, episode: &Episode, cx: &App) -> f64 {
        let key = episode_panel::key(episode);
        let current = self
            .state
            .read(cx)
            .player
            .current()
            .map(|entry| entry.id.clone());
        if current.as_deref() == Some(key.as_str()) {
            self.state.read(cx).player.current_time()
        } else {
            self.positions
                .iter()
                .find(|(id, _)| id == &key)
                .map(|(_, position)| f64::from(position.position))
                .unwrap_or(0.0)
        }
    }

    // -- the panel ---------------------------------------------------------

    fn open_panel(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(episode) = self.episodes.get(index).cloned() else {
            return;
        };
        self.cursor = index;
        let at = self.playhead_of(&episode, cx);
        match self.panel.as_mut() {
            Some(panel) if panel.key() == episode.id.as_str() => {}
            Some(panel) => {
                *panel = EpisodePanel::new(episode.clone());
                panel.transcript.follow_to(at);
            }
            None => {
                let mut panel = EpisodePanel::new(episode.clone());
                panel.transcript.follow_to(at);
                self.panel = Some(panel);
            }
        }
        self.ensure_chapters(episode.id.as_str(), cx);
        self.fetch_transcript_at(at, Tone::User, cx);
        self.install_keys(cx);
        cx.notify();
    }

    fn close_panel(&mut self, cx: &mut Context<Self>) {
        self.panel = None;
        self.row_menu = None;
        self.keys = None;
        cx.notify();
    }

    fn ensure_chapters(&mut self, episode_id: &str, cx: &mut Context<Self>) {
        if !self.chapters.wants(episode_id) {
            return;
        }
        let snapshot = self.state.read(cx);
        let tx = self.events.clone();
        self.chapters
            .load(snapshot, episode_id, move |episode, answer| {
                tx.send(Msg::Chapters(episode, Box::new(answer)));
            });
    }

    /// Échap closes the menu, then the panel, before the shell goes back: an
    /// interceptor, because `CloseTop` is the shell's and would navigate.
    fn install_keys(&mut self, cx: &mut Context<Self>) {
        if self.keys.is_some() {
            return;
        }
        let weak = cx.weak_entity();
        let subscription = App::intercept_keystrokes(&mut *cx, move |event, _window, cx| {
            if event.keystroke.key != "escape" {
                return;
            }
            let _ = weak.update(cx, |this, cx| {
                if this.row_menu.is_some() {
                    this.row_menu = None;
                } else if this.panel.is_some() {
                    this.panel = None;
                } else {
                    return;
                }
                this.keys = None;
                cx.notify();
                cx.stop_propagation();
            });
        });
        self.keys = Some(subscription);
    }

    // -- the transcript ----------------------------------------------------

    fn on_transcript(
        &mut self,
        episode: String,
        tone: Tone,
        answer: Result<EpisodeTranscript, ApiError>,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.panel.as_mut() else {
            return;
        };
        if panel.key() != episode {
            return;
        }
        match answer {
            Ok(page) => {
                let ready_now = page.status == "ready" && !panel.transcript.loaded();
                panel.transcript.apply(page, tone);
                let status = panel.transcript.status.clone();
                let progress = panel.transcript.progress;
                let count = panel.transcript.total;
                if let Some(entry) = self
                    .episodes
                    .iter_mut()
                    .find(|entry| entry.id.as_str() == episode)
                {
                    match entry.transcript.as_mut() {
                        Some(info) => {
                            info.status = status.clone();
                            info.progress = progress;
                            if count > 0 {
                                info.segment_count = count;
                            }
                        }
                        None if status != "none" => {
                            entry.transcript = Some(EpisodeTranscriptInfo {
                                status: status.clone(),
                                progress,
                                segment_count: count,
                                ..Default::default()
                            });
                        }
                        None => {}
                    }
                }
                if transcript::is_active(&status) {
                    self.arm_poll(cx);
                } else if ready_now {
                    // The job finished while the panel was open: its text is
                    // here, so read the first page.
                    self.fetch_transcript_at(0.0, Tone::User, cx);
                }
            }
            Err(error) => panel.transcript.fail(error),
        }
        cx.notify();
    }

    fn fetch_transcript_at(&mut self, at: f64, tone: Tone, cx: &mut Context<Self>) {
        let (Some(api), runtime) = (
            self.state.read(cx).api.clone(),
            self.state.read(cx).runtime.clone(),
        ) else {
            return;
        };
        let Some(panel) = self.panel.as_mut() else {
            return;
        };
        let Some(transcript::Fetch::Page { path, .. }) = panel.transcript.open(Some(at)) else {
            return;
        };
        let episode = panel.key().to_owned();
        let tx = self.events.clone();
        let lane = if tone == Tone::Poll {
            Lane::Prefetch
        } else {
            Lane::User
        };
        runtime.spawn(async move {
            let answer = get::<EpisodeTranscript>(&api, lane, &path).await;
            tx.send(Msg::Transcript {
                episode,
                tone,
                answer: Box::new(answer),
            });
        });
    }

    fn fetch_transcript_offset(&mut self, offset: u32, cx: &mut Context<Self>) {
        let (Some(api), runtime) = (
            self.state.read(cx).api.clone(),
            self.state.read(cx).runtime.clone(),
        ) else {
            return;
        };
        let Some(panel) = self.panel.as_mut() else {
            return;
        };
        let transcript::Fetch::Page { path, .. } = panel.transcript.page_fetch(None, offset) else {
            return;
        };
        let episode = panel.key().to_owned();
        let tx = self.events.clone();
        runtime.spawn(async move {
            let answer = get::<EpisodeTranscript>(&api, Lane::User, &path).await;
            tx.send(Msg::Transcript {
                episode,
                tone: Tone::User,
                answer: Box::new(answer),
            });
        });
    }

    fn run_transcript_search(&mut self, term: String, cx: &mut Context<Self>) {
        let (Some(api), runtime) = (
            self.state.read(cx).api.clone(),
            self.state.read(cx).runtime.clone(),
        ) else {
            return;
        };
        let Some(panel) = self.panel.as_mut() else {
            return;
        };
        let Some(transcript::Fetch::Search { path, term }) = panel.transcript.search(&term) else {
            cx.notify();
            return;
        };
        let episode = panel.key().to_owned();
        let tx = self.events.clone();
        runtime.spawn(async move {
            let answer = get::<EpisodeTranscript>(&api, Lane::User, &path).await;
            tx.send(Msg::TranscriptSearch {
                episode,
                term,
                answer: Box::new(answer),
            });
        });
        cx.notify();
    }

    fn panel_index(&self) -> Option<usize> {
        let panel = self.panel.as_ref()?;
        self.episodes
            .iter()
            .position(|episode| episode.id.as_str() == panel.key())
    }

    /// One verb of the transcript panel.
    fn ask_transcript(&mut self, asked: TranscriptAsk, cx: &mut Context<Self>) {
        match asked {
            TranscriptAsk::OpenAt(at) => self.fetch_transcript_at(at, Tone::User, cx),
            TranscriptAsk::Retry => {
                let at = self
                    .panel
                    .as_ref()
                    .and_then(|panel| panel.transcript.segments.first().map(|line| line.start))
                    .unwrap_or(0.0);
                if let Some(panel) = self.panel.as_mut() {
                    panel.transcript.failed = None;
                    panel.transcript.offline = false;
                }
                self.fetch_transcript_at(at, Tone::User, cx);
            }
            TranscriptAsk::Page(delta) => {
                let Some(panel) = self.panel.as_ref() else {
                    return;
                };
                let limit = f64::from(panel.transcript.limit.max(1));
                let next = f64::from(panel.transcript.offset) + f64::from(delta) * limit;
                if next < 0.0 || next >= f64::from(panel.transcript.total.max(1)) {
                    return;
                }
                self.fetch_transcript_offset(next as u32, cx);
            }
            TranscriptAsk::Search => {
                let term = self.transcript_query.clone();
                self.run_transcript_search(term, cx);
            }
            TranscriptAsk::ClearSearch => {
                if let Some(panel) = self.panel.as_mut() {
                    panel.transcript.search.clear();
                    panel.transcript.resume_follow();
                }
                self.transcript_query.clear();
                self.transcript_pending = Some(String::new());
                cx.notify();
            }
            TranscriptAsk::PickHit(start) => {
                if let Some(index) = self.panel_index() {
                    self.play_at(index, start, cx);
                }
                self.fetch_transcript_at(start, Tone::User, cx);
            }
            TranscriptAsk::Seek(start) => {
                if let Some(index) = self.panel_index() {
                    self.play_at(index, start, cx);
                }
            }
            TranscriptAsk::Follow => {
                if let Some(panel) = self.panel.as_mut() {
                    panel.transcript.resume_follow();
                    let at = panel
                        .transcript
                        .segments
                        .first()
                        .map(|line| line.start)
                        .unwrap_or(0.0);
                    panel.transcript.follow_to(at);
                }
                cx.notify();
            }
            TranscriptAsk::Start => self.job(Job::Started, cx),
            TranscriptAsk::Stop => self.job(Job::Stopped, cx),
            TranscriptAsk::Relaunch => {
                if let Some(panel) = self.panel.as_mut() {
                    panel.transcript.confirm = Some(Confirm::Relaunch);
                }
                cx.notify();
            }
            TranscriptAsk::Delete => {
                if let Some(panel) = self.panel.as_mut() {
                    panel.transcript.confirm = Some(Confirm::Delete);
                }
                cx.notify();
            }
            TranscriptAsk::RelaunchConfirm => self.job(Job::Relaunched, cx),
            TranscriptAsk::DeleteConfirm => self.job(Job::Deleted, cx),
            TranscriptAsk::Cancel => {
                if let Some(panel) = self.panel.as_mut() {
                    panel.transcript.confirm = None;
                }
                cx.notify();
            }
            TranscriptAsk::Language(code) => {
                if let Some(panel) = self.panel.as_mut() {
                    panel.transcript.relaunch_language = code;
                }
                cx.notify();
            }
        }
    }

    fn job(&mut self, done: Job, cx: &mut Context<Self>) {
        let (Some(api), runtime) = (
            self.state.read(cx).api.clone(),
            self.state.read(cx).runtime.clone(),
        ) else {
            return;
        };
        let Some(panel) = self.panel.as_mut() else {
            return;
        };
        let episode = panel.key().to_owned();
        let language = panel.transcript.relaunch_language.clone();
        match done {
            Job::Started => {
                panel.transcript.status = "queued".to_owned();
                panel.transcript.progress = 0.0;
            }
            Job::Stopped => panel.transcript.status = "stopped".to_owned(),
            _ => {}
        }
        panel.transcript.confirm = None;
        let tx = self.events.clone();
        runtime.spawn(async move {
            let answer = match done {
                Job::Started => api
                    .start_episode_transcript(&episode, &serde_json::json!({}))
                    .await
                    .map(|_| ()),
                Job::Stopped => api.stop_episode_transcript(&episode).await,
                Job::Relaunched => api
                    .start_episode_transcript(
                        &episode,
                        &serde_json::json!({ "force": true, "language": language }),
                    )
                    .await
                    .map(|_| ()),
                Job::Deleted => api.delete_episode_transcript(&episode).await,
            };
            tx.send(Msg::Job {
                episode,
                done,
                answer: Box::new(answer),
            });
        });
        cx.notify();
    }

    fn on_job(
        &mut self,
        episode: String,
        done: Job,
        answer: Result<(), ApiError>,
        cx: &mut Context<Self>,
    ) {
        match answer {
            Ok(()) => {
                self.notice(
                    ToastKind::Info,
                    match done {
                        Job::Started => "Épisode en file de transcription",
                        Job::Stopped => "Transcription arrêtée",
                        Job::Relaunched => "Transcription relancée",
                        Job::Deleted => "Transcription supprimée",
                    },
                    cx,
                );
                if let Some(panel) = self.panel.as_mut()
                    && panel.key() == episode
                {
                    match done {
                        Job::Deleted => {
                            let archived = panel.episode.archived;
                            let duration = panel.episode.duration;
                            panel.transcript =
                                transcript::Panel::new(&episode, Some(archived), duration);
                        }
                        Job::Relaunched => {
                            panel.transcript.segments.clear();
                            panel.transcript.total = 0;
                            panel.transcript.offset = 0;
                            panel.transcript.status = "queued".to_owned();
                            panel.transcript.progress = 0.0;
                        }
                        _ => {}
                    }
                }
                if matches!(done, Job::Started | Job::Relaunched) {
                    self.arm_poll(cx);
                }
                self.load(cx);
            }
            Err(error) => {
                self.notice(ToastKind::Error, transcribe_cause(&error), cx);
                // The server is the truth about a job: read the panel's state
                // again instead of guessing what it refused.
                let at = self
                    .panel
                    .as_ref()
                    .and_then(|panel| panel.transcript.segments.first().map(|line| line.start))
                    .unwrap_or(0.0);
                self.fetch_transcript_at(at, Tone::Poll, cx);
            }
        }
    }

    // -- the show's switches ----------------------------------------------

    fn toggle_auto_archive(&mut self, cx: &mut Context<Self>) {
        if self.auto_busy {
            return;
        }
        let Some(show) = self.show().cloned() else {
            return;
        };
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.auto_busy = true;
        let on = !show.auto_archive;
        if let Some(page) = self.page.ready().cloned() {
            let mut show = page.show.clone();
            show.auto_archive = on;
            self.page = Phase::Ready(PodcastPage { show, ..page });
        }
        let tx = self.events.clone();
        let id = self.id.clone();
        self.state.read(cx).runtime.spawn(async move {
            let answer = api.set_podcast_auto_archive(&id, on).await;
            tx.send(Msg::AutoArchive(Box::new(answer)));
        });
        cx.notify();
    }

    fn set_language(&mut self, language: String, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.language = language.clone();
        let tx = self.events.clone();
        let id = self.id.clone();
        self.state.read(cx).runtime.spawn(async move {
            let answer = api
                .set_show_transcribe(&id, &serde_json::json!({ "language": language }))
                .await;
            tx.send(Msg::ShowTranscribe(Box::new(answer)));
        });
        cx.notify();
    }

    fn toggle_auto_transcribe(&mut self, cx: &mut Context<Self>) {
        if self.batch_busy {
            return;
        }
        let Some(show) = self.show().cloned() else {
            return;
        };
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.batch_busy = true;
        let on = !show.auto_transcribe;
        let payload = if on {
            serde_json::json!({
                "auto": true,
                "backlog": self.backlog.wire(),
                "language": self.language.clone(),
            })
        } else {
            serde_json::json!({ "auto": false })
        };
        let tx = self.events.clone();
        let id = self.id.clone();
        self.state.read(cx).runtime.spawn(async move {
            let answer = api.set_show_transcribe(&id, &payload).await;
            tx.send(Msg::ShowTranscribe(Box::new(answer)));
        });
        cx.notify();
    }

    /// "Transcrire les N épisodes archivés les plus récents": one POST at a time,
    /// stopping on the refusal that will refuse the rest too (rule 3).
    fn start_batch(&mut self, cx: &mut Context<Self>) {
        if self.batch_busy {
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        let wanted: Vec<String> = self
            .episodes
            .iter()
            .filter(|episode| {
                episode.archived
                    && episode.transcript.as_ref().is_none_or(|info| {
                        matches!(info.status.as_str(), "none" | "" | "error" | "stopped")
                    })
            })
            .take(self.batch.max(1))
            .map(|episode| episode.id.as_str().to_owned())
            .collect();
        if wanted.is_empty() {
            self.notice(ToastKind::Info, "Aucun épisode à transcrire", cx);
            return;
        }
        self.batch_busy = true;
        let tx = self.events.clone();
        self.state.read(cx).runtime.spawn(async move {
            let mut queued = 0u32;
            let mut skipped = 0u32;
            let mut cause: Option<SharedString> = None;
            for episode in wanted {
                match api
                    .start_episode_transcript(&episode, &serde_json::json!({}))
                    .await
                {
                    Ok(_) => queued += 1,
                    Err(error) => {
                        skipped += 1;
                        cause.get_or_insert(transcribe_cause(&error));
                        if error.retry_after().is_some() {
                            break;
                        }
                    }
                }
            }
            tx.send(Msg::Batch {
                queued,
                skipped,
                cause,
            });
        });
        cx.notify();
    }

    // -- the show's transcript search -------------------------------------

    fn typed_show_search(&mut self, cx: &mut Context<Self>) {
        let term = self
            .search_field
            .as_ref()
            .map(|field| field.read(cx).value().to_string())
            .unwrap_or_default();
        self.search_query = term.clone();
        let trimmed = term.trim().to_owned();
        if trimmed.chars().count() < MIN_QUERY {
            self.search_pending = None;
            self.search_term.clear();
            self.search_hits.clear();
            self.search_total = 0;
            self.search_failed = None;
            self.search_loading = false;
            cx.notify();
            return;
        }
        self.search_pending = Some(trimmed.clone());
        self.search_loading = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(SEARCH_DEBOUNCE_MS))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.search_pending.as_deref() == Some(trimmed.as_str()) {
                    this.run_show_search(trimmed, cx);
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn run_show_search(&mut self, term: String, cx: &mut Context<Self>) {
        let (Some(api), runtime) = (
            self.state.read(cx).api.clone(),
            self.state.read(cx).runtime.clone(),
        ) else {
            return;
        };
        self.search_term = term.clone();
        self.search_loading = true;
        self.search_failed = None;
        let id = self.id.clone();
        let tx = self.events.clone();
        runtime.spawn(async move {
            let answer = api.show_transcript_search(&id, &term, 50).await;
            tx.send(Msg::ShowSearch(Box::new(answer)));
        });
    }

    fn clear_show_search(&mut self, cx: &mut Context<Self>) {
        self.search_query.clear();
        self.search_pending = None;
        self.search_loading = false;
        self.search_term.clear();
        self.search_hits.clear();
        self.search_total = 0;
        self.search_failed = None;
        // `set_value` needs a window: the next paint applies it (D8b's pitfall).
        self.search_cleared = true;
        cx.notify();
    }

    /// A hit: play its episode at that moment and open the panel there. A hit for
    /// an episode of a page not loaded yet still plays — the hit carries enough
    /// (`lib/transcript.js#hitEpisode`) — and the panel follows.
    fn play_hit(&mut self, hit: TranscriptHit, cx: &mut Context<Self>) {
        let Some(episode_id) = hit
            .episode_id
            .as_ref()
            .map(|id| id.as_str().to_owned())
            .or_else(|| {
                hit.episode
                    .as_ref()
                    .map(|episode| episode.id.as_str().to_owned())
            })
        else {
            return;
        };
        if let Some(index) = self
            .episodes
            .iter()
            .position(|episode| episode.id.as_str() == episode_id)
        {
            self.play_at(index, hit.start, cx);
            self.open_panel(index, cx);
            if let Some(panel) = self.panel.as_mut() {
                panel.transcript.pause_follow();
                panel.transcript.follow_to(hit.start);
            }
            self.fetch_transcript_at(hit.start, Tone::User, cx);
            return;
        }
        let title = hit
            .episode
            .as_ref()
            .map(|episode| episode.title.clone())
            .unwrap_or_default();
        let episode = Episode {
            id: nz_model::ids::EpisodeId::new(episode_id.clone()),
            title,
            channel_id: nz_model::ids::ChannelId::new(self.id.clone()),
            archived: true,
            podcast: true,
            transcript: Some(EpisodeTranscriptInfo {
                status: "ready".into(),
                progress: 1.0,
                ..Default::default()
            }),
            album: self.show().map(|show| nz_model::EpisodeAlbum {
                deezer_id: self.id.clone(),
                title: show.title.clone(),
                cover: show.cover.clone(),
            }),
            ..Episode::default()
        };
        set_resume(cx, &episode_id, hit.start, episode.duration);
        self.command(
            nz_player::Command::play_queue(vec![episode_panel::entry(&episode)], 0, self.context()),
            cx,
        );
        let mut panel = EpisodePanel::new(episode);
        panel.transcript.pause_follow();
        panel.transcript.follow_to(hit.start);
        self.panel = Some(panel);
        self.install_keys(cx);
        self.fetch_transcript_at(hit.start, Tone::User, cx);
        self.ensure_chapters(&episode_id, cx);
    }

    // -- rows ---------------------------------------------------------------

    fn on_intent(&mut self, intent: Intent, _window: &mut Window, cx: &mut Context<Self>) {
        let page = (track_list::TrackList::page_rows(px(800.)) / 4).max(1);
        match intent {
            Intent::Move(delta) => {
                let next = if delta < 0 {
                    self.cursor.saturating_sub(delta.unsigned_abs() as usize)
                } else {
                    self.cursor + delta as usize
                };
                self.select(next, cx);
            }
            Intent::First => self.select(0, cx),
            Intent::Last => self.select(self.episodes.len().saturating_sub(1), cx),
            Intent::Page(delta) => {
                let next = if delta < 0 {
                    self.cursor.saturating_sub(page)
                } else {
                    self.cursor + page
                };
                self.select(next, cx);
            }
            Intent::PlayFrom(at) => {
                let index = at.unwrap_or(self.cursor);
                self.toggle(index, cx);
                self.open_panel(index, cx);
            }
            Intent::PlayNextFrom(at) => {
                let index = at.unwrap_or(self.cursor);
                if let Some(entry) = self.episodes.get(index) {
                    self.command(
                        nz_player::Command::play_next(vec![episode_panel::entry(entry)]),
                        cx,
                    );
                }
            }
            Intent::QueueFrom(at) => {
                let index = at.unwrap_or(self.cursor);
                if let Some(entry) = self.episodes.get(index) {
                    self.command(
                        nz_player::Command::add_to_queue(vec![episode_panel::entry(entry)]),
                        cx,
                    );
                }
            }
            Intent::Menu(at) => {
                let index = at.unwrap_or(self.cursor);
                if index < self.episodes.len() {
                    self.cursor = index;
                    self.row_menu = Some(RowMenu { index, cursor: 0 });
                    self.install_keys(cx);
                }
            }
            // An episode is not a favourite of its own here: the show is what a
            // person keeps, and its switch is on the page.
            Intent::Like(_) | Intent::Dislike(_) => {}
        }
        cx.notify();
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.episodes.is_empty() {
            return;
        }
        let index = index.min(self.episodes.len() - 1);
        self.cursor = index;
        self.scroll
            .scroll_to_item(index, gpui_kit::ScrollStrategy::Center);
        cx.notify();
    }

    fn pick_row(&mut self, item: RowItem, cx: &mut Context<Self>) {
        let Some(menu) = self.row_menu.take() else {
            return;
        };
        let index = menu.index;
        let Some(episode) = self.episodes.get(index).cloned() else {
            return;
        };
        match item {
            RowItem::Play => {
                self.toggle(index, cx);
                self.open_panel(index, cx);
            }
            RowItem::PlayNext => self.command(
                nz_player::Command::play_next(vec![episode_panel::entry(&episode)]),
                cx,
            ),
            RowItem::Queue => self.command(
                nz_player::Command::add_to_queue(vec![episode_panel::entry(&episode)]),
                cx,
            ),
            RowItem::Download => self.download(index, cx),
            RowItem::Marker => {
                let at = self.playhead_of(&episode, cx);
                let key = episode_panel::key(&episode);
                self.add_marker(&key, markers::build_position(at), cx);
            }
            RowItem::Details => self.open_panel(index, cx),
        }
        cx.notify();
    }

    fn on_menu_key(&mut self, event: &gpui_kit::KeystrokeEvent, cx: &mut Context<Self>) {
        let Some(menu) = self.row_menu.as_mut() else {
            return;
        };
        match event.keystroke.key.as_str() {
            "up" | "k" => {
                menu.cursor = menu.cursor.saturating_sub(1);
                cx.notify();
            }
            "down" | "j" => {
                menu.cursor = (menu.cursor + 1).min(RowItem::ALL.len() - 1);
                cx.notify();
            }
            "enter" => {
                let item = RowItem::ALL[menu.cursor.min(RowItem::ALL.len() - 1)];
                self.pick_row(item, cx);
            }
            _ => {}
        }
    }

    // --- downloads (lot D18) ------------------------------------------------

    /// The keys of the episodes this page holds (page one plus « Charger
    /// plus »): what the show's download chooses.
    fn download_keys(&self) -> Vec<String> {
        self.episodes
            .iter()
            .map(episode_panel::key)
            .filter(|key| !key.is_empty())
            .collect()
    }

    /// The show's own download: one set for the episodes loaded here.
    fn show_download_state(&mut self) -> DownloadState {
        let keys = self.download_keys();
        download_control::collection(self.downloads.get(), "show", &self.id, &keys)
    }

    fn bind_download(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let for_download = weak.clone();
        gpui_kit::App::on_action(cx, move |_: &DownloadSelection, cx: &mut gpui_kit::App| {
            for_download
                .update(cx, |this, cx| {
                    if download_control::is_front(&this.state, &this.route, cx) {
                        this.download_all(cx);
                    }
                })
                .ok();
        });
        gpui_kit::App::on_action(cx, move |_: &RemoveDownload, cx: &mut gpui_kit::App| {
            weak.update(cx, |this, cx| {
                if download_control::is_front(&this.state, &this.route, cx) {
                    this.drop_show_download(cx);
                }
            })
            .ok();
        });
    }

    /// « Retirer » for the whole show: the set goes, files and all (a track
    /// another set wants stays, the store's own rule).
    fn drop_show_download(&mut self, cx: &mut Context<Self>) {
        let Some(service) = self.downloads.service().cloned() else {
            return;
        };
        match self.show_download_state() {
            DownloadState::Failed { failed, .. } => {
                service.retry_failures();
                self.notice(
                    ToastKind::Info,
                    format!(
                        "Nouvelle tentative pour {failed} épisode{}",
                        if failed > 1 { "s" } else { "" }
                    ),
                    cx,
                );
            }
            DownloadState::Queued { .. }
            | DownloadState::Running { .. }
            | DownloadState::Done { .. } => {
                service.drop_set("show", &self.id);
                self.notice(ToastKind::Info, "Téléchargement de l'émission retiré", cx);
            }
            DownloadState::Idle => {
                self.notice(
                    ToastKind::Info,
                    "Cette émission n'est pas téléchargée sur cet appareil.",
                    cx,
                );
            }
        }
        cx.notify();
    }

    fn download(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(episode) = self.episodes.get(index).cloned() else {
            return;
        };
        let Some(service) = crate::wiring::offline::attach(self.state.read(cx)) else {
            self.notice(
                ToastKind::Error,
                "Les téléchargements ne sont pas disponibles sans stockage local.",
                cx,
            );
            return;
        };
        let key = episode_panel::key(&episode);
        let title = episode.title.clone();
        // One episode is a set of one: the worker dedupes it, the downloads
        // screen lists it and can remove it like a playlist's. The verb is the
        // episode's own state, so a second press takes it back.
        if let Some(message) =
            download_control::row_verb(&service, self.downloads.get(), &key, &title)
        {
            self.notice(ToastKind::Info, message, cx);
        }
        cx.notify();
    }

    fn download_all(&mut self, cx: &mut Context<Self>) {
        if self.download_busy || self.episodes.is_empty() {
            return;
        }
        if let DownloadState::Failed { failed, .. } = self.show_download_state() {
            if let Some(service) = self.downloads.service().cloned() {
                service.retry_failures();
                self.notice(
                    ToastKind::Info,
                    format!(
                        "Nouvelle tentative pour {failed} épisode{}",
                        if failed > 1 { "s" } else { "" }
                    ),
                    cx,
                );
            }
            cx.notify();
            return;
        }
        let Some(service) = crate::wiring::offline::attach(self.state.read(cx)) else {
            self.notice(
                ToastKind::Error,
                "Les téléchargements ne sont pas disponibles sans stockage local.",
                cx,
            );
            return;
        };
        let title = self
            .show()
            .map(|show| show.title.clone())
            .unwrap_or_default();
        let choices: Vec<_> = self
            .episodes
            .iter()
            .map(|episode| {
                crate::wiring::offline::Choice::new(
                    episode_panel::key(episode),
                    episode.title.clone(),
                )
            })
            .collect();
        let count = choices.len();
        let show_id = self.id.clone();
        service.choose("show", &show_id, &title, choices);
        self.notice(
            ToastKind::Info,
            format!("{count} épisode{} en file de téléchargement", plural(count)),
            cx,
        );
        cx.notify();
    }

    fn add_marker(&mut self, episode: &str, position_ms: u32, cx: &mut Context<Self>) {
        let snapshot = self.state.read(cx);
        let tx = self.events.clone();
        let episode = episode.to_owned();
        if self
            .markers
            .add(
                snapshot,
                &episode,
                position_ms,
                None,
                move |episode, temporary, answer| {
                    tx.send(Msg::MarkerAdded(episode, temporary, Box::new(answer)));
                },
            )
            .is_some()
        {
            self.notice(
                ToastKind::Info,
                format!("Marqueur ajouté à {}", markers::format(position_ms)),
                cx,
            );
        }
    }

    fn remove_marker(&mut self, marker: Marker, cx: &mut Context<Self>) {
        let tx = self.events.clone();
        self.markers
            .remove(self.state.read(cx), &marker, move |marker, answer| {
                tx.send(Msg::MarkerRemoved(marker, Box::new(answer)));
            });
        cx.notify();
    }

    fn unsubscribe(&mut self, cx: &mut Context<Self>) {
        if !self.unsubscribe_confirm {
            self.unsubscribe_confirm = true;
            cx.notify();
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        let id = self.id.clone();
        let title = self
            .show()
            .map(|show| show.title.clone())
            .unwrap_or_default();
        let tx = self.events.clone();
        self.state.read(cx).runtime.spawn(async move {
            if let Err(error) = api.unsubscribe_podcast(&id).await {
                tx.send(Msg::ShowSearch(Box::new(Err(error))));
            }
        });
        self.notice(ToastKind::Info, format!("Désabonné de « {title} »"), cx);
        self.state
            .update(cx, |state, _| state.nav.push(Route::Podcasts));
    }

    fn covers_for(&mut self, urls: Vec<String>, cx: &mut Context<Self>) {
        let snapshot = self.state.read(cx);
        let mut covers = self.covers.borrow_mut();
        for url in urls {
            let for_request = url.clone();
            let for_message = url.clone();
            let tx = self.events.clone();
            covers.want(snapshot, &for_request, move |file| {
                tx.send(Msg::Cover(for_message, file))
            });
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.page = Phase::Loading;
        self.more_failed = None;
        self.covers.borrow_mut().retry_failed();
        self.load(cx);
        if self.is_admin(cx) {
            self.load_transcribe_settings(cx);
        }
        self.load_markers(cx);
        cx.notify();
    }

    /// The row a show's speed is stored under: `podcast.speeds` is keyed by
    /// `channel_id`, and a show's own id is that key.
    fn speed_entry(&self) -> Entry {
        Entry {
            id: self.id.clone(),
            title: self
                .show()
                .map(|show| show.title.clone())
                .unwrap_or_default(),
            podcast: Some(true),
            channel_id: Some(self.id.clone()),
            ..Entry::default()
        }
    }

    fn speed(&self, cx: &App) -> f64 {
        speed::speed_for(&self.speed_entry(), &speed::stored(self.state.read(cx)))
    }

    fn set_speed(&mut self, rate: f64, cx: &mut Context<Self>) {
        let entry = self.speed_entry();
        self.state.update(cx, |state, cx| {
            speed::set_rate(state, &entry, rate);
            let current = state.player.current().cloned();
            if let Some(current) = current.filter(|current| current.is_podcast()) {
                speed::apply(state, Some(&current));
            }
            state
                .toasts
                .push(ToastKind::Info, format!("Vitesse : {}", speed::label(rate)));
            cx.notify();
        });
        self.speed_open = false;
        cx.notify();
    }

    // -- rendering helpers --------------------------------------------------

    fn header(&self, show: &PodcastShow, compact: bool, cx: &mut Context<Self>) -> AnyElement {
        let cover_file = show
            .cover
            .as_deref()
            .and_then(|url| cover::file_in(&self.covers, url));
        let facts = card::facts(&[
            "Podcast".to_owned(),
            if show.local {
                "Local".to_owned()
            } else {
                String::new()
            },
            format!(
                "{} épisode{}",
                show.episode_count,
                if show.episode_count > 1 { "s" } else { "" }
            ),
            format!("{} archivés", show.archived_count),
        ]);
        card::hero_band(cx)
            .child(cover::view(
                cover_file.as_ref(),
                show.cover.as_deref(),
                &show.title,
                Kind::Podcast,
                if compact {
                    px(96.)
                } else {
                    card::hero_sleeve()
                },
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.375))
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::secondary(cx))
                            .child(facts),
                    )
                    .child(
                        div()
                            .text_size(rems(if compact { 1.5 } else { 2. }))
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .child(show.title.clone()),
                    )
                    .when_some(
                        show.description
                            .clone()
                            .filter(|text| !text.trim().is_empty() && !compact),
                        |column, description| {
                            column.child(
                                div()
                                    .max_w(rems(56.))
                                    .max_h(rems(3.))
                                    .overflow_hidden()
                                    .text_size(rems(0.75))
                                    .line_height(rems(1.125))
                                    .text_color(theme::secondary(cx))
                                    .child(description),
                            )
                        },
                    )
                    .when(show.local, |column| {
                        column.child(
                            div()
                                .max_w(rems(56.))
                                .text_size(rems(0.6875))
                                .text_color(theme::warning(cx))
                                .child(
                                    "Cette émission n'existe plus chez le fournisseur : tout ce \
                                     qui est archivé reste lisible, jouable et téléchargeable.",
                                ),
                        )
                    }),
            )
            .into_any_element()
    }

    fn actions(&mut self, show: &PodcastShow, cx: &mut Context<Self>) -> AnyElement {
        let download_state = self.show_download_state();
        let is_admin = self.is_admin(cx);
        let rate = self.speed(cx);
        let mut row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rems(0.5))
            .child(
                button(
                    "show-play",
                    if self.episodes.is_empty() {
                        "Aucun épisode"
                    } else {
                        "Lire"
                    },
                    ButtonVariant::Primary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.play_all(cx))),
            )
            // No session or no store: the control is absent rather than a door
            // that does not open (rule 2).
            .children(download_control::control(
                "show-download",
                &download_state,
                self.downloads.available(),
                cx,
            ))
            .child({
                let open = self.speed_open;
                div()
                    .id("show-speed")
                    .flex()
                    .items_center()
                    .gap(rems(0.25))
                    .h(rems(2.))
                    .px(rems(0.5))
                    .border_1()
                    .border_color(if (rate - 1.0).abs() > f64::EPSILON || open {
                        theme::accent(cx)
                    } else {
                        theme::border(cx)
                    })
                    .text_size(rems(0.75))
                    .text_color(theme::text(cx))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::chrome(cx)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.speed_open = !this.speed_open;
                        cx.notify();
                    }))
                    .child(icon(IconName::Rabbit).size(rems(0.875)))
                    .child(SharedString::from(format!(
                        "Vitesse {}",
                        speed::label(rate)
                    )))
            });
        if is_admin {
            row = row.child(
                button(
                    "show-unsubscribe",
                    if self.unsubscribe_confirm {
                        "Confirmer le désabonnement"
                    } else {
                        "Se désabonner"
                    },
                    if self.unsubscribe_confirm {
                        ButtonVariant::Danger
                    } else {
                        ButtonVariant::Secondary
                    },
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.unsubscribe(cx))),
            );
        }
        let _ = show;
        let mut column = div()
            .flex()
            .flex_col()
            .gap(rems(0.25))
            .px(rems(1.))
            .py(rems(0.75))
            .child(row);
        if self.speed_open {
            let mut menu = div()
                .id("show-speed-menu")
                .flex()
                .flex_col()
                .gap(rems(0.0625))
                .p(rems(0.25))
                .w(rems(12.))
                .border_1()
                .border_color(theme::border(cx))
                .bg(theme::chrome(cx))
                .shadow_lg();
            for (at, option) in speed::SPEEDS.iter().enumerate() {
                let selected = (option - rate).abs() < 1e-9;
                let held = *option;
                menu = menu.child(
                    div()
                        .id(("show-speed-rate", at))
                        .debug_selector(move || format!("show-speed-rate-{at}"))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(rems(0.5))
                        .h(rems(1.75))
                        .px(rems(0.375))
                        .text_size(rems(0.75))
                        .text_color(if selected {
                            theme::accent(cx)
                        } else {
                            theme::text(cx)
                        })
                        .cursor_pointer()
                        .hover(|style| style.bg(theme::selected(cx)))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_speed(held, cx)))
                        .child(if (held - 1.0).abs() < f64::EPSILON {
                            "Normale".to_owned()
                        } else {
                            speed::label(held)
                        })
                        .when(selected, |line| {
                            line.child(icon(IconName::Check).size(rems(0.75)))
                        }),
                );
            }
            column = column.child(menu);
        }
        column.into_any_element()
    }

    fn switches(&self, show: &PodcastShow, open: bool, cx: &mut Context<Self>) -> AnyElement {
        let is_admin = self.is_admin(cx);
        if !is_admin {
            return div().into_any_element();
        }
        let ready = self.ready_transcripts();
        let archived = self.archived_episodes();
        let mut summary = Vec::new();
        if !show.local {
            summary.push(format!(
                "Archivage auto {}",
                if show.auto_archive {
                    "activé"
                } else {
                    "désactivé"
                }
            ));
        }
        summary.push(format!(
            "{ready} sur {archived} transcrit{}",
            plural(archived)
        ));
        let mut column = div()
            .id("show-settings")
            .debug_selector(|| "show-settings".to_owned())
            .flex()
            .flex_col()
            .gap(rems(0.375))
            .px(rems(1.))
            .pb(rems(0.5));
        // One row that says what is folded inside, and opens it: the head may
        // not spend 220 px on it at 1280×800 (`head_fit`).
        column = column.child(
            div()
                .id("show-settings-row")
                .debug_selector(|| "show-settings-row".to_owned())
                .flex()
                .items_center()
                .gap(rems(0.375))
                .h(rems(2.125))
                .cursor_pointer()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .hover(|style| style.text_color(theme::text(cx)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.settings_open = Some(!this.settings_shown);
                    cx.notify();
                }))
                .child(
                    icon(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(rems(0.875)),
                )
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Réglages de l'émission"),
                )
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .child(SharedString::from(summary.join(" · "))),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(rems(0.6875))
                        .child(if open { "Replier" } else { "Ouvrir" }),
                ),
        );
        if !open {
            return column.into_any_element();
        }
        if !show.local {
            let archived = show.archived_count.min(show.episode_count);
            column = column.child(
                switch(
                    "show-auto-archive",
                    "Archiver automatiquement les nouveaux épisodes",
                    show.auto_archive,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_auto_archive(cx))),
            );
            column = column.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.375))
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(SharedString::from(format!(
                        "{} sur {} épisode{} archivé{} sur le serveur{}",
                        archived,
                        show.episode_count,
                        if show.episode_count > 1 { "s" } else { "" },
                        if archived > 1 { "s" } else { "" },
                        if self.archiving { " · archivage" } else { "" }
                    )))
                    .when(show.auto_archive && show.episode_count > 0, |row| {
                        row.child(bar(
                            archived as f32 / show.episode_count as f32,
                            theme::accent(cx),
                            cx,
                        ))
                    }),
            );
        }
        column = column.child(self.transcribe_card(cx));
        column.into_any_element()
    }

    fn transcribe_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut card = div()
            .id("show-transcribe")
            .flex()
            .flex_col()
            .gap(rems(0.375))
            .p(rems(0.625))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx));
        let ready = self.ready_transcripts();
        let archived = self.archived_episodes();
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.375))
                .child(
                    icon(IconName::ScrollText)
                        .size(rems(0.875))
                        .text_color(theme::secondary(cx)),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(rems(0.8125))
                        .font_weight(FontWeight::BOLD)
                        .child("Transcription des épisodes"),
                )
                .child(
                    div()
                        .text_size(rems(0.6875))
                        .font_family(theme::mono(cx))
                        .text_color(theme::secondary(cx))
                        .child(format!("{ready} sur {archived}")),
                ),
        );
        match &self.transcribe {
            Phase::Loading => {
                card = card.child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .child("Vérification de la transcription…"),
                );
            }
            Phase::Failed(error) => {
                card = card.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.375))
                        .child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::danger(cx))
                                .child(SharedString::from(format!(
                                    "Impossible de vérifier la transcription : {}",
                                    card::cause(error)
                                ))),
                        )
                        .child(
                            button(
                                "show-transcribe-retry",
                                "Réessayer",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(
                                cx.listener(|this, _, _, cx| this.load_transcribe_settings(cx)),
                            ),
                        ),
                );
            }
            Phase::Ready(settings) => {
                if !settings.available {
                    let reason = settings
                        .reason
                        .as_deref()
                        .map(transcript::reason_text)
                        .unwrap_or_else(|| transcript::reason_text(""));
                    card = card.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.375))
                            .child(
                                div()
                                    .text_size(rems(0.6875))
                                    .text_color(theme::warning(cx))
                                    .child(SharedString::from(reason)),
                            )
                            .child(
                                button(
                                    "show-transcribe-settings",
                                    "Ouvrir Réglages",
                                    ButtonVariant::Secondary,
                                    cx,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.state.update(cx, |state, _| {
                                            state.nav.push(Route::Settings {
                                                section: crate::nav::SettingsSection::Server,
                                            })
                                        })
                                    },
                                )),
                            ),
                    );
                } else {
                    let auto = self.show().is_some_and(|show| show.auto_transcribe);
                    card = card.child(
                        switch(
                            "show-auto-transcribe",
                            "Transcrire automatiquement les nouveaux épisodes",
                            auto,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_auto_transcribe(cx))),
                    );
                    let mut languages = div().flex().flex_wrap().items_center().gap(rems(0.25));
                    languages = languages.child(
                        div()
                            .text_size(rems(0.6875))
                            .text_color(theme::secondary(cx))
                            .child("Langue"),
                    );
                    let offered: Vec<String> = if settings.languages.is_empty() {
                        transcript::LANGUAGES
                            .iter()
                            .map(|code| (*code).to_owned())
                            .collect()
                    } else {
                        settings.languages.clone()
                    };
                    for (at, code) in offered.iter().enumerate() {
                        let selected = self.language == *code;
                        let asked = code.clone();
                        languages = languages.child(
                            div()
                                .id(("transcribe-language", at))
                                .debug_selector(move || format!("transcribe-language-{at}"))
                                .flex()
                                .items_center()
                                .h(rems(1.5))
                                .px(rems(0.375))
                                .border_1()
                                .border_color(if selected {
                                    theme::accent(cx)
                                } else {
                                    theme::border(cx)
                                })
                                .text_size(rems(0.6875))
                                .text_color(if selected {
                                    theme::accent(cx)
                                } else {
                                    theme::secondary(cx)
                                })
                                .cursor_pointer()
                                .hover(|style| style.text_color(theme::text(cx)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_language(asked.clone(), cx)
                                }))
                                .child(transcript::language_label(code)),
                        );
                    }
                    card = card.child(languages);

                    let mut backlog = div().flex().flex_wrap().items_center().gap(rems(0.25));
                    backlog = backlog.child(
                        div()
                            .text_size(rems(0.6875))
                            .text_color(theme::secondary(cx))
                            .child("Rattrapage"),
                    );
                    for (at, mode) in Backlog::ALL.iter().enumerate() {
                        let selected = self.backlog == *mode;
                        let held = *mode;
                        backlog = backlog.child(
                            div()
                                .id(("transcribe-backlog", at))
                                .debug_selector(move || format!("transcribe-backlog-{at}"))
                                .flex()
                                .items_center()
                                .h(rems(1.5))
                                .px(rems(0.375))
                                .border_1()
                                .border_color(if selected {
                                    theme::accent(cx)
                                } else {
                                    theme::border(cx)
                                })
                                .text_size(rems(0.6875))
                                .text_color(if selected {
                                    theme::accent(cx)
                                } else {
                                    theme::secondary(cx)
                                })
                                .cursor_pointer()
                                .hover(|style| style.text_color(theme::text(cx)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.backlog = held;
                                    cx.notify();
                                }))
                                .child(mode.label()),
                        );
                    }
                    card = card.child(backlog);

                    let eligible = self.eligible();
                    if eligible > 0 {
                        let choices = batch_choices(eligible);
                        let mut row = div().flex().items_center().gap(rems(0.375)).child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::secondary(cx))
                                .child("Transcrire les"),
                        );
                        for (at, count) in choices.iter().enumerate() {
                            let selected = self.batch == *count;
                            let held = *count;
                            row = row.child(
                                div()
                                    .id(("batch-count", at))
                                    .debug_selector(move || format!("batch-count-{at}"))
                                    .flex()
                                    .items_center()
                                    .h(rems(1.5))
                                    .px(rems(0.375))
                                    .border_1()
                                    .border_color(if selected {
                                        theme::accent(cx)
                                    } else {
                                        theme::border(cx)
                                    })
                                    .text_size(rems(0.6875))
                                    .text_color(if selected {
                                        theme::accent(cx)
                                    } else {
                                        theme::secondary(cx)
                                    })
                                    .cursor_pointer()
                                    .hover(|style| style.text_color(theme::text(cx)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.batch = held;
                                        cx.notify();
                                    }))
                                    .child(held.to_string()),
                            );
                        }
                        row = row.child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::secondary(cx))
                                .child(SharedString::from(format!(
                                    "épisode{} archivé{} les plus récents",
                                    plural(eligible),
                                    if eligible > 1 { "s" } else { "" }
                                ))),
                        );
                        row = row.child(
                            button(
                                "show-transcribe-batch",
                                if self.batch_busy {
                                    "Envoi…"
                                } else {
                                    "Transcrire"
                                },
                                ButtonVariant::Outline,
                                cx,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.start_batch(cx))),
                        );
                        card = card.child(row);
                        let seconds: u64 = self
                            .episodes
                            .iter()
                            .filter(|episode| episode.archived)
                            .map(|episode| u64::from(episode.duration))
                            .sum();
                        let hours = seconds as f64 / 3_600.0;
                        card = card.child(
                            div()
                                .text_size(rems(0.625))
                                .text_color(theme::secondary(cx))
                                .child(SharedString::from(format!(
                                    "{eligible} épisode{} · {:.1} h d'audio, environ {:.1} h de calcul",
                                    plural(eligible),
                                    hours,
                                    hours / 2.0
                                ))),
                        );
                    } else {
                        card = card.child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::secondary(cx))
                                .child("Aucun épisode archivé à transcrire."),
                        );
                    }

                    if let Some(active) = self.episodes.iter().find(|episode| {
                        episode
                            .transcript
                            .as_ref()
                            .is_some_and(|info| transcript::is_active(&info.status))
                    }) {
                        let info = active.transcript.clone().unwrap_or_default();
                        let percent = (info.progress.clamp(0.0, 1.0) * 100.0).round() as u32;
                        let id = active.id.as_str().to_owned();
                        card = card.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(rems(0.375))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(rems(0.6875))
                                        .child(SharedString::from(format!(
                                            "{} — {} %",
                                            active.title, percent
                                        ))),
                                )
                                .child(bar(
                                    info.progress.clamp(0.0, 1.0) as f32,
                                    theme::accent(cx),
                                    cx,
                                ))
                                .child(
                                    button(
                                        "show-transcribe-stop",
                                        "Arrêter",
                                        ButtonVariant::Secondary,
                                        cx,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            let Some(api) = this.state.read(cx).api.clone() else {
                                                return;
                                            };
                                            let id = id.clone();
                                            this.state.read(cx).runtime.spawn(async move {
                                                let _ = api.stop_episode_transcript(&id).await;
                                            });
                                            this.notice(ToastKind::Info, "Arrêt demandé", cx);
                                            this.arm_poll(cx);
                                        },
                                    )),
                                ),
                        );
                    }
                    let queued = self
                        .episodes
                        .iter()
                        .filter(|episode| {
                            episode
                                .transcript
                                .as_ref()
                                .is_some_and(|info| info.status == "queued")
                        })
                        .count();
                    if queued > 0 {
                        card = card.child(
                            div()
                                .text_size(rems(0.625))
                                .text_color(theme::secondary(cx))
                                .child(SharedString::from(format!(
                                    "{queued} épisode{} en attente de transcription",
                                    plural(queued)
                                ))),
                        );
                    }
                }
            }
        }
        card.into_any_element()
    }

    fn search_row(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let field = self
            .search_field
            .as_ref()
            .map(|field| gpui_omarchy::input("show-transcript-search", field, window, cx));
        let mut column = div()
            .flex()
            .flex_col()
            .gap(rems(0.25))
            .px(rems(1.))
            .pb(rems(0.5));
        let mut row = div().flex().items_center().gap(rems(0.375)).w_full().child(
            icon(IconName::Search)
                .size(rems(0.875))
                .text_color(theme::secondary(cx)),
        );
        if let Some(field) = field {
            row = row.child(div().flex_1().min_w_0().max_w(rems(42.)).child(field));
        }
        if !self.search_query.trim().is_empty() {
            row = row.child(
                icon_button(
                    "show-search-clear",
                    IconName::Close,
                    "Effacer la recherche",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.clear_show_search(cx))),
            );
        }
        if self.search_loading {
            row = row.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::accent(cx))
                    .child("Recherche…"),
            );
        }
        column = column.child(row);
        if let Some(error) = self.search_failed.clone() {
            let term = self.search_term.clone();
            column =
                column.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.375))
                        .child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::danger(cx))
                                .child(SharedString::from(if matches!(error, ApiError::Offline) {
                                    "La recherche dans les transcriptions a besoin du serveur."
                                        .to_owned()
                                } else {
                                    format!("La recherche a échoué : {}", card::cause(&error))
                                })),
                        )
                        .child(
                            button(
                                "show-search-retry",
                                "Réessayer",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.run_show_search(term.clone(), cx),
                            )),
                        ),
                );
        } else if !self.search_term.is_empty() && !self.search_loading {
            column = column.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(SharedString::from(format!(
                        "{} passage{} pour « {} »",
                        self.search_total,
                        plural(self.search_total as usize),
                        self.search_term
                    ))),
            );
            if self.search_hits.is_empty() {
                column = column.child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child(SharedString::from(format!(
                            "Aucun passage pour « {} » dans cette émission.",
                            self.search_term
                        ))),
                );
            } else {
                let mut hits = div().flex().flex_col().gap(rems(0.125)).w_full();
                for (index, hit) in self.search_hits.iter().take(20).enumerate() {
                    let held = hit.clone();
                    let episode_title = hit
                        .episode
                        .as_ref()
                        .map(|episode| episode.title.clone())
                        .unwrap_or_default();
                    hits = hits.child(
                        div()
                            .id(("show-hit", index))
                            .debug_selector(move || format!("show-hit-{index}"))
                            .flex()
                            .items_baseline()
                            .gap(rems(0.5))
                            .w_full()
                            .min_w_0()
                            .px(rems(0.375))
                            .py(rems(0.25))
                            .border_b_1()
                            .border_color(theme::border(cx).opacity(0.4))
                            .cursor_pointer()
                            .hover(|style| style.bg(theme::chrome(cx)))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.play_hit(held.clone(), cx)),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .w(rems(3.))
                                    .text_size(rems(0.6875))
                                    .font_family(theme::mono(cx))
                                    .text_color(theme::accent(cx))
                                    .child(duration_label(hit.start.max(0.0) as u32)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(rems(0.75))
                                    .child(hit.text.clone()),
                            )
                            .when(!episode_title.is_empty(), |row| {
                                row.child(
                                    div()
                                        .flex_shrink_0()
                                        .max_w(rems(18.))
                                        .truncate()
                                        .text_size(rems(0.625))
                                        .text_color(theme::secondary(cx))
                                        .child(episode_title),
                                )
                            }),
                    );
                }
                column = column.child(hits);
            }
        }
        column.into_any_element()
    }

    fn episode_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.episodes.is_empty() {
            let mut empty = card::empty(
                IconName::MicVocal,
                "Aucun épisode",
                "Cette émission n'a pas encore d'épisode lisible.",
                cx,
                None,
            );
            if self.next.is_some() {
                empty = empty.child(
                    button(
                        "show-more-empty",
                        "Charger des épisodes",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.load_more(cx))),
                );
            }
            return div().p(rems(1.5)).child(empty).into_any_element();
        }
        let sizes = Rc::clone(&self.sizes);
        let cursor = self.cursor;
        let scroll = self.scroll.clone();
        self.rendered_rows = 0;
        let list = virtual_list(
            cx.entity(),
            ElementId::Name("show-episodes".into()),
            sizes,
            move |this, range: std::ops::Range<usize>, _window, cx| {
                this.rendered_rows = range.len();
                range
                    .map(|index| this.row(index, index == cursor, cx))
                    .collect()
            },
            cx,
        )
        .h_full()
        .min_h_0()
        .track_scroll(&scroll);
        let mut column = div()
            .relative()
            .id("show-episodes-box")
            .debug_selector(|| "show-episodes-box".to_owned())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(div().flex_1().min_h_0().child(list));
        if self.row_menu.is_some() {
            column = column.child(self.menu_view(cx));
        }
        if self.next.is_some() || self.loading_more {
            column = column.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .py(rems(0.5))
                    .child(
                        button(
                            "show-more",
                            if self.loading_more {
                                "Chargement…".to_owned()
                            } else {
                                "Charger plus d'épisodes".to_owned()
                            },
                            ButtonVariant::Outline,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.load_more(cx))),
                    ),
            );
        }
        column.into_any_element()
    }

    fn row(&mut self, index: usize, selected: bool, cx: &mut Context<Self>) -> AnyElement {
        let chip = self
            .episodes
            .get(index)
            .map(episode_panel::key)
            .map(|key| self.rows.state(&key))
            .and_then(|state| download_control::chip(&state, cx));
        let Some(episode) = self.episodes.get(index).cloned() else {
            return div().into_any_element();
        };
        let key = episode_panel::key(&episode);
        let current = self
            .state
            .read(cx)
            .player
            .current()
            .map(|entry| entry.id.clone());
        let is_now = current.as_deref() == Some(key.as_str());
        let playing = self.state.read(cx).player.playing();
        let resume: Option<Resume> = episode_panel::resume_of(
            self.positions
                .iter()
                .find(|(id, _)| id == &key)
                .map(|(_, position)| position),
            episode.duration,
        );
        let mut foot: Vec<String> = vec![episode_panel::date_label(episode.published.unwrap_or(0))];
        if episode.duration > 0 {
            foot.push(duration_label(episode.duration));
        }
        if let Some(resume) = resume {
            foot.push(episode_panel::resume_label(resume));
        }
        let mut chips = div().flex().items_center().gap(rems(0.25)).min_w_0();
        if !episode.archived {
            chips = chips.child(badge("non archivé", theme::warning(cx), cx));
        }
        if let Some(info) = episode.transcript.as_ref() {
            chips = chips.child(badge(
                &episode_panel::transcript_state_label(info),
                if info.status == "ready" {
                    theme::success(cx)
                } else if info.status == "error" {
                    theme::danger(cx)
                } else {
                    theme::secondary(cx)
                },
                cx,
            ));
        }
        if episode.chapter_count > 0 {
            chips = chips.child(badge(
                &format!(
                    "{} chapitre{}",
                    episode.chapter_count,
                    if episode.chapter_count > 1 { "s" } else { "" }
                ),
                theme::secondary(cx),
                cx,
            ));
        }
        let marks = self.markers.for_episode(&key).len();
        if marks > 0 {
            chips = chips.child(badge(
                &format!("{marks} marqueur{}", if marks > 1 { "s" } else { "" }),
                theme::accent(cx),
                cx,
            ));
        }

        div()
            .id(("episode-row", index))
            .debug_selector(move || format!("episode-row-{index}"))
            .flex()
            .items_center()
            .gap(rems(0.5))
            .h(px(ROW_HEIGHT))
            .w_full()
            .min_w_0()
            .pl(rems(0.375))
            .pr(rems(0.5))
            .border_l_2()
            .border_color(if is_now {
                theme::accent(cx)
            } else if selected {
                theme::focus_ring(cx)
            } else {
                theme::shell(cx)
            })
            .when(selected, |row| row.bg(theme::selected(cx)))
            .cursor_pointer()
            .hover(|style| style.bg(theme::chrome(cx)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle(index, cx);
                this.open_panel(index, cx);
            }))
            .child(
                div()
                    .id(("episode-play", index))
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(rems(2.))
                    .h(rems(2.))
                    .flex_shrink_0()
                    .rounded_full()
                    .border_1()
                    .border_color(if is_now {
                        theme::accent(cx)
                    } else {
                        theme::border(cx)
                    })
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::chrome(cx)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle(index, cx);
                    }))
                    .child(
                        icon(if is_now && playing {
                            IconName::Pause
                        } else {
                            IconName::Play
                        })
                        .size(rems(0.875))
                        .text_color(if is_now {
                            theme::accent(cx)
                        } else {
                            theme::text(cx)
                        }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.125))
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(rems(0.8125))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if is_now {
                                theme::accent(cx)
                            } else {
                                theme::text(cx)
                            })
                            .child(episode.title.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.375))
                            .min_w_0()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(rems(0.6875))
                                    .font_family(theme::mono(cx))
                                    .text_color(theme::secondary(cx))
                                    .child(SharedString::from(
                                        foot.into_iter()
                                            .filter(|part| !part.is_empty())
                                            .collect::<Vec<_>>()
                                            .join(" · "),
                                    )),
                            )
                            .child(chips),
                    ),
            )
            .children(chip)
            .child(
                icon_button(
                    "row-menu",
                    IconName::EllipsisVertical,
                    "Options de l'épisode",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.cursor = index;
                    this.row_menu = Some(RowMenu { index, cursor: 0 });
                    this.install_keys(cx);
                    cx.notify();
                })),
            )
            .into_any_element()
    }

    fn menu_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(menu) = &self.row_menu else {
            return div().into_any_element();
        };
        let top = px(ROW_HEIGHT) * menu.index as f32 - self.scroll.base_handle().offset().y;
        let mut panel = div()
            .absolute()
            .left(rems(2.5))
            .top(top)
            .w(rems(17.))
            .flex()
            .flex_col()
            .gap(rems(0.0625))
            .p(rems(0.25))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .shadow_lg();
        // The episode's own state, from the rows resolved for this version (a
        // `&self` view cannot re-read the worker).
        let download = self
            .episodes
            .get(menu.index)
            .map(|episode| self.rows.state(&episode_panel::key(episode)))
            .unwrap_or(download_control::State::Idle);
        for (at, item) in RowItem::ALL.iter().enumerate() {
            let selected = at == menu.cursor;
            let held = *item;
            panel = panel.child(
                div()
                    .id(("row-menu-item", at))
                    .debug_selector(move || format!("row-menu-item-{at}"))
                    .flex()
                    .items_center()
                    .gap(rems(0.375))
                    .h(rems(1.75))
                    .px(rems(0.375))
                    .text_size(rems(0.75))
                    .text_color(if selected {
                        theme::text(cx)
                    } else {
                        theme::secondary(cx)
                    })
                    .when(selected, |row| row.bg(theme::selected(cx)))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::selected(cx)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.pick_row(held, cx);
                    }))
                    .child(icon(item.glyph()).size(rems(0.75)))
                    .child(item.label_for(&download)),
            );
        }
        panel.into_any_element()
    }

    fn panel_view(
        &mut self,
        panel: &mut EpisodePanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_admin = self.state.read(cx).is_admin();
        let current = self
            .state
            .read(cx)
            .player
            .current()
            .map(|entry| entry.id.clone());
        let playing = self.state.read(cx).player.playing();
        let is_current = current.as_deref() == Some(panel.key());
        let playhead = if is_current {
            self.state.read(cx).player.current_time()
        } else {
            0.0
        };
        if is_current {
            panel.transcript.follow_to(playhead);
        }
        let progress = self
            .positions
            .iter()
            .find(|(id, _)| id == panel.key())
            .map(|(_, position)| position.clone());
        let state = self.state.clone();
        let field = self.transcript_field.clone();
        let world = episode_panel::View {
            chapters: &self.chapters,
            markers: &self.markers,
            progress,
            is_admin,
            is_current,
            playing,
            playhead_s: playhead,
        };
        episode_panel::view(
            panel,
            cx.entity(),
            &state,
            world,
            field.as_ref(),
            window,
            cx,
            |this: &mut Self, act, cx| this.on_panel_act(act, cx),
        )
    }

    fn on_panel_act(&mut self, act: Act, cx: &mut Context<Self>) {
        match act {
            Act::Close => self.close_panel(cx),
            Act::Seek(at) => {
                if let Some(index) = self.panel_index() {
                    self.play_at(index, at, cx);
                }
            }
            Act::LoadChapters | Act::RetryChapters => {
                if let Some(panel) = self.panel.as_ref() {
                    let id = panel.key().to_owned();
                    if matches!(act, Act::RetryChapters) {
                        self.chapters.retry(&id);
                    }
                    self.ensure_chapters(&id, cx);
                }
            }
            Act::AddMarker(position_ms) => {
                if let Some(panel) = self.panel.as_ref() {
                    let id = panel.key().to_owned();
                    self.add_marker(&id, position_ms, cx);
                }
            }
            Act::DeleteMarker(marker) => self.remove_marker(marker, cx),
            Act::ToggleDescription => {
                if let Some(panel) = self.panel.as_mut() {
                    panel.description_open = !panel.description_open;
                }
                cx.notify();
            }
            Act::ToggleAllChapters => {
                if let Some(panel) = self.panel.as_mut() {
                    panel.chapters_expanded = !panel.chapters_expanded;
                }
                cx.notify();
            }
            Act::Transcript(asked) => self.ask_transcript(asked, cx),
        }
    }
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.search_field.is_none() {
            let field = cx.new(|cx| {
                InputState::new(window, cx).placeholder("Rechercher dans les transcriptions…")
            });
            let subscription =
                cx.subscribe(&field, |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => this.typed_show_search(cx),
                    InputEvent::PressEnter { .. } => {
                        let term = this.search_query.trim().to_owned();
                        if term.chars().count() >= MIN_QUERY {
                            this.search_pending = None;
                            this.run_show_search(term, cx);
                        }
                    }
                    _ => {}
                });
            self._subscriptions.push(subscription);
            self.search_field = Some(field);
        }
        if self.transcript_field.is_none() {
            let field =
                cx.new(|cx| InputState::new(window, cx).placeholder("Rechercher dans l'épisode…"));
            let subscription =
                cx.subscribe(&field, |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => {
                        this.transcript_query = this
                            .transcript_field
                            .as_ref()
                            .map(|field| field.read(cx).value().to_string())
                            .unwrap_or_default();
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => {
                        let term = this.transcript_query.trim().to_owned();
                        if term.chars().count() >= MIN_QUERY {
                            this.run_transcript_search(term, cx);
                        }
                    }
                    _ => {}
                });
            self._subscriptions.push(subscription);
            self.transcript_field = Some(field);
        }
        if std::mem::take(&mut self.search_cleared)
            && let Some(field) = self.search_field.clone()
        {
            field.update(cx, |state, cx| state.set_value("", window, cx));
        }
        if let Some(value) = self.transcript_pending.take()
            && let Some(field) = self.transcript_field.clone()
        {
            field.update(cx, |state, cx| state.set_value(value, window, cx));
        }
        if let Some(panel) = self.panel.as_mut() {
            let current = self
                .state
                .read(cx)
                .player
                .current()
                .map(|entry| entry.id.clone());
            if current.as_deref() == Some(panel.key()) {
                let at = self.state.read(cx).player.current_time();
                panel.transcript.follow_to(at);
            }
        }

        let root = div()
            .id("show-screen")
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .font_family(theme::font(cx))
            .text_color(theme::text(cx))
            .key_context("Neezer");
        let content = match self.page.ready().cloned() {
            Some(page) => self.ready(page, window, cx),
            None => match &self.page {
                Phase::Failed(error) => {
                    let cause = card::cause(error);
                    card::failed(
                        "Podcast introuvable",
                        cause,
                        cx,
                        |this: &mut Self, _, cx| this.reload(cx),
                    )
                    .into_any_element()
                }
                _ => div()
                    .p(rems(1.5))
                    .child(card::skeleton(6, px(148.), cx))
                    .into_any_element(),
            },
        };
        root.child(content)
    }
}

impl Screen {
    fn ready(
        &mut self,
        page: PodcastPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let show = page.show.clone();
        let mut urls: Vec<String> = show.cover.iter().cloned().collect();
        for episode in &self.episodes {
            if let Some(cover) = episode.cover.clone() {
                urls.push(cover);
            }
        }
        self.covers_for(urls, cx);

        // The downloads rows for this published version: one chip per episode,
        // and the state the row menu reads.
        let version = self.downloads.get().version;
        if version != self.rows_version {
            self.rows = Rc::new(Rows::build(self.downloads.get()));
            self.rows_version = version;
        }

        let fit = head_fit(page_height(window));
        let settings_open = self.settings_open.unwrap_or(fit.settings);
        self.settings_shown = settings_open;
        let header = self.header(&show, fit.compact, cx);
        let actions = self.actions(&show, cx);
        let switches = self.switches(&show, settings_open, cx);
        let search = if self.ready_transcripts() > 0
            || self.search_loading
            || !self.search_term.is_empty()
        {
            self.search_row(window, cx)
        } else {
            div().into_any_element()
        };

        // The body: the hits while a search is on, otherwise the list; and the
        // panel beside them when the window has the width.
        let content_width = f32::from(window.viewport_size().width)
            - SIDEBAR
            - if self.state.read(cx).side_panel.is_some() {
                QUEUE_PANEL
            } else {
                0.
            };
        let searching = !self.search_term.is_empty();
        let mut panel = self.panel.take();
        let body = if searching {
            div()
                .flex()
                .flex_col()
                .size_full()
                .min_h_0()
                .child(self.episode_list(cx))
                .into_any_element()
        } else if let Some(panel_ref) = panel.as_mut() {
            if content_width >= PANEL_MIN_CONTENT {
                // Every wrapper here is a FLEX container on purpose: a plain
                // block with `flex_1` gives its child no definite height, so the
                // virtual list's `h_full` resolved to 0 and the episode list
                // painted NOTHING beside an open panel (seen at 1280 and 2560;
                // the panel's own `h_full` collapsed the same way, which is why
                // its body overflowed over the player bar instead of scrolling).
                div()
                    .flex()
                    .size_full()
                    .min_h_0()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(self.episode_list(cx)),
                    )
                    .child(
                        div()
                            .id("show-panel")
                            .flex()
                            .flex_col()
                            .w(px(PANEL_WIDTH))
                            .h_full()
                            .flex_shrink_0()
                            .min_h_0()
                            .border_l_1()
                            .border_color(theme::border(cx))
                            .child(self.panel_view(panel_ref, window, cx)),
                    )
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .min_h_0()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.25))
                            .px(rems(0.75))
                            .py(rems(0.25))
                            .border_b_1()
                            .border_color(theme::border(cx))
                            .child(
                                button(
                                    "show-back-list",
                                    "Retour à la liste",
                                    ButtonVariant::Secondary,
                                    cx,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.close_panel(cx))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .child(self.panel_view(panel_ref, window, cx)),
                    )
                    .into_any_element()
            }
        } else {
            div()
                .flex()
                .flex_col()
                .size_full()
                .min_h_0()
                .child(self.episode_list(cx))
                .into_any_element()
        };
        self.panel = panel;

        // The head scrolls and never takes more than `HEAD_SHARE` of the page:
        // with the admin's transcription card open a 1280x800 window left the
        // episode list 38 px of height, which is the list being unusable, not a
        // compact page. `head_fit` is what keeps a control from being cut in
        // half at that cap (the folded admin row is 34 px, not 220).
        let head = div()
            .id("show-head")
            .debug_selector(|| "show-head".to_owned())
            .flex()
            .flex_col()
            .flex_shrink_0()
            .max_h(relative(HEAD_SHARE))
            .min_h_0()
            .overflow_y_scroll()
            .child(header)
            .child(actions)
            .child(switches)
            .child(search);
        let mut column = div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .child(head)
            .child(div().flex_1().min_h_0().child(body));
        if let Some(error) = self.more_failed.clone() {
            column = column.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .px(rems(1.))
                    .py(rems(0.375))
                    .text_size(rems(0.6875))
                    .text_color(theme::danger(cx))
                    .child(SharedString::from(format!(
                        "Les épisodes suivants n'ont pas pu être chargés : {}",
                        card::cause(&error)
                    )))
                    .child(
                        button("show-more-retry", "Réessayer", ButtonVariant::Secondary, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.load_more(cx))),
                    ),
            );
        }
        // The page's reading width: the same 1800 px cap D12's screens use, so a
        // 2560 px window is margins rather than a two-metre episode row.
        div()
            .size_full()
            .flex()
            .justify_center()
            .child(column.max_w(px(PAGE_MAX)))
            .into_any_element()
    }
}

/// The batch sizes a count of eligible episodes offers (the SPA's
/// `batchOptions`).
pub fn batch_choices(eligible: usize) -> Vec<usize> {
    if eligible == 0 {
        return Vec::new();
    }
    let mut out = vec![eligible.min(5)];
    if eligible >= 10 {
        out.push(10);
    }
    if eligible >= 20 {
        out.push(20);
    }
    out
}

/// A `409 already running` is a job at work, not a failure: the body's `code` is
/// what names it, which `nz-api` does not model yet (reported for its owner).
pub fn conflict_running(error: &ApiError) -> bool {
    match error {
        ApiError::Http {
            status: 409,
            message,
            ..
        } => message.contains("already_running") || message.contains("déjà en cours"),
        _ => false,
    }
}

/// A `f64` in a query string, rounded to 0.01 s with no trailing zeros.
fn trim_float(value: f64) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    if (rounded - rounded.round()).abs() < f64::EPSILON {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    }
}

fn plural(count: usize) -> &'static str {
    if count > 1 { "s" } else { "" }
}

fn badge(label: &str, colour: gpui_kit::Hsla, _cx: &App) -> AnyElement {
    div()
        .flex_shrink_0()
        .px(rems(0.25))
        .py(rems(0.0625))
        .border_1()
        .border_color(colour.opacity(0.5))
        .text_size(rems(0.5625))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colour)
        .child(SharedString::from(label.to_owned()))
        .into_any_element()
}

fn bar(fraction: f32, colour: gpui_kit::Hsla, cx: &App) -> AnyElement {
    div()
        .w(rems(8.))
        .h(rems(0.1875))
        .relative()
        .bg(theme::border(cx))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .w(gpui_kit::relative(fraction.clamp(0.0, 1.0)))
                .bg(colour),
        )
        .into_any_element()
}

/// What a transcription refusal says, in French: the server's own cause when it
/// gave one, the SPA's sentence otherwise.
pub fn transcribe_cause(error: &ApiError) -> SharedString {
    let (code, body) = match error {
        ApiError::Http { message, code, .. } => (code.clone().unwrap_or_default(), message.clone()),
        ApiError::Provider { message, code, .. } => (code.clone(), message.clone()),
        _ => (String::new(), String::new()),
    };
    if let Some(reason) = body
        .split("\"reason\":\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
    {
        return transcript::reason_text(reason).into();
    }
    match code.as_str() {
        "too_long" => "L'épisode dépasse la durée maximale de transcription.".into(),
        "not_archived" => "L'épisode doit d'abord être archivé sur le serveur.".into(),
        "no_duration" => "La durée de l'épisode est inconnue ; impossible de la borner.".into(),
        "queue_full" => "La file de transcription est pleine.".into(),
        "already_running" => "Cet épisode est déjà en cours de transcription.".into(),
        "ready" => "Cet épisode est déjà transcrit.".into(),
        _ => match error {
            ApiError::Offline => {
                "Le serveur ne répond pas : la transcription n'a pas démarré.".into()
            }
            ApiError::NotReady { retry_after } | ApiError::Busy { retry_after } => {
                format!("Le serveur est occupé : réessayez dans {retry_after} s.").into()
            }
            other => {
                let text = body.trim();
                if text.is_empty() {
                    card::cause(other)
                } else {
                    format!("{} : {}", card::cause(other), text).into()
                }
            }
        },
    }
}

/// A typed GET on a chosen lane: `Api::get_value` is the `User` lane, and a poll
/// must not take a `User` slot while the person browses (rule 1).
async fn get<T: serde::de::DeserializeOwned>(
    api: &Api,
    lane: Lane,
    path: &str,
) -> Result<T, ApiError> {
    api.get::<T>(lane, path).await.map(|fresh| fresh.value)
}

// --- the two transcription keys ----------------------------------------------

static KEYS_INSTALLED: AtomicBool = AtomicBool::new(false);

/// `Ctrl+T` starts the panel episode's transcription, `Ctrl+Maj+T` stops it: the
/// two actions D7 declared for this lot and left unbound. Installed ONCE for the
/// application in the keymap's own context (D8b's precedent for `Alt+↑/↓`), so no
/// lot edits `keymap.rs`.
fn install_transcribe_keys(cx: &mut App) {
    if KEYS_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    cx.bind_keys([
        KeyBinding::new("ctrl-t", StartTranscription, Some(crate::keymap::CONTEXT)),
        KeyBinding::new(
            "ctrl-shift-t",
            StopTranscription,
            Some(crate::keymap::CONTEXT),
        ),
    ]);
}

/// The app-level handlers those bindings reach: a screen that is gone answers
/// nothing (a weak handle), and no screen ever takes the focus (D8b's rule).
fn register_transcribe_actions(cx: &mut Context<Screen>) {
    let weak = cx.weak_entity();
    App::on_action(&mut *cx, move |_: &StartTranscription, cx: &mut App| {
        let _ = weak.update(cx, |this, cx| this.ask_transcript(TranscriptAsk::Start, cx));
    });
    let weak = cx.weak_entity();
    App::on_action(&mut *cx, move |_: &StopTranscription, cx: &mut App| {
        let _ = weak.update(cx, |this, cx| this.ask_transcript(TranscriptAsk::Stop, cx));
    });
}

// --- the position keeper -----------------------------------------------------

/// Start the podcast position keeper, once for the process.
///
/// Saving an episode's position is a player concern; `docs/agents/desktop.md`
/// (*Streams*) places it in `state.rs`, which another lot owns while D11 is
/// written. The keeper is therefore a process-wide task the two podcast screens
/// install, watching the shell's own `PlayerState` — it is what makes an episode
/// resume where the person stopped. The one remaining gap (an episode started
/// from a transcript hit before any podcast screen was opened) closes with one
/// call at startup in `state.rs`; D11's PR reports it.
///
/// Rules, each ported from `lib/podcastProgress.js` or measured:
/// - a write on pause, every [`PROGRESS_EVERY`] of playing, on an episode change,
///   and at quit (`App::on_app_quit`, awaited by GPUI's shutdown timeout);
/// - below [`PROGRESS_MIN`] nothing is written (a mis-tap is not a resume point);
/// - within [`DONE_TAIL`] of the end the episode is `finished` — the server's own
///   `_EPISODE_DONE_TAIL`, so a finished episode never resumes into the credits;
/// - offline, the write is one `pending_progress` row (last write wins) sent when
///   the link returns; one send every 5 s and only while something waits;
/// - the device copy is per ACCOUNT (`podcast.progress.<owner>`): two people on
///   one laptop never see each other's positions.
pub fn attach_keeper<V: Render>(state: Entity<AppState>, cx: &mut Context<V>) {
    if KEEPER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let owner = state.read(cx).api.as_ref().and_then(|api| api.owner());
    if let Some(owner) = owner.clone() {
        refresh_positions(&state, cx, &owner);
        // The server's copy, merged once: the newest write wins on both sides, and
        // the rows it never saw are pushed back by the flush below.
        if let Some(api) = state.read(cx).api.clone() {
            let runtime = state.read(cx).runtime.clone();
            let cache = state.read(cx).cache.clone();
            runtime.spawn(async move {
                if let Ok(page) = api.podcast_progress().await {
                    merge_server(&owner, cache.as_ref(), page.progress);
                }
            });
        }
    }

    // The beat: one second. It writes only when something moved.
    let quit_state = state.clone();
    cx.spawn(async move |_weak, cx| {
        let mut watch = Watch::default();
        let mut seen = version_of();
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let changed = cx.update(|app| {
                let (decision, playing) = {
                    let snapshot = state.read(app);
                    (
                        watch.decide(snapshot, Instant::now()),
                        snapshot.player.playing(),
                    )
                };
                watch.playing = playing;
                if let Decision::Save {
                    key,
                    position,
                    duration,
                } = decision
                {
                    let snapshot = state.read(app);
                    save(snapshot, &key, position, duration);
                }
                // What waited offline goes up once the link is back.
                if watch
                    .last_flushed
                    .is_none_or(|at| at.elapsed() >= Duration::from_secs(5))
                {
                    watch.last_flushed = Some(Instant::now());
                    let snapshot = state.read(app);
                    flush_pending(snapshot);
                }
                let version = version_of();
                let changed = version != seen;
                if changed {
                    seen = version;
                }
                changed
            });
            if changed {
                cx.update(|app| state.update(app, |_state, cx| cx.notify()));
            }
        }
    })
    .detach();

    // At quit: the device's copy, written now. The pending row is what the next
    // start sends — GPUI is shutting the runtime down, so no network here.
    cx.on_app_quit(move |_this, cx| {
        let state = quit_state.clone();
        let snapshot = state.read(cx);
        let entry = snapshot
            .player
            .current()
            .cloned()
            .filter(|entry| entry.is_podcast());
        let position = if snapshot.player.playing() {
            match snapshot.audio.as_ref() {
                Some(audio) if audio.position().is_finite() => audio.position(),
                _ => snapshot.player.current_time(),
            }
        } else {
            snapshot.player.current_time()
        };
        if let Some(entry) = entry
            && position >= PROGRESS_MIN
        {
            save_at_quit(snapshot, &entry.id, position, entry.duration);
        }
        async {}
    })
    .detach();
}

static KEEPER_STARTED: AtomicBool = AtomicBool::new(false);

/// The store: what this device remembers, shared by the UI (the rows), the
/// keeper's beat and the one merge task. A process-wide `static` because that is
/// the lifetime the SPA gives its `localStorage` mirror.
#[derive(Default)]
struct Store {
    rows: BTreeMap<String, Position>,
    version: u64,
}

fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(Store::default()))
}

fn version_of() -> u64 {
    store().lock().map(|store| store.version).unwrap_or(0)
}

fn bump(store: &mut Store) {
    store.version = store.version.wrapping_add(1);
}

/// One remembered position, as the device keeps it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub t: u32,
    pub d: u32,
    pub at_ms: i64,
    pub done: bool,
}

impl Position {
    pub fn model(&self) -> PodcastPosition {
        PodcastPosition {
            position: self.t,
            duration: self.d,
            finished: self.done,
            updated: self.at_ms.div_euclid(1_000),
        }
    }
}

/// The `kv` key a device copy lives under, per account.
pub fn device_key(owner: &str) -> String {
    format!("podcast.progress.{owner}")
}

/// The positions, as a screen reads them: a copy of the store, cheap enough to
/// take on every paint (a person's episodes, not a library).
pub fn positions(_cx: &App) -> Vec<(String, PodcastPosition)> {
    store()
        .lock()
        .map(|store| {
            store
                .rows
                .iter()
                .map(|(episode, position)| (episode.clone(), position.model()))
                .collect()
        })
        .unwrap_or_default()
}

/// An explicit resume point (a marker jump, a hit picked): kept without the noise
/// filter the automatic saves use (the SPA's `setResumePoint`).
pub fn set_resume(_cx: &mut App, episode: &str, at: f64, duration: u32) {
    if !at.is_finite() {
        return;
    }
    let at = at.max(0.0).round().min(f64::from(u32::MAX)) as u32;
    if let Ok(mut store) = store().lock() {
        store.rows.insert(
            episode.to_owned(),
            Position {
                t: at,
                d: duration,
                at_ms: now_ms(),
                done: false,
            },
        );
        bump(&mut store);
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or(0)
}

fn position_now(state: &Entity<AppState>, app: &App) -> f64 {
    let snapshot = state.read(app);
    if snapshot.player.playing()
        && let Some(audio) = &snapshot.audio
    {
        let position = audio.position();
        if position.is_finite() {
            return position;
        }
    }
    snapshot.player.current_time()
}

fn refresh_positions<V: Render>(state: &Entity<AppState>, cx: &mut Context<V>, owner: &str) {
    let Some(cache) = state.read(cx).cache.clone() else {
        return;
    };
    let stored: BTreeMap<String, Position> = cache
        .kv()
        .get::<BTreeMap<String, Position>>(&device_key(owner))
        .unwrap_or_default();
    if let Ok(mut store) = store().lock() {
        store.rows = stored;
        bump(&mut store);
    }
}

/// The server's copy merged into the device's: the newest write wins on both
/// sides; the merged map is written back on the network thread (SQLite never runs
/// on the main thread, D3's rule).
fn merge_server(
    owner: &str,
    cache: Option<&nz_cache::Cache>,
    remote: BTreeMap<String, PodcastPosition>,
) {
    let Ok(mut store) = store().lock() else {
        return;
    };
    for (episode, position) in remote {
        let server_at = position.updated.saturating_mul(1_000);
        let local_newer = store
            .rows
            .get(&episode)
            .is_some_and(|local| local.at_ms > server_at + 2_000);
        if !local_newer {
            store.rows.insert(
                episode,
                Position {
                    t: position.position,
                    d: position.duration,
                    at_ms: server_at,
                    done: position.finished,
                },
            );
        }
    }
    bump(&mut store);
    if let Some(cache) = cache
        && let Err(error) = cache.kv().set(&device_key(owner), &store.rows)
    {
        tracing::debug!(%error, "positions de podcast non enregistrées");
    }
}

/// Write one position: the store, the device's copy, a `pending_progress` row,
/// then the send (the flush does it when the link is up).
pub fn save(state: &AppState, episode: &str, position: u32, duration: u32) {
    if position == 0 {
        return;
    }
    let Some(owner) = state.api.as_ref().and_then(|api| api.owner()) else {
        return;
    };
    let duration = if duration > 0 { duration } else { position };
    let done = duration > 0 && position >= duration.saturating_sub(DONE_TAIL);
    let row = Position {
        t: position,
        d: duration,
        at_ms: now_ms(),
        done,
    };
    if let Ok(mut store) = store().lock() {
        store.rows.insert(episode.to_owned(), row);
        bump(&mut store);
        if let Some(cache) = state.cache.as_ref()
            && let Err(error) = cache.kv().set(&device_key(&owner), &store.rows)
        {
            tracing::debug!(%error, "positions de podcast non enregistrées");
        }
    }
    let Some(cache) = state.cache.as_ref() else {
        return;
    };
    let body = serde_json::json!({
        "episode_id": episode,
        "position": position,
        "duration": duration,
        "finished": done,
    })
    .to_string();
    if let Err(error) = cache.pending_progress().put(&owner, episode, &body) {
        tracing::debug!(%error, "position de podcast non enregistrée");
        return;
    }
    if state.online {
        flush_pending(state);
    }
}

/// The quit path: the device's copy and the pending row, no network.
fn save_at_quit(state: &AppState, episode: &str, position: f64, duration: u32) {
    let Some(owner) = state.api.as_ref().and_then(|api| api.owner()) else {
        return;
    };
    let seconds = position.max(0.0).round() as u32;
    let duration = if duration > 0 { duration } else { seconds };
    let done = duration > 0 && seconds >= duration.saturating_sub(DONE_TAIL);
    let row = Position {
        t: seconds,
        d: duration,
        at_ms: now_ms(),
        done,
    };
    if let Ok(mut store) = store().lock() {
        store.rows.insert(episode.to_owned(), row);
        bump(&mut store);
        if let Some(cache) = state.cache.as_ref() {
            let _ = cache.kv().set(&device_key(&owner), &store.rows);
        }
    }
    if let Some(cache) = state.cache.as_ref() {
        let body = serde_json::json!({
            "episode_id": episode,
            "position": seconds,
            "duration": duration,
            "finished": done,
        })
        .to_string();
        let _ = cache.pending_progress().put(&owner, episode, &body);
    }
}

/// Send what waits, one request each, oldest first. A refused row stays.
pub fn flush_pending(state: &AppState) {
    let (Some(api), Some(cache)) = (state.api.clone(), state.cache.clone()) else {
        return;
    };
    if !api.is_online() {
        return;
    }
    let Some(owner) = api.owner() else {
        return;
    };
    let Ok(rows) = cache.pending_progress().pending(&owner) else {
        return;
    };
    for (episode, raw) in rows {
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
        let position = parsed
            .get("position")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32;
        let duration = parsed
            .get("duration")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32;
        let finished = parsed
            .get("finished")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let api = api.clone();
        let cache = cache.clone();
        let owner = owner.clone();
        state.runtime.spawn(async move {
            if api
                .save_podcast_progress(&episode, position, duration, finished)
                .await
                .is_ok()
            {
                let _ = cache.pending_progress().sent(&owner, &episode);
            }
        });
    }
}

/// One beat's verdict: the keeper's pure half, so a test can hold it.
#[derive(Default)]
pub struct Watch {
    pub episode: Option<String>,
    pub playing: bool,
    pub last_sync: Option<Instant>,
    pub last_flushed: Option<Instant>,
    pub last_position: f64,
    pub duration: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    Nothing,
    Save {
        key: String,
        position: u32,
        duration: u32,
    },
}

impl Watch {
    /// An episode change writes the previous episode first; a pause writes at
    /// once; 30 s of playing write; nothing else does.
    pub fn decide(&mut self, state: &AppState, now: Instant) -> Decision {
        let Some(entry) = state.player.current().filter(|entry| entry.is_podcast()) else {
            self.episode = None;
            self.playing = false;
            return Decision::Nothing;
        };
        let position = audible_position(state);
        let key = entry.id.clone();
        let duration = entry.duration;
        let playing = state.player.playing();
        if self.episode.as_deref() != Some(key.as_str()) {
            let previous = self
                .episode
                .clone()
                .filter(|_| self.last_position >= PROGRESS_MIN)
                .map(|previous| (previous, self.last_position.round() as u32, self.duration));
            self.episode = Some(key);
            self.playing = playing;
            self.last_sync = None;
            self.last_position = position;
            self.duration = duration;
            return match previous {
                Some((key, position, duration)) => Decision::Save {
                    key,
                    position,
                    duration,
                },
                None => Decision::Nothing,
            };
        }
        let paused = self.playing && !playing;
        self.playing = playing;
        self.duration = duration;
        // A mis-tap is never a position — not even its pause.
        if position < PROGRESS_MIN {
            return Decision::Nothing;
        }
        let due = self
            .last_sync
            .is_none_or(|at| now.duration_since(at) >= PROGRESS_EVERY);
        if paused || (playing && due) {
            self.last_sync = Some(now);
            self.last_position = position;
            return Decision::Save {
                key,
                position: position.round() as u32,
                duration,
            };
        }
        Decision::Nothing
    }
}

fn audible_position(state: &AppState) -> f64 {
    if state.player.playing()
        && let Some(audio) = &state.audio
    {
        let position = audio.position();
        if position.is_finite() {
            return position;
        }
    }
    state.player.current_time()
}

fn count_skipped(value: &Value) -> u64 {
    match value {
        Value::Number(number) => number.as_u64().unwrap_or(0),
        Value::Object(map) => map.values().map(|value| value.as_u64().unwrap_or(0)).sum(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppConfig, AppState, event_channel};
    use nz_player::{Entry, PlayerState, Snapshot};

    /// A real `AppState` over a temporary store: the keeper reads the shell's own
    /// queue, so a fake would prove nothing.
    fn app(name: &str) -> (AppState, tokio::runtime::Runtime) {
        let dir = std::env::temp_dir().join(format!(
            "neezer-d11-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (events, _rx) = event_channel();
        let state = AppState::new(
            AppConfig {
                start_audio: false,
                start_mpris: false,
                ..AppConfig::for_test(&dir)
            },
            runtime.handle().clone(),
            events,
            None,
        );
        (state, runtime)
    }

    fn episode() -> Entry {
        Entry {
            id: "e1".into(),
            title: "Épisode".into(),
            duration: 600,
            podcast: Some(true),
            channel_id: Some("c1".into()),
            ..Default::default()
        }
    }

    fn queued(state: &mut AppState, position: f64) {
        let mut player = PlayerState::restore(Snapshot {
            queue: vec![episode()],
            index: 0,
            autoplay: false,
            ..Snapshot::default()
        })
        .0;
        player.set_position(position);
        player.set_duration(600.0);
        state.player = player;
    }

    /// The keeper's pure half: the first beat only starts the follow, a pause
    /// writes at once, a move within 30 s does not, and 30 s of playing does.
    #[test]
    fn the_watch_writes_on_pause_and_every_thirty_seconds() {
        let (mut state, _runtime) = app("keeper");
        queued(&mut state, 0.0);
        let mut watch = Watch::default();
        let now = Instant::now();
        assert_eq!(
            watch.decide(&state, now),
            Decision::Nothing,
            "nothing playing: nothing to write"
        );

        state.player.set_position(12.0);
        assert_eq!(
            watch.decide(&state, now),
            Decision::Nothing,
            "the first beat only starts the follow"
        );

        watch.playing = true;
        match watch.decide(&state, now) {
            Decision::Save { key, position, .. } => {
                assert_eq!(key, "e1");
                assert_eq!(position, 12);
            }
            other => panic!("a pause must write, got {other:?}"),
        }

        // Playing (the state machine says so), moved but inside the window.
        state.player.apply(
            nz_player::Command::Play {
                args: Default::default(),
            },
            0,
        );
        state.player.set_position(40.0);
        watch.playing = true;
        assert_eq!(
            watch.decide(&state, now),
            Decision::Nothing,
            "a move inside the 30 s window is not a write"
        );
        match watch.decide(&state, now + PROGRESS_EVERY + Duration::from_millis(1)) {
            Decision::Save { position, .. } => assert_eq!(position, 40),
            other => panic!("30 s of playing must write, got {other:?}"),
        }
    }

    /// A mis-tap is not a resume point: under 8 s nothing is written, even on a
    /// pause.
    #[test]
    fn the_first_seconds_are_not_a_position() {
        let (mut state, _runtime) = app("keeper-min");
        queued(&mut state, 3.0);
        let mut watch = Watch::default();
        assert_eq!(watch.decide(&state, Instant::now()), Decision::Nothing);
        watch.playing = true;
        assert_eq!(
            watch.decide(&state, Instant::now()),
            Decision::Nothing,
            "a pause at 3 s is still a mis-tap"
        );
    }

    /// The last 15 s are the server's own rule (`_EPISODE_DONE_TAIL`).
    #[test]
    fn a_position_in_the_last_fifteen_seconds_is_finished() {
        let duration = 600u32;
        assert!(590 >= duration.saturating_sub(DONE_TAIL));
        assert!(580 < duration.saturating_sub(DONE_TAIL));
    }

    /// An `already_running` 409 is a job at work, not a failure to explain.
    #[test]
    fn an_already_running_transcription_is_read_as_a_job() {
        let error = ApiError::Http {
            status: 409,
            message: "cet épisode est déjà en cours de transcription".into(),
            code: None,
        };
        assert!(conflict_running(&error));
        assert!(transcribe_cause(&error).contains("déjà en cours"));
        let other = ApiError::Http {
            status: 409,
            message: "something else".into(),
            code: None,
        };
        assert!(!conflict_running(&other));
    }

    #[test]
    fn a_show_offers_five_ten_or_twenty_episodes_to_transcribe() {
        assert_eq!(batch_choices(3), vec![3]);
        assert_eq!(batch_choices(7), vec![5]);
        assert_eq!(batch_choices(12), vec![5, 10]);
        assert_eq!(batch_choices(40), vec![5, 10, 20]);
        assert!(batch_choices(0).is_empty());
    }

    #[test]
    fn a_query_carries_a_trimmed_position() {
        assert_eq!(trim_float(95.4), "95.4");
        assert_eq!(trim_float(95.0), "95");
        assert_eq!(trim_float(0.), "0");
        assert_eq!(trim_float(12.345), "12.35");
    }

    /// The head's cap, at the four sizes this lot was looked at. The person's
    /// complaint was a control cut in half above the first episode: the admin
    /// block folds whenever the head with it would not fit `HEAD_SHARE`.
    #[test]
    fn the_head_folds_its_admin_block_before_it_cuts_the_list() {
        // 1280×800: 656 px of page, 295 px of head. Compact hero, folded block.
        let small = head_fit(656.);
        assert!(small.compact, "1280×800 goes compact");
        assert!(
            !small.settings,
            "the admin block cannot fit 45 % of a 656 px page"
        );
        // 2560×1600: 1456 px of page, 655 px of head: everything fits.
        let big = head_fit(1456.);
        assert!(!big.compact, "a 1600 px window keeps the full hero");
        assert!(big.settings, "the admin block fits a 1456 px page");
        // 1920×1080: the full hero fits, the admin block does not.
        let hd = head_fit(936.);
        assert!(!hd.compact);
        assert!(!hd.settings, "a 936 px page folds the admin block");
        // The threshold itself: 900 px is where the compact hero ends.
        assert!(head_fit(899.).compact);
        assert!(!head_fit(900.).compact);
        // And the folded head always fits the cap it was decided against, at
        // every size this lot was looked at (400 px of page — a 544 px window —
        // is below any real window and the head scrolls there).
        for page in [656., 760., 936., 1_200., 1_456.] {
            let fit = head_fit(page);
            let hero = if fit.compact { 152. } else { 236. };
            let folded = hero + 44. + 34. + 40.;
            assert!(
                folded <= page * HEAD_SHARE,
                "the folded head must fit {page}: {folded}"
            );
        }
    }

    const ME: &str = r#"{"user":{"id":"u1","name":"bench","admin":true,"access":{},"sources":[]}}"#;

    fn episode_json(index: usize) -> String {
        format!(
            r#"{{"id":"e{index}","deezer_id":"e{index}","title":"Épisode {index}",
                "duration":1800,"channel_id":"c1","published":1794748649,
                "archived":true,"unavailable":false,"explicit":false,"podcast":true,
                "status":"completed","chapter_count":2,"transcript":null,
                "artist":{{"deezer_id":"c1","name":"Les Histoires"}},
                "album":{{"deezer_id":"c1","title":"Les Histoires","cover":null}},
                "cover":null}}"#
        )
    }

    fn page_json(range: std::ops::Range<usize>, next: Option<&str>) -> String {
        format!(
            r#"{{"id":"c1","deezer_id":"9400001","title":"Les Histoires",
                "description":"","cover":null,"subscribed":true,"local":false,
                "status":"ok","episode_count":60,"archived_count":60,
                "auto_archive":false,"auto_transcribe":false,
                "transcribe_language":"auto","transcribed_count":0,
                "episodes":[{}],"next":{}}}"#,
            range.map(episode_json).collect::<Vec<_>>().join(","),
            next.map(|next| format!("\"{next}\""))
                .unwrap_or_else(|| "null".to_owned())
        )
    }

    /// The acceptance's "episode list paged by cursor (no unbounded fetch)",
    /// counted in requests: ONE page on open, ONE more when the person asks for
    /// the next, and the second carries the cursor the server gave. The rows are
    /// windowed, so 50 episodes do not build 50 rows.
    #[gpui::test]
    async fn a_show_page_asks_one_page_and_the_next_one_needs_the_cursor(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::widgets::card::stub;

        let first = page_json(0..50, Some("cursor-1"));
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/podcast/c1", first.as_str()),
            ("/api/podcast/c1/markers", r#"{"markers":{}}"#),
            (
                "/api/transcribe/settings",
                r#"{"available":false,"reason":"disabled"}"#,
            ),
            ("/api/podcast/progress", r#"{"progress":{}}"#),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (screen, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| Screen::new(Route::Show { id: "c1".into() }, state, cx)
        });
        stub::until(&runtime, cx, "the first page", |cx| {
            screen.read(cx).episodes.len() == 50
        });
        assert_eq!(
            stub.count("/api/podcast/c1"),
            1,
            "one page, one request (never one per episode)"
        );
        assert_eq!(
            stub.queries("/api/podcast/c1"),
            vec!["limit=50".to_owned()],
            "the first page carries the limit and no cursor"
        );
        assert_eq!(
            stub.count("/api/podcast/c1/markers"),
            1,
            "the whole show's markers are ONE read, not one per episode"
        );
        assert_eq!(
            stub.count("/api/transcribe/settings"),
            1,
            "an admin's transcription card is one read"
        );
        let per_episode = stub
            .hits()
            .iter()
            .filter(|hit| hit.path.starts_with("/api/podcast/episode/"))
            .count();
        assert_eq!(
            per_episode, 0,
            "nothing is fetched per episode: no chapters or transcript before a panel opens"
        );

        let built = cx.update(|_, cx| screen.read(cx).rendered_rows);
        assert!(
            (1..=25).contains(&built),
            "a page of 50 rows builds what the window shows, it built {built}"
        );

        // The next page is the cursor's: one more request, no unbounded fetch.
        cx.update(|_, cx| {
            screen.update(cx, |screen, cx| screen.load_more(cx));
        });
        stub::until(&runtime, cx, "the second page", |_| {
            stub.count("/api/podcast/c1") == 2
        });
        let queries = stub.queries("/api/podcast/c1");
        assert_eq!(queries.len(), 2, "one page, one request");
        assert!(
            queries[1].contains("cursor=cursor-1"),
            "the second page carries the cursor the server gave: {:?}",
            queries[1]
        );
        runtime.shutdown_background();
        stub.stop();
    }

    /// The list must still paint BESIDE an open episode panel: a plain block
    /// wrapper gave the virtual list no definite height, so 50 episodes painted
    /// nothing at 1280 and 2560 while the panel took the width (found by the
    /// Xvfb pass, not by the suite).
    #[gpui::test]
    async fn the_episode_list_survives_the_panel_beside_it(cx: &mut gpui::TestAppContext) {
        use crate::widgets::card::stub;

        let page = page_json(0..50, None);
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/podcast/c1", page.as_str()),
            ("/api/podcast/c1/markers", r#"{"markers":{}}"#),
            (
                "/api/transcribe/settings",
                r#"{"available":false,"reason":"disabled"}"#,
            ),
            ("/api/podcast/progress", r#"{"progress":{}}"#),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (screen, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| Screen::new(Route::Show { id: "c1".into() }, state, cx)
        });
        stub::until(&runtime, cx, "the page", |cx| {
            screen.read(cx).episodes.len() == 50
        });
        assert!(cx.update(|_, cx| screen.read(cx).rendered_rows) > 0);

        cx.update(|_, cx| {
            screen.update(cx, |screen, cx| screen.open_panel(0, cx));
        });
        stub::until(&runtime, cx, "the panel", |cx| {
            screen.read(cx).panel.is_some()
        });
        let built = cx.update(|_, cx| screen.read(cx).rendered_rows);
        let wide = cx.update(|window, _| {
            f32::from(window.viewport_size().width) - SIDEBAR >= PANEL_MIN_CONTENT
        });
        assert!(
            wide,
            "this test drives the panel BESIDE the list: it needs the wide branch"
        );
        assert!(
            built > 0,
            "the list keeps painting beside the panel (it built {built} rows)"
        );
        runtime.shutdown_background();
        stub.stop();
    }

    /// The list's own geometry and a frame's cost, on the real screen: the head
    /// stays inside its cap, the list keeps a usable height, and a paint of a
    /// 50-episode page is measured (a person's click must answer at once).
    #[gpui::test]
    async fn the_head_leaves_the_list_its_room_and_a_frame_costs_little(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::widgets::card::stub;

        let page = page_json(0..50, None);
        let stub = stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/podcast/c1", page.as_str()),
            ("/api/podcast/c1/markers", r#"{"markers":{}}"#),
            (
                "/api/transcribe/settings",
                r#"{"available":false,"reason":"disabled"}"#,
            ),
            ("/api/podcast/progress", r#"{"progress":{}}"#),
        ]);
        let (state, runtime) = stub::logged_in(cx, &stub);
        let (screen, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| Screen::new(Route::Show { id: "c1".into() }, state, cx)
        });
        stub::until(&runtime, cx, "the page", |cx| {
            screen.read(cx).episodes.len() == 50
        });

        let draw = |cx: &mut gpui::VisualTestContext| {
            cx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            });
        };
        let mut samples = Vec::new();
        for _ in 0..12 {
            let started = std::time::Instant::now();
            draw(cx);
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        let median = samples[samples.len() / 2];
        let profile = if cfg!(debug_assertions) {
            "debug, unoptimised"
        } else {
            "release"
        };
        println!("show screen frame, 50 episodes windowed ({profile}): median {median:.1} ms");
        // Release is the acceptance's number (4.2 ms is a 240 Hz frame); a debug
        // build is bounded loosely so CI notices a regression, as D13 does.
        let budget = if cfg!(debug_assertions) { 400.0 } else { 8.0 };
        assert!(
            median < budget,
            "a paint of the episode list stays inside {budget:.0} ms: {median:.1} ms"
        );

        // The head never takes more than its share, and the list keeps the rest.
        // `debug_bounds` only records with `debug_assertions` (D9's pitfall).
        #[cfg(debug_assertions)]
        {
            draw(cx);
            let head = cx.debug_bounds("show-head").expect("the head is drawn");
            let list = cx
                .debug_bounds("show-episodes-box")
                .expect("the list box is drawn");
            let page =
                cx.update(|window, _| f32::from(window.viewport_size().height)) - SHELL_CHROME;
            assert!(
                f32::from(head.size.height) <= page * HEAD_SHARE + 1.0,
                "the head stays inside {} % of the page ({} of {page})",
                HEAD_SHARE * 100.,
                f32::from(head.size.height)
            );
            assert!(
                f32::from(list.size.height) >= 240.,
                "the list keeps at least 240 px ({} px)",
                f32::from(list.size.height)
            );
        }
        let built = cx.update(|_, cx| screen.read(cx).rendered_rows);
        assert!(
            built < 50,
            "a page of 50 rows is windowed, never all built ({built})"
        );

        runtime.shutdown_background();
        stub.stop();
    }

    /// What this screen holds, measured in BYTES on the test's own thread
    /// (`testmem`), for the worst case this lot can reach: a page of 50
    /// episodes, a full transcript page (200 lines) and the 100 hits of a show
    /// search. Nothing is kept per row beyond the model.
    #[test]
    fn a_page_a_transcript_and_a_search_are_what_the_screen_holds() {
        use crate::testmem::Meter;
        use crate::widgets::transcript_panel::Panel;
        use nz_model::{EpisodeTranscript, TranscriptHit, TranscriptSegment};

        let before = Meter::now();
        let page: PodcastPage = serde_json::from_str(&page_json(0..50, None)).expect("a show page");
        let segments: Vec<TranscriptSegment> = (0..200)
            .map(|index| TranscriptSegment {
                start: index as f64 * 15.,
                end: index as f64 * 15. + 15.,
                text: format!(
                    "Ligne {} d'une transcription de deux cents lignes, avec ses mots.",
                    index
                ),
            })
            .collect();
        let transcript = EpisodeTranscript {
            status: "ready".into(),
            archived: true,
            total: 200,
            limit: 200,
            segments,
            ..EpisodeTranscript::default()
        };
        let mut panel = Panel::new("e1", Some(true), 3600);
        panel.apply(transcript, transcript::Tone::User);
        let hits: Vec<TranscriptHit> = (0..100)
            .map(|index| TranscriptHit {
                idx: index,
                start: index as f64 * 15.,
                end: index as f64 * 15. + 15.,
                text: format!("Une ligne qui parle de mémoire, la {index}e du show."),
                episode: Some(nz_model::TranscriptEpisodeRef {
                    id: format!("e{}", index % 50).into(),
                    title: format!("Épisode {index}"),
                    ..Default::default()
                }),
                ..TranscriptHit::default()
            })
            .collect();
        let held = before.since();
        // 50 episodes, 200 transcript lines and 100 hits: the screen keeps the
        // models, never an element or an image per row.
        assert!(
            held < 512 * 1024,
            "a page, a transcript page and its hits must stay under 512 KB, they hold {held} B"
        );
        assert_eq!(page.episodes.len(), 50);
        assert!(panel.loaded());
        assert_eq!(hits.len(), 100);
        eprintln!("D11 screen holds {held} B for 50 episodes + 200 lines + 100 hits");
    }
}
