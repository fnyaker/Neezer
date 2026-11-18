//! The track-actions FFI surface — lot MB10.
//!
//! Owned by **MB10** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this file — its own `#[uniffi::export] impl` blocks, its own
//! records — and edits no other file of the crate. UniFFI merges the
//! `impl Screens` block below into the ONE Kotlin `Screens` class, so nothing
//! shared is touched.
//!
//! # The shape of this boundary
//!
//! - **The menu is DATA**, not five callbacks: [`MenuEntry`] carries the label,
//!   the SPA's icon name, the verb ([`MenuAction`]) and its submenu, and the
//!   Kotlin sheet draws it without a second opinion about which entry belongs on
//!   which surface. `track_menu` is synchronous and free (0 requests).
//! - **A write is one call, optimistic, with its rollback inside**: `like`,
//!   `unlike`, `dislike`, `undislike`. The Kotlin side shows the state first
//!   anyway (`webapp.md`'s rule); the core also patches every open list, so a
//!   second screen already showing the title flips in the same frame.
//! - **A sheet asks its pair**: `cached_menu_playlists()` (disk only) then
//!   `menu_playlists()` (one request); the replacement sheet has one read
//!   (`replace_candidates`) and one commit (`replace`), whose rewrite is a
//!   SERVER job the screen polls (`replace_status`) — rule 2.
//! - **A surface that must re-window is TOLD**: `subscribe_track_actions`
//!   carries "these keys changed" with a counter, so a list that holds a
//!   `ListHandle` re-windows the page the person is looking at.
//!
//! Every method delegates to `nz_core::actions`; no rule lives here.

use nz_core::actions as core;

use crate::core::Screens;
use crate::errors::NzError;
use crate::lists::TrackRow;

/// One credited artist the surface knows, so "Aller à l'artiste" can name a page.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct MenuPerson {
    /// What the artist route accepts (the row's UUID when the library holds one,
    /// else the provider's id).
    pub id: String,
    pub name: String,
}

/// The last playlist a title was added to: the menu's one-tap shortcut.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct LastPlaylist {
    pub id: String,
    pub title: String,
    /// Only a list the person may write to is offered.
    pub editable: bool,
}

/// Everything the menu reads besides the row itself. The defaults are the SPA's
/// normal case, and the four gates (`offline`, `genre_tag`, `markers`) are the
/// entries another lot owns: an entry is present only when the device can honour
/// it (a button that does nothing is a bug, rule 5).
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct MenuContext {
    /// `"queue"` for the queue editor; anything else is an ordinary list.
    pub surface: String,
    /// The row's position in the queue, when the surface knows it.
    pub index: Option<u32>,
    /// The playing row's position: only what is AFTER it can be removed.
    pub current_index: Option<i32>,
    /// A read-only remote control offers no entry that writes.
    pub can_keep: bool,
    /// Whether a link out of the menu leads anywhere.
    pub can_browse: bool,
    /// The account administers the server: "Remplacer le titre…" is theirs.
    pub admin: bool,
    /// This device controls another player: no download entry, no marker.
    pub remote: bool,
    /// The title is already on this device.
    pub downloaded: bool,
    pub last_playlist_id: Option<String>,
    pub last_playlist_title: Option<String>,
    pub last_playlist_editable: bool,
    /// The credited artists the surface knows, main first.
    pub artists: Vec<MenuPerson>,
    /// A podcast episode: the SPA's own episode menu.
    pub episode: bool,
    /// This very episode is the one playing.
    pub episode_playing: bool,
    /// The show an episode belongs to.
    pub channel_id: Option<String>,
    /// The credits sheet is installed on this device (MB7, wired by the shell).
    pub credits: bool,
    /// The offline lot is on this device (MB15).
    pub offline: bool,
    /// The genre studio is on this device (MB18).
    pub genre_tag: bool,
    /// The podcast markers are on this device (MB13).
    pub markers: bool,
}

impl From<MenuContext> for core::MenuContext {
    fn from(ctx: MenuContext) -> Self {
        core::MenuContext {
            surface: core::Surface::parse(&ctx.surface),
            index: ctx.index,
            current_index: ctx.current_index,
            can_keep: ctx.can_keep,
            can_browse: ctx.can_browse,
            admin: ctx.admin,
            remote: ctx.remote,
            downloaded: ctx.downloaded,
            last_playlist: ctx.last_playlist_id.filter(|id| !id.is_empty()).map(|id| {
                core::LastPlaylist {
                    id,
                    title: ctx.last_playlist_title.unwrap_or_default(),
                    editable: ctx.last_playlist_editable,
                }
            }),
            artists: ctx
                .artists
                .into_iter()
                .map(|person| core::MenuPerson {
                    id: person.id,
                    name: person.name,
                })
                .collect(),
            episode: ctx.episode,
            episode_playing: ctx.episode_playing,
            channel_id: ctx.channel_id,
            credits: ctx.credits,
            offline: ctx.offline,
            genre_tag: ctx.genre_tag,
            markers: ctx.markers,
        }
    }
}

impl From<core::Actor> for MenuActor {
    fn from(actor: core::Actor) -> Self {
        MenuActor {
            admin: actor.admin,
            last_playlist: actor.last_playlist.map(LastPlaylist::from),
        }
    }
}

impl From<core::LastPlaylist> for LastPlaylist {
    fn from(last: core::LastPlaylist) -> Self {
        LastPlaylist {
            id: last.id,
            title: last.title,
            editable: last.editable,
        }
    }
}

/// What an item does when it is chosen — the SPA's `action` closure, as a verb.
/// The Kotlin side runs it: a player command for the transport verbs, one of this
/// file's calls for a write, the shell's routes for a link.
#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    PlayNext,
    AddToQueue,
    RemoveFromQueue { key: String },
    AddToLastPlaylist { playlist_id: String, title: String },
    OpenPlaylistPicker,
    OpenReplace,
    Download { quality: Option<String> },
    Undownload,
    Share,
    StartRadio,
    Favorite { on: bool },
    Dislike { on: bool },
    OpenCredits,
    GoToArtist { id: String },
    GoToAlbum { id: String },
    OpenPodcast { id: String },
    MarkPosition,
    TagGenre,
}

impl From<core::MenuAction> for MenuAction {
    fn from(action: core::MenuAction) -> Self {
        match action {
            core::MenuAction::PlayNext => MenuAction::PlayNext,
            core::MenuAction::AddToQueue => MenuAction::AddToQueue,
            core::MenuAction::RemoveFromQueue { key } => MenuAction::RemoveFromQueue { key },
            core::MenuAction::AddToLastPlaylist { playlist_id, title } => {
                MenuAction::AddToLastPlaylist { playlist_id, title }
            }
            core::MenuAction::OpenPlaylistPicker => MenuAction::OpenPlaylistPicker,
            core::MenuAction::OpenReplace => MenuAction::OpenReplace,
            core::MenuAction::Download { quality } => MenuAction::Download { quality },
            core::MenuAction::Undownload => MenuAction::Undownload,
            core::MenuAction::Share => MenuAction::Share,
            core::MenuAction::StartRadio => MenuAction::StartRadio,
            core::MenuAction::Favorite { on } => MenuAction::Favorite { on },
            core::MenuAction::Dislike { on } => MenuAction::Dislike { on },
            core::MenuAction::OpenCredits => MenuAction::OpenCredits,
            core::MenuAction::GoToArtist { id } => MenuAction::GoToArtist { id },
            core::MenuAction::GoToAlbum { id } => MenuAction::GoToAlbum { id },
            core::MenuAction::OpenPodcast { id } => MenuAction::OpenPodcast { id },
            core::MenuAction::MarkPosition => MenuAction::MarkPosition,
            core::MenuAction::TagGenre => MenuAction::TagGenre,
        }
    }
}

/// One actionable row of the menu.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    /// The French label, exactly the SPA's.
    pub label: String,
    /// The SPA's icon NAME (`heart`, `disc`, `bookmarkPlus`…): the sheet maps it
    /// to a Sillon glyph, so this side never names a glyph the app may not have.
    pub icon: String,
    pub action: MenuAction,
    /// An accordion of choices (the qualities, the credited artists).
    pub sub: Vec<MenuItem>,
    pub danger: bool,
}

impl From<core::MenuItem> for MenuItem {
    fn from(item: core::MenuItem) -> Self {
        MenuItem {
            label: item.label,
            icon: item.icon.to_owned(),
            action: item.action.into(),
            sub: item.sub.into_iter().map(MenuItem::from).collect(),
            danger: item.danger,
        }
    }
}

/// One entry: a row, or the hairline the SPA writes as `"divider"`.
#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    Item { item: MenuItem },
    Divider,
}

impl From<core::MenuEntry> for MenuEntry {
    fn from(entry: core::MenuEntry) -> Self {
        match entry {
            core::MenuEntry::Item(item) => MenuEntry::Item {
                item: MenuItem::from(item),
            },
            core::MenuEntry::Divider => MenuEntry::Divider,
        }
    }
}

/// Who is asking: the platform's own facts about the person, so the menu of a row
/// is the right one without a request.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct MenuActor {
    /// The account administers this server.
    pub admin: bool,
    /// The last playlist used, for the one-tap shortcut.
    pub last_playlist: Option<LastPlaylist>,
}

/// A playlist the picker offers.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct PlaylistChoice {
    pub id: String,
    pub title: String,
    pub cover: Option<String>,
    pub nb_tracks: u32,
    /// The person may write to it: a viewer's list is not a target.
    pub editable: bool,
    /// `owner`, `editor`, `viewer`.
    pub role: Option<String>,
    pub owner: Option<String>,
    /// The last playlist used, so its row carries « récente ».
    pub recent: bool,
    /// The playlist's content counter.
    pub version: u32,
}

impl From<core::PlaylistChoice> for PlaylistChoice {
    fn from(choice: core::PlaylistChoice) -> Self {
        PlaylistChoice {
            id: choice.id,
            title: choice.title,
            cover: choice.cover,
            nb_tracks: choice.nb_tracks,
            editable: choice.editable,
            role: choice.role,
            owner: choice.owner,
            recent: choice.recent,
            version: choice.version,
        }
    }
}

/// One stand-in the replacement sheet offers.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct ReplaceCandidate {
    pub key: String,
    /// What `/api/replace` names it by.
    pub id: String,
    pub title: String,
    pub artists: String,
    pub album: String,
    pub duration_s: u32,
    pub cover: Option<String>,
    /// `deezer`, `local`, `youtube` — the sheet's source mark.
    pub source: String,
    /// A file already on this server.
    pub local: bool,
}

impl From<core::ReplaceCandidate> for ReplaceCandidate {
    fn from(candidate: core::ReplaceCandidate) -> Self {
        ReplaceCandidate {
            key: candidate.key,
            id: candidate.id,
            title: candidate.title,
            artists: candidate.artists,
            album: candidate.album,
            duration_s: candidate.duration_s,
            cover: candidate.cover,
            source: candidate.source,
            local: candidate.local,
        }
    }
}

/// The replacement sheet's answer: the matches, and where the fallback answered
/// on its own.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct ReplaceAnswer {
    /// The query the server actually searched.
    pub query: String,
    pub candidates: Vec<ReplaceCandidate>,
    /// The fallback source that answered unprompted, when the main one found
    /// nothing.
    pub fallback_auto: Option<String>,
}

impl From<core::ReplaceAnswer> for ReplaceAnswer {
    fn from(answer: core::ReplaceAnswer) -> Self {
        ReplaceAnswer {
            query: answer.query,
            candidates: answer
                .candidates
                .into_iter()
                .map(ReplaceCandidate::from)
                .collect(),
            fallback_auto: answer.fallback_auto,
        }
    }
}

/// One title the server refused to delete, and why.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct KeptTitle {
    pub id: String,
    /// `archived`, `playable`, `error` — the server's own key; the sheet says it
    /// in French.
    pub reason: String,
}

/// A job's answer: what a replacement or a bulk deletion did (rule 2: a long job
/// says where it is, and names the CAUSE of each title it kept).
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct MutationReport {
    /// The server's job handle; `None` for a finished, non-job mutation.
    pub job: Option<String>,
    pub running: bool,
    pub ok: bool,
    pub playlists: u32,
    pub favorites: u32,
    pub deleted: Vec<String>,
    pub kept: Vec<KeptTitle>,
    pub missing: Vec<String>,
    pub done: u32,
    pub total: u32,
    pub error: Option<String>,
}

impl From<core::MutationReport> for MutationReport {
    fn from(report: core::MutationReport) -> Self {
        MutationReport {
            job: report.job,
            running: report.running,
            ok: report.ok,
            playlists: report.playlists,
            favorites: report.favorites,
            deleted: report.deleted,
            kept: report
                .kept
                .into_iter()
                .map(|kept| KeptTitle {
                    id: kept.id,
                    reason: kept.reason,
                })
                .collect(),
            missing: report.missing,
            done: report.done,
            total: report.total,
            error: report.error,
        }
    }
}

/// "These rows changed" — what every surface holding a `ListHandle` re-windows on.
#[uniffi::export(callback_interface)]
pub trait ActionsListener: Send + Sync {
    /// The keys whose rows changed, and the counter (`track_actions_version()`)
    /// the screen compares.
    fn on_rows_changed(&self, keys: Vec<String>, version: u64);
}

/// Carries the core's own signal to the Kotlin listener (it runs on the runtime's
/// worker, never on Kotlin's main thread).
struct ActionsSink(std::sync::Arc<dyn ActionsListener>);

impl core::ActionSink for ActionsSink {
    fn on_rows_changed(&self, keys: Vec<String>, version: u64) {
        self.0.on_rows_changed(keys, version);
    }
}

/// Subscribes the app's ONE surface to the "rows changed" signal.
///
/// The core is one per process, so this replaces a previous subscription rather
/// than adding a second: the shell calls it once.
#[uniffi::export]
pub fn subscribe_track_actions(listener: Box<dyn ActionsListener>) {
    core::subscribe(std::sync::Arc::new(ActionsSink(std::sync::Arc::from(
        listener,
    ))));
}

/// The counter `ActionsListener::on_rows_changed` carries: compare two readings.
#[uniffi::export]
pub fn track_actions_version() -> u64 {
    core::rows_version()
}

#[uniffi::export(async_runtime = "tokio")]
impl Screens {
    // -- the menu -------------------------------------------------------------

    /// The menu of `row`, exactly `webapp/src/lib/trackmenu.js`. **Synchronous
    /// and free**: no request, no lock, no disk — a long press answers in the
    /// frame it happened.
    pub fn track_menu(&self, row: TrackRow, ctx: MenuContext) -> Vec<MenuEntry> {
        let ctx: core::MenuContext = ctx.into();
        let row: nz_core::lists::Row = row.into();
        self.core()
            .track_menu(&row, &ctx)
            .into_iter()
            .map(MenuEntry::from)
            .collect()
    }

    /// Who is asking (the admin flag) and the last playlist used.
    pub fn menu_actor(&self) -> MenuActor {
        let actor = self.core().actor();
        MenuActor {
            admin: actor.admin,
            last_playlist: actor.last_playlist.map(LastPlaylist::from),
        }
    }

    // -- the picker -----------------------------------------------------------

    /// The playlist list this device last saw — disk only, **0 requests**.
    pub fn cached_menu_playlists(&self) -> Option<Vec<PlaylistChoice>> {
        self.core()
            .cached_menu_playlists()
            .map(|lists| lists.into_iter().map(PlaylistChoice::from).collect())
    }

    /// `GET /api/me/playlists` — ONE request, kept for the next opening.
    pub async fn menu_playlists(&self) -> Result<Vec<PlaylistChoice>, NzError> {
        self.core()
            .menu_playlists()
            .await
            .map(|lists| lists.into_iter().map(PlaylistChoice::from).collect())
            .map_err(NzError::from)
    }

    /// `POST /api/playlist/<id>/tracks` — append `keys` (one request).
    pub async fn add_to_playlist(
        &self,
        playlist_id: String,
        title: String,
        keys: Vec<String>,
    ) -> Result<u32, NzError> {
        self.core()
            .add_to_playlist(&playlist_id, &title, &keys)
            .await
            .map_err(NzError::from)
    }

    /// `POST /api/playlists` — create one with `keys` already in it.
    pub async fn create_playlist_from(
        &self,
        title: String,
        keys: Vec<String>,
    ) -> Result<PlaylistChoice, NzError> {
        self.core()
            .create_playlist(&title, &keys)
            .await
            .map(PlaylistChoice::from)
            .map_err(NzError::from)
    }

    // -- the heart and the thumb ---------------------------------------------

    /// `POST /api/favorite` — star it. The every-open-list patch happens BEFORE
    /// the request and is rolled back with the cause if the server refuses.
    pub async fn like(&self, key: String) -> Result<(), NzError> {
        self.core()
            .set_track_favorite(&key, true)
            .await
            .map_err(NzError::from)
    }

    /// `POST /api/favorite {on: false}`.
    pub async fn unlike(&self, key: String) -> Result<(), NzError> {
        self.core()
            .set_track_favorite(&key, false)
            .await
            .map_err(NzError::from)
    }

    /// `POST /api/dislike` — the title leaves the Flow, the radios and the mixes.
    pub async fn dislike(&self, key: String) -> Result<(), NzError> {
        self.core()
            .set_track_dislike(&key, true)
            .await
            .map_err(NzError::from)
    }

    /// Take "Je n'aime pas" back.
    pub async fn undislike(&self, key: String) -> Result<(), NzError> {
        self.core()
            .set_track_dislike(&key, false)
            .await
            .map_err(NzError::from)
    }

    // -- replacing a dead title ----------------------------------------------

    /// `GET /api/replace/candidates/<id>` — ONE request. An empty `query` is the
    /// server's own; `fallbacks` also asks the fallback sources.
    pub async fn replace_candidates(
        &self,
        key: String,
        query: String,
        fallbacks: bool,
    ) -> Result<ReplaceAnswer, NzError> {
        self.core()
            .replace_candidates(&key, &query, fallbacks)
            .await
            .map(ReplaceAnswer::from)
            .map_err(NzError::from)
    }

    /// `POST /api/replace` — swap it everywhere. Answers the server's job.
    pub async fn replace(&self, key: String, candidate: String) -> Result<MutationReport, NzError> {
        self.core()
            .replace(&key, &candidate)
            .await
            .map(MutationReport::from)
            .map_err(NzError::from)
    }

    /// `GET /api/replace/status/<job>` — how the rewrite is going.
    pub async fn replace_status(&self, job: String) -> Result<MutationReport, NzError> {
        self.core()
            .replace_status(&job)
            .await
            .map(MutationReport::from)
            .map_err(NzError::from)
    }

    // -- deleting ------------------------------------------------------------

    /// `DELETE /api/local/<id>` — one of your own uploads, row and file. Each key
    /// keeps its own cause.
    pub async fn delete_uploads(&self, keys: Vec<String>) -> Result<MutationReport, NzError> {
        self.core()
            .delete_uploads(&keys)
            .await
            .map(MutationReport::from)
            .map_err(NzError::from)
    }

    /// `POST /api/tracks/delete` — the bulk deletion of titles gone from both
    /// sources (at most 200; the server re-checks each).
    pub async fn delete_tracks(&self, keys: Vec<String>) -> Result<MutationReport, NzError> {
        self.core()
            .delete_tracks(&keys)
            .await
            .map(MutationReport::from)
            .map_err(NzError::from)
    }

    /// `GET /api/replace/status/<job>` — the bulk deletion's progress and causes.
    pub async fn delete_status(&self, job: String) -> Result<MutationReport, NzError> {
        self.core()
            .delete_status(&job)
            .await
            .map(MutationReport::from)
            .map_err(NzError::from)
    }

    // -- sharing -------------------------------------------------------------

    /// The address to share for `(kind, id)`:
    ///
    /// - `playlist`: the public read-only page (minted on the first ask, reused
    ///   after);
    /// - `track` / `episode`: `GET /api/share/file/<id>` on this server — the
    ///   whole archived file, and no request to build it.
    pub async fn share_link(&self, kind: String, id: String) -> Result<String, NzError> {
        self.core()
            .share_link(&kind, &id)
            .await
            .map_err(NzError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_core::lists::Row;

    fn ctx() -> MenuContext {
        MenuContext {
            surface: "list".to_owned(),
            index: None,
            current_index: None,
            can_keep: true,
            can_browse: true,
            admin: false,
            remote: false,
            downloaded: false,
            last_playlist_id: None,
            last_playlist_title: None,
            last_playlist_editable: false,
            artists: Vec::new(),
            episode: false,
            episode_playing: false,
            channel_id: None,
            credits: true,
            offline: false,
            genre_tag: false,
            markers: false,
        }
    }

    fn labels(entries: &[MenuEntry]) -> Vec<&str> {
        entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item { item } => Some(item.label.as_str()),
                MenuEntry::Divider => None,
            })
            .collect()
    }

    /// The builder's entries cross whole: label, the SPA's icon name, the verb,
    /// the submenu and the hairline — nothing is flattened into a string the sheet
    /// would have to parse.
    #[test]
    fn a_menu_crosses_with_its_labels_glyphs_verbs_and_submenus() {
        let row = Row::test_row(1, "Ouverture");
        let mut context = ctx();
        context.offline = true;
        context.last_playlist_id = Some("p-1".to_owned());
        context.last_playlist_title = Some("Route du soir".to_owned());
        context.last_playlist_editable = true;
        context.artists = vec![
            MenuPerson {
                id: "a-1".to_owned(),
                name: "Ascenseur".to_owned(),
            },
            MenuPerson {
                id: "a-2".to_owned(),
                name: "June".to_owned(),
            },
        ];
        let built: core::MenuContext = context.into();
        let entries: Vec<MenuEntry> = core::build(&row, &built)
            .into_iter()
            .map(MenuEntry::from)
            .collect();

        let names = labels(&entries);
        assert_eq!(names.first(), Some(&"Lire ensuite"));
        assert!(names.contains(&"Ajouter à « Route du soir »"));
        assert!(names.contains(&"Ajouter à une playlist…"));
        assert!(names.contains(&"Télécharger en…"));
        assert!(names.contains(&"Aller à l'album"));

        // The submenus crossed with their own verbs.
        let qualities = entries
            .iter()
            .find_map(|entry| match entry {
                MenuEntry::Item { item } if item.label == "Télécharger en…" => Some(&item.sub),
                _ => None,
            })
            .expect("the quality submenu");
        assert_eq!(qualities.len(), 6);
        assert_eq!(
            qualities[1].action,
            MenuAction::Download {
                quality: Some("OPUS_320".to_owned())
            }
        );
        assert_eq!(qualities[1].icon, "download");

        let artists = entries
            .iter()
            .find_map(|entry| match entry {
                MenuEntry::Item { item } if item.label == "Aller à l'artiste" => Some(&item.sub),
                _ => None,
            })
            .expect("the artist submenu");
        assert_eq!(artists.len(), 2);
        assert_eq!(
            artists[1].action,
            MenuAction::GoToArtist {
                id: "a-2".to_owned()
            }
        );

        // The heart's glyph and verb travel together.
        let heart = entries
            .iter()
            .find_map(|entry| match entry {
                MenuEntry::Item { item } if item.label == "Ajouter aux favoris" => Some(item),
                _ => None,
            })
            .expect("the heart");
        assert_eq!(heart.icon, "heart");
        assert_eq!(heart.action, MenuAction::Favorite { on: true });
        assert!(!heart.danger);
        assert!(entries.contains(&MenuEntry::Divider));
    }

    /// Every verb of the spec survives the conversion — the match is exhaustive,
    /// so a variant added to the core cannot be silently dropped here.
    #[test]
    fn every_verb_crosses_the_boundary() {
        let verbs = [
            core::MenuAction::PlayNext,
            core::MenuAction::AddToQueue,
            core::MenuAction::RemoveFromQueue {
                key: "deezer:1".to_owned(),
            },
            core::MenuAction::AddToLastPlaylist {
                playlist_id: "p".to_owned(),
                title: "t".to_owned(),
            },
            core::MenuAction::OpenPlaylistPicker,
            core::MenuAction::OpenReplace,
            core::MenuAction::Download { quality: None },
            core::MenuAction::Undownload,
            core::MenuAction::Share,
            core::MenuAction::StartRadio,
            core::MenuAction::Favorite { on: true },
            core::MenuAction::Dislike { on: false },
            core::MenuAction::OpenCredits,
            core::MenuAction::GoToArtist { id: "a".to_owned() },
            core::MenuAction::GoToAlbum { id: "b".to_owned() },
            core::MenuAction::OpenPodcast { id: "c".to_owned() },
            core::MenuAction::MarkPosition,
            core::MenuAction::TagGenre,
        ];
        let crossed: Vec<MenuAction> = verbs.into_iter().map(MenuAction::from).collect();
        assert!(matches!(crossed[0], MenuAction::PlayNext));
        assert!(matches!(
            &crossed[2],
            MenuAction::RemoveFromQueue { key } if key == "deezer:1"
        ));
        assert!(matches!(
            &crossed[3],
            MenuAction::AddToLastPlaylist { playlist_id, title } if playlist_id == "p" && title == "t"
        ));
        assert!(matches!(
            &crossed[15],
            MenuAction::OpenPodcast { id } if id == "c"
        ));
        assert_eq!(crossed.len(), 18);
    }

    /// The surface tells the core which builder and which gates: `"queue"` parses
    /// as the queue, and a gate left false hides the entry it owns.
    #[test]
    fn the_context_carries_the_surface_and_the_gates() {
        let mut context = ctx();
        context.surface = "queue".to_owned();
        context.index = Some(3);
        context.current_index = Some(1);
        let built: core::MenuContext = context.clone().into();
        assert_eq!(built.surface, core::Surface::Queue);
        assert_eq!(built.index, Some(3));

        context.surface = "".to_owned();
        let list: core::MenuContext = context.into();
        assert_eq!(
            list.surface,
            core::Surface::List,
            "empty is an ordinary list"
        );

        let mut admin = ctx();
        admin.admin = true;
        let built: core::MenuContext = admin.into();
        let names: Vec<String> = core::build(&Row::test_row(1, "T"), &built)
            .into_iter()
            .filter_map(|entry| match entry {
                core::MenuEntry::Item(item) => Some(item.label),
                core::MenuEntry::Divider => None,
            })
            .collect();
        assert!(names.contains(&"Remplacer le titre…".to_owned()));
        assert!(
            !names.contains(&"Étiqueter le genre…".to_owned()),
            "MB18's entry is absent until the device has it"
        );
    }

    /// A job's report crosses with each kept title's CAUSE — never a count.
    #[test]
    fn a_report_crosses_with_every_cause() {
        let report = core::MutationReport {
            job: Some("job-1".to_owned()),
            running: false,
            ok: true,
            playlists: 2,
            favorites: 1,
            deleted: vec!["deezer:9001".to_owned()],
            kept: vec![core::Kept {
                id: "deezer:9002".to_owned(),
                reason: "playable".to_owned(),
            }],
            missing: vec!["deezer:9003".to_owned()],
            done: 3,
            total: 3,
            error: None,
        };
        let crossed = MutationReport::from(report);
        assert_eq!(crossed.job.as_deref(), Some("job-1"));
        assert!(crossed.ok && !crossed.running);
        assert_eq!(crossed.kept[0].reason, "playable");
        assert_eq!(crossed.missing, ["deezer:9003"]);
        assert_eq!((crossed.done, crossed.total), (3, 3));
    }

    /// The replacement answer crosses with the source mark the sheet draws.
    #[test]
    fn the_replacement_answer_crosses_with_its_source_mark() {
        let answer = core::ReplaceAnswer {
            query: "Ascenseur Ouverture".to_owned(),
            candidates: vec![core::ReplaceCandidate {
                key: "local:uuid".to_owned(),
                id: "uuid".to_owned(),
                title: "Ouverture".to_owned(),
                artists: "Ascenseur".to_owned(),
                album: "Le matin".to_owned(),
                album_id: None,
                duration_s: 181,
                cover: None,
                source: "local".to_owned(),
                local: true,
            }],
            fallback_auto: Some("youtube".to_owned()),
        };
        let crossed = ReplaceAnswer::from(answer);
        assert_eq!(crossed.query, "Ascenseur Ouverture");
        assert!(crossed.candidates[0].local);
        assert_eq!(crossed.candidates[0].source, "local");
        assert_eq!(crossed.fallback_auto.as_deref(), Some("youtube"));
    }

    /// The picker's row crosses with what it decides on: writability, whose list
    /// it is, and the « récente » badge.
    #[test]
    fn a_playlist_choice_crosses_with_its_role_and_badge() {
        let choice = core::PlaylistChoice {
            id: "p-1".to_owned(),
            title: "Route du soir".to_owned(),
            cover: Some("/api/cover/p-1".to_owned()),
            nb_tracks: 4,
            editable: false,
            role: Some("viewer".to_owned()),
            owner: Some("June".to_owned()),
            recent: true,
            version: 7,
        };
        let crossed = PlaylistChoice::from(choice);
        assert!(!crossed.editable, "a viewer's list is not a target");
        assert_eq!(crossed.role.as_deref(), Some("viewer"));
        assert_eq!(crossed.owner.as_deref(), Some("June"));
        assert!(crossed.recent);
        assert_eq!(crossed.version, 7);
    }
}
