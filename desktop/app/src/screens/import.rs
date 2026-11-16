//! Importer une playlist partagée : coller un lien, suivre le travail, écouter
//! tout de suite, garder seulement si on veut.
//!
//! The server does the reading and the matching as a JOB (`playlist_import.py`:
//! `reading` → `matching` → `done`, then `saving` → `saved`), so this screen
//! polls it — 700 ms while it runs, three times slower after a failed poll —
//! and prints what the job says at each step: the playlist's name and cover, the
//! number matched, then every title it could NOT place, by name, with the search
//! that would find it by hand. Rule 2's "never just a count".
//!
//! The result is TEMPORARY until « Enregistrer » makes it one of the person's
//! playlists, which is the server's own rule and needs an administrator.

use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Div, ElementId, Entity, FocusHandle, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Window, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon, input};
use serde_json::Value;

use nz_model::Track;

use crate::nav::Route;
use crate::screens::library::{artist_line, format_duration, sleeve};
use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;

/// The poll period the SPA uses while an import runs (`POLL_MS`).
const POLL_MS: u64 = 700;

/// One line of the source playlist and what the server made of it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration: u32,
    /// The library/provider row, when one was found.
    pub track: Option<Track>,
    /// `"local"` or the provider's name.
    pub via: Option<String>,
    /// `false` for a "looks very close" match the person should check.
    pub sure: bool,
}

impl Line {
    fn parse(value: &Value) -> Self {
        let line = value.get("line").cloned().unwrap_or(Value::Null);
        let track = value
            .get("track")
            .filter(|track| !track.is_null())
            .and_then(|track| serde_json::from_value::<Track>(track.clone()).ok());
        Self {
            title: line
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            artists: line
                .get("artists")
                .and_then(Value::as_array)
                .map(|artists| {
                    artists
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            album: line.get("album").and_then(Value::as_str).map(str::to_owned),
            duration: line.get("duration").and_then(Value::as_u64).unwrap_or(0) as u32,
            track,
            via: value.get("via").and_then(Value::as_str).map(str::to_owned),
            sure: value.get("sure").and_then(Value::as_bool).unwrap_or(false),
        }
    }

    pub fn artists_line(&self) -> String {
        self.artists.join(", ")
    }
}

/// The job's snapshot (`_snapshot` in `playlist_import.py`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Job {
    pub state: String,
    pub name: String,
    pub cover: Option<String>,
    pub source_label: String,
    pub total: u32,
    pub done: u32,
    pub matched: u32,
    pub truncated: bool,
    pub error: Option<String>,
    pub playlist_id: Option<String>,
    pub counts: Vec<(String, u32)>,
    pub items: Vec<Line>,
}

impl Job {
    pub fn parse(value: &Value) -> Self {
        let counts = value
            .get("counts")
            .and_then(Value::as_object)
            .map(|object| {
                let mut rows: Vec<(String, u32)> = object
                    .iter()
                    .map(|(key, count)| {
                        (
                            key.clone(),
                            count.as_u64().unwrap_or(0).min(u32::MAX as u64) as u32,
                        )
                    })
                    .collect();
                rows.sort_by(|left, right| left.0.cmp(&right.0));
                rows
            })
            .unwrap_or_default();
        Self {
            state: value
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("reading")
                .to_owned(),
            name: value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            cover: value
                .get("cover")
                .and_then(Value::as_str)
                .map(str::to_owned),
            source_label: value
                .get("source_label")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            total: value.get("total").and_then(Value::as_u64).unwrap_or(0) as u32,
            done: value.get("done").and_then(Value::as_u64).unwrap_or(0) as u32,
            matched: value.get("matched").and_then(Value::as_u64).unwrap_or(0) as u32,
            truncated: value
                .get("truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            error: value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_owned),
            playlist_id: value
                .get("playlist_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            counts,
            items: value
                .get("items")
                .and_then(Value::as_array)
                .map(|items| items.iter().map(Line::parse).collect())
                .unwrap_or_default(),
        }
    }

    pub fn running(&self) -> bool {
        matches!(self.state.as_str(), "reading" | "matching" | "saving")
    }

    /// `done / total`, 0 while the playlist is still being read.
    pub fn progress(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.done.min(self.total) as f32) / (self.total as f32)
        }
    }

    pub fn missing(&self) -> Vec<&Line> {
        self.items
            .iter()
            .filter(|line| line.track.is_none())
            .collect()
    }

    pub fn unsure(&self) -> Vec<&Line> {
        self.items
            .iter()
            .filter(|line| line.track.is_some() && !line.sure)
            .collect()
    }

    pub fn tracks(&self) -> Vec<Track> {
        self.items
            .iter()
            .filter_map(|line| line.track.clone())
            .collect()
    }
}

/// The provider a pasted link names, by host — the same four the server reads
/// (`lib/linkimport.py`), so the screen can say "lien Deezer reconnu" before any
/// request.
pub fn recognise(url: &str) -> Option<&'static str> {
    let lowered = url.trim().to_ascii_lowercase();
    if !(lowered.starts_with("http://") || lowered.starts_with("https://")) {
        return None;
    }
    if lowered.contains("deezer.com") {
        Some("Deezer")
    } else if lowered.contains("spotify.com") {
        Some("Spotify")
    } else if lowered.contains("youtube.com") || lowered.contains("youtu.be") {
        Some("YouTube")
    } else if lowered.contains("music.apple.com") {
        Some("Apple Music")
    } else {
        None
    }
}

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    link: Option<Entity<InputState>>,
    /// The name field the "Enregistrer" gesture opens.
    title: Option<Entity<InputState>>,
    job: Option<Job>,
    job_id: Option<String>,
    error: Option<SharedString>,
    starting: bool,
    keeping: bool,
    /// The last URL the route carried, so a new one is not applied twice.
    applied_route: Option<String>,
    /// The link field is emptied on the next paint (see `reset`).
    clear_link: bool,
    focus: FocusHandle,
    focused_once: bool,
    _subscriptions: Vec<Subscription>,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let applied_route = match &route {
            Route::Import { url: Some(url) } => Some(url.clone()),
            _ => None,
        };
        let mut this = Self {
            route,
            state,
            link: None,
            title: None,
            job: None,
            job_id: None,
            error: None,
            starting: false,
            keeping: false,
            applied_route,
            clear_link: false,
            focus: cx.focus_handle(),
            focused_once: false,
            _subscriptions: Vec::new(),
        };
        this._subscriptions
            .push(cx.observe(&this.state.clone(), |_, _, cx| {
                cx.notify();
            }));
        this
    }

    fn ensure_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.link.is_some() {
            return;
        }
        let link = cx.new(|cx| {
            InputState::new(window, cx).placeholder("https://open.spotify.com/playlist/…")
        });
        self._subscriptions
            .push(cx.subscribe(&link, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.start(cx);
                } else if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Nom de la playlist"));
        self._subscriptions
            .push(cx.subscribe(&title, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.keep(cx);
                }
            }));
        self.link = Some(link);
        self.title = Some(title);
    }

    fn link_text(&self, cx: &gpui_kit::App) -> String {
        self.link
            .as_ref()
            .map(|link| link.read(cx).value().to_string())
            .unwrap_or_default()
    }

    fn request<T, F, G>(&self, cx: &mut Context<Self>, future: F, done: G)
    where
        T: Send + 'static,
        F: std::future::Future<Output = T> + Send + 'static,
        G: FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    {
        let runtime = self.state.read(cx).runtime.clone();
        let task = runtime.spawn(future);
        cx.spawn(async move |this, cx| {
            if let Ok(answer) = task.await {
                this.update(cx, |this, cx| done(this, answer, cx)).ok();
            }
        })
        .detach();
    }

    /// Start reading the pasted link. The job's id comes back at once; what it
    /// finds arrives through `poll`.
    fn start(&mut self, cx: &mut Context<Self>) {
        if self.starting {
            return;
        }
        let url = self.link_text(cx).trim().to_owned();
        if url.is_empty() {
            self.error = Some("Collez le lien d'une playlist.".into());
            cx.notify();
            return;
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.starting = true;
        self.error = None;
        self.job = None;
        self.job_id = None;
        cx.notify();
        self.request(
            cx,
            async move { api.import_playlist(&url).await },
            |this, result, cx| {
                this.starting = false;
                match result {
                    Ok(answer) => {
                        this.job_id = Some(answer.id.clone());
                        this.job = Some(Job {
                            state: "reading".into(),
                            ..Job::default()
                        });
                        this.schedule(0, cx);
                    }
                    Err(error) => {
                        this.error = Some(error.cause_fr().into());
                    }
                }
                cx.notify();
            },
        );
    }

    fn schedule(&mut self, delay_ms: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(delay_ms))
                .await;
            this.update(cx, |this, cx| this.poll(cx)).ok();
        })
        .detach();
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        let Some(job_id) = self.job_id.clone() else {
            return;
        };
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let request_id = job_id.clone();
        self.request(
            cx,
            async move { api.import_status(&request_id).await },
            move |this, result, cx| {
                if this.job_id.as_deref() != Some(job_id.as_str()) {
                    // A newer import started while this answer was on its way.
                    return;
                }
                match result {
                    Ok(value) => {
                        let job = Job::parse(&value);
                        // A save's polls carry no item list: keep the one on
                        // screen instead of redrawing it empty.
                        let items = if job.items.is_empty() {
                            this.job
                                .as_ref()
                                .map(|previous| previous.items.clone())
                                .unwrap_or_default()
                        } else {
                            job.items.clone()
                        };
                        let running = job.running();
                        let saved = job.state == "saved";
                        this.job = Some(Job { items, ..job });
                        if running {
                            this.schedule(POLL_MS, cx);
                        } else if saved {
                            this.on_saved(cx);
                        }
                    }
                    Err(nz_api::ApiError::Http { status: 404, .. }) => {
                        this.job = None;
                        this.job_id = None;
                        this.error = Some("Cet import a expiré : relancez-le.".into());
                    }
                    Err(error) => {
                        // A missed poll is not a failed import: ask again,
                        // slower, and keep what is on screen.
                        let _ = error;
                        this.schedule(POLL_MS * 3, cx);
                    }
                }
                cx.notify();
            },
        );
    }

    fn on_saved(&mut self, cx: &mut Context<Self>) {
        if let Some(api) = self.state.read(cx).api() {
            api.forget("/me/playlists");
        }
        let title = self
            .job
            .as_ref()
            .map(|job| job.name.clone())
            .unwrap_or_default();
        if !title.is_empty() {
            crate::widgets::sheets::push_toast(
                cx,
                &self.state,
                ToastKind::Info,
                format!("Playlist « {title} » enregistrée dans votre bibliothèque."),
            );
        }
    }

    fn open_keep(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .job
            .as_ref()
            .map(|job| job.name.clone())
            .unwrap_or_default();
        if let Some(title) = self.title.clone() {
            title.update(cx, |state, cx| {
                state.set_value(name, window, cx);
                state.focus(window, cx);
            });
        }
        self.keeping = true;
        cx.notify();
    }

    fn keep(&mut self, cx: &mut Context<Self>) {
        let Some(job_id) = self.job_id.clone() else {
            return;
        };
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let title = self
            .title
            .as_ref()
            .map(|title| title.read(cx).value().trim().to_owned())
            .unwrap_or_default();
        let fallback = self
            .job
            .as_ref()
            .map(|job| job.name.clone())
            .unwrap_or_default();
        let title = if title.is_empty() { fallback } else { title };
        self.keeping = false;
        self.request(
            cx,
            async move { api.import_save(&job_id, &title, true).await },
            |this, result, cx| {
                match result {
                    Ok(_) => {
                        if let Some(job) = &mut this.job {
                            job.state = "saving".into();
                        }
                        this.schedule(0, cx);
                    }
                    Err(error) => {
                        crate::widgets::sheets::push_toast(
                            cx,
                            &this.state,
                            ToastKind::Error,
                            error.cause_fr(),
                        );
                    }
                }
                cx.notify();
            },
        );
    }

    fn play(&mut self, shuffle: bool, cx: &mut Context<Self>) {
        let Some(job) = &self.job else {
            return;
        };
        let tracks = job.tracks();
        if tracks.is_empty() {
            return;
        }
        let entries: Vec<nz_player::Entry> =
            tracks.iter().map(crate::state::entry_from_track).collect();
        let context = Some(nz_player::Context {
            kind: "import".into(),
            id: self.job_id.clone(),
            title: Some(job.name.clone()),
        });
        let command = if shuffle {
            nz_player::Command::ShufflePlay {
                args: nz_player::ShuffleArgs {
                    tracks: entries,
                    context,
                },
            }
        } else {
            nz_player::Command::PlayQueue {
                args: nz_player::QueueArgs {
                    tracks: entries,
                    start: 0,
                    context,
                },
            }
        };
        self.state
            .update(cx, |state, cx| state.command(command, cx));
        cx.notify();
    }

    fn goto(&mut self, route: Route, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.nav.push(route);
            cx.notify();
        });
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.job = None;
        self.job_id = None;
        self.error = None;
        self.keeping = false;
        // The field is emptied on the next paint: `InputState::set_value` needs
        // the window, and a click handler's closure does not have to carry it
        // just for this.
        self.clear_link = true;
        cx.notify();
    }
}

impl Render for Screen {
    #[allow(refining_impl_trait)]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.ensure_fields(window, cx);
        if std::mem::take(&mut self.clear_link)
            && let Some(link) = self.link.clone()
        {
            link.update(cx, |state, cx| state.set_value("", window, cx));
        }
        if !self.focused_once {
            window.focus(&self.focus, cx);
            self.focused_once = true;
        }
        // `#/import/<url>` from the search box or a share target.
        let routed = match &self.route {
            Route::Import { url: Some(url) } => Some(url.clone()),
            _ => None,
        };
        if routed.is_some() && routed != self.applied_route {
            self.applied_route = routed.clone();
            if let (Some(url), Some(link)) = (routed, self.link.clone()) {
                let decoded = percent_decode(&url);
                link.update(cx, |state, cx| state.set_value(decoded, window, cx));
                self.start(cx);
            }
        }
        let link_text = self.link_text(cx);
        let recognised = recognise(&link_text);

        let mut root = div()
            .id("import-screen")
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .p(rems(1.5))
            .gap(rems(0.75))
            .track_focus(&self.focus);

        let Some(job) = self.job.clone().filter(|job| {
            // The intro comes back for a finished-with-error job.
            !matches!(job.state.as_str(), "error")
        }) else {
            root = root
                .child(
                    div()
                        .text_size(rems(1.5))
                        .font_weight(FontWeight::BOLD)
                        .child("Importer une playlist"),
                )
                .child(
                    div()
                        .max_w(rems(46.))
                        .text_size(rems(0.8125))
                        .text_color(theme::secondary(cx))
                        .child(
                            "Collez le lien d'une playlist qu'on vous a partagée : Deezer, Spotify, \
                             YouTube ou Apple Music. Chaque titre est retrouvé (votre bibliothèque \
                             d'abord), vous l'écoutez tout de suite, et vous la gardez seulement si \
                             vous voulez.",
                        ),
                );
            if let Some(link) = self.link.clone() {
                root = root.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .max_w(rems(46.))
                        .child(div().flex_1().child(input("import-url", &link, window, cx)))
                        .child(
                            button(
                                "import-start",
                                if self.starting { "…" } else { "Importer" },
                                ButtonVariant::Primary,
                                cx,
                            )
                            .child(icon(IconName::Link2).size(rems(0.875)))
                            .disabled(self.starting || link_text.trim().is_empty())
                            .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                        ),
                );
            }
            if let Some(label) = recognised
                && self.error.is_none()
            {
                root = root.child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::accent(cx))
                        .child(format!("Lien {label} reconnu.")),
                );
            }
            if let Some(error) = self.error.clone() {
                root = root.child(
                    div()
                        .flex()
                        .items_start()
                        .gap(rems(0.5))
                        .p(rems(0.5))
                        .max_w(rems(46.))
                        .border_1()
                        .border_color(theme::danger(cx))
                        .text_size(rems(0.75))
                        .text_color(theme::danger(cx))
                        .child(icon(IconName::CircleAlert).size(rems(1.)))
                        .child(div().flex_1().child(error)),
                );
            }
            return root.into_any_element();
        };

        // The job: what it is doing, and everything it found or missed.
        let mut head = div()
            .id("import-head")
            .flex()
            .items_center()
            .gap(rems(1.))
            .child(sleeve(job.cover.as_deref(), &job.name, 96., cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.25))
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(rems(0.625))
                            .text_color(theme::secondary(cx))
                            .child(format!(
                                "{} · {}",
                                job.source_label,
                                if job.state == "saved" {
                                    "Playlist enregistrée"
                                } else {
                                    "Playlist importée"
                                }
                            )),
                    )
                    .child(
                        div()
                            .text_size(rems(1.375))
                            .font_weight(FontWeight::BOLD)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(if job.name.is_empty() {
                                "Lecture de la playlist…".to_owned()
                            } else {
                                job.name.clone()
                            }),
                    ),
            );
        if job.state == "reading" {
            head = head.child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child("Lecture de la playlist…"),
            );
        } else if job.running() {
            head = head.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.25))
                    .w(rems(18.))
                    .child(
                        div()
                            .text_size(rems(0.75))
                            .text_color(theme::secondary(cx))
                            .child(format!(
                                "Recherche des titres · {} / {} · {} trouvés",
                                job.done.min(job.total),
                                job.total,
                                job.matched
                            )),
                    )
                    .child(
                        div().h(px(4.)).w_full().bg(theme::border(cx)).child(
                            div()
                                .h(px(4.))
                                .w(gpui_kit::relative(job.progress()))
                                .bg(theme::accent(cx)),
                        ),
                    ),
            );
        }
        root = root.child(head);

        let mut actions = div().flex().items_center().gap(rems(0.375)).flex_shrink_0();
        if !job.running() {
            actions = actions
                .child(
                    button("import-play", "Lire", ButtonVariant::Primary, cx)
                        .disabled(job.tracks().is_empty())
                        .on_click(cx.listener(|this, _, _, cx| this.play(false, cx))),
                )
                .child(
                    button("import-shuffle", "Mélanger", ButtonVariant::Secondary, cx)
                        .disabled(job.tracks().is_empty())
                        .on_click(cx.listener(|this, _, _, cx| this.play(true, cx))),
                );
        }
        if job.state == "saved" {
            if let Some(id) = job.playlist_id.clone() {
                actions = actions.child(
                    button(
                        "import-open",
                        "Ouvrir la playlist",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.goto(Route::Playlist { id: id.clone() }, cx)
                    })),
                );
            }
        } else if job.state == "saving" {
            actions = actions.child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child("Enregistrement…"),
            );
        } else if self.keeping {
            if let Some(title) = self.title.clone() {
                actions = actions
                    .child(
                        div()
                            .w(rems(18.))
                            .child(input("import-title", &title, window, cx)),
                    )
                    .child(
                        button("import-keep-go", "Enregistrer", ButtonVariant::Primary, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.keep(cx))),
                    )
                    .child(
                        button(
                            "import-keep-cancel",
                            "Annuler",
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.keeping = false;
                            cx.notify();
                        })),
                    );
            }
        } else if job.state == "done" && !job.tracks().is_empty() {
            actions = actions.child(
                button("import-keep", "Enregistrer", ButtonVariant::Outline, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.open_keep(window, cx))),
            );
        }
        actions = actions.child(div().flex_1()).child(
            button("import-other", "Autre lien", ButtonVariant::Secondary, cx)
                .on_click(cx.listener(|this, _, _, cx| this.reset(cx))),
        );
        root = root.child(actions);

        if let Some(error) = job.error.clone() {
            root = root.child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::danger(cx))
                    .child(error),
            );
        }
        if job.truncated {
            root = root.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(format!(
                        "Seuls les {} premiers titres de la playlist ont pu être lus.",
                        job.items.len()
                    )),
            );
        }
        if !job.counts.is_empty() {
            let mut chips = div().flex().items_center().gap(rems(0.375)).flex_shrink_0();
            for (via, count) in &job.counts {
                let where_ = if via == "local" {
                    "dans ta bibliothèque".to_owned()
                } else {
                    format!("sur {via}")
                };
                chips = chips.child(
                    div()
                        .px(rems(0.375))
                        .py(rems(0.125))
                        .border_1()
                        .border_color(theme::border(cx))
                        .text_size(rems(0.625))
                        .text_color(theme::secondary(cx))
                        .child(format!("{count} {where_}")),
                );
            }
            let missing = job.missing().len();
            if missing > 0 {
                chips = chips.child(
                    div()
                        .px(rems(0.375))
                        .py(rems(0.125))
                        .border_1()
                        .border_color(theme::warning(cx))
                        .text_size(rems(0.625))
                        .text_color(theme::warning(cx))
                        .child(format!("{missing} introuvable(s)")),
                );
            }
            root = root.child(chips);
        }

        let tracks = job.tracks();
        if !tracks.is_empty() {
            let mut list = div()
                .id("import-tracks")
                .flex()
                .flex_col()
                .gap(rems(0.125))
                .flex_1()
                .min_h_0()
                .overflow_y_scroll();
            for track in tracks.iter().take(500) {
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .h(px(44.))
                        .px(rems(0.5))
                        .child(sleeve(track.album.cover.as_deref(), &track.title, 32., cx))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_size(rems(0.75))
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(track.title.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(rems(0.625))
                                        .text_color(theme::secondary(cx))
                                        .child(artist_line(track)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(rems(0.625))
                                .text_color(theme::secondary(cx))
                                .child(format_duration(track.duration)),
                        ),
                );
            }
            root = root.child(list);
        }

        // What to check, and what could not be found at all: each line names
        // the title and the gesture that finds it by hand.
        let unsure = job.unsure();
        if !unsure.is_empty() {
            root = rest_section("À vérifier", &unsure, cx, root);
        }
        let missing = job.missing();
        if !missing.is_empty() {
            root = rest_section("Introuvables", &missing, cx, root);
        }
        root.into_any_element()
    }
}

/// A section of lines the import could not settle, each one a search away.
fn rest_section(
    title: &str,
    lines: &[&Line],
    cx: &mut Context<Screen>,
    root: gpui_kit::Stateful<Div>,
) -> gpui_kit::Stateful<Div> {
    let mut section = div()
        .flex()
        .flex_col()
        .gap(rems(0.125))
        .flex_shrink_0()
        .pt(rems(0.5));
    section = section.child(
        div()
            .text_size(rems(0.875))
            .font_weight(FontWeight::BOLD)
            .child(format!("{title} · {}", lines.len())),
    );
    for (index, line) in lines.iter().take(40).enumerate() {
        let query = format!(
            "{} {}",
            line.artists.first().cloned().unwrap_or_default(),
            line.title
        );
        let query = query.trim().to_owned();
        let id = ElementId::Name(format!("import-line-{title}-{index}").into());
        section = section.child(
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(rems(0.5))
                .min_h(px(36.))
                .px(rems(0.5))
                .cursor_pointer()
                .hover(|style| style.bg(theme::selected(cx)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.goto(
                        Route::Search {
                            query: query.clone(),
                        },
                        cx,
                    )
                }))
                .child(div().text_size(rems(0.6875)).child(line.title.clone()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(rems(0.625))
                        .text_color(theme::secondary(cx))
                        .child(format!(
                            "{}{}",
                            line.artists_line(),
                            if line.album.as_deref().is_some_and(|album| !album.is_empty()) {
                                format!(" · {}", line.album.as_deref().unwrap_or_default())
                            } else {
                                String::new()
                            }
                        )),
                )
                .child(
                    icon(IconName::Search)
                        .size(rems(0.875))
                        .text_color(theme::secondary(cx)),
                ),
        );
    }
    root.child(section)
}

/// `decodeURIComponent` for a route's `?url=` segment, without pulling a crate
/// in: a malformed sequence keeps the text as it came.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(value) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

impl std::fmt::Debug for Screen {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ImportScreen")
            .field("state", &self.job.as_ref().map(|job| job.state.clone()))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_is_recognised_by_its_host_and_nothing_else() {
        assert_eq!(
            recognise("https://www.deezer.com/fr/playlist/123"),
            Some("Deezer")
        );
        assert_eq!(
            recognise("https://open.spotify.com/playlist/abc"),
            Some("Spotify")
        );
        assert_eq!(
            recognise("https://music.youtube.com/playlist?list=x"),
            Some("YouTube")
        );
        assert_eq!(recognise("https://youtu.be/x"), Some("YouTube"));
        assert_eq!(
            recognise("https://music.apple.com/fr/playlist/x/pl.1"),
            Some("Apple Music")
        );
        assert_eq!(recognise("https://example.org/playlist"), None);
        assert_eq!(recognise("not a link"), None);
    }

    #[test]
    fn the_job_reads_its_states_and_its_progress() {
        let value: Value = serde_json::from_str(
            r#"{"state":"matching","name":"Route","source_label":"Spotify",
                "total":10,"done":4,"matched":3,"truncated":false,
                "counts":{"local":2,"deezer":1},
                "items":[{"line":{"title":"Été","artists":["Kaya"],"album":"A",
                    "duration":200},"track":null,"via":null,"sure":false}]}"#,
        )
        .expect("job");
        let job = Job::parse(&value);
        assert!(job.running());
        assert_eq!(job.done, 4);
        assert_eq!(job.progress(), 0.4);
        assert_eq!(
            job.counts,
            [("deezer".to_owned(), 1), ("local".to_owned(), 2)]
        );
        assert_eq!(job.missing().len(), 1);
        assert_eq!(job.missing()[0].artists_line(), "Kaya");
        assert!(job.tracks().is_empty());
    }

    #[test]
    fn a_finished_job_is_not_running_and_a_saved_one_names_its_playlist() {
        let value: Value = serde_json::from_str(
            r#"{"state":"saved","name":"Route","playlist_id":"pl-1","total":2,"done":2,
                "matched":2,"items":[]}"#,
        )
        .expect("job");
        let job = Job::parse(&value);
        assert!(!job.running());
        assert_eq!(job.state, "saved");
        assert_eq!(job.playlist_id.as_deref(), Some("pl-1"));
    }

    #[test]
    fn a_percent_encoded_url_comes_back_whole() {
        assert_eq!(
            percent_decode("https%3A%2F%2Fopen.spotify.com%2Fplaylist%2Fabc"),
            "https://open.spotify.com/playlist/abc"
        );
        assert_eq!(percent_decode("100%"), "100%", "a lone % is kept");
        assert_eq!(percent_decode("caf%C3%A9"), "café");
    }
}
