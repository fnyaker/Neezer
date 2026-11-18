//! Android Auto's browse tree: the five nodes a head unit shows, and what a tap
//! on one of them plays.
//!
//! Owned by **MB25** (docs/agents/mobile.md, "MB25 — Android Auto"): the tree
//! lives here, `nz-ffi/src/auto.rs` is the thin surface over it, and the
//! Kotlin side (`playback/.../auto/**`) turns these items into Media3
//! `MediaItem`s.
//!
//! # The five nodes, and what each costs
//!
//! | Media id | The node | Browse | Play |
//! |---|---|---|---|
//! | `neezer:root` | the root | 0 requests (static) | — |
//! | `neezer:favorites` | Favoris | 1 cold, 0 warm (`/me/favorites`) | queues the browsed titles (0) |
//! | `neezer:playlists` | Playlists | 1 cold, 0 warm (`/me/playlists`) | — (a folder) |
//! | `neezer:playlist:<id>` | one playlist | 1 cold, 0 warm (`/playlist/<id>`) | queues the browsed titles (0) |
//! | `neezer:flow` | Mon Flow | — (a playable leaf) | the PLAYER's own `PlayEntity("flow")`, one title at a time |
//! | `neezer:recent` | Récents | 1 cold, 0 warm (`/history?limit=30`) | queues the browsed titles (0) |
//! | `neezer:downloads` | Téléchargements | 0 (the device's own store) | queues the browsed titles (0) |
//!
//! "Warm" is the copy this module keeps in the cache directory ([`store`]):
//! a browse reads it with **no request at all** while it is younger than
//! [`FRESH`], and re-asks once after that. That is MB25's acceptance ("≤ 1
//! request per node, 0 warm") and rule 3's "what was fetched once is kept".
//!
//! # Lanes (rule 1)
//!
//! - **A browse is `Prefetch`**: nobody on the phone is doing it, the car is
//!   waiting, and the phone's own screen always comes first. The section says
//!   "never `User`" and this module has no `User` browse at all.
//! - **A tap is `User`**: [`Core::auto_play`] on a node the device has never
//!   seen is the action the person just took, so it asks on [`Lane::User`] —
//!   the only lane a finger may use.
//!
//! # Why the tree is not a `ListHandle`
//!
//! A car list is browsed a dozen rows at a time and the note caps a node at
//! [`MAX_CHILDREN`] children: an 8 000-title favourites list crosses as its
//! first hundred rows, in the server's own order, and the phone's own screens
//! keep the windowed `ListHandle` (MB8). A car that scrolled 8 000 rows would
//! spend the person's link to show rows nobody reaches.
//!
//! # Ordering of the playlists
//!
//! The person's own playlists first (`role == "owner"`), then the ones shared
//! with them, each in the server's order (folder position, then title). "Pinned
//! first" is MB24's car overlay that owns the pins (`webapp/src/lib/car.js`):
//! until that lot's pin store exists there is nothing to sort by, and inventing
//! a key it does not write would be a contract the two lots would have to break
//! (declared in the PR, and in this note's MB25 section).

mod store;
mod wire;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use nz_api::Lane;
use nz_model::{Playlist, Track};

use crate::core::{Core, CoreError};
use crate::lists::Row;
use crate::offline::{KEY_SET_PREFIX, KEY_SETS, QueuedSet};

/// The tag every request of this area is counted under (`NzCore::stats`).
pub const TAG_AUTO: &str = "auto";

/// How many children a node answers (`docs/agents/mobile.md`, MB25: ≤ 100).
pub const MAX_CHILDREN: usize = 100;

/// How many listens « Récents » shows — the SPA keeps the same thirty
/// (`lib/stores.js#pushRecent`: `next.slice(0, 30)`).
pub const RECENT_LIMIT: u32 = 30;

/// How long a node's copy is painted without asking again. Fifteen minutes: a
/// car trip browses the same few nodes, and a stale list that changes under the
/// driver is worse than one that learns on the next stop.
pub const FRESH: Duration = Duration::from_secs(15 * 60);

/// The root's media id — the one a controller connects to.
pub const ROOT: &str = "neezer:root";
pub const FAVORITES: &str = "neezer:favorites";
pub const PLAYLISTS: &str = "neezer:playlists";
pub const FLOW: &str = "neezer:flow";
pub const RECENT: &str = "neezer:recent";
pub const DOWNLOADS: &str = "neezer:downloads";
pub const PLAYLIST_PREFIX: &str = "neezer:playlist:";
pub const TRACK_PREFIX: &str = "neezer:track:";

/// The media id of one playlist, or of one title.
pub fn playlist_media_id(id: &str) -> String {
    format!("{PLAYLIST_PREFIX}{id}")
}

pub fn track_media_id(key: &str) -> String {
    format!("{TRACK_PREFIX}{key}")
}

/// The five sections, in the order a car draws them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    Root,
    Favorites,
    Playlists,
    Flow,
    Recent,
    Downloads,
    /// One playlist, by the server's id.
    Playlist,
    /// One title: a leaf that is only ever played, never browsed.
    Track,
}

/// What a media id names, and the id it carries (`""` for a section).
///
/// Nothing else is accepted: an id the tree did not build is a `None`, so a
/// hostile string can never choose a route, a file name or a query value.
pub fn parse(media_id: &str) -> Option<(Request, &str)> {
    let id = media_id.trim();
    match id {
        ROOT => Some((Request::Root, "")),
        FAVORITES => Some((Request::Favorites, "")),
        PLAYLISTS => Some((Request::Playlists, "")),
        FLOW => Some((Request::Flow, "")),
        RECENT => Some((Request::Recent, "")),
        DOWNLOADS => Some((Request::Downloads, "")),
        _ => {
            if let Some(argument) = id.strip_prefix(PLAYLIST_PREFIX) {
                // A playlist id is a UUID: letters, digits, dashes. A `/`, a `?`
                // or a `#` would choose another route.
                return valid(argument).then_some((Request::Playlist, argument));
            }
            let argument = id.strip_prefix(TRACK_PREFIX)?;
            // A row key may carry a source prefix (`deezer:123`), never a route
            // character.
            valid(argument).then_some((Request::Track, argument))
        }
    }
}

/// `true` for an id made of the characters every route argument here uses.
fn valid(argument: &str) -> bool {
    !argument.is_empty()
        && argument.len() <= 160
        && argument
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

/// What one row of the car's tree is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    /// A fixed node the tree always shows (Favoris, Playlists, Mon Flow, …).
    Section,
    /// One of the person's playlists.
    Playlist,
    /// One title, played by the same player as everywhere else.
    Track,
}

/// One row of the tree, shaped the way the car draws it — plus what the queue
/// needs when a title is tapped (`id`, `album_id`, `duration_s`, …), because a
/// Media3 `MediaItem` that crosses a Binder must carry everything the player
/// needs to rebuild the queue (MB3c's `NzPlayer.trackOf`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Item {
    /// `neezer:<node>` or `neezer:track:<key>`: stable across restarts, which
    /// is what lets a head unit remember where the person was.
    pub media_id: String,
    pub title: String,
    /// The line under the title: the artist of a title, the count of a list.
    pub subtitle: String,
    pub kind: Kind,
    pub browsable: bool,
    pub playable: bool,
    /// A URL or the server's relative cover path; `None` draws the grooves.
    pub cover: Option<String>,
    /// The id `/api/stream` takes, for a title.
    pub id: String,
    pub artists: String,
    pub album: String,
    pub album_id: Option<String>,
    pub duration_s: u32,
    pub liked: bool,
    pub explicit: bool,
    /// How many titles a list holds (`0` for a title).
    pub track_count: u32,
}

/// What a tap on a node must do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Play {
    /// These titles ARE the queue: the ones the car just showed, so the tap
    /// costs no request at all.
    Queue(Vec<Item>),
    /// The PLAYER resolves it (`PlayEntity`): Mon Flow is asked one title at a
    /// time by the queue's own refill (`nz-core/src/player/flow.rs`), which is
    /// the behaviour a Flow has everywhere else in the app.
    Entity { kind: String, id: String },
}

impl Item {
    /// A fixed node of the root.
    fn section(media_id: &str, title: &str, subtitle: &str) -> Item {
        Item {
            media_id: media_id.to_owned(),
            title: title.to_owned(),
            subtitle: subtitle.to_owned(),
            kind: Kind::Section,
            browsable: true,
            playable: false,
            cover: None,
            id: String::new(),
            artists: String::new(),
            album: String::new(),
            album_id: None,
            duration_s: 0,
            liked: false,
            explicit: false,
            track_count: 0,
        }
    }

    /// Mon Flow: a leaf that plays, and a list that holds one or two titles
    /// would be a lie (the queue asks for each next one).
    fn flow() -> Item {
        Item {
            browsable: false,
            playable: true,
            ..Item::section(FLOW, "Mon Flow", "Un flux qui suit mes écoutes")
        }
    }

    /// One playlist of the person's library.
    fn playlist(card: &Playlist) -> Item {
        Item {
            media_id: playlist_media_id(&card.id.to_string()),
            title: card.title.clone(),
            subtitle: count_label(card.nb_tracks),
            kind: Kind::Playlist,
            browsable: true,
            playable: true,
            cover: card.cover.clone(),
            id: String::new(),
            artists: String::new(),
            album: String::new(),
            album_id: None,
            duration_s: 0,
            liked: false,
            explicit: false,
            track_count: card.nb_tracks,
        }
    }

    /// One title, from any list (favourites, a playlist, the history).
    fn from_track(track: &Track, liked: bool) -> Item {
        let (key, id) = track_key(track);
        Item {
            media_id: track_media_id(&key),
            title: track.title.clone(),
            subtitle: artist_line(track),
            kind: Kind::Track,
            browsable: false,
            playable: true,
            cover: track.album.cover.clone(),
            id,
            artists: artist_line(track),
            album: track.album.title.clone(),
            album_id: (!track.album.deezer_id.to_string().is_empty())
                .then(|| track.album.deezer_id.to_string()),
            duration_s: track.duration,
            liked,
            explicit: track.explicit,
            track_count: 0,
        }
    }

    /// One title already decoded into a row (the device's own downloads).
    fn from_row(row: &Row) -> Item {
        Item {
            media_id: track_media_id(&row.key),
            title: row.title.clone(),
            subtitle: row.artists.clone(),
            kind: Kind::Track,
            browsable: false,
            playable: true,
            cover: row.cover.clone(),
            id: row.id.clone(),
            artists: row.artists.clone(),
            album: row.album.clone(),
            album_id: row.album_id.clone(),
            duration_s: row.duration_s,
            liked: row.liked,
            explicit: row.explicit,
            track_count: 0,
        }
    }
}

/// « 12 titres » — the line under a list's name, in French and never a bare
/// number (`1 titre` / `0 titre` / `12 titres`).
fn count_label(count: u32) -> String {
    if count <= 1 {
        format!("{count} titre")
    } else {
        format!("{count} titres")
    }
}

/// The row key and the request id of a title, restated from MB8's
/// `library/wire.rs#track_key` so a title has ONE identity in the whole app: its
/// row's UUID when the server sent one, else the provider's id.
fn track_key(track: &Track) -> (String, String) {
    let provider = track.deezer_id.to_string();
    let row = track.id.as_ref().map(|id| id.to_string());
    let key = row.clone().unwrap_or_else(|| provider.clone());
    (key, row.unwrap_or(provider))
}

/// The one artist line a list shows, already joined (`lib/format.js`'s
/// `artistLine`: every credited name, in the server's order).
fn artist_line(track: &Track) -> String {
    if let Some(line) = track.display_artist.clone().filter(|line| !line.is_empty()) {
        return line;
    }
    let names: Vec<&str> = track
        .artists
        .iter()
        .map(|role| role.name.as_str())
        .filter(|name| !name.is_empty())
        .collect();
    if names.is_empty() {
        track.artist.name.clone()
    } else {
        names.join(", ")
    }
}

/// The root's five sections. Static by construction: opening the tree costs no
/// request at all, which is what makes the car's first screen instant.
fn root_items() -> Vec<Item> {
    vec![
        collection(FAVORITES, "Favoris", "Les titres que j'aime"),
        Item::section(PLAYLISTS, "Playlists", "Mes listes"),
        Item::flow(),
        collection(RECENT, "Récents", "Les derniers titres écoutés"),
        collection(DOWNLOADS, "Téléchargements", "Sur cet appareil"),
    ]
}

/// A section the car can PLAY as well as browse: its « tout lire » button is
/// the same tap as a playlist's, and the queue is what the node showed.
fn collection(media_id: &str, title: &str, subtitle: &str) -> Item {
    Item {
        playable: true,
        ..Item::section(media_id, title, subtitle)
    }
}

impl Core {
    /// The node's children from the device's own copy: **no request, ever**.
    ///
    /// This is what the session thread paints (Media3 asks for a list
    /// synchronously through MB3c's `LibraryTree`), so it must be disk-only: a
    /// copy of at most [`MAX_CHILDREN`] rows, parsed in one gulp.
    pub fn auto_cached_children(&self, parent: &str) -> Vec<Item> {
        let Some((request, _)) = parse(parent) else {
            return Vec::new();
        };
        match request {
            Request::Root => root_items(),
            Request::Downloads => self.auto_downloads(),
            Request::Flow | Request::Track => Vec::new(),
            _ => store::read(self, &self.auto_owner(), parent).unwrap_or_default(),
        }
    }

    /// The node's children: the copy while it is younger than [`FRESH`], else
    /// ONE read on the `Prefetch` lane — never `User` (rule 1: the phone's
    /// screen comes first, and nobody there asked for this).
    pub async fn auto_children(&self, parent: &str) -> Result<Vec<Item>, CoreError> {
        let Some((request, argument)) = parse(parent) else {
            return Err(CoreError::Invalid {
                detail: "Nœud inconnu".to_owned(),
            });
        };
        match request {
            Request::Root => Ok(root_items()),
            Request::Downloads => Ok(self.auto_downloads()),
            // A Flow has no list of its own (the player asks each title), and a
            // title is a leaf.
            Request::Flow | Request::Track => Ok(Vec::new()),
            _ => {
                if let Some(items) = store::read_if_fresh(self, &self.auto_owner(), parent, FRESH) {
                    return Ok(items);
                }
                let items = self.auto_fetch(request, argument, Lane::Prefetch).await?;
                store::write(self, &self.auto_owner(), parent, &items)?;
                Ok(items)
            }
        }
    }

    /// What a tap on the car's play button must play.
    ///
    /// A node the car just browsed costs **no request**: the titles it showed
    /// ARE the queue. A node the device has never seen is the action the person
    /// just took, so it asks on [`Lane::User`] — the browsing lane stays
    /// `Prefetch` ([`Core::auto_children`]).
    pub async fn auto_play(&self, media_id: &str) -> Result<Play, CoreError> {
        let Some((request, argument)) = parse(media_id) else {
            return Err(CoreError::Invalid {
                detail: "Nœud inconnu".to_owned(),
            });
        };
        match request {
            // The queue asks for the Flow's next title itself, one request per
            // title, and keeps the level the phone set.
            Request::Flow => Ok(Play::Entity {
                kind: "flow".to_owned(),
                id: argument.to_owned(),
            }),
            Request::Favorites | Request::Recent | Request::Playlist | Request::Downloads => {
                let items = match self.auto_cached_children(media_id) {
                    items if !items.is_empty() => items,
                    _ if request == Request::Downloads => self.auto_downloads(),
                    _ => {
                        let items = self.auto_fetch(request, argument, Lane::User).await?;
                        store::write(self, &self.auto_owner(), media_id, &items)?;
                        items
                    }
                };
                if items.is_empty() {
                    return Err(CoreError::Invalid {
                        detail: "Cette liste est vide".to_owned(),
                    });
                }
                Ok(Play::Queue(items))
            }
            Request::Root | Request::Playlists => Err(CoreError::Invalid {
                detail: "Ce nœud ne se joue pas".to_owned(),
            }),
            Request::Track => Err(CoreError::Invalid {
                detail: "Un titre se joue depuis sa file".to_owned(),
            }),
        }
    }

    /// One request for one node, on `lane`, counted under [`TAG_AUTO`].
    async fn auto_fetch(
        &self,
        request: Request,
        argument: &str,
        lane: Lane,
    ) -> Result<Vec<Item>, CoreError> {
        let api = Arc::new(self.api()?.with_tag(TAG_AUTO));
        let before = wire::totals(&api);
        let answer = wire::fetch(&api, request, argument, lane).await;
        self.note_traffic(&api, before);
        let mut items = wire::items(&answer?, request)?;
        items.truncate(MAX_CHILDREN);
        Ok(items)
    }

    /// Carries one read's requests and bytes to the core's meter, so
    /// [`Core::stats`] — what the PR and the acceptance quote — names the `auto`
    /// tag instead of leaving the tree's cost invisible (rule 3, "counted, not
    /// timed").
    fn note_traffic(&self, api: &nz_api::Api, before: (u64, u64, u64)) {
        let after = wire::totals(api);
        let requests = after.0.saturating_sub(before.0);
        if requests == 0 {
            return;
        }
        // `Meter::record` counts ONE request and its bytes; the whole read's
        // bytes go with the first, the rest are zeros.
        self.meter().record(
            TAG_AUTO,
            after.1.saturating_sub(before.1),
            after.2.saturating_sub(before.2),
        );
        for _ in 1..requests {
            self.meter().record(TAG_AUTO, 0, 0);
        }
    }

    /// What the device holds, newest first — the store's own copy, no request
    /// and no worker: a car may be the first surface of a process
    /// (`:playback` starts before any screen), so nothing here may wait for the
    /// offline service to exist.
    fn auto_downloads(&self) -> Vec<Item> {
        let owner = self.auto_owner();
        let published = match self.cache().downloads().published(&owner) {
            Ok(ids) => ids,
            Err(cause) => {
                tracing::debug!(%cause, "les téléchargements n'ont pas pu être lus");
                return Vec::new();
            }
        };
        if published.is_empty() {
            return Vec::new();
        }
        // The labels, ONE pass over the sets MB15 keeps (`offline.set.<ctx>`):
        // a row's kv read would be a statement per title on a browse.
        let labels = download_labels(self);
        let mut rows: Vec<Row> = published
            .iter()
            .enumerate()
            .map(|(position, key)| {
                let label = labels.get(key);
                Row {
                    key: key.clone(),
                    id: key.clone(),
                    title: label
                        .as_ref()
                        .map(|label| label.title.clone())
                        .filter(|title| !title.is_empty())
                        .unwrap_or_else(|| key.clone()),
                    artists: label
                        .as_ref()
                        .map(|label| label.artist.clone())
                        .unwrap_or_default(),
                    album: String::new(),
                    album_id: None,
                    duration_s: label.as_ref().map(|label| label.duration_s).unwrap_or(0),
                    cover: label.as_ref().and_then(|label| label.cover.clone()),
                    explicit: label.as_ref().map(|label| label.explicit).unwrap_or(false),
                    liked: false,
                    disliked: false,
                    unavailable: false,
                    downloaded: true,
                    source: "deezer".to_owned(),
                    position: position as u32,
                    played_at: 0,
                    added_at: label
                        .as_ref()
                        .map(|label| label.added_at.max(0) as u64)
                        .unwrap_or(0),
                }
            })
            .collect();
        // Newest first, the SPA's own order (`lib/offline.js#listDownloads`),
        // then by title so two downloads of the same second never swap places
        // between two browses.
        rows.sort_by(|a, b| {
            b.added_at
                .cmp(&a.added_at)
                .then_with(|| a.title.cmp(&b.title))
        });
        rows.truncate(MAX_CHILDREN);
        rows.iter().map(Item::from_row).collect()
    }

    /// The account the device's copies belong to (MB8's rule, restated: a
    /// logout and a second person never see the first one's lists).
    fn auto_owner(&self) -> String {
        self.me()
            .map(|me| me.user.id.as_str().to_owned())
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| "anon".to_owned())
    }
}

/// The labels MB15's offline service keeps beside the downloads (`offline.set.`
/// keys, one blob per set), so a downloaded title shows its name in a car that
/// started the process. Read straight from the store: no service, no request.
fn download_labels(core: &Core) -> std::collections::HashMap<String, crate::offline::TrackLabel> {
    let contexts: Vec<String> = core.cache().kv().get(KEY_SETS).unwrap_or_default();
    let mut labels = std::collections::HashMap::new();
    for context in contexts {
        let store_key = format!("{KEY_SET_PREFIX}{context}");
        let Some(queued) = core.cache().kv().get::<QueuedSet>(&store_key) else {
            continue;
        };
        for label in queued.tracks {
            // The first set that named a title wins: every set carries the same
            // label for it (one `/api/stream` id, one row).
            labels.entry(label.id.clone()).or_insert(label);
        }
    }
    labels
}
