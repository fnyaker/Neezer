//! The transcript panel: one episode's words, following the playhead, searchable,
//! and — for an admin — the server job that produces or stops them.
//!
//! `webapp/src/components/TranscriptPanel.svelte` is the reference, and the rules
//! it taught are kept here:
//!
//! - **One page at a time, 200 lines** (`PAGE`), the server's own `limit`. A
//!   one-hour episode holds ~700 lines: the panel asks for the page around the
//!   playhead (`at`) and pages by `offset`, never the whole sidecar (rule 3).
//! - **It follows the audio, and it says so when it does not.** The active line is
//!   found by timestamp, and a person who scrolls or searches leaves the follow
//!   (the pane stops jumping under their eyes); « Revenir à la lecture » puts it
//!   back. The jump is one `scroll_to_item`, never a rebuild.
//! - **A seek is the screen's**: clicking a line, a hit or a chapter asks the
//!   screen for `Ask::Seek`, exactly like a click on the progress bar.
//! - **The job is a job**: `queued`/`running` show their state, their progress and
//!   each failure's cause in French; starting or stopping is `POST …/transcribe`
//!   and the panel POLLS (10 s) while a job is active, then stops.
//! - **The text stays on the device.** A page read is kept in the API cache under
//!   its own path, so a panel opened offline paints the last copy and says
//!   « Hors ligne — dernière copie » instead of an empty box (rule 3).
//! - **Local transcripts have their place here** (T4): [`LocalTranscript`] and
//!   [`set_local_source`] are the seam. When the server has no transcript for an
//!   episode — or the link is dead — the panel shows the one produced on THIS
//!   machine (Vulkan), badges its origin, and searches it in memory. T4 fills the
//!   hook with `nz-transcribe`'s sidecar reader; nothing in this file knows how a
//!   sidecar is read, and `nz-transcribe` is not touched by this lot.
//!
//! Nothing here is async: [`Panel`] is state plus a renderer, and the screen that
//! owns it performs the requests and hands the answers back.

use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};

use gpui_kit::base::input::InputState;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement, Pixels, Render, ScrollStrategy, SharedString, Size,
    StatefulInteractiveElement as _, Styled, Window, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon, icon_button, virtual_list};
use nz_api::ApiError;
use nz_model::{EpisodeTranscript, TranscriptSegment};

use crate::state::AppState;
use crate::theme;
use crate::widgets::card::encode;
use crate::widgets::track_list::duration_label;

/// Lines per page: the server's own default upper bound (`PAGE_DEFAULT` is 200
/// in `neezer/webui/transcribe.py`).
pub const PAGE: u32 = 200;
/// Hits a search inside one episode asks for (`SEARCH_PAGE` in the SPA).
pub const SEARCH_PAGE: u32 = 100;
/// The shortest term worth a request (`lib/transcript.js`).
pub const MIN_QUERY: usize = 2;
/// The statuses that mean "a job is at work".
pub const ACTIVE: [&str; 3] = ["queued", "leased", "running"];
/// The statuses that mean "the job is over", whatever it produced.
pub const TERMINAL: [&str; 3] = ["ready", "error", "stopped"];
/// One line's height. A transcript line is a timestamp and one line of speech;
/// a fixed height is what makes 200 of them a windowed list.
pub const ROW_HEIGHT: f32 = 30.;

/// The languages the server accepts (`transcribe.LANGUAGES`), with their French
/// names (`lib/transcript.js#LANGUAGE_LABELS`).
pub const LANGUAGES: [&str; 7] = ["auto", "fr", "en", "es", "de", "it", "pt"];

pub fn language_label(code: &str) -> String {
    match code.to_ascii_lowercase().as_str() {
        "" => String::new(),
        "auto" => "Détection automatique".to_owned(),
        "fr" => "Français".to_owned(),
        "en" => "Anglais".to_owned(),
        "es" => "Espagnol".to_owned(),
        "de" => "Allemand".to_owned(),
        "it" => "Italien".to_owned(),
        "pt" => "Portugais".to_owned(),
        other => other.to_ascii_uppercase(),
    }
}

/// Why the server cannot transcribe, in French — `lib/transcript.js#reasonText`,
/// the same four causes the settings screen prints.
pub fn reason_text(reason: &str) -> String {
    match reason {
        "disabled" => "La transcription est désactivée sur ce serveur.".to_owned(),
        "engine_missing" => {
            "Le moteur de transcription n'est pas installé sur le serveur.".to_owned()
        }
        "model_missing" => "Le modèle de transcription est introuvable sur le serveur.".to_owned(),
        "model_invalid" => {
            "Le fichier de modèle n'est pas un modèle whisper.cpp valide.".to_owned()
        }
        _ => "La transcription n'est pas configurée sur ce serveur.".to_owned(),
    }
}

/// Is a job at work?
pub fn is_active(status: &str) -> bool {
    ACTIVE.contains(&status)
}

/// Is the job over?
pub fn is_terminal(status: &str) -> bool {
    TERMINAL.contains(&status)
}

// --- T4's seam: a transcript produced on this machine ------------------------

/// A transcript `nz-transcribe` produced locally (T4): the sidecar's content,
/// already in the server's segment shape, so the panel cannot tell the two
/// apart except by the badge it shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalTranscript {
    /// `"whisper.cpp"`, as the crate writes it.
    pub engine: Option<String>,
    pub model: Option<String>,
    /// `"fr"`, `"auto"`, … from the model's own verdict.
    pub language: Option<String>,
    /// The segments, in absolute milliseconds in the sidecar; this type carries
    /// them as the panel does — seconds from the episode's start.
    pub segments: Vec<TranscriptSegment>,
    /// ISO-8601 of the local run, for the line that says when.
    pub created: Option<String>,
}

impl LocalTranscript {
    /// The sidecar's absolute milliseconds → the panel's seconds, once.
    pub fn from_milliseconds(
        segments: impl IntoIterator<Item = (u64, u64, String)>,
        engine: Option<String>,
        model: Option<String>,
        language: Option<String>,
    ) -> Self {
        Self {
            engine,
            model,
            language,
            segments: segments
                .into_iter()
                .map(|(start_ms, end_ms, text)| TranscriptSegment {
                    start: start_ms as f64 / 1_000.0,
                    end: end_ms as f64 / 1_000.0,
                    text,
                })
                .collect(),
            created: None,
        }
    }
}

/// How a local transcript is found for an episode id: T4 installs the reader
/// (a sidecar under `$XDG_DATA_HOME/neezer/transcripts/`), and returns `None`
/// for an episode this machine never transcribed.
pub type LocalSource = Arc<dyn Fn(&str) -> Option<LocalTranscript> + Send + Sync>;

static LOCAL_SOURCE: OnceLock<Mutex<Option<LocalSource>>> = OnceLock::new();

fn local_slot() -> &'static Mutex<Option<LocalSource>> {
    LOCAL_SOURCE.get_or_init(|| Mutex::new(None))
}

/// Install (or remove, with `None`) the local reader. Called once, by T4.
pub fn set_local_source(source: Option<LocalSource>) {
    if let Ok(mut slot) = local_slot().lock() {
        *slot = source;
    }
}

/// The local transcript of an episode, when this machine has one.
pub fn local_for(episode_id: &str) -> Option<LocalTranscript> {
    let reader = local_slot().lock().ok().and_then(|slot| slot.clone())?;
    let answer = reader(episode_id)?;
    (!answer.segments.is_empty()).then_some(answer)
}

// --- what the panel asks the screen for -------------------------------------

/// What a click or a key in the panel wants. The screen performs it: it owns the
/// network, the transport and the toasts.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// Load the page around this position (seconds).
    OpenAt(f64),
    /// Another page of lines, `delta` pages from the current one.
    Page(i32),
    /// A `Réessayer` of a failed load.
    Retry,
    /// Run the term in the field, now (Enter).
    Search,
    /// Forget the term and the hits.
    ClearSearch,
    /// Play from a hit's moment and show the lines around it.
    PickHit(f64),
    /// Start (or resume) the server job.
    Start,
    /// Stop the running job.
    Stop,
    /// Ask before re-running the job with another language.
    Relaunch,
    /// Re-run the job with the language the panel holds.
    RelaunchConfirm,
    /// Ask before deleting the server transcript.
    Delete,
    /// Delete the server transcript, as confirmed.
    DeleteConfirm,
    /// Close the confirmation.
    Cancel,
    /// Pick a language for the relaunch.
    Language(String),
    /// Play (or seek) from a line, in seconds.
    Seek(f64),
    /// Follow the playhead again.
    Follow,
}

/// One request the panel wants performed. The screen builds it from `Api`
/// primitives; the panel builds the PATH, so the cached key and the requested key
/// cannot differ by a byte.
#[derive(Clone, Debug, PartialEq)]
pub enum Fetch {
    /// `GET /podcast/episode/<id>/transcript?…`
    Page {
        path: String,
        lane_hint: bool,
        at: Option<f64>,
        offset: u32,
    },
    /// `GET …/transcript?q=` — a search inside this episode.
    Search { path: String, term: String },
}

/// The confirmation the panel is asking for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    /// Re-run the job (the text is replaced).
    Relaunch,
    /// Delete the server transcript (the audio stays).
    Delete,
}

/// What a page answer means for the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// A first read, or a page the person asked for.
    User,
    /// A poll of a running job: background, one request, never a storm.
    Poll,
}

/// One episode's transcript, as the panel holds it.
pub struct Panel {
    pub episode: String,
    /// The episode's audio is on disk — the job needs it.
    pub archived: Option<bool>,
    /// The episode's length, from the show page (ms once the server answers).
    pub duration_ms: u64,
    pub status: String,
    pub progress: f64,
    pub done_ms: u64,
    pub language: Option<String>,
    pub engine: Option<String>,
    pub model: Option<String>,
    pub source: Option<String>,
    pub worker: Option<String>,
    pub segments: Vec<TranscriptSegment>,
    pub total: u32,
    pub offset: u32,
    pub limit: u32,
    pub query: Option<String>,
    /// The load failed: the cause, and whether the link was the reason.
    pub failed: Option<ApiError>,
    pub offline: bool,
    /// The page came off the device (the API cache) and has not been refreshed.
    pub from_cache: bool,
    /// A job's own error code and message (an admin's cause).
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    /// The transcript produced on this machine, when there is one (T4).
    pub local: Option<LocalTranscript>,
    /// Another page or a search is in flight.
    pub loading: bool,
    pub search: Search,
    /// Follow the playhead (paused by a scroll or a search, resumed by « Revenir
    /// à la lecture »).
    pub follow: bool,
    /// The line under the playhead.
    pub active: Option<usize>,
    /// The line `Entrée` would play, when the keyboard is in the list.
    pub cursor: Option<usize>,
    /// A language the person picked for a relaunch.
    pub relaunch_language: String,
    /// The confirmation on screen, if any.
    pub confirm: Option<Confirm>,
    /// The lines' scroll position, kept across paints.
    scroll: gpui_kit::base::VirtualListScrollHandle,
    /// The panel is showing a page it read; `at` is where the playhead was.
    scrolled_to: Option<usize>,
    sizes: Rc<Vec<Size<Pixels>>>,
    sizes_for: usize,
}

impl Panel {
    pub fn new(episode_id: &str, archived: Option<bool>, duration_s: u32) -> Self {
        Self {
            episode: episode_id.to_owned(),
            archived,
            duration_ms: u64::from(duration_s) * 1_000,
            status: "none".to_owned(),
            progress: 0.0,
            done_ms: 0,
            language: None,
            engine: None,
            model: None,
            source: None,
            worker: None,
            segments: Vec::new(),
            total: 0,
            offset: 0,
            limit: PAGE,
            query: None,
            failed: None,
            offline: false,
            from_cache: false,
            error_code: None,
            error_message: None,
            local: None,
            loading: false,
            search: Search::default(),
            follow: true,
            active: None,
            cursor: None,
            relaunch_language: "auto".to_owned(),
            confirm: None,
            scroll: gpui_kit::base::VirtualListScrollHandle::new(),
            scrolled_to: None,
            sizes: Rc::new(Vec::new()),
            sizes_for: usize::MAX,
        }
    }

    // -- the requests -------------------------------------------------------

    /// The page to read when the panel opens: around the playhead when it is
    /// known, at the top otherwise. `None` once a page is here.
    pub fn open(&mut self, at: Option<f64>) -> Option<Fetch> {
        if self.loading {
            return None;
        }
        if self.loaded() && at.map(|at| self.covers(at)).unwrap_or(true) {
            return None;
        }
        Some(self.page_fetch(at, 0))
    }

    /// A poll of a job at work: one line, `limit=1`, which is the cheapest way to
    /// learn `status`/`progress` (`lib/transcript.js` does the same).
    pub fn poll(&self) -> Option<Fetch> {
        if !is_active(&self.status) {
            return None;
        }
        Some(Fetch::Page {
            path: format!("{}?limit=1&offset=0", self.path()),
            lane_hint: true,
            at: None,
            offset: 0,
        })
    }

    fn path(&self) -> String {
        format!("/podcast/episode/{}/transcript", encode(&self.episode))
    }

    pub fn page_fetch(&mut self, at: Option<f64>, offset: u32) -> Fetch {
        self.loading = true;
        let path = match at {
            Some(at) => format!(
                "{}?limit={}&at={}",
                self.path(),
                self.limit,
                trim_float(at.max(0.0))
            ),
            None => format!("{}?limit={}&offset={offset}", self.path(), self.limit),
        };
        Fetch::Page {
            path,
            lane_hint: false,
            at,
            offset,
        }
    }

    /// The search inside this episode, or a local one when the link is dead or
    /// the text is the machine's own.
    pub fn search(&mut self, term: &str) -> Option<Fetch> {
        let term = term.trim().to_owned();
        if term.chars().count() < MIN_QUERY {
            self.search.clear();
            return None;
        }
        self.search.term = term.clone();
        self.follow = false;
        // A local transcript is on this machine in full: search it in memory.
        let lines = self.lines().to_vec();
        if self
            .search
            .locally(&lines, &term, self.local.is_some())
            .is_some()
        {
            return None;
        }
        self.search.busy = true;
        self.search.error = None;
        Some(Fetch::Search {
            path: format!("{}?q={}&limit={}", self.path(), encode(&term), SEARCH_PAGE),
            term,
        })
    }

    /// Adopt a page. A `status: "none"` answer with a local transcript swaps to
    /// the machine's copy — that is T4's whole integration.
    pub fn apply(&mut self, payload: EpisodeTranscript, tone: Tone) {
        self.loading = false;
        self.confirm = None;
        self.failed = None;
        self.offline = false;
        self.from_cache = false;
        if !payload.status.is_empty() {
            self.status = payload.status.clone();
        }
        if payload.archived {
            self.archived = Some(true);
        } else if payload.status != "none" {
            self.archived = Some(false);
        }
        self.progress = payload.progress.clamp(0.0, 1.0);
        self.done_ms = payload.done_ms;
        if payload.duration_ms > 0 {
            self.duration_ms = payload.duration_ms;
        }
        if payload.language.is_some() {
            self.language = payload.language.clone();
        }
        if payload.engine.is_some() {
            self.engine = payload.engine.clone();
        }
        if payload.model.is_some() {
            self.model = payload.model.clone();
        }
        if payload.source.is_some() {
            self.source = payload.source.clone();
        }
        if payload.worker.is_some() {
            self.worker = payload.worker.clone();
        }
        self.error_code = payload.error.as_ref().map(|error| error.code.clone());
        self.error_message = payload
            .error
            .as_ref()
            .and_then(|error| error.message.clone());
        if payload.status == "none" && payload.query.is_none() {
            // No server transcript: the machine may have one (T4).
            self.local = local_for(&self.episode);
            if self.local.is_some() {
                self.status = "local".to_owned();
                self.segments.clear();
                self.total = 0;
            }
            return;
        }
        if payload.query.is_some() {
            return; // a search answered through `apply_search`
        }
        if tone == Tone::Poll && !self.loaded() && payload.segments.is_empty() {
            return; // a poll of a job with nothing written yet: keep the state only
        }
        self.segments = payload.segments;
        self.total = payload.total.max(self.segments.len() as u32);
        self.offset = payload.offset;
        if payload.limit > 0 {
            self.limit = payload.limit;
        }
        self.query = payload.query;
        self.local = None;
        self.search.clear();
        if self.follow {
            self.scrolled_to = None;
        }
    }

    /// Adopt a search's hits.
    pub fn apply_search(&mut self, payload: EpisodeTranscript, term: &str) {
        self.search.busy = false;
        self.search.error = None;
        self.search.local = false;
        self.search.term = term.to_owned();
        self.search.hits = payload.segments;
        self.search.total = payload.total.max(self.search.hits.len() as u32);
        self.search.searched = true;
    }

    /// The page read failed: the answer already on screen stays, the reason is a
    /// line above it, and « Réessayer » asks again.
    pub fn fail(&mut self, error: ApiError) {
        self.loading = false;
        self.confirm = None;
        self.offline = matches!(error, ApiError::Offline);
        if self.segments.is_empty() && self.local.is_none() {
            self.failed = Some(error.clone());
        }
        if self.offline && self.local.is_none() {
            self.local = local_for(&self.episode);
            if self.local.is_some() {
                self.status = "local".to_owned();
            }
        }
        self.search.busy = false;
    }

    /// A page kept on the device: painted at once, refreshed right after.
    pub fn stale(&mut self, payload: EpisodeTranscript) {
        self.apply(payload, Tone::Poll);
        self.from_cache = true;
    }

    // -- the playhead -------------------------------------------------------

    /// Has a page at all?
    pub fn loaded(&self) -> bool {
        !self.segments.is_empty() || self.status == "ready"
    }

    /// Does the page on screen hold `seconds`?
    pub fn covers(&self, seconds: f64) -> bool {
        match (self.segments.first(), self.segments.last()) {
            (Some(first), Some(last)) => {
                let step = if self.segments.len() > 1 {
                    last.start - self.segments[self.segments.len() - 2].start
                } else {
                    30.0
                };
                seconds >= first.start && seconds <= last.start + step.max(1.0)
            }
            _ => false,
        }
    }

    /// The line under `seconds`, or `None` before the first one.
    pub fn at(&self, seconds: f64) -> Option<usize> {
        segment_at(&self.segments, seconds)
    }

    /// Follow the playhead: the active line, and whether it moved. A page that no
    /// longer holds the playhead is not re-read by this (the screen decides).
    pub fn follow_to(&mut self, seconds: f64) -> bool {
        let active = self.at(seconds);
        if active == self.active {
            return false;
        }
        self.active = active;
        true
    }

    /// Leaving the follow (a scroll, a search): the pane stops moving under the
    /// person's eyes.
    pub fn pause_follow(&mut self) {
        self.follow = false;
    }

    pub fn resume_follow(&mut self) {
        self.follow = true;
        self.scrolled_to = None;
    }

    /// The text to show, and where it came from: the server's page or the
    /// machine's whole transcript.
    fn lines(&self) -> &[TranscriptSegment] {
        match &self.local {
            Some(local) if self.segments.is_empty() => &local.segments,
            _ => &self.segments,
        }
    }

    pub fn is_local(&self) -> bool {
        self.local.is_some()
    }

    /// The percent a job line prints.
    pub fn percent(&self) -> u32 {
        (self.progress.clamp(0.0, 1.0) * 100.0).round() as u32
    }

    fn rebuild_sizes(&mut self) {
        let len = self.lines().len();
        if self.sizes_for == len && self.sizes.len() == len {
            return;
        }
        self.sizes_for = len;
        self.sizes = Rc::new(vec![
            Size {
                width: px(0.),
                height: px(ROW_HEIGHT),
            };
            len
        ]);
    }
}

/// One search inside the panel: the term, the hits, and whether they came off
/// the whole transcript in memory (offline, or a local one).
#[derive(Default)]
pub struct Search {
    pub term: String,
    pub hits: Vec<TranscriptSegment>,
    pub total: u32,
    pub busy: bool,
    pub error: Option<ApiError>,
    /// The search ran over what the device holds, not the whole transcript.
    pub local: bool,
    /// A search has produced an answer (so the panel shows hits, not lines).
    pub searched: bool,
}

impl Search {
    pub fn clear(&mut self) {
        self.term.clear();
        self.hits.clear();
        self.total = 0;
        self.busy = false;
        self.error = None;
        self.local = false;
        self.searched = false;
    }

    pub fn active(&self) -> bool {
        !self.term.trim().is_empty()
    }

    /// Search the segments already in memory. Returns `Some(())` when the search
    /// was answered locally (a local transcript, or a dead link), `None` when the
    /// server must be asked.
    pub fn locally(
        &mut self,
        segments: &[TranscriptSegment],
        term: &str,
        local: bool,
    ) -> Option<()> {
        if !local {
            return None;
        }
        let words = fold_words(term);
        let hits: Vec<TranscriptSegment> = segments
            .iter()
            .filter(|segment| {
                let folded = fold_text(&segment.text);
                words.iter().all(|word| folded.contains(word))
            })
            .map(|segment| TranscriptSegment {
                start: segment.start,
                end: segment.end,
                text: excerpt_around(&segment.text, term, 240),
            })
            .collect();
        self.hits = hits;
        self.total = self.hits.len() as u32;
        self.busy = false;
        self.error = None;
        self.local = true;
        self.searched = true;
        Some(())
    }
}

/// Accent- and case-insensitive folding, `lib/transcript.js#foldText`: NFD, lower
/// case, combining marks dropped. Rust has no `normalize`, so the map below is
/// the Latin-1 + Latin Extended-A set the SPA's transcripts actually contain —
/// a term is a French or English word far more often than a Devanagari one.
pub fn fold_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        let folded = match character {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => {
                'a'
            }
            'æ' | 'Æ' => 'e',
            'ç' | 'Ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => 'i',
            'ñ' | 'Ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => {
                'o'
            }
            'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => 'u',
            'ý' | 'ÿ' | 'Ý' | 'Ÿ' => 'y',
            'ß' => 's',
            'œ' | 'Œ' => 'e',
            other => other,
        };
        for lower in folded.to_lowercase() {
            out.push(lower);
        }
    }
    out
}

fn fold_words(term: &str) -> Vec<String> {
    fold_text(term)
        .split_whitespace()
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The excerpt a local search shows around the first match
/// (`lib/transcript.js#excerptAround`).
pub fn excerpt_around(text: &str, term: &str, max: usize) -> String {
    let value: Vec<char> = text.chars().collect();
    if value.len() <= max {
        return text.to_owned();
    }
    let folded = fold_text(text);
    let folded_chars: Vec<char> = folded.chars().collect();
    let mut found: Option<usize> = None;
    for word in fold_words(term) {
        let needle: Vec<char> = word.chars().collect();
        if needle.is_empty() || needle.len() > folded_chars.len() {
            continue;
        }
        for at in 0..=(folded_chars.len() - needle.len()) {
            if folded_chars[at..at + needle.len()] == needle[..] {
                found = Some(match found {
                    Some(best) => best.min(at),
                    None => at,
                });
                break;
            }
        }
    }
    let Some(found) = found else {
        return value.iter().take(max.saturating_sub(1)).collect::<String>() + "…";
    };
    let start = found.saturating_sub(max / 2);
    let end = (start + max).min(value.len());
    let start = end.saturating_sub(max);
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(value[start..end].iter());
    if end < value.len() {
        out.push('…');
    }
    out
}

/// The line at `seconds` (`lib/transcript.js#segmentAt`), by bisection: a page is
/// 200 lines and the playhead asks on every tick.
pub fn segment_at(segments: &[TranscriptSegment], seconds: f64) -> Option<usize> {
    if segments.is_empty() || !seconds.is_finite() {
        return None;
    }
    let mut low = 0usize;
    let mut high = segments.len() - 1;
    let mut found = None;
    while low <= high {
        let middle = (low + high) / 2;
        if segments[middle].start <= seconds {
            found = Some(middle);
            low = middle + 1;
        } else {
            if middle == 0 {
                break;
            }
            high = middle - 1;
        }
    }
    found
}

/// The parts of a line, with the matched words marked
/// (`lib/transcript.js#highlightParts`): the whole term first, then each word,
/// accent-insensitively.
pub fn highlight_parts(text: &str, term: &str) -> Vec<(String, bool)> {
    let chars: Vec<char> = text.chars().collect();
    let folded: Vec<char> = fold_text(text).chars().collect();
    if chars.is_empty() || term.trim().is_empty() || folded.len() != chars.len() {
        return vec![(text.to_owned(), false)];
    }
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut needles: Vec<Vec<char>> = Vec::new();
    let whole = fold_text(term.trim());
    if !whole.is_empty() {
        needles.push(whole.chars().collect());
    }
    for word in fold_words(term) {
        let needle: Vec<char> = word.chars().collect();
        if !needles.contains(&needle) {
            needles.push(needle);
        }
    }
    for needle in &needles {
        if needle.is_empty() || needle.len() > folded.len() {
            continue;
        }
        let mut from = 0usize;
        while from + needle.len() <= folded.len() {
            if folded[from..from + needle.len()] == needle[..] {
                ranges.push((from, from + needle.len()));
                from += needle.len();
            } else {
                from += 1;
            }
        }
        if !ranges.is_empty() {
            break; // the whole term matched: that is the highlight
        }
    }
    if ranges.is_empty() {
        return vec![(text.to_owned(), false)];
    }
    ranges.sort_by_key(|(start, end)| (*start, std::cmp::Reverse(*end)));
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start < last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    let mut parts = Vec::new();
    let mut at = 0usize;
    for (start, end) in merged {
        if start > at {
            parts.push((chars[at..start].iter().collect(), false));
        }
        parts.push((chars[start..end].iter().collect(), true));
        at = end;
    }
    if at < chars.len() {
        parts.push((chars[at..].iter().collect(), false));
    }
    parts
}

/// `12.5` → `"12.5"`, `12.0` → `"12"`: the query a path carries.
fn trim_float(value: f64) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    if (rounded - rounded.round()).abs() < f64::EPSILON {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    }
}

// --- the panel's surface -----------------------------------------------------

/// The panel's whole surface: the job's state, the tools, the lines.
///
/// `Ask` tells the screen what the person wants; `is_admin` decides which job
/// controls are offered (the server enforces it either way).
#[allow(clippy::too_many_arguments)]
pub fn view<V: Render>(
    panel: &mut Panel,
    entity: Entity<V>,
    state: &Entity<AppState>,
    field: Option<&Entity<InputState>>,
    is_admin: bool,
    window: &mut Window,
    cx: &mut Context<V>,
    ask: impl Fn(&mut V, Ask, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    panel.rebuild_sizes();
    let online = state.read(cx).online;
    let mut column = div()
        .id("transcript-panel")
        .flex()
        .flex_col()
        .w_full()
        .min_w_0()
        .gap(rems(0.5));

    if let Some(card) = status_card(panel, is_admin, online, cx, ask.clone()) {
        column = column.child(card);
    }

    if panel.is_local() && panel.search.term.trim().is_empty() {
        column = column.child(local_badge(panel, cx));
    }

    if panel.lines().is_empty() {
        return column.into_any_element();
    }

    column = column.child(tools(panel, field, is_admin, window, cx, ask.clone()));
    column = column.child(confirm_block(panel, cx, ask.clone()));

    if let Some(error) = panel.search.error.clone() {
        column = column.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.25))
                .text_size(rems(0.6875))
                .text_color(theme::danger(cx))
                .child(icon(IconName::CircleAlert).size(rems(0.75)))
                .child(SharedString::from(format!(
                    "La recherche a échoué : {}",
                    crate::widgets::card::cause(&error)
                ))),
        );
    }

    // Follow, or come back to it: one line, always in the same place.
    if !panel.follow && other_pane_open() {
        column = column.child(
            div()
                .id("transcript-follow")
                .flex()
                .items_center()
                .gap(rems(0.25))
                .self_start()
                .h(rems(1.75))
                .px(rems(0.5))
                .border_1()
                .border_color(theme::accent(cx))
                .text_size(rems(0.6875))
                .text_color(theme::accent(cx))
                .cursor_pointer()
                .hover(|style| style.bg(theme::chrome(cx)))
                .on_click({
                    let ask = ask.clone();
                    cx.listener(move |this, _, _, cx| ask(this, Ask::Follow, cx))
                })
                .child(icon(IconName::Play).size(rems(0.75)))
                .child("Revenir à la lecture"),
        );
    }

    if panel.search.active() {
        column = column.child(search_block(panel, cx, ask.clone()));
        return column.into_any_element();
    }

    // The lines: windowed at a fixed height, so 200 of them are the rows on
    // screen and two more, not 200 elements.
    let active = panel.active;
    let follow = panel.follow;
    let should_scroll = follow && panel.active.is_some() && panel.scrolled_to != panel.active;
    panel.scrolled_to = if follow { panel.active } else { None };
    let term = panel.search.term.clone();
    let sizes = panel.sizes.clone();
    let scroll = panel.scroll.clone();
    column = column.child(div().flex_1().min_h_0().w_full().child(segments(
        panel,
        entity,
        active,
        sizes,
        &scroll,
        cx,
        ask.clone(),
        term,
        should_scroll,
    )));

    if panel.total > panel.limit || panel.offset > 0 {
        column = column.child(pager(panel, cx, ask));
    }
    column.into_any_element()
}

/// A person who scrolls the lines leaves the follow; the button that puts it back
/// is drawn from that flag. Follow clocks are per panel, so this is always true
/// at the widget level — the screen decides whether the episode is playing.
fn other_pane_open() -> bool {
    true
}

fn local_badge(panel: &Panel, cx: &App) -> AnyElement {
    let engine = panel
        .local
        .as_ref()
        .and_then(|local| local.engine.clone())
        .unwrap_or_else(|| "whisper.cpp".to_owned());
    let model = panel
        .local
        .as_ref()
        .and_then(|local| local.model.clone())
        .unwrap_or_default();
    let line = if model.is_empty() {
        format!("Transcrit sur cet ordinateur ({engine})")
    } else {
        format!("Transcrit sur cet ordinateur ({engine}, {model})")
    };
    div()
        .flex()
        .items_center()
        .gap(rems(0.25))
        .text_size(rems(0.6875))
        .text_color(theme::secondary(cx))
        .child(icon(IconName::Cpu).size(rems(0.75)))
        .child(SharedString::from(line))
        .into_any_element()
}

/// What the panel says when there is no text to read: the job's state, each
/// failure with its cause, and the one button that changes it.
fn status_card<V: Render>(
    panel: &Panel,
    is_admin: bool,
    online: bool,
    cx: &mut Context<V>,
    ask: impl Fn(&mut V, Ask, &mut Context<V>) + Clone + 'static,
) -> Option<AnyElement> {
    let has_lines = !panel.lines().is_empty();

    // A load that failed with nothing to show: the cause and « Réessayer ».
    if let Some(error) = &panel.failed {
        let cause = if panel.offline || !online {
            "La transcription a besoin du serveur : le texte reste sur le serveur.".to_owned()
        } else {
            crate::widgets::card::cause(error).to_string()
        };
        return Some(card_body(
            if panel.offline || !online {
                IconName::CloudOff
            } else {
                IconName::TriangleAlert
            },
            "Transcription indisponible".to_owned(),
            cause,
            None,
            vec![("transcript-retry", "Réessayer").into()],
            cx,
            ask,
            Ask::Retry,
        ));
    }

    match panel.status.as_str() {
        "none" => {
            if panel.archived == Some(false) {
                return Some(card_body(
                    IconName::CloudOff,
                    "Épisode non archivé".to_owned(),
                    "L'épisode doit d'abord être archivé sur le serveur pour être transcrit."
                        .to_owned(),
                    None,
                    Vec::new(),
                    cx,
                    ask,
                    Ask::Retry,
                ));
            }
            if !is_admin {
                return Some(card_body(
                    IconName::ScrollText,
                    "Aucune transcription".to_owned(),
                    "Aucune transcription pour cet épisode pour l'instant.".to_owned(),
                    None,
                    Vec::new(),
                    cx,
                    ask,
                    Ask::Retry,
                ));
            }
            let note = if panel.duration_ms > 0 {
                Some(format!(
                    "Un épisode de {} demande souvent plus de temps de calcul que sa durée ; le serveur continue si vous quittez cet écran.",
                    duration_label((panel.duration_ms / 1_000) as u32)
                ))
            } else {
                None
            };
            return Some(card_body(
                IconName::ScrollText,
                "Transcrire cet épisode".to_owned(),
                "Le texte est produit sur le serveur, à partir du fichier archivé. Aucun audio n'est envoyé à un tiers.".to_owned(),
                note,
                vec![("transcript-start", "Transcrire cet épisode").into()],
                cx,
                ask,
                Ask::Start,
            ));
        }
        "queued" => {
            if has_lines {
                return None;
            }
            return Some(banner(
                panel,
                "En attente de transcription…",
                "La file avance ; le serveur continue si vous quittez cet écran.",
                is_admin,
                cx,
                ask,
            ));
        }
        "running" | "leased" => {
            if has_lines {
                return Some(banner(
                    panel,
                    &format!("Transcription en cours — {} %", panel.percent().min(99)),
                    &format!(
                        "{} min transcrites sur {}",
                        panel.done_ms / 60_000,
                        minutes_label(panel.duration_ms)
                    ),
                    is_admin,
                    cx,
                    ask,
                ));
            }
            return Some(banner(
                panel,
                &format!("Transcription en cours — {} %", panel.percent()),
                &format!(
                    "{} min transcrites sur {}. Elle se met en pause pendant que vous écoutez.",
                    panel.done_ms / 60_000,
                    minutes_label(panel.duration_ms)
                ),
                is_admin,
                cx,
                ask,
            ));
        }
        "stopped" => {
            if has_lines {
                return Some(banner(
                    panel,
                    &format!("Transcription arrêtée à {} %.", panel.percent()),
                    "Le texte déjà transcrit reste lisible.",
                    is_admin,
                    cx,
                    ask,
                ));
            }
            return Some(card_body(
                IconName::CirclePause,
                format!("Transcription arrêtée à {} %.", panel.percent()),
                "Le texte déjà transcrit reste sur le serveur.".to_owned(),
                None,
                if is_admin {
                    vec![("transcript-resume", "Reprendre").into()]
                } else {
                    Vec::new()
                },
                cx,
                ask,
                Ask::Start,
            ));
        }
        "error" => {
            let message = match (is_admin, &panel.error_message) {
                (true, Some(message)) if !message.is_empty() => message.clone(),
                _ => "La transcription s'est arrêtée avant la fin.".to_owned(),
            };
            let note = panel
                .error_code
                .clone()
                .filter(|code| is_admin && !code.is_empty())
                .map(|code| format!("Code : {code}"));
            return Some(card_body(
                IconName::TriangleAlert,
                message,
                "Le texte déjà transcrit reste lisible.".to_owned(),
                note,
                if is_admin {
                    vec![("transcript-retry", "Réessayer").into()]
                } else {
                    Vec::new()
                },
                cx,
                ask,
                Ask::Start,
            ));
        }
        "ready" if !has_lines => {
            return Some(card_body(
                IconName::ScrollText,
                "Transcription vide".to_owned(),
                "Aucune parole n'a été reconnue dans cet épisode.".to_owned(),
                None,
                Vec::new(),
                cx,
                ask,
                Ask::Retry,
            ));
        }
        _ => {}
    }
    if !has_lines && panel.loading {
        return Some(
            div()
                .flex()
                .flex_col()
                .gap(rems(0.25))
                .child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child("Chargement de la transcription…"),
                )
                .into_any_element(),
        );
    }
    None
}

fn minutes_label(duration_ms: u64) -> String {
    if duration_ms == 0 {
        "?".to_owned()
    } else {
        format!("{} min", duration_ms / 60_000)
    }
}

/// The two buttons a card offers, named so a test can find them.
struct CardButton {
    id: &'static str,
    label: String,
}

impl From<(&'static str, &'static str)> for CardButton {
    fn from((id, label): (&'static str, &'static str)) -> Self {
        Self {
            id,
            label: label.to_owned(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn card_body<V: Render>(
    glyph: IconName,
    title: String,
    body: String,
    note: Option<String>,
    buttons: Vec<CardButton>,
    cx: &mut Context<V>,
    ask: impl Fn(&mut V, Ask, &mut Context<V>) + Clone + 'static,
    action: Ask,
) -> AnyElement {
    let mut card = div()
        .id("transcript-state")
        .flex()
        .flex_col()
        .items_start()
        .gap(rems(0.375))
        .p(rems(0.75))
        .border_1()
        .border_color(theme::border(cx))
        .bg(theme::chrome(cx))
        .w_full()
        .min_w_0()
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.375))
                .child(icon(glyph).size(rems(1.)).text_color(theme::secondary(cx)))
                .child(
                    div()
                        .text_size(rems(0.8125))
                        .font_weight(FontWeight::BOLD)
                        .child(title),
                ),
        )
        .child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child(body),
        );
    if let Some(note) = note {
        card = card.child(
            div()
                .text_size(rems(0.6875))
                .text_color(theme::secondary(cx))
                .child(note),
        );
    }
    for entry in buttons {
        let hook = ask.clone();
        let asked = action.clone();
        card = card.child(
            button(entry.id, entry.label, ButtonVariant::Outline, cx)
                .on_click(cx.listener(move |this, _, _, cx| hook(this, asked.clone(), cx))),
        );
    }
    card.into_any_element()
}

/// The line above a job at work, or one that stopped: its progress bar, the
/// minutes done, and « Arrêter » for an admin.
fn banner<V: Render>(
    panel: &Panel,
    title: &str,
    note: &str,
    is_admin: bool,
    cx: &mut Context<V>,
    ask: impl Fn(&mut V, Ask, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    let percent = panel.percent();
    let running = is_active(&panel.status);
    let mut line = div().flex().items_center().gap(rems(0.5)).child(
        div()
            .text_size(rems(0.75))
            .font_weight(FontWeight::SEMIBOLD)
            .child(SharedString::from(title.to_owned())),
    );
    if running && is_admin {
        line = line.child(
            button("transcript-stop", "Arrêter", ButtonVariant::Secondary, cx).on_click(
                cx.listener({
                    let ask = ask.clone();
                    move |this, _, _, cx| ask(this, Ask::Stop, cx)
                }),
            ),
        );
    }
    let block = div()
        .flex()
        .flex_col()
        .gap(rems(0.25))
        .w_full()
        .child(line)
        .child(
            div()
                .relative()
                .w_full()
                .h(rems(0.1875))
                .bg(theme::border(cx))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .h_full()
                        .w(gpui_kit::relative(percent as f32 / 100.0))
                        .bg(theme::accent(cx)),
                ),
        )
        .child(
            div()
                .text_size(rems(0.6875))
                .text_color(theme::secondary(cx))
                .child(SharedString::from(note.to_owned())),
        );
    block.into_any_element()
}

/// The search field, the "clear" and the two admin actions of a finished
/// transcript.
fn tools<V: Render>(
    panel: &mut Panel,
    field: Option<&Entity<InputState>>,
    is_admin: bool,
    window: &mut Window,
    cx: &mut Context<V>,
    ask: impl Fn(&mut V, Ask, &mut Context<V>) + Clone + 'static,
) -> Div {
    let mut row = div()
        .flex()
        .items_center()
        .gap(rems(0.25))
        .w_full()
        .min_w_0();
    if let Some(field) = field {
        row = row.child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(rems(0.25))
                .px(rems(0.25))
                .border_1()
                .border_color(theme::border(cx))
                .child(
                    icon(IconName::Search)
                        .size(rems(0.75))
                        .text_color(theme::secondary(cx)),
                )
                .child(div().flex_1().min_w_0().child(gpui_omarchy::input(
                    "transcript-search",
                    field,
                    window,
                    cx,
                ))),
        );
    }
    if panel.search.active() {
        row = row.child(
            icon_button(
                "transcript-search-clear",
                IconName::Close,
                "Effacer la recherche",
                ButtonVariant::Secondary,
                cx,
            )
            .on_click({
                let ask = ask.clone();
                cx.listener(move |this, _, _, cx| ask(this, Ask::ClearSearch, cx))
            }),
        );
    }
    if is_admin && is_terminal(&panel.status) {
        let relaunch = ask.clone();
        let delete = ask;
        row = row.child(
            icon_button(
                "transcript-relaunch",
                IconName::RotateCw,
                "Changer la langue et relancer",
                ButtonVariant::Secondary,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                relaunch(this, Ask::Relaunch, cx);
            })),
        );
        row = row.child(
            icon_button(
                "transcript-delete",
                IconName::Trash,
                "Supprimer la transcription",
                ButtonVariant::Secondary,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                delete(this, Ask::Delete, cx);
            })),
        );
    }
    div().w_full().child(row)
}

/// The confirmation a relaunch or a delete asks for: what will happen, and the
/// language to run it with.
fn confirm_block<V: Render>(
    panel: &Panel,
    cx: &mut Context<V>,
    ask: impl Fn(&mut V, Ask, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    let Some(confirm) = panel.confirm else {
        return div().id("transcript-confirm-empty").into_any_element();
    };
    let mut block = div()
        .id("transcript-confirm")
        .flex()
        .flex_col()
        .gap(rems(0.25))
        .p(rems(0.5))
        .border_1()
        .border_color(theme::border(cx))
        .bg(theme::chrome(cx))
        .w_full()
        .child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::text(cx))
                .child(match confirm {
                    Confirm::Relaunch => {
                        "Relancer la transcription ? Le texte actuel sera remplacé."
                    }
                    Confirm::Delete => {
                        "Supprimer la transcription ? Le texte sera perdu ; l'audio reste intact."
                    }
                }),
        );
    if confirm == Confirm::Relaunch {
        let mut chips = div().flex().flex_wrap().items_center().gap(rems(0.25));
        for code in LANGUAGES {
            let selected = panel.relaunch_language == code;
            chips = chips.child(
                div()
                    .id((
                        "relaunch-language",
                        LANGUAGES.iter().position(|c| *c == code).unwrap_or(0),
                    ))
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
                    .child(language_label(code)),
            );
        }
        block = block.child(chips);
    }
    let go = ask.clone();
    let action = match confirm {
        Confirm::Relaunch => Ask::RelaunchConfirm,
        Confirm::Delete => Ask::DeleteConfirm,
    };
    block
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.25))
                .child(
                    button(
                        "transcript-confirm-go",
                        match confirm {
                            Confirm::Relaunch => "Relancer",
                            Confirm::Delete => "Supprimer",
                        },
                        match confirm {
                            Confirm::Relaunch => ButtonVariant::Primary,
                            Confirm::Delete => ButtonVariant::Danger,
                        },
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| go(this, action.clone(), cx))),
                )
                .child(
                    button(
                        "transcript-confirm-cancel",
                        "Annuler",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| ask(this, Ask::Cancel, cx))),
                ),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn segments<V: Render>(
    panel: &Panel,
    entity: Entity<V>,
    active: Option<usize>,
    sizes: Rc<Vec<Size<Pixels>>>,
    scroll: &gpui_kit::base::VirtualListScrollHandle,
    cx: &mut Context<V>,
    ask: impl Fn(&mut V, Ask, &mut Context<V>) + Clone + 'static,
    term: String,
    scroll_now: bool,
) -> AnyElement {
    let lines = panel.lines().to_vec();
    let handle = scroll.clone();
    if scroll_now && let Some(index) = active {
        handle.scroll_to_item(index, ScrollStrategy::Center);
    }
    let list = virtual_list(
        entity,
        ElementId::Name("transcript-segments".into()),
        sizes,
        move |_this, range, _window, cx| {
            let mut built = Vec::with_capacity(range.len());
            for index in range {
                let Some(segment) = lines.get(index) else {
                    continue;
                };
                let is_now = active == Some(index);
                built.push(
                    div()
                        .id(("transcript-line", index))
                        .debug_selector(move || format!("transcript-line-{index}"))
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .h(px(ROW_HEIGHT))
                        .w_full()
                        .pl(rems(0.375))
                        .pr(rems(0.25))
                        .border_l_2()
                        .border_color(if is_now {
                            theme::accent(create_cx(cx))
                        } else {
                            theme::shell(create_cx(cx))
                        })
                        .when(is_now, |row| row.bg(theme::selected(create_cx(cx))))
                        .cursor_pointer()
                        .hover(|style| style.bg(theme::chrome(create_cx(cx))))
                        .on_click({
                            let ask = ask.clone();
                            let start = segment.start;
                            cx.listener(move |this, _, _, cx| ask(this, Ask::Seek(start), cx))
                        })
                        .child(
                            div()
                                .flex_shrink_0()
                                .w(rems(2.75))
                                .text_size(rems(0.625))
                                .font_family(theme::mono(create_cx(cx)))
                                .text_color(theme::secondary(create_cx(cx)))
                                .child(duration_label(segment.start.max(0.0) as u32)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(rems(0.75))
                                .text_color(if is_now {
                                    theme::text(create_cx(cx))
                                } else {
                                    theme::secondary(create_cx(cx))
                                })
                                .children(parts(segment.text.clone(), &term, is_now, cx)),
                        )
                        .into_any_element(),
                );
            }
            built
        },
        cx,
    )
    .h_full()
    .min_h_0()
    .track_scroll(&handle);
    div()
        .id("transcript-lines")
        .flex()
        .flex_col()
        .size_full()
        .child(list)
        .into_any_element()
}

fn parts<V: Render>(
    text: String,
    term: &str,
    current: bool,
    cx: &mut Context<V>,
) -> Vec<AnyElement> {
    if term.trim().is_empty() {
        return vec![text.into_any_element()];
    }
    highlight_parts(&text, term)
        .into_iter()
        .map(|(part, marked)| {
            let element = div().child(part);
            if marked {
                element
                    .text_color(theme::accent(cx))
                    .font_weight(FontWeight::SEMIBOLD)
                    .into_any_element()
            } else if current {
                element.text_color(theme::text(cx)).into_any_element()
            } else {
                element.into_any_element()
            }
        })
        .collect()
}

fn search_block<V: Render>(
    panel: &Panel,
    cx: &mut Context<V>,
    ask: impl Fn(&mut V, Ask, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    let term = panel.search.term.clone();
    let mut column = div()
        .id("transcript-hits")
        .flex()
        .flex_col()
        .w_full()
        .min_w_0()
        .gap(rems(0.25));
    let header = if panel.search.busy {
        "Recherche dans l'épisode…".to_owned()
    } else if panel.search.total > panel.search.hits.len() as u32 {
        format!(
            "{} résultats sur {}",
            panel.search.hits.len(),
            panel.search.total
        )
    } else {
        format!(
            "{} résultat{}",
            panel.search.total,
            if panel.search.total > 1 { "s" } else { "" }
        )
    };
    column = column.child(
        div()
            .text_size(rems(0.6875))
            .text_color(theme::secondary(cx))
            .child(SharedString::from(
                if panel.search.local && !panel.search.busy {
                    format!("{header} · recherche limitée au texte de cet appareil")
                } else {
                    header
                },
            )),
    );
    if !panel.search.busy && panel.search.hits.is_empty() {
        column = column.child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child(SharedString::from(format!(
                    "Aucun passage pour « {term} » dans cet épisode."
                ))),
        );
        return column.into_any_element();
    }
    for (index, hit) in panel
        .search
        .hits
        .iter()
        .take(SEARCH_PAGE as usize)
        .enumerate()
    {
        let start = hit.start;
        column = column.child(
            div()
                .id(("transcript-hit", index))
                .debug_selector(move || format!("transcript-hit-{index}"))
                .flex()
                .items_start()
                .gap(rems(0.5))
                .w_full()
                .min_w_0()
                .px(rems(0.375))
                .py(rems(0.25))
                .border_b_1()
                .border_color(theme::border(cx).opacity(0.4))
                .cursor_pointer()
                .hover(|style| style.bg(theme::chrome(cx)))
                .on_click({
                    let ask = ask.clone();
                    cx.listener(move |this, _, _, cx| ask(this, Ask::PickHit(start), cx))
                })
                .child(
                    div()
                        .flex_shrink_0()
                        .w(rems(2.75))
                        .text_size(rems(0.625))
                        .font_family(theme::mono(cx))
                        .text_color(theme::accent(cx))
                        .child(duration_label(start.max(0.0) as u32)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(rems(0.75))
                        .children(parts(hit.text.clone(), &term, false, cx)),
                ),
        );
    }
    column.into_any_element()
}

fn pager<V: Render>(
    panel: &Panel,
    cx: &mut Context<V>,
    ask: impl Fn(&mut V, Ask, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    let pages = (panel.total.max(1)).div_ceil(panel.limit.max(1)).max(1);
    let page = (panel.offset / panel.limit.max(1)) + 1;
    let previous = ask.clone();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(rems(0.25))
        .w_full()
        .child(
            button(
                "transcript-prev",
                "Page précédente",
                ButtonVariant::Secondary,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| previous(this, Ask::Page(-1), cx))),
        )
        .child(
            div()
                .text_size(rems(0.6875))
                .font_family(theme::mono(cx))
                .text_color(theme::secondary(cx))
                .child(SharedString::from(format!(
                    "Page {} sur {}",
                    page.min(pages),
                    pages
                ))),
        )
        .child(
            button(
                "transcript-next",
                "Page suivante",
                ButtonVariant::Secondary,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| ask(this, Ask::Page(1), cx))),
        )
        .into_any_element()
}

fn create_cx<'a, V>(cx: &'a Context<V>) -> &'a App {
    cx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(start: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start,
            end: start + 3.0,
            text: text.to_owned(),
        }
    }

    fn page(segments: Vec<TranscriptSegment>, total: u32, offset: u32) -> EpisodeTranscript {
        EpisodeTranscript {
            episode_id: nz_model::ids::EpisodeId::new("e1"),
            status: "ready".into(),
            archived: true,
            duration_ms: 3_600_000,
            total,
            offset,
            limit: PAGE,
            progress: 1.0,
            segments,
            ..EpisodeTranscript::default()
        }
    }

    /// The playhead's line is found by timestamp on every tick, and the answer is
    /// the last line that started at or before it.
    #[test]
    fn the_line_under_the_playhead_is_the_last_one_that_started() {
        let lines = vec![
            segment(0.0, "a"),
            segment(3.0, "b"),
            segment(8.0, "c"),
            segment(40.0, "d"),
        ];
        assert_eq!(segment_at(&lines, -1.0), None);
        assert_eq!(segment_at(&lines, 0.0), Some(0));
        assert_eq!(segment_at(&lines, 7.9), Some(1));
        assert_eq!(segment_at(&lines, 8.0), Some(2));
        assert_eq!(segment_at(&lines, 1_000.0), Some(3));
        assert_eq!(segment_at(&[], 3.0), None);
    }

    /// A panel that has a page is not asked for it again; one that lost the
    /// playhead is.
    #[test]
    fn a_page_is_read_once_and_re_read_when_the_playhead_leaves_it() {
        let mut panel = Panel::new("e1", Some(true), 3_600);
        let first = panel.open(Some(95.4)).expect("the first page");
        match &first {
            Fetch::Page { path, at, .. } => {
                assert_eq!(path, "/podcast/episode/e1/transcript?limit=200&at=95.4");
                assert_eq!(*at, Some(95.4));
            }
            other => panic!("unexpected: {other:?}"),
        }
        panel.apply(
            page(vec![segment(90.0, "a"), segment(96.0, "b")], 2, 0),
            Tone::User,
        );
        assert!(panel.open(Some(95.4)).is_none(), "the page covers it");
        assert!(
            panel.open(Some(3_000.0)).is_some(),
            "the playhead left the page"
        );
    }

    /// A poll is one request, and only while a job is at work: the panel must not
    /// keep asking a finished one.
    #[test]
    fn a_poll_only_happens_while_a_job_is_running() {
        let mut panel = Panel::new("e1", Some(true), 10);
        assert!(panel.poll().is_none(), "nothing is running");
        let mut running = page(Vec::new(), 0, 0);
        running.status = "running".into();
        running.progress = 0.4;
        panel.apply(running, Tone::Poll);
        assert_eq!(panel.status, "running");
        assert_eq!(panel.percent(), 40);
        match panel.poll().expect("a poll") {
            Fetch::Page {
                path, lane_hint, ..
            } => {
                assert_eq!(path, "/podcast/episode/e1/transcript?limit=1&offset=0");
                assert!(lane_hint, "a poll is background work");
            }
            other => panic!("unexpected: {other:?}"),
        }
        let mut ready = page(Vec::new(), 0, 0);
        ready.status = "ready".into();
        panel.apply(ready, Tone::Poll);
        assert!(panel.poll().is_none(), "the job is over");
    }

    /// Following stops at a search or a scroll and comes back on demand: the pane
    /// never moves under the person's eyes.
    #[test]
    fn following_the_playhead_can_be_left_and_resumed() {
        let mut panel = Panel::new("e1", Some(true), 100);
        panel.apply(
            page(vec![segment(0.0, "a"), segment(10.0, "b")], 2, 0),
            Tone::User,
        );
        assert!(panel.follow_to(11.0));
        assert!(!panel.follow_to(11.5), "the same line is not a move");
        assert_eq!(panel.active, Some(1));
        panel.pause_follow();
        assert!(!panel.follow);
        panel.resume_follow();
        assert!(panel.follow);
    }

    /// The whole term wins over its words, and the marks are case- and
    /// accent-insensitive — a person types "meme" and finds "même".
    #[test]
    fn a_search_highlights_what_folded_to_the_term() {
        let parts = highlight_parts("Le même meme, encore", "meme");
        let marked: Vec<&str> = parts
            .iter()
            .filter(|(_, marked)| *marked)
            .map(|(text, _)| text.as_str())
            .collect();
        assert_eq!(marked, vec!["même", "meme"]);
        assert_eq!(fold_text("Étienne Daho"), "etienne daho");
        let whole = highlight_parts("un chat noir", "chat noir");
        assert_eq!(
            whole.iter().filter(|(_, marked)| *marked).count(),
            1,
            "the whole term is one mark, not two"
        );
        assert_eq!(
            highlight_parts("rien", "xyz"),
            vec![("rien".to_owned(), false)]
        );
    }

    /// An offline search reads the lines on the device and says so.
    #[test]
    fn a_local_search_reads_the_page_in_memory() {
        let lines = vec![segment(0.0, "Le même sujet"), segment(30.0, "Autre chose")];
        let mut search = Search {
            term: "meme".into(),
            ..Search::default()
        };
        assert_eq!(search.locally(&lines, "meme", true), Some(()));
        assert_eq!(search.hits.len(), 1);
        assert!(search.local, "the panel says the search was local");
        assert_eq!(
            search.locally(&lines, "meme", false),
            None,
            "online with a server transcript, the server answers"
        );
    }

    /// T4's seam: with a source installed and the server answering `none`, the
    /// panel shows the machine's transcript and searches it in memory.
    #[test]
    fn a_local_transcript_fills_the_panel_when_the_server_has_none() {
        set_local_source(Some(Arc::new(|episode: &str| {
            (episode == "e-local").then(|| LocalTranscript {
                engine: Some("whisper.cpp".into()),
                model: Some("ggml-base".into()),
                language: Some("fr".into()),
                segments: vec![segment(0.0, "Bonjour"), segment(5.0, "au revoir")],
                created: None,
            })
        })));
        let mut panel = Panel::new("e-local", Some(true), 60);
        let none = EpisodeTranscript {
            status: "none".into(),
            ..EpisodeTranscript::default()
        };
        panel.apply(none, Tone::User);
        assert!(panel.is_local());
        assert_eq!(panel.lines().len(), 2);
        assert_eq!(panel.status, "local");
        assert!(panel.search("au revoir").is_none(), "answered in memory");
        assert_eq!(panel.search.hits.len(), 1);
        set_local_source(None);
        assert!(local_for("e-local").is_none());
        // A transcript read on the device is the whole episode, in seconds.
        let made = LocalTranscript::from_milliseconds(
            vec![
                (0, 1_500, "un".to_owned()),
                (1_500, 3_000, "deux".to_owned()),
            ],
            Some("whisper.cpp".into()),
            None,
            Some("fr".into()),
        );
        assert_eq!(made.segments[1].start, 1.5);
    }

    /// A failed read keeps what is on screen and names the cause; a dead link is
    /// `offline`, which is a sentence of its own.
    #[test]
    fn a_failed_read_names_its_cause_and_keeps_the_page() {
        let mut panel = Panel::new("e1", Some(true), 100);
        panel.apply(page(vec![segment(0.0, "a")], 1, 0), Tone::User);
        panel.fail(ApiError::Offline);
        assert!(panel.offline);
        assert!(panel.failed.is_none(), "a page on screen is not a failure");
        assert_eq!(panel.segments.len(), 1);

        let mut empty = Panel::new("e2", Some(true), 100);
        empty.fail(ApiError::Http {
            status: 500,
            message: "boom".into(),
            code: None,
        });
        assert!(empty.failed.is_some());
    }

    #[test]
    fn a_query_carries_a_trimmed_float() {
        assert_eq!(trim_float(95.4), "95.4");
        assert_eq!(trim_float(95.0), "95");
        assert_eq!(trim_float(0.0), "0");
        assert_eq!(trim_float(12.345), "12.35");
    }
}
