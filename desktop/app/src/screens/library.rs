//! Bibliothèque: the playlists and their folders, the titles one keeps, the
//! artists and albums one follows, the files one uploaded, the titles one set
//! aside, the ones that vanished, and the folder tree.
//!
//! Three rules shape this screen:
//!
//! - **A list paints from the disk first.** `/me/favorites`, `/me/playlists`,
//!   `/me/artists`, `/me/albums` and `/me/local` are cacheable paths, so the
//!   screen parses the cached body in the frame it is built and asks the server
//!   right after (the SPA's `swr`). A 4 000-title favourites list therefore
//!   opens without a round trip — measured in the tests.
//! - **Only what is visible is built.** Every long list is a virtual list; a
//!   grid paints one screenful and says how to see more (rule 3: nothing fans
//!   out unbounded).
//! - **A mutation is optimistic and names its cause.** The folder orders are
//!   last-write-wins (`playlist_folders.py` says so): a move writes the whole
//!   container, and a refusal reloads what the server holds instead of
//!   pretending the move happened.
//!
//! The screen never edits the shell: it navigates by pushing a [`Route`] on
//! `AppState::nav` (the root view rebuilds the screen from it) and plays by
//! sending one [`nz_player::Command`] through `AppState::command`.

use std::cell::RefCell;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::base::VirtualListScrollHandle;
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, ElementId, Entity, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, ScrollStrategy, SharedString, Size,
    StatefulInteractiveElement as _, Styled, Subscription, Window, div,
    prelude::FluentBuilder as _, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon, icon_button, input};

use nz_model::{
    Album, Artist, MyAlbums, MyArtists, MyFavorites, MyLocal, Playlist, PlaylistFolder,
    PlaylistList, Track,
};
use nz_player::{Command, Context as PlayerContext, Entry, QueueArgs, ShuffleArgs, TracksArgs};
use serde::de::DeserializeOwned;

use crate::actions::*;
use crate::nav::{LibraryTab, Route};
use crate::state::{AppState, entry_from_track};
use crate::theme;
use crate::toast::ToastKind;
use crate::widgets::cover::{self, Covers, Kind};
use crate::widgets::download_control::{self, Rows, State as DownloadState, Watch};
use crate::widgets::folder_tree::{FolderTree, NodeKind, Target};
use crate::widgets::playlist_picker::{PlaylistPicker, fold};
use crate::widgets::sheets::{
    ExportKind, ExportSheet, SheetEvent, card, field_label, note_line, push_toast,
};
use crate::wiring::events::Sender;
use crate::wiring::offline::Choice;

/// What the screen's own tasks answer back: the file the device holds for a
/// sleeve, or `None` when it has none (the row keeps its placeholder).
enum Msg {
    Cover(String, Option<PathBuf>),
}

/// Row height of a title at the default rem: 56 keeps a 36 px sleeve and two
/// lines without crowding (the SPA's own `.rec`).
pub const ROW: f32 = 56.;

/// How many cards a grid paints before it offers "Afficher plus".
const GRID_STEP: usize = 60;

/// One screen's data as it arrives: cached or fresh, then possibly failed. A
/// failure keeps a cause a person can read, never a spinner.
pub enum Loaded<T> {
    Loading,
    Ready(T),
    Failed(SharedString),
}

impl<T> Loaded<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Loaded::Ready(value) => Some(value),
            _ => None,
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Loaded::Loading)
    }

    pub fn failed(&self) -> Option<&SharedString> {
        match self {
            Loaded::Failed(cause) => Some(cause),
            _ => None,
        }
    }
}

/// How a list of titles is ordered. `Default` is the server's own order (the
/// starred-at order for favourites, the playlist's order for a playlist).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    #[default]
    Default,
    Title,
    Artist,
    Album,
    Duration,
}

impl SortKey {
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Default => "Ordre d'origine",
            SortKey::Title => "Titre",
            SortKey::Artist => "Artiste",
            SortKey::Album => "Album",
            SortKey::Duration => "Durée",
        }
    }

    pub fn next(self) -> Self {
        match self {
            SortKey::Default => SortKey::Title,
            SortKey::Title => SortKey::Artist,
            SortKey::Artist => SortKey::Album,
            SortKey::Album => SortKey::Duration,
            SortKey::Duration => SortKey::Default,
        }
    }
}

/// `duration`: `m:ss`, or `h:mm:ss` past an hour — the SPA's own shape.
pub fn format_duration(seconds: u32) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// The line under a row's title: the credited artists, else the main one
/// (`format.js#artistLine`).
pub fn artist_line(track: &Track) -> String {
    if let Some(line) = track
        .display_artist
        .as_ref()
        .filter(|line| !line.is_empty())
    {
        return line.clone();
    }
    if track.artists.len() > 1 {
        return track
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
    }
    if !track.artist.name.is_empty() {
        return track.artist.name.clone();
    }
    "Artiste inconnu".to_owned()
}

/// The tint a sleeve or a card gets when no artwork is on disk. Derived from the
/// title alone, so the same album is the same colour after a restart.
pub fn tint(seed: &str, cx: &App) -> gpui_kit::Hsla {
    match seed.bytes().fold(0u32, |sum, byte| {
        sum.wrapping_mul(31).wrapping_add(byte as u32)
    }) % 4
    {
        0 => theme::accent(cx),
        1 => theme::success(cx),
        2 => theme::warning(cx),
        _ => theme::danger(cx),
    }
}

/// The sleeve, when the cache holds it; a tinted plate with a disc glyph
/// otherwise. D8a's `cover` widget replaces this with the decoded image — until
/// it lands, a screen must not reach into the cover cache itself.
pub fn sleeve(cover: Option<&str>, seed: &str, size: f32, cx: &App) -> Div {
    let _ = cover;
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .size(px(size))
        .bg(tint(seed, cx).opacity(0.18))
        .text_color(tint(seed, cx).opacity(0.9))
        .child(icon(IconName::Music).size(px(size * 0.4)))
}

/// The middle of a title's row: the sleeve, the title, the artist line.
///
/// `file` is the sleeve this row shows, when the screen has one for it: the
/// 128 px copy the list asked for (`Covers::rows`), painted as a real pochette
/// once `Covers::show` has decoded it — a 36 px tile never holds a 1 000 px
/// image. `None` is the placeholder (a row drawn outside the windowed list, or
/// one whose art is not on the device).
pub fn track_cells(track: &Track, file: Option<&PathBuf>, cx: &App) -> Div {
    let url = track.album.cover.as_deref();
    div()
        .flex()
        .items_center()
        .gap(rems(0.75))
        .flex_1()
        .min_w_0()
        .child(cover::view(
            file,
            url,
            &track.album.title,
            Kind::Album,
            px(36.),
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.25))
                        .min_w_0()
                        .child(
                            div()
                                .text_size(rems(0.8125))
                                .font_weight(FontWeight::MEDIUM)
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(track.title.clone()),
                        )
                        .children(track.explicit.then(|| {
                            div()
                                .px(rems(0.1875))
                                .text_size(rems(0.5))
                                .bg(theme::sidebar(cx))
                                .text_color(theme::secondary(cx))
                                .child("E")
                        }))
                        .children(track.unavailable.then(|| {
                            div()
                                .text_size(rems(0.5625))
                                .text_color(theme::danger(cx))
                                .child("indisponible")
                        })),
                )
                .child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(artist_line(track)),
                ),
        )
}

/// The order a query and a sort project a list into: the INDICES to draw, so a
/// 4 000-row list is never cloned to be sorted (`toolbar.js#createProjector`'s
/// rule, without the Rust index it builds for the same reason).
pub fn project(tracks: &[Track], sort: SortKey, descending: bool, query: &str) -> Vec<usize> {
    let needle = fold(query.trim());
    let mut rows: Vec<usize> = (0..tracks.len())
        .filter(|index| {
            let track = &tracks[*index];
            needle.is_empty()
                || fold(&track.title).contains(&needle)
                || fold(&artist_line(track)).contains(&needle)
                || fold(&track.album.title).contains(&needle)
        })
        .collect();
    if sort != SortKey::Default {
        rows.sort_by(|left, right| {
            let left = &tracks[*left];
            let right = &tracks[*right];
            let order = match sort {
                SortKey::Title => fold(&left.title).cmp(&fold(&right.title)),
                SortKey::Artist => fold(&artist_line(left)).cmp(&fold(&artist_line(right))),
                SortKey::Album => fold(&left.album.title).cmp(&fold(&right.album.title)),
                SortKey::Duration => left.duration.cmp(&right.duration),
                SortKey::Default => std::cmp::Ordering::Equal,
            };
            order.then_with(|| fold(&left.title).cmp(&fold(&right.title)))
        });
    }
    if descending {
        rows.reverse();
    }
    rows
}

/// The row heights a virtual list is told: one uniform 56 px row per title.
fn row_sizes(count: usize) -> Rc<Vec<Size<gpui_kit::Pixels>>> {
    Rc::new(
        (0..count)
            .map(|_| Size {
                width: px(0.),
                height: px(ROW),
            })
            .collect(),
    )
}

/// The reference a mutation names a track by (`trackkey.js#trackRef`: the row's
/// UUID when there is one, else the Deezer id).
pub fn reference_of(track: &Track) -> String {
    track
        .id
        .as_ref()
        .map(|id| id.as_str().to_owned())
        .unwrap_or_else(|| track.deezer_id.as_str().to_owned())
}

/// The one-line prompt the screen opens for a new or renamed thing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Prompt {
    #[default]
    Closed,
    Playlist,
    Folder,
    RenameFolder,
}

pub struct Screen {
    route: Route,
    state: Entity<AppState>,
    tab: LibraryTab,
    filter: Option<Entity<InputState>>,
    prompt: Option<Entity<InputState>>,
    prompt_kind: Prompt,
    /// The filter's text, mirrored on each change so a projection is never
    /// recomputed through a borrow of the editor during a paint.
    query: String,
    sort: SortKey,
    descending: bool,
    favourites: Loaded<Vec<Track>>,
    playlists: Loaded<PlaylistList>,
    artists: Loaded<Vec<Artist>>,
    albums: Loaded<Vec<Album>>,
    uploads: Loaded<Vec<Track>>,
    dislikes: Loaded<Vec<Track>>,
    unavailable: Loaded<Vec<Track>>,
    /// The projected indices of the current list, rebuilt when its data, its
    /// query or its sort changes — never per paint.
    projection: Vec<usize>,
    /// The row the keyboard is on.
    selected: usize,
    /// How many cards a grid paints.
    grid_limit: usize,
    scroll: VirtualListScrollHandle,
    tree: FolderTree,
    /// The folder organiser: folders, or one container's playlists.
    organizing: Option<Target>,
    picker: Option<Entity<PlaylistPicker>>,
    export: Option<Entity<ExportSheet>>,
    busy: bool,
    /// The sleeves the list and the shelves draw: a 128 px copy per row, and
    /// the images decoded for what is on screen only (`cover::Covers::show`).
    covers: Rc<RefCell<Covers>>,
    /// The downloads worker, as this screen reads it (lot D18).
    downloads: Watch,
    rows: Rc<Rows>,
    rows_version: u64,
    /// The rows the last paint built, for the tests that hold the windowing and
    /// the sleeve requests to their promise.
    visible: Range<usize>,
    events: Sender<Msg>,
    _subscriptions: Vec<Subscription>,
}

impl Screen {
    /// The shell's constructor: no window, so the text fields a tab needs are
    /// made on the first paint (the screen is built inside `screens::build`).
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let downloads = Watch::attach(&state, cx);
        let tab = match &route {
            Route::Library { tab } => *tab,
            _ => LibraryTab::default(),
        };
        let (events, answers) = crate::wiring::events::channel::<Msg>();
        let mut this = Self {
            route,
            state,
            tab,
            filter: None,
            prompt: None,
            prompt_kind: Prompt::Closed,
            query: String::new(),
            sort: SortKey::Default,
            descending: false,
            favourites: Loaded::Loading,
            playlists: Loaded::Loading,
            artists: Loaded::Loading,
            albums: Loaded::Loading,
            uploads: Loaded::Loading,
            dislikes: Loaded::Loading,
            unavailable: Loaded::Loading,
            projection: Vec::new(),
            selected: 0,
            grid_limit: GRID_STEP,
            scroll: VirtualListScrollHandle::new(),
            tree: FolderTree::new(),
            organizing: None,
            picker: None,
            export: None,
            busy: false,
            covers: Rc::new(RefCell::new(Covers::rows())),
            downloads,
            rows: Rc::new(Rows::default()),
            rows_version: u64::MAX,
            visible: 0..0,
            events,
            _subscriptions: Vec::new(),
        };
        this._subscriptions
            .push(cx.observe(&this.state.clone(), |this, _, cx| {
                this.sync_tab(cx);
                cx.notify();
            }));
        this.drain(answers, cx);
        // The screen's sleeves are the process's, but the HOLD is this
        // screen's: leaving the library drops the images it was showing (and
        // their atlas tiles) instead of keeping them until the window dies.
        cx.on_release(|this, cx| this.covers.borrow_mut().release(cx))
            .detach();
        this.bind_list_actions(cx);
        this.bind_download(cx);
        this.load(cx);
        this
    }

    /// The sleeve answers, on the screen's own thread.
    fn drain(&mut self, answers: crate::wiring::events::Receiver<Msg>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(msg) = answers.next().await {
                let Msg::Cover(url, file) = msg;
                if this
                    .update(cx, |this, cx| {
                        this.covers.borrow_mut().resolved(&url, file);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    // --- downloads (lot D18) ------------------------------------------------

    /// The favourites' ids: the one list this screen downloads to the device
    /// (the SPA's Library only archives; the desktop does both).
    fn download_keys(&self) -> Vec<String> {
        self.titles()
            .map(|(_, tracks)| {
                tracks
                    .iter()
                    .map(download_control::key)
                    .filter(|key| !key.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The favourites are downloaded as the set `("favorites", "me")`: the store
    /// keys a set by `(kind, ident)`, and « mes favoris » has no server id.
    fn download_state(&mut self) -> DownloadState {
        let keys = self.download_keys();
        download_control::collection(self.downloads.get(), "favorites", "me", &keys)
    }

    fn bind_download(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let for_download = weak.clone();
        gpui_kit::App::on_action(cx, move |_: &DownloadSelection, cx: &mut gpui_kit::App| {
            for_download
                .update(cx, |this, cx| {
                    if download_control::is_front(&this.state, &this.route, cx) {
                        this.download(cx);
                    }
                })
                .ok();
        });
        gpui_kit::App::on_action(cx, move |_: &RemoveDownload, cx: &mut gpui_kit::App| {
            weak.update(cx, |this, cx| {
                if download_control::is_front(&this.state, &this.route, cx) {
                    this.drop_download(cx);
                }
            })
            .ok();
        });
    }

    fn download(&mut self, cx: &mut Context<Self>) {
        let Some(service) = self.downloads.service().cloned() else {
            push_toast(
                cx,
                &self.state,
                ToastKind::Error,
                "Les téléchargements ne sont pas disponibles sans stockage local.",
            );
            return;
        };
        match self.download_state() {
            DownloadState::Failed { failed, .. } => {
                service.retry_failures();
                push_toast(
                    cx,
                    &self.state,
                    ToastKind::Info,
                    format!(
                        "Nouvelle tentative pour {failed} titre{}",
                        if failed > 1 { "s" } else { "" }
                    ),
                );
            }
            DownloadState::Idle => {
                let choices: Vec<_> = self
                    .titles()
                    .map(|(_, tracks)| tracks)
                    .unwrap_or_default()
                    .iter()
                    .filter(|track| !download_control::key(track).is_empty())
                    .map(|track| Choice::new(download_control::key(track), track.title.clone()))
                    .collect();
                if choices.is_empty() {
                    push_toast(
                        cx,
                        &self.state,
                        ToastKind::Error,
                        "Aucun titre à télécharger dans cette liste.",
                    );
                    return;
                }
                let count = choices.len();
                service.choose("favorites", "me", "Favoris", choices);
                push_toast(
                    cx,
                    &self.state,
                    ToastKind::Info,
                    format!(
                        "{count} titre{} en file de téléchargement",
                        if count > 1 { "s" } else { "" }
                    ),
                );
            }
            DownloadState::Queued { .. }
            | DownloadState::Running { .. }
            | DownloadState::Done { .. } => {}
        }
    }

    fn drop_download(&mut self, cx: &mut Context<Self>) {
        let Some(service) = self.downloads.service().cloned() else {
            return;
        };
        match self.download_state() {
            DownloadState::Failed { failed, .. } => {
                service.retry_failures();
                push_toast(
                    cx,
                    &self.state,
                    ToastKind::Info,
                    format!(
                        "Nouvelle tentative pour {failed} titre{}",
                        if failed > 1 { "s" } else { "" }
                    ),
                );
            }
            DownloadState::Queued { .. }
            | DownloadState::Running { .. }
            | DownloadState::Done { .. } => {
                service.drop_set("favorites", "me");
                push_toast(
                    cx,
                    &self.state,
                    ToastKind::Info,
                    "Téléchargement des favoris retiré",
                );
            }
            DownloadState::Idle => {
                push_toast(
                    cx,
                    &self.state,
                    ToastKind::Info,
                    "Les favoris ne sont pas téléchargés sur cet appareil.",
                );
            }
        }
    }

    /// The list's verbs, bound to THIS screen and NOT to the keyboard focus.
    ///
    /// A screen must not steal the shell's focus anchor: GPUI looks a keystroke
    /// up along the path of the focused node, and once a screen that took focus
    /// is unmounted the anchor is gone and every global binding (Space, `q`,
    /// `g h`…) silently stops working — measured by D7's own journey test the
    /// first time this screen focused itself. So the actions are registered on
    /// the app, each holding a weak handle: they fire for the screen that is
    /// alive, and a dropped screen's handle simply answers `Err`.
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
            let count = this.row_count();
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
        on!(PlayFromHere, |this: &mut Self, cx: &mut Context<Self>| this
            .activate(cx));
        on!(AddHereToQueue, |this: &mut Self, cx: &mut Context<Self>| {
            this.add_selection_to_queue(cx)
        });
        on!(LikeHere, |this: &mut Self, cx: &mut Context<Self>| {
            this.like_selection(cx)
        });
        on!(DislikeHere, |this: &mut Self, cx: &mut Context<Self>| {
            this.dislike_selection(cx)
        });
        on!(Refresh, |this: &mut Self, cx: &mut Context<Self>| this
            .load(cx));
        on!(ImportPlaylist, |this: &mut Self, cx: &mut Context<Self>| {
            this.goto(Route::Import { url: None }, cx)
        });
        on!(MoveRowUp, |this: &mut Self, cx: &mut Context<Self>| {
            if matches!(this.tab, LibraryTab::Folders | LibraryTab::Playlists) {
                this.shift_tree(-1, cx);
            }
        });
        on!(MoveRowDown, |this: &mut Self, cx: &mut Context<Self>| {
            if matches!(this.tab, LibraryTab::Folders | LibraryTab::Playlists) {
                this.shift_tree(1, cx);
            }
        });
        install_row_keys(cx);
    }

    /// The fields are made on the first paint, when a window exists.
    fn ensure_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter.is_some() {
            return;
        }
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filtrer…"));
        self._subscriptions
            .push(cx.subscribe(&filter, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query = this
                        .filter
                        .as_ref()
                        .map(|filter| filter.read(cx).value().to_string())
                        .unwrap_or_default();
                    this.selected = 0;
                    this.recompute(cx);
                    cx.notify();
                }
            }));
        let prompt = cx.new(|cx| InputState::new(window, cx).placeholder("Nom"));
        self._subscriptions
            .push(cx.subscribe(&prompt, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.commit_prompt(cx);
                }
            }));
        self.filter = Some(filter);
        self.prompt = Some(prompt);
    }

    /// A new route on the same screen (a tab reached from the sidebar) adopts
    /// the tab and asks for what that tab shows.
    fn sync_tab(&mut self, cx: &mut Context<Self>) {
        let current = self.state.read(cx).nav.current().cloned();
        let Some(Route::Library { tab }) = current else {
            return;
        };
        if tab != self.tab {
            self.tab = tab;
            self.selected = 0;
            self.organizing = None;
            self.load(cx);
        }
    }

    // -- loading ------------------------------------------------------------

    fn load(&mut self, cx: &mut Context<Self>) {
        match self.tab {
            LibraryTab::Favorites => self.load_favourites(cx),
            LibraryTab::Playlists | LibraryTab::Folders => self.load_playlists(cx),
            LibraryTab::Artists => self.load_artists(cx),
            LibraryTab::Albums => self.load_albums(cx),
            LibraryTab::Uploads => self.load_uploads(cx),
            LibraryTab::Dislikes => self.load_dislikes(cx),
            LibraryTab::Unavailable => self.load_unavailable(cx),
        }
        if self.tab != LibraryTab::Playlists {
            // The tab strip's playlist count and the folder tree read this.
            self.load_playlists(cx);
        }
    }

    /// A cached body, parsed in the frame the screen is built. `None` when the
    /// cache is empty or holds a shape this build no longer reads (rule 7: an
    /// older copy is dropped, never served).
    fn cached<T: DeserializeOwned>(&self, cx: &App, path: &str) -> Option<T> {
        let state = self.state.read(cx);
        let cache = state.cache.as_ref()?;
        let api = state.api.as_ref()?;
        let owner = api.owner()?;
        let body = cache.get(&owner, path)?;
        serde_json::from_str::<T>(&body).ok()
    }

    fn load_favourites(&mut self, cx: &mut Context<Self>) {
        if let Some(list) = self.cached::<MyFavorites>(cx, "/me/favorites") {
            self.favourites = Loaded::Ready(list.tracks);
            self.recompute(cx);
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.request(
            cx,
            move |_| {
                let api = api.clone();
                async move { api.my_favorites().await }
            },
            |this, result, cx| {
                match result {
                    Ok(list) => {
                        this.favourites = Loaded::Ready(list.tracks);
                        this.recompute(cx);
                    }
                    Err(error) => {
                        if this.favourites.ready().is_none() {
                            this.favourites = Loaded::Failed(error.cause_fr().into());
                        } else {
                            push_toast(cx, &this.state, ToastKind::Error, error.cause_fr());
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    fn load_playlists(&mut self, cx: &mut Context<Self>) {
        if let Some(list) = self.cached::<PlaylistList>(cx, "/me/playlists") {
            self.adopt_playlists(list, cx);
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.request(
            cx,
            move |_| {
                let api = api.clone();
                async move { api.my_playlists().await }
            },
            |this, result, cx| {
                match result {
                    Ok(list) => this.adopt_playlists(list, cx),
                    Err(error) => {
                        if this.playlists.ready().is_none() {
                            this.playlists = Loaded::Failed(error.cause_fr().into());
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    fn adopt_playlists(&mut self, list: PlaylistList, cx: &mut Context<Self>) {
        self.tree.rebuild(&list.playlists, &list.folders);
        self.playlists = Loaded::Ready(list);
        self.clamp_selection();
        cx.notify();
    }

    fn load_artists(&mut self, cx: &mut Context<Self>) {
        if let Some(list) = self.cached::<MyArtists>(cx, "/me/artists") {
            self.artists = Loaded::Ready(list.artists);
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.request(
            cx,
            move |_| {
                let api = api.clone();
                async move { api.my_artists().await }
            },
            |this, result, cx| {
                match result {
                    Ok(list) => this.artists = Loaded::Ready(list.artists),
                    Err(error) => {
                        if this.artists.ready().is_none() {
                            this.artists = Loaded::Failed(error.cause_fr().into());
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    fn load_albums(&mut self, cx: &mut Context<Self>) {
        if let Some(list) = self.cached::<MyAlbums>(cx, "/me/albums") {
            self.albums = Loaded::Ready(list.albums);
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.request(
            cx,
            move |_| {
                let api = api.clone();
                async move { api.my_albums().await }
            },
            |this, result, cx| {
                match result {
                    Ok(list) => this.albums = Loaded::Ready(list.albums),
                    Err(error) => {
                        if this.albums.ready().is_none() {
                            this.albums = Loaded::Failed(error.cause_fr().into());
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    fn load_uploads(&mut self, cx: &mut Context<Self>) {
        if let Some(list) = self.cached::<MyLocal>(cx, "/me/local") {
            self.uploads = Loaded::Ready(list.tracks);
            self.recompute(cx);
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.request(
            cx,
            move |_| {
                let api = api.clone();
                async move { api.my_local().await }
            },
            |this, result, cx| {
                match result {
                    Ok(list) => {
                        this.uploads = Loaded::Ready(list.tracks);
                        this.recompute(cx);
                    }
                    Err(error) => {
                        if this.uploads.ready().is_none() {
                            this.uploads = Loaded::Failed(error.cause_fr().into());
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    fn load_dislikes(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.request(
            cx,
            move |_| {
                let api = api.clone();
                async move { api.my_dislikes().await }
            },
            |this, result, cx| {
                match result {
                    Ok(list) => {
                        this.dislikes = Loaded::Ready(list.tracks);
                        this.recompute(cx);
                    }
                    Err(error) => {
                        if this.dislikes.ready().is_none() {
                            this.dislikes = Loaded::Failed(error.cause_fr().into());
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    fn load_unavailable(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.request(
            cx,
            move |_| {
                let api = api.clone();
                async move { api.unavailable_tracks().await }
            },
            |this, result, cx| {
                match result {
                    Ok(list) => {
                        this.unavailable = Loaded::Ready(list.tracks);
                        this.recompute(cx);
                    }
                    Err(error) => {
                        if this.unavailable.ready().is_none() {
                            this.unavailable = Loaded::Failed(error.cause_fr().into());
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    /// Run one request on the network runtime and hand its answer back to this
    /// screen. One path, so a screen cannot quietly touch a second one.
    fn request<T, F, Fut, G>(&self, cx: &mut Context<Self>, build: F, done: G)
    where
        T: Send + 'static,
        F: FnOnce(&mut Context<Self>) -> Fut,
        Fut: std::future::Future<Output = T> + Send + 'static,
        G: FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    {
        let future = build(cx);
        let runtime = self.state.read(cx).runtime.clone();
        let task = runtime.spawn(future);
        cx.spawn(async move |this, cx| {
            if let Ok(answer) = task.await {
                this.update(cx, |this, cx| done(this, answer, cx)).ok();
            }
        })
        .detach();
    }

    // -- the list the current tab draws ------------------------------------

    /// The titles of the current tab, with the context kind a queue records.
    fn titles(&self) -> Option<(&'static str, &[Track])> {
        match self.tab {
            LibraryTab::Favorites => self
                .favourites
                .ready()
                .map(|tracks| ("favorites", tracks.as_slice())),
            LibraryTab::Uploads => self
                .uploads
                .ready()
                .map(|tracks| ("local", tracks.as_slice())),
            LibraryTab::Dislikes => self
                .dislikes
                .ready()
                .map(|tracks| ("dislikes", tracks.as_slice())),
            LibraryTab::Unavailable => self
                .unavailable
                .ready()
                .map(|tracks| ("unavailable", tracks.as_slice())),
            _ => None,
        }
    }

    /// Rebuild the projection after a load, a filter keystroke or a sort.
    fn recompute(&mut self, cx: &mut Context<Self>) {
        self.projection = match self.titles() {
            Some((_, tracks)) => project(tracks, self.sort, self.descending, &self.query),
            None => Vec::new(),
        };
        self.clamp_selection();
        let _ = cx;
    }

    fn projected_len(&self) -> usize {
        self.projection.len()
    }

    /// The rows the last paint built (a test's number, and what the windowing
    /// claim is about).
    pub fn visible_rows(&self) -> Range<usize> {
        self.visible.clone()
    }

    /// How many rows the keyboard can walk, whatever the tab shows.
    fn row_count(&self) -> usize {
        match self.tab {
            LibraryTab::Playlists => self
                .playlists
                .ready()
                .map_or(0, |list| list.playlists.len()),
            LibraryTab::Folders => self.tree.len(),
            LibraryTab::Artists => self.artists.ready().map_or(0, Vec::len),
            LibraryTab::Albums => self.albums.ready().map_or(0, Vec::len),
            _ => self.projected_len(),
        }
    }

    fn clamp_selection(&mut self) {
        let count = self.row_count();
        if count == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(count - 1);
        }
    }

    fn move_selection(&mut self, delta: isize, page: bool) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let step = if page { 10 } else { 1 };
        let target = if delta.is_negative() {
            self.selected.saturating_sub((delta.unsigned_abs()) * step)
        } else {
            self.selected
                .saturating_add((delta as usize).saturating_mul(step))
        };
        self.selected = target.min(count - 1);
        self.scroll
            .scroll_to_item(self.selected, ScrollStrategy::Center);
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        self.scroll.scroll_to_item(index, ScrollStrategy::Center);
        cx.notify();
    }

    // -- transport ---------------------------------------------------------

    fn play(&mut self, start: usize, shuffle: bool, cx: &mut Context<Self>) {
        let Some((kind, tracks)) = self.titles() else {
            return;
        };
        let entries: Vec<Entry> = self
            .projection
            .iter()
            .filter_map(|index| tracks.get(*index))
            .map(entry_from_track)
            .collect();
        if entries.is_empty() {
            return;
        }
        let context = Some(PlayerContext {
            kind: kind.to_owned(),
            id: None,
            title: Some(self.tab.title().to_owned()),
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

    fn activate(&mut self, cx: &mut Context<Self>) {
        if self.tab == LibraryTab::Playlists {
            if let Some(list) = self.playlists.ready()
                && let Some(playlist) = list.playlists.get(self.selected)
            {
                self.goto(
                    Route::Playlist {
                        id: playlist.id.as_str().to_owned(),
                    },
                    cx,
                );
            }
            return;
        }
        if self.tab == LibraryTab::Folders {
            match self.tree.current().map(|node| node.kind.clone()) {
                Some(NodeKind::Playlist { id, .. }) => self.goto(Route::Playlist { id }, cx),
                Some(NodeKind::Folder { .. }) => {
                    self.tree.toggle();
                    self.refold();
                    cx.notify();
                }
                None => {}
            }
            return;
        }
        if self.tab == LibraryTab::Artists {
            let Some(artists) = self.artists.ready() else {
                return;
            };
            let shown = self.artists_shown(artists);
            if let Some(artist) = shown.get(self.selected) {
                let id = artist
                    .id
                    .clone()
                    .unwrap_or_else(|| artist.deezer_id.as_str().to_owned());
                self.goto(Route::Artist { id }, cx);
            }
            return;
        }
        if self.tab == LibraryTab::Albums {
            let Some(albums) = self.albums.ready() else {
                return;
            };
            let shown = self.albums_shown(albums);
            if let Some(album) = shown.get(self.selected) {
                let id = album
                    .id
                    .as_ref()
                    .map(|id| id.as_str().to_owned())
                    .unwrap_or_else(|| album.deezer_id.as_str().to_owned());
                self.goto(Route::Album { id }, cx);
            }
            return;
        }
        if self.tab == LibraryTab::Unavailable {
            // A vanished title's only useful gesture here: look for a
            // replacement, which is the search with what the server still knows.
            let Some(tracks) = self.unavailable.ready() else {
                return;
            };
            if let Some(track) = self
                .projection
                .get(self.selected)
                .and_then(|index| tracks.get(*index))
            {
                let query = format!("{} {}", artist_line(track), track.title);
                self.goto(
                    Route::Search {
                        query: query.trim().to_owned(),
                    },
                    cx,
                );
            }
            return;
        }
        // A list of titles: play the queue from the focused row.
        self.play(self.selected, false, cx);
    }

    fn add_selection_to_queue(&mut self, cx: &mut Context<Self>) {
        let Some((_, tracks)) = self.titles() else {
            return;
        };
        let Some(entry) = self
            .projection
            .get(self.selected)
            .and_then(|index| tracks.get(*index))
            .map(entry_from_track)
        else {
            return;
        };
        self.state.update(cx, |state, cx| {
            state.command(
                Command::AddToQueue {
                    args: TracksArgs {
                        tracks: vec![entry],
                    },
                },
                cx,
            );
        });
        push_toast(cx, &self.state, ToastKind::Info, "Titre ajouté à la file.");
    }

    /// `l`: on the favourites tab it takes the title OUT of them, everywhere
    /// else it stars it. Optimistic, with the row put back when the server says
    /// no — rule 4's "a click answers at once, then the network".
    fn like_selection(&mut self, cx: &mut Context<Self>) {
        let Some((kind, tracks)) = self.titles() else {
            return;
        };
        let Some(track) = self
            .projection
            .get(self.selected)
            .and_then(|index| tracks.get(*index))
            .cloned()
        else {
            return;
        };
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let reference = reference_of(&track);
        let removing = kind == "favorites";
        let on = !removing;
        if removing && let Loaded::Ready(list) = &mut self.favourites {
            list.retain(|row| row.id != track.id);
        }
        self.recompute(cx);
        let restore = track.clone();
        self.request(
            cx,
            move |_| {
                let api = api.clone();
                async move { api.favorite(&reference, on).await }
            },
            move |this, result, cx| {
                match result {
                    Ok(()) => {
                        forget(this, cx, "/me/favorites");
                        push_toast(
                            cx,
                            &this.state,
                            ToastKind::Info,
                            if on {
                                "Titre aimé."
                            } else {
                                "Titre retiré des favoris."
                            },
                        );
                    }
                    Err(error) => {
                        if !on && let Loaded::Ready(list) = &mut this.favourites {
                            list.insert(0, restore.clone());
                        }
                        this.recompute(cx);
                        push_toast(cx, &this.state, ToastKind::Error, error.cause_fr());
                    }
                }
                cx.notify();
            },
        );
    }

    /// `d`: set a title aside (`POST /api/dislike`), or bring it back from the
    /// « Titres écartés » tab.
    fn dislike_selection(&mut self, cx: &mut Context<Self>) {
        let Some((kind, tracks)) = self.titles() else {
            return;
        };
        let Some(track) = self
            .projection
            .get(self.selected)
            .and_then(|index| tracks.get(*index))
            .cloned()
        else {
            return;
        };
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let reference = reference_of(&track);
        let restoring = kind == "dislikes";
        let on = !restoring;
        if restoring && let Loaded::Ready(list) = &mut self.dislikes {
            list.retain(|row| row.id != track.id);
        }
        self.recompute(cx);
        let restore = track.clone();
        self.request(
            cx,
            move |_| {
                let api = api.clone();
                async move { api.dislike(&reference, on).await }
            },
            move |this, result, cx| {
                match result {
                    Ok(()) => push_toast(
                        cx,
                        &this.state,
                        ToastKind::Info,
                        if on {
                            "Titre écarté : il ne passera plus dans le Flow."
                        } else {
                            "Titre réintégré."
                        },
                    ),
                    Err(error) => {
                        if restoring && let Loaded::Ready(list) = &mut this.dislikes {
                            list.insert(0, restore.clone());
                        }
                        this.recompute(cx);
                        push_toast(cx, &this.state, ToastKind::Error, error.cause_fr());
                    }
                }
                cx.notify();
            },
        );
    }

    /// « Télécharger » : ask the SERVER to archive the list, then say what it
    /// queued. The work is a server job (`/api/download` answers at once), which
    /// is why this is a User-lane request and not a download here.
    fn archive(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some((_, tracks)) = self.titles() else {
            return;
        };
        let ids: Vec<String> = tracks
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
        self.busy = true;
        self.request(
            cx,
            move |_| {
                let api = api.clone();
                async move { api.download(ids, "bulk").await }
            },
            |this, result, cx| {
                this.busy = false;
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

    // -- navigation --------------------------------------------------------

    fn goto(&mut self, route: Route, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.nav.push(route);
            cx.notify();
        });
    }

    fn relay(&mut self, route: Route, cx: &mut Context<Self>) {
        self.route = route.clone();
        self.state.update(cx, |state, cx| {
            state.nav.replace(route);
            cx.notify();
        });
    }

    // -- the sheets --------------------------------------------------------

    fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, tracks)) = self.titles() else {
            return;
        };
        let refs: Vec<String> = self
            .projection
            .get(self.selected)
            .and_then(|index| tracks.get(*index))
            .map(|track| vec![reference_of(track)])
            .unwrap_or_default();
        if refs.is_empty() {
            return;
        }
        let state = self.state.clone();
        let picker = cx.new(|cx| PlaylistPicker::new(state, refs, "1 titre", window, cx));
        cx.subscribe(&picker, |this, _, event: &SheetEvent, cx| match event {
            SheetEvent::Close => {
                this.picker = None;
                cx.notify();
            }
            SheetEvent::Added { .. } => {
                this.picker = None;
                this.invalidate_playlists(cx);
                cx.notify();
            }
            SheetEvent::Members(_) => {}
        })
        .detach();
        self.picker = Some(picker);
        cx.notify();
    }

    fn open_export(
        &mut self,
        kind: ExportKind,
        id: String,
        title: String,
        can_manage: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.state.clone();
        let export = cx.new(|cx| ExportSheet::new(state, kind, id, title, can_manage, window, cx));
        cx.subscribe(&export, |this, _, event: &SheetEvent, cx| {
            if matches!(event, SheetEvent::Close) {
                this.export = None;
                cx.notify();
            }
        })
        .detach();
        self.export = Some(export);
        cx.notify();
    }

    /// « Exporter » on a list tab: the favourites are exported as a whole
    /// account (`me`), which is the only `favorites` target the route takes.
    fn active_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (kind, id, title) = match self.tab {
            LibraryTab::Favorites => (ExportKind::Favorites, "me".to_owned(), "Favoris".to_owned()),
            other => (
                ExportKind::Favorites,
                "me".to_owned(),
                other.title().to_owned(),
            ),
        };
        self.open_export(kind, id, title, false, window, cx);
    }

    fn invalidate_playlists(&mut self, cx: &mut Context<Self>) {
        forget(self, cx, "/me/playlists");
        self.load_playlists(cx);
    }

    // -- folders -----------------------------------------------------------

    /// Rebuild the tree from the data on screen (after a fold, or a move).
    fn refold(&mut self) {
        let playlists = self.playlists_ref().to_vec();
        let folders = self.folders().to_vec();
        self.tree.rebuild(&playlists, &folders);
    }

    fn folders(&self) -> &[PlaylistFolder] {
        self.playlists
            .ready()
            .map(|list| list.folders.as_slice())
            .unwrap_or(&[])
    }

    fn playlists_ref(&self) -> &[Playlist] {
        self.playlists
            .ready()
            .map(|list| list.playlists.as_slice())
            .unwrap_or(&[])
    }

    fn open_prompt(
        &mut self,
        kind: Prompt,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.prompt.clone() else {
            return;
        };
        prompt.update(cx, |state, cx| {
            state.set_value(initial, window, cx);
            state.focus(window, cx);
        });
        self.prompt_kind = kind;
        cx.notify();
    }

    fn commit_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(prompt) = self.prompt.clone() else {
            return;
        };
        let title = prompt.read(cx).value().trim().to_owned();
        let kind = self.prompt_kind;
        if title.is_empty() || kind == Prompt::Closed {
            return;
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.prompt_kind = Prompt::Closed;
        self.busy = true;
        let folder = match &self.organizing {
            Some(Target::Container(folder)) => folder.clone(),
            _ => None,
        };
        let focused_folder = self.tree.current().map(|node| node.kind.clone());
        match kind {
            Prompt::Playlist => {
                self.request(
                    cx,
                    move |_| {
                        let api = api.clone();
                        let title = title.clone();
                        async move { api.create_playlist(&title, &[], folder.as_deref()).await }
                    },
                    |this, result, cx| {
                        this.busy = false;
                        match result {
                            Ok(playlist) => {
                                this.invalidate_playlists(cx);
                                this.goto(
                                    Route::Playlist {
                                        id: playlist.id.as_str().to_owned(),
                                    },
                                    cx,
                                );
                            }
                            Err(error) => {
                                push_toast(cx, &this.state, ToastKind::Error, error.cause_fr())
                            }
                        }
                        cx.notify();
                    },
                );
            }
            Prompt::Folder => {
                self.request(
                    cx,
                    move |_| {
                        let api = api.clone();
                        let title = title.clone();
                        async move { api.create_playlist_folder(&title, None).await }
                    },
                    |this, result, cx| {
                        this.busy = false;
                        match result {
                            Ok(_) => this.invalidate_playlists(cx),
                            Err(error) => {
                                push_toast(cx, &this.state, ToastKind::Error, error.cause_fr())
                            }
                        }
                        cx.notify();
                    },
                );
            }
            Prompt::RenameFolder => {
                let Some(NodeKind::Folder { id, .. }) = focused_folder else {
                    self.busy = false;
                    return;
                };
                self.request(
                    cx,
                    move |_| {
                        let api = api.clone();
                        let title = title.clone();
                        async move { api.rename_playlist_folder(&id, &title).await }
                    },
                    |this, result, cx| {
                        this.busy = false;
                        match result {
                            Ok(()) => this.invalidate_playlists(cx),
                            Err(error) => {
                                push_toast(cx, &this.state, ToastKind::Error, error.cause_fr())
                            }
                        }
                        cx.notify();
                    },
                );
            }
            Prompt::Closed => self.busy = false,
        }
    }

    fn delete_folder(&mut self, cx: &mut Context<Self>) {
        let Some(NodeKind::Folder { id, .. }) = self.tree.current().map(|node| node.kind.clone())
        else {
            return;
        };
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.request(
            cx,
            move |_| {
                let api = api.clone();
                async move { api.delete_playlist_folder(&id).await }
            },
            |this, result, cx| {
                match result {
                    Ok(()) => {
                        push_toast(
                            cx,
                            &this.state,
                            ToastKind::Info,
                            "Dossier supprimé : ses playlists sont revenues dans « Sans dossier ».",
                        );
                        this.invalidate_playlists(cx);
                    }
                    Err(error) => push_toast(cx, &this.state, ToastKind::Error, error.cause_fr()),
                }
                cx.notify();
            },
        );
    }

    /// Move the focused row (Alt+↑/↓ or the row's own buttons) and write the
    /// whole container. Optimistic: the tree moves first, the order is sent
    /// after, and a refusal reloads what the server holds.
    fn shift_tree(&mut self, delta: isize, cx: &mut Context<Self>) {
        let playlists = self.playlists_ref().to_vec();
        let folders = self.folders().to_vec();
        let Some((target, order)) = self.tree.shift(delta, &playlists, &folders) else {
            push_toast(cx, &self.state, ToastKind::Info, "Déjà en bout de liste.");
            return;
        };
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.request(
            cx,
            move |_| {
                let api = api.clone();
                async move {
                    match &target {
                        Target::Folders => api.reorder_playlist_folders(&order).await,
                        Target::Container(folder) => {
                            api.reorder_playlist_container(folder.as_deref(), &order)
                                .await
                        }
                    }
                }
            },
            |this, result, cx| {
                match result {
                    Ok(()) => this.invalidate_playlists(cx),
                    Err(error) => {
                        push_toast(
                            cx,
                            &this.state,
                            ToastKind::Error,
                            format!("Déplacement non enregistré : {}", error.cause_fr()),
                        );
                        this.invalidate_playlists(cx);
                    }
                }
                cx.notify();
            },
        );
    }

    /// File a playlist into a folder (or back to the root): the destination
    /// container's whole order, which the server reads as "these are in me, in
    /// this order" (`playlist_folders.py`).
    fn move_playlist_to(&mut self, playlist: &str, folder: Option<String>, cx: &mut Context<Self>) {
        let playlists = self.playlists_ref().to_vec();
        let mut order = crate::widgets::folder_tree::container_order(&playlists, folder.as_deref());
        order.retain(|id| id != playlist);
        order.push(playlist.to_owned());
        if let Loaded::Ready(list) = &mut self.playlists {
            if let Some(row) = list
                .playlists
                .iter_mut()
                .find(|row| row.id.as_str() == playlist)
            {
                row.folder_id = folder.clone();
            }
            let playlists = list.playlists.clone();
            let folders = list.folders.clone();
            if let Some(tree) = Some(&mut self.tree) {
                tree.rebuild(&playlists, &folders);
            }
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        self.request(
            cx,
            move |_| {
                let api = api.clone();
                async move {
                    api.reorder_playlist_container(folder.as_deref(), &order)
                        .await
                }
            },
            |this, result, cx| {
                match result {
                    Ok(()) => this.invalidate_playlists(cx),
                    Err(error) => {
                        push_toast(
                            cx,
                            &this.state,
                            ToastKind::Error,
                            format!("Déplacement non enregistré : {}", error.cause_fr()),
                        );
                        this.invalidate_playlists(cx);
                    }
                }
                cx.notify();
            },
        );
    }

    fn artists_shown<'a>(&self, artists: &'a [Artist]) -> Vec<&'a Artist> {
        let needle = fold(self.query.trim());
        let mut list: Vec<&Artist> = artists
            .iter()
            .filter(|artist| needle.is_empty() || fold(&artist.name).contains(&needle))
            .collect();
        if self.sort == SortKey::Title {
            list.sort_by_key(|artist| fold(&artist.name));
        } else if self.sort == SortKey::Duration {
            // "recent": when it was followed, the only other order the SPA has.
            list.sort_by_key(|artist| std::cmp::Reverse(artist.starred.clone()));
        }
        if self.descending {
            list.reverse();
        }
        list
    }

    fn albums_shown<'a>(&self, albums: &'a [Album]) -> Vec<&'a Album> {
        let needle = fold(self.query.trim());
        let mut list: Vec<&Album> = albums
            .iter()
            .filter(|album| {
                needle.is_empty()
                    || fold(&album.title).contains(&needle)
                    || fold(&album.artist.name).contains(&needle)
            })
            .collect();
        if self.sort == SortKey::Title {
            list.sort_by_key(|album| fold(&album.title));
        } else if self.sort == SortKey::Artist {
            list.sort_by_key(|album| fold(&album.artist.name));
        }
        if self.descending {
            list.reverse();
        }
        list
    }

    // -- rows --------------------------------------------------------------

    /// The title a projected row index shows, whatever the tab's ordering.
    fn track_at(&self, index: usize) -> Option<&Track> {
        let (_, tracks) = self.titles()?;
        self.projection
            .get(index)
            .and_then(|index| tracks.get(*index))
    }

    /// The sleeves of the rows in `range`, asked for and decoded — nothing for a
    /// row outside it.
    ///
    /// One request per cover at most [`cover::MAX_IN_FLIGHT`] of them; the file
    /// is the 128 px copy (`Covers::rows`), decoded only once it is shown, and
    /// the atlas tile of a row that left the window is dropped by `show`.
    fn show_covers(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        self.visible = range.clone();
        let urls: Vec<String> = range
            .filter_map(|index| {
                self.track_at(index)
                    .and_then(|track| track.album.cover.clone())
            })
            .collect();
        self.show_urls(urls, cx);
    }

    /// Ask for, and decode, the sleeves of exactly these covers — the cards a
    /// shelf draws, or the rows a list painted this frame.
    fn show_urls(&mut self, urls: Vec<String>, cx: &mut Context<Self>) {
        {
            let snapshot = self.state.read(cx);
            let mut covers = self.covers.borrow_mut();
            for url in &urls {
                let for_request = url.clone();
                let for_message = url.clone();
                let tx = self.events.clone();
                covers.want(snapshot, &for_request, move |file| {
                    tx.send(Msg::Cover(for_message, file));
                });
            }
        }
        let files: Vec<PathBuf> = urls
            .iter()
            .filter_map(|url| cover::file_in(&self.covers, url))
            .collect();
        self.covers.borrow_mut().show(files, cx);
    }

    /// One title's row: a play button (the mouse-only path to playback), the
    /// sleeve, the title, the album, the duration.
    fn track_row(&self, index: usize, cx: &mut Context<Self>) -> gpui_kit::Stateful<Div> {
        let Some(track) = self.track_at(index) else {
            return div().id(ElementId::Name(format!("library-row-{index}").into()));
        };
        let sleeve = track
            .album
            .cover
            .as_deref()
            .and_then(|url| cover::file_in(&self.covers, url));
        let selected = self.selected == index;
        let playing = self
            .state
            .read(cx)
            .player
            .current()
            .map(|entry| entry.id.clone())
            == Some(entry_from_track(track).id);
        let mut row = div()
            .id(ElementId::Name(format!("library-row-{index}").into()))
            .debug_selector({
                let key = format!("library-row-{index}");
                move || key.clone()
            })
            .flex()
            .items_center()
            .gap(rems(0.75))
            .w_full()
            .h(px(ROW))
            .px(rems(0.5))
            .cursor_pointer()
            .when(selected, |row| row.bg(theme::selected(cx)))
            .hover(|style| style.bg(theme::selected(cx)))
            .on_click(
                cx.listener(move |this, event: &gpui_kit::ClickEvent, _, cx| {
                    if event.click_count() > 1 {
                        this.select(index, cx);
                        this.play(this.selected, false, cx);
                    } else {
                        this.select(index, cx);
                    }
                }),
            )
            .child(
                icon_button(
                    ElementId::Name(format!("library-play-{index}").into()),
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
                    this.select(index, cx);
                    this.play(index, false, cx);
                })),
            )
            .child(track_cells(track, sleeve.as_ref(), cx))
            .child(
                div()
                    .w(rems(12.))
                    .flex_shrink_0()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(track.album.title.clone()),
            )
            .child(
                div()
                    .w(rems(3.5))
                    .flex_shrink_0()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(format_duration(track.duration)),
            );
        if self.tab == LibraryTab::Favorites
            && let Some(chip) =
                download_control::chip(&self.rows.state(&download_control::key(track)), cx)
        {
            row = row.child(chip);
        }
        if self.tab == LibraryTab::Dislikes {
            row = row.child(
                button(
                    ElementId::Name(format!("library-restore-{index}").into()),
                    "Réintégrer",
                    ButtonVariant::Secondary,
                    cx,
                )
                .child(icon(IconName::RotateCcw).size(rems(0.75)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select(index, cx);
                    this.dislike_selection(cx);
                })),
            );
        }
        row
    }

    fn list_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let label = self.tab.title();
        if self.projected_len() == 0 {
            if let Some(cause) = self.loaded_failure() {
                return failed_state(&cause, label, cx).into_any_element();
            }
            if self.loaded_loading() {
                return loading_state(label, cx).into_any_element();
            }
            return empty_state(self.tab, cx).into_any_element();
        }
        let count = self.projected_len();
        // The columns' names, so the album and the duration read as columns and
        // not as a longer artist line (the SPA's `headers`).
        let header = div()
            .id("library-columns")
            .flex()
            .items_center()
            .gap(rems(0.75))
            .px(rems(0.5))
            .pb(rems(0.125))
            .w_full()
            .flex_shrink_0()
            .text_size(rems(0.5625))
            .text_color(theme::secondary(cx))
            .child(div().w(rems(2.25)).flex_shrink_0())
            .child(div().flex_1().child("TITRE"))
            .child(div().w(rems(12.)).flex_shrink_0().child("ALBUM"))
            .child(div().w(rems(3.5)).flex_shrink_0().child("DURÉE"));
        let list = gpui_kit::base::v_virtual_list(
            cx.entity(),
            ElementId::Name(format!("library-{label}").into()),
            row_sizes(count),
            |this, range, _window, cx| {
                // The sleeves of the rows this paint builds, and only those:
                // the rows in the window, nothing for the rest of the list.
                this.show_covers(range.clone(), cx);
                let mut rows = Vec::with_capacity(range.len());
                for index in range {
                    rows.push(this.track_row(index, cx));
                }
                rows
            },
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(header)
            .child(list)
            .into_any_element()
    }

    fn loaded_failure(&self) -> Option<SharedString> {
        match self.tab {
            LibraryTab::Favorites => self.favourites.failed().cloned(),
            LibraryTab::Uploads => self.uploads.failed().cloned(),
            LibraryTab::Dislikes => self.dislikes.failed().cloned(),
            LibraryTab::Unavailable => self.unavailable.failed().cloned(),
            LibraryTab::Playlists | LibraryTab::Folders => self.playlists.failed().cloned(),
            LibraryTab::Artists => self.artists.failed().cloned(),
            LibraryTab::Albums => self.albums.failed().cloned(),
        }
    }

    fn loaded_loading(&self) -> bool {
        match self.tab {
            LibraryTab::Favorites => self.favourites.is_loading(),
            LibraryTab::Uploads => self.uploads.is_loading(),
            LibraryTab::Dislikes => self.dislikes.is_loading(),
            LibraryTab::Unavailable => self.unavailable.is_loading(),
            LibraryTab::Playlists | LibraryTab::Folders => self.playlists.is_loading(),
            LibraryTab::Artists => self.artists.is_loading(),
            LibraryTab::Albums => self.albums.is_loading(),
        }
    }

    fn playlists_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(cause) = self.playlists.failed() {
            return failed_state(cause, "les playlists", cx).into_any_element();
        }
        let Some(list) = self.playlists.ready().cloned() else {
            return loading_state("les playlists", cx).into_any_element();
        };
        if list.playlists.is_empty() && list.folders.is_empty() {
            return empty_state(self.tab, cx).into_any_element();
        }

        // The organiser replaces the shelf: one container at a time, the rows in
        // their real order, each with its two move buttons (mouse) while
        // Alt+↑/↓ does the same from the keyboard.
        if let Some(target) = self.organizing.clone() {
            let mut rows: Vec<(String, String, bool)> = Vec::new();
            match &target {
                Target::Folders => {
                    for folder in &list.folders {
                        let count = list
                            .playlists
                            .iter()
                            .filter(|p| p.folder_id.as_deref() == Some(folder.id.as_str()))
                            .count();
                        rows.push((
                            folder.id.clone(),
                            format!("{} · {} playlists", folder.title, count),
                            true,
                        ));
                    }
                }
                Target::Container(folder) => {
                    let known: Vec<&str> = list.folders.iter().map(|f| f.id.as_str()).collect();
                    for playlist in &list.playlists {
                        let filed = playlist
                            .folder_id
                            .as_deref()
                            .filter(|id| known.contains(id));
                        let wanted = match folder {
                            Some(folder) => filed == Some(folder.as_str()),
                            None => filed.is_none(),
                        };
                        if wanted {
                            rows.push((
                                playlist.id.as_str().to_owned(),
                                playlist.title.clone(),
                                false,
                            ));
                        }
                    }
                }
            }
            let mut column = div().flex().flex_col().gap(rems(0.25)).flex_1().min_h_0();
            column = column.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.375))
                    .child(
                        div()
                            .text_size(rems(0.8125))
                            .font_weight(FontWeight::BOLD)
                            .child(match &target {
                                Target::Folders => "Organiser les dossiers".to_owned(),
                                Target::Container(Some(folder)) => format!(
                                    "Organiser · {}",
                                    list.folders
                                        .iter()
                                        .find(|row| &row.id == folder)
                                        .map(|row| row.title.clone())
                                        .unwrap_or_else(|| "Dossier".to_owned())
                                ),
                                Target::Container(None) => "Organiser · Sans dossier".to_owned(),
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.25))
                            .child(
                                icon(IconName::ArrowUpDown)
                                    .size(rems(0.75))
                                    .text_color(theme::secondary(cx)),
                            )
                            .child(note_line("Alt+↑↓ pour déplacer", cx)),
                    )
                    .child(
                        button("organiser-done", "Terminer", ButtonVariant::Primary, cx).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.organizing = None;
                                cx.notify();
                            }),
                        ),
                    ),
            );
            if rows.is_empty() {
                column = column.child(note_line("Ce conteneur est vide.", cx));
            }
            for (index, (_, title, is_folder)) in rows.iter().enumerate() {
                column = column.child(
                    div()
                        .id(ElementId::Name(format!("organiser-{index}").into()))
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .h(px(44.))
                        .px(rems(0.5))
                        .when(index == self.selected % rows.len().max(1), |row| {
                            row.bg(theme::selected(cx))
                        })
                        .child(
                            div()
                                .text_size(rems(0.625))
                                .text_color(theme::secondary(cx))
                                .child(format!("{}", index + 1)),
                        )
                        .child(
                            icon(if *is_folder {
                                IconName::LibraryBig
                            } else {
                                IconName::ListMusic
                            })
                            .size(rems(0.875))
                            .text_color(theme::secondary(cx)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(rems(0.75))
                                .child(title.clone()),
                        )
                        .child(
                            icon_button(
                                ElementId::Name(format!("organiser-up-{index}").into()),
                                IconName::ChevronUp,
                                "Monter",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .disabled(index == 0)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.tree.selected = index;
                                    this.shift_tree(-1, cx);
                                },
                            )),
                        )
                        .child(
                            icon_button(
                                ElementId::Name(format!("organiser-down-{index}").into()),
                                IconName::ChevronDown,
                                "Descendre",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .disabled(index + 1 == rows.len())
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.tree.selected = index;
                                    this.shift_tree(1, cx);
                                },
                            )),
                        ),
                );
            }
            return column.into_any_element();
        }

        // The shelf: the folders, each with its playlists, then the ones at the
        // root, the ones shared with this person, and the imported ones.
        let grouping = crate::widgets::folder_tree::group(&list.playlists, &list.folders);
        let mut column = div()
            .id("library-sections")
            .flex()
            .flex_col()
            .gap(rems(1.))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        // The sleeves of the cards the shelves draw, collected as the shelves
        // are built and decoded in ONE `show` (one screen's shown set per
        // paint, or each shelf would evict the previous one's).
        let mut shelf_covers: Vec<String> = Vec::new();
        let mut index = 0;
        let folders = list.folders.clone();
        for (folder_id, title, _) in &grouping.folders {
            let ids: Vec<String> = list
                .playlists
                .iter()
                .filter(|playlist| playlist.folder_id.as_deref() == Some(folder_id.as_str()))
                .map(|playlist| playlist.id.as_str().to_owned())
                .collect();
            column = column.child(self.section(
                index,
                title,
                &ids,
                &folders,
                &list,
                &mut shelf_covers,
                cx,
            ));
            index += 1;
        }
        if !grouping.unfiled.is_empty() {
            column = column.child(self.section(
                index,
                "Sans dossier",
                &grouping.unfiled,
                &folders,
                &list,
                &mut shelf_covers,
                cx,
            ));
            index += 1;
        }
        let shared: Vec<String> = list
            .playlists
            .iter()
            .filter(|playlist| {
                playlist.role.as_deref().is_some_and(|role| role != "owner")
                    && playlist
                        .folder_id
                        .as_deref()
                        .is_none_or(|id| !folders.iter().any(|folder| folder.id == id))
            })
            .map(|playlist| playlist.id.as_str().to_owned())
            .collect();
        if !shared.is_empty() {
            column = column.child(self.section(
                index,
                "Partagées avec moi",
                &shared,
                &folders,
                &list,
                &mut shelf_covers,
                cx,
            ));
            index += 1;
        }
        if !grouping.imported.is_empty() {
            column = column.child(self.section(
                index,
                "Depuis Deezer",
                &grouping.imported,
                &folders,
                &list,
                &mut shelf_covers,
                cx,
            ));
        }
        self.show_urls(shelf_covers, cx);
        column.into_any_element()
    }

    /// One shelf of playlists: its heading, its count, its "Organiser" button
    /// and the first screenful of cards.
    #[allow(clippy::too_many_arguments)]
    fn section(
        &self,
        index: usize,
        title: &str,
        ids: &[String],
        folders: &[PlaylistFolder],
        list: &PlaylistList,
        covers: &mut Vec<String>,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut grid = div().flex().flex_wrap().gap(rems(0.5));
        for id in ids.iter().take(GRID_STEP) {
            let Some(playlist) = list
                .playlists
                .iter()
                .find(|playlist| playlist.id.as_str() == id)
            else {
                continue;
            };
            let target = playlist.id.as_str().to_owned();
            let title_text = playlist.title.clone();
            let cover = playlist.cover.clone();
            if let Some(url) = cover.clone() {
                covers.push(url);
            }
            let file = cover
                .as_deref()
                .and_then(|url| cover::file_in(&self.covers, url));
            let tracks = playlist.nb_tracks;
            let row_index = list
                .playlists
                .iter()
                .position(|row| row.id == playlist.id)
                .unwrap_or(0);
            grid = grid.child(
                div()
                    .id(ElementId::Name(format!("playlist-card-{target}").into()))
                    .flex()
                    .flex_col()
                    .gap(rems(0.25))
                    .w(rems(11.))
                    .p(rems(0.5))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::selected(cx)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = row_index;
                        this.tree.selected = row_index;
                        this.goto(Route::Playlist { id: target.clone() }, cx);
                    }))
                    .child(cover::view(
                        file.as_ref(),
                        cover.as_deref(),
                        &title_text,
                        Kind::Playlist,
                        px(120.),
                        cx,
                    ))
                    .child(
                        div()
                            .text_size(rems(0.75))
                            .font_weight(FontWeight::MEDIUM)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(title_text),
                    )
                    .child(
                        div()
                            .text_size(rems(0.625))
                            .text_color(theme::secondary(cx))
                            .child(format!(
                                "{tracks} titre{}",
                                if tracks > 1 { "s" } else { "" }
                            )),
                    ),
            );
        }
        let folder_for_section = folders
            .iter()
            .find(|folder| folder.title == title)
            .map(|folder| folder.id.clone())
            .or_else(|| {
                folders
                    .iter()
                    .find(|folder| {
                        ids.iter().any(|id| {
                            list.playlists.iter().any(|playlist| {
                                playlist.id.as_str() == id
                                    && playlist.folder_id.as_deref() == Some(folder.id.as_str())
                            })
                        })
                    })
                    .map(|folder| folder.id.clone())
            });
        div()
            .flex()
            .flex_col()
            .gap(rems(0.375))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.375))
                    .child(
                        div()
                            .text_size(rems(0.625))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::secondary(cx))
                            .child(format!("{title} · {}", ids.len())),
                    )
                    .child(div().flex_1())
                    .child(
                        button(
                            ElementId::Name(format!("organise-section-{index}").into()),
                            "Organiser",
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.organizing = Some(Target::Container(folder_for_section.clone()));
                            this.selected = 0;
                            this.tree.selected = 0;
                            cx.notify();
                        })),
                    ),
            )
            .child(grid)
    }

    fn folders_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(cause) = self.playlists.failed() {
            return failed_state(cause, "les dossiers", cx).into_any_element();
        }
        if self.playlists.ready().is_none() {
            return loading_state("les dossiers", cx).into_any_element();
        }
        let mut column = div()
            .id("folder-tree")
            .flex()
            .flex_col()
            .gap(rems(0.125))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        column = column.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.375))
                .child(note_line(
                    "Entrée ouvre un dossier ; Alt+↑↓ déplace la ligne.",
                    cx,
                ))
                .child(div().flex_1())
                .child(
                    button(
                        "tree-new-folder",
                        "Nouveau dossier",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_prompt(Prompt::Folder, "", window, cx)
                    })),
                )
                .child(
                    button("tree-rename", "Renommer", ButtonVariant::Secondary, cx).on_click(
                        cx.listener(|this, _, window, cx| {
                            let current = this
                                .tree
                                .current()
                                .map(|node| node.title.clone())
                                .unwrap_or_default();
                            this.open_prompt(Prompt::RenameFolder, &current, window, cx)
                        }),
                    ),
                )
                .child(
                    button("tree-delete", "Supprimer", ButtonVariant::Danger, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.delete_folder(cx))),
                ),
        );
        if self.tree.is_empty() {
            column = column.child(empty_state(LibraryTab::Folders, cx));
        }
        let nodes: Vec<_> = self.tree.nodes.clone();
        for (index, node) in nodes.iter().enumerate() {
            let is_folder = matches!(node.kind, NodeKind::Folder { .. });
            column = column.child(
                div()
                    .id(ElementId::Name(format!("tree-{}", node.key).into()))
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .h(px(40.))
                    .pl(rems(0.5 + node.depth as f32 * 1.25))
                    .pr(rems(0.5))
                    .cursor_pointer()
                    .when(index == self.tree.selected, |row| {
                        row.bg(theme::selected(cx))
                    })
                    .child(
                        icon(if is_folder {
                            if self.tree.collapsed.contains(&node.key) {
                                IconName::Folder
                            } else {
                                IconName::FolderOpen
                            }
                        } else {
                            IconName::ListMusic
                        })
                        .size(rems(0.875))
                        .text_color(theme::secondary(cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(rems(0.75))
                            .child(node.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(rems(0.625))
                            .text_color(theme::secondary(cx))
                            .child(match node.kind {
                                NodeKind::Folder { count, .. } => {
                                    format!("{count} playlist{}", if count > 1 { "s" } else { "" })
                                }
                                NodeKind::Playlist { .. } => String::new(),
                            }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tree.selected = index;
                        this.selected = index;
                        if is_folder {
                            this.tree.toggle();
                            this.refold();
                        } else if let Some(NodeKind::Playlist { id, .. }) =
                            this.tree.current().map(|node| node.kind.clone())
                        {
                            this.goto(Route::Playlist { id }, cx);
                        }
                        cx.notify();
                    })),
            );
        }
        column.into_any_element()
    }

    fn grid_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let tab = self.tab;
        let (failure, loading) = match tab {
            LibraryTab::Artists => (self.artists.failed().cloned(), self.artists.is_loading()),
            _ => (self.albums.failed().cloned(), self.albums.is_loading()),
        };
        if let Some(cause) = failure {
            return failed_state(&cause, tab.title(), cx).into_any_element();
        }
        if loading {
            return loading_state(tab.title(), cx).into_any_element();
        }
        let mut grid = div()
            .id("library-grid")
            .flex()
            .flex_wrap()
            .gap(rems(0.5))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        let total;
        if tab == LibraryTab::Artists {
            // The artist model carries no picture: nothing to decode here, and
            // the previous tab's sleeves are let go.
            self.show_urls(Vec::new(), cx);
            let Some(artists) = self.artists.ready() else {
                return grid.into_any_element();
            };
            let shown = self.artists_shown(artists);
            total = shown.len();
            for (index, artist) in shown.iter().take(self.grid_limit).enumerate() {
                let id = artist
                    .id
                    .clone()
                    .unwrap_or_else(|| artist.deezer_id.as_str().to_owned());
                let name = artist.name.clone();
                let tracks = artist.tracks.unwrap_or(0);
                grid = grid.child(
                    div()
                        .id(ElementId::Name(format!("artist-card-{index}").into()))
                        .flex()
                        .flex_col()
                        .gap(rems(0.25))
                        .w(rems(10.))
                        .p(rems(0.5))
                        .cursor_pointer()
                        .when(index == self.selected, |card| card.bg(theme::selected(cx)))
                        .hover(|style| style.bg(theme::selected(cx)))
                        .child(sleeve(None, &name, 112., cx))
                        .child(
                            div()
                                .text_size(rems(0.75))
                                .font_weight(FontWeight::MEDIUM)
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(name),
                        )
                        .child(
                            div()
                                .text_size(rems(0.625))
                                .text_color(theme::secondary(cx))
                                .child(format!(
                                    "{tracks} titre{}",
                                    if tracks > 1 { "s" } else { "" }
                                )),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected = index;
                            this.goto(Route::Artist { id: id.clone() }, cx);
                        })),
                );
            }
        } else {
            // The sleeves of the page of albums this grid draws, before the
            // borrow of the model that building the cards needs.
            let urls = self
                .albums
                .ready()
                .map(|albums| {
                    self.albums_shown(albums)
                        .into_iter()
                        .take(self.grid_limit)
                        .filter_map(|album| album.cover.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            self.show_urls(urls, cx);
            let Some(albums) = self.albums.ready() else {
                return grid.into_any_element();
            };
            let shown = self.albums_shown(albums);
            total = shown.len();
            for (index, album) in shown.iter().take(self.grid_limit).enumerate() {
                let id = album
                    .id
                    .as_ref()
                    .map(|id| id.as_str().to_owned())
                    .unwrap_or_else(|| album.deezer_id.as_str().to_owned());
                let title = album.title.clone();
                let artist = album.artist.name.clone();
                let year = album.year.clone().unwrap_or_default();
                let file = album
                    .cover
                    .as_deref()
                    .and_then(|url| cover::file_in(&self.covers, url));
                grid = grid.child(
                    div()
                        .id(ElementId::Name(format!("album-card-{index}").into()))
                        .flex()
                        .flex_col()
                        .gap(rems(0.25))
                        .w(rems(10.))
                        .p(rems(0.5))
                        .cursor_pointer()
                        .when(index == self.selected, |card| card.bg(theme::selected(cx)))
                        .hover(|style| style.bg(theme::selected(cx)))
                        .child(cover::view(
                            file.as_ref(),
                            album.cover.as_deref(),
                            &title,
                            Kind::Album,
                            px(112.),
                            cx,
                        ))
                        .child(
                            div()
                                .text_size(rems(0.75))
                                .font_weight(FontWeight::MEDIUM)
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(rems(0.625))
                                .text_color(theme::secondary(cx))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(format!("{artist} · {year}")),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected = index;
                            this.goto(Route::Album { id: id.clone() }, cx);
                        })),
                );
            }
        }
        if total == 0 {
            return empty_state(tab, cx).into_any_element();
        }
        if total > self.grid_limit {
            grid = grid.child(
                button("grid-more", "Afficher plus", ButtonVariant::Secondary, cx).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.grid_limit += GRID_STEP;
                        cx.notify();
                    }),
                ),
            );
        }
        grid.into_any_element()
    }
}

/// A marker so the two row-move bindings are installed once per application
/// (they belong to the shell keymap's context, which D7 freezes after that lot;
/// a screen may ADD a binding at runtime, it may not edit the table).
struct RowKeysInstalled;

impl gpui_kit::Global for RowKeysInstalled {}

/// `Alt+↑`/`Alt+↓` move the row under the keyboard. Bound in the shell's own
/// context (`Neezer && !Input`) so a text field still gets its selection.
pub(crate) fn install_row_keys(cx: &mut gpui_kit::App) {
    if cx.try_global::<RowKeysInstalled>().is_some() {
        return;
    }
    cx.set_global(RowKeysInstalled);
    cx.bind_keys([
        gpui_kit::KeyBinding::new("alt-up", MoveRowUp, Some(crate::keymap::CONTEXT)),
        gpui_kit::KeyBinding::new("alt-down", MoveRowDown, Some(crate::keymap::CONTEXT)),
    ]);
}

/// Forget a cached path after a mutation (the screen's one invalidation).
fn forget(screen: &mut Screen, cx: &mut Context<Screen>, path: &str) {
    if let Some(api) = screen.state.read(cx).api() {
        api.forget(path);
    }
}

/// What a screen says while it has nothing yet.
fn loading_state(label: &str, cx: &App) -> gpui_kit::Stateful<Div> {
    div()
        .id("library-loading")
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(rems(0.5))
        .flex_1()
        .child(
            icon(IconName::LoaderCircle)
                .size(rems(1.5))
                .text_color(theme::secondary(cx)),
        )
        .child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child(format!("Chargement de {label}…")),
        )
}

/// What a screen says when it could not load: the cause, and nothing else.
fn failed_state(cause: &SharedString, label: &str, cx: &App) -> gpui_kit::Stateful<Div> {
    div()
        .id("library-failed")
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(rems(0.5))
        .flex_1()
        .p(rems(1.5))
        .child(
            icon(IconName::CircleAlert)
                .size(rems(1.5))
                .text_color(theme::danger(cx)),
        )
        .child(
            div()
                .text_size(rems(0.875))
                .font_weight(FontWeight::BOLD)
                .child(format!("{label} n'ont pas pu être chargés")),
        )
        .child(
            div()
                .max_w(rems(28.))
                .text_center()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child(cause.clone()),
        )
}

/// An empty tab is a place with a gesture, never a grey sentence.
fn empty_state(tab: LibraryTab, cx: &App) -> gpui_kit::Stateful<Div> {
    let (title, text, hint) = match tab {
        LibraryTab::Favorites => (
            "Rien ici pour l'instant",
            "Touchez le cœur d'un titre pour le garder ici.",
            "Explorer la recherche",
        ),
        LibraryTab::Playlists => (
            "Aucune playlist",
            "Une playlist pour un moment, une humeur, une route : créez la première.",
            "Nouvelle playlist",
        ),
        LibraryTab::Artists => (
            "Aucun artiste suivi",
            "Ouvrez la page d'un artiste et touchez le cœur pour le suivre.",
            "Explorer la recherche",
        ),
        LibraryTab::Albums => (
            "Aucun album gardé",
            "Touchez le cœur sur la page d'un album pour le garder ici.",
            "Explorer la recherche",
        ),
        LibraryTab::Uploads => (
            "Aucun fichier à vous",
            "Les fichiers importés sur ce serveur apparaissent ici.",
            "Explorer la recherche",
        ),
        LibraryTab::Dislikes => (
            "Rien d'écarté",
            "Les titres que vous écartez quittent le Flow et les radios : ils sont listés ici.",
            "Écouter mes favoris",
        ),
        LibraryTab::Unavailable => (
            "Aucun titre indisponible",
            "Quand un titre disparaît du catalogue, il apparaît ici pour qu'on lui trouve un \
             remplaçant. Ceux déjà écoutés restent archivés sur le serveur.",
            "Écouter mes favoris",
        ),
        LibraryTab::Folders => (
            "Aucun dossier",
            "Un dossier range vos playlists par moment : route, soirée, travail.",
            "Nouveau dossier",
        ),
    };
    div()
        .id("library-empty")
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(rems(0.5))
        .flex_1()
        .p(rems(2.))
        .child(
            div()
                .text_size(rems(1.125))
                .font_weight(FontWeight::BOLD)
                .child(title),
        )
        .child(
            div()
                .max_w(rems(30.))
                .text_center()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child(text),
        )
        .child(
            div()
                .text_size(rems(0.625))
                .text_color(theme::accent(cx))
                .child(hint),
        )
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_fields(window, cx);
        // The sleeves the last paint declared and this one does not are freed
        // here (their atlas tiles with them), before this frame's own `show`s.
        self.covers.borrow_mut().begin_frame(window, cx);
        // A tab reached from the sidebar changes the route, not the screen.
        self.sync_tab(cx);
        // The downloads control: one state for the list, one chip per row. The
        // rows are resolved ONCE per published version, never per row.
        let download_state = self.download_state();
        let version = self.downloads.get().version;
        if version != self.rows_version {
            let done = matches!(download_state, DownloadState::Done { .. });
            self.rows = Rc::new(Rows::build(self.downloads.get(), done));
            self.rows_version = version;
        }

        let count = self.row_count();
        let favourite_count = self.favourites.ready().map_or(0, Vec::len);
        let playlist_count = self
            .playlists
            .ready()
            .map_or(0, |list| list.playlists.len());

        let mut header = div()
            .id("library-header")
            .flex()
            .items_center()
            .gap(rems(0.5))
            .flex_shrink_0()
            .pb(rems(0.5))
            .child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(format!(
                        "{favourite_count} titre{} · {playlist_count} playlist{}",
                        if favourite_count > 1 { "s" } else { "" },
                        if playlist_count > 1 { "s" } else { "" }
                    )),
            )
            .child(div().flex_1());
        if let Some(filter) = self.filter.clone() {
            header = header.child(div().w(rems(16.)).child(input(
                "library-filter",
                &filter,
                window,
                cx,
            )));
        }
        header = header
            .child(
                button(
                    "library-sort",
                    format!("Tri : {}", self.sort.label()),
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.sort = this.sort.next();
                    this.recompute(cx);
                    cx.notify();
                })),
            )
            .child(
                button(
                    "library-desc",
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
                    this.recompute(cx);
                    cx.notify();
                })),
            )
            .child(
                button(
                    "library-new",
                    "Nouvelle playlist",
                    ButtonVariant::Outline,
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_prompt(Prompt::Playlist, "", window, cx)
                })),
            )
            .child(
                icon_button(
                    "library-import",
                    IconName::Link2,
                    "Importer une playlist partagée",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.goto(Route::Import { url: None }, cx))),
            );

        // The tabs, as one strip: the eight leaves of the plan's route table.
        let mut tabs = div()
            .id("library-tabs")
            .flex()
            .items_center()
            .gap(rems(0.25))
            .flex_shrink_0()
            .pb(rems(0.5));
        for tab in [
            LibraryTab::Favorites,
            LibraryTab::Playlists,
            LibraryTab::Folders,
            LibraryTab::Artists,
            LibraryTab::Albums,
            LibraryTab::Uploads,
            LibraryTab::Dislikes,
            LibraryTab::Unavailable,
        ] {
            let active = tab == self.tab;
            tabs = tabs.child(
                button(
                    ElementId::Name(format!("library-tab-{}", tab.title()).into()),
                    tab.title(),
                    if active {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    },
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.pick_tab(tab, cx))),
            );
        }

        // The list actions that are not the shell's: play from here, queue,
        // star, set aside, and the folder moves (Alt+↑/↓, bound here because
        // the keymap is frozen after D7 and has no row-move binding).
        let mut body = div().flex().flex_col().flex_1().min_h_0();
        body = match self.tab {
            LibraryTab::Playlists => body.child(self.playlists_body(cx)),
            LibraryTab::Folders => body.child(self.folders_body(cx)),
            LibraryTab::Artists | LibraryTab::Albums => body.child(self.grid_body(cx)),
            _ => body.child(self.list_body(cx)),
        };

        // The list's own tool bar: only what the tab can do.
        // A wrapping row: with the 420 px queue panel open the toolbar has
        // 660 px, and the download control pushed « Exporter » off the edge
        // (seen on the lot's own capture). Wrapping keeps every verb reachable.
        let mut tools = div()
            .id("library-tools")
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rems(0.375))
            .flex_shrink_0()
            .py(rems(0.375));
        if self.titles().is_some() {
            tools = tools
                .child(
                    button("library-play", "Tout lire", ButtonVariant::Primary, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.play(0, false, cx))),
                )
                .child(
                    button("library-shuffle", "Mélanger", ButtonVariant::Secondary, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.play(0, true, cx))),
                )
                .child(
                    button(
                        "library-add",
                        "Ajouter à une playlist",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.open_picker(window, cx))),
                )
                .child(
                    button("library-archive", "Archiver", ButtonVariant::Secondary, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.archive(cx))),
                )
                .children(
                    // The device download, on the list that has one: the
                    // favourites (« télécharger mes favoris hors ligne »). The
                    // SPA's Library only archives; this does both, and the
                    // button above says which is which.
                    (self.tab == LibraryTab::Favorites)
                        .then(|| {
                            download_control::control(
                                "library-download",
                                &download_state,
                                self.downloads.available(),
                                cx,
                            )
                        })
                        .flatten(),
                )
                .child(
                    button("library-export", "Exporter", ButtonVariant::Secondary, cx)
                        .child(icon(IconName::Archive).size(rems(0.75)))
                        .on_click(cx.listener(|this, _, window, cx| {
                            // An account-wide export (`favorites` → `me`).
                            this.active_export(window, cx);
                        })),
                );
        }
        if self.tab == LibraryTab::Playlists {
            tools = tools.child(
                button(
                    "library-organise",
                    "Organiser",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.organizing = Some(Target::Folders);
                    this.tree.selected = 0;
                    cx.notify();
                })),
            );
        }
        tools = tools.child(div().flex_1()).child(
            div()
                .text_size(rems(0.625))
                .text_color(theme::secondary(cx))
                .child(format!("{count} ligne{}", if count > 1 { "s" } else { "" })),
        );

        let mut root = div()
            .id("library-screen")
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .p(rems(1.));

        root = root.child(header).child(tabs).child(tools).child(body);

        // The prompt and the sheets sit above the page, inside the shell's
        // content column (the player bar stays visible, which is the point: a
        // sheet never hides the transport).
        if self.prompt_kind != Prompt::Closed
            && let Some(prompt) = self.prompt.clone()
        {
            let label = match self.prompt_kind {
                Prompt::Playlist => "Nouvelle playlist",
                Prompt::Folder => "Nouveau dossier",
                Prompt::RenameFolder => "Renommer le dossier",
                Prompt::Closed => "",
            };
            let prompt_card = card("library-prompt", "Nom", None, 22., cx)
                .child(field_label("Nom", cx))
                .child(input("library-prompt-input", &prompt, window, cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.375))
                        .child(
                            button("library-prompt-ok", "Valider", ButtonVariant::Primary, cx)
                                .on_click(cx.listener(|this, _, _, cx| this.commit_prompt(cx))),
                        )
                        .child(
                            button(
                                "library-prompt-cancel",
                                "Annuler",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.prompt_kind = Prompt::Closed;
                                cx.notify();
                            })),
                        ),
                );
            let _ = label;
            root = root.child(
                crate::widgets::sheets::backdrop("library-prompt-backdrop", cx)
                    .on_mouse_down(
                        gpui_kit::MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.prompt_kind = Prompt::Closed;
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation()
                            })
                            .child(prompt_card),
                    ),
            );
        }
        if let Some(picker) = self.picker.clone() {
            root = root.child(picker);
        }
        if let Some(export) = self.export.clone() {
            root = root.child(export);
        }
        root
    }
}

impl Screen {
    /// A tab clicked inside the screen. The ROUTE is the source of truth (it is
    /// what the header names and what the back button returns to), so a click
    /// replaces it and `sync_tab` loads what the new leaf shows. Setting only
    /// `self.tab` would be reverted by the very next paint.
    fn pick_tab(&mut self, tab: LibraryTab, cx: &mut Context<Self>) {
        self.route = Route::Library { tab };
        if tab == self.tab {
            // The same leaf: nothing to reload, but the route still agrees.
            self.relay(Route::Library { tab }, cx);
            return;
        }
        self.tab = tab;
        self.selected = 0;
        self.organizing = None;
        self.recompute(cx);
        self.state.update(cx, |state, cx| {
            state.nav.replace(Route::Library { tab });
            cx.notify();
        });
        self.load(cx);
    }
}

impl std::fmt::Debug for Screen {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LibraryScreen")
            .field("tab", &self.tab)
            .field("rows", &self.row_count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, artist: &str, album: &str, duration: u32) -> Track {
        let mut track = Track {
            deezer_id: nz_model::TrackId::new(title),
            title: title.to_owned(),
            duration,
            ..Track::default()
        };
        track.artist.name = artist.to_owned();
        track.album.title = album.to_owned();
        track
    }

    #[test]
    fn the_default_order_is_the_servers_and_a_sort_is_stable() {
        let tracks = vec![
            track("Beta", "Zoe", "Un", 200),
            track("alpha", "Yann", "Deux", 100),
            track("Alpha", "Xavier", "Trois", 300),
        ];
        assert_eq!(project(&tracks, SortKey::Default, false, ""), [0, 1, 2]);
        assert_eq!(
            project(&tracks, SortKey::Title, false, ""),
            [1, 2, 0],
            "« alpha » and « Alpha » fold to the same word, so their server order"
        );
        assert_eq!(project(&tracks, SortKey::Title, false, ""), {
            let mut rows = project(&tracks, SortKey::Title, false, "");
            rows.sort_by_key(|index| fold(&tracks[*index].title));
            rows
        });
        assert_eq!(project(&tracks, SortKey::Duration, true, ""), [2, 0, 1]);
        assert_eq!(project(&tracks, SortKey::Default, true, ""), [2, 1, 0]);
    }

    #[test]
    fn a_filter_folds_accents_and_searches_the_artist_and_the_album() {
        let tracks = vec![
            track("Été", "Kaya Project", "Songs from the Inside", 200),
            track("Nuit", "Aurora", "Été", 240),
            track("Matins", "Aurora", "Aube", 180),
        ];
        assert_eq!(project(&tracks, SortKey::Default, false, "ete"), [0, 1]);
        assert_eq!(project(&tracks, SortKey::Default, false, "aurora"), [1, 2]);
        assert_eq!(project(&tracks, SortKey::Default, false, "aube"), [2]);
        assert_eq!(project(&tracks, SortKey::Default, false, ""), [0, 1, 2]);
        assert!(project(&tracks, SortKey::Default, false, "zzz").is_empty());
    }

    #[test]
    fn a_duration_is_written_the_way_the_web_app_writes_it() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(9), "0:09");
        assert_eq!(format_duration(59), "0:59");
        assert_eq!(format_duration(60), "1:00");
        assert_eq!(format_duration(3_599), "59:59");
        assert_eq!(format_duration(3_600), "1:00:00");
        assert_eq!(format_duration(7_325), "2:02:05");
    }

    #[test]
    fn the_artist_line_prefers_the_servers_own_line_then_the_credited_ones() {
        let mut one = track("T", "A", "Al", 1);
        assert_eq!(artist_line(&one), "A");
        one.display_artist = Some("A feat. B".into());
        assert_eq!(artist_line(&one), "A feat. B");
        let mut two = track("T", "A", "Al", 1);
        two.artists = vec![
            nz_model::TrackArtistRole {
                name: "A".into(),
                ..Default::default()
            },
            nz_model::TrackArtistRole {
                name: "B".into(),
                ..Default::default()
            },
        ];
        assert_eq!(artist_line(&two), "A, B");
    }
}

#[cfg(test)]
mod journey {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    use gpui_kit::{Entity, IntoElement, ParentElement, Render, Styled, div};

    use super::*;
    use crate::state::{AppConfig, AppEvent, event_channel};

    /// A stand-in server: `/api/version`, `/api/me`, `/api/login` and the
    /// library's lists. A real socket, so the client, the cookie jar and the
    /// screen's own request path are exercised.
    fn stub_server(favourites: impl Into<String>) -> String {
        let favourites = favourites.into();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("local address");
        std::thread::spawn(move || {
            let favourites = favourites.clone();
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0u8; 8192];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let body = if request.starts_with("GET /api/version") {
                    r#"{"version":"9.9.9"}"#.to_string()
                } else if request.starts_with("GET /api/me/favorites") {
                    favourites.to_string()
                } else if request.starts_with("GET /api/me/playlists") {
                    r#"{"folders":[],"playlists":[]}"#.to_string()
                } else if request.starts_with("GET /api/me/") {
                    r#"{"tracks":[]}"#.to_string()
                } else {
                    r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#
                        .to_string()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Set-Cookie: session=abc; Path=/\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{address}")
    }

    fn temporary(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neezer-d8b-library-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn favourites_json(count: usize) -> String {
        favourites_with(count, false)
    }

    /// The same list, every row's album carrying a sleeve the bench serves.
    fn favourites_with_covers(count: usize) -> String {
        favourites_with(count, true)
    }

    fn favourites_with(count: usize, covers: bool) -> String {
        let tracks: Vec<String> = (0..count)
            .map(|index| {
                let cover = if covers {
                    format!(r#""/api/cover/c{index}""#)
                } else {
                    "null".to_owned()
                };
                format!(
                    r#"{{"deezer_id":"{}","title":"Titre {index}","duration":200,
                        "artist":{{"deezer_id":"{}","name":"Artiste {index}"}},
                        "album":{{"deezer_id":"{}","title":"Album {index}","cover":{cover}}}}}"#,
                    3_000_000 + index,
                    1_000_000 + index,
                    2_000_000 + index
                )
            })
            .collect();
        format!(r#"{{"tracks":[{}]}}"#, tracks.join(","))
    }

    /// A cache with `/me/favorites` already on the disk, as the previous open of
    /// the tab would have left it.
    fn seed_cache(dir: &std::path::Path, count: usize) {
        let cache = nz_cache::Cache::open_with_cache_dir(dir.join("data"), dir.join("cache"))
            .expect("cache");
        cache.put("u1", "/me/favorites", &favourites_json(count));
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
    ) -> (Entity<AppState>, crate::wiring::events::Receiver<AppEvent>) {
        let (tx, rx) = event_channel();
        let config = AppConfig {
            server: Some(server),
            start_audio: false,
            start_mpris: false,
            ..AppConfig::for_test(dir)
        };
        let handle = runtime.handle().clone();
        let state = cx.update(|cx| cx.new(move |_| AppState::new(config, handle, tx, None)));
        (state, rx)
    }

    /// Draw the screen once in a real (headless) window.
    struct Harness(Entity<Screen>);

    impl Render for Harness {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.0.clone())
        }
    }

    fn rss_kb() -> u64 {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    }

    /// The acceptance's first half: four thousand favourites and a four
    /// thousand-row playlist open from the CACHE — parsed, projected and drawn
    /// — under 100 ms in a release build (the desktop's own numbers are release
    /// numbers; a debug build has no optimisation and this machine runs several
    /// at once, so it gets its own, looser bound printed beside it).
    #[gpui::test]
    async fn four_thousand_favourites_open_from_the_cache_under_the_budget(
        cx: &mut gpui::TestAppContext,
    ) {
        // 4 000 titles and their projection: in a CHILD process, because a
        // sibling test (D14a's settings page) asserts on this process's RSS
        // delta and two threads measuring one process cannot both be right.
        if let Some(printed) = crate::widgets::card::stub::in_child(
            "screens::library::journey::four_thousand_favourites_open_from_the_cache_under_the_budget",
        ) {
            assert!(
                printed.contains("D8b favourites open") || printed.contains("test result: ok"),
                "the child ran:\n{printed}"
            );
            return;
        }
        let dir = temporary("cache-open");
        seed_cache(&dir, 4_000);
        cx.update(|cx| {
            gpui_omarchy::init(cx);
            crate::keymap::install(cx);
        });
        let runtime = runtime();
        let (state, _events) = app_state(cx, &dir, stub_server(r#"{"tracks":[]}"#), &runtime);
        cx.update(|cx| {
            let api = state.read(cx).api.clone().expect("api");
            api.set_owner(Some("u1".to_owned()));
            assert!(state.read(cx).cache.is_some());
        });

        let started = Instant::now();
        let screen = cx.update(|cx| {
            cx.new(|cx| {
                Screen::new(
                    Route::Library {
                        tab: LibraryTab::Favorites,
                    },
                    state.clone(),
                    cx,
                )
            })
        });
        let parse_and_project = started.elapsed();
        cx.update(|cx| assert_eq!(screen.read(cx).projected_len(), 4_000));

        let started = Instant::now();
        let harness = screen.clone();
        let (_harness, vcx) = cx.add_window_view(move |_, _| Harness(harness));
        vcx.run_until_parked();
        vcx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let first_paint = started.elapsed();
        let open = parse_and_project + first_paint;
        let budget = if cfg!(debug_assertions) {
            Duration::from_millis(1_500)
        } else {
            Duration::from_millis(100)
        };
        println!(
            "D8b favourites open: parse+project {:?}, first paint {:?}, total {:?} (budget {:?})",
            parse_and_project, first_paint, open, budget
        );
        assert!(open < budget, "opened in {open:?}, budget {budget:?}");
        std::fs::remove_dir_all(dir).ok();
    }

    /// The memory the two screens cost at the worst realistic size: an
    /// 8 000-row favourites list and a 4 000-row playlist both parsed and
    /// projected (the widgets are windowed, so only the model follows).
    #[gpui::test]
    async fn the_worst_list_costs_a_bounded_amount_of_memory(cx: &mut gpui::TestAppContext) {
        // The heaviest test of this crate (12 000 rows held at once): a child
        // process, so the RSS it spends is only its own.
        if let Some(printed) = crate::widgets::card::stub::in_child(
            "screens::library::journey::the_worst_list_costs_a_bounded_amount_of_memory",
        ) {
            assert!(
                printed.contains("D8b RSS") || printed.contains("test result: ok"),
                "the child ran:\n{printed}"
            );
            return;
        }
        let dir = temporary("rss");
        seed_cache(&dir, 8_000);
        let playlist: Vec<String> = (0..4_000)
            .map(|index| {
                format!(
                    r#"{{"deezer_id":"{}","title":"Titre {index}","duration":200,
                        "artist":{{"deezer_id":"1","name":"Kaya"}},
                        "album":{{"deezer_id":"2","title":"Album","cover":null}}}}"#,
                    4_000_000 + index
                )
            })
            .collect();
        let cache_body = format!(
            r#"{{"playlist":{{"id":"pl-1","title":"Longue","nb_tracks":4000,"version":1,
                "editable":true,"can_manage":true,"members":[],"members_version":1}},
                "tracks":[{}]}}"#,
            playlist.join(",")
        );
        let cache = nz_cache::Cache::open_with_cache_dir(dir.join("data"), dir.join("cache"))
            .expect("cache");
        cache.put("u1", "/playlist/pl-1", &cache_body);
        drop(cache);
        cx.update(gpui_omarchy::init);
        let runtime = runtime();
        let (state, _events) = app_state(cx, &dir, stub_server(r#"{"tracks":[]}"#), &runtime);
        cx.update(|cx| {
            let api = state.read(cx).api.clone().expect("api");
            api.set_owner(Some("u1".to_owned()));
        });
        let before = rss_kb();
        let library = cx.update(|cx| {
            cx.new(|cx| {
                Screen::new(
                    Route::Library {
                        tab: LibraryTab::Favorites,
                    },
                    state.clone(),
                    cx,
                )
            })
        });
        let playlist = cx.update(|cx| {
            cx.new(|cx| {
                crate::screens::playlist::Screen::new(
                    Route::Playlist {
                        id: "pl-1".to_owned(),
                    },
                    state.clone(),
                    cx,
                )
            })
        });
        let after = rss_kb();
        let library_rows = cx.update(|cx| library.read(cx).projected_len());
        let playlist_rows = cx.update(|cx| playlist.read(cx).row_len());
        println!(
            "D8b RSS: {before} KB before, {after} KB after (+{} KB) for {library_rows} favourites \
             and {playlist_rows} playlist rows",
            after.saturating_sub(before)
        );
        assert_eq!(library_rows, 8_000);
        assert_eq!(playlist_rows, 4_000);
        assert!(
            after.saturating_sub(before) < 200_000,
            "the two models grew by {} KB",
            after.saturating_sub(before)
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// Wait, bounded, for the app to reach a state. The test thread drives the
    /// current-thread runtime, so every wake happens where GPUI's deterministic
    /// scheduler expects it (D7's pitfall).
    fn wait_until(
        runtime: &tokio::runtime::Runtime,
        cx: &mut gpui::VisualTestContext,
        state: &Entity<AppState>,
        what: &str,
        mut done: impl FnMut(&AppState) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            runtime.block_on(async {
                tokio::time::sleep(Duration::from_millis(1)).await;
            });
            cx.run_until_parked();
            if cx.update(|_, cx| done(state.read(cx))) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
        }
    }

    /// The list's verbs reach the screen WITHOUT it taking the keyboard focus:
    /// the action is dispatched at the app and the screen's weak handle answers.
    /// (A screen that tracked the focus would break every shell binding once it
    /// was unmounted, which is what this lot measured on D7's journey test.)
    #[gpui::test]
    async fn the_list_actions_reach_the_screen_without_stealing_the_focus(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = temporary("verbs");
        seed_cache(&dir, 3);
        cx.update(|cx| {
            gpui_omarchy::init(cx);
            crate::keymap::install(cx);
        });
        let runtime = runtime();
        let (state, _events) = app_state(cx, &dir, stub_server(r#"{"tracks":[]}"#), &runtime);
        let screen = cx.update(|cx| {
            cx.new(|cx| {
                Screen::new(
                    Route::Library {
                        tab: LibraryTab::Favorites,
                    },
                    state.clone(),
                    cx,
                )
            })
        });
        cx.update(|cx| {
            let api = state.read(cx).api.clone().expect("api");
            api.set_owner(Some("u1".to_owned()));
        });
        // The screen read the cache before the owner was set in this test, so
        // load it again through the same path the app uses.
        cx.update(|cx| screen.update(cx, |this, cx| this.load(cx)));
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(screen.read(cx).projected_len(), 3));
        cx.update(|cx| cx.dispatch_action(&ListNext));
        cx.update(|cx| assert_eq!(screen.read(cx).selected, 1, "j moved the cursor"));
        cx.update(|cx| cx.dispatch_action(&ListLast));
        cx.update(|cx| assert_eq!(screen.read(cx).selected, 2));
        cx.update(|cx| cx.dispatch_action(&ListFirst));
        cx.update(|cx| assert_eq!(screen.read(cx).selected, 0));
        std::fs::remove_dir_all(dir).ok();
    }

    /// The shell's own layers still work over a real screen: `q` opens the
    /// queue panel and `Échap` closes it. (A screen that took the focus broke
    /// every one of these; this is the regression test for that.)
    #[gpui::test]
    async fn the_queue_panel_still_opens_over_the_library(cx: &mut gpui::TestAppContext) {
        let dir = temporary("queue");
        seed_cache(&dir, 2);
        let server = stub_server(favourites_json(2));
        cx.update(|cx| {
            gpui_omarchy::init(cx);
            crate::keymap::install(cx);
        });
        let runtime = runtime();
        let (state, events) = app_state(cx, &dir, server.clone(), &runtime);
        let login_server = server.clone();
        cx.update(|cx| {
            state.update(cx, |state, _| {
                state.login(login_server, "bench".into(), "secret".into());
            });
        });
        let (_view, vcx) = cx.add_window_view({
            let state = state.clone();
            move |window, cx| crate::app::RootView::new(state, events, window, cx)
        });
        wait_until(&runtime, vcx, &state, "the session", |state| {
            state.auth.is_logged_in()
        });
        vcx.simulate_keystrokes("g l");
        vcx.run_until_parked();
        vcx.simulate_keystrokes("q");
        vcx.run_until_parked();
        vcx.update(|_, cx| {
            assert!(
                state.read(cx).side_panel.is_some(),
                "q opens the queue panel over a real screen"
            );
        });
        vcx.simulate_keystrokes("q");
        vcx.run_until_parked();
        vcx.update(|_, cx| assert!(state.read(cx).side_panel.is_none()));
        std::fs::remove_dir_all(dir).ok();
    }

    /// The keyboard alone: `g l` opens the library on the cached favourites,
    /// `j` walks down, `Entrée` plays the row under the cursor.
    #[gpui::test]
    async fn the_keyboard_opens_the_library_and_plays_the_row_under_the_cursor(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = temporary("keyboard");
        seed_cache(&dir, 2);
        let server = stub_server(favourites_json(2));
        cx.update(|cx| {
            gpui_omarchy::init(cx);
            crate::keymap::install(cx);
        });
        let runtime = runtime();
        let (state, events) = app_state(cx, &dir, server.clone(), &runtime);
        let login_server = server.clone();
        cx.update(|cx| {
            state.update(cx, |state, _| {
                state.login(login_server, "bench".into(), "secret".into());
            });
        });
        let (_view, vcx) = cx.add_window_view({
            let state = state.clone();
            move |window, cx| crate::app::RootView::new(state, events, window, cx)
        });
        wait_until(&runtime, vcx, &state, "the session", |state| {
            state.auth.is_logged_in()
        });

        vcx.simulate_keystrokes("g l");
        vcx.run_until_parked();
        vcx.update(|_, cx| {
            assert!(matches!(
                state.read(cx).nav.current(),
                Some(Route::Library { .. })
            ));
        });

        // The favourites tab, as the strip's own button opens it: its body comes
        // from the cache, so the list is there in the same frame.
        vcx.update(|_, cx| {
            state.update(cx, |state, cx| {
                state.nav.reset(Route::Library {
                    tab: LibraryTab::Favorites,
                });
                cx.notify();
            });
        });
        vcx.run_until_parked();
        vcx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        vcx.run_until_parked();

        vcx.simulate_keystrokes("j");
        vcx.update(|_, cx| {
            assert!(
                state.read(cx).player.current().is_none(),
                "moving down a list must not start playback"
            );
        });
        vcx.simulate_keystrokes("enter");
        wait_until(&runtime, vcx, &state, "the queue", |state| {
            state.player.current().is_some()
        });
        vcx.update(|_, cx| {
            let state = state.read(cx);
            let current = state.player.current().expect("a row is playing");
            assert_eq!(
                current.title, "Titre 1",
                "the row under the cursor is the second favourite, not the first"
            );
            let pending = state.pending_load.as_ref().expect("the engine boundary");
            assert!(pending.autoplay, "Entrée starts the sound");
        });
        vcx.simulate_keystrokes("l");
        wait_until(&runtime, vcx, &state, "the star", |state| {
            state
                .toasts
                .items()
                .iter()
                .any(|toast| toast.message.contains("favoris"))
        });
        std::fs::remove_dir_all(dir).ok();
    }

    /// The login answer the stub gives (`/api/login` and `/api/me`).
    const ME: &str =
        r#"{"user":{"id":"u1","name":"bench","admin":false,"access":{},"sources":[]}}"#;

    /// A real 128 px JPEG, encoded by the same crate the app decodes with: the
    /// file a row paints, 65 536 B decoded.
    fn sleeve_jpeg() -> Vec<u8> {
        let picture = image::RgbImage::from_fn(128, 128, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
        });
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 82);
        encoder.encode_image(&picture).expect("encode");
        bytes
    }

    fn cover_hits(stub: &crate::widgets::card::stub::Stub) -> usize {
        stub.hits()
            .into_iter()
            .filter(|hit| hit.path.starts_with("/api/cover/"))
            .count()
    }

    /// The rule, on the screen a person opens: **a row paints the sleeve it is
    /// SEEN at, and only a visible row asks for one.**
    ///
    /// 400 favourites, one sleeve per album, the test window: the paint asks
    /// for the visible rows' covers (never the ones below the fold), each one
    /// request, the 128 px copy is decoded once (65 536 B of pixels), and
    /// scrolling away frees the pixels and the atlas tile of the rows that left.
    #[gpui::test]
    async fn the_favourites_list_paints_only_the_sleeves_it_shows(cx: &mut gpui::TestAppContext) {
        let jpeg = sleeve_jpeg();
        let mut routes: Vec<(&'static str, Vec<u8>)> = vec![
            ("/api/login", ME.as_bytes().to_vec()),
            ("/api/me", ME.as_bytes().to_vec()),
            ("/api/version", br#"{"version":"9.9.9"}"#.to_vec()),
            (
                "/api/me/favorites",
                favourites_with_covers(400).into_bytes(),
            ),
            (
                "/api/me/playlists",
                br#"{"playlists":[],"folders":[]}"#.to_vec(),
            ),
        ];
        for index in 0..400 {
            // The stub borrows its paths, and this test binary outlives it.
            let path: &'static str = Box::leak(format!("/api/cover/c{index}").into_boxed_str());
            routes.push((path, jpeg.clone()));
        }
        let stub = crate::widgets::card::stub::Stub::start_bytes(&routes);
        let (state, runtime) = crate::widgets::card::stub::logged_in(cx, &stub);
        // The idle rule a test can see: `IDLE_FOR` is 2 s, which is right for a
        // person and too slow for a test, and 0 would evict on the transient
        // one-row layout frame a virtual list draws while it measures. 40 ms is
        // the same rule, timed by the test.
        cx.update(|cx| {
            cx.set_global(cover::Sleeves::with_rules(
                cover::DECODED_BUDGET,
                std::time::Duration::from_millis(40),
            ))
        });

        let (screen, vcx) = cx.add_window_view({
            let state = state.clone();
            move |_window, cx| {
                Screen::new(
                    Route::Library {
                        tab: LibraryTab::Favorites,
                    },
                    state,
                    cx,
                )
            }
        });
        crate::widgets::card::stub::until(&runtime, vcx, "the favourites", |cx| {
            screen.read(cx).projected_len() == 400
        });
        vcx.run_until_parked();

        let visible = vcx.update(|_, cx| screen.read(cx).visible_rows());
        let shown = visible.len();
        assert!(
            (8..=40).contains(&shown),
            "the window builds a screenful, not the list: {visible:?}"
        );
        assert!(
            cover_hits(&stub) <= shown.max(cover::MAX_IN_FLIGHT),
            "the first paint asks for a screenful at most: {} requests for {shown} rows",
            cover_hits(&stub)
        );
        // Let every visible row's sleeve land: each answer lets the next ask,
        // and each frame's own declaration starts the next decode.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while cover_hits(&stub) < shown
            || vcx.update(|_, cx| cx.global::<cover::Sleeves>().resident().1)
                < shown.saturating_sub(1)
        {
            runtime
                .block_on(async { tokio::time::sleep(std::time::Duration::from_millis(2)).await });
            vcx.update(|window, cx| {
                cx.notify(screen.entity_id());
                window.refresh();
                window.draw(cx).clear(cx);
            });
            vcx.run_until_parked();
            assert!(
                std::time::Instant::now() < deadline,
                "the visible rows' sleeves never all landed: {} requests, {:?} resident",
                cover_hits(&stub),
                vcx.update(|_, cx| cx.global::<cover::Sleeves>().resident())
            );
        }
        // The picture, not a placeholder: one decoded image per visible row,
        // each a 128 px copy (65 536 B), and no request for a row below the fold.
        let (bytes, images) = vcx.update(|_, cx| cx.global::<cover::Sleeves>().resident());
        assert!(
            images >= shown.saturating_sub(1),
            "{images} decoded for {shown} visible rows"
        );
        assert_eq!(bytes, images * 65_536, "a row's image is 128 × 128 × 4 B");
        assert_eq!(
            stub.count("/api/cover/c0"),
            1,
            "the first row's sleeve, asked for once"
        );
        let below = format!("/api/cover/c{}", visible.end + 100);
        assert_eq!(
            stub.count(&below),
            0,
            "a row below the fold must not cost a request"
        );
        // The image is IN the window's atlas, painted through
        // `ImageSource::Render` (the tile `Covers::show` can drop again).
        let file = vcx
            .update(|_, cx| {
                let screen = screen.read(cx);
                screen
                    .track_at(visible.start)
                    .and_then(|track| track.album.cover.as_deref())
                    .and_then(|url| cover::file_in(&screen.covers, url))
            })
            .expect("the first row's file");
        let image = vcx.update(|_, cx| match cx.global::<cover::Sleeves>().lookup(&file) {
            cover::Look::Ready(image) => image,
            _ => panic!("the first row's sleeve is not decoded"),
        });
        vcx.update(|window, _| {
            assert!(
                window.has_image_atlas_entry(&image),
                "the visible row's sleeve is uploaded once"
            )
        });

        // Scroll far down: the new rows are asked for by name, and the frame
        // after that frees the ones that left — pixels and atlas tile. Row 300:
        // far enough that no row of the first screenful is still on screen
        // (`ScrollStrategy::Top` leaves the row toward the bottom of the
        // viewport in this snapshot, three rows of overscan below it).
        let target = 300;
        screen.update(vcx, |this, cx| {
            this.scroll.scroll_to_item(target, ScrollStrategy::Top);
            cx.notify();
        });
        vcx.run_until_parked();
        // A virtual list applies a deferred scroll on its next layout: the test
        // draws the frames (a `refresh` so the list REALLY lays out again, not
        // a repaint of the same layout) and waits for the list to BE there.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !vcx.update(|_, cx| screen.read(cx).visible_rows().contains(&target)) {
            runtime
                .block_on(async { tokio::time::sleep(std::time::Duration::from_millis(2)).await });
            vcx.update(|window, cx| {
                cx.notify(screen.entity_id());
                window.refresh();
                window.draw(cx).clear(cx);
            });
            vcx.run_until_parked();
            assert!(
                std::time::Instant::now() < deadline,
                "the list never followed the scroll: {:?}",
                vcx.update(|_, cx| screen.read(cx).visible_rows())
            );
        }
        crate::widgets::card::stub::until(&runtime, vcx, "the new rows' sleeves", |_| {
            cover_hits(&stub) > shown
        });
        // `IDLE_FOR` has to pass before the rows that left are freed, and the
        // sweep runs on a frame: this is the person scrolling on and stopping.
        // The wait is on the FIRST row's own pixels, never on a count — a
        // count would pass while the images that must go are still there.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while vcx.update(|_, cx| cx.global::<cover::Sleeves>().holds(&file)) {
            runtime
                .block_on(async { tokio::time::sleep(std::time::Duration::from_millis(6)).await });
            vcx.update(|window, cx| {
                cx.notify(screen.entity_id());
                window.refresh();
                window.draw(cx).clear(cx);
            });
            vcx.run_until_parked();
            assert!(
                std::time::Instant::now() < deadline,
                "the sleeve of a row that left the window was never freed"
            );
        }
        let (bytes_after, images_after) =
            vcx.update(|_, cx| cx.global::<cover::Sleeves>().resident());
        let visible_after = vcx.update(|_, cx| screen.read(cx).visible_rows());
        assert!(
            images_after <= visible_after.len() + 1,
            "the rows that left were freed: {images_after} images for {:?}",
            visible_after
        );
        assert_eq!(bytes_after, images_after * 65_536);
        assert_eq!(
            stub.count("/api/cover/c0"),
            1,
            "a sleeve already on the device is never re-requested"
        );
        vcx.update(|window, _| {
            assert!(
                !window.has_image_atlas_entry(&image),
                "the tile of a row that left the window is dropped"
            )
        });

        runtime.shutdown_background();
        stub.stop();
    }
}
