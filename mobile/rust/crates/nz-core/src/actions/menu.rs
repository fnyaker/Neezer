//! The ONE track-actions spec, `webapp/src/lib/trackmenu.js` PORTED.
//!
//! Every surface that lists a title — a playlist, the queue, a search hit, an
//! episode list, the history, the "Je n'aime pas" list, a release's preview rows
//! — opens the SAME menu, because the labels and the actions are built once,
//! here, and a surface only says which row it is about. `webapp/test/menu/run.mjs`
//! is the SPA's own gate for it (242 checks); this file is the port of the
//! builder it holds to, and its tests carry that bench's expected lists.
//!
//! # Why it is data, and not five closures
//!
//! The SPA's builder returns items whose `action` is a JavaScript closure. Across
//! the FFI a closure cannot travel, so an item carries a [`MenuAction`] — the
//! verb, plus the little data a verb needs (a playlist's id and title, the artist
//! page's id) — and the Kotlin side runs it. What that buys: the menu of every
//! list is built by THIS code, in one place, and the Kotlin sheet draws whatever
//! it is handed (label, glyph, submenu, danger) without a second opinion.
//!
//! # What a menu costs
//!
//! **Zero requests.** `build` reads the row it is given and the [`MenuContext`]
//! the surface passes, and nothing else: no store is consulted, no favourite is
//! fetched, no playlist list is loaded. That is the SPA's own measure ("opening
//! it costs 0 `/api` request, 32 measured opens") and it is the rule that keeps a
//! long press instant on a bad link.
//!
//! # What a row cannot name, the menu does not offer
//!
//! The SPA's track payload carries `artists[]` with each artist's id, so
//! `trackmenu.js` can always offer "Aller à l'artiste". A phone row
//! ([`Row`]) carries the artists' NAMES joined for display and its album's id,
//! and no artist id: the surface that knows the credits passes them in
//! [`MenuContext::artists`], and a row that does not gets no "Aller à l'artiste"
//! — the SPA's own principle ("a link must lead somewhere", `lib/entity.js`),
//! applied to a shape that cannot name the page. "Aller à l'album" needs only
//! [`Row::album_id`], which every library row has.
//!
//! # What other lots own, and how a menu entry waits for them
//!
//! Four entries belong to a feature that is not on the device yet. Each one is
//! gated on what the context says the device can do, so a menu never carries a
//! button that does nothing (rule 5):
//!
//! | Entry | Owner | Gate |
//! |---|---|---|
//! | "Télécharger" / "Télécharger en…" | MB15 (offline) | [`MenuContext::offline`] |
//! | "Étiqueter le genre…" | MB18 (genre studio) | [`MenuContext::genre_tag`] |
//! | "Marquer cette position" | MB13 (podcast markers) | [`MenuContext::markers`] |
//!
//! The default is `false` in every case: a context that says nothing gets the
//! menu this build can honour, and the lot that lands the feature flips ONE flag
//! at the call site.

use crate::lists::Row;

/// The qualities the SPA's "Télécharger en…" submenu offers
/// (`trackmenu.js#DL_QUALITIES`), in its order.
pub const DL_QUALITIES: [(&str, &str); 6] = [
    ("FLAC", "FLAC"),
    ("OPUS_320", "Opus 320"),
    ("OPUS_256", "Opus 256"),
    ("OPUS_192", "Opus 192"),
    ("OPUS_128", "Opus 128"),
    ("OPUS_64", "Opus 64"),
];

/// Which surface opened the menu. The two differ exactly as the SPA's do: a
/// queue row is not a place to queue the same title again, and it is the only
/// place that removes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Surface {
    /// Any list of titles: a playlist, an album, a search hit, the history…
    #[default]
    List,
    /// The queue editor. `webapp.md`: only what is still AHEAD of the playhead
    /// can be removed, and the playing row cannot.
    Queue,
}

impl Surface {
    /// `"queue"` is the queue; anything else — including an empty string, which
    /// is what a caller that does not care passes — is an ordinary list.
    pub fn parse(value: &str) -> Surface {
        if value.trim().eq_ignore_ascii_case("queue") {
            Surface::Queue
        } else {
            Surface::List
        }
    }
}

/// One credited artist the surface happens to know, so "Aller à l'artiste" can
/// name a page. `id` is what the artist route accepts (`entityRef`: the row's
/// UUID when the library holds one, else the provider's id).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MenuPerson {
    pub id: String,
    pub name: String,
}

/// The last playlist a title was added to — the SPA's `lastPlaylist` store, which
/// the menu offers as a one-tap shortcut. Kept by this area between two openings
/// (and across launches, in the kv store).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LastPlaylist {
    pub id: String,
    pub title: String,
    /// A viewer's list is not a target: the shortcut only shows for a list the
    /// person may write to (`p.editable` in the SPA).
    pub editable: bool,
}

/// Everything the menu reads besides the row itself.
///
/// The defaults are the SPA's normal case: a locally-owned account looking at
/// its own list, with nothing downloaded, no remote link, and no optional
/// feature on the device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MenuContext {
    pub surface: Surface,
    /// The row's position in the queue, when the surface knows it.
    pub index: Option<u32>,
    /// The playing row's position: a queue row is removable only when it is
    /// strictly AFTER it (`options.index > options.currentIndex`).
    pub current_index: Option<i32>,
    /// `CAN_KEEP`: a read-only remote control shows the owner's hearts but cannot
    /// move them, so every entry that writes sets this false.
    pub can_keep: bool,
    /// `CAN_BROWSE`: whether a link out of the menu leads anywhere at all.
    pub can_browse: bool,
    /// The account administers this server (`isAdmin`): swapping a track
    /// everywhere it appears is theirs to do.
    pub admin: bool,
    /// This device is a remote control of another player: the download entries
    /// and the episode's marker are not offered (the audio is elsewhere).
    pub remote: bool,
    /// The title is already on this device ([`Row::downloaded`]).
    pub downloaded: bool,
    /// The last playlist used, for the one-tap shortcut.
    pub last_playlist: Option<LastPlaylist>,
    /// The credited artists the surface knows, main first. Empty for a row whose
    /// payload carries names only.
    pub artists: Vec<MenuPerson>,
    /// A podcast episode: the SPA's `buildEpisodeMenu`, not `buildTrackMenu`.
    pub episode: bool,
    /// This very episode is the one playing, so "Marquer cette position" can name
    /// an instant.
    pub episode_playing: bool,
    /// The show an episode belongs to (`ep.channel_id` in the SPA).
    pub channel_id: Option<String>,
    /// The credits sheet is installed on this device (MB7's component, wired by
    /// the shell): without it the entry would lead nowhere.
    pub credits: bool,
    /// The offline lot is on this device (MB15).
    pub offline: bool,
    /// The genre studio is on this device (MB18).
    pub genre_tag: bool,
    /// The podcast markers are on this device (MB13).
    pub markers: bool,
}

impl MenuContext {
    /// A locally-owned account, looking at its own list: the SPA's normal case.
    pub fn own_list() -> MenuContext {
        MenuContext {
            can_keep: true,
            can_browse: true,
            credits: true,
            ..MenuContext::default()
        }
    }

    /// The queue editor's context, with the playhead the surface knows.
    pub fn queue(index: u32, current_index: Option<i32>) -> MenuContext {
        MenuContext {
            surface: Surface::Queue,
            index: Some(index),
            current_index,
            ..MenuContext::own_list()
        }
    }

    /// Whether this queue row may be removed: only what is still ahead of the
    /// playhead (`trackmenu.js#buildTrackMenu`), and only in the queue.
    fn removable(&self) -> bool {
        let Some(index) = self.index else {
            return false;
        };
        match self.surface {
            Surface::List => false,
            Surface::Queue => i64::from(index) > i64::from(self.current_index.unwrap_or(-1)),
        }
    }
}

/// What an item does when it is chosen — the SPA's `action` closure, as a verb.
///
/// Nothing here sends a request by itself: the Kotlin side runs the verb through
/// this lot's FFI methods (or the player, or the shell's routes), which is what
/// keeps a menu instant and a failure's cause in one place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    /// Insert the title right after the playing one (`player.playNext`).
    PlayNext,
    /// Append the title to the queue (`player.addToQueue`).
    AddToQueue,
    /// Remove this queue row — by KEY, so a playhead that moved under an open
    /// menu cannot remove the wrong row (`webapp.md`'s own lesson).
    RemoveFromQueue { key: String },
    /// One tap into the last playlist used.
    AddToLastPlaylist { playlist_id: String, title: String },
    /// Open the searchable picker.
    OpenPlaylistPicker,
    /// The replacement sheet, with the dead title already in hand.
    OpenReplace,
    /// Download at the default quality, or at `quality` (a [`DL_QUALITIES`] id).
    Download { quality: Option<String> },
    /// Drop this device's copy.
    Undownload,
    /// Produce a link and hand it to the platform's share sheet.
    Share,
    /// A station seeded on this title.
    StartRadio,
    /// Star or unstar it (`on` is the value to write).
    Favorite { on: bool },
    /// "Je n'aime pas", or take it back.
    Dislike { on: bool },
    /// The credits sheet over the current screen.
    OpenCredits,
    /// A credit's artist page.
    GoToArtist { id: String },
    /// The title's album page.
    GoToAlbum { id: String },
    /// The show an episode belongs to.
    OpenPodcast { id: String },
    /// A one-tap marker at the playing position (MB13).
    MarkPosition,
    /// The genre tag sheet over the current screen (MB18).
    TagGenre,
}

/// One entry of the menu: a row, or the hairline the SPA writes as `"divider"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    Item(MenuItem),
    Divider,
}

/// One actionable row: what it says, the glyph it draws, and what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    /// The French label, exactly the SPA's (`trackmenu.js`, one place).
    pub label: String,
    /// The SPA's icon NAME (`heart`, `heartFilled`, `disc`…): the Kotlin sheet maps
    /// it to a Sillon glyph, so this side never names a glyph the app may not have.
    pub icon: &'static str,
    pub action: MenuAction,
    /// An accordion of choices (the qualities, the credited artists). Empty for a
    /// leaf.
    pub sub: Vec<MenuItem>,
    /// A destructive entry (the SPA's `.danger`).
    pub danger: bool,
}

impl MenuItem {
    fn new(label: impl Into<String>, icon: &'static str, action: MenuAction) -> MenuItem {
        MenuItem {
            label: label.into(),
            icon,
            action,
            sub: Vec::new(),
            danger: false,
        }
    }

    fn with_sub(mut self, sub: Vec<MenuItem>) -> MenuItem {
        self.sub = sub;
        self
    }
}

/// The items for `row`, exactly `trackmenu.js#buildTrackMenu` (or
/// `#buildEpisodeMenu` when the context says it is an episode).
///
/// Pure: no lock, no disk, no request. What it reads is `row` and `ctx`.
pub fn build(row: &Row, ctx: &MenuContext) -> Vec<MenuEntry> {
    if ctx.episode {
        episode_menu(row, ctx)
    } else {
        track_menu(row, ctx)
    }
}

/// `buildEpisodeMenu`: an episode is shaped like a track but its id is a UUID and
/// its "artist"/"album" are the show, so the normal menu's favourite, radio,
/// playlist and go-to links would answer `400`/`404`. Its own minimal menu.
fn episode_menu(row: &Row, ctx: &MenuContext) -> Vec<MenuEntry> {
    let mut entries = Vec::new();
    if ctx.removable() {
        entries.push(item(MenuItem::new(
            "Retirer de la file",
            "close",
            MenuAction::RemoveFromQueue {
                key: row.key.clone(),
            },
        )));
    } else if ctx.surface == Surface::List {
        entries.push(item(MenuItem::new(
            "Lire ensuite",
            "next",
            MenuAction::PlayNext,
        )));
        entries.push(item(MenuItem::new(
            "Ajouter à la file",
            "queue",
            MenuAction::AddToQueue,
        )));
    }
    // Playing this very episode: offer a one-tap marker at the current position.
    if ctx.episode_playing && ctx.markers {
        entries.push(item(MenuItem::new(
            "Marquer cette position",
            "bookmarkPlus",
            MenuAction::MarkPosition,
        )));
    }
    entries.push(item(MenuItem::new("Partager…", "share", MenuAction::Share)));
    if ctx.offline && !ctx.remote {
        entries.push(MenuEntry::Divider);
        entries.push(item(download_item(ctx)));
    }
    if let Some(channel) = ctx.channel_id.as_ref().filter(|id| !id.is_empty()) {
        entries.push(MenuEntry::Divider);
        entries.push(item(MenuItem::new(
            "Ouvrir le podcast",
            "mic",
            MenuAction::OpenPodcast {
                id: channel.clone(),
            },
        )));
    }
    normalise(entries)
}

/// `buildTrackMenu`, in the SPA's own order.
fn track_menu(row: &Row, ctx: &MenuContext) -> Vec<MenuEntry> {
    let mut entries = Vec::new();
    if ctx.removable() {
        entries.push(item(MenuItem::new(
            "Retirer de la file",
            "close",
            MenuAction::RemoveFromQueue {
                key: row.key.clone(),
            },
        )));
    } else if ctx.surface == Surface::List {
        entries.push(item(MenuItem::new(
            "Lire ensuite",
            "next",
            MenuAction::PlayNext,
        )));
        entries.push(item(MenuItem::new(
            "Ajouter à la file",
            "queue",
            MenuAction::AddToQueue,
        )));
    }

    // Adding to a playlist is every account's business (a shared playlist with an
    // editor is a legitimate target), so these follow `can_keep`.
    if ctx.can_keep {
        if let Some(last) = ctx.last_playlist.as_ref().filter(|last| last.editable) {
            entries.push(item(MenuItem::new(
                format!("Ajouter à « {} »", last.title),
                "plus",
                MenuAction::AddToLastPlaylist {
                    playlist_id: last.id.clone(),
                    title: last.title.clone(),
                },
            )));
        }
        entries.push(item(MenuItem::new(
            "Ajouter à une playlist…",
            "music",
            MenuAction::OpenPlaylistPicker,
        )));
    }

    if ctx.admin && ctx.can_keep {
        // Always offered — a track can also be simply *wrong* (bad master, wrong
        // version), not only dead.
        entries.push(item(MenuItem::new(
            "Remplacer le titre…",
            "refresh",
            MenuAction::OpenReplace,
        )));
        // Tagging from wherever the track is (MB18's sheet, over this screen).
        if ctx.genre_tag {
            entries.push(item(MenuItem::new(
                "Étiqueter le genre…",
                "tag",
                MenuAction::TagGenre,
            )));
        }
    }

    // Offline download — device-local, so available to every account. Not on a
    // remote control: this device is not the one playing.
    if ctx.offline && !ctx.remote {
        entries.push(MenuEntry::Divider);
        entries.push(item(download_item(ctx)));
        if !ctx.downloaded {
            let sub = DL_QUALITIES
                .iter()
                .map(|(id, label)| {
                    MenuItem::new(
                        *label,
                        "download",
                        MenuAction::Download {
                            quality: Some((*id).to_owned()),
                        },
                    )
                })
                .collect();
            entries.push(item(
                MenuItem::new(
                    "Télécharger en…",
                    "download",
                    MenuAction::Download { quality: None },
                )
                .with_sub(sub),
            ));
        }
    }

    entries.push(MenuEntry::Divider);
    if ctx.can_keep {
        entries.push(item(MenuItem::new("Partager…", "share", MenuAction::Share)));
    }
    entries.push(item(MenuItem::new(
        "Lancer la radio",
        "radio",
        MenuAction::StartRadio,
    )));
    if ctx.can_keep {
        entries.push(item(MenuItem::new(
            if row.liked {
                "Retirer des favoris"
            } else {
                "Ajouter aux favoris"
            },
            if row.liked { "heartFilled" } else { "heart" },
            MenuAction::Favorite { on: !row.liked },
        )));
        entries.push(item(MenuItem::new(
            if row.disliked {
                "Retirer de « Je n'aime pas »"
            } else {
                "Je n'aime pas"
            },
            if row.disliked {
                "thumbsDownFilled"
            } else {
                "thumbsDown"
            },
            MenuAction::Dislike { on: !row.disliked },
        )));
    }

    entries.push(MenuEntry::Divider);
    // Who wrote, composed and produced it — over the current screen.
    if ctx.can_browse && ctx.credits {
        entries.push(item(MenuItem::new(
            "Voir les crédits",
            "users",
            MenuAction::OpenCredits,
        )));
    }
    // One entry per credited artist. A single credit stays a plain item (the
    // common case should not grow a submenu for nothing); a feat. track opens a
    // submenu so the guest is reachable too.
    let people: Vec<&MenuPerson> = ctx
        .artists
        .iter()
        .filter(|person| !person.id.is_empty())
        .collect();
    match people.len() {
        0 => {}
        1 => entries.push(item(MenuItem::new(
            "Aller à l'artiste",
            "user",
            MenuAction::GoToArtist {
                id: people[0].id.clone(),
            },
        ))),
        _ => {
            let sub = people
                .iter()
                .map(|person| {
                    MenuItem::new(
                        person.name.clone(),
                        "user",
                        MenuAction::GoToArtist {
                            id: person.id.clone(),
                        },
                    )
                })
                .collect();
            entries.push(item(
                MenuItem::new(
                    "Aller à l'artiste",
                    "user",
                    MenuAction::GoToArtist {
                        id: people[0].id.clone(),
                    },
                )
                .with_sub(sub),
            ));
        }
    }
    if let Some(album) = row.album_id.as_ref().filter(|id| !id.is_empty()) {
        entries.push(item(MenuItem::new(
            "Aller à l'album",
            "disc",
            MenuAction::GoToAlbum { id: album.clone() },
        )));
    }
    normalise(entries)
}

/// The one download row: "Télécharger", or "Retirer le téléchargement" once it is
/// on the device.
fn download_item(ctx: &MenuContext) -> MenuItem {
    if ctx.downloaded {
        MenuItem::new(
            "Retirer le téléchargement",
            "downloaded",
            MenuAction::Undownload,
        )
    } else {
        MenuItem::new(
            "Télécharger",
            "download",
            MenuAction::Download { quality: None },
        )
    }
}

fn item(item: MenuItem) -> MenuEntry {
    MenuEntry::Item(item)
}

/// Drops a leading, trailing or doubled divider — what a group that showed
/// nothing (a feature not on the device) would leave behind.
fn normalise(entries: Vec<MenuEntry>) -> Vec<MenuEntry> {
    let mut out: Vec<MenuEntry> = Vec::with_capacity(entries.len());
    for entry in entries {
        if matches!(entry, MenuEntry::Divider)
            && (out.is_empty() || matches!(out.last(), Some(MenuEntry::Divider)))
        {
            continue;
        }
        out.push(entry);
    }
    if matches!(out.last(), Some(MenuEntry::Divider)) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(n: usize, title: &str) -> Row {
        Row::test_row(n, title)
    }

    /// The labels a surface must carry, in order, without the dividers.
    fn labels_of(entries: &[MenuEntry]) -> Vec<&str> {
        entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) => Some(item.label.as_str()),
                MenuEntry::Divider => None,
            })
            .collect()
    }

    fn action_of<'a>(entries: &'a [MenuEntry], label: &str) -> Option<&'a MenuAction> {
        entries.iter().find_map(|entry| match entry {
            MenuEntry::Item(item) if item.label == label => Some(&item.action),
            _ => None,
        })
    }

    fn dividers(entries: &[MenuEntry]) -> usize {
        entries
            .iter()
            .filter(|entry| matches!(entry, MenuEntry::Divider))
            .count()
    }

    /// `webapp/test/menu/run.mjs#TRACK`: what a title's menu must carry, on every
    /// surface that shows one. The download entries are MB15's (their gate is
    /// off here), so they are asserted separately.
    #[test]
    fn a_library_title_carries_the_spas_own_list() {
        let mut ctx = MenuContext::own_list();
        ctx.artists = vec![MenuPerson {
            id: "aaaaaaaa-1111-4111-8111-111111111111".to_owned(),
            name: "Ascenseur".to_owned(),
        }];
        let entries = build(&row(1, "Ouverture"), &ctx);
        assert_eq!(
            labels_of(&entries),
            [
                "Lire ensuite",
                "Ajouter à la file",
                "Ajouter à une playlist…",
                "Partager…",
                "Lancer la radio",
                "Ajouter aux favoris",
                "Je n'aime pas",
                "Voir les crédits",
                "Aller à l'artiste",
                "Aller à l'album",
            ],
        );
        // The bench's `not`: a list row cannot remove anything from a queue.
        assert!(!labels_of(&entries).contains(&"Retirer de la file"));
        assert_eq!(dividers(&entries), 2, "the SPA's own two hairlines");
    }

    /// An episode has its own menu (the SPA's `buildEpisodeMenu`): no favourite,
    /// no radio, no playlist, no links — its UUID would answer 400 there.
    #[test]
    fn an_episode_menu_keeps_the_five_items_the_spa_gives_it() {
        let mut ctx = MenuContext::own_list();
        ctx.episode = true;
        ctx.channel_id = Some("dddddddd-1111-4111-8111-111111111111".to_owned());
        let entries = build(&row(2, "Épisode 12"), &ctx);
        assert_eq!(
            labels_of(&entries),
            [
                "Lire ensuite",
                "Ajouter à la file",
                "Partager…",
                "Ouvrir le podcast",
            ],
        );
        for absent in ["Aller à l'album", "Lancer la radio", "Ajouter aux favoris"] {
            assert!(!labels_of(&entries).contains(&absent), "{absent}");
        }
    }

    /// The queue's own row: what comes later can be removed, and nothing is
    /// queued twice (`run.mjs`'s "file d'attente (à venir)" surface).
    #[test]
    fn a_queue_row_ahead_of_the_playhead_removes_and_does_not_queue_again() {
        let entries = build(&row(3, "Titre 3"), &MenuContext::queue(3, Some(1)));
        let labels = labels_of(&entries);
        assert_eq!(labels.first(), Some(&"Retirer de la file"));
        assert!(!labels.contains(&"Lire ensuite"));
        assert!(!labels.contains(&"Ajouter à la file"));
        assert_eq!(
            action_of(&entries, "Retirer de la file"),
            Some(&MenuAction::RemoveFromQueue {
                key: "test:3".to_owned()
            }),
        );
    }

    /// The removing entry is named by KEY, so a playhead that moved between the
    /// open and the tap cannot remove the wrong row.
    #[test]
    fn the_playing_queue_row_cannot_be_removed() {
        let entries = build(&row(2, "Titre 2"), &MenuContext::queue(2, Some(2)));
        let labels = labels_of(&entries);
        assert!(!labels.contains(&"Retirer de la file"));
        // And a row BEFORE the playhead, too (what a repeat/shuffle leaves behind).
        let past = build(&row(0, "Titre 0"), &MenuContext::queue(0, Some(2)));
        assert!(!labels_of(&past).contains(&"Retirer de la file"));
        // Nor is a queue row ever a place to queue the same title again.
        assert!(!labels.contains(&"Lire ensuite"));
    }

    /// A queue row with no index (a surface that does not know its position)
    /// offers nothing to remove: the SPA requires an integer index.
    #[test]
    fn a_queue_row_without_an_index_offers_no_removal() {
        let ctx = MenuContext {
            surface: Surface::Queue,
            ..MenuContext::own_list()
        };
        let entries = build(&row(1, "Titre"), &ctx);
        let labels = labels_of(&entries);
        assert!(!labels.contains(&"Retirer de la file"));
        assert!(!labels.contains(&"Lire ensuite"));
        assert!(labels.contains(&"Voir les crédits"));
    }

    /// A remote control at read level writes nothing: no favourite, no dislike, no
    /// playlist, no share (the SPA's `CAN_KEEP`).
    #[test]
    fn a_read_only_remote_control_carries_no_writing_entry() {
        let ctx = MenuContext {
            can_keep: false,
            ..MenuContext::own_list()
        };
        let entries = build(&row(1, "Titre"), &ctx);
        let labels = labels_of(&entries);
        assert!(!labels.contains(&"Ajouter aux favoris"));
        assert!(!labels.contains(&"Je n'aime pas"));
        assert!(!labels.contains(&"Partager…"));
        assert!(!labels.contains(&"Ajouter à une playlist…"));
        // Reading is still allowed.
        assert!(labels.contains(&"Lancer la radio"));
        assert!(labels.contains(&"Voir les crédits"));
    }

    /// Only an admin may swap a title everywhere; the entry is absent otherwise.
    #[test]
    fn replacing_a_title_is_the_admins_verb() {
        let mut admin = MenuContext::own_list();
        admin.admin = true;
        assert!(labels_of(&build(&row(1, "Titre"), &admin)).contains(&"Remplacer le titre…"));
        assert!(
            !labels_of(&build(&row(1, "Titre"), &MenuContext::own_list()))
                .contains(&"Remplacer le titre…")
        );
    }

    /// The four entries another lot owns are gated on their flag, and the
    /// dividers around an empty group collapse: no double hairline, no entry that
    /// does nothing.
    #[test]
    fn a_feature_that_is_not_on_the_device_leaves_no_button_behind() {
        let mut ctx = MenuContext::own_list();
        ctx.admin = true;
        // Nothing from MB15/MB18/MB13 on this build.
        let plain = build(&row(1, "Titre"), &ctx);
        for absent in [
            "Télécharger",
            "Télécharger en…",
            "Retirer le téléchargement",
            "Étiqueter le genre…",
        ] {
            assert!(!labels_of(&plain).contains(&absent), "{absent}");
        }
        assert_eq!(dividers(&plain), 2);

        // The credits sheet belongs to MB7; a shell that has not installed it
        // hides the entry rather than opening nothing.
        let mut no_credits = MenuContext::own_list();
        no_credits.credits = false;
        let without = build(&row(1, "Titre"), &no_credits);
        assert!(!labels_of(&without).contains(&"Voir les crédits"));
        assert!(labels_of(&plain).contains(&"Voir les crédits"));

        // With the feature on, the SPA's full list comes back.
        ctx.offline = true;
        ctx.genre_tag = true;
        let full = build(&row(1, "Titre"), &ctx);
        for present in ["Télécharger", "Télécharger en…", "Étiqueter le genre…"] {
            assert!(labels_of(&full).contains(&present), "{present}");
        }
        assert_eq!(
            dividers(&full),
            3,
            "the SPA's three hairlines, and never two in a row"
        );
    }

    /// The download row flips, and its submenu carries the six qualities the SPA
    /// offers, in its order.
    #[test]
    fn the_download_entry_flips_and_its_submenu_lists_the_six_qualities() {
        let mut ctx = MenuContext::own_list();
        ctx.offline = true;
        let entries = build(&row(1, "Titre"), &ctx);
        assert!(labels_of(&entries).contains(&"Télécharger"));
        let qualities: Vec<&str> = entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) if item.label == "Télécharger en…" => Some(
                    item.sub
                        .iter()
                        .map(|sub| sub.label.as_str())
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(
            qualities,
            [
                "FLAC", "Opus 320", "Opus 256", "Opus 192", "Opus 128", "Opus 64"
            ]
        );

        ctx.downloaded = true;
        let downloaded = build(&row(1, "Titre"), &ctx);
        assert!(labels_of(&downloaded).contains(&"Retirer le téléchargement"));
        assert!(!labels_of(&downloaded).contains(&"Télécharger"));
        assert!(
            !labels_of(&downloaded).contains(&"Télécharger en…"),
            "a title already here has no second quality to pick"
        );
    }

    /// The favourite and the dislike flip their label, their glyph and their verb
    /// together — one source of truth for all three.
    #[test]
    fn the_heart_and_the_thumb_read_the_rows_own_flags() {
        let mut liked = row(1, "Titre");
        liked.liked = true;
        let entries = build(&liked, &MenuContext::own_list());
        assert_eq!(
            action_of(&entries, "Retirer des favoris"),
            Some(&MenuAction::Favorite { on: false })
        );
        let glyph = entries.iter().find_map(|entry| match entry {
            MenuEntry::Item(item) if item.label == "Retirer des favoris" => Some(item.icon),
            _ => None,
        });
        assert_eq!(glyph, Some("heartFilled"));

        let mut disliked = row(1, "Titre");
        disliked.disliked = true;
        let entries = build(&disliked, &MenuContext::own_list());
        assert_eq!(
            action_of(&entries, "Retirer de « Je n'aime pas »"),
            Some(&MenuAction::Dislike { on: false })
        );
    }

    /// The one-tap playlist shortcut only shows for a list that may be written to.
    #[test]
    fn the_last_playlist_shortcut_needs_a_writable_target() {
        let mut ctx = MenuContext::own_list();
        ctx.last_playlist = Some(LastPlaylist {
            id: "p-1".to_owned(),
            title: "Route du soir".to_owned(),
            editable: true,
        });
        let entries = build(&row(1, "Titre"), &ctx);
        assert_eq!(
            action_of(&entries, "Ajouter à « Route du soir »"),
            Some(&MenuAction::AddToLastPlaylist {
                playlist_id: "p-1".to_owned(),
                title: "Route du soir".to_owned(),
            })
        );

        ctx.last_playlist = Some(LastPlaylist {
            id: "p-2".to_owned(),
            title: "Celle de quelqu'un".to_owned(),
            editable: false,
        });
        let read_only = labels_of(&build(&row(1, "Titre"), &ctx)).join("|");
        assert!(!read_only.contains("Celle de quelqu'un"));
    }

    /// A feat. track opens the artists as a submenu; a single credit stays plain.
    #[test]
    fn several_credited_artists_open_a_submenu() {
        let mut ctx = MenuContext::own_list();
        ctx.artists = vec![
            MenuPerson {
                id: "a-1".to_owned(),
                name: "Ascenseur".to_owned(),
            },
            MenuPerson {
                id: "a-2".to_owned(),
                name: "June".to_owned(),
            },
        ];
        let entries = build(&row(1, "Titre"), &ctx);
        let item = entries.iter().find_map(|entry| match entry {
            MenuEntry::Item(item) if item.label == "Aller à l'artiste" => Some(item),
            _ => None,
        });
        let item = item.expect("the artist entry");
        assert_eq!(item.sub.len(), 2);
        assert_eq!(item.sub[1].label, "June");
        assert_eq!(
            item.sub[1].action,
            MenuAction::GoToArtist {
                id: "a-2".to_owned()
            }
        );
    }

    /// A row that cannot name an artist page does not offer one — and a YouTube
    /// row (no album row, no credits) differs exactly as the SPA's shape makes it
    /// differ.
    #[test]
    fn a_row_that_cannot_name_a_page_offers_no_link() {
        let mut youtube = row(7, "Reprise");
        youtube.source = "youtube".to_owned();
        youtube.album_id = None;
        youtube.album = String::new();
        let built = build(&youtube, &MenuContext::own_list());
        let labels = labels_of(&built).join("|");
        assert!(!labels.contains("Aller à l'artiste"));
        assert!(!labels.contains("Aller à l'album"));
        // Everything a YouTube hit CAN do is still there.
        assert!(labels.contains("Lancer la radio"));
        assert!(labels.contains("Voir les crédits"));
    }

    /// An upload keeps its album entry (an upload can have one) and gains nothing
    /// the SPA does not give it: its menu is the track menu.
    #[test]
    fn an_upload_takes_the_ordinary_title_menu() {
        let mut upload = row(9, "Ma prise");
        upload.source = "upload".to_owned();
        upload.album_id = Some("bbbbbbbb-1111-4111-8111-111111111111".to_owned());
        let entries = build(&upload, &MenuContext::own_list());
        let labels = labels_of(&entries);
        assert!(labels.contains(&"Aller à l'album"));
        assert!(labels.contains(&"Ajouter aux favoris"));
        assert!(!labels.contains(&"Ouvrir le podcast"));
    }

    /// An episode that is playing, on a device with markers, offers the one-tap
    /// marker; without the marker feature it offers nothing that would not work.
    #[test]
    fn the_marker_entry_belongs_to_a_playing_episode_and_its_own_feature() {
        let mut ctx = MenuContext::own_list();
        ctx.episode = true;
        ctx.episode_playing = true;
        let without = labels_of(&build(&row(1, "Épisode"), &ctx)).join("|");
        assert!(!without.contains("Marquer cette position"));

        ctx.markers = true;
        let entries = build(&row(1, "Épisode"), &ctx);
        assert_eq!(
            entries.iter().find_map(|entry| match entry {
                MenuEntry::Item(item) if item.label == "Marquer cette position" => {
                    Some(item.action.clone())
                }
                _ => None,
            }),
            Some(MenuAction::MarkPosition)
        );
    }

    /// A queue episode removes, and the SPA's episode menu still offers nothing
    /// to queue twice.
    #[test]
    fn the_queue_episode_menu_is_the_episode_menu_plus_removal() {
        let mut ctx = MenuContext::queue(4, Some(0));
        ctx.episode = true;
        let entries = build(&row(4, "Épisode"), &ctx);
        let labels = labels_of(&entries);
        assert_eq!(labels.first(), Some(&"Retirer de la file"));
        assert!(!labels.contains(&"Lire ensuite"));
    }

    /// Opening a menu costs nothing: `build` takes no core, no client, and no
    /// lock — this test's proof is that it is a pure function of its arguments,
    /// and that the same row builds the same menu twice.
    #[test]
    fn building_a_menu_is_pure() {
        let ctx = MenuContext::own_list();
        let row = row(1, "Titre");
        assert_eq!(build(&row, &ctx), build(&row, &ctx));
    }
}
