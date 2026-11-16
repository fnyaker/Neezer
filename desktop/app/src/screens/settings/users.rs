//! Réglages → Utilisateurs: the accounts, their role, their state, and the
//! invitations that create the next ones.
//!
//! Lot D14b. The SPA's reference is `AdminUsers.svelte` and its pure helpers
//! (`lib/adminusers.js`), ported here: the same 25-row pages with the same 250
//! row cap, the same six filters, the same invitation link built from the
//! server's own origin, and the same rule that matters most — **an invitation's
//! token is shown once**, right after it is minted, and never again.
//!
//! Everything is `@admin_required` on the server (`admin_users.py`,
//! `invites.py`) and the server stays the enforcement; a refusal is shown with
//! the server's own code translated (`guard_fr`), never as a bare status.
//!
//! Dates are printed absolutely rather than as "il y a 3 h": the payloads carry
//! naive server-local timestamps and this workspace has no timezone crate, so an
//! absolute date is exact where a relative one would be off by the offset (see
//! the note's D14b paragraph).

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Div, Entity, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, Subscription, Window, div, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, focus_scope, icon, input};
use nz_api::{Api, ApiError};
use nz_model::{AdminInvite, AdminUser};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::nav::SettingsSection;
use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;

use super::server::{Inbox, date_fr, datetime_fr};

/// `lib/adminusers.js#PAGE_SIZE`.
const PAGE_SIZE: u32 = 25;
/// `lib/adminusers.js#MAX_ROWS`: the SPA stops appending there.
const MAX_ROWS: usize = 250;
/// The SPA's search debounce (250 ms, `AdminUsers.svelte`).
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);
/// The invitation note's cap (`invites.NOTE_MAX`).
const NOTE_MAX: usize = 64;

// ---------------------------------------------------------------------------
// The pure half, ported from `webapp/src/lib/adminusers.js`.
// ---------------------------------------------------------------------------

/// The six filters, in the SPA's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Filter {
    All,
    Active,
    Invited,
    Admins,
    Members,
    Disabled,
}

impl Filter {
    const ALL: [Filter; 6] = [
        Filter::All,
        Filter::Active,
        Filter::Invited,
        Filter::Admins,
        Filter::Members,
        Filter::Disabled,
    ];

    fn id(self) -> &'static str {
        match self {
            Filter::All => "all",
            Filter::Active => "active",
            Filter::Invited => "invited",
            Filter::Admins => "admins",
            Filter::Members => "members",
            Filter::Disabled => "disabled",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Filter::All => "Tous",
            Filter::Active => "Actifs",
            Filter::Invited => "Invités",
            Filter::Admins => "Administrateurs",
            Filter::Members => "Membres",
            Filter::Disabled => "Désactivés",
        }
    }
}

/// `lib/adminusers.js#foldText`, without a Unicode-normalisation crate: the
/// French letters the server's own fold covers, mapped to ASCII.
fn fold_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.trim().chars() {
        let folded = match character {
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' | 'À' | 'Á' | 'Â' | 'Ä' | 'Ã' | 'Å' => {
                'a'
            }
            'ç' | 'Ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => 'i',
            'ñ' | 'Ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' | 'Ò' | 'Ó' | 'Ô' | 'Ö' | 'Õ' => 'o',
            'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => 'u',
            'ý' | 'ÿ' | 'Ý' => 'y',
            'œ' | 'Œ' => 'o',
            'æ' | 'Æ' => 'a',
            other => other,
        };
        for lower in folded.to_lowercase() {
            out.push(lower);
        }
    }
    out
}

/// `lib/adminusers.js#countLabel`.
fn count_label(count: usize, singular: &str) -> String {
    format!(
        "{count} {}",
        if count > 1 {
            format!("{singular}s")
        } else {
            singular.to_owned()
        }
    )
}

/// The state of an invitation, from the server's own `state`.
fn invite_state(invite: &AdminInvite) -> (&'static str, &'static str) {
    match invite.state.as_str() {
        "used" => ("used", "Utilisée"),
        "revoked" => ("revoked", "Révoquée"),
        "expired" => ("expired", "Expirée"),
        _ => ("invite", "Invité"),
    }
}

/// `lib/adminusers.js#roleLabel`.
fn role_label(user: &AdminUser) -> &'static str {
    if user.admin {
        "Administrateur"
    } else {
        "Membre"
    }
}

/// The invitation's own line in the list.
fn invite_line(invite: &AdminInvite) -> String {
    match (&invite.expires, invite.state.as_str()) {
        (_, "used") => match (&invite.used_by, &invite.used_at) {
            (Some(name), Some(at)) => format!("Utilisée par {name} le {}", datetime_fr(at)),
            (Some(name), None) => format!("Utilisée par {name}"),
            _ => "Utilisée".to_owned(),
        },
        (_, "revoked" | "expired") => format!(
            "Expirée le {}",
            date_fr(invite.expires.as_deref().unwrap_or(""))
        ),
        (Some(expires), _) => format!("Expire le {}", date_fr(expires)),
        (None, _) => "Sans date d'expiration".to_owned(),
    }
}

/// The account's own line: when it was last seen, absolutely.
fn user_line(user: &AdminUser) -> String {
    match &user.last_login {
        Some(at) => format!("Vu le {}", datetime_fr(at)),
        None => "Jamais connecté".to_owned(),
    }
}

/// The SPA's `inviteUrl`, against this app's own server.
fn invite_link(server: &str, token: &str) -> String {
    let base = server.trim_end_matches('/');
    format!("{base}/app/#/invite/{token}")
}

/// `lib/adminusers.js#REFUSALS`: the server's codes, in the SPA's words.
fn guard_fr(error: &ApiError) -> String {
    let code = match error {
        ApiError::Http {
            code: Some(code), ..
        } => Some(code.as_str()),
        _ => None,
    };
    if let Some(code) = code {
        let sentence = match code {
            "last_admin" => "Impossible : c'est le dernier administrateur actif.",
            "self_delete" => "Vous ne pouvez pas supprimer votre propre compte.",
            "self_disable" => "Vous ne pouvez pas désactiver votre propre compte.",
            "server_account_holder" => {
                "Ce compte porte le compte Deezer du serveur. Changez-en le titulaire avant de le supprimer."
            }
            "forbidden" => "Votre compte n'a plus les droits d'administration.",
            "not_found" => "Ce compte n'existe plus.",
            "invite_not_found" => "Cette invitation n'existe plus.",
            "invite_used" => "Cette invitation a déjà servi.",
            "invite_invalid" => "Cette invitation n'est plus valide.",
            "rate_limited" => "Trop de tentatives. Réessayez dans quelques minutes.",
            "invalid_note" => "La note doit faire 64 caractères au plus.",
            "invalid_expires_in" => "La durée doit être comprise entre 1 heure et 30 jours.",
            "too_many_workers" => "Trop d'appareils enrôlés.",
            "too_large" => "La demande est trop volumineuse.",
            "confirm_mismatch" => "Le nom tapé ne correspond pas.",
            _ => "",
        };
        if !sentence.is_empty() {
            return sentence.to_owned();
        }
    }
    match error {
        ApiError::Offline => "Le serveur ne répond pas (réseau).".to_owned(),
        ApiError::Unauthorized => "Session expirée : reconnectez-vous.".to_owned(),
        ApiError::Http { status: 403, .. } => {
            "Votre compte n'a plus les droits d'administration.".to_owned()
        }
        ApiError::Http { status: 404, .. } => "Cet élément n'existe plus.".to_owned(),
        ApiError::Http { status: 409, .. } => {
            "Cette action est impossible dans l'état actuel.".to_owned()
        }
        ApiError::Http { status: 429, .. } => {
            "Trop de tentatives. Réessayez dans quelques minutes.".to_owned()
        }
        other => other.cause_fr(),
    }
}

// ---------------------------------------------------------------------------
// Payloads the committed types do not cover.
// ---------------------------------------------------------------------------

/// `GET /api/admin/users/<id>`: the account plus what it owns.
#[derive(Debug, Clone, Default, Deserialize)]
struct UserDetail {
    #[serde(default)]
    user: AdminUser,
    #[serde(default)]
    uploads: u32,
    #[serde(default)]
    playlists: u32,
}

/// `POST /api/admin/invites` and `.../renew`: the row and the token, ONCE.
#[derive(Debug, Clone, Default, Deserialize)]
struct IssuedInvite {
    #[serde(default)]
    invite: AdminInvite,
    #[serde(default)]
    token: String,
}

/// What a background request reports back.
enum Msg {
    Users {
        result: Result<nz_model::AdminUsers, ApiError>,
        seq: u64,
        append: bool,
    },
    Invites(Result<nz_model::AdminInvites, ApiError>),
    Detail {
        id: String,
        result: Result<Value, ApiError>,
    },
    Patched {
        id: String,
        field: &'static str,
        previous: bool,
        result: Result<Value, ApiError>,
    },
    Deleted {
        id: String,
        result: Result<Value, ApiError>,
    },
    Issued {
        renew: Option<String>,
        result: Result<IssuedInvite, ApiError>,
    },
    Revoked {
        id: String,
        result: Result<Value, ApiError>,
    },
    /// The search debounce elapsed for `seq`; a newer keystroke wins.
    SearchDue(u64),
}

// ---------------------------------------------------------------------------
// The two sheets.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ttl {
    Hour,
    Day,
    Week,
    Month,
}

impl Ttl {
    const ALL: [Ttl; 4] = [Ttl::Hour, Ttl::Day, Ttl::Week, Ttl::Month];

    fn seconds(self) -> u64 {
        match self {
            Ttl::Hour => 3_600,
            Ttl::Day => 86_400,
            Ttl::Week => 604_800,
            Ttl::Month => 2_592_000,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Ttl::Hour => "1 heure",
            Ttl::Day => "24 heures",
            Ttl::Week => "7 jours",
            Ttl::Month => "30 jours",
        }
    }
}

/// The invitation sheet: the form, then the link shown once.
struct InviteSheet {
    /// The invitation being renewed, `None` for a new one.
    renew: Option<String>,
    note: String,
    ttl: Ttl,
    busy: bool,
    error: Option<SharedString>,
    /// The minted link and its token, shown ONCE.
    issued: Option<IssuedInvite>,
    link: String,
    copied: bool,
    confirm_close: bool,
    confirm_revoke: bool,
    revoking: bool,
}

impl InviteSheet {
    fn new(renew: Option<String>, note: String) -> Self {
        Self {
            renew,
            note,
            ttl: Ttl::Week,
            busy: false,
            error: None,
            issued: None,
            link: String::new(),
            copied: false,
            confirm_close: false,
            confirm_revoke: false,
            revoking: false,
        }
    }
}

/// The account sheet: what the account owns, and the three guarded actions.
struct UserSheet {
    id: String,
    name: String,
    admin: bool,
    disabled: bool,
    detail: Option<UserDetail>,
    busy: Option<&'static str>,
    error: Option<SharedString>,
    confirm_delete: bool,
    confirm_disable: bool,
}

/// The invitation sheet's own state.
struct Sheet {
    invite: Option<InviteSheet>,
    user: Option<UserSheet>,
}

impl Sheet {
    fn none() -> Self {
        Self {
            invite: None,
            user: None,
        }
    }

    fn is_open(&self) -> bool {
        self.invite.is_some() || self.user.is_some()
    }
}

// ---------------------------------------------------------------------------
// The screen.
// ---------------------------------------------------------------------------

/// Réglages → Utilisateurs.
pub struct Screen {
    section: SettingsSection,
    state: Entity<AppState>,
    tx: UnboundedSender<Msg>,
    subscriptions: Vec<Subscription>,
    root_focus: FocusHandle,
    scroll: gpui_kit::ScrollHandle,
    viewport: gpui_kit::Pixels,
    sheet_focus: FocusHandle,
    /// Who had the focus before a sheet opened; the shell's own anchor when the
    /// person came from outside the screen, restored when the sheet closes.
    previous_focus: Option<FocusHandle>,

    search: Option<Entity<InputState>>,
    term: String,
    seq: u64,

    users: Vec<AdminUser>,
    total: usize,
    next: Option<String>,
    loading: bool,
    loading_more: bool,
    error: Option<SharedString>,

    invites: Vec<AdminInvite>,
    invites_loaded: bool,
    invites_error: Option<SharedString>,

    filter: Filter,
    sheet: Sheet,
    note: Option<Entity<InputState>>,
    confirm_name: Option<Entity<InputState>>,

    /// Set while a mutation runs, so a double click cannot send it twice.
    mutating: Option<&'static str>,
}

impl Screen {
    /// The route table's constructor (`screens::settings::Screen::new`).
    pub fn new(section: SettingsSection, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let inbox = Inbox::new();
        let tx = inbox.sender();
        let root_focus = cx.focus_handle();
        let sheet_focus = cx.focus_handle();
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        let mut this = Self {
            section,
            state,
            tx,
            subscriptions: vec![subscription],
            root_focus,
            scroll: gpui_kit::ScrollHandle::new(),
            viewport: gpui_kit::px(800.),
            sheet_focus,
            previous_focus: None,
            search: None,
            term: String::new(),
            seq: 0,
            users: Vec::new(),
            total: 0,
            next: None,
            loading: false,
            loading_more: false,
            error: None,
            invites: Vec::new(),
            invites_loaded: false,
            invites_error: None,
            filter: Filter::All,
            sheet: Sheet::none(),
            note: None,
            confirm_name: None,
            mutating: None,
        };
        inbox.spawn::<Self>(cx, |this, message, cx| this.on_message(message, cx));
        super::server::install_scroll_keys(cx);
        super::server::bind_scroll_actions::<Self>(cx, |this, direction, cx| {
            let page = this.viewport * 0.8;
            this.scroll_by(page * direction, cx);
        });
        this.load_first(cx);
        this.load_invites(cx);
        this
    }

    /// The section this view draws (`Utilisateurs`); the tests read it.
    pub fn section(&self) -> SettingsSection {
        self.section
    }

    fn api(&self, cx: &App) -> Option<Arc<Api>> {
        self.state.read(cx).api.clone()
    }

    /// The first page of accounts, for `term`.
    fn load_first(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            self.error = Some("La session n'est pas ouverte : reconnectez-vous.".into());
            cx.notify();
            return;
        };
        self.seq += 1;
        let seq = self.seq;
        self.loading = self.users.is_empty();
        self.loading_more = !self.users.is_empty();
        self.error = None;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        let term = self.term.clone();
        runtime.spawn(async move {
            let result = api.admin_users(&term, None, PAGE_SIZE).await;
            let _ = tx.send(Msg::Users {
                result,
                seq,
                append: false,
            });
        });
        cx.notify();
    }

    /// The next page (the SPA's "Charger plus"), keyset cursor from `next`.
    fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.loading_more || self.loading {
            return;
        }
        let Some(after) = self.next.clone() else {
            return;
        };
        if self.users.len() >= MAX_ROWS {
            return;
        }
        let Some(api) = self.api(cx) else {
            return;
        };
        let seq = self.seq;
        self.loading_more = true;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        let term = self.term.clone();
        runtime.spawn(async move {
            let result = api.admin_users(&term, Some(&after), PAGE_SIZE).await;
            let _ = tx.send(Msg::Users {
                result,
                seq,
                append: true,
            });
        });
        cx.notify();
    }

    fn load_invites(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let _ = tx.send(Msg::Invites(api.admin_invites(None, 100, None, None).await));
        });
    }

    fn load_detail(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let result = api.admin_user(&id).await;
            let _ = tx.send(Msg::Detail { id, result });
        });
    }

    fn on_message(&mut self, message: Msg, cx: &mut Context<Self>) {
        match message {
            Msg::Users {
                result,
                seq,
                append,
            } => {
                if seq != self.seq {
                    return;
                }
                self.loading = false;
                self.loading_more = false;
                match result {
                    Ok(page) => {
                        if append {
                            let seen: std::collections::HashSet<String> =
                                self.users.iter().map(|user| user.id.to_string()).collect();
                            for user in page.users {
                                if !seen.contains(&user.id.to_string()) {
                                    self.users.push(user);
                                }
                            }
                        } else {
                            self.users = page.users;
                        }
                        self.total = page.total as usize;
                        self.next = page
                            .next
                            .as_ref()
                            .and_then(|value| value.as_str())
                            .map(str::to_owned);
                        self.error = None;
                    }
                    Err(error) => {
                        if matches!(error, ApiError::Http { status: 403, .. }) {
                            self.users.clear();
                            self.total = 0;
                            self.next = None;
                        }
                        self.error = Some(guard_fr(&error).into());
                    }
                }
            }
            Msg::Invites(result) => {
                self.invites_loaded = true;
                match result {
                    Ok(page) => {
                        self.invites = page.invites;
                        self.invites_error = None;
                    }
                    Err(error) => self.invites_error = Some(guard_fr(&error).into()),
                }
            }
            Msg::Detail { id, result } => match result {
                Ok(value) => {
                    if let Some(user) = self.sheet.user.as_mut()
                        && user.id == id
                    {
                        match serde_json::from_value::<UserDetail>(value) {
                            Ok(detail) => user.detail = Some(detail),
                            Err(error) => {
                                user.error =
                                    Some(format!("Réponse du serveur illisible : {error}").into());
                            }
                        }
                    }
                }
                Err(error) => {
                    if let Some(user) = self.sheet.user.as_mut() {
                        user.error = Some(guard_fr(&error).into());
                    }
                }
            },
            Msg::Patched {
                id,
                field,
                previous,
                result,
            } => {
                self.mutating = None;
                match result {
                    Ok(value) => {
                        let user = value.get("user").cloned().unwrap_or(Value::Null);
                        if let Ok(user) = serde_json::from_value::<AdminUser>(user) {
                            if let Some(row) = self.users.iter_mut().find(|row| row.id == user.id) {
                                *row = user.clone();
                            }
                            if let Some(sheet) = self.sheet.user.as_mut()
                                && sheet.id == id
                            {
                                sheet.admin = user.admin;
                                sheet.disabled = user.disabled;
                                sheet.busy = None;
                            }
                        }
                        self.toasts(
                            ToastKind::Info,
                            if field == "admin" {
                                "Rôle enregistré.".to_owned()
                            } else {
                                "État du compte enregistré.".to_owned()
                            },
                            cx,
                        );
                    }
                    Err(error) => {
                        // Roll the row back to what the server still holds.
                        if let Some(row) =
                            self.users.iter_mut().find(|row| row.id.to_string() == id)
                        {
                            if field == "admin" {
                                row.admin = previous;
                            } else {
                                row.disabled = previous;
                            }
                        }
                        if let Some(sheet) = self.sheet.user.as_mut() {
                            sheet.busy = None;
                            if field == "admin" {
                                sheet.admin = previous;
                            } else {
                                sheet.disabled = previous;
                            }
                            sheet.error = Some(guard_fr(&error).into());
                        }
                        self.toasts(ToastKind::Error, guard_fr(&error), cx);
                    }
                }
            }
            Msg::Deleted { id, result } => {
                self.mutating = None;
                match result {
                    Ok(value) => {
                        let uploads = value
                            .get("uploads_reassigned")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        let podcasts = value
                            .get("podcasts_reassigned")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        if let Some(sheet) = self.sheet.user.as_mut() {
                            sheet.busy = None;
                        }
                        self.sheet = Sheet::none();
                        self.users.retain(|user| user.id.to_string() != id);
                        self.total = self.total.saturating_sub(1);
                        self.toasts(
                            ToastKind::Info,
                            format!(
                                "Compte supprimé ; {uploads} fichier(s) et {podcasts} podcast(s) réattribués."
                            ), cx);
                    }
                    Err(error) => {
                        if let Some(sheet) = self.sheet.user.as_mut() {
                            sheet.busy = None;
                            sheet.error = Some(guard_fr(&error).into());
                        }
                        self.toasts(ToastKind::Error, guard_fr(&error), cx);
                    }
                }
            }
            Msg::Issued { renew, result } => {
                self.mutating = None;
                if let Some(sheet) = self.sheet.invite.as_mut() {
                    sheet.busy = false;
                }
                match result {
                    Ok(issued) => {
                        let server = self.state.read(cx).server.clone();
                        let link = invite_link(&server, &issued.token);
                        // The token exists exactly once: if the sheet was
                        // dismissed anyway, it comes back rather than being lost.
                        let sheet = self
                            .sheet
                            .invite
                            .get_or_insert_with(|| InviteSheet::new(renew.clone(), String::new()));
                        sheet.link = link;
                        sheet.issued = Some(issued);
                        sheet.copied = false;
                        sheet.confirm_close = false;
                        sheet.error = None;
                        if let Some(id) = renew {
                            self.invites.retain(|invite| invite.id != id);
                        }
                        self.load_invites(cx);
                    }
                    Err(error) => {
                        if let Some(sheet) = self.sheet.invite.as_mut() {
                            sheet.error = Some(guard_fr(&error).into());
                        }
                    }
                }
            }
            Msg::SearchDue(seq) => {
                if seq != self.seq {
                    return;
                }
                self.users.clear();
                self.next = None;
                self.loading = true;
                self.load_first(cx);
            }
            Msg::Revoked { id, result } => {
                if let Some(sheet) = self.sheet.invite.as_mut() {
                    sheet.revoking = false;
                }
                match result {
                    Ok(_) => {
                        self.invites.retain(|invite| invite.id != id);
                        if let Some(sheet) = self.sheet.invite.as_mut()
                            && sheet.renew.as_deref() == Some(id.as_str())
                        {
                            self.sheet = Sheet::none();
                        }
                        self.toasts(ToastKind::Info, "Invitation révoquée.".to_owned(), cx);
                        self.load_invites(cx);
                    }
                    Err(error) => {
                        if let Some(sheet) = self.sheet.invite.as_mut() {
                            sheet.error = Some(guard_fr(&error).into());
                        }
                        self.toasts(ToastKind::Error, guard_fr(&error), cx);
                    }
                }
            }
        }
        cx.notify();
    }

    // -- actions ---------------------------------------------------------

    fn open_user(&mut self, user: &AdminUser, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet = Sheet {
            invite: None,
            user: Some(UserSheet {
                id: user.id.to_string(),
                name: user.name.clone(),
                admin: user.admin,
                disabled: user.disabled,
                detail: None,
                busy: None,
                error: None,
                confirm_delete: false,
                confirm_disable: false,
            }),
        };
        self.load_detail(user.id.to_string(), cx);
        self.previous_focus = window.focused(cx);
        window.focus(&self.sheet_focus, cx);
        cx.notify();
    }

    fn open_invite(
        &mut self,
        invite: Option<&AdminInvite>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sheet = Sheet {
            invite: Some(InviteSheet::new(
                invite.map(|invite| invite.id.clone()),
                invite
                    .and_then(|invite| invite.note.clone())
                    .unwrap_or_default(),
            )),
            user: None,
        };
        // The note field is built with the sheet: a person who invited someone
        // arrives to type, and the sheet's own focus is taken at once.
        self.previous_focus = window.focused(cx);
        self.note = Some(cx.new(|cx| {
            InputState::new(window, cx).placeholder("Prénom ou note, pour vous y retrouver")
        }));
        window.focus(&self.sheet_focus, cx);
        cx.notify();
    }

    fn close_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A request in flight owns the answer (and, for an invitation, the only
        // copy of the token): the sheet cannot be dismissed under it.
        if self.mutating.is_some() {
            return;
        }
        // A minted link that was never copied is not recoverable: ask first.
        if let Some(sheet) = self.sheet.invite.as_mut()
            && sheet.issued.is_some()
            && !sheet.copied
            && !sheet.confirm_close
        {
            sheet.confirm_close = true;
            cx.notify();
            return;
        }
        self.sheet = Sheet::none();
        self.note = None;
        self.confirm_name = None;
        self.restore_focus(window, cx);
        cx.notify();
    }

    /// Give the keyboard back to whoever had it (the shell's anchor when the
    /// sheet was opened from outside this screen), never to a dead handle.
    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.previous_focus.take() {
            window.focus(&handle, cx);
        } else {
            window.focus(&self.root_focus, cx);
        }
    }

    fn submit_invite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mutating.is_some() {
            return;
        }
        let Some(api) = self.api(cx) else {
            return;
        };
        let typed_note = self
            .note
            .as_ref()
            .map(|note| note.read(cx).value().trim().to_string())
            .unwrap_or_default();
        let Some(sheet) = self.sheet.invite.as_mut() else {
            return;
        };
        if sheet.busy {
            return;
        }
        let note = if typed_note.is_empty() {
            sheet.note.trim().to_string()
        } else {
            typed_note
        };
        if note.chars().count() > NOTE_MAX {
            sheet.error = Some("La note doit faire 64 caractères au plus.".into());
            cx.notify();
            return;
        }
        let renew = sheet.renew.clone();
        let ttl = sheet.ttl.seconds();
        sheet.busy = true;
        sheet.error = None;
        self.mutating = Some("invite");
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let result = match &renew {
                Some(id) => api.admin_invite_renew(id).await.and_then(decode_issued),
                None => {
                    let mut body = serde_json::Map::new();
                    if !note.is_empty() {
                        body.insert("note".into(), Value::String(note));
                    }
                    body.insert("expires_in".into(), Value::from(ttl));
                    api.admin_invite_create(&Value::Object(body))
                        .await
                        .and_then(decode_issued)
                }
            };
            let _ = tx.send(Msg::Issued { renew, result });
        });
        let _ = window;
        cx.notify();
    }

    fn revoke_invite(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        if let Some(sheet) = self.sheet.invite.as_mut() {
            sheet.revoking = true;
        }
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let _ = tx.send(Msg::Revoked {
                id: id.clone(),
                result: api.admin_invite_revoke(&id).await,
            });
        });
        cx.notify();
    }

    fn patch_user(&mut self, field: &'static str, value: bool, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let Some(sheet) = self.sheet.user.as_mut() else {
            return;
        };
        if sheet.busy.is_some() {
            return;
        }
        let id = sheet.id.clone();
        let previous = if field == "admin" {
            sheet.admin
        } else {
            sheet.disabled
        };
        // Optimistic: the control answers now, the server's answer decides.
        if field == "admin" {
            sheet.admin = value;
        } else {
            sheet.disabled = value;
        }
        sheet.busy = Some(field);
        sheet.error = None;
        if let Some(row) = self.users.iter_mut().find(|row| row.id.to_string() == id) {
            if field == "admin" {
                row.admin = value;
            } else {
                row.disabled = value;
            }
        }
        self.mutating = Some(field);
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let body = serde_json::json!({ field: value });
            let result = api.admin_user_patch(&id, &body).await;
            let _ = tx.send(Msg::Patched {
                id,
                field,
                previous,
                result,
            });
        });
        cx.notify();
    }

    fn delete_user(&mut self, cx: &mut Context<Self>) {
        let typed = self
            .confirm_name
            .as_ref()
            .map(|field| field.read(cx).value().trim().to_string())
            .unwrap_or_default();
        let Some(api) = self.api(cx) else {
            return;
        };
        let Some(sheet) = self.sheet.user.as_mut() else {
            return;
        };
        if sheet.busy.is_some() {
            return;
        }
        if typed != sheet.name {
            sheet.error = Some("Tapez le nom exact du compte pour confirmer.".into());
            cx.notify();
            return;
        }
        let id = sheet.id.clone();
        let name = sheet.name.clone();
        sheet.busy = Some("delete");
        self.mutating = Some("delete");
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            let result = api.admin_user_delete(&id, &name).await;
            let _ = tx.send(Msg::Deleted { id, result });
        });
        cx.notify();
    }

    fn copy_link(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.sheet.invite.as_mut() else {
            return;
        };
        if sheet.link.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(sheet.link.clone()));
        sheet.copied = true;
        self.toasts(ToastKind::Info, "Lien d'invitation copié.".to_owned(), cx);
        cx.notify();
    }

    fn scroll_by(&mut self, distance: gpui_kit::Pixels, cx: &mut Context<Self>) {
        let mut offset = self.scroll.offset();
        offset.y = (offset.y - distance).min(gpui_kit::px(0.));
        self.scroll.set_offset(offset);
        cx.notify();
    }

    fn toasts(&mut self, kind: ToastKind, message: String, cx: &mut Context<Self>) {
        self.state.update(cx, |state, _| {
            state.toasts.push(kind, message);
        });
    }

    // -- the search field ------------------------------------------------

    fn search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some(field) = &self.search {
            return field.clone();
        }
        let field = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Rechercher un compte (nom ou courriel)")
        });
        let subscription = cx.subscribe(&field, |this, field, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
                let term = field.read(cx).value().to_string();
                if term.trim() == this.term {
                    return;
                }
                this.term = term.trim().to_owned();
                this.debounce_search(cx);
            }
        });
        self.subscriptions.push(subscription);
        self.search = Some(field.clone());
        field
    }

    /// One request per pause in the typing (the SPA's 250 ms), and the `seq`
    /// guard drops any answer for a query the person has already moved past.
    fn debounce_search(&mut self, cx: &mut Context<Self>) {
        self.seq += 1;
        let seq = self.seq;
        let runtime = self.state.read(cx).runtime.clone();
        let tx = self.tx.clone();
        runtime.spawn(async move {
            tokio::time::sleep(SEARCH_DEBOUNCE).await;
            let _ = tx.send(Msg::SearchDue(seq));
        });
    }

    // -- rendering -------------------------------------------------------

    fn matches(
        &self,
        filter: Filter,
        term: &str,
    ) -> Vec<(Option<&AdminUser>, Option<&AdminInvite>)> {
        let needle = fold_text(term);
        let mut rows: Vec<(Option<&AdminUser>, Option<&AdminInvite>)> = Vec::new();
        if filter != Filter::Invited {
            for user in &self.users {
                let keep = match filter {
                    Filter::Active => !user.disabled,
                    Filter::Disabled => user.disabled,
                    Filter::Admins => user.admin,
                    Filter::Members => !user.admin,
                    _ => true,
                };
                if keep {
                    rows.push((Some(user), None));
                }
            }
        }
        if filter == Filter::All || filter == Filter::Invited {
            for invite in &self.invites {
                if needle.is_empty()
                    || fold_text(invite.note.as_deref().unwrap_or("")).contains(&needle)
                {
                    rows.push((None, Some(invite)));
                }
            }
        }
        rows
    }

    fn chip_count(&self, filter: Filter) -> Option<usize> {
        match filter {
            Filter::All => Some(self.total + self.invites.len()),
            Filter::Invited => Some(self.invites.len()),
            Filter::Active => Some(self.users.iter().filter(|user| !user.disabled).count()),
            Filter::Disabled => Some(self.users.iter().filter(|user| user.disabled).count()),
            Filter::Admins => Some(self.users.iter().filter(|user| user.admin).count()),
            Filter::Members => Some(self.users.iter().filter(|user| !user.admin).count()),
        }
    }

    fn render_chips(&mut self, cx: &mut Context<Self>) -> Div {
        let mut chips = div().flex().items_center().gap(rems(0.375)).flex_wrap();
        let active = self.filter;
        for filter in Filter::ALL {
            let count = self.chip_count(filter);
            let label = match count {
                Some(count) if filter != Filter::All => format!("{} {count}", filter.label()),
                _ => filter.label().to_owned(),
            };
            let id = format!("filter-{}", filter.id());
            let mut chip = button(
                id,
                label,
                if filter == active {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                },
                cx,
            );
            chip = chip.on_click(cx.listener(move |this, _, _, cx| {
                this.filter = filter;
                cx.notify();
            }));
            chips = chips.child(chip);
        }
        chips
    }

    fn render_row(
        &mut self,
        index: usize,
        user: Option<&AdminUser>,
        invite: Option<&AdminInvite>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let (name, sub, state_label, state_colour, admin) = if let Some(user) = user {
            (
                user.name.clone(),
                user_line(user),
                if user.disabled {
                    "Désactivé"
                } else {
                    "Actif"
                },
                if user.disabled {
                    theme::secondary(cx)
                } else {
                    theme::success(cx)
                },
                user.admin,
            )
        } else if let Some(invite) = invite {
            let (_, label) = invite_state(invite);
            (
                invite
                    .note
                    .clone()
                    .filter(|note| !note.trim().is_empty())
                    .unwrap_or_else(|| "Invitation sans note".to_owned()),
                invite_line(invite),
                label,
                theme::warning(cx),
                false,
            )
        } else {
            return div().into_any_element();
        };
        let accessible = name.clone();
        let mut row = button(
            format!("admin-row-{index}"),
            "",
            ButtonVariant::Secondary,
            cx,
        )
        .accessibility_label(accessible)
        .w_full()
        .justify_start()
        .border_color(theme::border(cx))
        .bg(theme::shell(cx))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.625))
                .w_full()
                .min_w_0()
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
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_ellipsis()
                                .child(name),
                        )
                        .child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::secondary(cx))
                                .text_ellipsis()
                                .child(sub),
                        ),
                )
                .child(self.pill(
                    if admin { "Administrateur" } else { "Membre" },
                    theme::secondary(cx),
                    cx,
                ))
                .child(self.pill(state_label, state_colour, cx)),
        );

        match (user, invite) {
            (Some(user), _) => {
                let user = user.clone();
                row = row.on_click(cx.listener(move |this, _, window, cx| {
                    this.open_user(&user, window, cx);
                }));
            }
            (_, Some(invite)) => {
                let invite = invite.clone();
                row = row.on_click(cx.listener(move |this, _, window, cx| {
                    this.open_invite(Some(&invite), window, cx);
                }));
            }
            _ => {}
        }
        row.into_any_element()
    }

    fn render_list(&mut self, cx: &mut Context<Self>) -> Div {
        let rows = self.matches(self.filter, &self.term);
        let term = self.term.clone();
        let mut list = div().flex().flex_col().gap(rems(0.25));
        if let Some(error) = &self.error {
            list = list.child(
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
                    .child(div().flex_1().child(error.clone()))
                    .child(
                        button("users-retry", "Réessayer", ButtonVariant::Secondary, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.load_first(cx))),
                    ),
            );
        } else if rows.is_empty() {
            list = list.child(self.hint(
                if self.loading {
                    "Chargement des comptes…".to_owned()
                } else if term.is_empty() {
                    "Aucun compte.".to_owned()
                } else {
                    format!("Aucun compte ne correspond à « {term} ».")
                },
                cx,
            ));
        } else {
            // The rows are cloned out of the borrow before they are built: a
            // row's click listener needs `&mut Context<Self>`.
            let rows: Vec<(Option<AdminUser>, Option<AdminInvite>)> = rows
                .into_iter()
                .map(|(user, invite)| (user.cloned(), invite.cloned()))
                .collect();
            for (index, (user, invite)) in rows.into_iter().enumerate() {
                list = list.child(self.render_row(index, user.as_ref(), invite.as_ref(), cx));
            }
        }

        if let Some(error) = &self.invites_error {
            list = list.child(self.hint(format!("Invitations indisponibles : {error}"), cx));
        }

        let more = self.next.is_some() && self.users.len() < MAX_ROWS;
        if more {
            list = list.child(
                div().flex().items_center().gap(rems(0.5)).child(
                    button(
                        "users-more",
                        if self.loading_more {
                            "Chargement…"
                        } else {
                            "Charger plus de comptes"
                        },
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(self.loading_more)
                    .on_click(cx.listener(|this, _, _, cx| this.load_more(cx))),
                ),
            );
        } else if self.users.len() >= MAX_ROWS {
            list = list.child(self.hint(
                format!("{MAX_ROWS} comptes affichés : affinez la recherche pour voir les autres."),
                cx,
            ));
        }
        list
    }

    fn render_sheet(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        if self.sheet.invite.is_none() && self.sheet.user.is_none() {
            return None;
        }
        let content = if self.sheet.invite.is_some() {
            self.render_invite_sheet(window, cx)
        } else {
            self.render_user_sheet(window, cx)
        };
        let me = cx.entity();
        let card = div()
            .id("admin-sheet-card")
            .w(rems(30.))
            .max_h(rems(40.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(rems(0.75))
            .p(rems(1.25))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .text_color(theme::text(cx))
            .on_click(move |_, _, cx: &mut App| {
                // Clicks inside the card must not reach the veil.
                let _ = &me;
                cx.stop_propagation();
            })
            .child(content);
        let veil = div()
            .id("admin-sheet-veil")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::shell(cx).opacity(0.86))
            .track_focus(&self.sheet_focus)
            .key_context("AdminSheet")
            .on_key_down(
                cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        this.close_sheet(window, cx);
                    }
                }),
            )
            .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx)))
            .child(card);
        Some(veil)
    }

    fn render_invite_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let sheet = self
            .sheet
            .invite
            .take()
            .unwrap_or_else(|| InviteSheet::new(None, String::new()));
        let renewing = sheet.renew.is_some();
        let mut block = div().flex().flex_col().gap(rems(0.75)).child(
            div()
                .flex()
                .items_start()
                .justify_between()
                .gap(rems(0.75))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.25))
                        .flex_1()
                        .child(
                            div()
                                .text_size(rems(1.))
                                .font_weight(FontWeight::BOLD)
                                .child(if sheet.issued.is_some() {
                                    "Lien d'invitation créé"
                                } else if renewing {
                                    "Renvoyer l'invitation"
                                } else {
                                    "Inviter quelqu'un"
                                }),
                        )
                        .child(
                            div()
                                .text_size(rems(0.75))
                                .text_color(theme::secondary(cx))
                                .child(if sheet.issued.is_some() {
                                    "Ce lien ne sera plus affiché. Il ne sert qu'à un compte."
                                } else if renewing {
                                    "L'ancien lien sera révoqué immédiatement. Seul le nouveau fonctionnera."
                                } else {
                                    "Le lien crée un compte membre. Il ne sert qu'une fois."
                                }),
                        ),
                )
                .child(
                    button("invite-close", "Fermer", ButtonVariant::Secondary, cx)
                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                ),
        );

        if let Some(issued) = sheet.issued.clone() {
            block = block
                .child(
                    div()
                        .id("invite-link")
                        .w_full()
                        .p(rems(0.625))
                        .border_1()
                        .border_color(theme::border(cx))
                        .bg(theme::sidebar(cx))
                        .font_family(theme::mono(cx))
                        .text_size(rems(0.75))
                        .child(sheet.link.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(rems(0.375))
                        .text_size(rems(0.75))
                        .text_color(theme::warning(cx))
                        .child(
                            icon(IconName::TriangleAlert)
                                .size(rems(1.))
                                .mt(rems(0.0625)),
                        )
                        .child("Copiez ce lien maintenant : il ne sera plus affiché."),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .flex_wrap()
                        .child(
                            button(
                                "invite-copy",
                                if sheet.copied {
                                    "Lien copié"
                                } else {
                                    "Copier le lien"
                                },
                                ButtonVariant::Primary,
                                cx,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.copy_link(cx))),
                        )
                        .child(
                            button(
                                "invite-done",
                                "J'ai copié le lien",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.sheet = Sheet::none();
                                    this.note = None;
                                    this.restore_focus(window, cx);
                                    cx.notify();
                                },
                            )),
                        ),
                );
            if let Some(error) = &sheet.error {
                block = block.child(self.error_text(error.clone(), cx));
            }
            if sheet.confirm_close {
                block = block
                    .child(self.warning_text(
                        "Fermer sans copier le lien ? Il ne sera plus affiché ; vous pourrez renvoyer une invitation, mais ce lien ne reviendra pas.",
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.5))
                            .justify_end()
                            .child(
                                button("invite-close-cancel", "Annuler", ButtonVariant::Secondary, cx)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(sheet) = this.sheet.invite.as_mut() {
                                            sheet.confirm_close = false;
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                button("invite-close-confirm", "Fermer quand même", ButtonVariant::Danger, cx)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.sheet = Sheet::none();
                                        this.note = None;
                                        this.restore_focus(window, cx);
                                        cx.notify();
                                    })),
                            ),
                    );
            }
            if !issued.invite.id.is_empty() {
                let id = issued.invite.id.clone();
                block = block.child(self.invite_revoke_row(
                    id,
                    sheet.confirm_revoke,
                    sheet.revoking,
                    cx,
                ));
            }
            self.sheet.invite = Some(sheet);
            return block;
        }

        if renewing {
            if let Some(id) = &sheet.renew
                && let Some(invite) = self.invites.iter().find(|invite| &invite.id == id)
            {
                block = block.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.25))
                        .p(rems(0.625))
                        .border_1()
                        .border_color(theme::border(cx))
                        .child(self.label("Pour qui"))
                        .child(
                            div().text_size(rems(0.8125)).child(
                                invite
                                    .note
                                    .clone()
                                    .unwrap_or_else(|| "Sans note".to_owned()),
                            ),
                        )
                        .child(self.label("Expiration"))
                        .child(div().text_size(rems(0.8125)).child(invite_line(invite))),
                );
            }
        } else {
            let note = match &self.note {
                Some(note) => note.clone(),
                None => {
                    let field = cx.new(|cx| {
                        InputState::new(window, cx)
                            .placeholder("Prénom ou note, pour vous y retrouver")
                    });
                    self.note = Some(field.clone());
                    field
                }
            };
            block = block
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.25))
                        .child(self.label("Pour qui ? (facultatif)"))
                        .child(input("invite-note", &note, window, cx)),
                )
                .child(self.hint(
                    "Ce nom ne crée pas de compte et n'est pas montré à la personne invitée : il vous sert seulement à reconnaître le lien.",
                    cx,
                ))
                .child(self.label("Durée de validité"));
            let mut segments = div().flex().items_center().gap(rems(0.375)).flex_wrap();
            for ttl in Ttl::ALL {
                segments = segments.child(
                    button(
                        format!("invite-ttl-{}", ttl.seconds()),
                        ttl.label(),
                        if sheet.ttl == ttl {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        },
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(sheet) = this.sheet.invite.as_mut() {
                            sheet.ttl = ttl;
                        }
                        cx.notify();
                    })),
                );
            }
            block = block.child(segments);
        }

        if let Some(error) = &sheet.error {
            block = block.child(self.error_text(error.clone(), cx));
        }

        if renewing {
            block = block.child(self.invite_revoke_row(
                sheet.renew.clone().unwrap_or_default(),
                sheet.confirm_revoke,
                sheet.revoking,
                cx,
            ));
        }

        let busy = sheet.busy;
        let renewing = sheet.renew.is_some();
        block = block.child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(rems(0.5))
                .flex_wrap()
                .child(
                    button("invite-cancel", "Annuler", ButtonVariant::Secondary, cx)
                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                )
                .child(
                    button(
                        "invite-create",
                        if busy {
                            if renewing {
                                "Renouvellement…"
                            } else {
                                "Création…"
                            }
                        } else if renewing {
                            "Créer un nouveau lien"
                        } else {
                            "Créer le lien"
                        },
                        ButtonVariant::Primary,
                        cx,
                    )
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.submit_invite(window, cx))),
                ),
        );
        self.sheet.invite = Some(sheet);
        block
    }

    fn invite_revoke_row(
        &mut self,
        id: String,
        confirming: bool,
        revoking: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        if confirming {
            div()
                .flex()
                .flex_col()
                .gap(rems(0.375))
                .p(rems(0.5))
                .border_1()
                .border_color(theme::border(cx))
                .child(self.warning_text(
                    "Révoquer l'invitation ? Le lien actuel cessera de fonctionner.",
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(rems(0.5))
                        .child(
                            button(
                                "invite-revoke-cancel",
                                "Annuler",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(sheet) = this.sheet.invite.as_mut() {
                                    sheet.confirm_revoke = false;
                                }
                                cx.notify();
                            })),
                        )
                        .child(
                            button(
                                "invite-revoke-confirm",
                                if revoking {
                                    "Révocation…"
                                } else {
                                    "Révoquer"
                                },
                                ButtonVariant::Danger,
                                cx,
                            )
                            .disabled(revoking)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.revoke_invite(id.clone(), cx);
                                },
                            )),
                        ),
                )
        } else {
            div().flex().child(
                button(
                    "invite-revoke",
                    "Révoquer l'invitation",
                    ButtonVariant::Danger,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(sheet) = this.sheet.invite.as_mut() {
                        sheet.confirm_revoke = true;
                    }
                    cx.notify();
                })),
            )
        }
    }

    fn render_user_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let sheet = self.sheet.user.take().expect("a user sheet");
        let mut block = div().flex().flex_col().gap(rems(0.75)).child(
            div()
                .flex()
                .items_start()
                .justify_between()
                .gap(rems(0.75))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.125))
                        .flex_1()
                        .child(
                            div()
                                .text_size(rems(1.))
                                .font_weight(FontWeight::BOLD)
                                .child(sheet.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(rems(0.75))
                                .text_color(theme::secondary(cx))
                                .child(self.pill(
                                    role_label_from(sheet.admin),
                                    theme::secondary(cx),
                                    cx,
                                )),
                        ),
                )
                .child(
                    button("user-close", "Fermer", ButtonVariant::Secondary, cx)
                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                ),
        );

        if let Some(detail) = &sheet.detail {
            let mut facts = div().flex().flex_col().gap(rems(0.25));
            facts = facts
                .child(self.fact(
                    "Courriel",
                    detail.user.mail.clone().unwrap_or_else(|| "—".to_owned()),
                    cx,
                ))
                .child(self.fact("Dernière connexion", user_line(&detail.user), cx))
                .child(self.fact(
                    "Fichiers importés",
                    count_label(detail.uploads as usize, "fichier"),
                    cx,
                ))
                .child(self.fact(
                    "Playlists",
                    count_label(detail.playlists as usize, "playlist"),
                    cx,
                ));
            block = block.child(facts);
        } else {
            block = block.child(self.hint("Chargement du compte…", cx));
        }

        if let Some(error) = &sheet.error {
            block = block.child(self.error_text(error.clone(), cx));
        }

        let busy = sheet.busy;
        block = block
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .flex_wrap()
                    .child(
                        button(
                            "user-admin",
                            if sheet.admin {
                                "Retirer les droits d'administration"
                            } else {
                                "Nommer administrateur"
                            },
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .disabled(busy.is_some())
                        .on_click(cx.listener(|this, _, _, cx| {
                            let next = this
                                .sheet
                                .user
                                .as_ref()
                                .map(|sheet| !sheet.admin)
                                .unwrap_or(false);
                            this.patch_user("admin", next, cx);
                        })),
                    )
                    .child(
                        button(
                            "user-disable",
                            if sheet.disabled {
                                "Réactiver le compte"
                            } else {
                                "Désactiver le compte"
                            },
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .disabled(busy.is_some())
                        .on_click(cx.listener(|this, _, _, cx| {
                            let next = this
                                .sheet
                                .user
                                .as_ref()
                                .map(|sheet| !sheet.disabled)
                                .unwrap_or(false);
                            this.patch_user("disabled", next, cx);
                        })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.375))
                    .mt(rems(0.25))
                    .pt(rems(0.75))
                    .border_t_1()
                    .border_color(theme::border(cx)),
            );

        if sheet.confirm_delete {
            let typed = match &self.confirm_name {
                Some(field) => field.clone(),
                None => {
                    let field =
                        cx.new(|cx| InputState::new(window, cx).placeholder(sheet.name.clone()));
                    self.confirm_name = Some(field.clone());
                    field
                }
            };
            block = block
                .child(self.warning_text(
                    "Supprimer ce compte ? Ses fichiers importés et ses podcasts sont réattribués, ses playlists disparaissent. Cette action est définitive.",
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.25))
                        .child(self.label("Tapez le nom du compte pour confirmer"))
                        .child(input("user-confirm-name", &typed, window, cx)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(rems(0.5))
                        .child(
                            button("user-delete-cancel", "Annuler", ButtonVariant::Secondary, cx)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(sheet) = this.sheet.user.as_mut() {
                                        sheet.confirm_delete = false;
                                    }
                                    this.confirm_name = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            button(
                                "user-delete-confirm",
                                if busy == Some("delete") {
                                    "Suppression…"
                                } else {
                                    "Supprimer le compte"
                                },
                                ButtonVariant::Danger,
                                cx,
                            )
                            .disabled(busy.is_some())
                            .on_click(cx.listener(|this, _, _, cx| this.delete_user(cx))),
                        ),
                );
        } else {
            block = block.child(
                div().flex().child(
                    button(
                        "user-delete",
                        "Supprimer le compte",
                        ButtonVariant::Danger,
                        cx,
                    )
                    .disabled(busy.is_some())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(sheet) = this.sheet.user.as_mut() {
                            sheet.confirm_delete = true;
                        }
                        cx.notify();
                    })),
                ),
            );
        }

        self.sheet.user = Some(sheet);
        block
    }

    fn error_text(&self, text: SharedString, cx: &App) -> Div {
        div()
            .flex()
            .items_start()
            .gap(rems(0.375))
            .p(rems(0.5))
            .border_1()
            .border_color(theme::danger(cx))
            .text_size(rems(0.75))
            .text_color(theme::danger(cx))
            .child(
                icon(IconName::CircleAlert)
                    .size(rems(0.875))
                    .mt(rems(0.0625)),
            )
            .child(div().flex_1().child(text))
    }

    fn warning_text(&self, text: &str, cx: &App) -> Div {
        div()
            .flex()
            .items_start()
            .gap(rems(0.375))
            .text_size(rems(0.75))
            .text_color(theme::warning(cx))
            .child(
                icon(IconName::TriangleAlert)
                    .size(rems(0.875))
                    .mt(rems(0.0625)),
            )
            .child(div().flex_1().child(text.to_owned()))
    }

    fn label(&self, text: &'static str) -> Div {
        div()
            .text_size(rems(0.625))
            .font_weight(FontWeight::SEMIBOLD)
            .child(text)
    }

    fn hint(&self, text: impl Into<SharedString>, cx: &App) -> Div {
        div()
            .text_size(rems(0.6875))
            .text_color(theme::secondary(cx))
            .child(text.into())
    }

    fn fact(&self, name: &'static str, value: String, cx: &App) -> Div {
        div()
            .flex()
            .items_start()
            .gap(rems(0.625))
            .text_size(rems(0.75))
            .child(
                div()
                    .w(rems(7.))
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
}

fn role_label_from(admin: bool) -> &'static str {
    if admin { "Administrateur" } else { "Membre" }
}

fn decode_issued(value: Value) -> Result<IssuedInvite, ApiError> {
    serde_json::from_value::<IssuedInvite>(value)
        .map_err(|error| ApiError::Decode(format!("invitation illisible : {error}")))
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let search = self.search_field(window, cx);
        let term = self.term.clone();
        let total = self.total;
        let invites = self.invites.len();
        let loading = self.loading;

        let column = div()
            .w_full()
            .max_w(rems(64.))
            .flex()
            .flex_col()
            .gap(rems(0.75))
            .px(rems(1.5))
            .py(rems(1.25))
            .child(
                div()
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
                                        "Les comptes de ce serveur, leur rôle, leur état, et les invitations qui en créent d'autres.",
                                    ),
                            ),
                    )
                    .child(
                        button("invite-open", "Inviter quelqu'un", ButtonVariant::Primary, cx)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_invite(None, window, cx);
                            })),
                    ),
            )
            .child(
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
                            .flex_1()
                            .min_w(rems(18.))
                            .child(self.label("Recherche"))
                            .child(input("users-search", &search, window, cx)),
                    )
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .text_color(theme::secondary(cx))
                            .child(if loading {
                                "Chargement des comptes…".to_owned()
                            } else if term.is_empty() {
                                format!(
                                    "{} · {} en attente",
                                    count_label(total, "compte"),
                                    count_label(invites, "invitation")
                                )
                            } else {
                                format!(
                                    "{} correspondant(s)",
                                    count_label(total, "compte")
                                )
                            }),
                    ),
            )
            .child(self.render_chips(cx))
            .child(self.render_list(cx))
            .child(div().h(rems(0.5)));

        self.viewport = window.viewport_size().height;

        let content = focus_scope("settings-users")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .track_focus(&self.root_focus)
            .bg(theme::shell(cx))
            .font_family(theme::font(cx))
            .text_color(theme::text(cx))
            .child(div().w_full().flex().justify_center().child(column));

        match self.render_sheet(window, cx) {
            Some(sheet) => div()
                .relative()
                .size_full()
                .child(content)
                .child(sheet)
                .into_any_element(),
            None => content.into_any_element(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn user(name: &str, admin: bool, disabled: bool, last_login: Option<&str>) -> AdminUser {
        AdminUser {
            id: nz_model::UserId::from(format!("id-{name}").as_str()),
            name: name.to_owned(),
            admin,
            disabled,
            last_login: last_login.map(str::to_owned),
            ..AdminUser::default()
        }
    }

    fn invite(note: Option<&str>, state: &str, expires: Option<&str>) -> AdminInvite {
        AdminInvite {
            id: format!("invite-{}", note.unwrap_or("none")),
            note: note.map(str::to_owned),
            state: state.to_owned(),
            expires: expires.map(str::to_owned),
            ..AdminInvite::default()
        }
    }

    #[test]
    fn the_fold_strips_the_french_accents_the_spa_normalises() {
        assert_eq!(fold_text("  Élodie  "), "elodie");
        assert_eq!(fold_text("François"), "francois");
        assert_eq!(fold_text("ÇA"), "ca");
        assert_eq!(fold_text("œuf"), "ouf");
    }

    #[test]
    fn the_invitation_link_opens_the_spa_on_this_server() {
        assert_eq!(
            invite_link("http://192.168.1.20:5000", "abc.def"),
            "http://192.168.1.20:5000/app/#/invite/abc.def"
        );
        assert_eq!(
            invite_link("http://host:5000/", "t"),
            "http://host:5000/app/#/invite/t",
            "a trailing slash never doubles"
        );
    }

    #[test]
    fn a_refusal_is_the_spas_sentence() {
        let error = ApiError::Http {
            status: 409,
            message: "refused".into(),
            code: Some("last_admin".into()),
        };
        assert_eq!(
            guard_fr(&error),
            "Impossible : c'est le dernier administrateur actif."
        );
        assert_eq!(
            guard_fr(&ApiError::Offline),
            "Le serveur ne répond pas (réseau)."
        );
        assert_eq!(
            guard_fr(&ApiError::Http {
                status: 403,
                message: "Réservé aux administrateurs.".into(),
                code: Some("forbidden".into()),
            }),
            "Votre compte n'a plus les droits d'administration."
        );
        assert_eq!(
            guard_fr(&ApiError::Http {
                status: 500,
                message: "boom".into(),
                code: None,
            }),
            "erreur du serveur (HTTP 500 : boom)"
        );
    }

    #[test]
    fn the_invitation_line_says_what_happened_to_it() {
        let pending = invite(Some("Camille"), "pending", Some("2026-11-23T20:00:00"));
        assert_eq!(invite_state(&pending), ("invite", "Invité"));
        assert_eq!(invite_line(&pending), "Expire le 23 nov. 2026");

        let mut used = invite(Some("Camille"), "used", Some("2026-11-23T20:00:00"));
        used.used_by = Some("camille".into());
        used.used_at = Some("2026-11-17T09:30:00".into());
        assert_eq!(invite_state(&used), ("used", "Utilisée"));
        assert_eq!(
            invite_line(&used),
            "Utilisée par camille le 17 nov. 2026 à 09:30"
        );

        let expired = invite(None, "expired", Some("2026-11-01T00:00:00"));
        assert_eq!(invite_state(&expired).1, "Expirée");
        assert_eq!(invite_line(&expired), "Expirée le 1 nov. 2026");
    }

    #[test]
    fn the_account_line_is_absolute_never_a_guess() {
        let seen = user("bench", false, false, Some("2026-11-16T20:15:00"));
        assert_eq!(user_line(&seen), "Vu le 16 nov. 2026 à 20:15");
        let never = user("neuf", false, false, None);
        assert_eq!(user_line(&never), "Jamais connecté");
    }

    #[test]
    fn the_ttls_are_the_spas_four() {
        let seconds: Vec<u64> = Ttl::ALL.iter().map(|ttl| ttl.seconds()).collect();
        assert_eq!(seconds, vec![3_600, 86_400, 604_800, 2_592_000]);
        assert_eq!(Ttl::Week.label(), "7 jours");
    }

    #[test]
    fn an_invitation_payload_decodes_the_token_once() {
        let issued = decode_issued(serde_json::json!({
            "invite": {"id": "abc", "note": "Camille", "created": "2026-11-16T20:00:00",
                       "expires": "2026-11-23T20:00:00", "state": "pending"},
            "token": "abc.token",
        }))
        .expect("issued invitation");
        assert_eq!(issued.token, "abc.token");
        assert_eq!(issued.invite.note.as_deref(), Some("Camille"));
    }

    fn rss_kb() -> i64 {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    }

    /// The screen's own worst case: the 250 accounts `MAX_ROWS` allows and the
    /// 200 invitations two 100-row pages bring. The rows are the only state that
    /// grows; the widgets are built per paint, so the count never follows.
    #[test]
    fn two_hundred_fifty_accounts_and_two_hundred_invitations_stay_small() {
        let before = rss_kb();
        let users: Vec<AdminUser> = (0..250)
            .map(|index| AdminUser {
                id: nz_model::UserId::from(
                    format!("3f7c1d2e-9a4b-5c6d-8e0f-{index:012x}").as_str(),
                ),
                name: format!("utilisateur {index}"),
                mail: Some(format!("utilisateur{index}@example.org")),
                last_login: Some("2026-11-16T20:15:00".to_owned()),
                ..AdminUser::default()
            })
            .collect();
        let invites: Vec<AdminInvite> = (0..200)
            .map(|index| AdminInvite {
                id: format!("invite-{index}"),
                note: Some(format!("invité {index}")),
                state: "pending".to_owned(),
                expires: Some("2026-11-23T10:00:00".to_owned()),
                ..AdminInvite::default()
            })
            .collect();
        let after = rss_kb();
        let cost = after.saturating_sub(before);
        println!("Utilisateurs: 250 comptes + 200 invitations = +{cost} KB de RSS");
        assert!(cost < 4 * 1024, "the capped rows cost {cost} KB");
        assert_eq!(users.len(), MAX_ROWS);
        assert_eq!(invites.len(), 200);
    }

    #[test]
    fn count_labels_agree() {
        assert_eq!(count_label(0, "compte"), "0 compte");
        assert_eq!(count_label(1, "compte"), "1 compte");
        assert_eq!(count_label(3, "compte"), "3 comptes");
    }
}

// ---------------------------------------------------------------------------
// The screen against a real socket: the two reads, the token once, the guards.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod screen_tests {
    use super::super::server::stub::{self, Stub};
    use super::*;

    const USER_ID: &str = "3f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f";
    const OTHER_ID: &str = "4f7c1d2e-9a4b-5c6d-8e0f-1a2b3c4d5e6f";
    const INVITE_ID: &str = "abc123";

    fn users_routes() -> Vec<(&'static str, String)> {
        vec![
            (
                "GET /api/admin/users?",
                serde_json::json!({
                    "users": [
                        {"id": USER_ID, "name": "bench", "admin": true, "jukebox": false,
                         "disabled": false, "mail": "bench@example.org",
                         "last_login": "2026-11-16T20:15:00", "onboarding": "done"},
                        {"id": OTHER_ID, "name": "camille", "admin": false, "jukebox": false,
                         "disabled": false, "mail": null, "last_login": null,
                         "onboarding": "done"}
                    ],
                    "total": 2, "next": null
                })
                .to_string(),
            ),
            (
                "GET /api/admin/users/3f7c1d2e",
                serde_json::json!({
                    "user": {"id": USER_ID, "name": "bench", "admin": true, "jukebox": false,
                             "disabled": false, "mail": "bench@example.org",
                             "last_login": "2026-11-16T20:15:00", "onboarding": "done"},
                    "uploads": 3, "playlists": 2
                })
                .to_string(),
            ),
            (
                "PATCH /api/admin/users/",
                serde_json::json!({"user": {"id": USER_ID, "name": "bench", "admin": false,
                                            "jukebox": false, "disabled": false, "mail": null,
                                            "last_login": null, "onboarding": "done"}})
                .to_string(),
            ),
            (
                "DELETE /api/admin/users/",
                serde_json::json!({"deleted": true, "uploads_reassigned": 3,
                                   "podcasts_reassigned": 0})
                .to_string(),
            ),
            (
                "GET /api/admin/invites?",
                serde_json::json!({"invites": [
                    {"id": INVITE_ID, "note": "Camille", "created": "2026-11-16T10:00:00",
                     "expires": "2026-11-23T10:00:00", "state": "pending",
                     "created_by": "bench", "used_by": null, "used_at": null}
                ], "next": null})
                .to_string(),
            ),
            (
                "POST /api/admin/invites/",
                serde_json::json!({"revoked": true}).to_string(),
            ),
            (
                "POST /api/admin/invites",
                serde_json::json!({
                    "invite": {"id": "new-invite", "note": "Camille",
                               "created": "2026-11-16T20:00:00",
                               "expires": "2026-11-23T20:00:00", "state": "pending"},
                    "token": "new-invite.secret"
                })
                .to_string(),
            ),
        ]
    }

    /// The acceptance's shape: opening the tab is TWO reads, the rows carry the
    /// server's own counters, and an invitation's link is shown once.
    #[gpui::test]
    async fn the_users_screen_reads_two_pages_and_shows_the_counts(cx: &mut gpui::TestAppContext) {
        let stub = Stub::start(users_routes());
        let (state, runtime, dir) = stub::logged_in(cx, &stub, "users-list");
        let (view, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| Screen::new(SettingsSection::Users, state, cx)
        });

        stub::wait_until(&runtime, cx, &view, "the two pages", |screen| {
            screen.invites_loaded && !screen.loading && screen.users.len() == 2
        });

        assert_eq!(stub.total(), 2, "one page of accounts, one of invitations");
        println!(
            "Utilisateurs: {} requêtes, {} octets de JSON",
            stub.total(),
            stub.bytes()
        );
        assert!(stub.bytes() < 8 * 1024);
        assert_eq!(stub.count("GET /api/admin/users?"), 1);
        assert_eq!(stub.count("GET /api/admin/invites?"), 1);

        cx.update(|_, cx| {
            let screen = view.read(cx);
            assert_eq!(screen.total, 2);
            assert_eq!(
                screen.matches(Filter::All, "").len(),
                3,
                "two accounts + one invite"
            );
            assert_eq!(screen.matches(Filter::Admins, "").len(), 1);
            assert_eq!(screen.matches(Filter::Members, "").len(), 1);
            assert_eq!(screen.matches(Filter::Invited, "").len(), 1);
            // The ACCOUNTS are filtered by the server (`?q=`), the invitations
            // here: the stub ignores `q`, so only the invite is narrowed.
            assert_eq!(screen.matches(Filter::All, "cami").len(), 3);
            assert_eq!(screen.matches(Filter::Invited, "cami").len(), 1);
            assert_eq!(screen.matches(Filter::Invited, "zzz").len(), 0);
            assert_eq!(screen.chip_count(Filter::All), Some(3));
        });

        // The invitation: minted once, its link shown, never fetched again.
        cx.update(|window, cx| {
            view.update(cx, |screen, cx| screen.open_invite(None, window, cx));
        });
        cx.update(|window, cx| {
            view.update(cx, |screen, cx| {
                let note = screen.note.clone().expect("the note field");
                note.update(cx, |field, cx| field.set_value("Camille", window, cx));
                screen.submit_invite(window, cx);
            });
        });
        stub::wait_until(&runtime, cx, &view, "the minted link", |screen| {
            screen
                .sheet
                .invite
                .as_ref()
                .is_some_and(|sheet| sheet.issued.is_some())
        });
        cx.update(|_, cx| {
            let sheet = view.read(cx).sheet.invite.as_ref().expect("the sheet");
            let expected = format!("{}/app/#/invite/new-invite.secret", stub.base);
            assert_eq!(sheet.link, expected);
            assert!(!sheet.copied);
        });
        assert_eq!(stub.count("POST /api/admin/invites"), 1);

        // Closing without copying asks first (the link is unrecoverable).
        cx.simulate_keystrokes("escape");
        cx.update(|_, cx| {
            assert!(
                view.read(cx)
                    .sheet
                    .invite
                    .as_ref()
                    .is_some_and(|sheet| sheet.confirm_close),
                "the sheet warns before dropping an uncopied link"
            );
        });

        // Copying marks it delivered; closing then keeps no token anywhere.
        cx.update(|_, cx| {
            view.update(cx, |screen, cx| screen.copy_link(cx));
            let sheet = view.read(cx).sheet.invite.as_ref().expect("the sheet");
            assert!(sheet.copied);
        });
        cx.update(|window, cx| {
            view.update(cx, |screen, cx| screen.close_sheet(window, cx));
        });
        cx.update(|window, cx| {
            view.update(cx, |screen, cx| screen.open_invite(None, window, cx));
            let sheet = view.read(cx).sheet.invite.as_ref().expect("the sheet");
            assert!(
                sheet.issued.is_none() && sheet.link.is_empty(),
                "reopening shows the form, never the previous token"
            );
        });

        // Revoking an invitation is the DELETE the server expects.
        cx.update(|_, cx| {
            view.update(cx, |screen, cx| {
                screen.revoke_invite(INVITE_ID.to_owned(), cx)
            });
        });
        stub::wait_for_calls(
            &runtime,
            cx,
            &stub,
            "POST /api/admin/invites/abc123/revoke",
            1,
        );

        std::fs::remove_dir_all(dir).ok();
        drop(runtime);
    }

    /// The account sheet: the detail is its own read, a role change is optimistic
    /// and rolled back on refusal, and deleting needs the name typed.
    #[gpui::test]
    async fn an_account_is_promoted_and_deleted_with_the_name_typed(cx: &mut gpui::TestAppContext) {
        let stub = Stub::start(users_routes());
        let (state, runtime, dir) = stub::logged_in(cx, &stub, "users-detail");
        let (view, cx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| Screen::new(SettingsSection::Users, state, cx)
        });
        stub::wait_until(&runtime, cx, &view, "the accounts", |screen| {
            screen.users.len() == 2 && screen.invites_loaded
        });

        cx.update(|window, cx| {
            let user = view.read(cx).users[0].clone();
            view.update(cx, |screen, cx| screen.open_user(&user, window, cx));
        });
        stub::wait_for_calls(&runtime, cx, &stub, "GET /api/admin/users/3f7c1d2e", 1);
        stub::wait_until(&runtime, cx, &view, "the account detail", |screen| {
            screen
                .sheet
                .user
                .as_ref()
                .is_some_and(|sheet| sheet.detail.is_some())
        });
        cx.update(|_, cx| {
            let sheet = view.read(cx).sheet.user.as_ref().expect("the sheet");
            assert_eq!(sheet.detail.as_ref().unwrap().uploads, 3);
            assert!(sheet.admin);
        });

        // A role change: optimistic, then the server's row replaces it.
        cx.update(|_, cx| {
            view.update(cx, |screen, cx| screen.patch_user("admin", false, cx));
        });
        cx.update(|_, cx| {
            assert!(
                !view.read(cx).users[0].admin,
                "the row answers before the network"
            );
        });
        stub::wait_for_calls(&runtime, cx, &stub, "PATCH /api/admin/users/3f7c1d2e", 1);
        stub::wait_until(&runtime, cx, &view, "the patched role", |screen| {
            screen
                .sheet
                .user
                .as_ref()
                .is_some_and(|sheet| sheet.busy.is_none() && !sheet.admin)
        });

        // Deleting refuses without the exact name, then goes through.
        cx.update(|_, cx| {
            view.update(cx, |screen, cx| screen.delete_user(cx));
            let sheet = view.read(cx).sheet.user.as_ref().expect("the sheet");
            assert!(sheet.error.is_some(), "an untyped confirmation is refused");
        });
        assert_eq!(stub.count("DELETE /api/admin/users/"), 0);

        cx.update(|window, cx| {
            view.update(cx, |screen, cx| {
                let field = cx.new(|cx| gpui_kit::base::input::InputState::new(window, cx));
                field.update(cx, |field, cx| field.set_value("bench", window, cx));
                screen.confirm_name = Some(field);
                screen.delete_user(cx);
            });
        });
        stub::wait_for_calls(&runtime, cx, &stub, "DELETE /api/admin/users/", 1);
        stub::wait_until(&runtime, cx, &view, "the removed account", |screen| {
            screen.users.len() == 1 && screen.sheet.user.is_none()
        });
        cx.update(|_, cx| {
            assert!(
                state
                    .read(cx)
                    .toasts
                    .items()
                    .iter()
                    .any(|toast| toast.message.contains("Compte supprimé"))
            );
        });

        std::fs::remove_dir_all(dir).ok();
        drop(runtime);
    }
}
