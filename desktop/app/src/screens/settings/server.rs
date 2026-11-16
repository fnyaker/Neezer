//! Réglages → Serveur: the archive, its rules, its cleanup, the disk, the
//! library backfills and the server's own transcription.
//!
//! Lot D14b. Everything here is `@admin_required` on the server; the screen is
//! reached through Réglages' admin sections and the server refuses anyway.
//!
//! Rule 2 is the shape of this page: every action that can take more than a
//! second is a JOB — it starts at once, the card polls its own `*_status` route
//! while it runs, and when it ends the card says what it did or the CAUSE of
//! each failure (`webapp/src/components/JobFailures.svelte`'s contract), never
//! a bare count. Nothing here blocks the GPUI thread: the requests run on the
//! app's `nz-net` runtime and the answers come back through an [`Inbox`].
//!
//! Three shapes are read as raw JSON on purpose, and the reason is the server:
//! `/enrich/backfill` answers the shared `libjob` snapshot plus its coverage,
//! but `nz_model::EnrichStatus` (D1) typed only the coverage half; and
//! `/transcribe/workers` never reached the SPA's `api.js`, so D2's typed
//! surface has no function for it. The local types below are the three fields
//! those routes send that no committed type carries — the note records it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Div, Entity, FocusHandle, Focusable as _,
    FontWeight, InteractiveElement as _, IntoElement, ParentElement, Render, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled, Subscription, Window, div, px, relative,
    rems,
};
use gpui_omarchy::{
    ButtonVariant, ChoiceItem, ChoiceState, IconName, button, focus_scope, icon, input, select,
    switch,
};
use nz_api::{Api, ApiError, Lane, Method};
use nz_model::{
    ArchiveRuleSet, ArchiveRules, ArchiveStatus, CleanupPreview, CreditsBackfill, JobStatus,
    MigrateStatus, ServerSettings, Storage, TranscribeSettings,
};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::nav::SettingsSection;
use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;

/// The archive's own poll cadence: the SPA's 1.5 s (`Settings.svelte#pollJob`).
const ARCHIVE_POLL: Duration = Duration::from_millis(1_500);
/// The disk migration's, the SPA's 2 s (`Settings.svelte#pollMigrate`).
const MIGRATE_POLL: Duration = Duration::from_millis(2_000);
/// The library backfills', the SPA's 1.5 s (`EnrichBackfill`, `CreditsBackfill`).
const JOB_POLL: Duration = Duration::from_millis(1_500);
/// How many failure rows a card prints before it counts the rest.
const FAILURES_SHOWN: usize = 8;
/// The SPA's cleanup preview prints twelve rows (`Settings.svelte`).
const CLEANUP_ROWS: usize = 12;

// ---------------------------------------------------------------------------
// The screen's inbox: worker threads answer here, the GPUI thread applies it.
// ---------------------------------------------------------------------------

/// A one-way channel from `nz-net` to a screen's `Render` state.
///
/// The shell owns the one channel for [`crate::state::AppEvent`]; a screen that
/// runs its own requests needs the same discipline without adding a variant per
/// screen to the app's own enum (which D7 owns and later lots must not edit).
/// The receiver is drained on GPUI's foreground executor, so no worker ever
/// touches view state, and the channel closes when the screen is dropped.
pub(super) struct Inbox<M> {
    tx: UnboundedSender<M>,
    rx: UnboundedReceiver<M>,
}

impl<M: 'static> Inbox<M> {
    pub(super) fn new() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self { tx, rx }
    }

    pub(super) fn sender(&self) -> UnboundedSender<M> {
        self.tx.clone()
    }

    /// Drain the channel for the view's life, handing every message to `apply`.
    pub(super) fn spawn<V: 'static>(
        self,
        cx: &mut Context<V>,
        mut apply: impl 'static + FnMut(&mut V, M, &mut Context<V>),
    ) {
        let mut rx = self.rx;
        cx.spawn(async move |this, cx| {
            while let Some(message) = rx.recv().await {
                if this
                    .update(cx, |view, cx| apply(view, message, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }
}

// ---------------------------------------------------------------------------
// The payloads no committed type carries.
// ---------------------------------------------------------------------------

/// `/api/enrich/backfill`: the shared `libjob` snapshot (flattened, like
/// `nz_model::CreditsBackfill`) plus the MusicBrainz coverage.
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct EnrichLive {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub acoustid: bool,
    #[serde(default)]
    pub started: Option<String>,
    #[serde(default)]
    pub coverage: nz_model::EnrichCoverage,
    #[serde(flatten)]
    pub job: JobStatus,
}

/// One row of `GET /api/transcribe/workers` (`transcribe_worker.py#_worker_row`).
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct WorkerRow {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// `"server"` or `"client"`.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub last_seen: Option<String>,
    #[serde(default)]
    pub revoked: Option<String>,
    #[serde(default)]
    pub lease: Option<WorkerLease>,
}

/// The lease a worker currently holds, if any.
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct WorkerLease {
    #[serde(default)]
    pub episode_id: String,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub heartbeat_at: Option<String>,
    #[serde(default)]
    pub attempts: u32,
}

/// `GET /api/transcribe/workers`.
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct WorkerList {
    #[serde(default)]
    pub workers: Vec<WorkerRow>,
}

/// `POST /api/transcribe/workers`: the row, and the token shown ONCE.
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct IssuedWorker {
    #[serde(default)]
    pub worker: WorkerRow,
    #[serde(default)]
    pub token: String,
}

/// One row of a job's failure sample (`neezer/libjob.py#_record_failure`).
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct Failure {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub track: String,
    #[serde(default)]
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Small pure helpers, ported from the SPA (`lib/format.js`, `lib/transcript.js`).
// ---------------------------------------------------------------------------

/// `lib/format.js#bytes`: 1024-based, one decimal below ten, French units.
pub(super) fn bytes(value: u64) -> String {
    let units = ["o", "Ko", "Mo", "Go", "To"];
    let mut number = value as f64;
    let mut unit = 0;
    while number >= 1024.0 && unit < units.len() - 1 {
        number /= 1024.0;
        unit += 1;
    }
    if number < 10.0 && unit > 0 {
        format!("{number:.1} {}", units[unit])
    } else {
        format!("{} {}", number.round() as u64, units[unit])
    }
}

/// A percentage, clamped, as the gauge wants it.
pub(super) fn fraction(done: u32, total: u32) -> Option<f32> {
    if total == 0 {
        return None;
    }
    Some((done as f32 / total as f32).clamp(0.0, 1.0))
}

/// `lib/transcript.js#languageLabel`, with the server's own list of codes.
pub(super) fn language_label(code: &str) -> String {
    match code {
        "auto" => "Détection automatique".to_owned(),
        "fr" => "Français".to_owned(),
        "en" => "Anglais".to_owned(),
        "es" => "Espagnol".to_owned(),
        "de" => "Allemand".to_owned(),
        "it" => "Italien".to_owned(),
        "pt" => "Portugais".to_owned(),
        other => other.to_uppercase(),
    }
}

/// `lib/transcript.js#reasonText`: why the server's engine is unavailable.
pub(super) fn transcribe_reason(reason: &str) -> String {
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

/// A month name, for the absolute dates these screens show.
const MONTHS: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
    "déc.",
];

/// The server's own wall-clock timestamp, formatted in French.
///
/// The workspace has no date crate, and the payloads carry NAIVE server-local
/// timestamps (`datetime.isoformat()`, no offset). Showing them as they were
/// written is exact; a relative "il y a 3 h" would need a timezone the client
/// does not have, so these screens print absolutes instead.
pub(super) fn date_fr(iso: &str) -> String {
    // `get`, never `[a..b]`: a payload is hostile input and a byte slice inside
    // a multi-byte character would panic the UI thread.
    let (Some(year), Some(month), Some(day)) = (iso.get(0..4), iso.get(5..7), iso.get(8..10))
    else {
        return iso.to_owned();
    };
    let Ok(month) = month.parse::<usize>() else {
        return iso.to_owned();
    };
    let Ok(day) = day.parse::<u32>() else {
        return iso.to_owned();
    };
    if month == 0 || month > 12 || day == 0 {
        return iso.to_owned();
    }
    format!("{day} {} {year}", MONTHS[month - 1])
}

/// The same, with the time when the payload carried one.
pub(super) fn datetime_fr(iso: &str) -> String {
    let date = date_fr(iso);
    match iso.get(11..16) {
        // A time is `HH:MM` and nothing else: a payload with letters where the
        // clock belongs keeps the date only instead of printing garbage.
        Some(time)
            if iso.as_bytes().get(10) == Some(&b'T')
                && time.as_bytes().get(2) == Some(&b':')
                && time
                    .bytes()
                    .enumerate()
                    .all(|(index, byte)| index == 2 || byte.is_ascii_digit()) =>
        {
            format!("{date} à {time}")
        }
        _ => date,
    }
}

/// `true` for a canonical lowercase-or-uppercase UUID.
///
/// The worker routes take the id as a path segment: an id read from a payload is
/// still validated before it becomes a URL (never trust it to be one).
pub(super) fn is_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        match index {
            8 | 13 | 18 | 23 => {
                if *byte != b'-' {
                    return false;
                }
            }
            _ => {
                if !byte.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }
    true
}

/// The card a failure sample prints: the tally of causes, then the rows.
pub(super) fn failure_report(label: &str, job: &JobStatus) -> String {
    let mut out = format!("Neezer — {} : {} en échec\n", label, job.failed);
    let mut reasons: Vec<(&String, &u32)> = job.failure_reasons.iter().collect();
    reasons.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    for (reason, count) in &reasons {
        out.push_str(&format!("  {count}x {reason}\n"));
    }
    out.push('\n');
    for row in &job.failures {
        let failure: Failure = serde_json::from_value(row.clone()).unwrap_or_default();
        let name = if failure.track.is_empty() {
            failure.id.as_str()
        } else {
            failure.track.as_str()
        };
        out.push_str(&format!("  {name} — {}\n", failure.reason));
    }
    out
}

// ---------------------------------------------------------------------------
// The screen.
// ---------------------------------------------------------------------------

/// Which lazy number field is asked for. One per editable number, so the field
/// and the setting it commits cannot drift.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Field {
    ArtistLimit,
    CleanFreeGb,
    CleanStaleDays,
    MaxMinutes,
    DailyMinutes,
    SearchSegments,
    Quota,
    WorkerName,
}

/// Which lazy picker is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Picker {
    ArtistScope,
    CleanOrder,
    Language,
    WorkerKind,
}

/// What a background request reports back.
enum Msg {
    Archive(Result<ArchiveStatus, ApiError>),
    Rules(Result<ArchiveRules, ApiError>),
    RuleSaved {
        key: &'static str,
        previous: Value,
        result: Result<Value, ApiError>,
    },
    Migrate(Result<MigrateStatus, ApiError>),
    Enrich(Result<EnrichLive, ApiError>),
    Credits(Result<CreditsBackfill, ApiError>),
    Cleanup(Result<CleanupPreview, ApiError>),
    Cleaned(Result<Value, ApiError>),
    Storage(Result<Storage, ApiError>),
    Flushed(Result<(), ApiError>),
    Settings(Result<ServerSettings, ApiError>),
    QuotaSaved(Result<Value, ApiError>),
    Transcribe(Result<TranscribeSettings, ApiError>),
    TranscribeSaved {
        key: &'static str,
        previous: Value,
        result: Result<Value, ApiError>,
    },
    Workers(Result<Vec<WorkerRow>, ApiError>),
    WorkerIssued(Result<IssuedWorker, ApiError>),
    WorkerRevoked {
        id: String,
        result: Result<Value, ApiError>,
    },
    /// A job was started; `Err` carries the refusal, `Ok` says what to say.
    Started {
        what: &'static str,
        result: Result<Value, ApiError>,
    },
    /// The page's ten reads finished (their results arrived on their own).
    Loaded,
    /// A job's poll is due.
    PollDue(Poll),
}

/// Réglages → Serveur.
pub struct Screen {
    section: SettingsSection,
    state: Entity<AppState>,
    tx: UnboundedSender<Msg>,
    root_focus: FocusHandle,
    scroll: gpui_kit::ScrollHandle,
    focused: bool,
    subscriptions: Vec<Subscription>,
    fields: HashMap<Field, Entity<InputState>>,
    pickers: HashMap<Picker, Entity<ChoiceState>>,
    picker_values: HashMap<Picker, Vec<String>>,
    last_picked: HashMap<Picker, String>,

    loading: bool,
    page_error: Option<SharedString>,

    archive: Option<ArchiveStatus>,
    archive_busy: bool,
    archive_poll: bool,

    rules: Option<ArchiveRules>,
    saving_rule: Option<&'static str>,

    migrate: Option<MigrateStatus>,
    migrate_busy: bool,
    migrate_poll: bool,

    enrich: Option<EnrichLive>,
    enrich_busy: bool,
    enrich_poll: bool,
    enrich_force: bool,

    credits: Option<CreditsBackfill>,
    credits_busy: bool,
    credits_poll: bool,
    credits_force: bool,

    cleanup: Option<CleanupPreview>,
    cleaning: bool,

    storage: Option<Storage>,
    flushing: bool,

    settings: Option<ServerSettings>,
    quota_busy: bool,

    transcribe: Option<TranscribeSettings>,
    transcribe_busy: Option<&'static str>,
    probing: bool,

    workers: Option<Vec<WorkerRow>>,
    workers_error: Option<SharedString>,
    revoking: Option<String>,
    confirming_revoke: Option<String>,
    issued: Option<IssuedWorker>,
    issued_copied: bool,
    worker_busy: bool,

    open_failures: Vec<&'static str>,
}

impl Screen {
    /// The route table's constructor (`screens::settings::Screen::new`).
    pub fn new(section: SettingsSection, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let inbox = Inbox::new();
        let tx = inbox.sender();
        let root_focus = cx.focus_handle();
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());

        let mut this = Self {
            section,
            state,
            tx,
            root_focus,
            scroll: gpui_kit::ScrollHandle::new(),
            focused: false,
            subscriptions: vec![subscription],
            fields: HashMap::new(),
            pickers: HashMap::new(),
            picker_values: HashMap::new(),
            last_picked: HashMap::new(),
            loading: false,
            page_error: None,
            archive: None,
            archive_busy: false,
            archive_poll: false,
            rules: None,
            saving_rule: None,
            migrate: None,
            migrate_busy: false,
            migrate_poll: false,
            enrich: None,
            enrich_busy: false,
            enrich_poll: false,
            enrich_force: false,
            credits: None,
            credits_busy: false,
            credits_poll: false,
            credits_force: false,
            cleanup: None,
            cleaning: false,
            storage: None,
            flushing: false,
            settings: None,
            quota_busy: false,
            transcribe: None,
            transcribe_busy: None,
            probing: false,
            workers: None,
            workers_error: None,
            revoking: None,
            confirming_revoke: None,
            issued: None,
            issued_copied: false,
            worker_busy: false,
            open_failures: Vec::new(),
        };
        inbox.spawn::<Self>(cx, |this, message, cx| this.on_message(message, cx));
        this.load(cx);
        this
    }

    /// The section this view draws (`Serveur`); the tests read it.
    pub fn section(&self) -> SettingsSection {
        self.section
    }

    fn api(&self, cx: &App) -> Option<Arc<Api>> {
        self.state.read(cx).api.clone()
    }

    /// The page's requests, in one sequential task on `nz-net`.
    ///
    /// Ten reads, the same ones the SPA's Serveur tab makes, and they run one
    /// after the other on purpose: each answer paints its card as it lands, and
    /// a slow link sees one request at a time instead of ten competing for it
    /// (rule 3). A failure is per-card, so one dead route never hides the rest.
    fn load(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let Some(api) = self.api(cx) else {
            self.page_error = Some(
                "La session n'est pas ouverte : reconnectez-vous pour charger le serveur.".into(),
            );
            cx.notify();
            return;
        };
        self.loading = true;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            macro_rules! send {
                ($call:expr, $wrap:path) => {
                    let _ = tx.send($wrap($call.await));
                };
            }
            send!(api.archive_status(), Msg::Archive);
            send!(api.archive_rules(), Msg::Rules);
            send!(api.archive_migrate_status(), Msg::Migrate);
            let enriched = api
                .get::<EnrichLive>(Lane::User, "/enrich/backfill")
                .await
                .map(|fresh| fresh.value);
            let _ = tx.send(Msg::Enrich(enriched));
            send!(api.credits_backfill_status(), Msg::Credits);
            send!(api.cleanup_preview(), Msg::Cleanup);
            send!(api.storage(), Msg::Storage);
            send!(api.get_settings(), Msg::Settings);
            send!(api.transcribe_settings(false), Msg::Transcribe);
            let workers = api
                .get::<WorkerList>(Lane::User, "/transcribe/workers")
                .await
                .map(|fresh| fresh.value.workers);
            let _ = tx.send(Msg::Workers(workers));
            let _ = tx.send(Msg::Loaded);
        });
        cx.notify();
    }

    fn fetch_archive(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else { return };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let _ = tx.send(Msg::Archive(api.archive_status().await));
        });
    }

    fn fetch_cleanup(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else { return };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let _ = tx.send(Msg::Cleanup(api.cleanup_preview().await));
        });
    }

    fn fetch_storage(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else { return };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let _ = tx.send(Msg::Storage(api.storage().await));
        });
    }

    fn fetch_workers(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else { return };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let result = api
                .get::<WorkerList>(Lane::User, "/transcribe/workers")
                .await
                .map(|fresh| fresh.value.workers);
            let _ = tx.send(Msg::Workers(result));
        });
    }

    /// One delayed poll, per job, never two at once.
    ///
    /// The timer lives on the app's `nz-net` runtime, not on GPUI's background
    /// executor: one clock for every background step of this screen, and the
    /// tests drive it the same way the D7 journeys drive the network.
    fn schedule(&mut self, which: Poll, cx: &mut Context<Self>) {
        let slot = match which {
            Poll::Archive => &mut self.archive_poll,
            Poll::Migrate => &mut self.migrate_poll,
            Poll::Enrich => &mut self.enrich_poll,
            Poll::Credits => &mut self.credits_poll,
        };
        if *slot {
            return;
        }
        *slot = true;
        let delay = match which {
            Poll::Archive => ARCHIVE_POLL,
            Poll::Migrate => MIGRATE_POLL,
            Poll::Enrich | Poll::Credits => JOB_POLL,
        };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = tx.send(Msg::PollDue(which));
        });
    }

    fn on_message(&mut self, message: Msg, cx: &mut Context<Self>) {
        match message {
            Msg::Archive(result) => match result {
                Ok(status) => {
                    let was_running = self.archive.as_ref().is_some_and(|job| job.running);
                    let running = status.running;
                    let done = status.done;
                    let total = status.total;
                    let archived = status.archived;
                    let unavailable = status.unavailable;
                    let failed = status.failed;
                    self.archive = Some(status);
                    if running {
                        self.schedule(Poll::Archive, cx);
                    } else {
                        self.archive_busy = false;
                        if was_running {
                            self.toasts(
                                ToastKind::Info,
                                archive_done_sentence(archived, unavailable, failed),
                                cx,
                            );
                            self.fetch_storage(cx);
                            self.fetch_cleanup(cx);
                        } else if total > 0 && archived == 0 && done > 0 {
                            // A pass that ran fully elsewhere (a restart): say it.
                            self.toasts(
                                ToastKind::Info,
                                format!("Archivage terminé : {done} / {total} vus."),
                                cx,
                            );
                        }
                    }
                }
                Err(error) => {
                    self.archive_busy = false;
                    self.fail("Impossible de lire l'archivage", &error, cx);
                }
            },
            Msg::Rules(result) => match result {
                Ok(rules) => {
                    self.sync_pickers(&rules, None, cx);
                    self.rules = Some(rules);
                }
                Err(error) => self.fail("Impossible de lire les règles d'archivage", &error, cx),
            },
            Msg::RuleSaved {
                key,
                previous,
                result,
            } => {
                self.saving_rule = None;
                match result {
                    Ok(value) => {
                        if let Some(rules) = value.get("rules").and_then(|rules| {
                            serde_json::from_value::<ArchiveRuleSet>(rules.clone()).ok()
                        }) && let Some(current) = self.rules.as_mut()
                        {
                            current.rules = rules;
                        }
                    }
                    Err(error) => {
                        // Put the switch back where the server says it is: an
                        // optimistic control that stays moved is a lie.
                        if let Some(current) = self.rules.as_mut() {
                            restore_rule(&mut current.rules, key, previous);
                        }
                        self.sync_pickers_from_rules(cx);
                        self.fail("Réglage non enregistré", &error, cx);
                    }
                }
            }
            Msg::Migrate(result) => match result {
                Ok(status) => {
                    let was_running = self.migrate.as_ref().is_some_and(|job| job.running);
                    let running = status.running;
                    let done = status.done;
                    let failed = status.failed;
                    self.migrate = Some(status);
                    if running {
                        self.schedule(Poll::Migrate, cx);
                    } else {
                        self.migrate_busy = false;
                        if was_running {
                            self.toasts(
                                ToastKind::Info,
                                if failed > 0 {
                                    format!("Mise à niveau terminée : {done} fichier(s), {failed} échec(s).")
                                } else {
                                    format!("Mise à niveau terminée : {done} fichier(s) vérifié(s).")
                                }, cx);
                            self.fetch_storage(cx);
                        }
                    }
                }
                Err(error) => {
                    self.migrate_busy = false;
                    self.fail("Impossible de lire la mise à niveau", &error, cx);
                }
            },
            Msg::Enrich(result) => match result {
                Ok(job) => {
                    let was_running = self.enrich.as_ref().is_some_and(|live| live.running);
                    let running = job.running;
                    let failed = job.job.failed;
                    self.enrich = Some(job);
                    if running {
                        self.schedule(Poll::Enrich, cx);
                    } else {
                        self.enrich_busy = false;
                        if was_running {
                            self.toasts(
                                ToastKind::Info,
                                if failed > 0 {
                                    format!("Faits complétés : {failed} titre(s) en échec.")
                                } else {
                                    "Faits à jour.".to_owned()
                                },
                                cx,
                            );
                        }
                    }
                }
                Err(error) => {
                    self.enrich_busy = false;
                    self.fail("Impossible de lire les faits MusicBrainz", &error, cx);
                }
            },
            Msg::Credits(result) => match result {
                Ok(job) => {
                    let was_running = self.credits.as_ref().is_some_and(|live| live.job.running);
                    let running = job.job.running;
                    let failed = job.job.failed;
                    self.credits = Some(job);
                    if running {
                        self.schedule(Poll::Credits, cx);
                    } else {
                        self.credits_busy = false;
                        if was_running {
                            self.toasts(
                                ToastKind::Info,
                                if failed > 0 {
                                    format!("Crédits complétés : {failed} titre(s) en échec.")
                                } else {
                                    "Crédits à jour.".to_owned()
                                },
                                cx,
                            );
                        }
                    }
                }
                Err(error) => {
                    self.credits_busy = false;
                    self.fail("Impossible de lire les crédits", &error, cx);
                }
            },
            Msg::Cleanup(result) => match result {
                Ok(preview) => self.cleanup = Some(preview),
                Err(error) => self.fail("Impossible de prévisualiser le nettoyage", &error, cx),
            },
            Msg::Cleaned(result) => {
                self.cleaning = false;
                match result {
                    Ok(value) => {
                        let deleted = value.get("deleted").and_then(Value::as_u64).unwrap_or(0);
                        let freed = value.get("freed").and_then(Value::as_u64).unwrap_or(0);
                        let skipped = value
                            .get("skipped")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        if deleted > 0 {
                            self.toasts(
                                ToastKind::Info,
                                format!(
                                    "{deleted} fichier(s) supprimé(s), {} libérés.",
                                    bytes(freed)
                                ),
                                cx,
                            );
                        } else if skipped {
                            self.toasts(
                                ToastKind::Info,
                                "Rien à libérer : le disque est au-dessus du seuil.".to_owned(),
                                cx,
                            );
                        } else {
                            self.toasts(ToastKind::Info, "Rien n'a été supprimé.".to_owned(), cx);
                        }
                        self.fetch_cleanup(cx);
                        self.fetch_storage(cx);
                    }
                    Err(error) => self.fail("Nettoyage impossible", &error, cx),
                }
            }
            Msg::Storage(result) => match result {
                Ok(storage) => self.storage = Some(storage),
                Err(error) => self.fail("Impossible de lire le stockage", &error, cx),
            },
            Msg::Flushed(result) => {
                self.flushing = false;
                match result {
                    Ok(()) => {
                        self.toasts(ToastKind::Info, "Cache du serveur vidé.".to_owned(), cx);
                        self.fetch_storage(cx);
                    }
                    Err(error) => self.fail("Impossible de vider le cache", &error, cx),
                }
            }
            Msg::Settings(result) => match result {
                Ok(settings) => self.settings = Some(settings),
                Err(error) => self.fail("Impossible de lire les réglages du serveur", &error, cx),
            },
            Msg::QuotaSaved(result) => {
                self.quota_busy = false;
                match result {
                    Ok(value) => {
                        let quota = value
                            .get("upload_quota_gb")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0);
                        if let Some(settings) = self.settings.as_mut() {
                            settings.upload_quota_gb = quota;
                        }
                        self.toasts(
                            ToastKind::Info,
                            if quota == 0.0 {
                                "Quota d'upload désactivé (illimité).".to_owned()
                            } else {
                                format!("Quota d'upload réglé à {quota} Go par utilisateur.")
                            },
                            cx,
                        );
                    }
                    Err(error) => self.fail("Échec de l'enregistrement du quota", &error, cx),
                }
            }
            Msg::Transcribe(result) => {
                self.probing = false;
                match result {
                    Ok(settings) => {
                        self.sync_pickers_from_transcribe(&settings, cx);
                        self.transcribe = Some(settings);
                    }
                    Err(error) => self.fail(
                        "Impossible de lire les réglages de transcription",
                        &error,
                        cx,
                    ),
                }
            }
            Msg::TranscribeSaved {
                key,
                previous,
                result,
            } => {
                self.transcribe_busy = None;
                match result {
                    Ok(value) => match serde_json::from_value::<TranscribeSettings>(value) {
                        Ok(settings) => {
                            self.sync_pickers_from_transcribe(&settings, cx);
                            self.transcribe = Some(settings);
                        }
                        Err(error) => {
                            self.fail(
                                "Réponse de transcription illisible",
                                &ApiError::Decode(error.to_string()),
                                cx,
                            );
                        }
                    },
                    Err(error) => {
                        if let Some(settings) = self.transcribe.as_mut() {
                            restore_transcribe(settings, key, previous);
                        }
                        let message = match (error.code(), key) {
                            (Some(code), "enabled")
                                if code == "unavailable" || code == "disabled" =>
                            {
                                let reason = transcribe_reason(
                                    self.transcribe
                                        .as_ref()
                                        .and_then(|settings| settings.reason.as_deref())
                                        .unwrap_or("disabled"),
                                );
                                format!("Impossible d'activer : {reason}")
                            }
                            _ => format!("Réglage non enregistré : {}", error.cause_fr()),
                        };
                        self.toasts(ToastKind::Error, message, cx);
                    }
                }
            }
            Msg::Workers(result) => match result {
                Ok(workers) => {
                    self.workers = Some(workers);
                    self.workers_error = None;
                }
                Err(ApiError::Http { status: 404, .. }) => {
                    self.workers_error =
                        Some("Ce serveur n'a pas encore les appareils de transcription.".into());
                }
                Err(error) => {
                    self.workers_error = Some(error.cause_fr().into());
                }
            },
            Msg::WorkerIssued(result) => {
                self.worker_busy = false;
                match result {
                    Ok(issued) => {
                        self.issued_copied = false;
                        self.issued = Some(issued);
                        self.fetch_workers(cx);
                    }
                    Err(error) => self.fail("Impossible de créer l'appareil", &error, cx),
                }
            }
            Msg::WorkerRevoked { id, result } => {
                self.revoking = None;
                self.confirming_revoke = None;
                match result {
                    Ok(value) => {
                        let revoked = value
                            .get("revoked")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let released = value.get("released").and_then(Value::as_u64).unwrap_or(0);
                        self.toasts(
                            ToastKind::Info,
                            if revoked {
                                if released > 0 {
                                    format!("Appareil révoqué ; {released} travail(s) rendu(s) à la file.")
                                } else {
                                    "Appareil révoqué.".to_owned()
                                }
                            } else {
                                "Cet appareil était déjà révoqué.".to_owned()
                            }, cx);
                        let _ = id;
                        self.fetch_workers(cx);
                    }
                    Err(error) => self.fail("Révocation impossible", &error, cx),
                }
            }
            Msg::Started { what, result } => match result {
                Ok(_) => {}
                Err(error) => {
                    match what {
                        "archive" => self.archive_busy = false,
                        "migrate" => self.migrate_busy = false,
                        "enrich" => self.enrich_busy = false,
                        "credits" => self.credits_busy = false,
                        _ => {}
                    }
                    self.fail("Impossible de lancer la tâche", &error, cx);
                }
            },
            Msg::Loaded => {
                self.loading = false;
            }
            Msg::PollDue(which) => match which {
                Poll::Archive => {
                    self.archive_poll = false;
                    self.fetch_archive(cx);
                }
                Poll::Migrate => {
                    self.migrate_poll = false;
                    if let Some(api) = self.api(cx) {
                        let runtime = self.state.read(cx).runtime.clone();
                        let tx = self.tx.clone();
                        runtime.spawn(async move {
                            let _ = tx.send(Msg::Migrate(api.archive_migrate_status().await));
                        });
                    }
                }
                Poll::Enrich => {
                    self.enrich_poll = false;
                    if let Some(api) = self.api(cx) {
                        let runtime = self.state.read(cx).runtime.clone();
                        let tx = self.tx.clone();
                        runtime.spawn(async move {
                            let result = api
                                .get::<EnrichLive>(Lane::User, "/enrich/backfill")
                                .await
                                .map(|fresh| fresh.value);
                            let _ = tx.send(Msg::Enrich(result));
                        });
                    }
                }
                Poll::Credits => {
                    self.credits_poll = false;
                    if let Some(api) = self.api(cx) {
                        let runtime = self.state.read(cx).runtime.clone();
                        let tx = self.tx.clone();
                        runtime.spawn(async move {
                            let _ = tx.send(Msg::Credits(api.credits_backfill_status().await));
                        });
                    }
                }
            },
        }
        cx.notify();
    }

    // -- actions ---------------------------------------------------------

    fn start_archive(&mut self, scope: &'static str, cx: &mut Context<Self>) {
        if self.archive_busy {
            return;
        }
        let Some(api) = self.api(cx) else {
            self.toasts(
                ToastKind::Error,
                "Le serveur n'est pas joignable.".to_owned(),
                cx,
            );
            return;
        };
        self.archive_busy = true;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        let fetch = self.tx.clone();
        runtime.spawn(async move {
            let result = api.archive_backfill(scope).await;
            let ok = result.is_ok();
            let _ = tx.send(Msg::Started {
                what: "archive",
                result,
            });
            if ok {
                let _ = fetch.send(Msg::Archive(api.archive_status().await));
            }
        });
        cx.notify();
    }

    fn start_migrate(&mut self, cx: &mut Context<Self>) {
        if self.migrate_busy {
            return;
        }
        let Some(api) = self.api(cx) else {
            self.toasts(
                ToastKind::Error,
                "Le serveur n'est pas joignable.".to_owned(),
                cx,
            );
            return;
        };
        self.migrate_busy = true;
        self.toasts(
            ToastKind::Info,
            "Mise à niveau lancée — elle continue même si vous fermez l'application.".to_owned(),
            cx,
        );
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            match api.archive_migrate(false).await {
                Ok(_) => {
                    let _ = tx.send(Msg::Migrate(api.archive_migrate_status().await));
                }
                Err(error) => {
                    let _ = tx.send(Msg::Started {
                        what: "migrate",
                        result: Err(error),
                    });
                }
            }
        });
        cx.notify();
    }

    fn start_enrich(&mut self, cx: &mut Context<Self>) {
        if self.enrich_busy {
            return;
        }
        let Some(api) = self.api(cx) else {
            self.toasts(
                ToastKind::Error,
                "Le serveur n'est pas joignable.".to_owned(),
                cx,
            );
            return;
        };
        let force = self.enrich_force;
        self.enrich_busy = true;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            match api.enrich_backfill(force).await {
                Ok(_) => {
                    let result = api
                        .get::<EnrichLive>(Lane::User, "/enrich/backfill")
                        .await
                        .map(|fresh| fresh.value);
                    let _ = tx.send(Msg::Enrich(result));
                }
                Err(error) => {
                    let _ = tx.send(Msg::Started {
                        what: "enrich",
                        result: Err(error),
                    });
                }
            }
        });
        cx.notify();
    }

    fn start_credits(&mut self, cx: &mut Context<Self>) {
        if self.credits_busy {
            return;
        }
        let Some(api) = self.api(cx) else {
            self.toasts(
                ToastKind::Error,
                "Le serveur n'est pas joignable.".to_owned(),
                cx,
            );
            return;
        };
        let force = self.credits_force;
        self.credits_busy = true;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            match api.credits_backfill(force).await {
                Ok(_) => {
                    let _ = tx.send(Msg::Credits(api.credits_backfill_status().await));
                }
                Err(error) => {
                    let _ = tx.send(Msg::Started {
                        what: "credits",
                        result: Err(error),
                    });
                }
            }
        });
        cx.notify();
    }

    fn run_cleanup(&mut self, cx: &mut Context<Self>) {
        if self.cleaning {
            return;
        }
        let Some(api) = self.api(cx) else {
            return;
        };
        self.cleaning = true;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let _ = tx.send(Msg::Cleaned(api.run_cleanup().await));
        });
        cx.notify();
    }

    fn flush_cache(&mut self, cx: &mut Context<Self>) {
        if self.flushing {
            return;
        }
        let Some(api) = self.api(cx) else {
            return;
        };
        self.flushing = true;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let _ = tx.send(Msg::Flushed(api.flush_cache().await));
        });
        cx.notify();
    }

    /// Optimistic — the switch moves under the finger, then the server's answer
    /// decides (`saveRule` in the SPA).
    fn save_rule(&mut self, key: &'static str, next: bool, cx: &mut Context<Self>) {
        let Some(current) = self.rules.as_mut() else {
            return;
        };
        let previous = rule_value(&current.rules, key);
        set_rule(&mut current.rules, key, Value::Bool(next));
        self.saving_rule = Some(key);
        self.push_rule(key, Value::Bool(next), previous, cx);
    }

    fn save_rule_value(&mut self, key: &'static str, next: Value, cx: &mut Context<Self>) {
        let Some(current) = self.rules.as_mut() else {
            return;
        };
        let previous = rule_value(&current.rules, key);
        if let Some(value) = rule_from_json(key, &next) {
            set_rule_json(&mut current.rules, key, value);
        }
        self.saving_rule = Some(key);
        self.push_rule(key, next, previous, cx);
    }

    fn push_rule(
        &mut self,
        key: &'static str,
        next: Value,
        previous: Value,
        cx: &mut Context<Self>,
    ) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let body = serde_json::json!({ key: next });
            let result = api.set_archive_rules(&body).await;
            let _ = tx.send(Msg::RuleSaved {
                key,
                previous,
                result,
            });
        });
    }

    fn save_quota(&mut self, text: &str, cx: &mut Context<Self>) {
        let value = match text.trim().replace(',', ".").parse::<f64>() {
            Ok(value) if value.is_finite() && value >= 0.0 => value,
            _ => {
                self.toasts(
                    ToastKind::Error,
                    "Valeur de quota invalide : un nombre de Go, 0 pour aucune limite.".to_owned(),
                    cx,
                );
                return;
            }
        };
        let Some(api) = self.api(cx) else {
            return;
        };
        self.quota_busy = true;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let body = serde_json::json!({ "upload_quota_gb": value });
            let _ = tx.send(Msg::QuotaSaved(api.set_settings(&body).await));
        });
        cx.notify();
    }

    fn save_transcribe(&mut self, key: &'static str, next: Value, cx: &mut Context<Self>) {
        let Some(current) = self.transcribe.as_mut() else {
            return;
        };
        let previous = transcribe_value(current, key);
        apply_transcribe(current, key, &next);
        self.transcribe_busy = Some(key);
        self.push_transcribe(key, next, previous, cx);
    }

    fn push_transcribe(
        &mut self,
        key: &'static str,
        next: Value,
        previous: Value,
        cx: &mut Context<Self>,
    ) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let body = serde_json::json!({ key: next });
            let result = api.save_transcribe_settings(&body).await;
            let _ = tx.send(Msg::TranscribeSaved {
                key,
                previous,
                result,
            });
        });
    }

    fn probe_engine(&mut self, cx: &mut Context<Self>) {
        if self.probing {
            return;
        }
        let Some(api) = self.api(cx) else {
            return;
        };
        self.probing = true;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let _ = tx.send(Msg::Transcribe(api.transcribe_settings(true).await));
        });
        cx.notify();
    }

    fn issue_worker(&mut self, cx: &mut Context<Self>) {
        if self.worker_busy {
            return;
        }
        let name = self
            .fields
            .get(&Field::WorkerName)
            .map(|field| field.read(cx).value().trim().to_string())
            .unwrap_or_default();
        if name.is_empty() {
            self.toasts(
                ToastKind::Error,
                "Donnez un nom à cet appareil.".to_owned(),
                cx,
            );
            return;
        }
        let kind = self
            .pickers
            .get(&Picker::WorkerKind)
            .and_then(|picker| {
                picker
                    .read(cx)
                    .selected()
                    .map(|item| item.value.to_string())
            })
            .unwrap_or_else(|| "client".to_owned());
        let Some(api) = self.api(cx) else {
            return;
        };
        self.worker_busy = true;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let body = serde_json::json!({ "name": name, "kind": kind });
            let result = api
                .send::<Value, IssuedWorker>(
                    Lane::User,
                    Method::POST,
                    "/transcribe/workers",
                    Some(&body),
                )
                .await
                .and_then(|answer| {
                    answer.ok_or_else(|| {
                        ApiError::Decode("/transcribe/workers: une réponse était attendue".into())
                    })
                });
            let _ = tx.send(Msg::WorkerIssued(result));
        });
        cx.notify();
    }

    fn revoke_worker(&mut self, id: &str, cx: &mut Context<Self>) {
        if !is_uuid(id) {
            self.toasts(
                ToastKind::Error,
                "Identifiant d'appareil invalide.".to_owned(),
                cx,
            );
            return;
        }
        if self.revoking.is_some() {
            return;
        }
        let Some(api) = self.api(cx) else {
            return;
        };
        let id = id.to_owned();
        self.revoking = Some(id.clone());
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        let path = format!("/transcribe/workers/{id}");
        runtime.spawn(async move {
            let result = api
                .send::<(), Value>(Lane::User, Method::DELETE, &path, None::<&()>)
                .await
                .map(|answer| answer.unwrap_or(Value::Null));
            let _ = tx.send(Msg::WorkerRevoked { id, result });
        });
        cx.notify();
    }

    fn copy_issued(&mut self, cx: &mut Context<Self>) {
        let Some(issued) = &self.issued else {
            return;
        };
        if issued.token.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(issued.token.clone()));
        self.issued_copied = true;
        self.toasts(ToastKind::Info, "Jeton copié.".to_owned(), cx);
    }

    fn copy_failure_report(
        &mut self,
        label: &'static str,
        job: &JobStatus,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(failure_report(label, job)));
        self.toasts(ToastKind::Info, "Détail copié.".to_owned(), cx);
    }

    fn toggle_failures(&mut self, label: &'static str, cx: &mut Context<Self>) {
        if let Some(index) = self.open_failures.iter().position(|key| *key == label) {
            self.open_failures.remove(index);
        } else {
            self.open_failures.push(label);
        }
        cx.notify();
    }

    /// Keyboard scrolling for a keyboard-only person: Page Down / Page Up by
    /// four fifths of a screen, Home and End. Handled here (and only the four
    /// keys, everything else propagates) because `keymap.rs` is D7's file and a
    /// page is not a list: `j`/`k` move in a track list, not in Réglages.
    fn on_scroll_key(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page = window.viewport_size().height * 0.8;
        match event.keystroke.key.as_str() {
            "pagedown" => self.scroll_by(page, cx),
            "pageup" => self.scroll_by(-page, cx),
            "home" => {
                self.scroll.set_offset(Default::default());
                cx.notify();
            }
            "end" => {
                self.scroll.scroll_to_bottom();
                cx.notify();
            }
            _ => cx.propagate(),
        }
    }

    fn scroll_by(&mut self, distance: gpui_kit::Pixels, cx: &mut Context<Self>) {
        let mut offset = self.scroll.offset();
        // The offset is the distance from the container's top to the content's:
        // further down is more negative, and the container clamps the bottom.
        offset.y = (offset.y - distance).min(gpui_kit::px(0.));
        self.scroll.set_offset(offset);
        cx.notify();
    }

    fn fail(&mut self, what: &str, error: &ApiError, cx: &mut Context<Self>) {
        self.toasts(
            ToastKind::Error,
            format!("{what} : {}", error.cause_fr()),
            cx,
        );
    }

    fn toasts(&mut self, kind: ToastKind, message: String, cx: &mut Context<Self>) {
        self.state.update(cx, |state, _| {
            state.toasts.push(kind, message);
        });
    }

    // -- lazy controls ---------------------------------------------------

    fn field(
        &mut self,
        key: Field,
        value: String,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(field) = self.fields.get(&key) {
            return field.clone();
        }
        let field = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value)
                .placeholder(placeholder)
        });
        let subscription = cx.subscribe(&field, move |this, field, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                let text = field.read(cx).value().to_string();
                this.commit_field(key, text, cx);
            }
        });
        self.subscriptions.push(subscription);
        self.fields.insert(key, field.clone());
        field
    }

    /// Show the server's value in a number field unless the person is editing
    /// it — a chip, a clamp or a reload must not leave a stale number behind.
    fn sync_field(
        &mut self,
        key: Field,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(field) = self.fields.get(&key).cloned() else {
            return;
        };
        if field.read(cx).value().as_ref() == value.as_str() {
            return;
        }
        if field.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        field.update(cx, |field, cx| field.set_value(value, window, cx));
    }

    fn commit_field(&mut self, key: Field, text: String, cx: &mut Context<Self>) {
        match key {
            Field::Quota => self.save_quota(&text, cx),
            Field::ArtistLimit => {
                if let Ok(value) = text.trim().parse::<u32>() {
                    let clamped = value.clamp(1, 1000);
                    self.save_rule_value("artist_limit", Value::from(clamped), cx);
                }
            }
            Field::CleanFreeGb => {
                if let Ok(value) = text.trim().replace(',', ".").parse::<f64>() {
                    self.save_rule_value(
                        "clean_min_free_gb",
                        serde_json::json!(value.max(0.0)),
                        cx,
                    );
                }
            }
            Field::CleanStaleDays => {
                if let Ok(value) = text.trim().parse::<u32>() {
                    self.save_rule_value("clean_stale_days", Value::from(value.clamp(7, 3650)), cx);
                }
            }
            Field::MaxMinutes => {
                let value = text.trim().parse::<u32>().unwrap_or(0).clamp(5, 1440);
                self.save_transcribe("max_minutes", Value::from(value), cx);
            }
            Field::DailyMinutes => {
                let value = text.trim().parse::<u32>().unwrap_or(0).min(1440);
                self.save_transcribe("daily_minutes", Value::from(value), cx);
            }
            Field::SearchSegments => {
                let value = text.trim().parse::<u64>().unwrap_or(0).min(10_000_000);
                self.save_transcribe("search_segments", Value::from(value), cx);
            }
            Field::WorkerName => {}
        }
    }

    fn picker(
        &mut self,
        key: Picker,
        items: Vec<(String, String)>,
        current: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ChoiceState> {
        if let Some(picker) = self.pickers.get(&key) {
            return picker.clone();
        }
        let selected = items
            .iter()
            .position(|(value, _)| value == current)
            .unwrap_or(0);
        self.picker_values
            .insert(key, items.iter().map(|(value, _)| value.clone()).collect());
        let choices: Vec<ChoiceItem> = items
            .into_iter()
            .map(|(value, label)| ChoiceItem::new(value, label))
            .collect();
        let picker = cx.new(|cx| {
            ChoiceState::new(choices, window, cx)
                .label("Choisir")
                .default_selected(selected)
        });
        self.last_picked.insert(key, current.to_owned());
        let subscription = cx.observe(&picker, move |this, picker, cx| {
            let Some(value) = picker
                .read(cx)
                .selected()
                .map(|item| item.value.to_string())
            else {
                return;
            };
            if this.last_picked.get(&key) == Some(&value) {
                return;
            }
            this.last_picked.insert(key, value.clone());
            this.commit_picker(key, value, cx);
        });
        self.subscriptions.push(subscription);
        self.pickers.insert(key, picker.clone());
        picker
    }

    fn commit_picker(&mut self, key: Picker, value: String, cx: &mut Context<Self>) {
        match key {
            Picker::ArtistScope => {
                self.save_rule_value("artist_scope", Value::from(value), cx);
            }
            Picker::CleanOrder => {
                self.save_rule_value("clean_order", Value::from(value), cx);
            }
            Picker::Language => {
                self.save_transcribe("language", Value::from(value), cx);
            }
            Picker::WorkerKind => {}
        }
    }

    /// Point the artist-scope and cleanup-order pickers at what the server holds,
    /// without treating that as a change (a reload must not re-save).
    fn sync_pickers(&mut self, rules: &ArchiveRules, _window: Option<()>, cx: &mut Context<Self>) {
        let scope = rules.rules.artist_scope.clone();
        self.align_picker(Picker::ArtistScope, &scope, cx);
        let order = rules.rules.clean_order.clone();
        self.align_picker(Picker::CleanOrder, &order, cx);
    }

    fn sync_pickers_from_rules(&mut self, cx: &mut Context<Self>) {
        if let Some(rules) = self.rules.clone() {
            let scope = rules.rules.artist_scope.clone();
            self.align_picker(Picker::ArtistScope, &scope, cx);
            let order = rules.rules.clean_order.clone();
            self.align_picker(Picker::CleanOrder, &order, cx);
        }
    }

    fn sync_pickers_from_transcribe(
        &mut self,
        settings: &TranscribeSettings,
        cx: &mut Context<Self>,
    ) {
        self.align_picker(Picker::Language, &settings.language.clone(), cx);
    }

    fn align_picker(&mut self, key: Picker, value: &str, cx: &mut Context<Self>) {
        let Some(picker) = self.pickers.get(&key).cloned() else {
            return;
        };
        if self.last_picked.get(&key).map(String::as_str) == Some(value) {
            return;
        }
        let Some(index) = self
            .picker_values
            .get(&key)
            .and_then(|values| values.iter().position(|item| item == value))
        else {
            return;
        };
        // `last_picked` first: the observer must not read this as a change and
        // save it back to the server.
        self.last_picked.insert(key, value.to_owned());
        picker.update(cx, |picker, cx| picker.set_selected(Some(index), cx));
    }

    // -- rendering -------------------------------------------------------

    fn render_header(&self, cx: &mut Context<Self>) -> Div {
        let loading = self.loading;
        let mut head = div()
            .flex()
            .items_start()
            .justify_between()
            .gap(rems(1.))
            .flex_wrap()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.25))
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .text_size(rems(1.125))
                            .font_weight(FontWeight::BOLD)
                            .child(self.section.title()),
                    )
                    .child(
                        div()
                            .text_size(rems(0.75))
                            .text_color(theme::secondary(cx))
                            .child(
                                "L'archive de ce serveur, son nettoyage, l'espace disque, \
                                 les rattrapages et la transcription des podcasts.",
                            ),
                    ),
            )
            .child(
                button(
                    "server-refresh",
                    if loading {
                        "Chargement…"
                    } else {
                        "Actualiser"
                    },
                    ButtonVariant::Secondary,
                    cx,
                )
                .disabled(loading)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.page_error = None;
                    this.load(cx);
                })),
            );
        if let Some(error) = &self.page_error {
            head = head.child(
                div()
                    .flex()
                    .items_start()
                    .gap(rems(0.5))
                    .p(rems(0.625))
                    .border_1()
                    .border_color(theme::danger(cx))
                    .text_size(rems(0.75))
                    .text_color(theme::danger(cx))
                    .child(icon(IconName::CircleAlert).size(rems(1.)).mt(rems(0.0625)))
                    .child(div().flex_1().child(error.clone())),
            );
        }
        head
    }

    fn render_archive(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let status = self.archive.clone();
        let busy = self.archive_busy;
        let running = status.as_ref().is_some_and(|job| job.running);
        let mut card = self
            .card("server-archive", "Archive du serveur", cx)
            .child(self.sub(
                "Un titre archivé reste lisible même si Deezer le retire de son catalogue ou \
             devient injoignable. L'archivage part tout seul dès qu'un titre entre dans votre \
             bibliothèque ; ce bouton rattrape ce qui manquerait encore. Rien n'est jamais \
             supprimé de l'archive.",
                cx,
            ));

        if let Some(job) = &status
            && running
        {
            card = card
                .child(self.gauge(fraction(job.done, job.total), cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(rems(0.75))
                        .flex_wrap()
                        .text_size(rems(0.75))
                        .child(format!("Archivage en cours… {} / {}", job.done, job.total))
                        .child(
                            div()
                                .text_color(theme::secondary(cx))
                                .child(archive_counts(job.archived, job.unavailable)),
                        ),
                )
                .child(self.hint(
                    "Vous pouvez fermer l'application : le serveur continue.",
                    cx,
                ));
        } else {
            let mut actions = div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .flex_wrap()
                .child(
                    button(
                        "archive-all",
                        if busy {
                            "Démarrage…"
                        } else {
                            "Tout archiver maintenant"
                        },
                        ButtonVariant::Primary,
                        cx,
                    )
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.start_archive("all", cx))),
                );
            for (scope, label) in [
                ("favorites", "Favoris"),
                ("playlists", "Playlists"),
                ("podcasts", "Podcasts"),
            ] {
                actions = actions.child(
                    button(
                        format!("archive-{scope}"),
                        label,
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.start_archive(scope, cx))),
                );
            }
            card = card.child(actions);
            if let Some(job) = &status {
                if job.total > 0 {
                    card = card.child(self.hint(
                        format!(
                            "Dernier passage : {} archivé(s){}{}.",
                            job.archived,
                            if job.unavailable > 0 {
                                format!(", {} indisponible(s)", job.unavailable)
                            } else {
                                String::new()
                            },
                            if job.failed > 0 {
                                format!(", {} échec(s)", job.failed)
                            } else {
                                String::new()
                            },
                        ),
                        cx,
                    ));
                }
                if let Some(error) = &job.error {
                    card = card.child(self.error_line(error.clone(), cx));
                }
            }
        }
        card
    }

    fn render_migrate(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let status = self.migrate.clone();
        let busy = self.migrate_busy;
        let running = status.as_ref().is_some_and(|job| job.running);
        let mut card = self
            .card("server-migrate", "Mise à niveau des fichiers", cx)
            .child(self.sub(
                "Réécrit les titres archivés dans le format actuel : plus de volume Deezer dans \
                 les étiquettes, la date de sortie la plus ancienne, les faits dans le fichier \
                 compagnon. Relancer ne change plus rien.",
                cx,
            ));
        if let Some(job) = &status
            && running
        {
            let tags = job.counts.get("tags").copied().unwrap_or(0);
            let bpm = job.counts.get("bpm").copied().unwrap_or(0);
            card = card.child(self.gauge(None, cx)).child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(rems(0.75))
                    .flex_wrap()
                    .text_size(rems(0.75))
                    .child(format!("Mise à niveau en cours… {} fichier(s)", job.done))
                    .child(
                        div()
                            .text_color(theme::secondary(cx))
                            .child(format!("{tags} étiquette(s) · {bpm} tempo(s) appris")),
                    ),
            );
        } else {
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .flex_wrap()
                    .child(
                        button(
                            "migrate-start",
                            if busy {
                                "Démarrage…"
                            } else {
                                "Mettre à niveau les fichiers"
                            },
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.start_migrate(cx))),
                    ),
            );
            if let Some(job) = &status {
                if job.done > 0 || job.failed > 0 {
                    card = card.child(self.hint(
                        format!(
                            "Dernier passage : {} fichier(s){}{}.",
                            job.done,
                            if job.counts.get("tags").copied().unwrap_or(0) > 0 {
                                format!(
                                    ", {} étiquette(s)",
                                    job.counts.get("tags").copied().unwrap_or(0)
                                )
                            } else {
                                String::new()
                            },
                            if job.failed > 0 {
                                format!(", {} échec(s)", job.failed)
                            } else {
                                String::new()
                            },
                        ),
                        cx,
                    ));
                }
                if let Some(error) = &job.error {
                    card = card.child(self.error_line(error.clone(), cx));
                }
                let snapshot = JobStatus {
                    running: job.running,
                    done: job.done,
                    failed: job.failed,
                    error: job.error.clone(),
                    failure_reasons: job.failure_reasons.clone(),
                    failures: job.failures.clone(),
                    ..JobStatus::default()
                };
                if let Some(block) = self.failure_block("migrate", &snapshot, cx) {
                    card = card.child(block);
                }
            }
        }
        card
    }

    fn render_enrich(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let live = self.enrich.clone();
        let busy = self.enrich_busy;
        let force = self.enrich_force;
        let running = live.as_ref().is_some_and(|job| job.running);
        let mut card = self
            .card("server-enrich", "Faits complétés par MusicBrainz", cx)
            .child(self.sub(
                "Date de première sortie, compositeur, parolier et producteur, complétés en \
                 arrière-plan à une requête par seconde — jamais pendant la lecture.",
                cx,
            ));
        if let Some(job) = &live
            && running
        {
            card = card
                .child(self.gauge(fraction(job.job.done + job.job.failed, job.job.total), cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(rems(0.75))
                        .flex_wrap()
                        .text_size(rems(0.75))
                        .child(format!(
                            "{}/{} titres",
                            job.job.done + job.job.failed,
                            if job.job.total > 0 {
                                job.job.total.to_string()
                            } else {
                                "?".to_owned()
                            }
                        ))
                        .child(div().text_color(theme::secondary(cx)).child(
                            if job.job.failed > 0 {
                                format!("{} en échec", job.job.failed)
                            } else {
                                String::new()
                            },
                        )),
                );
        } else {
            let enabled = live.as_ref().map(|job| job.enabled).unwrap_or(true);
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.75))
                    .flex_wrap()
                    .child(
                        button(
                            "enrich-start",
                            if busy {
                                "Démarrage…"
                            } else {
                                "Compléter depuis MusicBrainz"
                            },
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .disabled(busy || !enabled)
                        .on_click(cx.listener(|this, _, _, cx| this.start_enrich(cx))),
                    )
                    .child(self.bind_enrich_force(force, cx)),
            );
            if !enabled {
                card = card
                    .child(self.hint("Désactivé dans la configuration ([enrich] enabled).", cx));
            } else if let Some(job) = &live {
                let found: u32 = job.coverage.found.values().sum();
                let by_method = ["isrc", "acoustid", "search"]
                    .iter()
                    .filter_map(|method| {
                        job.coverage
                            .found
                            .get(*method)
                            .filter(|count| **count > 0)
                            .map(|count| format!("{count} par {method}"))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                card = card.child(self.hint(
                    if job.coverage.tried > 0 {
                        format!(
                            "{found} titre(s) complété(s){}, {} pas encore essayé(s).",
                            if by_method.is_empty() {
                                String::new()
                            } else {
                                format!(" ({by_method})")
                            },
                            job.coverage.untried
                        )
                    } else {
                        "Un titre archivé ou téléversé est complété tout seul, à une requête par seconde."
                            .to_owned()
                    },
                    cx,
                ));
                if !job.acoustid {
                    card = card.child(self.hint(
                        "L'empreinte sonore (AcoustID) demande l'outil fpcalc et une clé.",
                        cx,
                    ));
                }
                if let Some(error) = &job.job.error {
                    card = card.child(self.error_line(error.clone(), cx));
                }
                if let Some(block) = self.failure_block("enrich", &job.job, cx) {
                    card = card.child(block);
                }
            }
        }
        card
    }

    fn render_credits(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let live = self.credits.clone();
        let busy = self.credits_busy;
        let force = self.credits_force;
        let running = live.as_ref().is_some_and(|job| job.job.running);
        let mut card = self
            .card("server-credits", "Crédits des titres", cx)
            .child(self.sub(
                "Compositeur, auteur et producteur de chaque titre, gardés en base pour le lecteur \
             plein écran et complétés en arrière-plan.",
                cx,
            ));
        if let Some(job) = &live
            && running
        {
            let deezer = job.counts.get("deezer").copied().unwrap_or(0);
            let local = job.counts.get("local").copied().unwrap_or(0);
            card = card
                .child(self.gauge(fraction(job.job.done + job.job.failed, job.job.total), cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(rems(0.75))
                        .flex_wrap()
                        .text_size(rems(0.75))
                        .child(format!(
                            "{}/{} titres",
                            job.job.done + job.job.failed,
                            if job.job.total > 0 {
                                job.job.total.to_string()
                            } else {
                                "?".to_owned()
                            }
                        ))
                        .child(div().text_color(theme::secondary(cx)).child(format!(
                            "{deezer} par Deezer · {local} depuis les fichiers{}",
                            if job.job.failed > 0 {
                                format!(" · {} en échec", job.job.failed)
                            } else {
                                String::new()
                            }
                        ))),
                );
        } else {
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.75))
                    .flex_wrap()
                    .child(
                        button(
                            "credits-start",
                            if busy {
                                "Démarrage…"
                            } else {
                                "Compléter les crédits"
                            },
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.start_credits(cx))),
                    )
                    .child(self.bind_credits_force(force, cx)),
            );
            if let Some(job) = &live {
                card = card.child(self.hint(
                    if job.tracks > 0 {
                        format!(
                            "{} titre(s) ont leurs crédits. {}",
                            job.tracks,
                            if job.completed {
                                "La bibliothèque archivée avant a été complétée."
                            } else {
                                "La bibliothèque archivée avant sera complétée au prochain démarrage."
                            }
                        )
                    } else {
                        "Compositeur, auteur et producteur de chaque titre, sans jamais ralentir la lecture."
                            .to_owned()
                    },
                    cx,
                ));
                if let Some(error) = &job.job.error {
                    card = card.child(self.error_line(error.clone(), cx));
                }
                if let Some(block) = self.failure_block("credits", &job.job, cx) {
                    card = card.child(block);
                }
            }
        }
        card
    }

    fn render_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let Some(rules) = self.rules.clone() else {
            return self
                .card("server-rules", "Règles d'archivage", cx)
                .child(self.hint("Chargement des règles…", cx));
        };
        let saving = self.saving_rule;
        let mut card = self
            .card("server-rules", "Règles d'archivage", cx)
            .child(self.sub(
            "Ce qui déclenche un archivage. La lecture n'est pas dans la liste : un titre lu est \
             toujours archivé, c'est ainsi que fonctionne le transcodage.",
            cx,
        ));
        if !rules.archive_library {
            card = card.child(self.warning_line(
                "L'archivage est désactivé côté serveur (archive_library) : ces règles sont sans effet.",
                cx,
            ));
        }

        let mut events = div().flex().flex_col().gap(rems(0.125));
        for event in &rules.events {
            let Some(key) = event_key(event) else {
                continue;
            };
            let checked = rule_value(&rules.rules, key).as_bool().unwrap_or(false);
            events = events.child(self.bind_rule_switch(
                event_id(key),
                event_label(key),
                checked,
                key,
                saving,
                cx,
            ));
        }
        card = card.child(events);

        card = card.child(
            div()
                .text_size(rems(0.8125))
                .font_weight(FontWeight::SEMIBOLD)
                .mt(rems(0.5))
                .child("Artiste mis en favori"),
        ).child(self.hint(
            "Une discographie complète peut représenter des dizaines de Go. Limitez-la si le disque est petit.",
            cx,
        ));

        let mut scopes: Vec<(String, String)> = rules
            .artist_scopes
            .iter()
            .map(|scope| (scope.clone(), scope_label(scope).to_owned()))
            .collect();
        if scopes.is_empty() {
            scopes.push(("all".to_owned(), scope_label("all").to_owned()));
        }
        let scope_picker = self.picker(
            Picker::ArtistScope,
            scopes,
            &rules.rules.artist_scope,
            window,
            cx,
        );
        let mut artist_row = div().flex().items_end().gap(rems(0.75)).flex_wrap().child(
            div()
                .flex()
                .flex_col()
                .gap(rems(0.25))
                .w(rems(18.))
                .child(self.label("Portée"))
                .child(select("rule-artist-scope", &scope_picker, window, cx)),
        );
        if rules.rules.artist_scope != "all" {
            let limit = self.field(
                Field::ArtistLimit,
                rules.rules.artist_limit.to_string(),
                "N",
                window,
                cx,
            );
            artist_row = artist_row.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.25))
                    .child(self.label("Combien de titres"))
                    .child(input("rule-artist-limit", &limit, window, cx).max_w(rems(7.))),
            );
        }
        card = card.child(artist_row);

        card = card.child(
            div()
                .text_size(rems(0.8125))
                .font_weight(FontWeight::SEMIBOLD)
                .mt(rems(0.5))
                .child("Chapitres des épisodes"),
        );
        card = card
            .child(self.bind_rule_switch(
                "rule-chapters",
                "Lire les chapitres à l'archivage — depuis le fichier de l'épisode, sans requête à personne",
                rules.rules.chapters,
                "chapters",
                saving,
                cx,
            ))
            .child(self.bind_rule_switch(
                "rule-chapter-urls",
                "Suivre aussi le lien de chapitres du fichier — une requête tierce par épisode",
                rules.rules.chapter_urls,
                "chapter_urls",
                saving,
                cx,
            ));
        card
    }

    fn render_cleanup(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let Some(rules) = self.rules.clone() else {
            return self.card("server-cleanup", "Nettoyage automatique", cx);
        };
        let preview = self.cleanup.clone();
        let cleaning = self.cleaning;
        let saving = self.saving_rule;
        let mut card = self
            .card("server-cleanup", "Nettoyage automatique", cx)
            .border_color(theme::danger(cx))
            .child(self.sub(
                "La seule fonction qui supprime de l'audio archivé. Elle ne touche que des titres \
                 re-téléchargeables depuis Deezer — un fichier que vous avez importé vous-même \
                 n'existe nulle part ailleurs et n'est jamais concerné. Seuls les octets partent : \
                 le titre reste dans vos playlists et se réarchive à la prochaine lecture.",
                cx,
            ))
            .child(self.bind_rule_switch(
                "clean-on",
                "Activer le nettoyage quand le disque se remplit",
                rules.rules.clean_on,
                "clean_on",
                saving,
                cx,
            ));

        if rules.rules.clean_on {
            let free = self.field(
                Field::CleanFreeGb,
                trim_float(rules.rules.clean_min_free_gb),
                "Go",
                window,
                cx,
            );
            let days = self.field(
                Field::CleanStaleDays,
                rules.rules.clean_stale_days.to_string(),
                "jours",
                window,
                cx,
            );
            card = card.child(
                div()
                    .flex()
                    .items_end()
                    .gap(rems(0.75))
                    .flex_wrap()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(rems(0.25))
                            .child(self.label("Déclencher sous (Go libres)"))
                            .child(input("clean-free-gb", &free, window, cx).max_w(rems(8.))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(rems(0.25))
                            .child(self.label("Pas écouté depuis (jours)"))
                            .child(input("clean-stale-days", &days, window, cx).max_w(rems(8.))),
                    ),
            );
            if rules.rules.clean_min_free_gb == 0.0 {
                card = card
                    .child(self.warning_line("Seuil à 0 Go : rien ne sera jamais supprimé.", cx));
            }

            card = card
                .child(
                    div()
                        .text_size(rems(0.8125))
                        .font_weight(FontWeight::SEMIBOLD)
                        .mt(rems(0.5))
                        .child("Ce qui est protégé"),
                )
                .child(self.bind_rule_switch(
                    "clean-keep-fav",
                    "Mes favoris",
                    rules.rules.clean_keep_fav,
                    "clean_keep_fav",
                    saving,
                    cx,
                ))
                .child(self.bind_rule_switch(
                    "clean-keep-playlist",
                    "Les titres présents dans une playlist",
                    rules.rules.clean_keep_playlist,
                    "clean_keep_playlist",
                    saving,
                    cx,
                ))
                .child(self.bind_rule_switch(
                    "clean-keep-podcast",
                    "Les épisodes de podcast — un podcast disparu de Deezer reste protégé quoi qu'il arrive",
                    rules.rules.clean_keep_podcast,
                    "clean_keep_podcast",
                    saving,
                    cx,
                ))
                .child(
                    div()
                        .text_size(rems(0.8125))
                        .font_weight(FontWeight::SEMIBOLD)
                        .mt(rems(0.5))
                        .child("Priorité de suppression"),
                )
                .child(self.hint(
                    "Ce qui part en premier quand il faut faire de la place.",
                    cx,
                ));

            let mut orders: Vec<(String, String)> = rules
                .cleanup_orders
                .iter()
                .map(|order| (order.clone(), order_label(order).to_owned()))
                .collect();
            if orders.is_empty() {
                orders.push((
                    "oldest_play".to_owned(),
                    order_label("oldest_play").to_owned(),
                ));
            }
            let order = self.picker(
                Picker::CleanOrder,
                orders,
                &rules.rules.clean_order,
                window,
                cx,
            );
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.25))
                    .w(rems(18.))
                    .child(self.label("Ordre"))
                    .child(select("clean-order", &order, window, cx)),
            );

            let can_run = preview
                .as_ref()
                .map(|preview| preview.needed_bytes > 0)
                .unwrap_or(false);
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .flex_wrap()
                    .mt(rems(0.25))
                    .child(
                        button(
                            "cleanup-preview",
                            "Prévisualiser",
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.fetch_cleanup(cx))),
                    )
                    .child(
                        button(
                            "cleanup-run",
                            if cleaning {
                                "Nettoyage…"
                            } else {
                                "Libérer maintenant"
                            },
                            ButtonVariant::Danger,
                            cx,
                        )
                        .disabled(cleaning || !can_run)
                        .on_click(cx.listener(|this, _, _, cx| this.run_cleanup(cx))),
                    ),
            );

            if let Some(preview) = &preview {
                card = card.child(self.hint(
                    if preview.needed_bytes > 0 {
                        format!(
                            "Il manque {} pour revenir au seuil ; {} titre(s) partiraient, soit {}.",
                            bytes(preview.needed_bytes),
                            preview.tracks.len(),
                            bytes(preview.would_free)
                        )
                    } else {
                        format!(
                            "Le disque est au-dessus du seuil : rien ne sera supprimé. \
                             {} titre(s) seraient éligibles si nécessaire.",
                            preview.eligible
                        )
                    },
                    cx,
                ));
                if !preview.tracks.is_empty() {
                    let mut list = div().flex().flex_col().gap(rems(0.125)).mt(rems(0.25));
                    for row in preview.tracks.iter().take(CLEANUP_ROWS) {
                        let failure: Failure =
                            serde_json::from_value(row.clone()).unwrap_or_default();
                        let title = row
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or(failure.track.as_str())
                            .to_owned();
                        let artist = row
                            .get("artist")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        let size = row.get("bytes").and_then(Value::as_u64).unwrap_or(0);
                        list = list.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(rems(0.75))
                                .py(rems(0.125))
                                .border_b_1()
                                .border_color(theme::border(cx))
                                .text_size(rems(0.75))
                                .child(div().flex_1().min_w_0().text_ellipsis().child(title))
                                .child(
                                    div()
                                        .w(rems(12.))
                                        .min_w_0()
                                        .text_ellipsis()
                                        .text_color(theme::secondary(cx))
                                        .child(artist),
                                )
                                .child(div().text_color(theme::secondary(cx)).child(bytes(size))),
                        );
                    }
                    card = card.child(list);
                }
            }
        }
        card
    }

    fn render_storage(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let Some(storage) = self.storage.clone() else {
            return self
                .card("server-storage", "Stockage serveur", cx)
                .child(self.hint("Chargement de l'espace disque…", cx));
        };
        let flushing = self.flushing;
        let mut card = self
            .card("server-storage", "Stockage serveur", cx)
            .child(
                div().flex().items_center().justify_between().gap(rems(0.75)).flex_wrap()
                    .child(self.sub(
                        "Le cache (transcodages Opus, pochettes proxifiées) se reconstruit tout \
                         seul : le vider ne coûte que du CPU. L'archive, elle, n'est pas un cache — \
                         aucun bouton ici ne peut y toucher.",
                        cx,
                    ))
                    .child(
                        button(
                            "storage-flush",
                            if flushing { "Vidage…" } else { "Vider le cache" },
                            ButtonVariant::Danger,
                            cx,
                        )
                        .disabled(flushing)
                        .on_click(cx.listener(|this, _, _, cx| this.flush_cache(cx))),
                    ),
            );

        let used = storage.disk_total.saturating_sub(storage.disk_free);
        card = card
            .child(self.gauge(fraction_u64(used, storage.disk_total), cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(rems(0.75))
                    .flex_wrap()
                    .text_size(rems(0.75))
                    .child(format!(
                        "{} libres sur le disque d'archives",
                        bytes(storage.disk_free)
                    ))
                    .child(
                        div()
                            .text_color(theme::secondary(cx))
                            .child(format!("{} au total", bytes(storage.disk_total))),
                    ),
            );

        let mut stats = div().flex().gap(rems(0.5)).flex_wrap();
        for (value, label) in [
            (bytes(storage.archive_bytes), "Archive"),
            (
                format!("{} / {}", storage.tracks_archived, storage.tracks_total),
                "Titres archivés",
            ),
            (bytes(storage.transcode_bytes), "Cache transcodage"),
            (bytes(storage.cache_bytes), "Cache images"),
        ] {
            stats = stats.child(self.stat(value, label, cx));
        }
        card = card.child(stats);
        if !storage.archive_dir.is_empty() {
            card = card.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .font_family(theme::mono(cx))
                    .child(storage.archive_dir.clone()),
            );
        }
        card
    }

    fn render_transcribe(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let Some(settings) = self.transcribe.clone() else {
            return self
                .card("server-transcribe", "Transcription des podcasts", cx)
                .child(self.hint("Chargement des réglages de transcription…", cx));
        };
        let busy = self.transcribe_busy;
        let probing = self.probing;
        let reason = settings
            .reason
            .as_deref()
            .map(transcribe_reason)
            .unwrap_or_default();
        let mut card = self
            .card("server-transcribe", "Transcription des podcasts", cx)
            .child(self.sub(
                "Le texte est produit sur ce serveur, à partir des fichiers archivés. Aucun audio \
                 n'est envoyé à un tiers. Rien n'est transcrit automatiquement : chaque épisode se \
                 demande depuis sa page.",
                cx,
            ));

        if !settings.available {
            card = card.child(self.warning_line(&reason, cx));
        }

        card = card.child(self.bind_transcribe_switch(
            "transcribe-enabled",
            "Transcrire les podcasts",
            settings.enabled,
            "enabled",
            busy,
            cx,
        ));

        let languages: Vec<(String, String)> = if settings.languages.is_empty() {
            ["auto", "fr", "en"]
                .iter()
                .map(|code| (code.to_string(), language_label(code)))
                .collect()
        } else {
            settings
                .languages
                .iter()
                .map(|code| (code.clone(), language_label(code)))
                .collect()
        };
        let language = self.picker(Picker::Language, languages, &settings.language, window, cx);
        let max_minutes = self.field(
            Field::MaxMinutes,
            settings.max_minutes.to_string(),
            "min",
            window,
            cx,
        );
        self.sync_field(
            Field::MaxMinutes,
            settings.max_minutes.to_string(),
            window,
            cx,
        );
        let daily = self.field(
            Field::DailyMinutes,
            settings.daily_minutes.to_string(),
            "min",
            window,
            cx,
        );
        self.sync_field(
            Field::DailyMinutes,
            settings.daily_minutes.to_string(),
            window,
            cx,
        );
        let segments = self.field(
            Field::SearchSegments,
            settings.search_segments.to_string(),
            "segments",
            window,
            cx,
        );
        self.sync_field(
            Field::SearchSegments,
            settings.search_segments.to_string(),
            window,
            cx,
        );

        card = card.child(
            div()
                .flex()
                .flex_wrap()
                .gap(rems(0.75))
                .mt(rems(0.25))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.25))
                        .w(rems(16.))
                        .child(self.label("Langue des prochaines transcriptions"))
                        .child(select("transcribe-language", &language, window, cx)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.25))
                        .child(self.label("Durée maximale d'un épisode (min)"))
                        .child(input("transcribe-max", &max_minutes, window, cx).max_w(rems(7.))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.25))
                        .child(self.label("Budget automatique par jour (min)"))
                        .child(input("transcribe-daily", &daily, window, cx).max_w(rems(7.))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.25))
                        .child(self.label("Recherche : segments parcourus au plus"))
                        .child(input("transcribe-segments", &segments, window, cx).max_w(rems(9.))),
                ),
        );

        let mut chips = div().flex().items_center().gap(rems(0.375)).flex_wrap();
        for minutes in [30u32, 60, 120, 180, 240, 360] {
            let selected = settings.max_minutes == minutes;
            chips = chips.child(
                button(
                    format!("transcribe-chip-{minutes}"),
                    format!("{minutes} min"),
                    if selected {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    },
                    cx,
                )
                .disabled(busy.is_some())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.save_transcribe("max_minutes", Value::from(minutes), cx);
                })),
            );
        }
        card = card.child(chips);

        let mut facts = div().flex().flex_col().gap(rems(0.125)).mt(rems(0.25));
        let engine = settings.engine.clone().unwrap_or_default();
        facts = facts.child(self.fact_line(
            "Moteur",
            format!(
                "{}{} — {}",
                if engine.name.is_empty() {
                    "whisper.cpp".to_owned()
                } else {
                    engine.name.clone()
                },
                engine
                    .version
                    .as_ref()
                    .map(|version| format!(" {version}"))
                    .unwrap_or_default(),
                if engine.ok { "trouvé" } else { "absent" }
            ),
            cx,
        ));
        if let Some(model) = &settings.model {
            facts = facts.child(self.fact_line(
                "Modèle",
                if model.name.as_deref().unwrap_or("").is_empty() {
                    "aucun modèle configuré — introuvable".to_owned()
                } else {
                    format!(
                        "{} · {}{}",
                        model.name.clone().unwrap_or_default(),
                        bytes(model.size_mb as u64 * 1024 * 1024),
                        if model.ok {
                            " — valide"
                        } else {
                            " — invalide"
                        }
                    )
                },
                cx,
            ));
        }
        if settings.threads > 0 {
            facts = facts.child(self.fact_line(
                "Calcul",
                format!("{} fils de calcul sur ce serveur", settings.threads),
                cx,
            ));
        }
        let active = settings.queue.running + settings.queue.leased;
        if active > 0 || settings.queue.queued > 0 {
            facts = facts.child(self.fact_line(
                "File",
                format!("{active} en cours, {} en attente", settings.queue.queued),
                cx,
            ));
        }
        card = card.child(facts);

        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.75))
                .flex_wrap()
                .mt(rems(0.25))
                .child(
                    button(
                        "transcribe-probe",
                        if probing {
                            "Vérification…"
                        } else {
                            "Tester le moteur"
                        },
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(probing)
                    .on_click(cx.listener(|this, _, _, cx| this.probe_engine(cx))),
                )
                .child(if settings.available {
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.375))
                        .text_size(rems(0.75))
                        .text_color(theme::success(cx))
                        .child(icon(IconName::CircleCheck).size(rems(0.875)))
                        .child("Prêt à transcrire")
                } else {
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child("Le moteur reste désactivé tant que le serveur n'est pas prêt.")
                }),
        );

        card = card.child(self.render_workers(window, cx));
        card
    }

    fn render_workers(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mut block = div()
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .mt(rems(0.75))
            .pt(rems(0.75))
            .border_t_1()
            .border_color(theme::border(cx))
            .child(
                div()
                    .text_size(rems(0.8125))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Appareils de transcription"),
            )
            .child(self.hint(
                "Les machines qui décodent les épisodes. Un jeton n'est affiché qu'une fois, à la \
                 création : copiez-le sur la machine, il ne revient jamais par ici.",
                cx,
            ));

        if let Some(issued) = self.issued.clone() {
            let copied = self.issued_copied;
            block = block.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.375))
                    .p(rems(0.75))
                    .border_1()
                    .border_color(theme::accent(cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.5))
                            .text_size(rems(0.8125))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(
                                icon(IconName::TriangleAlert)
                                    .size(rems(1.))
                                    .text_color(theme::warning(cx)),
                            )
                            .child(format!("Jeton de « {} »", issued.worker.name))
                            .child(
                                div()
                                    .text_size(rems(0.6875))
                                    .font_weight(FontWeight::NORMAL)
                                    .text_color(theme::secondary(cx))
                                    .child("— affiché une seule fois"),
                            ),
                    )
                    .child(
                        div()
                            .id("worker-token")
                            .w_full()
                            .p(rems(0.5))
                            .border_1()
                            .border_color(theme::border(cx))
                            .bg(theme::sidebar(cx))
                            .font_family(theme::mono(cx))
                            .text_size(rems(0.75))
                            .text_ellipsis()
                            .child(issued.token.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.5))
                            .child(
                                button(
                                    "worker-token-copy",
                                    if copied {
                                        "Jeton copié"
                                    } else {
                                        "Copier le jeton"
                                    },
                                    ButtonVariant::Primary,
                                    cx,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.copy_issued(cx))),
                            )
                            .child(
                                button(
                                    "worker-token-done",
                                    "J'ai copié le jeton",
                                    ButtonVariant::Secondary,
                                    cx,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.issued = None;
                                        this.issued_copied = false;
                                        cx.notify();
                                    },
                                )),
                            ),
                    ),
            );
        }

        let name = self.field(
            Field::WorkerName,
            String::new(),
            "Nom de la machine",
            window,
            cx,
        );
        let kinds = vec![
            ("client".to_owned(), "Client (ce PC)".to_owned()),
            ("server".to_owned(), "Serveur".to_owned()),
        ];
        let kind = self.picker(Picker::WorkerKind, kinds, "client", window, cx);
        let busy = self.worker_busy;
        block = block.child(
            div()
                .flex()
                .items_end()
                .gap(rems(0.5))
                .flex_wrap()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.25))
                        .flex_1()
                        .min_w(rems(12.))
                        .child(self.label("Enrôler un appareil"))
                        .child(input("worker-name", &name, window, cx)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.25))
                        .w(rems(12.))
                        .child(self.label("Type"))
                        .child(select("worker-kind", &kind, window, cx)),
                )
                .child(
                    button(
                        "worker-create",
                        if busy {
                            "Création…"
                        } else {
                            "Créer l'appareil"
                        },
                        ButtonVariant::Primary,
                        cx,
                    )
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.issue_worker(cx))),
                ),
        );

        if let Some(error) = &self.workers_error {
            block = block.child(self.error_line(error.clone(), cx));
        } else if let Some(workers) = self.workers.clone() {
            if workers.is_empty() {
                block = block.child(self.hint("Aucun appareil enrôlé pour l'instant.", cx));
            } else {
                let mut list = div().flex().flex_col();
                for worker in &workers {
                    list = list.child(self.worker_row(worker, cx));
                }
                block = block.child(list);
            }
        } else {
            block = block.child(self.hint("Chargement des appareils…", cx));
        }
        block
    }

    fn worker_row(&self, worker: &WorkerRow, cx: &mut Context<Self>) -> Div {
        let revoked = worker.revoked.is_some();
        let confirming = self.confirming_revoke.as_deref() == Some(worker.id.as_str());
        let revoking = self.revoking.as_deref() == Some(worker.id.as_str());
        let id = worker.id.clone();
        let id_confirm = id.clone();
        let id_revoke = id.clone();
        let mut row = div()
            .flex()
            .items_center()
            .gap(rems(0.625))
            .flex_wrap()
            .py(rems(0.375))
            .border_b_1()
            .border_color(theme::border(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.125))
                    .flex_1()
                    .min_w(rems(10.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.5))
                            .text_size(rems(0.8125))
                            .child(worker.name.clone())
                            .child(self.pill(
                                if worker.kind == "server" {
                                    "Serveur"
                                } else {
                                    "Client"
                                },
                                theme::secondary(cx),
                                cx,
                            ))
                            .child(self.pill(
                                if revoked { "Révoqué" } else { "Actif" },
                                if revoked {
                                    theme::secondary(cx)
                                } else {
                                    theme::success(cx)
                                },
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .text_color(theme::secondary(cx))
                            .child(worker_detail(worker)),
                    ),
            );
        if !revoked {
            row = row.child(if confirming {
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.375))
                    .child(
                        button("worker-cancel", "Annuler", ButtonVariant::Secondary, cx).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.confirming_revoke = None;
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        button(
                            "worker-confirm",
                            if revoking {
                                "Révocation…"
                            } else {
                                "Confirmer"
                            },
                            ButtonVariant::Danger,
                            cx,
                        )
                        .disabled(revoking)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.revoke_worker(&id_revoke, cx);
                        })),
                    )
            } else {
                div().child(
                    button("worker-revoke", "Révoquer", ButtonVariant::Danger, cx).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.confirming_revoke = Some(id_confirm.clone());
                            cx.notify();
                        }),
                    ),
                )
            });
        }
        row
    }

    fn render_quota(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let Some(settings) = self.settings.clone() else {
            return self
                .card("server-quota", "Quota d'upload", cx)
                .child(self.hint("Chargement du quota…", cx));
        };
        let busy = self.quota_busy;
        let field = self.field(
            Field::Quota,
            trim_float(settings.upload_quota_gb),
            "Go",
            window,
            cx,
        );
        self.sync_field(
            Field::Quota,
            trim_float(settings.upload_quota_gb),
            window,
            cx,
        );
        self.card(
            "server-quota",
            "Quota d'upload (utilisateurs non-admin)",
            cx,
        )
        .child(self.sub(
            "Limite l'espace total que chaque utilisateur non-administrateur peut occuper \
                 avec ses fichiers importés. Les administrateurs ne sont pas limités. 0 désactive \
                 la limite.",
            cx,
        ))
        .child(
            div()
                .flex()
                .items_end()
                .gap(rems(0.5))
                .flex_wrap()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.25))
                        .child(self.label("Go par utilisateur"))
                        .child(input("quota-gb", &field, window, cx).max_w(rems(9.))),
                )
                .child(
                    button(
                        "quota-save",
                        if busy {
                            "Enregistrement…"
                        } else {
                            "Enregistrer"
                        },
                        ButtonVariant::Primary,
                        cx,
                    )
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let text = this
                            .fields
                            .get(&Field::Quota)
                            .map(|field| field.read(cx).value().to_string())
                            .unwrap_or_default();
                        this.save_quota(&text, cx);
                    })),
                ),
        )
    }

    // -- element helpers -------------------------------------------------

    fn card(&self, id: &'static str, title: &'static str, cx: &App) -> Stateful<Div> {
        div()
            .id(id)
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .p(rems(1.125))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .font_family(theme::font(cx))
            .text_color(theme::text(cx))
            .child(
                div()
                    .text_size(rems(0.9375))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
    }

    fn label(&self, text: &'static str) -> Div {
        div()
            .text_size(rems(0.625))
            .font_weight(FontWeight::SEMIBOLD)
            .child(text)
    }

    fn sub(&self, text: &str, cx: &App) -> Div {
        div()
            .max_w(rems(52.))
            .text_size(rems(0.75))
            .text_color(theme::secondary(cx))
            .child(text.to_owned())
    }

    fn hint(&self, text: impl Into<SharedString>, cx: &App) -> Div {
        div()
            .max_w(rems(52.))
            .text_size(rems(0.6875))
            .text_color(theme::secondary(cx))
            .child(text.into())
    }

    fn error_line(&self, text: impl Into<SharedString>, cx: &App) -> Div {
        div()
            .flex()
            .items_start()
            .gap(rems(0.375))
            .text_size(rems(0.6875))
            .text_color(theme::danger(cx))
            .child(
                icon(IconName::CircleAlert)
                    .size(rems(0.875))
                    .mt(rems(0.0625)),
            )
            .child(div().flex_1().child(text.into()))
    }

    fn warning_line(&self, text: &str, cx: &App) -> Div {
        div()
            .flex()
            .items_start()
            .gap(rems(0.375))
            .text_size(rems(0.6875))
            .text_color(theme::warning(cx))
            .child(
                icon(IconName::TriangleAlert)
                    .size(rems(0.875))
                    .mt(rems(0.0625)),
            )
            .child(div().flex_1().child(text.to_owned()))
    }

    fn gauge(&self, fraction: Option<f32>, cx: &App) -> Div {
        let fill = fraction.unwrap_or(1.0);
        div().h(px(3.)).w_full().bg(theme::border(cx)).child(
            div()
                .h_full()
                .w(relative(fill.clamp(0.0, 1.0)))
                .bg(theme::accent(cx)),
        )
    }

    fn stat(&self, value: String, label: &'static str, cx: &App) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(rems(0.125))
            .min_w(rems(8.))
            .p(rems(0.5))
            .border_1()
            .border_color(theme::border(cx))
            .child(
                div()
                    .text_size(rems(0.9375))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(value),
            )
            .child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(label),
            )
    }

    fn fact_line(&self, name: &'static str, value: String, cx: &App) -> Div {
        div()
            .flex()
            .items_start()
            .gap(rems(0.625))
            .text_size(rems(0.75))
            .child(
                div()
                    .w(rems(5.))
                    .text_color(theme::secondary(cx))
                    .child(name),
            )
            .child(div().flex_1().min_w_0().child(value))
    }

    fn pill(&self, text: &'static str, colour: gpui_kit::Hsla, cx: &App) -> Div {
        div()
            .px(rems(0.375))
            .py(rems(0.0625))
            .border_1()
            .border_color(theme::border(cx))
            .text_size(rems(0.625))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(colour)
            .child(text)
    }

    fn bind_rule_switch(
        &self,
        id: &'static str,
        label: &'static str,
        checked: bool,
        key: &'static str,
        saving: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let me = cx.entity();
        switch(id, label, checked, cx)
            .disabled(saving == Some(key))
            .on_change(move |next, _, _, cx| {
                me.update(cx, |this, cx| this.save_rule(key, next, cx));
            })
    }

    fn bind_enrich_force(&self, force: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let me = cx.entity();
        switch("enrich-force", "Tout recommencer", force, cx).on_change(move |next, _, _, cx| {
            me.update(cx, |this, cx| {
                this.enrich_force = next;
                cx.notify();
            });
        })
    }

    fn bind_credits_force(&self, force: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let me = cx.entity();
        switch("credits-force", "Tout recommencer", force, cx).on_change(move |next, _, _, cx| {
            me.update(cx, |this, cx| {
                this.credits_force = next;
                cx.notify();
            });
        })
    }

    fn bind_transcribe_switch(
        &self,
        id: &'static str,
        label: &'static str,
        checked: bool,
        key: &'static str,
        busy: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let me = cx.entity();
        switch(id, label, checked, cx)
            .disabled(busy == Some(key))
            .on_change(move |next, _, _, cx| {
                me.update(cx, |this, cx| {
                    this.save_transcribe(key, Value::Bool(next), cx);
                });
            })
    }

    fn failure_block(
        &self,
        slot: &'static str,
        job: &JobStatus,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        if job.failed == 0 {
            return None;
        }
        let open = self.open_failures.contains(&slot);
        let mut block = div()
            .flex()
            .flex_col()
            .gap(rems(0.25))
            .mt(rems(0.25))
            .p(rems(0.625))
            .border_1()
            .border_color(theme::danger(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.375))
                    .text_size(rems(0.75))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::danger(cx))
                    .child(
                        icon(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(rems(0.875)),
                    )
                    .child(if job.failure_reasons.len() > 1 {
                        format!(
                            "{} titre(s) en échec, {} causes",
                            job.failed,
                            job.failure_reasons.len()
                        )
                    } else {
                        format!("{} titre(s) en échec", job.failed)
                    }),
            );
        if open {
            let mut reasons: Vec<(&String, &u32)> = job.failure_reasons.iter().collect();
            reasons.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
            for (reason, count) in reasons {
                block = block.child(
                    div()
                        .flex()
                        .items_start()
                        .gap(rems(0.375))
                        .text_size(rems(0.6875))
                        .child(
                            div()
                                .text_color(theme::danger(cx))
                                .child(format!("{count}×")),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_color(theme::secondary(cx))
                                .child(reason.clone()),
                        ),
                );
            }
            for row in job.failures.iter().take(FAILURES_SHOWN) {
                let failure: Failure = serde_json::from_value(row.clone()).unwrap_or_default();
                let name = if failure.track.is_empty() {
                    failure.id.clone()
                } else {
                    failure.track.clone()
                };
                block = block.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.0625))
                        .text_size(rems(0.6875))
                        .child(div().child(name))
                        .child(
                            div()
                                .text_color(theme::secondary(cx))
                                .font_family(theme::mono(cx))
                                .child(failure.reason),
                        ),
                );
            }
            let hidden = job.failed as usize - job.failures.len().min(FAILURES_SHOWN);
            if hidden > 0 {
                block = block.child(self.hint(
                    format!("… et {hidden} autre(s) non détaillé(s) (le serveur en garde un échantillon)."),
                    cx,
                ));
            }
            let copy_job = job.clone();
            block = block.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        button(
                            "failures-copy",
                            "Copier le détail",
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_failure_report(slot, &copy_job, cx);
                        })),
                    )
                    .child(
                        button("failures-close", "Masquer", ButtonVariant::Secondary, cx).on_click(
                            cx.listener(move |this, _, _, cx| this.toggle_failures(slot, cx)),
                        ),
                    ),
            );
        } else {
            let me = cx.entity();
            block = block.child(
                div()
                    .id(format!("failures-open-{slot}"))
                    .cursor_pointer()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .on_click(move |_, _, cx: &mut App| {
                        me.update(cx, |this, cx| this.toggle_failures(slot, cx));
                    })
                    .child("Voir les causes et les titres concernés"),
            );
        }
        Some(block)
    }
}

/// Which job a delayed poll belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Poll {
    Archive,
    Migrate,
    Enrich,
    Credits,
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut column = div()
            .w_full()
            .max_w(rems(64.))
            .flex()
            .flex_col()
            .gap(rems(0.75))
            .px(rems(1.5))
            .py(rems(1.25))
            .child(self.render_header(cx))
            .child(self.render_archive(cx))
            .child(self.render_migrate(cx))
            .child(self.render_enrich(cx))
            .child(self.render_credits(cx))
            .child(self.render_rules(window, cx))
            .child(self.render_cleanup(window, cx))
            .child(self.render_storage(cx))
            .child(self.render_transcribe(window, cx))
            .child(self.render_quota(window, cx));

        // The one scroll container: a 660 px pane and a 1940 px one scroll the
        // same column, and the cards wrap instead of overflowing.
        column = column.child(div().h(rems(0.5)));

        if !self.focused {
            self.focused = true;
            window.focus(&self.root_focus, cx);
        }

        focus_scope("settings-server")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .track_focus(&self.root_focus)
            .on_key_down(
                cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    this.on_scroll_key(event, window, cx);
                }),
            )
            .bg(theme::shell(cx))
            .font_family(theme::font(cx))
            .text_color(theme::text(cx))
            .child(div().w_full().flex().justify_center().child(column))
    }
}

// ---------------------------------------------------------------------------
// Copy and payload plumbing.
// ---------------------------------------------------------------------------

fn archive_counts(archived: u32, unavailable: u32) -> String {
    if unavailable > 0 {
        format!("{archived} archivé(s) · {unavailable} indisponible(s)")
    } else {
        format!("{archived} archivé(s)")
    }
}

fn archive_done_sentence(archived: u32, unavailable: u32, failed: u32) -> String {
    let mut parts = vec![format!("{archived} archivé(s)")];
    if unavailable > 0 {
        parts.push(format!("{unavailable} indisponible(s)"));
    }
    if failed > 0 {
        parts.push(format!("{failed} échec(s)"));
    }
    format!("Archivage terminé : {}.", parts.join(", "))
}

/// The seven event rules, as the `&'static str` keys the switches bind.
fn event_key(event: &str) -> Option<&'static str> {
    Some(match event {
        "on_play_context" => "on_play_context",
        "on_fav_track" => "on_fav_track",
        "on_fav_album" => "on_fav_album",
        "on_fav_playlist" => "on_fav_playlist",
        "on_fav_artist" => "on_fav_artist",
        "on_playlist_add" => "on_playlist_add",
        "on_podcast" => "on_podcast",
        _ => return None,
    })
}

/// The stable element id of an event rule's switch.
fn event_id(key: &str) -> &'static str {
    match key {
        "on_play_context" => "rule-on-play-context",
        "on_fav_track" => "rule-on-fav-track",
        "on_fav_album" => "rule-on-fav-album",
        "on_fav_playlist" => "rule-on-fav-playlist",
        "on_fav_artist" => "rule-on-fav-artist",
        "on_playlist_add" => "rule-on-playlist-add",
        "on_podcast" => "rule-on-podcast",
        _ => "rule-unknown",
    }
}

/// The event rule's French sentence (`Settings.svelte`'s `EVENT_LABELS`).
fn event_label(event: &str) -> &'static str {
    match event {
        "on_play_context" => "Lire un titre archive tout son album / sa playlist",
        "on_fav_track" => "Titre mis en favori",
        "on_fav_album" => "Album mis en favori",
        "on_fav_playlist" => "Playlist mise en favori",
        "on_fav_artist" => "Artiste mis en favori",
        "on_playlist_add" => "Titre ajouté à une playlist",
        "on_podcast" => "Nouveau podcast : archivage automatique activé (réglable sur sa page)",
        _ => "Règle inconnue",
    }
}

fn scope_label(scope: &str) -> &'static str {
    match scope {
        "all" => "Toute la discographie",
        "releases" => "Les N sorties les plus récentes",
        "top" => "Les N titres les plus écoutés",
        _ => "Portée inconnue",
    }
}

fn order_label(order: &str) -> &'static str {
    match order {
        "oldest_play" => "Le plus anciennement écouté",
        "least_played" => "Le moins écouté",
        "largest" => "Le plus volumineux",
        "oldest" => "Le plus anciennement archivé",
        _ => "Ordre inconnu",
    }
}

fn worker_detail(worker: &WorkerRow) -> String {
    let mut parts = Vec::new();
    if let Some(revoked) = &worker.revoked {
        parts.push(format!("révoqué le {}", datetime_fr(revoked)));
    }
    match &worker.last_seen {
        Some(seen) => parts.push(format!("vu le {}", datetime_fr(seen))),
        None => parts.push("jamais connecté".to_owned()),
    }
    if let Some(lease) = &worker.lease {
        parts.push(format!(
            "épisode {} (essai {}, {} revendiqué le {})",
            lease.episode_id,
            lease.attempts.max(1),
            if lease.heartbeat_at.is_some() {
                "dernier battement"
            } else {
                "aucun battement"
            },
            datetime_fr(
                lease
                    .heartbeat_at
                    .as_deref()
                    .unwrap_or(lease.expires_at.as_deref().unwrap_or(""))
            )
        ));
    }
    if let Some(created) = &worker.created {
        parts.push(format!("créé le {}", date_fr(created)));
    }
    parts.join(" · ")
}

fn trim_float(value: f64) -> String {
    if (value.fract()).abs() < f64::EPSILON {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

fn fraction_u64(used: u64, total: u64) -> Option<f32> {
    if total == 0 {
        return None;
    }
    Some((used as f32 / total as f32).clamp(0.0, 1.0))
}

// -- rule set field access --------------------------------------------------

fn rule_value(rules: &ArchiveRuleSet, key: &str) -> Value {
    match key {
        "on_fav_track" => Value::Bool(rules.on_fav_track),
        "on_fav_album" => Value::Bool(rules.on_fav_album),
        "on_fav_artist" => Value::Bool(rules.on_fav_artist),
        "on_fav_playlist" => Value::Bool(rules.on_fav_playlist),
        "on_playlist_add" => Value::Bool(rules.on_playlist_add),
        "on_play_context" => Value::Bool(rules.on_play_context),
        "on_podcast" => Value::Bool(rules.on_podcast),
        "chapters" => Value::Bool(rules.chapters),
        "chapter_urls" => Value::Bool(rules.chapter_urls),
        "clean_on" => Value::Bool(rules.clean_on),
        "clean_keep_fav" => Value::Bool(rules.clean_keep_fav),
        "clean_keep_playlist" => Value::Bool(rules.clean_keep_playlist),
        "clean_keep_podcast" => Value::Bool(rules.clean_keep_podcast),
        "artist_scope" => Value::String(rules.artist_scope.clone()),
        "artist_limit" => Value::from(rules.artist_limit),
        "clean_order" => Value::String(rules.clean_order.clone()),
        "clean_min_free_gb" => Value::from(rules.clean_min_free_gb),
        "clean_stale_days" => Value::from(rules.clean_stale_days),
        _ => Value::Null,
    }
}

fn set_rule(rules: &mut ArchiveRuleSet, key: &str, value: Value) {
    if let Some(value) = rule_from_json(key, &value) {
        set_rule_json(rules, key, value);
    }
}

fn restore_rule(rules: &mut ArchiveRuleSet, key: &str, previous: Value) {
    set_rule(rules, key, previous);
}

#[derive(Clone)]
enum JsonRule {
    Bool(bool),
    Text(String),
    Int(u32),
    Float(f64),
}

fn rule_from_json(key: &str, value: &Value) -> Option<JsonRule> {
    match key {
        "artist_scope" | "clean_order" => {
            value.as_str().map(|text| JsonRule::Text(text.to_owned()))
        }
        "artist_limit" | "clean_stale_days" => value
            .as_u64()
            .map(|number| JsonRule::Int(number.min(u32::MAX as u64) as u32)),
        "clean_min_free_gb" => value.as_f64().map(JsonRule::Float),
        _ => value.as_bool().map(JsonRule::Bool),
    }
}

fn set_rule_json(rules: &mut ArchiveRuleSet, key: &str, value: JsonRule) {
    match (key, value) {
        ("on_fav_track", JsonRule::Bool(v)) => rules.on_fav_track = v,
        ("on_fav_album", JsonRule::Bool(v)) => rules.on_fav_album = v,
        ("on_fav_artist", JsonRule::Bool(v)) => rules.on_fav_artist = v,
        ("on_fav_playlist", JsonRule::Bool(v)) => rules.on_fav_playlist = v,
        ("on_playlist_add", JsonRule::Bool(v)) => rules.on_playlist_add = v,
        ("on_play_context", JsonRule::Bool(v)) => rules.on_play_context = v,
        ("on_podcast", JsonRule::Bool(v)) => rules.on_podcast = v,
        ("chapters", JsonRule::Bool(v)) => rules.chapters = v,
        ("chapter_urls", JsonRule::Bool(v)) => rules.chapter_urls = v,
        ("clean_on", JsonRule::Bool(v)) => rules.clean_on = v,
        ("clean_keep_fav", JsonRule::Bool(v)) => rules.clean_keep_fav = v,
        ("clean_keep_playlist", JsonRule::Bool(v)) => rules.clean_keep_playlist = v,
        ("clean_keep_podcast", JsonRule::Bool(v)) => rules.clean_keep_podcast = v,
        ("artist_scope", JsonRule::Text(v)) => rules.artist_scope = v,
        ("artist_limit", JsonRule::Int(v)) => rules.artist_limit = v,
        ("clean_order", JsonRule::Text(v)) => rules.clean_order = v,
        ("clean_min_free_gb", JsonRule::Float(v)) => rules.clean_min_free_gb = v,
        ("clean_stale_days", JsonRule::Int(v)) => rules.clean_stale_days = v,
        _ => {}
    }
}

// -- transcribe field access ------------------------------------------------

fn transcribe_value(settings: &TranscribeSettings, key: &str) -> Value {
    match key {
        "enabled" => Value::Bool(settings.enabled),
        "language" => Value::String(settings.language.clone()),
        "max_minutes" => Value::from(settings.max_minutes),
        "daily_minutes" => Value::from(settings.daily_minutes),
        "search_segments" => Value::from(settings.search_segments),
        _ => Value::Null,
    }
}

fn apply_transcribe(settings: &mut TranscribeSettings, key: &str, value: &Value) {
    match key {
        "enabled" => {
            if let Some(next) = value.as_bool() {
                settings.enabled = next;
            }
        }
        "language" => {
            if let Some(next) = value.as_str() {
                settings.language = next.to_owned();
            }
        }
        "max_minutes" => {
            if let Some(next) = value.as_u64() {
                settings.max_minutes = next as u32;
            }
        }
        "daily_minutes" => {
            if let Some(next) = value.as_u64() {
                settings.daily_minutes = next as u32;
            }
        }
        "search_segments" => {
            if let Some(next) = value.as_u64() {
                settings.search_segments = next;
            }
        }
        _ => {}
    }
}

fn restore_transcribe(settings: &mut TranscribeSettings, key: &str, previous: Value) {
    apply_transcribe(settings, key, &previous);
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_are_the_pages_french_units() {
        assert_eq!(bytes(0), "0 o");
        assert_eq!(bytes(999), "999 o");
        assert_eq!(bytes(1024), "1.0 Ko");
        assert_eq!(bytes(1536), "1.5 Ko");
        assert_eq!(bytes(10 * 1024), "10 Ko");
        assert_eq!(bytes(3 * 1024 * 1024 * 1024), "3.0 Go");
        assert_eq!(bytes(1536 * 1024 * 1024), "1.5 Go");
    }

    #[test]
    fn a_gauge_never_divides_by_zero() {
        assert_eq!(fraction(0, 0), None);
        assert_eq!(fraction(0, 10), Some(0.0));
        assert_eq!(fraction(10, 10), Some(1.0));
        assert_eq!(fraction(20, 10), Some(1.0), "clamped");
    }

    #[test]
    fn the_language_labels_are_the_spas() {
        assert_eq!(language_label("auto"), "Détection automatique");
        assert_eq!(language_label("fr"), "Français");
        assert_eq!(language_label("de"), "Allemand");
        assert_eq!(language_label("xx"), "XX");
        assert_eq!(
            transcribe_reason("engine_missing"),
            "Le moteur de transcription n'est pas installé sur le serveur."
        );
    }

    #[test]
    fn dates_are_printed_as_the_server_wrote_them() {
        assert_eq!(date_fr("2026-11-16T20:15:00"), "16 nov. 2026");
        assert_eq!(datetime_fr("2026-11-16T20:15:00"), "16 nov. 2026 à 20:15");
        assert_eq!(
            datetime_fr("2026-01-02T03:04:05.123456"),
            "2 janv. 2026 à 03:04"
        );
        assert_eq!(date_fr("nonsense"), "nonsense");
        assert_eq!(date_fr("2026-13-01T00:00:00"), "2026-13-01T00:00:00");
        // Hostile input: a multi-byte character where a digit belongs must not
        // panic on a byte slice.
        assert_eq!(date_fr("éééé-11-16"), "éééé-11-16");
        assert_eq!(datetime_fr("2026-11-16Téé:15:00"), "16 nov. 2026");
    }

    #[test]
    fn a_worker_id_is_validated_before_it_becomes_a_path() {
        assert!(is_uuid("3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f"));
        assert!(!is_uuid("3f7c1d2e9a4b5c6d8e0f1a2b3c4d5e6f"));
        assert!(!is_uuid("../../etc/passwd"));
        assert!(!is_uuid("3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6g"));
        assert!(!is_uuid(""));
    }

    #[test]
    fn the_enrich_payload_keeps_the_job_counters_no_committed_type_carries() {
        let live: EnrichLive = serde_json::from_str(
            r#"{"enabled":true,"running":true,"acoustid":true,"started":"2026-11-16T20:00:00",
                "done":12,"failed":2,"total":40,"scanned":14,
                "failure_reasons":{"no match":2},
                "failures":[{"id":"1","track":"a.flac","reason":"no match"}],
                "coverage":{"tried":14,"untried":26,"found":{"isrc":9},"status":{}}}"#,
        )
        .expect("enrich payload");
        assert!(live.running);
        assert_eq!(live.job.done, 12);
        assert_eq!(live.job.failed, 2);
        assert_eq!(live.job.failure_reasons.get("no match"), Some(&2));
        assert_eq!(live.coverage.found.get("isrc"), Some(&9));
    }

    #[test]
    fn a_worker_row_parses_with_and_without_a_lease() {
        let list: WorkerList = serde_json::from_str(
            r#"{"workers":[
                {"id":"3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f","name":"banc-t3","kind":"client",
                 "created":"2026-11-16T10:00:00","enabled":true,"last_seen":"2026-11-16T20:00:00",
                 "revoked":null,
                 "lease":{"episode_id":"31254005","expires_at":"2026-11-16T20:05:00",
                          "heartbeat_at":"2026-11-16T20:00:30","attempts":2}},
                {"id":"4f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f","name":"serveur","kind":"server",
                 "created":"2026-11-15T10:00:00","enabled":false,"last_seen":null,
                 "revoked":"2026-11-16T09:00:00","lease":null}]}"#,
        )
        .expect("worker list");
        assert_eq!(list.workers.len(), 2);
        assert_eq!(
            list.workers[0].lease.as_ref().map(|lease| lease.attempts),
            Some(2)
        );
        assert!(list.workers[1].lease.is_none());
        assert!(list.workers[1].revoked.is_some());
    }

    #[test]
    fn an_issued_worker_keeps_the_token_shown_once() {
        let issued: IssuedWorker = serde_json::from_str(
            r#"{"worker":{"id":"3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f","name":"ce-pc",
                "kind":"client","created":"2026-11-16T20:00:00"},"token":"uuid.secret"}"#,
        )
        .expect("issued worker");
        assert_eq!(issued.worker.name, "ce-pc");
        assert_eq!(issued.token, "uuid.secret");
    }

    #[test]
    fn every_rule_key_round_trips_through_its_json() {
        let mut rules = ArchiveRuleSet::default();
        for key in [
            "on_fav_track",
            "on_fav_album",
            "on_fav_artist",
            "on_fav_playlist",
            "on_playlist_add",
            "on_play_context",
            "on_podcast",
            "chapters",
            "chapter_urls",
            "clean_on",
            "clean_keep_fav",
            "clean_keep_playlist",
            "clean_keep_podcast",
        ] {
            let before = rule_value(&rules, key);
            set_rule(&mut rules, key, Value::Bool(true));
            assert_eq!(
                rule_value(&rules, key),
                Value::Bool(true),
                "{key} did not move"
            );
            restore_rule(&mut rules, key, before);
        }
        set_rule(&mut rules, "artist_scope", Value::from("top"));
        assert_eq!(rules.artist_scope, "top");
        set_rule(&mut rules, "artist_limit", Value::from(42));
        assert_eq!(rules.artist_limit, 42);
        set_rule(&mut rules, "clean_order", Value::from("largest"));
        assert_eq!(rules.clean_order, "largest");
        set_rule(&mut rules, "clean_min_free_gb", serde_json::json!(2.5));
        assert_eq!(rules.clean_min_free_gb, 2.5);
        set_rule(&mut rules, "clean_stale_days", Value::from(90));
        assert_eq!(rules.clean_stale_days, 90);
    }

    #[test]
    fn the_failure_report_names_every_cause_and_row() {
        let mut job = JobStatus {
            failed: 3,
            ..JobStatus::default()
        };
        job.failure_reasons.insert("no match".to_owned(), 2);
        job.failure_reasons.insert("timeout".to_owned(), 1);
        job.failures = vec![
            serde_json::json!({"id":"1","track":"a.flac","reason":"no match"}),
            serde_json::json!({"id":"2","track":"b.flac","reason":"timeout"}),
        ];
        let report = failure_report("faits", &job);
        assert!(report.starts_with("Neezer — faits : 3 en échec"));
        assert!(report.contains("2x no match"));
        assert!(report.contains("a.flac — no match"));
        assert!(report.contains("b.flac — timeout"));
    }

    #[test]
    fn the_transcribe_optimistic_edit_rolls_back() {
        let mut settings = TranscribeSettings {
            language: "fr".into(),
            max_minutes: 120,
            ..TranscribeSettings::default()
        };
        let previous = transcribe_value(&settings, "max_minutes");
        apply_transcribe(&mut settings, "max_minutes", &Value::from(360));
        assert_eq!(settings.max_minutes, 360);
        restore_transcribe(&mut settings, "max_minutes", previous);
        assert_eq!(settings.max_minutes, 120);
    }
}

// ---------------------------------------------------------------------------
// The screen against a real socket: request counts, polling and the token.
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod stub {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use gpui_kit::{AppContext as _, Entity};
    use serde_json::json;

    use crate::state::{AppConfig, AppState, Auth, event_channel};

    /// A throwaway HTTP server: one request line in, one body out, and every
    /// request remembered so a test can count them.
    pub(crate) struct Stub {
        pub base: String,
        seen: Arc<Mutex<Vec<String>>>,
        bodies: Arc<Mutex<HashMap<String, String>>>,
        bytes: Arc<Mutex<usize>>,
    }

    impl Stub {
        pub(crate) fn start(routes: Vec<(&'static str, String)>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let address = listener.local_addr().expect("local address");
            let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let bytes: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
            let bodies: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(
                routes
                    .into_iter()
                    .map(|(needle, body)| (needle.to_owned(), body))
                    .collect(),
            ));
            let log = Arc::clone(&seen);
            let table = Arc::clone(&bodies);
            let counted = Arc::clone(&bytes);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let mut buffer = [0u8; 8192];
                    let read = stream.read(&mut buffer).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                    let line = request.lines().next().unwrap_or_default().to_owned();
                    log.lock().unwrap().push(line.clone());
                    let body = table
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|(needle, _)| line.starts_with(needle.as_str()))
                        .map(|(_, body)| body.clone())
                        .unwrap_or_else(|| "{}".to_owned());
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    *counted.lock().unwrap() += body.len();
                    let _ = stream.write_all(response.as_bytes());
                }
            });
            Self {
                base: format!("http://{address}"),
                seen,
                bodies,
                bytes,
            }
        }

        /// Change what a route answers from now on (a job that finishes).
        pub(crate) fn set(&self, needle: &str, body: impl Into<String>) {
            self.bodies
                .lock()
                .unwrap()
                .insert(needle.to_owned(), body.into());
        }

        /// Bytes of JSON the screen pulled (the response bodies).
        pub(crate) fn bytes(&self) -> usize {
            *self.bytes.lock().unwrap()
        }

        pub(crate) fn requests(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }

        pub(crate) fn total(&self) -> usize {
            self.seen.lock().unwrap().len()
        }

        pub(crate) fn count(&self, needle: &str) -> usize {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .filter(|line| line.starts_with(needle))
                .count()
        }

        pub(crate) fn bodies(&self) -> Vec<String> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .filter(|line| {
                    line.starts_with("POST ")
                        || line.starts_with("PATCH ")
                        || line.starts_with("DELETE ")
                })
                .cloned()
                .collect()
        }
    }

    /// A logged-in app pointed at the stub, with temp directories under `name`.
    pub(crate) fn logged_in(
        cx: &mut gpui::TestAppContext,
        stub: &Stub,
        name: &str,
    ) -> (
        Entity<AppState>,
        tokio::runtime::Runtime,
        std::path::PathBuf,
    ) {
        cx.update(gpui_omarchy::init);
        let dir = std::env::temp_dir().join(format!(
            "neezer-d14b-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let (tx, _rx) = event_channel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let config = AppConfig {
            server: Some(stub.base.clone()),
            start_audio: false,
            start_mpris: false,
            ..AppConfig::for_test(&dir)
        };
        let state = {
            let handle = runtime.handle().clone();
            cx.update(|cx| cx.new(move |_| AppState::new(config, handle, tx, None)))
        };
        let me: nz_model::Me = serde_json::from_str(
            r#"{"user":{"id":"u1","name":"bench","admin":true,"access":{},"sources":[]}}"#,
        )
        .expect("me");
        cx.update(|cx| {
            state.update(cx, |state, _| {
                state.auth = Auth::LoggedIn { me: Box::new(me) };
            });
        });
        (state, runtime, dir)
    }

    /// Wait, bounded, for a condition on the screen, driving the current-thread
    /// runtime on this same thread (the D7 rule: a tokio worker must not wake a
    /// foreground task in the deterministic scheduler).
    pub(crate) fn wait_until<T: 'static>(
        runtime: &tokio::runtime::Runtime,
        cx: &mut gpui::VisualTestContext,
        view: &Entity<T>,
        what: &str,
        mut done: impl FnMut(&T) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            runtime.block_on(async {
                tokio::time::sleep(Duration::from_millis(1)).await;
            });
            cx.run_until_parked();
            if cx.update(|_, cx| done(view.read(cx))) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
        }
    }

    /// Wait until a route has been called at least `count` times.
    pub(crate) fn wait_for_calls(
        runtime: &tokio::runtime::Runtime,
        cx: &mut gpui::VisualTestContext,
        stub: &Stub,
        needle: &str,
        count: usize,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            runtime.block_on(async {
                tokio::time::sleep(Duration::from_millis(1)).await;
            });
            cx.run_until_parked();
            if stub.count(needle) >= count {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {count} calls to {needle}: {:?}",
                stub.requests()
            );
        }
    }

    /// The Serveur tab's ten reads, with the server's own bodies.
    pub(crate) fn server_routes() -> Vec<(&'static str, String)> {
        vec![
            (
                "GET /api/archive/status",
                json!({"running": false, "scope": null, "total": 4, "done": 4, "archived": 4,
                       "failed": 0, "unavailable": 0, "error": null})
                .to_string(),
            ),
            ("GET /api/archive/rules", rules_body()),
            (
                "GET /api/archive/migrate",
                json!({"name": "disk-migrate", "running": false, "done": 3, "failed": 0,
                       "error": null, "counts": {"tags": 2, "sidecar": 1, "bpm": 0,
                       "bpm_unreachable": 0, "unchanged": 0}, "failures": [],
                       "failure_reasons": {}})
                .to_string(),
            ),
            (
                "GET /api/enrich/backfill",
                json!({"enabled": true, "running": false, "acoustid": false,
                       "started": null, "done": 5, "failed": 0, "total": 5, "scanned": 5,
                       "failure_reasons": {}, "failures": [],
                       "coverage": {"tried": 5, "untried": 0, "found": {"isrc": 5}, "status": {}}})
                .to_string(),
            ),
            (
                "GET /api/credits/backfill",
                json!({"running": false, "done": 5, "failed": 0, "total": 5, "name": "credits",
                       "tracks": 5, "completed": true, "counts": {"deezer": 4, "local": 1},
                       "failures": [], "failure_reasons": {}})
                .to_string(),
            ),
            (
                "GET /api/archive/cleanup/preview",
                json!({"enabled": true, "running": false, "eligible": 2, "would_free": 0,
                       "free_bytes": 1099511627776u64, "needed_bytes": 0,
                       "tracks": [{"id": "1", "title": "Vieux titre", "artist": "Kaya",
                                   "bytes": 34000000u64, "play_count": 1, "last_play": null}]})
                .to_string(),
            ),
            (
                "GET /api/storage",
                json!({"archive_dir": "/srv/neezer/archive", "archive_bytes": 42940000000u64,
                       "cache_bytes": 1024u64, "cache_limit": 0u64, "transcode_bytes": 2048u64,
                       "transcode_limit": 0u64, "disk_free": 1099511627776u64,
                       "disk_total": 2199023255552u64, "tracks_archived": 4000, "tracks_total": 8000})
                .to_string(),
            ),
            (
                "GET /api/settings",
                json!({"upload_quota_gb": 0, "deezer": {}}).to_string(),
            ),
            (
                "GET /api/transcribe/settings",
                json!({"available": true, "enabled": true, "runner": "inline", "reason": null,
                       "language": "fr", "languages": ["auto", "fr", "en"], "daily_minutes": 0,
                       "max_minutes": 120, "threads": 4, "search_segments": 100000,
                       "queue": {"queued": 0, "leased": 0, "running": 0, "ready": 2, "error": 0,
                                 "stopped": 0, "budget_limit_ms": 0, "budget_used_ms": 0},
                       "engine": {"name": "whisper.cpp", "ok": true, "version": "1.8.3"},
                       "model": {"name": "ggml-small.bin", "ok": true, "size_mb": 466}})
                .to_string(),
            ),
            ("GET /api/transcribe/workers", json!({"workers": []}).to_string()),
        ]
    }

    pub(crate) fn rules_body() -> String {
        json!({
            "rules": {
                "on_fav_track": true, "on_fav_album": true, "on_fav_artist": false,
                "on_fav_playlist": true, "on_playlist_add": false, "on_play_context": false,
                "on_podcast": true, "artist_scope": "releases", "artist_limit": 3,
                "chapters": true, "chapter_urls": false, "clean_on": true,
                "clean_stale_days": 90, "clean_min_free_gb": 5.0, "clean_order": "oldest_play",
                "clean_keep_fav": true, "clean_keep_playlist": true, "clean_keep_podcast": true
            },
            "defaults": {},
            "events": ["on_play_context", "on_fav_track", "on_fav_album", "on_fav_playlist",
                       "on_fav_artist", "on_playlist_add", "on_podcast"],
            "artist_scopes": ["all", "releases", "top"],
            "cleanup_orders": ["oldest_play", "least_played", "largest", "oldest"],
            "archive_library": true
        })
        .to_string()
    }
}

#[cfg(test)]
mod screen_tests {
    use super::stub::{self, Stub};
    use super::*;

    /// The acceptance's shape: the Serveur tab is TEN reads, one per card, and a
    /// finished job is not polled again. Measured against a real socket.
    #[gpui::test]
    async fn the_server_screen_reads_ten_cards_and_stops_polling(cx: &mut gpui::TestAppContext) {
        let stub = Stub::start(stub::server_routes());
        let (state, runtime, dir) = stub::logged_in(cx, &stub, "server-cards");
        let (view, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| super::Screen::new(SettingsSection::Server, state, cx)
        });

        stub::wait_until(&runtime, cx, &view, "the ten cards", |screen| {
            screen.archive.is_some()
                && screen.rules.is_some()
                && screen.migrate.is_some()
                && screen.enrich.is_some()
                && screen.credits.is_some()
                && screen.cleanup.is_some()
                && screen.storage.is_some()
                && screen.settings.is_some()
                && screen.transcribe.is_some()
                && screen.workers.is_some()
        });

        // Ten routes, ten requests — the SPA's Serveur tab makes eleven or
        // twelve. Nothing is fetched twice on open.
        assert_eq!(stub.total(), 10, "one read per card: {:?}", stub.requests());
        // Rule 3's other half: the bytes. Printed for the PR's table.
        println!(
            "Serveur: {} requêtes, {} octets de JSON",
            stub.total(),
            stub.bytes()
        );
        assert!(
            stub.bytes() < 16 * 1024,
            "the ten cards are a few kilobytes, not a download: {} B",
            stub.bytes()
        );
        assert_eq!(stub.count("GET /api/archive/status"), 1);

        // Nothing is running, so no poll timer fires: the count stays put.
        for _ in 0..25 {
            runtime.block_on(async {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            });
            cx.run_until_parked();
        }
        assert_eq!(stub.total(), 10, "an idle screen polls nothing");

        cx.update(|_, cx| {
            let screen = view.read(cx);
            assert_eq!(screen.storage.as_ref().unwrap().tracks_archived, 4000);
            assert_eq!(screen.rules.as_ref().unwrap().events.len(), 7);
            assert!(screen.workers.as_ref().unwrap().is_empty());
            assert!(!screen.archive_poll);
        });

        std::fs::remove_dir_all(dir).ok();
        drop(runtime);
    }

    /// Rule 2: a running job is polled until it ends, and the end says what it
    /// did. The poll cadence is the SPA's 1.5 s.
    #[gpui::test]
    async fn a_running_archive_is_polled_then_announced(cx: &mut gpui::TestAppContext) {
        let mut routes = stub::server_routes();
        for (needle, body) in routes.iter_mut() {
            if *needle == "GET /api/archive/status" {
                *body = serde_json::json!({
                    "running": true, "scope": "all", "total": 3, "done": 1,
                    "archived": 1, "failed": 0, "unavailable": 0, "error": null
                })
                .to_string();
            }
        }
        let stub = Stub::start(routes);
        let (state, runtime, dir) = stub::logged_in(cx, &stub, "archive-poll");
        let (view, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| super::Screen::new(SettingsSection::Server, state, cx)
        });

        stub::wait_until(&runtime, cx, &view, "the running archive", |screen| {
            screen.archive.as_ref().is_some_and(|job| job.running)
        });
        assert_eq!(stub.count("GET /api/archive/status"), 1);

        // The job finishes on the server; the next poll is the one that sees it.
        stub.set(
            "GET /api/archive/status",
            serde_json::json!({
                "running": false, "scope": "all", "total": 3, "done": 3,
                "archived": 2, "failed": 0, "unavailable": 1, "error": null
            })
            .to_string(),
        );
        stub::wait_for_calls(&runtime, cx, &stub, "GET /api/archive/status", 2);
        let calls = stub.count("GET /api/archive/status");
        assert!(
            calls <= 3,
            "one poll per 1.5 s, never a storm: {calls} calls"
        );

        stub::wait_until(&runtime, cx, &view, "the closing toast", |_| true);
        let text = cx.update(|_, cx| {
            state
                .read(cx)
                .toasts
                .items()
                .iter()
                .map(|toast| toast.message.to_string())
                .collect::<Vec<_>>()
        });
        assert!(
            text.iter()
                .any(|message| message.contains("Archivage terminé")
                    && message.contains("2 archivé")
                    && message.contains("1 indisponible")),
            "the end names what it did: {text:?}"
        );
        assert_eq!(
            stub.count("GET /api/archive/status"),
            calls,
            "polling stopped"
        );

        std::fs::remove_dir_all(dir).ok();
        drop(runtime);
    }

    /// The worker list: creating one shows the token ONCE, revoking one sends
    /// the DELETE for that worker's own id, and a hostile id never becomes a path.
    #[gpui::test]
    async fn a_worker_token_is_shown_once_and_revoked(cx: &mut gpui::TestAppContext) {
        let mut routes = stub::server_routes();
        for (needle, body) in routes.iter_mut() {
            if *needle == "GET /api/transcribe/workers" {
                *body = serde_json::json!({"workers": [{
                    "id": "3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f", "name": "banc-t3",
                    "kind": "client", "created": "2026-11-16T10:00:00", "enabled": true,
                    "last_seen": "2026-11-16T20:00:00", "revoked": null, "lease": null
                }]})
                .to_string();
            }
        }
        routes.push((
            "POST /api/transcribe/workers",
            serde_json::json!({
                "worker": {"id": "4f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f", "name": "ce-pc",
                           "kind": "client", "created": "2026-11-16T20:00:00"},
                "token": "4f7c1d2e.token"
            })
            .to_string(),
        ));
        routes.push((
            "DELETE /api/transcribe/workers/",
            serde_json::json!({"id": "3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f",
                               "revoked": true, "released": 1})
            .to_string(),
        ));
        let stub = Stub::start(routes);
        let (state, runtime, dir) = stub::logged_in(cx, &stub, "workers");
        let (view, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| super::Screen::new(SettingsSection::Server, state, cx)
        });

        stub::wait_until(&runtime, cx, &view, "the workers", |screen| {
            screen.workers.is_some() && screen.transcribe.is_some()
        });

        // The form's name field is created on the first paint; write it.
        cx.update(|window, cx| {
            view.update(cx, |screen, cx| {
                let name = screen.field(
                    Field::WorkerName,
                    String::new(),
                    "Nom de la machine",
                    window,
                    cx,
                );
                name.update(cx, |field, cx| field.set_value("ce-pc", window, cx));
            });
        });
        cx.update(|_, cx| {
            view.update(cx, |screen, cx| screen.issue_worker(cx));
        });
        stub::wait_until(&runtime, cx, &view, "the issued token", |screen| {
            screen.issued.is_some()
        });
        let issued = cx.update(|_, cx| view.read(cx).issued.clone().expect("issued"));
        assert_eq!(issued.token, "4f7c1d2e.token");
        assert_eq!(issued.worker.name, "ce-pc");
        // A second read cannot bring the token back: the screen holds it once,
        // and the server never answers it again (that is the route's contract).
        assert!(
            stub.bodies()
                .iter()
                .any(|line| line.starts_with("POST /api/transcribe/workers"))
        );

        // Revoke the known worker: the UUID goes into the path untouched.
        cx.update(|_, cx| {
            view.update(cx, |screen, cx| {
                screen.revoke_worker("3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f", cx);
            });
        });
        stub::wait_for_calls(
            &runtime,
            cx,
            &stub,
            "DELETE /api/transcribe/workers/3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f",
            1,
        );
        // An id that is not a UUID never becomes a route.
        let before = stub.total();
        cx.update(|_, cx| {
            view.update(cx, |screen, cx| {
                screen.revoke_worker("../../admin/users", cx);
            });
        });
        assert_eq!(stub.total(), before, "a hostile id sends nothing");

        std::fs::remove_dir_all(dir).ok();
        drop(runtime);
    }
}
