//! Bienvenue: the first-run flow, in two steps.
//!
//! `routes/Onboarding.svelte`, `components/OnboardingArtistCard.svelte`,
//! `components/OnboardingAccount.svelte` and `lib/onboarding.js` are the
//! reference:
//!
//! - **A like is optimistic and one request.** The tile flips in the frame, the
//!   `POST /api/onboarding/artists/like` follows, and a failure rolls the tile
//!   back with its cause (rule 3). Offline, a choice waits in a queue that is
//!   flushed in order when the link returns — « Continuer » refuses while any of
//!   it is unsent, because a finished onboarding that lost the choices is worse
//!   than one that waited.
//! - **The feed is paged, never fetched whole**: 36 tiles, then one GET per
//!   « Voir plus d'artistes » by the server's cursor (`after`).
//! - **The search asks one term 350 ms after the last keystroke**, and its
//!   failure keeps the typed text and offers « Réessayer ».
//! - **The account step is one card per way in** (Deezer's ARL, YouTube's
//!   session headers, a playlist link), each skippable, and the import is a JOB
//!   the server runs: the screen polls it, shows its progress and names a
//!   failure's cause (rule 2). Nothing a provider does here writes to the
//!   provider's account.
//!
//! Requests: `GET /api/onboarding` and one page of artists on open; one per
//! settled search term; one per like/unlike; `GET /account/options` when the
//! second step opens, then one `GET /import/<id>` per 1.5 s while a job runs.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, Window, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon, input, toggle};
use nz_api::ApiError;
use nz_model::{
    OnboardingAccountOptions, OnboardingArtist, OnboardingProvider, OnboardingState,
    OnboardingYouTube,
};

use crate::nav::Route;
use crate::state::AppState;
use crate::theme;
use crate::widgets::card;
use crate::widgets::cover::{self, Covers, Kind};
use crate::wiring::events::{self, Sender};

/// How many tiles a page carries (`PAGE_FIRST`/`PAGE_NEXT`).
pub const PAGE: u32 = 36;
/// How many hits a search asks for (`SEARCH_LIMIT`).
pub const SEARCH_LIMIT: u32 = 24;
/// How long after the last keystroke a term is asked (`Onboarding.svelte`).
pub const DEBOUNCE_MS: u64 = 350;
/// How often a running import is polled (`POLL_MS`).
pub const POLL_MS: u64 = 1_500;
/// How long a poll may run before it says so and stops (`POLL_MAX`).
pub const POLL_MAX_MS: u64 = 5 * 60 * 1_000;

/// The two steps the flow has (`OnboardingState#step`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Step {
    #[default]
    Artists,
    Account,
}

impl Step {
    fn number(self) -> u8 {
        match self {
            Step::Artists => 1,
            Step::Account => 2,
        }
    }
}

/// The text fields, created on a paint (`InputState::new` needs a window, and
/// `Screen::new` has none).
#[derive(Default)]
struct Fields {
    search: Option<Entity<InputState>>,
    arl: Option<Entity<InputState>>,
    headers: Option<Entity<InputState>>,
    link: Option<Entity<InputState>>,
}

enum Msg {
    State(Box<Result<OnboardingState, ApiError>>),
    Artists {
        first: bool,
        answer: Box<Result<nz_model::OnboardingArtists, ApiError>>,
    },
    Search(Box<Result<nz_model::OnboardingSearch, ApiError>>),
    Liked {
        id: String,
        liked: bool,
        answer: Box<Result<(), ApiError>>,
    },
    Done(Box<Result<serde_json::Value, ApiError>>),
    Options(Box<Result<OnboardingAccountOptions, ApiError>>),
    Linked(Box<Result<serde_json::Value, ApiError>>),
    Imported(Box<Result<serde_json::Value, ApiError>>),
    Job(Box<Result<serde_json::Value, ApiError>>),
    Finished(Box<Result<serde_json::Value, ApiError>>),
    Flushed,
    Cover(String, Option<PathBuf>),
}

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    step: Step,
    doc: Option<OnboardingState>,
    loading: bool,
    load_error: Option<ApiError>,
    /// The feed: the server's artists, paged by its cursor.
    artists: Vec<OnboardingArtist>,
    next: Option<String>,
    liked: HashSet<String>,
    liked_total: u32,
    /// A like is in flight for this id (the tile says so).
    pending: HashSet<String>,
    /// Choices made while the link was dead, in order (the SPA's queue).
    queue: Vec<(String, bool)>,
    /// The offline queue is being sent (one message per entry).
    flushing: bool,
    list_error: Option<ApiError>,
    more: bool,
    fields: Fields,
    query: String,
    /// The debounce's generation and whether a term is waiting for it.
    search_generation: u64,
    search_pending: bool,
    searching: bool,
    found: Option<Vec<OnboardingArtist>>,
    search_error: Option<ApiError>,
    /// The account step.
    options: Option<OnboardingAccountOptions>,
    options_error: Option<ApiError>,
    busy: Option<&'static str>,
    job: Option<serde_json::Value>,
    job_error: Option<SharedString>,
    polled: Option<Instant>,
    /// A job poll is armed (never two at once).
    polling: bool,
    /// The status line (« Continuer » explains what it is waiting for).
    status: Option<SharedString>,
    action_error: Option<SharedString>,
    finishing: bool,
    notice: Option<SharedString>,
    covers: Rc<RefCell<Covers>>,
    replies: Sender<Msg>,
    _subscriptions: Vec<Subscription>,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = events::channel::<Msg>();
        let mut screen = Self {
            route,
            state: state.clone(),
            step: Step::Artists,
            doc: None,
            loading: true,
            load_error: None,
            artists: Vec::new(),
            next: None,
            liked: HashSet::new(),
            liked_total: 0,
            pending: HashSet::new(),
            queue: Vec::new(),
            flushing: false,
            list_error: None,
            more: false,
            fields: Fields::default(),
            query: String::new(),
            search_generation: 0,
            search_pending: false,
            searching: false,
            found: None,
            search_error: None,
            options: None,
            options_error: None,
            busy: None,
            job: None,
            job_error: None,
            polled: None,
            polling: false,
            status: None,
            action_error: None,
            finishing: false,
            notice: None,
            covers: Rc::new(RefCell::new(Covers::new())),
            replies: tx,
            _subscriptions: vec![cx.observe(&state, |_, _, cx| cx.notify())],
        };
        screen.drain(rx, cx);
        screen.load(cx);
        screen
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

    /// Where the flow stands, and the first page of artists.
    fn load(&mut self, cx: &mut Context<Self>) {
        let (Some(api), runtime) = (
            self.state.read(cx).api.clone(),
            self.state.read(cx).runtime.clone(),
        ) else {
            return;
        };
        let tx = self.replies.clone();
        let api_for_state = api.clone();
        runtime.spawn(async move {
            tx.send(Msg::State(Box::new(api_for_state.onboarding_state().await)));
        });
        let tx = self.replies.clone();
        let runtime = self.state.read(cx).runtime.clone();
        let api = api.clone();
        runtime.spawn(async move {
            let answer = api.onboarding_artists(None, Some(PAGE), "").await;
            tx.send(Msg::Artists {
                first: true,
                answer: Box::new(answer),
            });
        });
    }

    fn next_page(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        if self.more {
            return;
        }
        self.more = true;
        let cursor = self.next.clone();
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.replies.clone();
        runtime.spawn(async move {
            let answer = api
                .onboarding_artists(cursor.as_deref(), Some(PAGE), "")
                .await;
            tx.send(Msg::Artists {
                first: false,
                answer: Box::new(answer),
            });
        });
    }

    fn search(&mut self, term: String, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        let term = term.trim().to_owned();
        if term.is_empty() {
            self.found = None;
            self.query.clear();
            self.search_error = None;
            cx.notify();
            return;
        }
        self.searching = true;
        self.search_error = None;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.replies.clone();
        runtime.spawn(async move {
            let answer = api.onboarding_search(&term, Some(SEARCH_LIMIT)).await;
            tx.send(Msg::Search(Box::new(answer)));
        });
        cx.notify();
    }

    /// The tiles on screen: the search's hits when there is a term, the feed
    /// otherwise.
    fn items(&self) -> &[OnboardingArtist] {
        match &self.found {
            Some(found) => found,
            None => &self.artists,
        }
    }

    /// Keep or take back an artist. Optimistic, and queued while offline.
    fn set_like(&mut self, id: String, liked: bool, cx: &mut Context<Self>) {
        if self.pending.contains(&id) {
            return;
        }
        if liked {
            self.liked.insert(id.clone());
            self.liked_total = self.liked_total.saturating_add(1);
        } else {
            self.liked.remove(&id);
            self.liked_total = self.liked_total.saturating_sub(1);
        }
        self.action_error = None;
        self.status = None;
        if !self.state.read(cx).online {
            self.queue.push((id.clone(), liked));
            self.status = Some(SharedString::from(
                "Hors ligne : vos choix seront enregistrés au retour du réseau.",
            ));
            cx.notify();
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.pending.insert(id.clone());
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.replies.clone();
        let seen: Vec<String> = self
            .items()
            .iter()
            .map(|artist| artist.id.clone())
            .filter(|id| !id.is_empty())
            .take(200)
            .collect();
        cx.notify();
        runtime.spawn(async move {
            let answer = if liked {
                api.onboarding_like(&id, Some(&seen)).await.map(|_| ())
            } else {
                api.onboarding_unlike(&id).await
            };
            tx.send(Msg::Liked {
                id,
                liked,
                answer: Box::new(answer),
            });
        });
    }

    /// The queued choices, one at a time, when the link is back.
    fn flush_queue(&mut self, cx: &mut Context<Self>) {
        if self.queue.is_empty() || !self.state.read(cx).online || self.flushing {
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.flushing = true;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.replies.clone();
        let queue = std::mem::take(&mut self.queue);
        runtime.spawn(async move {
            for (id, liked) in queue {
                let answer = if liked {
                    api.onboarding_like(&id, None).await.map(|_| ())
                } else {
                    api.onboarding_unlike(&id).await
                };
                tx.send(Msg::Liked {
                    id,
                    liked,
                    answer: Box::new(answer),
                });
            }
            tx.send(Msg::Flushed);
        });
    }

    /// « Continuer »: the artist step is done, then the account step (or home).
    fn finish_artists(&mut self, cx: &mut Context<Self>) {
        if self.finishing {
            return;
        }
        if !self.queue.is_empty() || !self.pending.is_empty() || self.flushing {
            self.status = Some(SharedString::from(
                "Vos derniers choix sont en cours d'enregistrement : un instant.",
            ));
            cx.notify();
            return;
        }
        if self.liked_total == 0 {
            self.status = Some(SharedString::from(
                "Choisissez au moins un artiste, ou passez cette étape.",
            ));
            cx.notify();
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.finishing = true;
        self.status = None;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.replies.clone();
        runtime.spawn(async move {
            tx.send(Msg::Done(Box::new(api.onboarding_artists_done(None).await)));
        });
        cx.notify();
    }

    /// "Passer pour l'instant": only when the feed could not be built
    /// (`canPassArtists`).
    fn pass_artists(&mut self, cx: &mut Context<Self>) {
        if self.finishing {
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.finishing = true;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.replies.clone();
        runtime.spawn(async move {
            let payload = serde_json::json!({ "skip": true });
            tx.send(Msg::Done(Box::new(
                api.onboarding_artists_done(Some(&payload)).await,
            )));
        });
        cx.notify();
    }

    /// Whether the feed is unavailable, which is the only case that offers
    /// « Passer pour l'instant » (`canPassArtists`).
    fn passed_available(&self) -> bool {
        self.doc
            .as_ref()
            .is_some_and(|doc| doc.feed.state == "unavailable" && !doc.feed.building)
    }

    /// The second step: what may still be linked.
    fn open_account(&mut self, cx: &mut Context<Self>) {
        self.step = Step::Account;
        self.status = None;
        self.load_options(cx);
    }

    fn load_options(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.replies.clone();
        runtime.spawn(async move {
            tx.send(Msg::Options(Box::new(
                api.onboarding_account_options().await,
            )));
        });
    }

    fn secret_of(&self, provider: &str, cx: &App) -> String {
        let field = match provider {
            "deezer" => self.fields.arl.as_ref(),
            _ => self.fields.headers.as_ref(),
        };
        field
            .map(|field| field.read(cx).value().trim().to_owned())
            .unwrap_or_default()
    }

    /// Link a provider: the ARL typed in the field, or the session headers.
    fn link_provider(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let value = self.secret_of(provider, cx);
        if value.is_empty() {
            self.action_error = Some(SharedString::from(match provider {
                "deezer" => "Collez votre ARL Deezer.",
                _ => "Collez les en-têtes d'une session YouTube Music.",
            }));
            cx.notify();
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.busy = Some(provider);
        self.action_error = None;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.replies.clone();
        runtime.spawn(async move {
            let payload = if provider == "deezer" {
                serde_json::json!({ "provider": "deezer", "arl": value })
            } else {
                serde_json::json!({ "provider": provider, "headers": value })
            };
            tx.send(Msg::Linked(Box::new(
                api.onboarding_account_link(&payload).await,
            )));
        });
        cx.notify();
    }

    /// Import a playlist link: the server analyses it as a job.
    fn import_link(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let url = self
            .fields
            .link
            .as_ref()
            .map(|field| field.read(cx).value().trim().to_owned())
            .unwrap_or_default();
        if url.is_empty() {
            self.action_error = Some(SharedString::from("Collez le lien d'une playlist."));
            cx.notify();
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.busy = Some("playlist");
        self.action_error = None;
        self.job_error = None;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.replies.clone();
        runtime.spawn(async move {
            let payload = serde_json::json!({ "url": url });
            tx.send(Msg::Imported(Box::new(
                api.onboarding_account_import(&payload).await,
            )));
        });
        cx.notify();
    }

    /// « Terminer » on the account step.
    fn finish_account(&mut self, cx: &mut Context<Self>) {
        if self.finishing {
            return;
        }
        if !self.state.read(cx).online {
            self.status = Some(SharedString::from(
                "Hors ligne : terminez quand la connexion revient.",
            ));
            cx.notify();
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        self.finishing = true;
        self.status = None;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.replies.clone();
        runtime.spawn(async move {
            tx.send(Msg::Finished(Box::new(api.onboarding_account_done().await)));
        });
        cx.notify();
    }

    /// Poll the running job: one request per [`POLL_MS`], capped at
    /// [`POLL_MAX_MS`], and the work itself keeps going on the server.
    fn poll_job(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.job_id() else {
            return;
        };
        if !running(&self.job) || self.polling {
            return;
        }
        self.polling = true;
        let started = *self.polled.get_or_insert_with(Instant::now);
        if started.elapsed().as_millis() as u64 > POLL_MAX_MS {
            self.job_error = Some(SharedString::from(
                "L'analyse dure plus longtemps que prévu : elle continue sur le serveur.",
            ));
            cx.notify();
            return;
        }
        let Some(api) = self.state.read(cx).api.clone() else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.replies.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(POLL_MS))
                .await;
            let _ = this.update(cx, |this, _| this.polling = false);
            runtime.spawn(async move {
                tx.send(Msg::Job(Box::new(api.import_status(&id).await)));
            });
        })
        .detach();
    }

    fn job_id(&self) -> Option<String> {
        self.job
            .as_ref()
            .and_then(|job| job.get("id"))
            .and_then(|id| id.as_str())
            .map(str::to_owned)
    }

    fn on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::State(answer) => {
                self.loading = false;
                match *answer {
                    Ok(doc) => {
                        self.liked_total = doc.liked;
                        if doc.step == "done" {
                            self.go_home(cx);
                            return;
                        }
                        if doc.step == "account" {
                            self.open_account(cx);
                        }
                        self.doc = Some(doc);
                    }
                    Err(error) => self.load_error = Some(error),
                }
            }
            Msg::Artists { first, answer } => {
                self.more = false;
                match *answer {
                    Ok(page) => {
                        self.next = page.next.clone();
                        if first && self.artists.is_empty() {
                            self.artists = page.items;
                        } else {
                            for artist in page.items {
                                if !self.artists.iter().any(|known| known.id == artist.id) {
                                    self.artists.push(artist);
                                }
                            }
                        }
                        self.list_error = None;
                        if let Some(doc) = self.doc.as_mut() {
                            doc.feed.state = page.state;
                            doc.feed.building = page.building;
                            doc.feed.error = page.error;
                        }
                    }
                    Err(error) => self.list_error = Some(error),
                }
            }
            Msg::Search(answer) => {
                self.searching = false;
                match *answer {
                    Ok(found) => {
                        self.query = found.query.clone();
                        self.found = Some(found.items);
                        self.search_error = None;
                    }
                    Err(error) => self.search_error = Some(error),
                }
            }
            Msg::Liked { id, liked, answer } => {
                self.pending.remove(&id);
                if let Err(error) = *answer {
                    // The optimistic flip was wrong: take it back and say why.
                    if liked {
                        self.liked.remove(&id);
                        self.liked_total = self.liked_total.saturating_sub(1);
                    } else {
                        self.liked.insert(id);
                        self.liked_total = self.liked_total.saturating_add(1);
                    }
                    self.action_error = Some(SharedString::from(format!(
                        "Votre choix n'a pas été enregistré : {}",
                        card::cause(&error)
                    )));
                }
            }
            Msg::Flushed => self.flushing = false,
            Msg::Done(answer) => {
                self.finishing = false;
                match *answer {
                    Ok(value) => {
                        let step = value
                            .get("step")
                            .and_then(|step| step.as_str())
                            .unwrap_or("account");
                        if step == "done" {
                            self.go_home(cx);
                            return;
                        }
                        self.open_account(cx);
                    }
                    Err(error) => {
                        self.status = Some(SharedString::from(format!(
                            "L'étape n'a pas pu être enregistrée : {}",
                            card::cause(&error)
                        )));
                    }
                }
            }
            Msg::Options(answer) => match *answer {
                Ok(options) => {
                    if options.step == "done" {
                        self.go_home(cx);
                        return;
                    }
                    let poll = running(&options.job);
                    self.options = Some(options);
                    self.options_error = None;
                    if poll {
                        self.schedule_options_poll(cx);
                    }
                }
                Err(error) => self.options_error = Some(error),
            },
            Msg::Linked(answer) => {
                self.busy = None;
                match *answer {
                    Ok(_) => {
                        self.notice = Some(SharedString::from(
                            "Compte relié. L'import de vos favoris a démarré sur le serveur.",
                        ));
                        self.load_options(cx);
                        self.schedule_options_poll(cx);
                    }
                    Err(error) => {
                        self.action_error = Some(SharedString::from(link_failure(&error)))
                    }
                }
            }
            Msg::Imported(answer) => {
                self.busy = None;
                match *answer {
                    Ok(value) => {
                        self.job = value.get("job").cloned().or(Some(value));
                        self.polled = Some(Instant::now());
                        self.poll_job(cx);
                    }
                    Err(error) => {
                        self.action_error = Some(SharedString::from(import_failure(&error)))
                    }
                }
            }
            Msg::Job(answer) => match *answer {
                Ok(job) => {
                    if !job.is_null() {
                        self.job = Some(job);
                    }
                    if running(&self.job) {
                        self.poll_job(cx);
                    }
                }
                Err(error) => {
                    if matches!(error, ApiError::Http { status: 404, .. }) {
                        self.job_error =
                            Some(SharedString::from("L'analyse a expiré : relancez-la."));
                        self.job = None;
                    } else {
                        self.job_error = Some(card::cause(&error));
                        self.poll_job(cx);
                    }
                }
            },
            Msg::Finished(answer) => {
                self.finishing = false;
                match *answer {
                    Ok(_) => self.go_home(cx),
                    Err(error) => {
                        self.status = Some(SharedString::from(format!(
                            "L'étape n'a pas pu être terminée : {}",
                            card::cause(&error)
                        )));
                    }
                }
            }
            Msg::Cover(url, file) => self.covers.borrow_mut().resolved(&url, file),
        }
        cx.notify();
    }

    fn schedule_options_poll(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(POLL_MS))
                .await;
            let _ = this.update(cx, |this, cx| this.load_options(cx));
        })
        .detach();
    }

    /// The flow is done: the shell's home is the next screen (`finishToHome`).
    fn go_home(&mut self, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, _| state.nav.reset(Route::Home));
    }

    fn want_covers(&mut self, cx: &mut Context<Self>) {
        let mut asked: Vec<String> = Vec::new();
        for artist in self.items() {
            if let Some(picture) = artist.picture.clone().or(artist.picture_small.clone()) {
                asked.push(picture);
            }
        }
        let snapshot = self.state.read(cx);
        let mut covers = self.covers.borrow_mut();
        for url in asked {
            let for_request = url.clone();
            let for_message = url.clone();
            let tx = self.replies.clone();
            covers.want(snapshot, &for_request, move |file| {
                tx.send(Msg::Cover(for_message, file))
            });
        }
    }
}

/// One import job is still running (`lib/onboarding.js#isImportRunning`).
pub fn running(job: &Option<serde_json::Value>) -> bool {
    let Some(state) = job
        .as_ref()
        .and_then(|job| job.get("state"))
        .and_then(|state| state.as_str())
    else {
        return false;
    };
    matches!(state, "reading" | "matching" | "saving")
}

/// A job's one-line progress and percentage (`importProgress`).
pub fn job_progress(job: &serde_json::Value) -> (String, Option<u32>) {
    let number = |key: &str| job.get(key).and_then(|value| value.as_u64()).unwrap_or(0) as u32;
    match job.get("state").and_then(|state| state.as_str()) {
        Some("reading") => ("Lecture de la playlist…".to_owned(), None),
        Some("matching") => {
            let total = number("total");
            let done = number("done");
            let label = if total > 0 {
                format!("Recherche des titres : {done} sur {total}")
            } else {
                "Recherche des titres…".to_owned()
            };
            let pct = (total > 0).then(|| (100 * done / total).min(100));
            (label, pct)
        }
        Some("saving") => ("Ajout à votre bibliothèque…".to_owned(), None),
        Some("saved") => (
            format!(
                "Ajouté : {} titre{}.",
                number("matched"),
                if number("matched") > 1 { "s" } else { "" }
            ),
            Some(100),
        ),
        Some("done") => (
            format!(
                "Terminé : {} titre{} ajouté{}.",
                number("matched"),
                if number("matched") > 1 { "s" } else { "" },
                if number("matched") > 1 { "s" } else { "" }
            ),
            Some(100),
        ),
        Some("error") | Some("failed") => ("L'analyse a échoué.".to_owned(), None),
        _ => ("Analyse en cours…".to_owned(), None),
    }
}

/// The unmatched titles a job could not find (`unmatchedItems`), at most 50.
pub fn unmatched(job: &serde_json::Value) -> Vec<String> {
    job.get("items")
        .and_then(|items| items.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|item| item.get("track").is_none_or(|track| track.is_null()))
                .filter_map(|item| {
                    item.get("title")
                        .and_then(|title| title.as_str())
                        .map(str::to_owned)
                })
                .take(50)
                .collect()
        })
        .unwrap_or_default()
}

/// A provider's state in one line and a tone (`accountSummary`).
pub fn provider_state(provider: &OnboardingProvider) -> (&'static str, &'static str) {
    if provider.linked {
        return ("Compte relié.", "ok");
    }
    if provider.available {
        ("Non relié", "idle")
    } else {
        match provider.reason.as_deref() {
            Some("deezer_disabled") => ("Désactivé sur ce serveur.", "error"),
            Some("denied") => ("Votre compte n'a pas accès à ce service.", "error"),
            _ => ("Indisponible sur ce serveur.", "error"),
        }
    }
}

/// The server's own messages for a link failure (`lib/onboarding.js#linkFailure`).
pub fn link_failure(error: &ApiError) -> String {
    match error {
        ApiError::Offline => {
            "Vous êtes hors ligne : la liaison attend le retour du réseau.".to_owned()
        }
        ApiError::Http { status: 429, .. } => {
            "Trop de tentatives. Réessayez dans quelques minutes.".to_owned()
        }
        ApiError::Http { status: 503, .. } => {
            "Le fournisseur ne répond pas. Réessayez dans un instant.".to_owned()
        }
        other => format!("La liaison a échoué : {}", other.cause_fr()),
    }
}

/// The same for an import (`importFailure`).
pub fn import_failure(error: &ApiError) -> String {
    match error {
        ApiError::Offline => {
            "Vous êtes hors ligne : l'import attend le retour du réseau.".to_owned()
        }
        ApiError::Http { status: 429, .. } => {
            "Un import est déjà en cours. Attendez qu'il finisse.".to_owned()
        }
        other => format!("L'import n'a pas pu démarrer : {}", other.cause_fr()),
    }
}

/// "3 artistes" / "1 artiste" (`lib/onboarding.js#artistCountLabel`).
pub fn artist_count_label(count: u32) -> String {
    format!("{count} artiste{}", if count > 1 { "s" } else { "" })
}

/// What to do next, in the SPA's words (`progressHint`).
pub fn progress_hint(count: u32) -> &'static str {
    match count {
        0 => "Choisissez-en au moins un. Trois à cinq nous donnent un bon départ.",
        1..=2 => "Encore quelques-uns et le Flow saura quoi vous proposer.",
        _ => "C'est un bon départ.",
    }
}

/// The YouTube card's sentence (`OnboardingAccount.svelte#ytState`).
pub fn youtube_state(youtube: &OnboardingYouTube) -> (&'static str, &'static str) {
    if !youtube.enabled && !youtube.installed {
        return ("YouTube Music n'est pas installé sur ce serveur.", "error");
    }
    if !youtube.enabled {
        return ("YouTube Music est désactivé sur ce serveur.", "error");
    }
    if youtube.denied {
        return ("Votre compte n'a pas accès à ce service.", "error");
    }
    ("Disponible.", "idle")
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.state.read(cx).online {
            self.flush_queue(cx);
        }
        self.create_fields(window, cx);
        self.want_covers(cx);

        let root = div()
            .id("onboarding-scroll")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .min_w(px(0.))
            .min_h(px(0.))
            .font_family(theme::font(cx))
            .text_color(theme::text(cx))
            .key_context("Neezer");

        let mut page = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .max_w(px(1800.))
            .gap(rems(1.))
            .p(rems(2.));

        // The header: where the person is in the flow, and the finish button.
        let label = if self.step == Step::Account {
            "Terminer"
        } else {
            "Continuer"
        };
        page = page.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.75))
                .child(
                    icon(IconName::Disc3)
                        .size(rems(1.25))
                        .text_color(theme::accent(cx)),
                )
                .child(
                    div()
                        .text_size(rems(0.9375))
                        .font_weight(FontWeight::BOLD)
                        .child("Neezer"),
                )
                .child(
                    div()
                        .font_family(theme::mono(cx))
                        .text_size(rems(0.625))
                        .text_color(theme::secondary(cx))
                        .child(format!("Étape {} sur 2", self.step.number())),
                )
                .child(div().flex_1())
                .when(self.step == Step::Account, |row| {
                    row.child(
                        button("onboarding-back", "Artistes", ButtonVariant::Secondary, cx)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.step = Step::Artists;
                                cx.notify();
                            })),
                    )
                })
                .child(
                    button("onboarding-finish", label, ButtonVariant::Primary, cx)
                        .disabled(self.finishing)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.step == Step::Artists {
                                this.finish_artists(cx);
                            } else {
                                this.finish_account(cx);
                            }
                        })),
                )
                .child(
                    div().w(px(120.)).h(px(3.)).bg(theme::border(cx)).child(
                        div()
                            .h(px(3.))
                            .w(px(if self.step == Step::Artists {
                                60.
                            } else {
                                120.
                            }))
                            .bg(theme::accent(cx)),
                    ),
                ),
        );

        if let Some(status) = self.status.clone() {
            page = page.child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(status),
            );
        }
        if let Some(error) = self.action_error.clone() {
            page = page.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.375))
                    .p(rems(0.5))
                    .border_1()
                    .border_color(theme::danger(cx))
                    .text_size(rems(0.75))
                    .text_color(theme::danger(cx))
                    .child(icon(IconName::CircleAlert).size(rems(0.875)))
                    .child(error),
            );
        }
        if let Some(notice) = self.notice.clone() {
            page = page.child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::success(cx))
                    .child(notice),
            );
        }
        if !self.state.read(cx).online {
            page = page.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.375))
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(icon(IconName::CloudOff).size(rems(0.875)))
                    .child(if self.step == Step::Account {
                        "Hors ligne : les liaisons et les imports attendent le retour du réseau."
                    } else {
                        "Hors ligne : vos choix seront enregistrés au retour du réseau."
                    }),
            );
        }

        match self.step {
            Step::Artists => page = self.artists_step(page, window, cx),
            Step::Account => page = self.account_step(page, window, cx),
        }

        root.child(page).into_any_element()
    }
}

impl Screen {
    /// The four text fields, created on the first paint (a draft survives a
    /// re-render; `Screen::new` has no window).
    fn create_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fields.search.is_none() {
            let field =
                cx.new(|cx| InputState::new(window, cx).placeholder("Chercher un artiste…"));
            let subscription =
                cx.subscribe(&field, |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => {
                        let value = this
                            .fields
                            .search
                            .as_ref()
                            .map(|field| field.read(cx).value().to_string())
                            .unwrap_or_default();
                        this.query = value;
                        this.debounce(cx);
                    }
                    InputEvent::PressEnter { .. } => {
                        let term = this.query.clone();
                        this.search(term, cx);
                    }
                    _ => {}
                });
            self._subscriptions.push(subscription);
            self.fields.search = Some(field);
        }
        if self.fields.arl.is_none() {
            self.fields.arl = Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder("ARL Deezer (jamais conservé ici)")
            }));
        }
        if self.fields.headers.is_none() {
            self.fields.headers = Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder("En-têtes d'une session YouTube Music")
            }));
        }
        if self.fields.link.is_none() {
            self.fields.link =
                Some(cx.new(|cx| InputState::new(window, cx).placeholder("Lien d'une playlist")));
        }
    }

    /// One term is asked [`DEBOUNCE_MS`] after the last keystroke, and only one
    /// search is in flight: a newer term is queued behind the answer.
    fn debounce(&mut self, cx: &mut Context<Self>) {
        self.search_pending = true;
        let generation = {
            self.search_generation += 1;
            self.search_generation
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(DEBOUNCE_MS))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.search_generation != generation || !this.search_pending {
                    return;
                }
                this.search_pending = false;
                let term = this.query.clone();
                this.search(term, cx);
            });
        })
        .detach();
    }

    fn artists_step(
        &mut self,
        mut page: gpui_kit::Div,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        page = page.child(
            div()
                .flex()
                .flex_col()
                .gap(rems(0.25))
                .child(
                    div()
                        .text_size(rems(1.5))
                        .font_weight(FontWeight::BOLD)
                        .child("Quels artistes écoutez-vous ?"),
                )
                .child(
                    div()
                        .max_w(rems(46.))
                        .text_size(rems(0.8125))
                        .text_color(theme::secondary(cx))
                        .child(
                            "Choisissez-en au moins un. Trois à cinq nous donnent un bon départ.",
                        ),
                ),
        );

        // The progress line: five marks and the count the SPA prints.
        let mut dots = div().flex().items_center().gap(rems(0.25));
        for index in 0..5u32 {
            dots = dots.child(div().w(rems(0.375)).h(rems(0.375)).bg(
                if index < self.liked_total {
                    theme::accent(cx)
                } else {
                    theme::border(cx)
                },
            ));
        }
        page = page.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(dots)
                .child(
                    div()
                        .text_size(rems(0.75))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(artist_count_label(self.liked_total)),
                )
                .child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child(progress_hint(self.liked_total)),
                ),
        );

        if let Some(field) = self.fields.search.clone() {
            page = page.child(div().max_w(rems(28.)).child(input(
                "onboarding-search",
                &field,
                window,
                cx,
            )));
        }

        if let Some(error) = &self.load_error {
            return page.child(card::failed(
                "Impossible de charger cette étape",
                format!(
                    "{} — votre compte est créé : rien n'est perdu.",
                    card::cause(error)
                ),
                cx,
                |this: &mut Self, _, cx| this.load(cx),
            ));
        }
        if let Some(error) = &self.search_error {
            return page.child(card::failed(
                "La recherche n'a pas abouti",
                card::cause(error),
                cx,
                |this: &mut Self, _, cx| {
                    let term = this.query.clone();
                    this.search(term, cx);
                },
            ));
        }
        if let Some(error) = &self.list_error
            && self.artists.is_empty()
        {
            return page.child(card::failed(
                "Impossible de charger les artistes",
                format!(
                    "{} — vous pourrez réessayer sans perdre votre compte.",
                    card::cause(error)
                ),
                cx,
                |this: &mut Self, _, cx| this.load(cx),
            ));
        }
        if (self.loading && self.artists.is_empty()) || (self.searching && self.found.is_none()) {
            return page.child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child("Nous préparons une sélection…"),
            );
        }
        if self.items().is_empty() {
            let mut empty = card::empty(
                IconName::Compass,
                if self.found.is_some() {
                    "Aucun artiste ne porte ce nom"
                } else {
                    "Aucun artiste n'est disponible pour l'instant"
                },
                if self.found.is_some() {
                    "Essayez un autre nom, ou effacez la recherche pour revoir la sélection."
                } else {
                    "Le serveur n'a pas pu construire de sélection : vous pouvez passer cette \
                     étape et suivre des artistes plus tard."
                },
                cx,
                None,
            );
            if self.found.is_none() && self.passed_available() {
                empty = empty.child(
                    button(
                        "onboarding-pass",
                        "Passer pour l'instant",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.pass_artists(cx))),
                );
            }
            return page.child(empty);
        }

        // The tiles: the whole tile is a toggle (a real button with
        // `aria-pressed`), so a click and `Entrée` do the same thing.
        let mut grid = div().flex().flex_wrap().gap(rems(0.5));
        for artist in self.items().to_vec() {
            grid = grid.child(self.tile(artist, cx));
        }
        page = page.child(grid);
        if self.found.is_none() {
            let label = if self.more {
                "Chargement…".to_owned()
            } else {
                "Voir plus d'artistes".to_owned()
            };
            page = page.child(
                button("onboarding-more", label, ButtonVariant::Secondary, cx)
                    .disabled(self.more || self.next.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.next_page(cx))),
            );
        }
        page
    }

    fn tile(&mut self, artist: OnboardingArtist, cx: &mut Context<Self>) -> AnyElement {
        let id = artist.id.clone();
        let liked = self.liked.contains(&id);
        let pending = self.pending.contains(&id);
        let file = artist
            .picture
            .as_deref()
            .or(artist.picture_small.as_deref())
            .and_then(|url| cover::file_in(&self.covers, url));
        let picture = artist.picture.clone().or(artist.picture_small.clone());
        let genres = artist
            .genres
            .iter()
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ");
        let hint = artist
            .hint
            .clone()
            .or(artist.via.clone())
            .unwrap_or_default();
        let mut tile = toggle(
            SharedString::from(format!("onboarding-artist-{id}")),
            "",
            liked,
            cx,
        )
        .debug_selector({
            let id = id.clone();
            move || format!("ob-artist-{id}")
        })
        .w(px(168.))
        .flex_col()
        .gap(rems(0.375))
        .p(rems(0.5))
        .border_1()
        .border_color(if liked {
            theme::accent(cx)
        } else {
            theme::border(cx)
        })
        .bg(if liked {
            theme::selected(cx)
        } else {
            theme::chrome(cx)
        })
        .child(cover::view(
            file.as_ref(),
            picture.as_deref(),
            &artist.name,
            Kind::Artist,
            px(96.),
            cx,
        ))
        .child(
            div()
                .truncate()
                .max_w(px(148.))
                .text_size(rems(0.8125))
                .font_weight(FontWeight::SEMIBOLD)
                .child(artist.name.clone()),
        );
        if !genres.is_empty() {
            tile = tile.child(
                div()
                    .truncate()
                    .max_w(px(148.))
                    .text_size(rems(0.625))
                    .text_color(theme::secondary(cx))
                    .child(genres),
            );
        }
        if !hint.is_empty() && hint != "library" {
            tile = tile.child(
                div()
                    .truncate()
                    .max_w(px(148.))
                    .text_size(rems(0.5625))
                    .text_color(theme::secondary(cx))
                    .child(hint),
            );
        }
        if pending {
            tile = tile.child(
                div()
                    .text_size(rems(0.5625))
                    .text_color(theme::secondary(cx))
                    .child("Enregistrement…"),
            );
        }
        // `Toggle::on_change` is not a GPUI context listener: it holds a weak
        // entity, so a tile that outlives its screen does nothing.
        let weak = cx.weak_entity();
        let like_id = id.clone();
        tile.on_change(move |pressed, _, _, cx| {
            let _ = weak.update(cx, |this, cx| this.set_like(like_id.clone(), pressed, cx));
        })
        .into_any_element()
    }

    fn account_step(
        &mut self,
        mut page: gpui_kit::Div,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        page = page.child(
            div()
                .flex()
                .flex_col()
                .gap(rems(0.25))
                .child(
                    div()
                        .text_size(rems(1.5))
                        .font_weight(FontWeight::BOLD)
                        .child("Reliez ce que vous écoutez ailleurs"),
                )
                .child(
                    div()
                        .max_w(rems(46.))
                        .text_size(rems(0.8125))
                        .text_color(theme::secondary(cx))
                        .child(
                            "Chaque option est facultative. Rien n'est publié ni modifié sur vos \
                             comptes.",
                        ),
                ),
        );

        if let Some(error) = &self.options_error {
            return page.child(card::failed(
                "Les options n'ont pas pu être lues",
                card::cause(error),
                cx,
                |this: &mut Self, _, cx| this.load_options(cx),
            ));
        }
        let Some(options) = self.options.clone() else {
            return page.child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child("Lecture des options…"),
            );
        };

        // Deezer: an ARL is the only credential the desktop asks for, and it is
        // never stored on the device (`docs/agents/desktop.md`, *Auth*).
        let (dz_text, dz_tone) = provider_state(&options.deezer);
        let deezer = self.provider_card(
            "deezer",
            "Deezer",
            "Collez votre ARL : il part au serveur et n'est pas conservé ici.",
            dz_text,
            dz_tone,
            options.deezer.available && !options.deezer.linked,
            cx,
        );
        let deezer = if options.deezer.available && !options.deezer.linked {
            match self.fields.arl.clone() {
                Some(field) => deezer.child(div().max_w(rems(28.)).child(input(
                    "onboarding-arl",
                    &field,
                    window,
                    cx,
                ))),
                None => deezer,
            }
        } else {
            deezer
        };
        page = page.child(deezer);

        // YouTube Music: a session's headers.
        let (yt_text, yt_tone) = if options.youtube.available || options.youtube.linked {
            let (text, tone) = provider_state(&options.youtube);
            (text.to_owned(), tone)
        } else {
            let (text, tone) = youtube_state(
                &self
                    .doc
                    .as_ref()
                    .map(|doc| doc.youtube.clone())
                    .unwrap_or_default(),
            );
            (text.to_owned(), tone)
        };
        let mut youtube = self.provider_card(
            "youtube",
            "YouTube Music",
            "Collez les en-têtes d'une session YouTube Music (la valeur de son cookie).",
            &yt_text,
            yt_tone,
            options.youtube.available && !options.youtube.linked,
            cx,
        );
        if options.youtube.available
            && !options.youtube.linked
            && let Some(field) = self.fields.headers.clone()
        {
            youtube = youtube.child(div().max_w(rems(28.)).child(input(
                "onboarding-headers",
                &field,
                window,
                cx,
            )));
        }
        page = page.child(youtube);

        // The imports: a playlist link, analysed by the server as a job.
        let available = options
            .imports
            .iter()
            .any(|import| import.available.unwrap_or(false));
        let mut imports = div()
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .p(rems(1.))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .text_size(rems(0.9375))
                    .font_weight(FontWeight::BOLD)
                    .child("Importer une playlist"),
            )
            .child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(
                        "Un lien de playlist d'un autre service : le serveur lit les titres et \
                         cherche lesquels il possède déjà, en arrière-plan.",
                    ),
            );
        for import in &options.imports {
            if import.available == Some(false) {
                imports = imports.child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child(format!(
                            "{} : {}",
                            import.label,
                            import
                                .reason
                                .clone()
                                .unwrap_or_else(|| "indisponible".into())
                        )),
                );
            }
        }
        if available {
            if let Some(field) = self.fields.link.clone() {
                imports = imports.child(div().max_w(rems(32.)).child(input(
                    "onboarding-import",
                    &field,
                    window,
                    cx,
                )));
            }
            let label = if self.busy == Some("playlist") {
                "Lancement…".to_owned()
            } else {
                "Importer".to_owned()
            };
            imports = imports.child(
                button("onboarding-import-start", label, ButtonVariant::Primary, cx)
                    .disabled(self.busy.is_some())
                    .on_click(cx.listener(|this, _, _, cx| this.import_link(cx))),
            );
        }
        if let Some(job) = &self.job {
            let (label, pct) = job_progress(job);
            let failed = job.get("state").and_then(|state| state.as_str()) == Some("error")
                || job.get("error").is_some_and(|error| !error.is_null());
            let mut panel = div().flex().flex_col().gap(rems(0.25)).child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .text_size(rems(0.75))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(label),
                    )
                    .when_some(pct, |row, pct| {
                        row.child(
                            div()
                                .font_family(theme::mono(cx))
                                .text_size(rems(0.625))
                                .text_color(theme::secondary(cx))
                                .child(format!("{pct} %")),
                        )
                    }),
            );
            if let Some(pct) = pct {
                panel = panel.child(
                    div().h(px(3.)).w_full().bg(theme::border(cx)).child(
                        div()
                            .h(px(3.))
                            .w(px(2.4 * pct as f32))
                            .bg(theme::accent(cx)),
                    ),
                );
            }
            if let Some(error) = job
                .get("error")
                .and_then(|error| error.as_str())
                .filter(|error| !error.is_empty())
            {
                panel = panel.child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::danger(cx))
                        .child(error.to_owned()),
                );
            }
            let missed = unmatched(job);
            if !missed.is_empty() {
                panel = panel.child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .child(format!(
                            "{} titre{} n'ont pas été trouvés : {}",
                            missed.len(),
                            if missed.len() > 1 { "s" } else { "" },
                            missed
                                .iter()
                                .take(6)
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", ")
                        )),
                );
            }
            if failed && self.job_error.is_none() {
                self.job_error = Some(SharedString::from(
                    "L'import a échoué : vérifiez le lien, ou réessayez plus tard.",
                ));
            }
            imports = imports.child(panel);
        }
        if let Some(error) = &self.job_error {
            imports = imports.child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::danger(cx))
                    .child(error.clone()),
            );
        }
        page = page.child(imports);
        page.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child("Vous pourrez relier tout cela plus tard, dans Réglages → Sources."),
                )
                .child(div().flex_1())
                .child(
                    button(
                        "onboarding-finish-account",
                        "Terminer",
                        ButtonVariant::Primary,
                        cx,
                    )
                    .disabled(self.finishing)
                    .on_click(cx.listener(|this, _, _, cx| this.finish_account(cx))),
                ),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn provider_card(
        &mut self,
        id: &'static str,
        title: &str,
        hint: &str,
        state: &str,
        tone: &str,
        can_link: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let color = match tone {
            "ok" => theme::success(cx),
            "error" => theme::danger(cx),
            _ => theme::secondary(cx),
        };
        let busy = self.busy == Some(id);
        let mut card = div()
            .id(SharedString::from(format!("onboarding-{id}")))
            .flex()
            .flex_col()
            .gap(rems(0.375))
            .p(rems(1.))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .text_size(rems(0.9375))
                            .font_weight(FontWeight::BOLD)
                            .child(title.to_owned()),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(rems(0.75))
                            .text_color(color)
                            .child(state.to_owned()),
                    ),
            )
            .child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(hint.to_owned()),
            );
        if can_link {
            card = card.child(
                button(
                    SharedString::from(format!("onboarding-link-{id}")),
                    if busy { "Liaison…" } else { "Relier" },
                    ButtonVariant::Secondary,
                    cx,
                )
                .disabled(busy)
                .on_click(cx.listener(move |this, _, _, cx| this.link_provider(id, cx))),
            );
        }
        card
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str =
        r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#;
    const STATE: &str = r#"{"step":"artists","liked":0,"min_artists":1,"fallback":"",
        "feed":{"source":"deezer","state":"ready","ready":true,"building":false,
                "stale":false,"kept":0},
        "diversity":{"window":0,"max_same_genre":0,"max_streak":0,"chart_max":0,
                     "country_max":0,"country_window":0,"region_max":0,"region_window":0},
        "account":{"can_save_imports":false,"deezer":{"available":true,"linked":false},
                   "youtube":{"available":false,"linked":false},"imports":[]},
        "youtube":{"enabled":false,"installed":false,"denied":false,"held":false,
                   "error":null,"reason":null,"retry_in":0}}"#;
    const ARTISTS: &str = r#"{"items":[
        {"id":"42","deezer_id":"42","name":"Kaya Project","picture":null,"provider":"deezer",
         "liked":false},
        {"id":"43","deezer_id":"43","name":"Bonobo","picture":null,"provider":"deezer",
         "liked":false}],"next":null,"total":2,"state":"ready","source":"deezer"}"#;

    /// The acceptance: the first step is complete with the KEYBOARD alone — Tab
    /// walks the stops and `Entrée` keeps an artist, exactly as a click would.
    #[gpui::test]
    async fn a_tile_is_kept_with_the_keyboard_alone(cx: &mut gpui::TestAppContext) {
        let stub = crate::widgets::card::stub::Stub::start(&[
            ("/api/login", ME),
            ("/api/me", ME),
            ("/api/onboarding", STATE),
            ("/api/onboarding/artists", ARTISTS),
            ("/api/onboarding/artists/like", "{}"),
        ]);
        let (state, runtime) = crate::widgets::card::stub::logged_in(cx, &stub);
        let (screen, cx) =
            cx.add_window_view(move |_, cx| Screen::new(Route::Onboarding, state.clone(), cx));
        crate::widgets::card::stub::until(&runtime, cx, "the feed", |cx| {
            screen.read(cx).artists.len() == 2
        });
        // Every control is a Tab stop: `Window::focus_next` is what
        // gpui-omarchy's Tab binding calls. The walk happens on a laid-out frame
        // (tab stops are registered during layout, not by the first paint), and
        // the DISTINCT handles it visits are the stops — a wrap does not change
        // the set.
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
        let mut seen: Vec<String> = Vec::new();
        for _ in 0..12 {
            cx.update(|window, cx| window.focus_next(cx));
            cx.run_until_parked();
            if let Some(handle) = cx.update(|window, cx| window.focused(cx)) {
                let key = format!("{handle:?}");
                if !seen.contains(&key) {
                    seen.push(key);
                }
            }
        }
        // « Continuer », the search field and the two tiles: a disabled
        // « Voir plus d'artistes » is not a stop, which is the point of
        // disabling it.
        assert_eq!(seen.len(), 4, "the first step's tab stops, tiles included");
        // The like is the SAME handler the tile's activation calls (a click and
        // `Entrée` both arrive as `on_change` on the toggle): one request, and
        // the tile flips before the answer.
        cx.update(|_, cx| screen.update(cx, |screen, cx| screen.set_like("42".into(), true, cx)));
        cx.update(|_, cx| {
            let screen = screen.read(cx);
            assert_eq!(screen.liked_total, 1, "the tile flips in the frame");
            assert!(screen.liked.contains("42"));
        });
        crate::widgets::card::stub::until(&runtime, cx, "the answer", |cx| {
            screen.read(cx).pending.is_empty()
        });
        assert_eq!(stub.count("/api/onboarding/artists/like"), 1);
        runtime.shutdown_background();
        stub.stop();
    }
}
