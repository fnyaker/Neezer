//! Contrôle à distance (lot D15a) : confier CE lecteur à quelqu'un, ou piloter
//! celui d'un autre.
//!
//! The SPA's `RemoteSheet.svelte` + `RemoteHostChip.svelte` + `RemoteClaim.svelte`
//! in one screen, because a desktop app has no link to open in a browser: a
//! controller claims by pasting the link it was sent. The contract is
//! `neezer/webui/remote.py`, the behaviour `webapp/src/lib/remote/`, the notes
//! [party-remote.md](../../../../docs/agents/party-remote.md).
//!
//! Three rules, each one the SPA's:
//!
//! - **A link drives ONE device, the one it was made on**, and the owner keeps
//!   the hand: the live line counts who is driving, every link has its own
//!   « Couper », « Tout couper » asks once, and the owner's own buttons work as
//!   they always did (last action wins).
//! - **Four levels, each a superset of the last** (`queue`, `read`, `full`,
//!   `admin`); the sheet SAYS what each lends, hides `admin` from a person who is
//!   not one, and the server's `POLICY` is the enforcement — this screen only
//!   never offers what would be refused.
//! - **Nothing long happens in a frame** (rule 2): every request is a task on
//!   `nz-net`, its answer lands in this entity, and a failure prints its cause in
//!   its own sentence. The loops themselves live in
//!   [`crate::wiring::remote_host`] and [`crate::wiring::remote_controller`], so a
//!   loan keeps answering — or following the owner — after this screen is gone.
//!
//! Every control is a `button`/`toggle`, so Tab reaches it and Entrée presses it:
//! the keyboard-only and mouse-only paths are the same handlers.

use std::time::{Duration, Instant};

use gpui_kit::base::input::InputState;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, SharedString, StatefulInteractiveElement, Styled, Subscription,
    Window, div, px, rems,
};
use gpui_omarchy::{
    ButtonVariant, IconName, button, icon, icon_button, input, progress, toggle, toggle_group,
    virtual_list,
};
use nz_model::{RemoteLevel, RemoteLinks};
use nz_player::{Command, Entry};

use crate::nav::Route;
use crate::screens::settings::{card, card_title, eid, failed, hint};
use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;
use crate::widgets::sheets::push_toast;
use crate::wiring::remote_controller::{self, Controller, Phase};
use crate::wiring::remote_host::{self, LinkRow, RemoteHost};

/// How long the sheet waits between « who is driving? » reads while it is open
/// (`RemoteSheet.svelte`'s 4 s interval).
const LINKS_REFRESH: Duration = Duration::from_secs(4);
/// The paint clock: who is driving and what the owner's player is doing change on
/// `nz-net`, and this screen repaints on a change of either version.
const PAINT: Duration = Duration::from_millis(250);
/// One queue row's height, the lists' own (D8a's `track_list`).
const ROW_HEIGHT: f32 = 56.;

/// The durations `RemoteSheet.svelte#DURATIONS` offers, in seconds (`None` = no
/// expiry).
const DURATIONS: [(Option<i64>, &str); 4] = [
    (Some(3600), "1 heure"),
    (Some(86_400), "1 jour"),
    (Some(7 * 86_400), "7 jours"),
    (None, "Sans limite"),
];

/// What the claim form is doing.
#[derive(Clone, Debug, Default, PartialEq)]
enum Claim {
    #[default]
    Idle,
    Busy,
    Failed(SharedString),
}

/// `mm:ss`, the player bar's own format.
fn clock(seconds: f64) -> String {
    crate::player_bar::clock(seconds)
}

/// A tile that carries a level's glyph (`RemoteSheet.svelte`'s `.av`/`.li`).
fn level_tile(level: &str, live: bool, cx: &App) -> Div {
    div()
        .flex_none()
        .size(rems(2.25))
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(if live {
            theme::accent(cx)
        } else {
            theme::border(cx)
        })
        .bg(if live {
            theme::selected(cx)
        } else {
            theme::shell(cx)
        })
        .text_color(if live {
            theme::accent(cx)
        } else {
            theme::secondary(cx)
        })
        .child(icon(level_icon(level)).size(rems(1.)))
}

/// The glyph of a level, the SPA's `LEVEL_INFO[level].icon`.
fn level_icon(level: &str) -> IconName {
    match RemoteLevel::parse(level) {
        Some(RemoteLevel::Queue) => IconName::ListMusic,
        Some(RemoteLevel::Read) => IconName::Eye,
        Some(RemoteLevel::Full) => IconName::SlidersVertical,
        Some(RemoteLevel::Admin) => IconName::Shield,
        None => IconName::Cast,
    }
}

/// The dot that says « somebody is driving » (`RemoteHostChip.svelte`'s `.dot`).
fn live_dot(cx: &App) -> Div {
    div()
        .flex_none()
        .size(rems(0.5))
        .rounded_full()
        .bg(theme::success(cx))
}

pub struct Screen {
    route: Route,
    state: Entity<AppState>,

    // -- the owner's side ---------------------------------------------------
    host: Option<RemoteHost>,
    /// The links onto this device, as last read.
    links: Vec<LinkRow>,
    links_loaded: bool,
    links_busy: bool,
    sheet_error: Option<SharedString>,
    /// The link being made: its level, its duration (an index in [`DURATIONS`])
    /// and the name the owner gives it.
    level: RemoteLevel,
    duration: usize,
    label: Option<Entity<InputState>>,
    /// The link just created: its URL is what the person hands over, so it stays
    /// on screen until they say « Terminé ».
    invite: Option<LinkRow>,
    confirm_cut_all: bool,

    // -- the controller's side ----------------------------------------------
    controller: Option<Controller>,
    token: Option<Entity<InputState>>,
    claim: Claim,
    /// The owner's queue, and the `qv` it belongs to: 5 000 rows are cloned only
    /// when the owner's queue really moved.
    queue: Vec<Entry>,
    queue_version: String,
    queue_sizes: std::rc::Rc<Vec<gpui_kit::Size<Pixels>>>,
    scroll: gpui_kit::base::VirtualListScrollHandle,

    // -- paint bookkeeping --------------------------------------------------
    painted_host: u64,
    painted_loan: u64,
    last_links: Option<Instant>,
    _state: Subscription,
}

impl Screen {
    /// The screen `screens::build` calls. No request and no field: a screen is
    /// built without a window (D7), so the two text fields are made on the first
    /// paint (`ensure_fields`).
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let host = RemoteHost::get();
        let controller = Controller::get();
        let _state = cx.observe(&state, |_, _, cx| cx.notify());
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PAINT).await;
                let alive = this.update(cx, |this, cx| this.paint(cx));
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            route,
            state,
            host,
            links: Vec::new(),
            links_loaded: false,
            links_busy: false,
            sheet_error: None,
            level: RemoteLevel::Queue,
            duration: 1,
            label: None,
            invite: None,
            confirm_cut_all: false,
            controller,
            token: None,
            claim: Claim::Idle,
            queue: Vec::new(),
            queue_version: String::new(),
            queue_sizes: std::rc::Rc::new(Vec::new()),
            scroll: gpui_kit::base::VirtualListScrollHandle::new(),
            painted_host: 0,
            painted_loan: 0,
            last_links: None,
            _state,
        }
    }

    /// The route this screen was built for (`screens::build` hands it over).
    pub fn route(&self) -> &Route {
        &self.route
    }

    /// The loan this session runs under, when it is one.
    pub fn loan(&self) -> Option<remote_controller::Grant> {
        self.controller.as_ref().and_then(Controller::grant)
    }

    // --- the paint clock ----------------------------------------------------

    /// What the services changed since the last paint: a controller arrived, the
    /// owner's queue moved, the grant ended. Nothing here touches the network.
    fn paint(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        if let Some(host) = &self.host {
            let version = host.status().version;
            if version != self.painted_host {
                self.painted_host = version;
                changed = true;
            }
        }
        if let Some(controller) = &self.controller {
            let version = controller.status().version;
            if version != self.painted_loan {
                self.painted_loan = version;
                changed = true;
            }
            let queue_version = controller.queue_version();
            if queue_version != self.queue_version {
                self.queue_version = queue_version;
                self.queue = controller.queue();
                self.queue_sizes = std::rc::Rc::new(
                    (0..self.queue.len())
                        .map(|_| gpui_kit::Size::new(px(0.), px(ROW_HEIGHT)))
                        .collect(),
                );
                changed = true;
            }
        }
        // The sheet re-reads its links while it is open, and only when nothing
        // else is in flight: a dead link must not become a request storm.
        let due = self
            .last_links
            .is_none_or(|at| at.elapsed() >= LINKS_REFRESH);
        if due && !self.links_busy && self.loan().is_none() {
            self.refresh(cx);
        }
        if changed {
            cx.notify();
        }
    }

    fn ensure_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.label.is_none() {
            self.label = Some(
                window
                    .use_keyed_state("remote-label", cx, |window, cx| {
                        InputState::new(window, cx)
                            .placeholder("Prénom, pour s'y retrouver (facultatif)")
                    })
                    .clone(),
            );
        }
        if self.token.is_none() {
            self.token = Some(
                window
                    .use_keyed_state("remote-token", cx, |window, cx| {
                        InputState::new(window, cx)
                            .placeholder("Collez le lien de contrôle (…/rc/<jeton>)")
                    })
                    .clone(),
            );
        }
    }

    // --- the owner's side ---------------------------------------------------

    /// Re-read the links onto this device. `GET /api/remote/links?device=`.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let Some(device) = self.device(cx) else {
            return;
        };
        self.links_busy = true;
        self.last_links = Some(Instant::now());
        let runtime = self.state.read(cx).runtime.clone();
        let task = runtime.spawn(async move { api.remote_links(Some(&device)).await });
        cx.spawn(async move |this, cx| {
            let answer = task.await;
            this.update(cx, |this, cx| {
                this.links_busy = false;
                match answer {
                    Ok(Ok(links)) => this.adopt_links(links, cx),
                    // A read that failed keeps what is on screen; a CAUSE is for an
                    // action the person took, not for a poll.
                    Ok(Err(error)) => this.sheet_error = Some(error.cause_fr().into()),
                    Err(_) => {}
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn adopt_links(&mut self, links: RemoteLinks, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.links = links
            .links
            .iter()
            .map(|link| LinkRow::from_link(&api, link))
            .collect();
        self.links_loaded = true;
        self.sheet_error = None;
        if let Some(invite) = self.invite.clone()
            && let Some(row) = self.links.iter().find(|row| row.id == invite.id)
        {
            self.invite = Some(row.clone());
        }
    }

    /// This device's id: the service's own when it exists, the store's otherwise.
    fn device(&self, cx: &App) -> Option<String> {
        if let Some(host) = &self.host {
            return Some(host.device());
        }
        self.state
            .read(cx)
            .cache
            .as_ref()
            .map(remote_host::device_id)
    }

    /// `POST /api/remote/links` — lend this player, then answer at once.
    fn create(&mut self, cx: &mut Context<Self>) {
        if self.links_busy {
            return;
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let Some(device) = self.device(cx) else {
            self.sheet_error = Some("Le stockage local est indisponible.".into());
            return;
        };
        let label = self
            .label
            .as_ref()
            .map(|field| field.read(cx).value().trim().to_owned())
            .filter(|label| !label.is_empty());
        let ttl = DURATIONS[self.duration.min(DURATIONS.len() - 1)].0;
        let mut options = serde_json::json!({
            "level": self.level.wire(),
            "device": device,
            "device_name": remote_host::device_name(),
        });
        if let Some(label) = &label {
            options["label"] = serde_json::Value::String(label.clone());
        }
        if let Some(ttl) = ttl {
            options["ttl"] = serde_json::Value::from(ttl);
        }
        self.links_busy = true;
        self.sheet_error = None;
        cx.notify();
        let runtime = self.state.read(cx).runtime.clone();
        let host = self.host.clone();
        let state = self.state.clone();
        let task = runtime.spawn(async move { api.remote_create(&options).await });
        cx.spawn(async move |this, cx| {
            let answer = task.await;
            this.update(cx, |this, cx| {
                this.links_busy = false;
                match answer {
                    Ok(Ok(created)) => {
                        let api = state.read(cx).api();
                        let row = api
                            .map(|api| LinkRow::from_link(&api, &created.link))
                            .unwrap_or_default();
                        this.invite = Some(row.clone());
                        this.links.insert(0, row);
                        this.links_loaded = true;
                        // This device is lent from now on: it answers.
                        if let Some(host) = &host {
                            host.start();
                        }
                        push_toast(
                            cx,
                            &state,
                            ToastKind::Info,
                            "Lien créé : ce lecteur répond au contrôle à distance.",
                        );
                    }
                    Ok(Err(error)) => this.sheet_error = Some(create_cause(&error).into()),
                    Err(_) => {
                        this.sheet_error = Some("Le lien n'a pas pu être créé.".into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// `DELETE /api/remote/links/<id>` — one link, away at once.
    fn cut(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        if let Some(invite) = &self.invite
            && invite.id == id
        {
            self.invite = None;
        }
        self.links.retain(|row| row.id != id);
        self.confirm_cut_all = false;
        let state = self.state.clone();
        let runtime = self.state.read(cx).runtime.clone();
        let task = runtime.spawn(async move { api.remote_revoke(&id).await });
        cx.spawn(async move |_this, cx| {
            let answer = task.await;
            cx.update(|cx| match answer {
                Ok(Ok(())) => push_toast(cx, &state, ToastKind::Info, "Lien coupé."),
                Ok(Err(error)) => push_toast(
                    cx,
                    &state,
                    ToastKind::Error,
                    format!("Impossible de couper ce lien : {}", error.cause_fr()),
                ),
                Err(_) => {}
            });
        })
        .detach();
        cx.notify();
    }

    /// `DELETE /api/remote/links?device=` — everybody off this player.
    fn cut_everything(&mut self, cx: &mut Context<Self>) {
        if !self.confirm_cut_all {
            self.confirm_cut_all = true;
            cx.notify();
            return;
        }
        self.confirm_cut_all = false;
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let Some(device) = self.device(cx) else {
            return;
        };
        self.links.clear();
        self.invite = None;
        // The loops stop before the answer: nothing is sent on a link that is
        // being cut.
        if let Some(host) = &self.host {
            host.stop();
        }
        let state = self.state.clone();
        let runtime = self.state.read(cx).runtime.clone();
        let task = runtime.spawn(async move { api.remote_revoke_device(&device).await });
        cx.spawn(async move |_this, cx| {
            let answer = task.await;
            cx.update(|cx| match answer {
                Ok(Ok(())) => push_toast(cx, &state, ToastKind::Info, "Contrôle à distance coupé."),
                Ok(Err(error)) => push_toast(
                    cx,
                    &state,
                    ToastKind::Error,
                    format!("Impossible de tout couper : {}", error.cause_fr()),
                ),
                Err(_) => {}
            });
        })
        .detach();
        cx.notify();
    }

    /// Copy a link: the capability IS the URL, so this is how a desktop hands one
    /// over.
    fn copy(&mut self, url: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(url));
        let state = self.state.clone();
        push_toast(
            cx,
            &state,
            ToastKind::Info,
            "Lien copié — envoyez-le à la personne qui doit piloter ce lecteur.",
        );
    }

    // --- the controller's side ----------------------------------------------

    /// `POST /api/remote/claim`: this app becomes the controller, then the session
    /// is read again so `/api/me` reports the loan and the shell rebuilds around
    /// it (no audio engine, no offline copy of the owner's library).
    fn claim(&mut self, cx: &mut Context<Self>) {
        if self.claim == Claim::Busy {
            return;
        }
        let raw = self
            .token
            .as_ref()
            .map(|field| field.read(cx).value().to_string())
            .unwrap_or_default();
        let Some(token) = remote_controller::token_from_input(&raw) else {
            self.claim = Claim::Failed("Collez le lien complet (…/rc/<jeton>).".into());
            cx.notify();
            return;
        };
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.claim = Claim::Busy;
        cx.notify();
        let runtime = self.state.read(cx).runtime.clone();
        let task = runtime.spawn(async move { remote_controller::claim(&api, &token).await });
        let state = self.state.clone();
        cx.spawn(async move |this, cx| {
            let answer = task.await;
            this.update(cx, |this, cx| {
                match answer {
                    Ok(Ok(_)) => {
                        this.claim = Claim::Idle;
                        // `/api/me` now carries the grant: the shell rebuilds as
                        // a controller and drops the engine with it.
                        state.update(cx, |state, cx| state.retry_session(cx));
                    }
                    Ok(Err(error)) => this.claim = Claim::Failed(claim_cause(&error).into()),
                    Err(_) => {
                        this.claim = Claim::Failed("Le lien n'a pas pu être ouvert.".into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Leave the loan: the server gives back the account this app had, if any.
    fn leave(&mut self, cx: &mut Context<Self>) {
        if let Some(controller) = &self.controller {
            controller.stop();
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        let state = self.state.clone();
        let task = runtime.spawn(async move { api.remote_leave().await });
        cx.spawn(async move |_this, cx| {
            let _ = task.await;
            cx.update(|cx| state.update(cx, |state, cx| state.retry_session(cx)));
        })
        .detach();
        cx.notify();
    }

    /// Send one transport command to the owner's player.
    fn send(&mut self, command: Command, cx: &mut Context<Self>) {
        let Some(controller) = self.controller.clone() else {
            return;
        };
        self.state
            .update(cx, |state, cx| controller.command(state, command, cx));
    }

    /// `0..=1` of the owner's track, from its published line.
    fn fraction(&self, cx: &App) -> f32 {
        let (position, duration) = match &self.controller {
            Some(controller) => (controller.position(), controller.duration()),
            None => (0.0, 0.0),
        };
        let _ = cx;
        if duration > 0.0 {
            (position / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        }
    }

    /// A seek to `fraction` of the owner's track, the player bar's own gesture.
    fn seek(&mut self, fraction: f64, cx: &mut Context<Self>) {
        let Some(controller) = self.controller.clone() else {
            return;
        };
        let duration = controller.duration();
        let at = (fraction.clamp(0.0, 1.0) * duration).clamp(0.0, (duration - 0.25).max(0.0));
        self.send(Command::seek(at), cx);
    }

    // --- the two views ------------------------------------------------------

    fn owner_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let status = self
            .host
            .as_ref()
            .map(RemoteHost::status)
            .unwrap_or_default();
        let live = status.live();
        let is_admin = self.state.read(cx).is_admin();
        let mut column = div()
            .id("remote-column")
            .flex()
            .flex_col()
            .w_full()
            .max_w(rems(56.))
            .gap(rems(1.))
            .px(rems(1.25))
            .py(rems(1.125))
            .child(status_card(&status, live, cx));

        if let Some(invite) = self.invite.clone() {
            column = column.child(self.invite_card(&invite, cx));
        } else {
            column = column.child(self.new_link_card(is_admin, window, cx));
        }
        column = column.child(self.links_card(&status, cx));
        column = column.child(self.claim_card(window, cx));
        column = column.child(
            div()
                .flex()
                .items_center()
                .flex_wrap()
                .gap(rems(0.5))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.375))
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child(icon(IconName::Monitor).size(rems(0.875)))
                        .child(format!("Ce lecteur : {}", remote_host::device_name())),
                )
                .child(div().flex_1())
                .child({
                    let armed = self.confirm_cut_all;
                    let label = if armed {
                        "Confirmer : tout le monde est déconnecté"
                    } else {
                        "Tout couper"
                    };
                    div().child(
                        button(
                            "remote-cut-all",
                            label,
                            if armed {
                                ButtonVariant::Danger
                            } else {
                                ButtonVariant::Secondary
                            },
                            cx,
                        )
                        .child(icon(IconName::LogOut).size(rems(0.75)))
                        .on_click(cx.listener(|this, _, _, cx| this.cut_everything(cx))),
                    )
                }),
        );
        centred(column, cx)
    }

    fn invite_card(&self, invite: &LinkRow, cx: &mut Context<Self>) -> AnyElement {
        let url = invite.url.clone();
        card("remote-invite", cx)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.625))
                    .child(level_tile(&invite.level, true, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(rems(0.125))
                            .child(
                                div()
                                    .text_size(rems(0.8125))
                                    .font_weight(FontWeight::BOLD)
                                    .child(invite.title()),
                            )
                            .child(hint(
                                format!("Scannez, ou envoyez le lien. {}.", invite.expiry),
                                cx,
                            )),
                    ),
            )
            .child(
                div()
                    .id("remote-invite-url")
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .p(rems(0.625))
                    .border_1()
                    .border_color(theme::border(cx))
                    .bg(theme::shell(cx))
                    .font_family(theme::mono(cx))
                    .text_size(rems(0.75))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(url.clone()),
                    )
                    .child(
                        icon_button(
                            "remote-invite-copy",
                            IconName::Copy,
                            "Copier le lien",
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .on_click(cx.listener({
                            let url = url.clone();
                            move |this, _, _, cx| this.copy(url.clone(), cx)
                        })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.375))
                    .child(div().flex_1())
                    .child(
                        button(
                            "remote-invite-done",
                            "Terminé",
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .child(icon(IconName::Check).size(rems(0.75)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.invite = None;
                            cx.notify();
                        })),
                    ),
            )
            .into_any_element()
    }

    fn new_link_card(
        &self,
        is_admin: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut levels = div().flex().flex_wrap().gap(rems(0.5));
        let offered: Vec<RemoteLevel> = [RemoteLevel::Queue, RemoteLevel::Read, RemoteLevel::Full]
            .into_iter()
            .chain(is_admin.then_some(RemoteLevel::Admin))
            .collect();
        for level in offered {
            let selected = self.level == level;
            let wire = level.wire();
            levels = levels.child(
                div().flex_1().min_w(rems(13.)).child(
                    button(
                        ElementId::Name(format!("remote-level-{wire}").into()),
                        "",
                        if selected {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        },
                        cx,
                    )
                    .selected(selected)
                    .flex()
                    .items_start()
                    .gap(rems(0.5))
                    .w_full()
                    .py(rems(0.5))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.level = level;
                        cx.notify();
                    }))
                    .child(level_tile(wire, selected, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(rems(0.125))
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .child(remote_host::level_label(wire)),
                            )
                            .child(
                                div()
                                    .text_size(rems(0.6875))
                                    .text_color(theme::secondary(cx))
                                    .child(remote_host::level_blurb(wire)),
                            ),
                    ),
                ),
            );
        }

        let mut durations = toggle_group("remote-durations", cx);
        for (index, (_, label)) in DURATIONS.iter().enumerate() {
            let selected = self.duration == index;
            durations = durations.child(
                toggle(
                    eid(format!("remote-duration-{index}")),
                    *label,
                    selected,
                    cx,
                )
                .debug_selector(move || format!("remote-duration-{index}"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.duration = index;
                    cx.notify();
                })),
            );
        }

        let mut body = card("remote-new", cx)
            .child(card_title("Nouveau lien", cx))
            .child(hint(
                "Ce lien prête CE lecteur : la personne qui l'ouvre le pilote depuis \
                 son téléphone, avec la même app.",
                cx,
            ))
            .child(levels)
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .w(rems(4.5))
                            .text_size(rems(0.75))
                            .text_color(theme::secondary(cx))
                            .child("Durée"),
                    )
                    .child(durations),
            );
        if let Some(field) = &self.label {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .w(rems(4.5))
                            .text_size(rems(0.75))
                            .text_color(theme::secondary(cx))
                            .child("Pour"),
                    )
                    .child(div().flex_1().min_w_0().child(input(
                        "remote-label-field",
                        field,
                        window,
                        cx,
                    ))),
            );
        }
        let busy = self.links_busy;
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(div().flex_1())
                .child(
                    div().child(
                        button(
                            "remote-create",
                            if busy {
                                "Création…"
                            } else {
                                "Créer le lien"
                            },
                            ButtonVariant::Primary,
                            cx,
                        )
                        .disabled(busy)
                        .child(
                            icon(if busy {
                                IconName::LoaderCircle
                            } else {
                                IconName::Link
                            })
                            .size(rems(0.75)),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.create(cx))),
                    ),
                ),
        );
        if let Some(cause) = &self.sheet_error {
            body = body.child(failed("remote-new-error", cause.clone(), cx));
        }
        body.into_any_element()
    }

    fn links_card(&self, status: &remote_host::HostStatus, cx: &mut Context<Self>) -> AnyElement {
        let mut card = card("remote-links", cx).child(card_title("Liens actifs", cx));
        if !self.links_loaded {
            card = card.child(hint("Lecture des liens…", cx));
        } else if self.links.is_empty() {
            card = card.child(hint(
                "Aucun lien sur ce lecteur. Celui que vous créez apparaît ici, avec qui \
                 s'en sert.",
                cx,
            ));
        } else {
            let mut rows = div().flex().flex_col().gap(rems(0.125));
            for link in &self.links {
                let live = link.controllers > 0;
                let level = link.level.clone();
                let id = link.id.clone();
                rows = rows.child(
                    div()
                        .debug_selector({
                            let id = id.clone();
                            move || format!("remote-link-{id}")
                        })
                        .flex()
                        .items_center()
                        .gap(rems(0.625))
                        .py(rems(0.25))
                        .child(level_tile(&level, live, cx))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(rems(0.125))
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_size(rems(0.8125))
                                        .font_weight(FontWeight::BOLD)
                                        .child(link.title()),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(rems(0.25))
                                        .text_size(rems(0.6875))
                                        .text_color(theme::secondary(cx))
                                        .child(link.expiry.clone())
                                        .when(live, |row| {
                                            row.child(div().text_color(theme::success(cx)).child(
                                                if link.controllers == 1 {
                                                    " · connecté".to_owned()
                                                } else {
                                                    format!(" · {} connectés", link.controllers)
                                                },
                                            ))
                                        }),
                                ),
                        )
                        .child(
                            icon_button(
                                ElementId::Name(format!("remote-cut-{id}").into()),
                                IconName::Close,
                                "Couper ce lien",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| this.cut(id.clone(), cx))),
                        ),
                );
            }
            card = card.child(rows);
        }
        if status.live() > 0 {
            card = card.child(hint(
                format!(
                    "{} {} ce lecteur en ce moment.",
                    status.live(),
                    if status.live() == 1 {
                        "appareil pilote"
                    } else {
                        "appareils pilotent"
                    }
                ),
                cx,
            ));
        }
        card.into_any_element()
    }

    fn claim_card(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut body = card("remote-claim", cx)
            .child(card_title("Piloter un autre lecteur", cx))
            .child(hint(
                "Vous avez reçu un lien de contrôle ? Collez-le ici : cette app pilotera \
                 le lecteur de la personne, sans rien garder de sa bibliothèque.",
                cx,
            ));
        if let Some(field) = &self.token {
            body = body.child(div().min_w_0().child(input(
                "remote-token-field",
                field,
                window,
                cx,
            )));
        }
        let busy = self.claim == Claim::Busy;
        body = body.child(
            div().flex().items_center().gap(rems(0.5)).child(
                div().child(
                    button(
                        "remote-pilot",
                        if busy { "Connexion…" } else { "Piloter" },
                        ButtonVariant::Outline,
                        cx,
                    )
                    .disabled(busy)
                    .child(icon(IconName::Cast).size(rems(0.75)))
                    .on_click(cx.listener(|this, _, _, cx| this.claim(cx))),
                ),
            ),
        );
        if let Claim::Failed(cause) = &self.claim {
            body = body.child(failed("remote-claim-error", cause.clone(), cx));
        }
        body.into_any_element()
    }

    fn controller_view(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(controller) = self.controller.clone() else {
            return div().into_any_element();
        };
        let status = controller.status();
        let grant = controller.grant().unwrap_or_default();
        let ended = status.phase() == Phase::Ended;
        let (position, duration) = (controller.position(), controller.duration());
        let playing = self.state.read(cx).player.playing();
        let queue = self.queue.clone();
        let index = self.state.read(cx).player.index();
        let fraction = self.fraction(cx);

        // -- the banner: whose player this app drives, and the way out --------
        let mut banner = card("remote-loan", cx).child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.625))
                .child(level_tile(grant.level.wire(), !ended, cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.125))
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(rems(0.875))
                                .font_weight(FontWeight::BOLD)
                                .child(if ended {
                                    "Le contrôle est terminé".to_owned()
                                } else {
                                    format!("Vous pilotez le lecteur de {}", grant.owner)
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(rems(0.375))
                                .text_size(rems(0.75))
                                .text_color(theme::secondary(cx))
                                .when(!ended, |row| row.child(live_dot(cx)))
                                .child(if ended {
                                    "Ce lien a été coupé, ou il a expiré.".to_owned()
                                } else {
                                    status.line()
                                })
                                .when(!grant.device.is_empty() && !ended, |row| {
                                    row.child(format!(" · {}", grant.device))
                                }),
                        ),
                )
                .child(if ended {
                    button("remote-back", "Revenir", ButtonVariant::Primary, cx)
                        .child(icon(IconName::House).size(rems(0.75)))
                        .on_click(cx.listener(|this, _, _, cx| this.leave(cx)))
                        .into_any_element()
                } else {
                    button("remote-leave", "Quitter", ButtonVariant::Secondary, cx)
                        .child(icon(IconName::LogOut).size(rems(0.75)))
                        .on_click(cx.listener(|this, _, _, cx| this.leave(cx)))
                        .into_any_element()
                }),
        );
        if let Some(cause) = &status.error
            && !ended
        {
            banner = banner.child(failed("remote-loan-error", cause.clone(), cx));
        }
        if ended {
            return centred(banner, cx);
        }
        if status.phase() == Phase::Reconnecting {
            banner = banner.child(
                div().flex().items_center().gap(rems(0.5)).child(
                    button("remote-retry", "Réessayer", ButtonVariant::Outline, cx)
                        .child(icon(IconName::RefreshCw).size(rems(0.75)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.controller.as_ref().map(Controller::poke);
                            cx.notify();
                        })),
                ),
            );
        }

        // -- the transport ----------------------------------------------------
        let mut transport = card("remote-transport", cx).child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.375))
                .child(icon_button(
                    "remote-prev",
                    IconName::SkipBack,
                    "Titre précédent",
                    ButtonVariant::Secondary,
                    cx,
                ))
                .child(icon_button(
                    "remote-toggle",
                    if playing {
                        IconName::Pause
                    } else {
                        IconName::Play
                    },
                    if playing { "Pause" } else { "Lecture" },
                    ButtonVariant::Primary,
                    cx,
                ))
                .child(icon_button(
                    "remote-next",
                    IconName::SkipForward,
                    "Titre suivant",
                    ButtonVariant::Secondary,
                    cx,
                )),
        );
        // The three transport buttons of the line above, wired once here.
        transport = transport
            .child(div().h(rems(0.25)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(div().flex_1().min_w_0().child(progress(
                        "remote-progress",
                        fraction * 100.,
                        cx,
                    )))
                    .child(
                        div()
                            .font_family(theme::mono(cx))
                            .text_size(rems(0.6875))
                            .text_color(theme::secondary(cx))
                            .child(format!("{} / {}", clock(position), clock(duration))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap(rems(0.375))
                    .child(button(
                        "remote-back-10",
                        "−10 s",
                        ButtonVariant::Secondary,
                        cx,
                    ))
                    .child(button(
                        "remote-fwd-10",
                        "+10 s",
                        ButtonVariant::Secondary,
                        cx,
                    ))
                    .child(div().w(rems(0.5)))
                    .child(button(
                        "remote-volume-down",
                        "−",
                        ButtonVariant::Secondary,
                        cx,
                    ))
                    .child(button(
                        "remote-volume-up",
                        "+",
                        ButtonVariant::Secondary,
                        cx,
                    ))
                    .child(button("remote-mute", "Muet", ButtonVariant::Secondary, cx))
                    .child(button(
                        "remote-shuffle",
                        "Aléatoire",
                        ButtonVariant::Secondary,
                        cx,
                    ))
                    .child(button(
                        "remote-repeat",
                        "Répéter",
                        ButtonVariant::Secondary,
                        cx,
                    )),
            );

        // -- the owner's queue -------------------------------------------------
        let mut queue_card =
            card("remote-queue", cx).child(card_title("File du lecteur piloté", cx));
        if queue.is_empty() {
            queue_card = queue_card.child(hint("Ce lecteur n'a rien dans sa file.", cx));
        } else {
            let sizes = self.queue_sizes.clone();
            let rows = queue.clone();
            let playing_row = index;
            let scroll = self.scroll.clone();
            let list = virtual_list(
                cx.entity(),
                "remote-queue-list",
                sizes,
                move |this, range, _, cx| {
                    range
                        .filter_map(|row| {
                            let entry = rows.get(row)?.clone();
                            Some(this.loan_row(row, entry, playing_row, cx))
                        })
                        .collect()
                },
                cx,
            )
            .h(px(ROW_HEIGHT * 6.))
            .track_scroll(&scroll);
            queue_card = queue_card.child(list);
        }

        let column = div()
            .id("remote-column")
            .flex()
            .flex_col()
            .w_full()
            .max_w(rems(56.))
            .gap(rems(1.))
            .px(rems(1.25))
            .py(rems(1.125))
            .child(banner)
            .child(transport)
            .child(queue_card);
        centred(column, cx)
    }

    fn loan_row(
        &self,
        row: usize,
        entry: Entry,
        playing: i64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_now = row as i64 == playing;
        let title = entry.title.clone();
        let artist = entry.artist_name().to_owned();
        let may_edit = self
            .controller
            .as_ref()
            .map(|controller| controller.allows("remove", None))
            .unwrap_or(false);
        div()
            .id(("remote-row", row))
            .h(px(ROW_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .gap(rems(0.5))
            .px(rems(0.5))
            .border_l_2()
            .border_color(if is_now {
                theme::accent(cx)
            } else {
                theme::shell(cx)
            })
            .bg(if is_now {
                theme::selected(cx)
            } else {
                theme::shell(cx)
            })
            .cursor_pointer()
            .hover(|style| style.bg(theme::selected(cx)))
            .on_click(cx.listener(move |this, _, _, cx| {
                let Some(controller) = this.controller.clone() else {
                    return;
                };
                let current = this.queue.get(row).map(Entry::key).unwrap_or_default();
                this.send(Command::jump_named(row, current), cx);
                let _ = controller;
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(rems(0.8125))
                            .when(is_now, |row| row.font_weight(FontWeight::BOLD))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .text_color(theme::secondary(cx))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(artist),
                    ),
            )
            .when(may_edit, |row_el| {
                row_el.child(
                    div()
                        .debug_selector(|| format!("remote-remove-{row}"))
                        .child(
                            icon_button(
                                ElementId::Name(format!("remote-remove-{row}").into()),
                                IconName::Close,
                                "Retirer de la file",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    let Some(entry) = this.queue.get(row).cloned() else {
                                        return;
                                    };
                                    this.send(
                                        Command::remove_named(row, entry.key().to_owned()),
                                        cx,
                                    );
                                },
                            )),
                        ),
                )
            })
            .into_any_element()
    }
}

/// The screen's centred reading column: one paragraph width, never a 2 560 px
/// line (the Réglages pages' own measure).
fn centred(column: impl IntoElement, cx: &App) -> AnyElement {
    div()
        .id("remote-page")
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .overflow_y_scroll()
        .bg(theme::shell(cx))
        .font_family(theme::font(cx))
        .text_color(theme::text(cx))
        .child(column)
        .into_any_element()
}

/// The owner's status card: what this device is, and who is driving it.
fn status_card(status: &remote_host::HostStatus, live: usize, cx: &App) -> AnyElement {
    let mut card = card("remote-status", cx).child(
        div()
            .flex()
            .items_center()
            .gap(rems(0.75))
            .child(
                div()
                    .flex_none()
                    .size(rems(2.75))
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_1()
                    .border_color(if live > 0 {
                        theme::accent(cx)
                    } else {
                        theme::border(cx)
                    })
                    .bg(if live > 0 {
                        theme::selected(cx)
                    } else {
                        theme::shell(cx)
                    })
                    .text_color(if live > 0 {
                        theme::accent(cx)
                    } else {
                        theme::text(cx)
                    })
                    .child(icon(IconName::Cast).size(rems(1.375))),
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
                            .text_size(rems(1.))
                            .font_weight(FontWeight::BOLD)
                            .child("Contrôle à distance"),
                    )
                    .child(if live > 0 {
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.375))
                            .text_size(rems(0.75))
                            .text_color(theme::secondary(cx))
                            .child(live_dot(cx))
                            .child(status.who())
                            .when_some(status.level(), |row, level| {
                                row.child(format!(" · {level}"))
                            })
                            .into_any_element()
                    } else {
                        hint(
                            "Confiez ce lecteur à quelqu'un : il le pilote depuis son \
                             téléphone, avec la même app.",
                            cx,
                        )
                        .into_any_element()
                    }),
            ),
    );
    if let Some(cause) = &status.error {
        card = card.child(failed("remote-host-error", cause.clone(), cx));
    }
    card.into_any_element()
}

/// The server's own causes on a create (`remote.py#remote_link_create`).
fn create_cause(error: &nz_api::ApiError) -> String {
    match error {
        nz_api::ApiError::Http { status: 400, .. } => "Niveau ou durée invalide.".to_owned(),
        nz_api::ApiError::Http { status: 403, .. } => {
            "Seul un administrateur peut prêter l'administration.".to_owned()
        }
        nz_api::ApiError::Http { status: 409, .. } => {
            "Trop de liens actifs : coupez-en un d'abord.".to_owned()
        }
        nz_api::ApiError::Offline => "Serveur injoignable : le lien n'a pas été créé.".to_owned(),
        other => other.cause_fr(),
    }
}

/// What a claim that failed says, in that person's words.
fn claim_cause(error: &nz_api::ApiError) -> String {
    match error {
        nz_api::ApiError::Http { status: 404, .. } => {
            "Ce lien n'est plus valide : il a été coupé ou il a expiré.".to_owned()
        }
        nz_api::ApiError::Http { status: 429, .. } => {
            "Trop de tentatives : réessayez dans quelques minutes.".to_owned()
        }
        nz_api::ApiError::Offline => {
            "Serveur injoignable : réessayez quand le réseau revient.".to_owned()
        }
        other => other.cause_fr(),
    }
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_fields(window, cx);
        if self.loan().is_some() {
            return self.controller_view(window, cx);
        }
        self.owner_view(window, cx)
    }
}

impl std::fmt::Debug for Screen {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("remote::Screen")
            .field("route", &self.route)
            .field("links", &self.links.len())
            .field("loan", &self.loan().is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_durations_are_the_sheets_own() {
        assert_eq!(DURATIONS[0], (Some(3600), "1 heure"));
        assert_eq!(DURATIONS[1], (Some(86_400), "1 jour"));
        assert_eq!(DURATIONS[2], (Some(7 * 86_400), "7 jours"));
        assert_eq!(DURATIONS[3], (None, "Sans limite"));
    }

    #[test]
    fn every_level_of_the_sheet_has_a_glyph_and_a_sentence() {
        // A level with no icon renders NOTHING and keeps its hit box (D7), and a
        // level with no blurb offers a choice with no words.
        for level in ["queue", "read", "full", "admin"] {
            assert!(!remote_host::level_label(level).is_empty(), "{level}");
            assert!(!remote_host::level_blurb(level).is_empty(), "{level}");
        }
        for icon in [
            IconName::Cast,
            IconName::ListMusic,
            IconName::Eye,
            IconName::SlidersVertical,
            IconName::Shield,
        ] {
            assert!(
                crate::SHELL_ICONS.contains(&icon),
                "{icon:?} is drawn by D15a but missing from ShellAssets"
            );
        }
    }

    #[test]
    fn every_glyph_this_screen_draws_is_in_the_shell_bundle() {
        for icon in [
            IconName::Cast,
            IconName::Check,
            IconName::CircleAlert,
            IconName::Close,
            IconName::Copy,
            IconName::Eye,
            IconName::House,
            IconName::Link,
            IconName::ListMusic,
            IconName::LoaderCircle,
            IconName::LogOut,
            IconName::Monitor,
            IconName::Pause,
            IconName::Play,
            IconName::RefreshCw,
            IconName::Shield,
            IconName::SkipBack,
            IconName::SkipForward,
            IconName::SlidersVertical,
        ] {
            assert!(
                crate::SHELL_ICONS.contains(&icon),
                "{icon:?} is drawn by D15a but missing from ShellAssets"
            );
        }
    }
}
