//! Une playlist: sa page, son ordre, ses gens, son lien, ses suggestions.
//!
//! The screen follows the web app's `routes/Playlist.svelte` step for step, and
//! its two harder rules:
//!
//! - **Every edit is optimistic and carries the version the page had.** A
//!   reorder, a removal or an addition changes the list on screen first, then
//!   sends `version`, and a `409` re-reads ONCE and says so — never a merge,
//!   never a replay (`playlist.py` refuses a stale write on purpose).
//! - **A removal is by ROW INDEX**, because a playlist may hold the same title
//!   three times (`removePlaylistIndexes`). Removing "the track" would take the
//!   wrong copy out.
//!
//! Reordering answers both hands: `Alt+↑/↓` (or the row's own two buttons) for
//! the keyboard, and press-move-release on the row for the mouse. A burst of
//! moves is coalesced into ONE `PUT /order` 350 ms after the last one, which is
//! the SPA's own debounce.

use std::rc::Rc;

use gpui_kit::base::VirtualListScrollHandle;
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Div, ElementId, Entity, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, ScrollStrategy, SharedString, Size,
    StatefulInteractiveElement as _, Styled, Subscription, Window, div,
    prelude::FluentBuilder as _, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon, icon_button, input};

use nz_model::{Playlist, PlaylistPage, Track};
use nz_player::{Command, Context as PlayerContext, Entry, QueueArgs, ShuffleArgs};
use serde_json::json;

use crate::actions::*;
use crate::nav::Route;
use crate::screens::library::{
    Loaded, SortKey, artist_line, format_duration, project, reference_of, sleeve, track_cells,
};
use crate::state::{AppState, entry_from_track};
use crate::theme;
use crate::toast::ToastKind;
use crate::widgets::sheets::{
    ExportKind, ExportSheet, MembersAnswer, MembersSheet, SheetEvent, error_line, face,
    is_conflict, is_forbidden, note_line, push_toast,
};

/// Row height in edit mode: taller than a plain row because it carries the two
/// move buttons and the removal (a 44 px target is the minimum a finger wants).
const EDIT_ROW: f32 = 64.;
/// The same row in view mode.
const ROW: f32 = 56.;

/// Where the page stands.
pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    id: String,
    page: Loaded<PlaylistPage>,
    /// The list as shown; edit mode mutates it before the server answers.
    tracks: Vec<Track>,
    error: Option<SharedString>,
    /// The playlist stopped being accessible under an open page.
    gone: bool,
    editing: bool,
    selected: usize,
    filter: Option<Entity<InputState>>,
    query: String,
    sort: SortKey,
    descending: bool,
    projection: Vec<usize>,
    /// The drag in progress: the row picked up, the row under the pointer.
    dragging: Option<usize>,
    drag_over: Option<usize>,
    /// The order waiting to be sent (coalesced), and whether a flush is armed.
    pending_order: Option<Vec<String>>,
    order_armed: bool,
    /// Recommendations, asked for after the page is on screen.
    recommendations: Loaded<Vec<Track>>,
    rec_busy: bool,
    rename_title: Option<Entity<InputState>>,
    rename_description: Option<Entity<InputState>>,
    renaming: bool,
    confirm_delete: bool,
    confirm_leave: bool,
    members: Option<Entity<MembersSheet>>,
    export: Option<Entity<ExportSheet>>,
    scroll: VirtualListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl Screen {
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let id = match &route {
            Route::Playlist { id } => id.clone(),
            _ => String::new(),
        };
        let mut this = Self {
            route,
            state,
            id,
            page: Loaded::Loading,
            tracks: Vec::new(),
            error: None,
            gone: false,
            editing: false,
            selected: 0,
            filter: None,
            query: String::new(),
            sort: SortKey::Default,
            descending: false,
            projection: Vec::new(),
            dragging: None,
            drag_over: None,
            pending_order: None,
            order_armed: false,
            recommendations: Loaded::Loading,
            rec_busy: false,
            rename_title: None,
            rename_description: None,
            renaming: false,
            confirm_delete: false,
            confirm_leave: false,
            members: None,
            export: None,
            scroll: VirtualListScrollHandle::new(),
            _subscriptions: Vec::new(),
        };
        this._subscriptions
            .push(cx.observe(&this.state.clone(), |_, _, cx| {
                cx.notify();
            }));
        this.bind_list_actions(cx);
        this.load(cx);
        this
    }

    /// The list's verbs, registered on the app with a weak handle — never on the
    /// focus anchor, which belongs to the shell: a screen that takes the focus
    /// and is then unmounted leaves the window with nothing focused, and every
    /// global binding (Space, `q`, `g h`…) silently stops working (D7 measured
    /// it; this lot's first journey test re-measured it).
    fn bind_list_actions(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        macro_rules! on {
            ($action:ty, $handler:expr) => {{
                let weak = weak.clone();
                let handler = $handler;
                gpui_kit::App::on_action(cx, move |_: &$action, cx: &mut gpui_kit::App| {
                    weak.update(cx, |this, cx| handler(this, cx)).ok();
                });
            }};
        }
        on!(ListNext, |this: &mut Self, cx: &mut Context<Self>| {
            this.move_selection(1, false);
            cx.notify();
        });
        on!(ListPrevious, |this: &mut Self, cx: &mut Context<Self>| {
            this.move_selection(-1, false);
            cx.notify();
        });
        on!(ListFirst, |this: &mut Self, cx: &mut Context<Self>| {
            this.selected = 0;
            this.scroll.scroll_to_item(0, ScrollStrategy::Top);
            cx.notify();
        });
        on!(ListLast, |this: &mut Self, cx: &mut Context<Self>| {
            let count = if this.manual_view() {
                this.tracks.len()
            } else {
                this.projection.len()
            };
            this.selected = count.saturating_sub(1);
            this.scroll
                .scroll_to_item(this.selected, ScrollStrategy::Bottom);
            cx.notify();
        });
        on!(ListPageDown, |this: &mut Self, cx: &mut Context<Self>| {
            this.move_selection(1, true);
            cx.notify();
        });
        on!(ListPageUp, |this: &mut Self, cx: &mut Context<Self>| {
            this.move_selection(-1, true);
            cx.notify();
        });
        on!(PlayFromHere, |this: &mut Self, cx: &mut Context<Self>| {
            let at = this.selected;
            this.play(at, false, cx)
        });
        on!(Refresh, |this: &mut Self, cx: &mut Context<Self>| this
            .load(cx));
        on!(MoveRowUp, |this: &mut Self, cx: &mut Context<Self>| {
            if this.editing
                && let Some(index) = this.track_index(this.selected)
            {
                this.reorder(index, index.saturating_sub(1), cx);
            }
        });
        on!(MoveRowDown, |this: &mut Self, cx: &mut Context<Self>| {
            if this.editing
                && let Some(index) = this.track_index(this.selected)
            {
                this.reorder(index, index + 1, cx);
            }
        });
        crate::screens::library::install_row_keys(cx);
    }

    fn ensure_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter.is_some() {
            return;
        }
        let filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Rechercher dans la playlist"));
        self._subscriptions
            .push(cx.subscribe(&filter, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query = this
                        .filter
                        .as_ref()
                        .map(|filter| filter.read(cx).value().to_string())
                        .unwrap_or_default();
                    this.recompute();
                    this.selected = 0;
                    cx.notify();
                }
            }));
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Nom de la playlist"));
        self._subscriptions
            .push(cx.subscribe(&title, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_meta(cx);
                }
            }));
        let description =
            cx.new(|cx| InputState::new(window, cx).placeholder("Description (optionnel)"));
        self.filter = Some(filter);
        self.rename_title = Some(title);
        self.rename_description = Some(description);
    }

    // -- loading ------------------------------------------------------------

    fn path(&self) -> String {
        format!("/playlist/{}", self.id)
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        // The cached page first: a big playlist opened before is on screen in
        // the frame the screen is built, and the fresh copy reconciles after.
        if let Some(page) = self.cached(cx) {
            self.adopt(page, cx);
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let id = self.id.clone();
        let runtime = self.state.read(cx).runtime.clone();
        let task = runtime.spawn(async move { api.playlist(&id).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = task.await else {
                return;
            };
            this.update(cx, |this, cx| {
                match result {
                    Ok(page) => {
                        let editable = page.playlist.editable;
                        this.adopt(page, cx);
                        if editable {
                            this.load_recommendations(cx);
                        }
                    }
                    Err(error) if is_missing(&error) => {
                        this.gone = true;
                        this.page =
                            Loaded::Failed("Cette playlist ne vous est plus accessible.".into());
                        push_toast(
                            cx,
                            &this.state,
                            ToastKind::Info,
                            "Cette playlist ne vous est plus accessible.",
                        );
                    }
                    Err(error) => {
                        if this.page.ready().is_none() {
                            this.page = Loaded::Failed(error.cause_fr().into());
                        } else {
                            push_toast(cx, &this.state, ToastKind::Error, error.cause_fr());
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn cached(&self, cx: &gpui_kit::App) -> Option<PlaylistPage> {
        let state = self.state.read(cx);
        let cache = state.cache.as_ref()?;
        let api = state.api.as_ref()?;
        let owner = api.owner()?;
        let body = cache.get(&owner, &self.path())?;
        serde_json::from_str::<PlaylistPage>(&body).ok()
    }

    /// Take the page the server (or the disk) gave, keeping the reader's place:
    /// only the rows that changed are replaced, and the selection stays on its
    /// row when it still exists.
    fn adopt(&mut self, page: PlaylistPage, cx: &mut Context<Self>) {
        self.tracks = page.tracks.clone();
        self.page = Loaded::Ready(page);
        self.recompute();
        self.clamp_selection();
        cx.notify();
    }

    /// Whether a sheet is open (a test's way to see the click landed).
    pub fn members_visible(&self) -> bool {
        self.members.is_some()
    }

    /// The same for the export panel.
    pub fn export_visible(&self) -> bool {
        self.export.is_some()
    }

    /// How many rows the page holds (the acceptance's 4 000).
    pub fn row_len(&self) -> usize {
        self.tracks.len()
    }

    fn playlist(&self) -> Option<&Playlist> {
        self.page.ready().map(|page| &page.playlist)
    }

    fn version(&self) -> Option<u32> {
        self.playlist().map(|playlist| playlist.version)
    }

    fn editable(&self) -> bool {
        self.playlist().is_some_and(|playlist| playlist.editable)
    }

    fn can_manage(&self) -> bool {
        self.playlist().is_some_and(|playlist| playlist.can_manage)
    }

    /// Adopt the version a write answered with, so the next queued write is not
    /// stale before it leaves.
    fn adopt_version(&mut self, value: &serde_json::Value) {
        let Some(page) = self.page.ready().cloned() else {
            return;
        };
        let mut page = page;
        if let Some(version) = value.get("version").and_then(serde_json::Value::as_u64) {
            page.playlist.version = version as u32;
        }
        if let Some(version) = value
            .get("members_version")
            .and_then(serde_json::Value::as_u64)
        {
            page.playlist.members_version = version as u32;
        }
        if let Some(count) = value.get("nb_tracks").and_then(serde_json::Value::as_u64) {
            page.playlist.nb_tracks = count as u32;
        }
        self.page = Loaded::Ready(page);
    }

    /// One request, answered on the UI thread.
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

    /// A refusal: a conflict re-reads once, a lost right drops edit mode, a
    /// vanished playlist leaves the page. Everything else keeps its cause.
    fn failed(&mut self, error: &nz_api::ApiError, what: &str, cx: &mut Context<Self>) {
        if is_conflict(error) {
            push_toast(
                cx,
                &self.state,
                ToastKind::Info,
                "La playlist a changé ailleurs : elle est rechargée.",
            );
            self.load(cx);
            return;
        }
        if is_forbidden(error) {
            self.editing = false;
            self.error = Some("Vous n'avez plus le droit de modifier cette playlist.".into());
            self.load(cx);
            cx.notify();
            return;
        }
        if is_missing(error) {
            self.gone = true;
            self.page = Loaded::Failed("Cette playlist ne vous est plus accessible.".into());
            cx.notify();
            return;
        }
        push_toast(
            cx,
            &self.state,
            ToastKind::Error,
            format!("{what} : {}", error.cause_fr()),
        );
    }

    // -- the projected list ------------------------------------------------

    fn recompute(&mut self) {
        self.projection = project(&self.tracks, self.sort, self.descending, &self.query);
    }

    fn manual_view(&self) -> bool {
        self.sort == SortKey::Default && self.query.trim().is_empty()
    }

    fn clamp_selection(&mut self) {
        let count = if self.manual_view() {
            self.tracks.len()
        } else {
            self.projection.len()
        };
        self.selected = if count == 0 {
            0
        } else {
            self.selected.min(count - 1)
        };
    }

    /// The row index in `tracks` for a visible row.
    fn track_index(&self, visible: usize) -> Option<usize> {
        if self.manual_view() {
            (visible < self.tracks.len()).then_some(visible)
        } else {
            self.projection.get(visible).copied()
        }
    }

    fn move_selection(&mut self, delta: isize, page: bool) {
        let count = if self.manual_view() {
            self.tracks.len()
        } else {
            self.projection.len()
        };
        if count == 0 {
            return;
        }
        let step = if page { 10 } else { 1 };
        let target = if delta.is_negative() {
            self.selected.saturating_sub(delta.unsigned_abs() * step)
        } else {
            self.selected
                .saturating_add((delta as usize).saturating_mul(step))
        };
        self.selected = target.min(count - 1);
        self.scroll
            .scroll_to_item(self.selected, ScrollStrategy::Center);
    }

    // -- transport ---------------------------------------------------------

    fn play(&mut self, start: usize, shuffle: bool, cx: &mut Context<Self>) {
        let entries: Vec<Entry> = self
            .ordered_indices()
            .into_iter()
            .filter_map(|index| self.tracks.get(index))
            .map(entry_from_track)
            .collect();
        if entries.is_empty() {
            return;
        }
        let context = Some(PlayerContext {
            kind: "playlist".into(),
            id: Some(self.id.clone()),
            title: self.playlist().map(|playlist| playlist.title.clone()),
        });
        let start = start.min(entries.len() - 1);
        let command = if shuffle {
            Command::ShufflePlay {
                args: ShuffleArgs {
                    tracks: entries,
                    context,
                },
            }
        } else {
            Command::PlayQueue {
                args: QueueArgs {
                    tracks: entries,
                    start,
                    context,
                },
            }
        };
        self.state
            .update(cx, |state, cx| state.command(command, cx));
        cx.notify();
    }

    fn ordered_indices(&self) -> Vec<usize> {
        if self.manual_view() {
            (0..self.tracks.len()).collect()
        } else {
            self.projection.clone()
        }
    }

    // -- edits -------------------------------------------------------------

    /// The optimistic reorder. The list moves at once; one `PUT /order` leaves
    /// 350 ms after the last move (the SPA's debounce), and a failure says the
    /// order was not kept.
    fn reorder(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if !self.editable() || from == to || from >= self.tracks.len() || to >= self.tracks.len() {
            return;
        }
        let track = self.tracks.remove(from);
        self.tracks.insert(to, track);
        self.recompute();
        self.selected = to;
        self.pending_order = Some(self.tracks.iter().map(reference_of).collect());
        self.arm_order(cx);
        cx.notify();
    }

    fn arm_order(&mut self, cx: &mut Context<Self>) {
        if self.order_armed {
            return;
        }
        self.order_armed = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(350))
                .await;
            this.update(cx, |this, cx| {
                this.order_armed = false;
                this.flush_order(cx);
            })
            .ok();
        })
        .detach();
    }

    fn flush_order(&mut self, cx: &mut Context<Self>) {
        let Some(order) = self.pending_order.take() else {
            return;
        };
        if order.is_empty() {
            return;
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let id = self.id.clone();
        let version = self.version();
        self.request(
            cx,
            async move { api.reorder_playlist(&id, &order, version).await },
            |this, result, cx| match result {
                Ok(value) => {
                    this.adopt_version(&value);
                    forget(this, cx, &this.path());
                }
                Err(error) => this.failed(&error, "Ordre non enregistré", cx),
            },
        );
    }

    /// Remove one ROW (the SPA's `removeAt`): the server resolves the index
    /// against its own order, so a pending reorder is sent first.
    fn remove(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.editable() || index >= self.tracks.len() {
            return;
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.flush_order(cx);
        let removed = self.tracks.remove(index);
        self.recompute();
        self.clamp_selection();
        let id = self.id.clone();
        let version = self.version();
        self.request(
            cx,
            async move {
                api.remove_playlist_indexes(&id, &[index as u32], version)
                    .await
            },
            move |this, result, cx| match result {
                Ok(value) => {
                    this.adopt_version(&value);
                    forget(this, cx, &this.path());
                    push_toast(
                        cx,
                        &this.state,
                        ToastKind::Info,
                        format!("« {} » retiré de la playlist.", removed.title),
                    );
                }
                Err(error) => {
                    let at = index.min(this.tracks.len());
                    this.tracks.insert(at, removed.clone());
                    this.recompute();
                    this.failed(&error, "Retrait impossible", cx);
                }
            },
        );
    }

    fn save_meta(&mut self, cx: &mut Context<Self>) {
        let (Some(title), Some(description)) =
            (self.rename_title.clone(), self.rename_description.clone())
        else {
            return;
        };
        let title = title.read(cx).value().trim().to_owned();
        if title.is_empty() {
            self.error = Some("Le nom ne peut pas être vide.".into());
            cx.notify();
            return;
        }
        let description = description.read(cx).value().trim().to_owned();
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let id = self.id.clone();
        let version = self.version();
        let previous = self
            .playlist()
            .map(|playlist| (playlist.title.clone(), playlist.description.clone()));
        if let Some(page) = self.page.ready().cloned() {
            let mut page = page;
            page.playlist.title = title.clone();
            page.playlist.description = Some(description.clone());
            self.page = Loaded::Ready(page);
        }
        self.renaming = false;
        self.request(
            cx,
            async move {
                api.edit_playlist(
                    &id,
                    &json!({ "title": title, "description": description }),
                    version,
                )
                .await
            },
            move |this, result, cx| match result {
                Ok(value) => {
                    this.adopt_version(&value);
                    forget(this, cx, &this.path());
                    forget(this, cx, "/me/playlists");
                    push_toast(cx, &this.state, ToastKind::Info, "Playlist renommée.");
                }
                Err(error) => {
                    if let (Some(page), Some((title, description))) =
                        (this.page.ready().cloned(), previous.clone())
                    {
                        let mut page = page;
                        page.playlist.title = title;
                        page.playlist.description = description;
                        this.page = Loaded::Ready(page);
                    }
                    this.failed(&error, "Renommage impossible", cx);
                }
            },
        );
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let id = self.id.clone();
        let version = self.version();
        self.confirm_delete = false;
        self.request(
            cx,
            async move { api.delete_playlist(&id, version).await },
            |this, result, cx| match result {
                Ok(()) => {
                    forget(this, cx, "/me/playlists");
                    push_toast(cx, &this.state, ToastKind::Info, "Playlist supprimée.");
                    this.back(cx);
                }
                Err(error) => this.failed(&error, "Suppression impossible", cx),
            },
        );
    }

    fn leave(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let id = self.id.clone();
        self.confirm_leave = false;
        self.request(
            cx,
            async move { api.leave_playlist(&id).await },
            |this, result, cx| match result {
                Ok(()) => {
                    forget(this, cx, "/me/playlists");
                    push_toast(
                        cx,
                        &this.state,
                        ToastKind::Info,
                        "Vous avez quitté la playlist.",
                    );
                    this.back(cx);
                }
                Err(error) if is_conflict(&error) => {
                    this.error = Some("Le propriétaire ne peut pas quitter sa playlist.".into());
                    cx.notify();
                }
                Err(error) => this.failed(&error, "Impossible de quitter la playlist", cx),
            },
        );
    }

    /// « Télécharger » : ask the server to archive this playlist (rule 2: the
    /// work is a server job; the answer says how much it queued).
    fn archive(&mut self, cx: &mut Context<Self>) {
        if self.rec_busy {
            return;
        }
        let ids: Vec<String> = self
            .tracks
            .iter()
            .filter(|track| !track.local && track.source.as_deref() != Some("youtube"))
            .map(|track| track.deezer_id.as_str().to_owned())
            .collect();
        if ids.is_empty() {
            push_toast(
                cx,
                &self.state,
                ToastKind::Info,
                "Rien à archiver : ces fichiers sont déjà sur le serveur.",
            );
            return;
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.rec_busy = true;
        self.request(
            cx,
            async move { api.download(ids, "bulk").await },
            |this, result, cx| {
                this.rec_busy = false;
                match result {
                    Ok(answer) => push_toast(
                        cx,
                        &this.state,
                        ToastKind::Info,
                        format!(
                            "Archivage lancé : {} titres en file, {} refusés (file pleine).",
                            answer.queued, answer.dropped
                        ),
                    ),
                    Err(error) => push_toast(cx, &this.state, ToastKind::Error, error.cause_fr()),
                }
                cx.notify();
            },
        );
    }

    // -- recommendations ----------------------------------------------------

    fn load_recommendations(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let id = self.id.clone();
        self.recommendations = Loaded::Loading;
        self.request(
            cx,
            async move { api.playlist_recommendations(&id, 12).await },
            |this, result, cx| {
                this.recommendations = match result {
                    Ok(answer) => {
                        let existing: Vec<&str> = this
                            .tracks
                            .iter()
                            .map(|track| track.deezer_id.as_str())
                            .collect();
                        Loaded::Ready(
                            answer
                                .tracks
                                .into_iter()
                                .filter(|track| !existing.contains(&track.deezer_id.as_str()))
                                .collect(),
                        )
                    }
                    Err(error) => Loaded::Failed(error.cause_fr().into()),
                };
                cx.notify();
            },
        );
    }

    fn add_recommendation(&mut self, track: Track, cx: &mut Context<Self>) {
        if !self.editable() || self.rec_busy {
            return;
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let reference = reference_of(&track);
        let id = self.id.clone();
        let version = self.version();
        // Optimistic: the row joins the playlist below at once, and the
        // suggestions lose it.
        self.tracks.push(track.clone());
        if let Loaded::Ready(list) = &mut self.recommendations {
            list.retain(|row| row.deezer_id != track.deezer_id);
        }
        self.recompute();
        self.rec_busy = true;
        let restore = track.clone();
        self.request(
            cx,
            async move { api.add_to_playlist(&id, &[reference], version).await },
            move |this, result, cx| {
                this.rec_busy = false;
                match result {
                    Ok(value) => {
                        this.adopt_version(&value);
                        forget(this, cx, &this.path());
                        push_toast(
                            cx,
                            &this.state,
                            ToastKind::Info,
                            format!("« {} » ajouté à la playlist.", restore.title),
                        );
                    }
                    Err(error) => {
                        if let Some(at) = this
                            .tracks
                            .iter()
                            .position(|row| row.deezer_id == restore.deezer_id)
                        {
                            this.tracks.remove(at);
                            this.recompute();
                            if let Loaded::Ready(list) = &mut this.recommendations {
                                list.insert(0, restore.clone());
                            }
                        }
                        this.failed(&error, "Ajout impossible", cx);
                    }
                }
                cx.notify();
            },
        );
    }

    fn add_all_recommendations(&mut self, cx: &mut Context<Self>) {
        let Some(list) = self.recommendations.ready().cloned() else {
            return;
        };
        if list.is_empty() || self.rec_busy || !self.editable() {
            return;
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let refs: Vec<String> = list.iter().map(reference_of).collect();
        let id = self.id.clone();
        let version = self.version();
        self.tracks.extend(list.iter().cloned());
        self.recommendations = Loaded::Ready(Vec::new());
        self.recompute();
        self.rec_busy = true;
        let count = list.len();
        let restore = list.clone();
        self.request(
            cx,
            async move { api.add_to_playlist(&id, &refs, version).await },
            move |this, result, cx| {
                this.rec_busy = false;
                match result {
                    Ok(value) => {
                        this.adopt_version(&value);
                        forget(this, cx, &this.path());
                        push_toast(
                            cx,
                            &this.state,
                            ToastKind::Info,
                            format!("{count} titres ajoutés à la playlist."),
                        );
                    }
                    Err(error) => {
                        // A batch that half landed is re-read, never re-sent.
                        this.recommendations = Loaded::Ready(restore.clone());
                        this.failed(&error, "Ajout impossible", cx);
                        this.load(cx);
                    }
                }
                cx.notify();
            },
        );
    }

    // -- sheets ------------------------------------------------------------

    fn open_members(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(playlist) = self.playlist().cloned() else {
            return;
        };
        let answer = MembersAnswer {
            members: playlist.members.clone(),
            members_version: playlist.members_version,
            role: playlist.role.clone(),
            can_manage: playlist.can_manage,
            can_edit: playlist.editable,
            admin: playlist.admin,
        };
        let me = self
            .state
            .read(cx)
            .user_name()
            .unwrap_or_default()
            .to_owned();
        let state = self.state.clone();
        let sheet = cx.new(|cx| {
            MembersSheet::new(
                state,
                self.id.clone(),
                playlist.title.clone(),
                &answer,
                me,
                window,
                cx,
            )
        });
        cx.subscribe(&sheet, |this, _, event: &SheetEvent, cx| match event {
            SheetEvent::Close => {
                this.members = None;
                forget(this, cx, "/me/playlists");
                this.load(cx);
                cx.notify();
            }
            SheetEvent::Members(answer) => {
                if let Some(page) = this.page.ready().cloned() {
                    let mut page = page;
                    page.playlist.members = answer.members.clone();
                    page.playlist.members_version = answer.members_version;
                    page.playlist.role = answer.role.clone();
                    page.playlist.can_manage = answer.can_manage;
                    page.playlist.editable = answer.can_edit;
                    page.playlist.admin = answer.admin;
                    if !answer.can_edit {
                        this.editing = false;
                    }
                    this.page = Loaded::Ready(page);
                }
                cx.notify();
            }
            SheetEvent::Added { .. } => {}
        })
        .detach();
        self.members = Some(sheet);
        cx.notify();
    }

    fn open_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(playlist) = self.playlist().cloned() else {
            return;
        };
        let state = self.state.clone();
        let sheet = cx.new(|cx| {
            ExportSheet::new(
                state,
                ExportKind::Playlist,
                self.id.clone(),
                playlist.title.clone(),
                playlist.can_manage,
                window,
                cx,
            )
        });
        cx.subscribe(&sheet, |this, _, event: &SheetEvent, cx| {
            if matches!(event, SheetEvent::Close) {
                this.export = None;
                cx.notify();
            }
        })
        .detach();
        self.export = Some(sheet);
        cx.notify();
    }

    fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(playlist) = self.playlist().cloned() else {
            return;
        };
        if let (Some(title), Some(description)) =
            (self.rename_title.clone(), self.rename_description.clone())
        {
            title.update(cx, |state, cx| {
                state.set_value(playlist.title.clone(), window, cx)
            });
            description.update(cx, |state, cx| {
                state.set_value(playlist.description.clone().unwrap_or_default(), window, cx)
            });
            title.update(cx, |state, cx| state.focus(window, cx));
        }
        self.renaming = true;
        cx.notify();
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.nav.back();
            cx.notify();
        });
    }

    fn goto(&mut self, route: Route, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.nav.push(route);
            cx.notify();
        });
    }

    // -- rows --------------------------------------------------------------

    fn row(&self, index: usize, cx: &mut Context<Self>) -> gpui_kit::Stateful<Div> {
        let Some(track) = self.tracks.get(index) else {
            return div().id(ElementId::Name(format!("titre-{index}").into()));
        };
        let playing = self
            .state
            .read(cx)
            .player
            .current()
            .map(|entry| entry.id.clone())
            == Some(entry_from_track(track).id);
        let visible = if self.manual_view() {
            index
        } else {
            self.projection
                .iter()
                .position(|row| *row == index)
                .unwrap_or(usize::MAX)
        };
        let selected = visible == self.selected;
        let editing = self.editing;
        let label = if editing {
            format!("ligne-{index}")
        } else {
            format!("titre-{index}")
        };
        let mut row = div()
            .id(ElementId::Name(label.into()))
            .debug_selector({
                let key = format!("playlist-row-{visible}");
                move || key.clone()
            })
            .flex()
            .items_center()
            .gap(rems(0.5))
            .w_full()
            .h(px(if editing { EDIT_ROW } else { ROW }))
            .px(rems(0.5))
            .when(selected, |row| row.bg(theme::selected(cx)))
            .when(
                self.dragging.is_some()
                    && self.drag_over == Some(index)
                    && self.dragging != Some(index),
                |row| row.bg(theme::accent(cx).opacity(0.16)),
            )
            .hover(|style| style.bg(theme::selected(cx)));
        if editing {
            row = row
                .cursor_grab()
                .child(
                    icon(IconName::GripVertical)
                        .size(rems(0.875))
                        .text_color(theme::secondary(cx)),
                )
                .on_mouse_down(
                    gpui_kit::MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.dragging = Some(index);
                        this.drag_over = Some(index);
                        cx.notify();
                    }),
                )
                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                    if this.dragging.is_some() && this.drag_over != Some(index) {
                        this.drag_over = Some(index);
                        cx.notify();
                    }
                }))
                .on_mouse_up(
                    gpui_kit::MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        let from = this.dragging.take();
                        this.drag_over = None;
                        if let Some(from) = from {
                            this.reorder(from, index, cx);
                        }
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .w(rems(2.))
                        .flex_shrink_0()
                        .text_size(rems(0.625))
                        .text_color(theme::secondary(cx))
                        .child(format!("{}", index + 1)),
                );
        } else {
            row = row
                .cursor_pointer()
                .on_click(
                    cx.listener(move |this, event: &gpui_kit::ClickEvent, _, cx| {
                        this.selected = visible.min(this.tracks.len().saturating_sub(1));
                        if event.click_count() > 1 {
                            this.play(this.selected, false, cx);
                        }
                        cx.notify();
                    }),
                )
                .child(
                    icon_button(
                        ElementId::Name(format!("playlist-play-{index}").into()),
                        if playing {
                            IconName::AudioLines
                        } else {
                            IconName::Play
                        },
                        format!("Lire {}", track.title),
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = visible;
                        this.play(visible, false, cx);
                    })),
                );
        }
        row = row.child(track_cells(track, cx)).child(
            div()
                .w(rems(3.5))
                .flex_shrink_0()
                .text_size(rems(0.6875))
                .text_color(theme::secondary(cx))
                .child(format_duration(track.duration)),
        );
        if editing {
            let can_up = index > 0;
            let can_down = index + 1 < self.tracks.len();
            row = row
                .child(
                    icon_button(
                        ElementId::Name(format!("playlist-up-{index}").into()),
                        IconName::ChevronUp,
                        "Monter",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(!can_up)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.reorder(index, index.saturating_sub(1), cx);
                    })),
                )
                .child(
                    icon_button(
                        ElementId::Name(format!("playlist-down-{index}").into()),
                        IconName::ChevronDown,
                        "Descendre",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(!can_down)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.reorder(index, index + 1, cx);
                    })),
                )
                .child(
                    icon_button(
                        ElementId::Name(format!("playlist-remove-{index}").into()),
                        IconName::Minus,
                        format!("Retirer {}", track.title),
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.remove(index, cx))),
                );
        }
        row
    }

    fn list_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let count = if self.manual_view() {
            self.tracks.len()
        } else {
            self.projection.len()
        };
        if count == 0 {
            return if self.tracks.is_empty() {
                div()
                    .id("playlist-empty")
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(rems(0.5))
                    .flex_1()
                    .text_size(rems(0.875))
                    .text_color(theme::secondary(cx))
                    .child("Cette playlist est vide.")
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .child("« Modifier » puis les flèches rangent ; les suggestions du bas l'alimentent."),
                    )
                    .into_any_element()
            } else {
                div()
                    .id("playlist-no-match")
                    .p(rems(1.))
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(format!("Aucun titre ne correspond à « {} ».", self.query))
                    .into_any_element()
            };
        }
        let editor = cx.entity();
        let mut list = div()
            .id("playlist-rows")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        // The virtual list keeps a 4 000-row playlist at the display rate; in
        // edit mode the rows are drawn directly, because every row is a drop
        // target and a whole-list drag surface (the SPA's own reason).
        if self.editing {
            for visible in 0..count {
                if let Some(index) = self.track_index(visible) {
                    list = list.child(self.row(index, cx));
                }
            }
            let _ = editor;
            return list.into_any_element();
        }
        let sizes: Rc<Vec<Size<gpui_kit::Pixels>>> = Rc::new(
            (0..count)
                .map(|_| Size {
                    width: px(0.),
                    height: px(ROW),
                })
                .collect(),
        );
        let view = gpui_kit::base::v_virtual_list(
            editor,
            ElementId::Name("playlist-list".into()),
            sizes,
            move |this, range, _window, cx| {
                let mut rows = Vec::with_capacity(range.len());
                for visible in range {
                    if let Some(index) = this.track_index(visible) {
                        rows.push(this.row(index, cx));
                    }
                }
                rows
            },
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0();
        view.into_any_element()
    }
}

fn is_missing(error: &nz_api::ApiError) -> bool {
    matches!(error, nz_api::ApiError::Http { status: 404, .. })
}

fn forget(screen: &mut Screen, cx: &mut Context<Screen>, path: &str) {
    if let Some(api) = screen.state.read(cx).api() {
        api.forget(path);
    }
}

impl Render for Screen {
    #[allow(refining_impl_trait)]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.ensure_fields(window, cx);

        if let Some(cause) = self.page.failed().cloned() {
            return div()
                .id("playlist-failed")
                .flex()
                .flex_col()
                .size_full()
                .items_center()
                .justify_center()
                .gap(rems(0.5))
                .child(
                    icon(IconName::CircleAlert)
                        .size(rems(1.75))
                        .text_color(theme::danger(cx)),
                )
                .child(
                    div()
                        .text_size(rems(0.9375))
                        .font_weight(FontWeight::BOLD)
                        .child("Playlist indisponible"),
                )
                .child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child(cause),
                )
                .child(
                    button("playlist-retry", "Réessayer", ButtonVariant::Outline, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                )
                .into_any_element();
        }
        let Some(playlist) = self.playlist().cloned() else {
            return div()
                .id("playlist-loading")
                .flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child("Chargement de la playlist…")
                .into_any_element();
        };

        let total: u32 = self.tracks.iter().map(|track| track.duration).sum();
        let editable = playlist.editable;
        let can_manage = playlist.can_manage;
        let members = playlist.members.clone();
        let role = playlist.role.clone();
        let member_count = members.len();

        // One meta column: the facts, the title, the description — the sleeve
        // beside it. The title must not be a second flex child, or it lands to
        // the right of the facts (seen at 1280×800 on the real server).
        let facts = format!(
            "Playlist · {} · {} titre{} · {}{}",
            playlist.owner.clone().unwrap_or_else(|| "vous".to_owned()),
            self.tracks.len(),
            if self.tracks.len() > 1 { "s" } else { "" },
            format_duration(total),
            if member_count > 1 {
                format!(" · {member_count} collaborateurs")
            } else {
                String::new()
            }
        );
        let mut meta = div()
            .flex()
            .flex_col()
            .gap(rems(0.25))
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .text_size(rems(0.625))
                    .text_color(theme::secondary(cx))
                    .child(facts),
            );
        if self.renaming {
            let fields: Vec<AnyElement> =
                [self.rename_title.clone(), self.rename_description.clone()]
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(|(index, field)| {
                        let id = if index == 0 {
                            "playlist-title-input"
                        } else {
                            "playlist-description-input"
                        };
                        input(id, &field, window, cx).into_any_element()
                    })
                    .collect();
            for field in fields {
                meta = meta.child(field);
            }
            meta = meta.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.375))
                    .child(
                        button(
                            "playlist-save-meta",
                            "Enregistrer",
                            ButtonVariant::Primary,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.save_meta(cx))),
                    )
                    .child(
                        button(
                            "playlist-cancel-meta",
                            "Annuler",
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.renaming = false;
                            cx.notify();
                        })),
                    ),
            );
        } else {
            meta = meta.child(
                div()
                    .text_size(rems(1.75))
                    .font_weight(FontWeight::BOLD)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(playlist.title.clone()),
            );
            if let Some(description) = playlist.description.clone().filter(|text| !text.is_empty())
            {
                meta = meta.child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child(description),
                );
            }
        }
        let mut banner = div()
            .id("playlist-banner")
            .flex()
            .items_end()
            .gap(rems(1.25))
            .flex_shrink_0()
            .pb(rems(0.75))
            .child(sleeve(playlist.cover.as_deref(), &playlist.title, 128., cx))
            .child(meta);
        if member_count > 1 {
            let mut stack = div().flex().items_center().gap(rems(0.125));
            for member in members.iter().take(3) {
                stack = stack.child(face(&member.name, 22., cx));
            }
            if member_count > 3 {
                stack = stack.child(
                    div()
                        .text_size(rems(0.625))
                        .text_color(theme::secondary(cx))
                        .child(format!("+{}", member_count - 3)),
                );
            }
            banner = banner.child(
                div()
                    .id("playlist-members")
                    .cursor_pointer()
                    .px(rems(0.375))
                    .py(rems(0.25))
                    .hover(|style| style.bg(theme::selected(cx)))
                    .on_click(cx.listener(|this, _, window, cx| this.open_members(window, cx)))
                    .child(stack),
            );
        }

        let mut actions = div()
            .id("playlist-actions")
            .flex()
            .items_center()
            .gap(rems(0.375))
            .flex_shrink_0()
            .pb(rems(0.5))
            .child(
                button("playlist-play", "Lire", ButtonVariant::Primary, cx)
                    .child(icon(IconName::Play).size(rems(0.75)))
                    .on_click(cx.listener(|this, _, _, cx| this.play(0, false, cx))),
            )
            .child(
                button("playlist-shuffle", "Mélanger", ButtonVariant::Secondary, cx)
                    .child(icon(IconName::Shuffle).size(rems(0.75)))
                    .on_click(cx.listener(|this, _, _, cx| this.play(0, true, cx))),
            )
            .child(
                button(
                    "playlist-download",
                    "Télécharger",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.archive(cx))),
            )
            .child(
                div().debug_selector(|| "playlist-export".to_owned()).child(
                    button("playlist-export", "Exporter", ButtonVariant::Secondary, cx)
                        .child(icon(IconName::Share2).size(rems(0.75)))
                        .on_click(cx.listener(|this, _, window, cx| this.open_export(window, cx))),
                ),
            );
        if editable {
            actions = actions.child(
                button(
                    "playlist-edit",
                    if self.editing { "Terminé" } else { "Modifier" },
                    if self.editing {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    },
                    cx,
                )
                .debug_selector(|| "playlist-edit".to_owned())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.editing = !this.editing;
                    this.recompute();
                    cx.notify();
                })),
            );
        }
        if can_manage {
            actions = actions
                .child(
                    button("playlist-rename", "Renommer", ButtonVariant::Secondary, cx)
                        .child(icon(IconName::Pencil).size(rems(0.75)))
                        .on_click(cx.listener(|this, _, window, cx| this.start_rename(window, cx))),
                )
                .child(if self.confirm_delete {
                    button(
                        "playlist-delete",
                        "Confirmer la suppression",
                        ButtonVariant::Danger,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.delete(cx)))
                } else {
                    button(
                        "playlist-delete-ask",
                        "Supprimer",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_delete = true;
                        cx.notify();
                    }))
                });
        } else if role.as_deref().is_some_and(|role| role != "owner") {
            actions = actions.child(if self.confirm_leave {
                button("playlist-leave", "Confirmer", ButtonVariant::Danger, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.leave(cx)))
            } else {
                button(
                    "playlist-leave-ask",
                    "Quitter",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.confirm_leave = true;
                    cx.notify();
                }))
            });
        }
        if member_count > 0 {
            actions = actions.child(
                div()
                    .debug_selector(|| "playlist-members-open".to_owned())
                    .child(
                        button(
                            "playlist-members-open",
                            "Collaborateurs",
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .child(icon(IconName::Users).size(rems(0.75)))
                        .on_click(cx.listener(|this, _, window, cx| this.open_members(window, cx))),
                    ),
            );
        }
        actions = actions.child(div().flex_1()).child(
            div()
                .text_size(rems(0.625))
                .text_color(theme::secondary(cx))
                .child(if editable {
                    "Alt+↑↓ pour déplacer"
                } else {
                    "Lecture seule"
                }),
        );

        let mut tools = div()
            .id("playlist-tools")
            .flex()
            .items_center()
            .gap(rems(0.375))
            .flex_shrink_0()
            .pb(rems(0.375));
        if let Some(filter) = self.filter.clone() {
            tools = tools.child(div().w(rems(18.)).child(input(
                "playlist-filter",
                &filter,
                window,
                cx,
            )));
        }
        tools = tools
            .child(
                button(
                    "playlist-sort",
                    format!("Tri : {}", self.sort.label()),
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.sort = this.sort.next();
                    this.recompute();
                    this.clamp_selection();
                    cx.notify();
                })),
            )
            .child(
                button(
                    "playlist-desc",
                    if self.descending {
                        "Décroissant"
                    } else {
                        "Croissant"
                    },
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.descending = !this.descending;
                    this.recompute();
                    cx.notify();
                })),
            );

        let mut root = div()
            .id("playlist-screen")
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .p(rems(1.))
            .on_mouse_up(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.dragging = None;
                    this.drag_over = None;
                }),
            );

        root = root
            .child(banner)
            .child(actions)
            .child(tools)
            .child(self.list_body(cx));

        // The bottom of the playlist: what the library suggests next. It loads
        // AFTER the list is on screen and never moves it.
        if editable && !self.editing && self.query.trim().is_empty() {
            let mut recs = div()
                .id("playlist-recommendations")
                .flex()
                .flex_col()
                .gap(rems(0.25))
                .flex_shrink_0()
                .pt(rems(0.75))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.375))
                        .child(
                            div()
                                .text_size(rems(0.875))
                                .font_weight(FontWeight::BOLD)
                                .child("Titres recommandés"),
                        )
                        .child(div().flex_1())
                        .children(
                            self.recommendations
                                .ready()
                                .is_some_and(|list| list.len() > 1)
                                .then(|| {
                                    button(
                                        "playlist-add-recs",
                                        "Tout ajouter",
                                        ButtonVariant::Secondary,
                                        cx,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| this.add_all_recommendations(cx),
                                    ))
                                }),
                        ),
                );
            match &self.recommendations {
                Loaded::Loading => recs = recs.child(note_line("Recherche de suggestions…", cx)),
                Loaded::Failed(cause) => {
                    recs = recs.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.5))
                            .child(
                                div()
                                    .text_size(rems(0.75))
                                    .text_color(theme::danger(cx))
                                    .child(format!("Suggestions indisponibles : {cause}")),
                            )
                            .child(
                                button(
                                    "playlist-recs-retry",
                                    "Réessayer",
                                    ButtonVariant::Secondary,
                                    cx,
                                )
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.load_recommendations(cx)),
                                ),
                            ),
                    );
                }
                Loaded::Ready(list) if list.is_empty() => {
                    recs = recs.child(note_line("Pas de suggestion pour l'instant.", cx))
                }
                Loaded::Ready(list) => {
                    let mut rows = div().flex().flex_col().gap(rems(0.125));
                    for track in list.iter().take(12) {
                        let target = track.clone();
                        rows = rows.child(
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
                                )
                                .child(
                                    icon_button(
                                        ElementId::Name(
                                            format!("playlist-add-{}", track.deezer_id.as_str())
                                                .into(),
                                        ),
                                        IconName::Plus,
                                        format!("Ajouter {}", track.title),
                                        ButtonVariant::Secondary,
                                        cx,
                                    )
                                    .disabled(self.rec_busy)
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.add_recommendation(target.clone(), cx)
                                        },
                                    )),
                                ),
                        );
                    }
                    recs = recs.child(rows);
                }
            }
            root = root.child(recs);
        }

        if let Some(error) = self.error.clone() {
            root = root.child(
                div()
                    .flex_shrink_0()
                    .pt(rems(0.375))
                    .child(error_line(&error, cx)),
            );
        }
        if let Some(members) = self.members.clone() {
            root = root.child(members);
        }
        if let Some(export) = self.export.clone() {
            root = root.child(export);
        }
        root.into_any_element()
    }
}

impl std::fmt::Debug for Screen {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlaylistScreen")
            .field("id", &self.id)
            .field("tracks", &self.tracks.len())
            .field("editing", &self.editing)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, deezer: &str) -> Track {
        Track {
            deezer_id: nz_model::TrackId::new(deezer),
            title: title.to_owned(),
            duration: 100,
            ..Track::default()
        }
    }

    #[test]
    fn a_reorder_moves_the_row_the_person_grabbed_and_sends_references() {
        // The arithmetic `moveItem` uses, on the refs the server wants.
        let tracks = vec![track("Un", "1"), track("Deux", "2"), track("Trois", "3")];
        let mut order: Vec<Track> = tracks.clone();
        let moved = order.remove(2);
        order.insert(0, moved);
        assert_eq!(
            order.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["Trois", "Un", "Deux"]
        );
        assert_eq!(
            order.iter().map(reference_of).collect::<Vec<_>>(),
            ["3", "1", "2"]
        );
    }
}

#[cfg(test)]
mod journey {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use gpui_kit::{Entity, IntoElement, ParentElement, Render, Styled, div};

    use super::*;
    use crate::state::{AppConfig, event_channel};

    /// What the stand-in server was asked, in order: `("PUT", "/api/playlist/pl-1/order",
    /// Some(body))`. The screen's whole effect on the world is in here.
    type Log = Arc<Mutex<Vec<(String, String, Option<String>)>>>;

    const PAGE: &str = r#"{"playlist":{"id":"pl-1","title":"Route","nb_tracks":3,
        "version":1,"editable":true,"can_manage":true,"role":"owner",
        "members":[{"id":"u1","name":"bench","role":"owner"}],"members_version":1},
        "tracks":[
          {"id":"row-1","deezer_id":"3000001","title":"Un","duration":100},
          {"id":"row-2","deezer_id":"3000002","title":"Deux","duration":100},
          {"id":"row-3","deezer_id":"3000003","title":"Trois","duration":100}]}"#;

    /// A server that serves the page and REFUSES the reorder with the `409` a
    /// stale version gets (`playlist.py`), recording every request.
    fn stub() -> (String, Log) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("local address");
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let thread_log = Arc::clone(&log);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0u8; 16384];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let line = request.lines().next().unwrap_or_default().to_owned();
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or_default().to_owned();
                let path = parts.next().unwrap_or_default().to_owned();
                let body = request
                    .split("\r\n\r\n")
                    .nth(1)
                    .filter(|body| !body.is_empty())
                    .map(str::to_owned);
                thread_log
                    .lock()
                    .expect("log")
                    .push((method.clone(), path.clone(), body));
                let (status, payload) = if path == "/api/playlist/pl-1/order" {
                    (
                        "409 Conflict",
                        r#"{"error":"conflict","scope":"content"}"#.to_owned(),
                    )
                } else if path == "/api/playlist/pl-1" {
                    ("200 OK", PAGE.to_owned())
                } else if path == "/api/playlist/pl-1/recommendations" {
                    (
                        "200 OK",
                        r#"{"playlist":{"id":"pl-1"},"tracks":[]}"#.to_owned(),
                    )
                } else if path == "/api/me/playlists" {
                    ("200 OK", r#"{"folders":[],"playlists":[]}"#.to_owned())
                } else if path == "/api/version" {
                    ("200 OK", r#"{"version":"9.9.9"}"#.to_owned())
                } else {
                    (
                        "200 OK",
                        r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#
                            .to_owned(),
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                     Set-Cookie: session=abc; Path=/\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (format!("http://{address}"), log)
    }

    fn temporary(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neezer-d8b-playlist-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
    }

    fn app_state(
        cx: &mut gpui::TestAppContext,
        dir: &std::path::Path,
        server: String,
        runtime: &tokio::runtime::Runtime,
    ) -> Entity<AppState> {
        let (tx, _rx) = event_channel();
        let config = AppConfig {
            server: Some(server),
            start_audio: false,
            start_mpris: false,
            ..AppConfig::for_test(dir)
        };
        let handle = runtime.handle().clone();
        let state = cx.update(|cx| cx.new(move |_| AppState::new(config, handle, tx, None)));
        let login_server = cx.update(|cx| state.read(cx).server.clone());
        cx.update(|cx| {
            state.update(cx, |state, _| {
                state.login(login_server, "bench".into(), "secret".into());
            });
        });
        state
    }

    /// Drive the current-thread runtime from the test thread until `done`.
    fn wait_for(
        runtime: &tokio::runtime::Runtime,
        cx: &mut gpui::TestAppContext,
        what: &str,
        mut done: impl FnMut(&mut gpui::TestAppContext) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            runtime.block_on(async {
                tokio::time::sleep(Duration::from_millis(1)).await;
            });
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
        }
    }

    fn ids(screen: &Screen) -> Vec<&str> {
        screen
            .tracks
            .iter()
            .map(|track| track.title.as_str())
            .collect()
    }

    /// `Alt+↓` moves the row AT ONCE, one `PUT /order` leaves after the
    /// debounce, and the server's `409` puts the list back where it was and says
    /// why — never a silent overwrite, never a replay.
    #[gpui::test]
    async fn alt_down_moves_the_row_and_a_refusal_puts_it_back(cx: &mut gpui::TestAppContext) {
        let dir = temporary("reorder");
        let (server, log) = stub();
        cx.update(gpui_omarchy::init);
        let runtime = runtime();
        let state = app_state(cx, &dir, server, &runtime);
        let screen = cx.update(|cx| {
            cx.new(|cx| {
                Screen::new(
                    Route::Playlist {
                        id: "pl-1".to_owned(),
                    },
                    state.clone(),
                    cx,
                )
            })
        });
        let probe = screen.clone();
        wait_for(&runtime, cx, "the page", |cx| {
            cx.update(|cx| probe.read(cx).row_len()) == 3
        });
        // Edit mode is the gesture's precondition: the SPA only offers a reorder
        // on the playlist's own order.
        cx.update(|cx| {
            screen.update(cx, |this, _| {
                this.editing = true;
                this.selected = 0;
            })
        });
        cx.update(|cx| cx.dispatch_action(&MoveRowDown));
        cx.update(|cx| {
            assert_eq!(
                ids(screen.read(cx)),
                ["Deux", "Un", "Trois"],
                "the list moves in the frame of the gesture"
            );
        });

        // The debounce, then the write.
        cx.executor().advance_clock(Duration::from_millis(400));
        cx.run_until_parked();
        let wrote = Arc::clone(&log);
        wait_for(&runtime, cx, "the order write", move |_| {
            wrote
                .lock()
                .expect("log")
                .iter()
                .any(|(method, path, _)| method == "PUT" && path == "/api/playlist/pl-1/order")
        });
        cx.update(|_| {
            let log = log.lock().expect("log");
            let body = log
                .iter()
                .find(|(method, path, _)| method == "PUT" && path == "/api/playlist/pl-1/order")
                .and_then(|(_, _, body)| body.clone())
                .expect("a body");
            assert!(
                body.contains(r#""tracks":["row-2","row-1","row-3"]"#),
                "the write carries the order the person sees: {body}"
            );
        });

        // The refusal reloads the page (a second GET) and says so.
        let reloaded = Arc::clone(&log);
        wait_for(&runtime, cx, "the reload", move |_| {
            reloaded
                .lock()
                .expect("log")
                .iter()
                .filter(|(method, path, _)| method == "GET" && path == "/api/playlist/pl-1")
                .count()
                >= 2
        });
        cx.update(|cx| {
            let screen = screen.read(cx);
            assert_eq!(
                ids(screen),
                ["Un", "Deux", "Trois"],
                "the server's order is back on screen"
            );
            let toasts = &state.read(cx).toasts;
            assert!(
                toasts
                    .items()
                    .iter()
                    .any(|toast| toast.message.contains("changé ailleurs")),
                "the conflict is a sentence, not a silent revert: {:?}",
                toasts
                    .items()
                    .iter()
                    .map(|toast| toast.message.clone())
                    .collect::<Vec<_>>()
            );
        });
        std::fs::remove_dir_all(dir).ok();
    }

    /// The mouse alone: press a row, move over another, release — the same
    /// reorder, through the same write.
    #[gpui::test]
    async fn dragging_a_row_moves_it(cx: &mut gpui::TestAppContext) {
        struct Harness(Entity<Screen>);

        impl Render for Harness {
            fn render(
                &mut self,
                _window: &mut Window,
                _cx: &mut Context<Self>,
            ) -> impl IntoElement {
                div().size_full().child(self.0.clone())
            }
        }

        let dir = temporary("drag");
        let (server, _log) = stub();
        cx.update(gpui_omarchy::init);
        let runtime = runtime();
        let state = app_state(cx, &dir, server, &runtime);
        let screen = cx.update(|cx| {
            cx.new(|cx| {
                Screen::new(
                    Route::Playlist {
                        id: "pl-1".to_owned(),
                    },
                    state.clone(),
                    cx,
                )
            })
        });
        let probe = screen.clone();
        wait_for(&runtime, cx, "the page", |cx| {
            cx.update(|cx| probe.read(cx).row_len()) == 3
        });
        cx.update(|cx| screen.update(cx, |this, _| this.editing = true));

        let harness = screen.clone();
        let (_harness, vcx) = cx.add_window_view(move |_, _| Harness(harness));
        vcx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        vcx.run_until_parked();
        let third = vcx
            .debug_bounds("playlist-row-2")
            .expect("the third row is laid out");
        let first = vcx
            .debug_bounds("playlist-row-0")
            .expect("the first row is laid out");
        vcx.simulate_mouse_down(
            third.center(),
            gpui_kit::MouseButton::Left,
            gpui_kit::Modifiers::default(),
        );
        vcx.simulate_mouse_move(
            third.center(),
            Some(gpui_kit::MouseButton::Left),
            gpui_kit::Modifiers::default(),
        );
        vcx.simulate_mouse_move(
            first.center(),
            Some(gpui_kit::MouseButton::Left),
            gpui_kit::Modifiers::default(),
        );
        vcx.simulate_mouse_up(
            first.center(),
            gpui_kit::MouseButton::Left,
            gpui_kit::Modifiers::default(),
        );
        vcx.run_until_parked();
        vcx.update(|_, cx| {
            assert_eq!(
                ids(screen.read(cx)),
                ["Trois", "Un", "Deux"],
                "the grabbed row landed where it was dropped"
            );
        });
        std::fs::remove_dir_all(dir).ok();
    }
}

#[cfg(test)]
mod sheets_journey {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use gpui_kit::{Entity, IntoElement, ParentElement, Render, Styled, div};

    use super::*;
    use crate::state::{AppConfig, event_channel};

    const PAGE: &str = r#"{"playlist":{"id":"pl-1","title":"Route","nb_tracks":2,
        "version":1,"editable":true,"can_manage":true,"role":"owner",
        "members":[{"id":"u1","name":"bench","role":"owner"}],"members_version":1},
        "tracks":[
          {"id":"row-1","deezer_id":"3000001","title":"Un","duration":100},
          {"id":"row-2","deezer_id":"3000002","title":"Deux","duration":100}]}"#;

    /// A server that serves the page, the public link and the list file. Every
    /// request is recorded, so a test can prove a file was really asked for.
    fn stub() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("local address");
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0u8; 16384];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let line = request.lines().next().unwrap_or_default().to_owned();
                log.lock().expect("log").push(line.clone());
                let path = line.split_whitespace().nth(1).unwrap_or_default();
                let (status, payload) = if path == "/api/playlist/pl-1" {
                    ("200 OK", PAGE.to_owned())
                } else if path == "/api/playlist/pl-1/share" {
                    ("200 OK", r#"{"link":null}"#.to_owned())
                } else if path.starts_with("/api/services/list/") {
                    ("200 OK", "title,artist\nUn,Kaya\nDeux,Kaya\n".to_owned())
                } else if path == "/api/playlist/pl-1/recommendations" {
                    (
                        "200 OK",
                        r#"{"playlist":{"id":"pl-1"},"tracks":[]}"#.to_owned(),
                    )
                } else if path == "/api/me/playlists" {
                    ("200 OK", r#"{"folders":[],"playlists":[]}"#.to_owned())
                } else if path == "/api/export/formats" {
                    ("200 OK", r#"{"default":"flac","formats":[{"id":"flac","label":"FLAC","ext":"zip"}]}"#.to_owned())
                } else if path == "/api/services" {
                    ("200 OK", r#"{"services":[],"formats":[]}"#.to_owned())
                } else if path == "/api/version" {
                    ("200 OK", r#"{"version":"9.9.9"}"#.to_owned())
                } else {
                    (
                        "200 OK",
                        r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#
                            .to_owned(),
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                     Set-Cookie: session=abc; Path=/\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{address}")
    }

    fn temporary(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neezer-d8b-sheets-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn wait_for(
        runtime: &tokio::runtime::Runtime,
        cx: &mut gpui::TestAppContext,
        what: &str,
        mut done: impl FnMut(&mut gpui::TestAppContext) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            runtime.block_on(async {
                tokio::time::sleep(Duration::from_millis(1)).await;
            });
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
        }
    }

    struct Harness(Entity<Screen>);

    impl Render for Harness {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.0.clone())
        }
    }

    /// Open a playlist page with a Harness, ready to click.
    fn opened(
        cx: &mut gpui::TestAppContext,
        name: &str,
    ) -> (Entity<Screen>, String, tokio::runtime::Runtime) {
        let dir = temporary(name);
        let server = stub();
        cx.update(gpui_omarchy::init);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (tx, _rx) = event_channel();
        let config = AppConfig {
            server: Some(server.clone()),
            start_audio: false,
            start_mpris: false,
            ..AppConfig::for_test(&dir)
        };
        let handle = runtime.handle().clone();
        let state = cx.update(|cx| cx.new(move |_| AppState::new(config, handle, tx, None)));
        let login_server = cx.update(|cx| state.read(cx).server.clone());
        cx.update(|cx| {
            state.update(cx, |state, _| {
                state.login(login_server, "bench".into(), "secret".into());
            });
        });
        let state_for_screen = state.clone();
        let screen = cx.update(|cx| {
            cx.new(|cx| {
                Screen::new(
                    Route::Playlist {
                        id: "pl-1".to_owned(),
                    },
                    state_for_screen,
                    cx,
                )
            })
        });
        let probe = screen.clone();
        wait_for(&runtime, cx, "the page", |cx| {
            cx.update(|cx| probe.read(cx).row_len()) == 2
        });
        (screen, dir.display().to_string(), runtime)
    }

    /// The collaborators sheet opens from the page's own payload, and the export
    /// panel offers the public link, the archive and the list file — and a list
    /// download REALLY writes the file (to the test's own data directory).
    #[gpui::test]
    async fn the_collaborators_sheet_and_the_export_panel_open_and_a_list_downloads(
        cx: &mut gpui::TestAppContext,
    ) {
        let (screen, dir, runtime) = opened(cx, "sheets");
        let harness = screen.clone();
        let (_harness, vcx) = cx.add_window_view(move |_, _| Harness(harness));
        vcx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        vcx.run_until_parked();

        // The export panel first (the page is freshly drawn).
        let export = vcx
            .debug_bounds("playlist-export")
            .expect("the export button is laid out");
        vcx.simulate_click(export.center(), gpui_kit::Modifiers::default());
        vcx.run_until_parked();
        vcx.update(|_, cx| assert!(screen.read(cx).export_visible()));

        // The list file, through the app's own download path.
        let download = vcx
            .debug_bounds("export-download-list")
            .expect("the download button");
        vcx.simulate_click(download.center(), gpui_kit::Modifiers::default());
        let target = std::path::Path::new(&dir).join("data/Exports/Road.csv");
        let alt = std::path::Path::new(&dir).join("data/Exports");
        wait_for(&runtime, vcx, "the written list", |_| {
            std::fs::read_dir(&alt)
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .any(|entry| entry.path().extension().is_some_and(|e| e == "csv"))
                })
                .unwrap_or(false)
        });
        let written = std::fs::read_dir(&alt)
            .expect("the export directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|extension| extension == "csv"))
            .expect("a CSV was written");
        let text = std::fs::read_to_string(&written).expect("the file");
        assert!(
            text.contains("Kaya"),
            "the server's CSV, written whole: {text}"
        );
        let _ = target;
        // The collaborators sheet, from the page's own payload.
        let close_export = vcx
            .debug_bounds("export-close")
            .expect("the export panel's close button");
        vcx.simulate_click(close_export.center(), gpui_kit::Modifiers::default());
        vcx.run_until_parked();
        vcx.update(|_, cx| assert!(!screen.read(cx).export_visible()));
        vcx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        vcx.run_until_parked();
        let members = vcx
            .debug_bounds("playlist-members-open")
            .expect("the collaborators button is laid out");
        vcx.simulate_click(members.center(), gpui_kit::Modifiers::default());
        vcx.run_until_parked();
        vcx.update(|_, cx| {
            assert!(
                screen.read(cx).members_visible(),
                "the sheet opened from the page's own members"
            );
        });
        std::fs::remove_dir_all(dir).ok();
    }
}
