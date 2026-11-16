//! The sheets the library and playlist screens open: a shared frame, the
//! collaborators, the export panel and the little prompts a playlist needs.
//!
//! A sheet is an entity with its own network work, exactly like a screen: it
//! owns what it loads, names its own failures in French, and tells the screen
//! what changed by EMITTING it (the screen subscribes and reapplies the
//! answer). Two rules this file exists to keep:
//!
//! - **Every mutation is optimistic and carries the version** the page had
//!   (`playlist_members.py`, `playlist_share.py`). A `409` re-reads once and
//!   says who moved the list; a `403` re-reads what the caller may still do and
//!   says so. Nothing is ever replayed: a batch that half landed is re-read,
//!   not re-sent.
//! - **A button never does nothing.** The export panel cannot withhold a ZIP
//!   from the server without a byte-level GET (`nz-api` has none — it builds
//!   URLs for a download manager), so its two file tabs hand the person the
//!   exact URL instead of pretending to download, and the panel says it.
//!
//! Nothing here draws a glyph the icon bundle does not register (`main.rs`): a
//! missing glyph renders nothing at all and keeps its hit box, which D7
//! measured on the first real run.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Div, ElementId, Entity, EventEmitter, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Window, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon, icon_button, input};
use serde_json::Value;

use nz_api::{ApiError, Lane};
use nz_model::{ExportFormat, PlaylistMember, PlaylistShareLink, Service};

use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;

/// What a sheet tells its screen.
#[derive(Clone, Debug, PartialEq)]
pub enum SheetEvent {
    /// The person asked to close it (its own button, the veil, or Échap).
    Close,
    /// The members or the caller's rights changed: the page adopts this.
    Members(MembersAnswer),
    /// Tracks landed in a playlist: `playlist` is the id, `count` how many.
    Added { playlist: String, count: usize },
}

/// Run `future` on the app's network runtime and hand its answer back on the
/// GPUI thread. One path for every request a sheet makes, so no caller invents
/// a second way to touch state.
fn spawn_answer<S, T, F, G>(cx: &mut Context<S>, state: &Entity<AppState>, future: F, done: G)
where
    S: Render + 'static,
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
    G: FnOnce(&mut S, T, &mut Context<S>) + 'static,
{
    let runtime = state.read(cx).runtime.clone();
    let task = runtime.spawn(future);
    cx.spawn(async move |this, cx| {
        if let Ok(answer) = task.await {
            this.update(cx, |this, cx| done(this, answer, cx)).ok();
        }
    })
    .detach();
}

/// Put a message in the shell's toast stack from a sheet (a sheet never owns
/// one: the stack belongs to the app).
pub fn push_toast(
    cx: &mut App,
    state: &Entity<AppState>,
    kind: ToastKind,
    message: impl Into<SharedString>,
) {
    let message = message.into();
    state.update(cx, |state, cx| {
        state.toasts.push(kind, message);
        cx.notify();
    });
}

/// The full-window veil a sheet sits on. A press outside the card closes it;
/// the caller attaches that listener, because only it knows what closing means.
pub fn backdrop(id: impl Into<ElementId>, cx: &App) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::shell(cx).opacity(0.92))
        .occlude()
}

/// The card itself: a title, an optional line that says what it acts on, then
/// whatever the sheet puts in it. Width in rems so zoom keeps it proportional.
pub fn card(
    id: impl Into<ElementId>,
    title: &'static str,
    subtitle: Option<SharedString>,
    width: f32,
    cx: &App,
) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap(rems(0.875))
        .w(rems(width))
        .max_h(rems(34.))
        .p(rems(1.25))
        .border_1()
        .border_color(theme::border(cx))
        .bg(theme::chrome(cx))
        .text_color(theme::text(cx))
        .font_family(theme::font(cx))
        .child(
            div().flex().items_center().gap(rems(0.5)).child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(rems(0.9375))
                            .font_weight(FontWeight::BOLD)
                            .child(title),
                    )
                    .children(subtitle.map(|subtitle| {
                        div()
                            .text_size(rems(0.6875))
                            .text_color(theme::secondary(cx))
                            .child(subtitle)
                    })),
            ),
        )
        .child(div().h(px(1.)).w_full().bg(theme::border(cx)))
}

/// The small label above a field.
pub fn field_label(text: &'static str, cx: &App) -> Div {
    div()
        .text_size(rems(0.625))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::secondary(cx))
        .child(text)
}

/// A failure, where it happened: the cause in words, never a bare code.
pub fn error_line(message: &SharedString, cx: &App) -> Div {
    div()
        .flex()
        .items_start()
        .gap(rems(0.5))
        .p(rems(0.5))
        .border_1()
        .border_color(theme::danger(cx))
        .text_size(rems(0.6875))
        .text_color(theme::danger(cx))
        .child(icon(IconName::CircleAlert).size(rems(0.875)))
        .child(div().flex_1().child(message.clone()))
}

/// A quiet line of context.
pub fn note_line(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(rems(0.6875))
        .text_color(theme::secondary(cx))
        .child(text.into())
}

/// `roleLabel`: the roles the server knows, in the words the screen uses.
pub fn role_label(role: &str) -> &'static str {
    match role {
        "owner" => "Propriétaire",
        "editor" => "Peut modifier",
        "viewer" => "Lecture seule",
        "admin" => "Administrateur",
        _ => "Accès",
    }
}

/// The initials a member's disc carries (no avatar upload exists for a person).
pub fn initials(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        return String::new();
    }
    let mut letters: Vec<char> = name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .collect();
    if letters.is_empty() {
        letters = name.chars().take(1).collect();
    }
    letters
        .into_iter()
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

/// A coloured disc with the member's initials: the playlist header's face stack
/// and the members sheet draw the same thing, so it lives here. The tint is
/// derived from the name only, so the same person is the same colour on every
/// screen and after every restart.
pub fn face(name: &str, size: f32, cx: &App) -> Div {
    let hue = name.bytes().fold(0u32, |sum, byte| {
        sum.wrapping_mul(31).wrapping_add(byte as u32)
    });
    let tint = match hue % 4 {
        0 => theme::accent(cx),
        1 => theme::success(cx),
        2 => theme::warning(cx),
        _ => theme::danger(cx),
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .size(px(size))
        .bg(tint.opacity(0.22))
        .text_color(tint)
        .text_size(px(size * 0.4))
        .font_weight(FontWeight::BOLD)
        .child(initials(name))
}

/// Report one download's end into the sheet that started it: the running flag,
/// the failure's cause, or the file that landed.
fn follow_download(
    cx: &mut Context<ExportSheet>,
    state: &Entity<AppState>,
    task: tokio::task::JoinHandle<Result<nz_api::DownloadReport, ApiError>>,
    _cancel: Arc<AtomicBool>,
) {
    let state = state.clone();
    cx.spawn(async move |this, cx| {
        let Ok(result) = task.await else {
            return;
        };
        this.update(cx, |this, cx| {
            let Some(download) = &mut this.download else {
                return;
            };
            download.running = false;
            match result {
                Ok(report) => {
                    let name = report
                        .path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    push_toast(
                        cx,
                        &state,
                        ToastKind::Info,
                        format!("« {name} » enregistré ({}).", human_bytes(report.bytes)),
                    );
                }
                Err(error) => {
                    download.error = Some(error.cause_fr().into());
                }
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Whether an error is a version conflict — the list moved under us.
/// Whether an error is a version conflict — the list moved under us.
///
/// `nz-api` now tells the two `409`s apart (`ApiError::Conflict` vs
/// `ApiError::NotReady`, by the server's own `Retry-After`: only
/// `_not_archived` sets it), so a playlist's stale `version` is matched as what
/// it is and a stream still being produced is not mistaken for one. This
/// predicate used to read EVERY `409` as a conflict, which was the best a screen
/// could do before the client made the distinction.
pub fn is_conflict(error: &ApiError) -> bool {
    matches!(error, ApiError::Conflict { .. })
}

/// Whether the server refused the action for want of rights (`403`).
pub fn is_forbidden(error: &ApiError) -> bool {
    matches!(error, ApiError::Http { status: 403, .. })
}

/// Whether the thing the sheet talks about is gone (`404`).
pub fn is_gone(error: &ApiError) -> bool {
    matches!(error, ApiError::Http { status: 404, .. })
}

/// The members answer the screen keeps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MembersAnswer {
    pub members: Vec<PlaylistMember>,
    pub members_version: u32,
    pub role: Option<String>,
    pub can_manage: bool,
    pub can_edit: bool,
    pub admin: bool,
}

/// `/api/playlist/<id>/members`: who is on a playlist, their role, and the
/// invite that adds one. Painted from the page's own payload (no request when it
/// opens); every mutation carries `members_version` and a conflict makes ONE
/// re-read, never a merge and never a replay.
pub struct MembersSheet {
    state: Entity<AppState>,
    id: String,
    title: String,
    members: Vec<PlaylistMember>,
    version: u32,
    role: Option<String>,
    can_manage: bool,
    can_edit: bool,
    admin: bool,
    me: String,
    invite: Entity<InputState>,
    invite_role: &'static str,
    busy: bool,
    /// A successful invite empties the field on the next paint (setting an
    /// `InputState`'s value needs the window, which a task does not have).
    clear_invite: bool,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SheetEvent> for MembersSheet {}

impl MembersSheet {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: Entity<AppState>,
        id: String,
        title: String,
        answer: &MembersAnswer,
        me: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let invite = cx.new(|cx| InputState::new(window, cx).placeholder("nom d'utilisateur"));
        let subscription = cx.subscribe(&invite, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.invite(cx);
            }
        });
        let _ = window;
        Self {
            state,
            id,
            title,
            members: answer.members.clone(),
            version: answer.members_version,
            role: answer.role.clone(),
            can_manage: answer.can_manage,
            can_edit: answer.can_edit,
            admin: answer.admin,
            me,
            invite,
            invite_role: "editor",
            busy: false,
            clear_invite: false,
            error: None,
            _subscriptions: vec![subscription],
        }
    }

    /// What the screen re-reads after the sheet changed something.
    pub fn answer(&self) -> MembersAnswer {
        MembersAnswer {
            members: self.members.clone(),
            members_version: self.version,
            role: self.role.clone(),
            can_manage: self.can_manage,
            can_edit: self.can_edit,
            admin: self.admin,
        }
    }

    /// The owner is the first row and is never editable.
    fn owner(&self) -> Option<&PlaylistMember> {
        self.members
            .iter()
            .find(|member| member.role == "owner")
            .or_else(|| self.members.first())
    }

    fn adopt(&mut self, answer: MembersAnswer) {
        self.members = answer.members;
        self.version = answer.members_version;
        self.role = answer.role;
        self.can_manage = answer.can_manage;
        self.can_edit = answer.can_edit;
        self.admin = answer.admin;
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let id = self.id.clone();
        spawn_answer(
            cx,
            &self.state.clone(),
            async move { api.playlist_members(&id).await },
            |this, result, cx| match result {
                Ok(answer) => {
                    this.adopt(MembersAnswer {
                        members: answer.members,
                        members_version: answer.members_version,
                        role: answer.role,
                        can_manage: answer.can_manage,
                        can_edit: answer.can_edit,
                        admin: answer.admin,
                    });
                    cx.notify();
                }
                Err(error) => {
                    this.error = Some(error.cause_fr().into());
                    cx.notify();
                }
            },
        );
    }

    fn failed(&mut self, error: &ApiError, cx: &mut Context<Self>) {
        if is_conflict(error) {
            push_toast(
                cx,
                &self.state,
                ToastKind::Info,
                "La playlist a changé : liste rechargée.",
            );
            self.reload(cx);
            return;
        }
        if is_forbidden(error) {
            self.can_manage = false;
            self.can_edit = false;
            self.error = Some("Vous n'avez plus le droit de gérer cette playlist.".into());
            self.reload(cx);
            return;
        }
        if is_gone(error) {
            self.error = Some("Cette playlist ne vous est plus accessible.".into());
            cx.emit(SheetEvent::Close);
            return;
        }
        self.error = Some(error.cause_fr().into());
        cx.notify();
    }

    fn invite(&mut self, cx: &mut Context<Self>) {
        if self.busy || !self.can_manage {
            return;
        }
        let name = self.invite.read(cx).value().trim().to_owned();
        if name.is_empty() {
            self.error = Some("Indiquez un nom d'utilisateur.".into());
            cx.notify();
            return;
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.busy = true;
        self.error = None;
        let id = self.id.clone();
        let role = self.invite_role.to_owned();
        let version = self.version;
        spawn_answer(
            cx,
            &self.state.clone(),
            async move {
                api.invite_playlist_member(&id, &name, &role, Some(version))
                    .await
            },
            move |this, result, cx| {
                this.busy = false;
                match result {
                    Ok(_) => {
                        this.clear_invite = true;
                        this.reload(cx);
                    }
                    Err(error) => this.failed(&error, cx),
                }
                cx.notify();
            },
        );
    }

    fn set_role(&mut self, member: &PlaylistMember, role: &'static str, cx: &mut Context<Self>) {
        if self.busy || !self.can_manage || member.role == role {
            return;
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let id = self.id.clone();
        let user = member.id.as_str().to_owned();
        let version = self.version;
        let previous = member.role.clone();
        let target = member.id.clone();
        if let Some(row) = self.members.iter_mut().find(|row| row.id == target) {
            row.role = role.to_owned();
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        spawn_answer(
            cx,
            &self.state.clone(),
            async move {
                api.set_playlist_member_role(&id, &user, role, Some(version))
                    .await
            },
            move |this, result, cx| {
                this.busy = false;
                match result {
                    Ok(_) => this.reload(cx),
                    Err(error) => {
                        if let Some(row) = this.members.iter_mut().find(|row| row.id == target) {
                            row.role = previous.clone();
                        }
                        this.failed(&error, cx);
                    }
                }
                cx.notify();
            },
        );
    }

    fn remove(&mut self, member: &PlaylistMember, cx: &mut Context<Self>) {
        if self.busy || !self.can_manage {
            return;
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let id = self.id.clone();
        let user = member.id.as_str().to_owned();
        let version = self.version;
        let removed = member.clone();
        let at = self.members.iter().position(|row| row.id == removed.id);
        self.members.retain(|row| row.id != removed.id);
        self.busy = true;
        self.error = None;
        cx.notify();
        spawn_answer(
            cx,
            &self.state.clone(),
            async move { api.remove_playlist_member(&id, &user, Some(version)).await },
            move |this, result, cx| {
                this.busy = false;
                match result {
                    Ok(_) => this.reload(cx),
                    Err(error) => {
                        if let Some(at) = at {
                            let at = at.min(this.members.len());
                            this.members.insert(at, removed.clone());
                        }
                        this.failed(&error, cx);
                    }
                }
                cx.notify();
            },
        );
    }

    fn leave(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.can_manage {
            return;
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let id = self.id.clone();
        self.busy = true;
        cx.notify();
        spawn_answer(
            cx,
            &self.state.clone(),
            async move { api.leave_playlist(&id).await },
            move |this, result, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        push_toast(
                            cx,
                            &this.state,
                            ToastKind::Info,
                            "Vous avez quitté la playlist.",
                        );
                        cx.emit(SheetEvent::Close);
                    }
                    Err(error) if is_conflict(&error) => {
                        this.error =
                            Some("Le propriétaire ne peut pas quitter sa playlist.".into());
                        cx.notify();
                    }
                    Err(error) => this.failed(&error, cx),
                }
            },
        );
    }

    fn can_leave(&self) -> bool {
        matches!(self.role.as_deref(), Some("editor" | "viewer"))
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(SheetEvent::Close);
    }
}

impl Render for MembersSheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.clear_invite) {
            self.invite
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        let managed = self.can_manage;
        let owner = self.owner().cloned();
        let others: Vec<PlaylistMember> = self
            .members
            .iter()
            .filter(|member| Some(member.id.as_str()) != owner.as_ref().map(|o| o.id.as_str()))
            .cloned()
            .collect();
        let me = self.me.clone();
        let busy = self.busy;
        let error = self.error.clone();
        let role = self.role.clone();

        let mut body = div()
            .id("members-list")
            .flex()
            .flex_col()
            .gap(rems(0.25))
            .max_h(rems(13.))
            .overflow_y_scroll()
            .min_h_0();
        if let Some(owner) = &owner {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.625))
                    .px(rems(0.375))
                    .py(rems(0.375))
                    .child(face(&owner.name, 28., cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.375))
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(rems(0.75)).child(owner.name.clone()))
                            .children((owner.name == me).then(|| {
                                div()
                                    .text_size(rems(0.5625))
                                    .text_color(theme::accent(cx))
                                    .child("Vous")
                            })),
                    )
                    .child(
                        div()
                            .text_size(rems(0.625))
                            .text_color(theme::secondary(cx))
                            .child("Propriétaire"),
                    ),
            );
        }
        if others.is_empty() {
            body = body.child(note_line("Personne d'autre sur cette playlist.", cx));
        }
        for member in &others {
            let target = member.clone();
            let row_id = format!("member-{}", member.id.as_str());
            let mut row = div()
                .id(ElementId::Name(row_id.clone().into()))
                .flex()
                .items_center()
                .gap(rems(0.625))
                .px(rems(0.375))
                .py(rems(0.25))
                .child(face(&member.name, 28., cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.375))
                        .flex_1()
                        .min_w_0()
                        .child(div().text_size(rems(0.75)).child(member.name.clone()))
                        .children((member.name == me).then(|| {
                            div()
                                .text_size(rems(0.5625))
                                .text_color(theme::accent(cx))
                                .child("Vous")
                        })),
                );
            if managed && member.role != "owner" {
                let as_editor = target.clone();
                let as_viewer = target.clone();
                let remove = target.clone();
                row = row
                    .child(
                        button(
                            ElementId::Name(format!("{row_id}-editor").into()),
                            "Peut modifier",
                            if member.role == "editor" {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Secondary
                            },
                            cx,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_role(&as_editor, "editor", cx);
                        })),
                    )
                    .child(
                        button(
                            ElementId::Name(format!("{row_id}-viewer").into()),
                            "Lecture seule",
                            if member.role == "viewer" {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Secondary
                            },
                            cx,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_role(&as_viewer, "viewer", cx);
                        })),
                    )
                    .child(
                        icon_button(
                            ElementId::Name(format!("{row_id}-remove").into()),
                            IconName::Trash,
                            format!("Retirer {}", member.name),
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove(&remove, cx);
                        })),
                    );
            } else {
                row = row.child(
                    div()
                        .text_size(rems(0.625))
                        .text_color(theme::secondary(cx))
                        .child(role_label(&member.role)),
                );
            }
            body = body.child(row);
        }

        let mut footer = div().flex().items_center().gap(rems(0.375)).pt(rems(0.25));
        if self.can_leave() {
            footer = footer.child(
                button(
                    "members-leave",
                    "Quitter la playlist",
                    ButtonVariant::Danger,
                    cx,
                )
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.leave(cx))),
            );
        }
        footer = footer.child(div().flex_1()).child(
            button("members-close", "Fermer", ButtonVariant::Secondary, cx)
                .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
        );

        let mut sheet = card(
            "members-sheet",
            "Collaborateurs",
            Some(self.title.clone().into()),
            if managed { 30. } else { 26. },
            cx,
        )
        .child(body);

        if managed {
            sheet = sheet
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.375))
                        .child(field_label("Inviter un compte", cx))
                        .child(input("members-invite", &self.invite, window, cx)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.375))
                        .child(
                            button(
                                "members-role-editor",
                                "Peut modifier",
                                if self.invite_role == "editor" {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                },
                                cx,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.invite_role = "editor";
                                cx.notify();
                            })),
                        )
                        .child(
                            button(
                                "members-role-viewer",
                                "Lecture seule",
                                if self.invite_role == "viewer" {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                },
                                cx,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.invite_role = "viewer";
                                cx.notify();
                            })),
                        )
                        .child(div().flex_1())
                        .child(
                            button("members-invite-go", "Inviter", ButtonVariant::Outline, cx)
                                .child(icon(IconName::UserPlus).size(rems(0.75)))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.invite(cx))),
                        ),
                )
                .child(note_line(
                    "La personne doit déjà avoir un compte sur ce serveur.",
                    cx,
                ));
        } else if role.as_deref() == Some("viewer") {
            sheet = sheet.child(note_line(
                "Lecture seule : seul le propriétaire ajoute des personnes.",
                cx,
            ));
        }
        if let Some(error) = &error {
            sheet = sheet.child(error_line(error, cx));
        }
        sheet = sheet.child(footer);

        backdrop("members-backdrop", cx)
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close(cx)),
            )
            .child(
                div()
                    .id("members-plate")
                    .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation()
                    })
                    .child(sheet),
            )
    }
}

/// Which list a sheet exports, and the route each one takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    Favorites,
    Playlist,
    Album,
}

impl ExportKind {
    /// The wire name the export routes take.
    pub fn wire(self) -> &'static str {
        match self {
            ExportKind::Favorites => "favorites",
            ExportKind::Playlist => "playlist",
            ExportKind::Album => "album",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ExportKind::Favorites => "vos favoris",
            ExportKind::Playlist => "cette playlist",
            ExportKind::Album => "cet album",
        }
    }

    /// The id the route takes (`favorites` names the account as `me`).
    pub fn target(self, id: &str) -> &str {
        match self {
            ExportKind::Favorites => "me",
            _ => id,
        }
    }
}

/// How the export panel is acting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExportMode {
    Audio,
    #[default]
    List,
    Service,
    Link,
}

/// `/api/services/export/<job>`'s answer, as the sheet reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServiceJob {
    pub running: bool,
    pub stage: String,
    pub total: u32,
    pub done: u32,
    pub matched: u32,
    pub missing: Vec<(String, String)>,
    pub error: Option<String>,
    pub relink: bool,
}

impl ServiceJob {
    /// Read the job's JSON. Tolerant on purpose: a field this build does not
    /// know is ignored, and a missing one is not a failure (rule 7).
    pub fn parse(value: &Value) -> Self {
        let missing = value
            .get("missing")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .map(|row| {
                        (
                            row.get("artist")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            row.get("title")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            running: value
                .get("running")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            stage: value
                .get("stage")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            total: value.get("total").and_then(Value::as_u64).unwrap_or(0) as u32,
            done: value.get("done").and_then(Value::as_u64).unwrap_or(0) as u32,
            matched: value.get("matched").and_then(Value::as_u64).unwrap_or(0) as u32,
            missing,
            error: value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_owned),
            relink: value
                .get("relink")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }

    /// `"12 / 30 titres · 9 trouvés"`, or the stage while matching.
    pub fn progress_line(&self) -> String {
        match self.stage.as_str() {
            "match" | "matching" if self.total == 0 => "Recherche des titres…".to_owned(),
            _ if self.total > 0 => format!(
                "{} / {} titres · {} trouvés",
                self.done, self.total, self.matched
            ),
            _ => "Envoi en cours…".to_owned(),
        }
    }
}

/// One file being written to disk, and its progress.
///
/// The bytes are counted by the download's own callback (on the network thread)
/// into an `AtomicU64`, so the panel repaints what really arrived instead of a
/// guess; the cancel flag is read before each chunk by `Api::download_to`, which
/// then removes its partial file. Rule 2: what is running says so, and what
/// failed says why.
pub struct Download {
    path: PathBuf,
    written: Arc<AtomicU64>,
    length: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    running: bool,
    error: Option<SharedString>,
}

impl Download {
    /// `"12,3 Mo / 84,1 Mo"` (or just the bytes when the server sent no length).
    pub fn progress_line(&self) -> String {
        let written = human_bytes(self.written.load(Ordering::Relaxed));
        let length = self.length.load(Ordering::Relaxed);
        if length == 0 {
            format!("{written} reçus…")
        } else {
            format!("{written} / {}", human_bytes(length))
        }
    }

    /// `true` while the file is still being written.
    pub fn running(&self) -> bool {
        self.running
    }
}

/// `12,3 Mo` — the units a French panel prints.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["o", "ko", "Mo", "Go", "To"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} o")
    } else {
        format!("{value:.1} {}", UNITS[unit]).replace('.', ",")
    }
}

/// A file name that survives every filesystem: the title's letters and digits,
/// dashes for the rest, at most 80 characters.
pub fn safe_file_name(title: &str, extension: &str) -> String {
    let mut stem: String = title
        .chars()
        .take(80)
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                // A space, a dash, a slash, a quote: one dash, so the name is a
                // slug rather than a filesystem's problem.
                '-'
            }
        })
        .collect();
    while stem.contains("--") {
        stem = stem.replace("--", "-");
    }
    let stem = stem.trim_matches('-');
    let stem = if stem.is_empty() { "neezer" } else { stem };
    format!("{stem}.{extension}")
}

/// Exporter: an archive, a list file, another service, or the public
/// read-only link of a playlist. The two file tabs hand the person the exact
/// URL (the app has no byte-level GET to stream the archive itself), the link
/// tab mints and revokes the capability token, and the service tab follows a
/// job with its progress and its failures — rule 2, never a silent button.
pub struct ExportSheet {
    state: Entity<AppState>,
    kind: ExportKind,
    id: String,
    title: String,
    can_manage: bool,
    mode: ExportMode,
    formats: Option<Vec<ExportFormat>>,
    format: Option<String>,
    list_format: &'static str,
    link: Option<PlaylistShareLink>,
    link_loaded: bool,
    link_busy: bool,
    confirm_revoke: bool,
    services: Option<Vec<Service>>,
    service: Option<String>,
    name: Entity<InputState>,
    public: bool,
    job: Option<ServiceJob>,
    job_id: Option<String>,
    job_busy: bool,
    /// The file being written to disk right now, or the one just written.
    download: Option<Download>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SheetEvent> for ExportSheet {}

impl ExportSheet {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: Entity<AppState>,
        kind: ExportKind,
        id: String,
        title: String,
        can_manage: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).default_value(title.clone()));
        let mut this = Self {
            state,
            kind,
            id,
            title,
            can_manage,
            mode: ExportMode::List,
            formats: None,
            format: None,
            list_format: "csv",
            link: None,
            link_loaded: false,
            link_busy: false,
            confirm_revoke: false,
            services: None,
            service: None,
            name,
            public: false,
            job: None,
            job_id: None,
            job_busy: false,
            download: None,
            error: None,
            _subscriptions: Vec::new(),
        };
        this.load(cx);
        this
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let formats_api = api.clone();
        spawn_answer(
            cx,
            &self.state.clone(),
            async move { formats_api.export_formats().await },
            |this, result, cx| {
                match result {
                    Ok(answer) => {
                        this.format = answer
                            .formats
                            .iter()
                            .find(|format| format.id == answer.default)
                            .or_else(|| answer.formats.first())
                            .map(|format| format.id.clone());
                        this.formats = Some(answer.formats);
                    }
                    Err(error) => {
                        this.error = Some(error.cause_fr().into());
                    }
                }
                cx.notify();
            },
        );
        let services_api = api.clone();
        spawn_answer(
            cx,
            &self.state.clone(),
            async move { services_api.services().await },
            |this, result, cx| {
                match result {
                    Ok(answer) => {
                        this.service = answer
                            .services
                            .iter()
                            .find(|service| service.ready)
                            .map(|service| service.id.clone());
                        this.services = Some(answer.services);
                    }
                    Err(_) => this.services = Some(Vec::new()),
                }
                cx.notify();
            },
        );
        if self.kind == ExportKind::Playlist && self.can_manage {
            self.load_link(cx);
        }
    }

    fn load_link(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let id = self.id.clone();
        self.link_loaded = true;
        spawn_answer(
            cx,
            &self.state.clone(),
            async move { api.playlist_share(&id).await },
            |this, result, cx| {
                match result {
                    Ok(answer) => this.link = answer.link,
                    Err(error) => {
                        if !is_forbidden(&error) {
                            this.error = Some(error.cause_fr().into());
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    fn create_link(&mut self, cx: &mut Context<Self>) {
        if self.link_busy {
            return;
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let id = self.id.clone();
        self.link_busy = true;
        cx.notify();
        spawn_answer(
            cx,
            &self.state.clone(),
            async move { api.create_playlist_share(&id, None).await },
            |this, result, cx| {
                this.link_busy = false;
                match result {
                    Ok(answer) => {
                        this.link = answer.link;
                        push_toast(cx, &this.state, ToastKind::Info, "Lien public créé.");
                    }
                    Err(error) => this.fail(&error, cx),
                }
                cx.notify();
            },
        );
    }

    fn revoke_link(&mut self, cx: &mut Context<Self>) {
        if self.link_busy {
            return;
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let id = self.id.clone();
        self.link_busy = true;
        cx.notify();
        spawn_answer(
            cx,
            &self.state.clone(),
            async move { api.revoke_playlist_share(&id).await },
            |this, result, cx| {
                this.link_busy = false;
                this.confirm_revoke = false;
                match result {
                    Ok(()) => {
                        this.link = None;
                        push_toast(cx, &this.state, ToastKind::Info, "Lien révoqué.");
                    }
                    Err(error) => this.fail(&error, cx),
                }
                cx.notify();
            },
        );
    }

    /// The absolute URL of the public page: the capability IS the token, so it
    /// works in any browser with no session — that is what makes it shareable.
    fn public_url(&self, cx: &App) -> Option<String> {
        let link = self.link.as_ref()?;
        let api = self.state.read(cx).api()?;
        Some(format!("{}{}", api.base_url(), link.url))
    }

    fn copy(&mut self, text: String, what: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        push_toast(
            cx,
            &self.state,
            ToastKind::Info,
            format!("{what} copié — collez-le dans votre navigateur."),
        );
    }

    /// Where a file goes when the person asks for one: their download folder,
    /// or the app's data directory when the system names none. `None` means the
    /// app has nowhere to write, and the panel says so instead of failing later.
    fn download_dir(&self, cx: &App) -> PathBuf {
        // A test (or a development run) points its data directory at a temp
        // folder; nothing may write into the real Downloads then.
        if let Some(data) = self.state.read(cx).config.data_dir.clone() {
            return data.join("Exports");
        }
        if let Some(dirs) = directories::UserDirs::new()
            && let Some(downloads) = dirs.download_dir()
        {
            return downloads.to_path_buf();
        }
        std::env::temp_dir().join("neezer-exports")
    }

    /// « Télécharger » : the server streams the archive (or the list file) and
    /// the app writes it to the download folder, chunk by chunk — never in
    /// memory (rule 3) — on `Lane::Bulk` so a running download yields to what
    /// the person is listening to (rule 1). Progress and cause are on screen,
    /// and « Annuler » removes the partial file.
    fn start_download(&mut self, archive: bool, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let (url, extension) = if archive {
            let Some(format) = self.format.clone() else {
                self.error = Some("Choisissez un format d'abord.".into());
                cx.notify();
                return;
            };
            let extension = self
                .formats
                .as_ref()
                .and_then(|formats| formats.iter().find(|row| row.id == format))
                .map(|row| row.ext.clone())
                .unwrap_or_else(|| format.to_lowercase());
            (
                api.export_url(self.kind.wire(), self.kind.target(&self.id), &format),
                extension,
            )
        } else {
            let extension = match self.list_format {
                "xspf" => "xspf".to_owned(),
                "txt" => "txt".to_owned(),
                _ => "csv".to_owned(),
            };
            (
                api.list_file_url(
                    self.kind.wire(),
                    self.kind.target(&self.id),
                    self.list_format,
                ),
                extension,
            )
        };
        let dir = self.download_dir(cx);
        let path = dir.join(safe_file_name(&self.title, &extension));
        let written = Arc::new(AtomicU64::new(0));
        let length = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        self.download = Some(Download {
            path: path.clone(),
            written: Arc::clone(&written),
            length: Arc::clone(&length),
            cancel: Arc::clone(&cancel),
            running: true,
            error: None,
        });
        self.error = None;
        cx.notify();

        let runtime = self.state.read(cx).runtime.clone();
        let counter = Arc::clone(&written);
        let total = Arc::clone(&length);
        let flag = Arc::clone(&cancel);
        let task = runtime.spawn(async move {
            api.download_to(
                Lane::Bulk,
                &url.strip_prefix(&format!("{}/api", api.base_url()))
                    .map(str::to_owned)
                    .unwrap_or(url),
                &path,
                move |bytes, length_now| {
                    counter.store(bytes, Ordering::Relaxed);
                    if let Some(length_now) = length_now {
                        total.store(length_now, Ordering::Relaxed);
                    }
                },
                &flag,
            )
            .await
        });
        crate::widgets::sheets::follow_download(cx, &self.state, task, Arc::clone(&cancel));
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        let (Some(api), Some(service)) = (self.state.read(cx).api(), self.service.clone()) else {
            return;
        };
        if self.job_busy {
            return;
        }
        let kind = self.kind;
        let id = self.id.clone();
        let name = self.name.read(cx).value().trim().to_owned();
        let public = self.public;
        self.job_busy = true;
        self.error = None;
        cx.notify();
        spawn_answer(
            cx,
            &self.state.clone(),
            async move {
                api.export_to_service(&service, kind.wire(), kind.target(&id), &name, public)
                    .await
            },
            |this, result, cx| {
                match result {
                    Ok(answer) => {
                        let job = answer.get("job").and_then(Value::as_str).map(str::to_owned);
                        if let Some(job) = job {
                            this.job_id = Some(job.clone());
                            this.job = Some(ServiceJob {
                                running: true,
                                stage: "match".into(),
                                ..ServiceJob::default()
                            });
                            this.follow(job, cx);
                        } else {
                            this.error = Some("Le serveur n'a pas nommé l'export.".into());
                        }
                    }
                    Err(error) => this.fail(&error, cx),
                }
                cx.notify();
            },
        );
    }

    /// Poll the export job until it ends, once every 800 ms, printing what the
    /// server says it is doing (`stage`, `done`/`total`, `matched`) and every
    /// title it could not place. Never a spinner with no sentence.
    fn follow(&mut self, job: String, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        spawn_answer(
            cx,
            &self.state.clone(),
            async move { api.service_export_job(&job).await },
            |this, result, cx| {
                match result {
                    Ok(answer) => {
                        let parsed = ServiceJob::parse(&answer);
                        let running = parsed.running;
                        this.job = Some(parsed);
                        if running {
                            cx.spawn(async move |this, cx| {
                                cx.background_executor()
                                    .timer(std::time::Duration::from_millis(800))
                                    .await;
                                this.update(cx, |this, cx| this.resume(cx)).ok();
                            })
                            .detach();
                        } else {
                            this.job_busy = false;
                            this.refresh_after_job(cx);
                        }
                    }
                    Err(error) => {
                        // A missed poll is not a failed export: say the cause
                        // once and stop asking.
                        this.job_busy = false;
                        this.fail(&error, cx);
                    }
                }
                cx.notify();
            },
        );
    }

    /// Re-arm the poll of the job currently on screen.
    fn resume(&mut self, cx: &mut Context<Self>) {
        if let Some(job) = self.job_id.clone() {
            self.follow(job, cx);
        }
    }

    /// What the end of a service export says: what it placed, what it could
    /// not, and the one gesture that fixes a lost credential.
    fn refresh_after_job(&mut self, cx: &mut Context<Self>) {
        let Some(job) = self.job.clone() else {
            return;
        };
        if let Some(error) = &job.error {
            self.error = Some(error.clone().into());
        } else if job.relink {
            push_toast(
                cx,
                &self.state,
                ToastKind::Error,
                "Le compte du service doit être reconnecté : Réglages → Compte.",
            );
        } else if job.missing.is_empty() {
            push_toast(
                cx,
                &self.state,
                ToastKind::Info,
                format!("Export terminé : {} titres placés.", job.matched),
            );
        } else {
            push_toast(
                cx,
                &self.state,
                ToastKind::Error,
                format!(
                    "Export terminé : {} placés, {} introuvables (listés ci-dessous).",
                    job.matched,
                    job.missing.len()
                ),
            );
        }
        cx.notify();
    }

    fn fail(&mut self, error: &ApiError, cx: &mut Context<Self>) {
        if is_forbidden(error) {
            self.error = Some("Vous ne pouvez pas exporter cette liste.".into());
        } else if is_gone(error) {
            self.error = Some("Cette liste ne vous est plus accessible.".into());
        } else {
            self.error = Some(error.cause_fr().into());
        }
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(SheetEvent::Close);
    }
}

impl Render for ExportSheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ready_services: Vec<Service> = self
            .services
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|service| service.ready)
            .collect();
        let linkable = self.kind == ExportKind::Playlist && self.can_manage;
        let error = self.error.clone();
        let busy = self.link_busy || self.job_busy;

        // The tabs: only the ones this list can actually use. A tab that would
        // answer "impossible" is worse than no tab.
        let mut tabs = div().flex().items_center().gap(rems(0.25));
        for (mode, label) in [
            (ExportMode::Audio, "Audio (ZIP)"),
            (ExportMode::List, "Liste"),
            (ExportMode::Service, "Service"),
            (ExportMode::Link, "Lien public"),
        ] {
            if mode == ExportMode::Link && !linkable {
                continue;
            }
            if mode == ExportMode::Service && self.services.as_ref().is_some_and(Vec::is_empty) {
                continue;
            }
            tabs = tabs.child(
                button(
                    ElementId::Name(format!("export-tab-{label}").into()),
                    label,
                    if self.mode == mode {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    },
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.mode = mode;
                    cx.notify();
                })),
            );
        }

        let mut body = div()
            .id("export-body")
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .max_h(rems(20.))
            .overflow_y_scroll()
            .min_h_0();

        match self.mode {
            ExportMode::Audio => {
                match &self.formats {
                    None => body = body.child(note_line("Chargement des formats…", cx)),
                    Some(formats) if formats.is_empty() => {
                        body = body.child(note_line(
                            "Aucun format d'export disponible sur ce serveur.",
                            cx,
                        ));
                    }
                    Some(formats) => {
                        let formats = formats.clone();
                        let mut list = div().flex().flex_wrap().gap(rems(0.375));
                        for format in formats {
                            let id = format.id.clone();
                            list = list.child(
                                button(
                                    ElementId::Name(format!("export-format-{}", format.id).into()),
                                    format!("{} .{}", format.label, format.ext),
                                    if self.format.as_deref() == Some(format.id.as_str()) {
                                        ButtonVariant::Primary
                                    } else {
                                        ButtonVariant::Secondary
                                    },
                                    cx,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.format = Some(id.clone());
                                        cx.notify();
                                    },
                                )),
                            );
                        }
                        body = body.child(list).child(
                            div()
                                .flex()
                                .items_center()
                                .gap(rems(0.375))
                                .child(
                                    button(
                                        "export-download-archive",
                                        "Télécharger",
                                        ButtonVariant::Primary,
                                        cx,
                                    )
                                    .child(icon(IconName::Download).size(rems(0.75)))
                                    .disabled(busy)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.start_download(true, cx)),
                                    ),
                                )
                                .child(
                                    button(
                                        "export-copy-archive",
                                        "Copier le lien",
                                        ButtonVariant::Secondary,
                                        cx,
                                    )
                                    .child(icon(IconName::Copy).size(rems(0.75)))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.start_download(true, cx)),
                                    ),
                                ),
                        );
                    }
                }
                body = body.child(note_line(
                    "L'archive est fabriquée par le serveur, titre par titre. Le lien s'ouvre dans \
                     votre navigateur, où votre session Neezer suffit.",
                    cx,
                ));
            }
            ExportMode::List => {
                let mut list = div().flex().items_center().gap(rems(0.375));
                for (id, label) in [
                    ("csv", "CSV (TuneMyMusic, Soundiiz…)"),
                    ("xspf", "XSPF (VLC, lecteurs)"),
                    ("txt", "Texte « Artiste - Titre »"),
                ] {
                    list = list.child(
                        button(
                            ElementId::Name(format!("export-list-{id}").into()),
                            label,
                            if self.list_format == id {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Secondary
                            },
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.list_format = id;
                            cx.notify();
                        })),
                    );
                }
                body = body
                    .child(list)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.375))
                            .child(
                                div()
                                    .debug_selector(|| "export-download-list".to_owned())
                                    .child(
                                        button(
                                            "export-download-list",
                                            "Télécharger",
                                            ButtonVariant::Primary,
                                            cx,
                                        )
                                        .child(icon(IconName::Download).size(rems(0.75)))
                                        .disabled(busy)
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.start_download(false, cx)
                                            }),
                                        ),
                                    ),
                            )
                            .child(
                                button(
                                    "export-copy-list",
                                    "Copier le lien",
                                    ButtonVariant::Secondary,
                                    cx,
                                )
                                .child(icon(IconName::Copy).size(rems(0.75)))
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.start_download(false, cx)),
                                ),
                            ),
                    )
                    .child(note_line(
                        "Une liste de titres à importer ailleurs, sans audio.",
                        cx,
                    ));
            }
            ExportMode::Service => {
                if ready_services.is_empty() {
                    body = body.child(note_line(
                        "Aucun service relié. Reliez un compte dans Réglages → Compte.",
                        cx,
                    ));
                } else {
                    let mut list = div().flex().flex_wrap().gap(rems(0.375));
                    for service in &ready_services {
                        let id = service.id.clone();
                        let account = service
                            .account
                            .clone()
                            .unwrap_or_else(|| service.name.clone());
                        list = list.child(
                            button(
                                ElementId::Name(format!("export-service-{}", service.id).into()),
                                account,
                                if self.service.as_deref() == Some(service.id.as_str()) {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                },
                                cx,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.service = Some(id.clone());
                                    cx.notify();
                                },
                            )),
                        );
                    }
                    body = body
                        .child(list)
                        .child(field_label("Nom là-bas", cx))
                        .child(input("export-name", &self.name, window, cx))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(rems(0.5))
                                .child(
                                    button(
                                        "export-public",
                                        if self.public { "Publique" } else { "Privée" },
                                        ButtonVariant::Secondary,
                                        cx,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.public = !this.public;
                                            cx.notify();
                                        },
                                    )),
                                )
                                .child(
                                    button("export-send", "Envoyer", ButtonVariant::Outline, cx)
                                        .disabled(busy)
                                        .on_click(cx.listener(|this, _, _, cx| this.send(cx))),
                                ),
                        );
                }
                if let Some(job) = &self.job {
                    body = body.child(note_line(job.progress_line(), cx));
                    if let Some(error) = &job.error {
                        body = body.child(error_line(&SharedString::from(error.clone()), cx));
                    }
                    for (artist, title) in job.missing.iter().take(8) {
                        body = body.child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::secondary(cx))
                                .child(format!("Introuvable : {artist} — {title}")),
                        );
                    }
                }
            }
            ExportMode::Link => {
                if !self.link_loaded {
                    body = body.child(note_line("Lecture du lien public…", cx));
                } else if let Some(url) = self.public_url(cx) {
                    let expiry = self
                        .link
                        .as_ref()
                        .and_then(|link| link.expires.clone())
                        .map(|expires| format!("Expire le {expires}"))
                        .unwrap_or_else(|| "Sans expiration".to_owned());
                    let copy = url.clone();
                    body = body
                        .child(
                            div()
                                .p(rems(0.5))
                                .border_1()
                                .border_color(theme::border(cx))
                                .text_size(rems(0.6875))
                                .child(url),
                        )
                        .child(note_line(expiry, cx))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(rems(0.375))
                                .child(
                                    button("export-link-copy", "Copier", ButtonVariant::Outline, cx)
                                        .child(icon(IconName::Copy).size(rems(0.75)))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.copy(copy.clone(), "Lien public", cx)
                                        })),
                                )
                                .child(if self.confirm_revoke {
                                    button(
                                        "export-link-revoke",
                                        "Confirmer la révocation",
                                        ButtonVariant::Danger,
                                        cx,
                                    )
                                    .child(icon(IconName::Trash).size(rems(0.75)))
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, _, cx| this.revoke_link(cx)))
                                } else {
                                    button(
                                        "export-link-ask-revoke",
                                        "Révoquer",
                                        ButtonVariant::Secondary,
                                        cx,
                                    )
                                    .child(icon(IconName::Trash).size(rems(0.75)))
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.confirm_revoke = true;
                                        cx.notify();
                                    }))
                                }),
                        )
                        .child(note_line(
                            "Toute personne qui a ce lien lit la playlist et l'écoute, sans compte. \
                             Révoquer le lien le tue immédiatement.",
                            cx,
                        ));
                } else {
                    body = body
                        .child(note_line(
                            "Aucun lien public : la playlist reste privée.",
                            cx,
                        ))
                        .child(
                            button(
                                "export-link-create",
                                "Créer un lien public",
                                ButtonVariant::Outline,
                                cx,
                            )
                            .child(icon(IconName::Link2).size(rems(0.75)))
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.create_link(cx))),
                        )
                        .child(note_line(
                            "Le lien ouvre une page en lecture seule, sans compte : idéal pour \
                             partager une sélection.",
                            cx,
                        ));
                }
            }
        }

        if let Some(download) = &self.download {
            let mut row = div()
                .id("export-download")
                .flex()
                .items_center()
                .gap(rems(0.5))
                .px(rems(0.5))
                .py(rems(0.25))
                .border_1()
                .border_color(theme::border(cx))
                .child(
                    icon(if download.running {
                        IconName::LoaderCircle
                    } else {
                        IconName::CircleCheck
                    })
                    .size(rems(0.875))
                    .text_color(if download.running {
                        theme::secondary(cx)
                    } else {
                        theme::success(cx)
                    }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(rems(0.6875))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(
                                    download
                                        .path
                                        .file_name()
                                        .map(|name| name.to_string_lossy().into_owned())
                                        .unwrap_or_default(),
                                ),
                        )
                        .child(
                            div()
                                .text_size(rems(0.625))
                                .text_color(theme::secondary(cx))
                                .child(download.progress_line()),
                        ),
                );
            if download.running {
                let cancel = Arc::clone(&download.cancel);
                row = row.child(
                    button(
                        "export-download-cancel",
                        "Annuler",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cancel.store(true, Ordering::Relaxed);
                        this.error = Some("Téléchargement annulé.".into());
                        cx.notify();
                    })),
                );
            } else if let Some(cause) = &download.error {
                row = row.child(
                    div()
                        .text_size(rems(0.625))
                        .text_color(theme::danger(cx))
                        .child(cause.clone()),
                );
            }
            body = body.child(row);
        }
        if let Some(error) = &error {
            body = body.child(error_line(error, cx));
        }

        let footer = div()
            .flex()
            .items_center()
            .gap(rems(0.375))
            .child(div().flex_1())
            .child(
                div().debug_selector(|| "export-close".to_owned()).child(
                    button("export-close", "Fermer", ButtonVariant::Secondary, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                ),
            );

        let sheet = card(
            "export-sheet",
            "Exporter",
            Some(format!("{} — « {} »", self.kind.label(), self.title).into()),
            32.,
            cx,
        )
        .child(tabs)
        .child(body)
        .child(footer);

        backdrop("export-backdrop", cx)
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close(cx)),
            )
            .child(
                div()
                    .id("export-plate")
                    .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation()
                    })
                    .child(sheet),
            )
    }
}

impl std::fmt::Debug for ExportSheet {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExportSheet")
            .field("kind", &self.kind)
            .field("id", &self.id)
            .field("mode", &self.mode)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every glyph this lot draws is in the asset bundle `main.rs` registers.
    /// An icon outside it renders NOTHING and keeps its hit box (D7 measured a
    /// player bar with only Play and a sidebar without Bibliothèque), so a
    /// button would be there, invisible. This test is the list's other half.
    #[test]
    fn every_glyph_this_lot_draws_is_in_the_shell_bundle() {
        for icon in [
            IconName::AudioLines,
            IconName::CircleAlert,
            IconName::Close,
            IconName::ExternalLink,
            IconName::LibraryBig,
            IconName::ListMusic,
            IconName::LoaderCircle,
            IconName::Minus,
            IconName::Music,
            IconName::Play,
            IconName::Plus,
            IconName::Search,
        ] {
            assert!(
                crate::SHELL_ICONS.contains(&icon),
                "{icon:?} is drawn by D8b but missing from ShellAssets"
            );
        }
    }

    /// The member disc keeps two letters and never a 3-byte mess.
    #[test]
    fn an_initials_disc_has_one_or_two_letters() {
        assert_eq!(initials("Kaya Project"), "KP");
        assert_eq!(initials("bench"), "B");
        assert_eq!(initials("  "), "");
        assert_eq!(initials("éloïse"), "É");
    }

    /// The service job's JSON is read tolerantly: a key this build does not know
    /// is ignored and a missing one is not a failure (rule 7).
    #[test]
    fn a_service_job_reads_what_it_knows_and_ignores_the_rest() {
        let value: serde_json::Value = serde_json::from_str(
            r#"{"running":true,"stage":"match","total":10,"done":3,"matched":2,
                "missing":[{"artist":"Kaya","title":"Été"}],"unknown":42}"#,
        )
        .expect("job");
        let job = ServiceJob::parse(&value);
        assert!(job.running);
        assert_eq!(job.progress_line(), "3 / 10 titres · 2 trouvés");
        assert_eq!(job.missing, [("Kaya".to_owned(), "Été".to_owned())]);
        assert!(
            ServiceJob::parse(&serde_json::json!({}))
                .progress_line()
                .contains("Envoi")
        );
    }

    /// The export routes name the account differently from every other kind.
    #[test]
    fn the_favourites_export_names_the_account_as_me() {
        assert_eq!(ExportKind::Favorites.wire(), "favorites");
        assert_eq!(ExportKind::Favorites.target("ignored"), "me");
        assert_eq!(ExportKind::Playlist.wire(), "playlist");
        assert_eq!(ExportKind::Playlist.target("pl-1"), "pl-1");
    }
}
