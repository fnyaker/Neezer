//! The ONE track-actions spec every list shares: like/dislike, play next, add to
//! queue, radio, add to playlist, credits, replace, share.
//!
//! Owned by **MB10** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this directory and edits no other file of the crate.
//!
//! # What this area is
//!
//! `webapp/src/lib/trackmenu.js` decides WHICH entries a title's menu has, and
//! `lib/actions.js`, `lib/replace.js`, `components/PlaylistPicker.svelte` and
//! `components/ReplaceSheet.svelte` decide what each one DOES. This area is both
//! halves, ported:
//!
//! | This area | The SPA |
//! |---|---|
//! | [`menu`] — the builder, as data | `lib/trackmenu.js` |
//! | [`Core::set_favorite`] | `lib/actions.js#toggleFavorite` |
//! | [`Core::set_dislike`] | `lib/actions.js#toggleDislike` |
//! | [`Core::menu_playlists`], [`Core::add_to_playlist`], [`Core::create_playlist`] | `PlaylistPicker.svelte` + `lib/actions.js` |
//! | [`Core::replace_candidates`], [`Core::replace`], [`Core::replace_status`] | `ReplaceSheet.svelte` + `lib/replace.js` |
//! | [`Core::delete_uploads`], [`Core::delete_tracks`] | `DELETE /api/local/<id>`, `UnavailableList.svelte#deleteSelected` |
//! | [`Core::share_link`] | the share sheet's link, `playlist_share.py` |
//!
//! Everything that PLAYS (`playNext`, `addToQueue`, `remove`) or LEADS somewhere
//! (`credits`, `artist`, `album`, `podcast`) is a player command or a route, not a
//! request: the menu hands the verb to the Kotlin side, which uses the player the
//! app already has and the shell's `NzNavigator`.
//!
//! # Writes are optimistic, and every list sees it in the same frame
//!
//! A like patches every open [`crate::lists::List`] row carrying the key
//! (`Registry::update_rows`) BEFORE the request is sent, and patches it back when
//! the server refuses — the desktop's rule, and the reason a heart never waits for
//! the link. The patch bumps each list's `version`, so a screen that windows a
//! handle re-reads the row it is drawing; MB10's own listener
//! (`nz-ffi`'s `subscribe_actions`) tells every surface when it happened.
//!
//! # Counted, not timed
//!
//! Every request of this area is booked under [`TAG_ACTIONS`], so
//! `NzCore::stats()` answers what the menu cost: **0 requests to open it** (it is
//! a pure function), 1 for a favourite, 1 for the picker's list, 1 to add, 1 to
//! look for a replacement.

mod menu;
mod store;
mod wire;

#[cfg(test)]
mod testing;

use std::sync::atomic::{AtomicU64, Ordering};

use nz_api::Api;

pub use menu::{
    DL_QUALITIES, LastPlaylist, MenuAction, MenuContext, MenuEntry, MenuItem, MenuPerson, Surface,
    build,
};

use crate::core::{Core, CoreError};
use crate::lists::Row;

/// The tag every request of this area is counted under (`NzCore::stats`).
pub const TAG_ACTIONS: &str = "actions";

/// A playlist the picker offers: only the fields its row draws.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaylistChoice {
    pub id: String,
    pub title: String,
    pub cover: Option<String>,
    pub nb_tracks: u32,
    /// The person may write to it: a viewer's list is not a target (the SPA's own
    /// filter, `editableList`).
    pub editable: bool,
    /// `owner`, `editor`, `viewer` — a shared list says whose it is.
    pub role: Option<String>,
    pub owner: Option<String>,
    /// The last playlist used, so its row can carry the « récente » badge.
    pub recent: bool,
    /// The playlist's content counter: what a `409` would carry, and what the
    /// playlist page windows by. Kept so the add can send it when it has one.
    pub version: u32,
}

/// One stand-in the replacement sheet offers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplaceCandidate {
    /// The row key a like or a patch would use (`<source>:<id>`).
    pub key: String,
    /// What `/api/replace` names it by (a row UUID, or the provider id of a
    /// catalogue result it will import).
    pub id: String,
    pub title: String,
    pub artists: String,
    pub album: String,
    pub album_id: Option<String>,
    pub duration_s: u32,
    pub cover: Option<String>,
    /// `deezer`, `local`, `youtube` — the sheet's source mark.
    pub source: String,
    /// A file already on this server: it can never go away again.
    pub local: bool,
}

/// `GET /api/replace/candidates/<id>`: the matches, and where the fallback
/// answered on its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplaceAnswer {
    /// The query the server actually searched (the sheet's box is pre-filled with
    /// it).
    pub query: String,
    pub candidates: Vec<ReplaceCandidate>,
    /// The fallback source that answered unprompted (`fallback_auto`), when the
    /// main source found nothing.
    pub fallback_auto: Option<String>,
}

/// One title the server refused to delete, and why — never a bare count (rule 2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Kept {
    pub id: String,
    /// `archived`, `playable`, `error` — the server's own reason key.
    pub reason: String,
}

/// A job's answer: what a replacement or a bulk deletion did.
///
/// The rewrite runs on a server worker, so the first answer carries the handle and
/// nothing else; the screen polls [`Core::replace_status`] /
/// [`Core::delete_status`] until `running` is false, then reads `deleted`/`kept`
/// and the causes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MutationReport {
    pub job: Option<String>,
    pub running: bool,
    pub ok: bool,
    /// How many playlists the rewrite touched.
    pub playlists: u32,
    pub favorites: u32,
    /// The titles removed (a bulk deletion), in the server's order.
    pub deleted: Vec<String>,
    /// The titles kept, each with its cause.
    pub kept: Vec<Kept>,
    /// The titles the server never had.
    pub missing: Vec<String>,
    /// How far the job is: `done` of `total`.
    pub done: u32,
    pub total: u32,
    /// The failure's cause, when the job failed as a whole.
    pub error: Option<String>,
}

impl MutationReport {
    /// The handle a mutation answered, before its first status read.
    fn started(job: String) -> MutationReport {
        MutationReport {
            job: Some(job),
            running: true,
            ..MutationReport::default()
        }
    }
}

/// Who is asking, for the menu's own flags: the account's rights and the last
/// playlist used. A platform-shaped record with no FFI type (the FFI converts).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Actor {
    /// The account administers this server (`isAdmin` in the SPA).
    pub admin: bool,
    /// The last playlist a title was added to.
    pub last_playlist: Option<LastPlaylist>,
}

/// What the app's own lists are told when a menu entry changed their rows: the
/// keys, and the counter a screen compares to re-window.
pub trait ActionSink: Send + Sync {
    fn on_rows_changed(&self, keys: Vec<String>, version: u64);
}

/// The count of every `.part`-safe write this area has done — the version a
/// screen re-windows on when its rows changed without a re-fetch.
static ROWS_VERSION: AtomicU64 = AtomicU64::new(0);

/// The counter the listener pushes: it only ever grows, so a screen can compare
/// two readings.
pub fn rows_version() -> u64 {
    ROWS_VERSION.load(Ordering::SeqCst)
}

/// The subscribed surfaces, in a module-level list rather than on `Core`: this
/// lot owns its own files and `core/mod.rs` is MB2's. The core is ONE per process
/// (`NzCore`), so the lifetime is the same.
static ACTION_SINKS: std::sync::Mutex<Vec<std::sync::Arc<dyn ActionSink>>> =
    std::sync::Mutex::new(Vec::new());

/// Subscribes a surface to the "these rows changed" signal, replacing a previous
/// subscription from the same process (the app has ONE shell).
pub fn subscribe(sink: std::sync::Arc<dyn ActionSink>) {
    let mut sinks = ACTION_SINKS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    sinks.clear();
    sinks.push(sink);
}

fn action_sinks() -> Vec<std::sync::Arc<dyn ActionSink>> {
    ACTION_SINKS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

/// The request reference a row key names: `<source>:<id>` → the id the server
/// resolves (`trackkey.js#trackRef` — the row's UUID when it has one, else the
/// provider's id).
pub fn reference_of(key: &str) -> &str {
    match key.split_once(':') {
        Some((_, id)) if !id.is_empty() => id,
        _ => key,
    }
}

/// The running counters of one tag, so a call can book what it spent.
#[derive(Debug, Clone, Copy, Default)]
struct Totals {
    requests: u64,
    bytes_in: u64,
    bytes_out: u64,
}

fn totals(api: &Api) -> Totals {
    api.stats()
        .into_iter()
        .find(|row| row.tag == TAG_ACTIONS)
        .map(|row| Totals {
            requests: row.requests,
            bytes_in: row.bytes_in,
            bytes_out: row.bytes_out,
        })
        .unwrap_or_default()
}

impl Core {
    /// The menu of `row`, exactly `lib/trackmenu.js#buildTrackMenu` (or the
    /// episode builder). **Pure and free**: no request, no lock, no disk.
    pub fn track_menu(&self, row: &Row, ctx: &MenuContext) -> Vec<MenuEntry> {
        menu::build(row, ctx)
    }

    /// The last playlist a title was added to, for the one-tap shortcut.
    pub fn last_playlist(&self) -> Option<LastPlaylist> {
        store::read_last_playlist(self)
    }

    /// Who is asking: the admin flag from the restored session, and the last
    /// playlist used. **No request** — `/api/me` was read at `restore`.
    pub fn actor(&self) -> Actor {
        Actor {
            admin: self.me().is_some_and(|me| me.user.admin),
            last_playlist: self.last_playlist(),
        }
    }

    /// The playlist list this device last saw — disk only, **0 requests**. `None`
    /// the first time, which is the one moment the picker shows a skeleton.
    pub fn cached_menu_playlists(&self) -> Option<Vec<PlaylistChoice>> {
        let body = store::read_playlists(self)?;
        let list: nz_model::PlaylistList = serde_json::from_str(&body).ok()?;
        Some(self.choices(list.playlists))
    }

    /// `GET /api/me/playlists` — ONE request, kept for the next opening and for a
    /// bad link.
    pub async fn menu_playlists(&self) -> Result<Vec<PlaylistChoice>, CoreError> {
        let body = self
            .spend(|api| async move { wire::fetch_text(&api, "/me/playlists").await })
            .await?;
        // The copy is written before the parse: a body this build cannot read is
        // still what the next one may read, and a failure to write it never costs
        // the opening (`cached_menu_playlists` simply asks again).
        if let Err(cause) = store::write_playlists(self, &body) {
            tracing::debug!(%cause, "the picker's playlist copy was not kept");
        }
        let list: nz_model::PlaylistList =
            serde_json::from_str(&body).map_err(|error| CoreError::Internal {
                detail: format!("/me/playlists : {error}"),
            })?;
        Ok(self.choices(list.playlists))
    }

    /// `POST /api/favorite` — star or unstar one title. **The rows flip before the
    /// request**, and flip back if the server refuses.
    pub async fn set_track_favorite(&self, key: &str, on: bool) -> Result<(), CoreError> {
        self.patch_and_write(
            key,
            |row| row.liked = on,
            |row| row.liked = !on,
            |api| async move { wire::favorite(&api, reference_of(key), on).await },
        )
        .await
    }

    /// `POST /api/dislike` — the title leaves the Flow, the radios and the mixes.
    /// Optimistic, with the same rollback.
    pub async fn set_track_dislike(&self, key: &str, on: bool) -> Result<(), CoreError> {
        self.patch_and_write(
            key,
            |row| row.disliked = on,
            |row| row.disliked = !on,
            |api| async move { wire::dislike(&api, reference_of(key), on).await },
        )
        .await
    }

    /// `POST /api/playlist/<id>/tracks` — append `keys` (one request) and remember
    /// the playlist for the one-tap shortcut.
    pub async fn add_to_playlist(
        &self,
        playlist_id: &str,
        title: &str,
        keys: &[String],
    ) -> Result<u32, CoreError> {
        if keys.is_empty() {
            return Ok(0);
        }
        let added = self
            .spend(|api| {
                let keys = keys.to_vec();
                async move { wire::add_to_playlist(&api, playlist_id, &keys).await }
            })
            .await?;
        store::write_last_playlist(self, playlist_id, title, true);
        Ok(added)
    }

    /// `POST /api/playlists` — create one with `keys` already in it, and remember
    /// it as the last playlist used (the SPA's `postNewPlaylist`).
    pub async fn create_playlist(
        &self,
        title: &str,
        keys: &[String],
    ) -> Result<PlaylistChoice, CoreError> {
        let created = self
            .spend(|api| {
                let keys = keys.to_vec();
                async move { wire::create_playlist(&api, title, &keys).await }
            })
            .await?;
        store::write_last_playlist(self, created.id.as_str(), title, true);
        Ok(self.choice(created))
    }

    /// `GET /api/replace/candidates/<id>` — ONE request. An empty `query` is the
    /// server's own (`"<artist> <title>"`).
    pub async fn replace_candidates(
        &self,
        key: &str,
        query: &str,
        fallbacks: bool,
    ) -> Result<ReplaceAnswer, CoreError> {
        let answer = self
            .spend(|api| {
                let reference = reference_of(key).to_owned();
                let query = query.to_owned();
                async move { wire::replace_candidates(&api, &reference, &query, fallbacks).await }
            })
            .await?;
        let candidates: Vec<ReplaceCandidate> = answer
            .candidates
            .iter()
            .filter_map(wire::candidate)
            .collect();
        Ok(ReplaceAnswer {
            query: answer.query,
            candidates,
            fallback_auto: answer.fallback_auto.or_else(|| {
                answer
                    .youtube_auto
                    .filter(|auto| *auto)
                    .map(|_| "youtube".to_owned())
            }),
        })
    }

    /// `POST /api/replace` — swap `key` for `candidate` everywhere. ONE request;
    /// the rewrite itself is the server's job (`MutationReport` carries its
    /// handle).
    pub async fn replace(&self, key: &str, candidate: &str) -> Result<MutationReport, CoreError> {
        let from = reference_of(key).to_owned();
        let to = candidate.to_owned();
        let job = self
            .spend(|api| async move { wire::replace(&api, &from, &to).await })
            .await?;
        Ok(MutationReport::started(job))
    }

    /// `GET /api/replace/status/<job>` — how the rewrite is going.
    pub async fn replace_status(&self, job: &str) -> Result<MutationReport, CoreError> {
        let answer = self
            .spend(|api| {
                let job = job.to_owned();
                async move { wire::replace_status(&api, &job).await }
            })
            .await?;
        Ok(report_of(&answer))
    }

    /// `POST /api/tracks/delete` — the bulk deletion of titles gone from both
    /// sources (at most 200 per request; the server re-checks each).
    pub async fn delete_tracks(&self, keys: &[String]) -> Result<MutationReport, CoreError> {
        if keys.is_empty() {
            return Ok(MutationReport::default());
        }
        let answer = self
            .spend(|api| {
                let keys = keys.to_vec();
                async move { wire::delete_tracks(&api, &keys).await }
            })
            .await?;
        let job = answer
            .get("job")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| CoreError::Invalid {
                detail: "le serveur n'a pas rendu de travaux".to_owned(),
            })?
            .to_owned();
        let mut report = MutationReport::started(job);
        report.total = answer
            .get("total")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(keys.len() as u64) as u32;
        Ok(report)
    }

    /// `GET /api/replace/status/<job>` — the bulk deletion's progress and causes.
    pub async fn delete_status(&self, job: &str) -> Result<MutationReport, CoreError> {
        self.replace_status(job).await
    }

    /// `DELETE /api/local/<id>` — one of your own uploads, row and file.
    ///
    /// One request per key, and **each failure keeps its own cause**: a bulk of
    /// five uploads where one belongs to somebody else says so for that one and
    /// removes the other four (rule 2: never just a count).
    pub async fn delete_uploads(&self, keys: &[String]) -> Result<MutationReport, CoreError> {
        let mut report = MutationReport {
            ok: true,
            ..MutationReport::default()
        };
        for key in keys {
            let id = reference_of(key).to_owned();
            let outcome = self
                .spend(|api| async move { wire::delete_upload(&api, &id).await })
                .await;
            match outcome {
                Ok(()) => report.deleted.push(key.clone()),
                Err(cause) => {
                    report.ok = false;
                    report.kept.push(Kept {
                        id: key.clone(),
                        reason: cause.kind().to_owned(),
                    });
                }
            }
        }
        Ok(report)
    }

    /// The address to share for `(kind, id)`:
    ///
    /// - `playlist`: the public read-only page, minted on the first ask and
    ///   reused afterwards (the server's one-link-per-playlist rule);
    /// - `track` / `episode`: `GET /api/share/file/<id>` on this server — the
    ///   whole archived file, what the SPA's share sheet hands out.
    pub async fn share_link(&self, kind: &str, id: &str) -> Result<String, CoreError> {
        let origin = self.origin().ok_or_else(|| CoreError::Invalid {
            detail: "Aucun serveur n'est configuré".to_owned(),
        })?;
        let origin = origin.as_str().to_owned();
        let kind = kind.trim().to_ascii_lowercase();
        if kind == "playlist" {
            let link = self
                .spend(|api| {
                    let id = id.to_owned();
                    async move { wire::playlist_share_link(&api, &id).await }
                })
                .await?;
            return Ok(absolute(&origin, &link));
        }
        // A title: the file route, named by what the server resolves.
        Ok(format!(
            "{}/api/share/file/{}",
            origin.trim_end_matches('/'),
            wire::seg(reference_of(id))
        ))
    }

    /// The per-account playlist list, as choices, with the last one marked.
    fn choices(&self, playlists: Vec<nz_model::Playlist>) -> Vec<PlaylistChoice> {
        let last = store::read_last_playlist(self).map(|last| last.id);
        playlists
            .into_iter()
            .map(|playlist| {
                let recent = last.as_deref().is_some_and(|id| id == playlist.id.as_str());
                let mut choice = self.choice(playlist);
                choice.recent = recent;
                choice
            })
            .collect()
    }

    fn choice(&self, playlist: nz_model::Playlist) -> PlaylistChoice {
        PlaylistChoice {
            id: playlist.id.as_str().to_owned(),
            title: playlist.title,
            cover: playlist.cover,
            nb_tracks: playlist.nb_tracks,
            editable: playlist.editable,
            role: playlist.role,
            owner: playlist.owner,
            recent: false,
            version: playlist.version,
        }
    }

    /// Runs one request on the `User` lane under [`TAG_ACTIONS`], and books what it
    /// spent in the core's own meter (rule 3's "counted, not timed").
    async fn spend<T, F, Fut>(&self, run: F) -> Result<T, CoreError>
    where
        F: FnOnce(Api) -> Fut,
        Fut: std::future::Future<Output = Result<T, CoreError>>,
    {
        let api = self.api()?.with_tag(TAG_ACTIONS);
        let before = totals(&api);
        let answer = run(api.clone()).await;
        let after = totals(&api);
        let requests = after.requests.saturating_sub(before.requests);
        if requests > 0 {
            // `Meter::record` counts ONE request and its bytes: the whole run's
            // bytes go with the first, the remaining N−1 are zeros.
            self.meter().record(
                TAG_ACTIONS,
                after.bytes_in.saturating_sub(before.bytes_in),
                after.bytes_out.saturating_sub(before.bytes_out),
            );
            for _ in 1..requests {
                self.meter().record(TAG_ACTIONS, 0, 0);
            }
        }
        answer
    }

    /// The optimistic write every mutating entry of this area uses: patch every
    /// open list row NOW, tell the surfaces, send the request, and patch back with
    /// the reason when it is refused.
    async fn patch_and_write<P, R, F, Fut>(
        &self,
        key: &str,
        patch: P,
        rollback: R,
        write: F,
    ) -> Result<(), CoreError>
    where
        P: Fn(&mut Row),
        R: Fn(&mut Row),
        F: FnOnce(Api) -> Fut,
        Fut: std::future::Future<Output = Result<(), CoreError>>,
    {
        let patched = self.lists().update_rows(key, patch);
        if patched > 0 {
            self.announce(vec![key.to_owned()]);
        }
        match self.spend(write).await {
            Ok(()) => Ok(()),
            Err(cause) => {
                if patched > 0 {
                    self.lists().update_rows(key, rollback);
                    self.announce(vec![key.to_owned()]);
                }
                Err(cause)
            }
        }
    }

    /// Tells the subscribed surfaces which rows changed (MB10's listener; a screen
    /// re-windows its handle on the version).
    fn announce(&self, keys: Vec<String>) {
        let version = ROWS_VERSION.fetch_add(1, Ordering::SeqCst) + 1;
        let sinks = action_sinks();
        for sink in sinks {
            sink.on_rows_changed(keys.clone(), version);
        }
    }
}

/// `origin` + a server path, without a doubled slash.
fn absolute(origin: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_owned();
    }
    format!(
        "{}/{}",
        origin.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// The status body, as the report a screen reads. The server's job dict carries
/// the counters, the lists and the cause of a whole-job failure.
fn report_of(answer: &serde_json::Value) -> MutationReport {
    let flag = |name: &str| answer.get(name).and_then(serde_json::Value::as_bool);
    let number = |name: &str| {
        answer
            .get(name)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32
    };
    let strings = |name: &str| {
        answer
            .get(name)
            .and_then(serde_json::Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let kept = answer
        .get("kept")
        .and_then(serde_json::Value::as_array)
        .map(|list| {
            list.iter()
                .map(|entry| Kept {
                    id: entry
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    reason: entry
                        .get("reason")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();
    MutationReport {
        job: answer
            .get("job")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        running: flag("running").unwrap_or(false),
        ok: flag("ok").unwrap_or(false),
        playlists: number("playlists"),
        favorites: number("favorites"),
        deleted: strings("deleted"),
        kept,
        missing: strings("missing"),
        done: number("done"),
        total: number("total"),
        error: answer
            .get("error")
            .and_then(serde_json::Value::as_str)
            .filter(|error| !error.is_empty())
            .map(str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::NetworkState;

    fn rows() -> Vec<Row> {
        vec![Row::test_row(0, "Ouverture"), Row::test_row(1, "Suite")]
    }

    /// The acceptance's first line: a like is on every open list BEFORE the
    /// request is answered, one POST leaves, and the version a screen re-windows
    /// on moved.
    #[tokio::test]
    async fn a_like_flips_every_open_list_before_the_request() {
        let (server, _dir, core) = testing::configured().await;
        let registry = core.lists();
        let favourites = registry.open(rows());
        let queue = registry.open(rows());
        let before = favourites.version();
        let version_before = rows_version();
        // The sink is per PROCESS (the app has one shell), so this test asserts
        // what its own write caused and tolerates a sibling test's announce — the
        // one assertion a shared static can honestly make.
        let listening = std::sync::Arc::new(CountingSink::default());
        subscribe(listening.clone());

        core.set_track_favorite("test:1", true)
            .await
            .expect("a star the server accepted");

        // One request, and only one.
        assert_eq!(server.routes(), ["POST /api/favorite"]);
        let body = server.last_body("/api/favorite").expect("a body");
        assert!(body.contains(r#""id":"1""#), "{body}");
        assert!(body.contains(r#""on":true"#), "{body}");
        // Every open list carries it, and its version moved.
        assert!(favourites.window(0, 2)[1].liked);
        assert!(queue.window(0, 2)[1].liked);
        assert!(favourites.version() > before);
        assert!(listening.keys().contains(&"test:1".to_owned()));
        assert!(
            rows_version() > version_before,
            "the counter a screen compares moved"
        );
        // Counted under this area's tag.
        let stats = core.stats();
        let actions = stats
            .iter()
            .find(|row| row.tag == TAG_ACTIONS)
            .expect("the actions tag");
        assert_eq!(actions.requests, 1);
    }

    /// A refusal puts the row back where it was and answers the cause — never a
    /// heart that stays lit on a failed write.
    #[tokio::test]
    async fn a_refused_like_rolls_back_and_keeps_its_cause() {
        let (server, _dir, core) = testing::configured().await;
        server.break_route("/api/favorite", 500);
        let registry = core.lists();
        let list = registry.open(rows());

        let cause = core
            .set_track_favorite("test:1", true)
            .await
            .expect_err("the server refused");
        assert!(matches!(cause, CoreError::Server { status: 500, .. }));
        assert!(!list.window(0, 2)[1].liked, "rolled back");
    }

    /// The heart and the thumb are two flags: disliking a starred title leaves the
    /// star alone (the SPA keeps two stores, and so does this).
    #[tokio::test]
    async fn the_heart_and_the_thumb_are_independent() {
        let (server, _dir, core) = testing::configured().await;
        let registry = core.lists();
        let list = registry.open(rows());
        core.set_track_favorite("test:1", true)
            .await
            .expect("starred");
        core.set_track_dislike("test:1", true)
            .await
            .expect("disliked");
        let row = list.window(0, 2)[1].clone();
        assert!(row.liked && row.disliked);
        assert_eq!(server.routes(), ["POST /api/favorite", "POST /api/dislike"]);
    }

    /// A dead link answers `Offline` in zero sockets, and the row is put back:
    /// nothing was written, so nothing may stay lit.
    #[tokio::test]
    async fn a_dead_link_answers_offline_without_a_socket() {
        let (server, _dir, core) = testing::configured().await;
        let registry = core.lists();
        let list = registry.open(rows());
        core.set_network(NetworkState::None);
        let cause = core
            .set_track_favorite("test:1", true)
            .await
            .expect_err("no link");
        assert!(matches!(cause, CoreError::Offline));
        assert_eq!(server.accepted(), 0, "not one connection");
        assert_eq!(server.count(), 0, "not one request");
        assert!(!list.window(0, 2)[1].liked);
    }

    /// Opening the menu is free — the SPA's own measure — and it is free even on a
    /// dead link, because it asks nothing.
    #[tokio::test]
    async fn the_menu_costs_no_request_at_all() {
        let (server, _dir, core) = testing::configured().await;
        let row = Row::test_row(1, "Ouverture");
        let entries = core.track_menu(&row, &MenuContext::own_list());
        assert!(!entries.is_empty());
        core.set_network(NetworkState::None);
        let offline = core.track_menu(&row, &MenuContext::own_list());
        assert_eq!(entries, offline, "the same menu, with no link");
        assert_eq!(server.count(), 0);
        assert_eq!(server.accepted(), 0);
    }

    /// The picker: ONE request on a cold open, ZERO on the next one (the device's
    /// copy), and a read-only list is still offered (the SCREEN decides what it may
    /// write to; the SPA's shelf shows the list's role).
    #[tokio::test]
    async fn the_picker_asks_once_then_paints_from_the_device() {
        let (server, _dir, core) = testing::configured().await;
        assert!(core.cached_menu_playlists().is_none(), "nothing yet");
        let lists = core.menu_playlists().await.expect("the first answer");
        assert_eq!(server.routes(), ["GET /api/me/playlists"]);
        assert_eq!(lists.len(), 3);
        assert_eq!(lists[0].title, "Route du soir");
        assert!(lists[0].editable);
        assert_eq!(lists[2].role.as_deref(), Some("viewer"));

        // Counted bytes, not a feeling: the one request is the list's own body.
        let actions = core
            .stats()
            .into_iter()
            .find(|row| row.tag == TAG_ACTIONS)
            .expect("the actions tag");
        assert_eq!(actions.requests, 1);
        assert_eq!(
            actions.bytes_in as usize,
            server.playlists_bytes(),
            "one request, and its own bytes ({})",
            server.playlists_bytes()
        );

        server.forget();
        let warm = core.cached_menu_playlists().expect("the device's copy");
        assert_eq!(warm.len(), 3);
        assert_eq!(server.count(), 0, "a warm open asks nothing");
        assert_eq!(
            core.stats()
                .into_iter()
                .find(|row| row.tag == TAG_ACTIONS)
                .map(|row| row.requests),
            Some(1),
            "a warm open adds nothing to the meter"
        );
    }

    /// Adding sends the append WITHOUT a version (the cross-screen picker's own
    /// rule: an append is commutative), and the list becomes the one-tap shortcut
    /// — which is then READ by the menu, not by the picker.
    #[tokio::test]
    async fn adding_a_title_appends_once_and_remembers_the_playlist() {
        let (server, _dir, core) = testing::configured().await;
        let added = core
            .add_to_playlist(
                "11111111-1111-4111-8111-111111111111",
                "Route du soir",
                &["deezer:9001".to_owned()],
            )
            .await
            .expect("appended");
        assert_eq!(added, 1);
        assert_eq!(
            server.routes(),
            ["POST /api/playlist/11111111-1111-4111-8111-111111111111/tracks"]
        );
        let body = server
            .last_body("/api/playlist/11111111-1111-4111-8111-111111111111/tracks")
            .expect("a body");
        assert!(body.contains(r#""tracks":["deezer:9001"]"#), "{body}");
        assert!(
            !body.contains("version"),
            "an append carries no counter: {body}"
        );

        let last = core.last_playlist().expect("remembered");
        assert_eq!(last.id, "11111111-1111-4111-8111-111111111111");
        assert_eq!(last.title, "Route du soir");
        assert!(last.editable);

        // The shortcut is in the menu of the next title, at the top of the
        // playlist group — and opening it still costs nothing.
        server.forget();
        let mut ctx = MenuContext::own_list();
        ctx.last_playlist = core.last_playlist();
        let entries = core.track_menu(&Row::test_row(2, "Suite"), &ctx);
        let labels: Vec<String> = entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) => Some(item.label.clone()),
                MenuEntry::Divider => None,
            })
            .collect();
        assert!(labels.contains(&"Ajouter à « Route du soir »".to_owned()));
        assert_eq!(server.count(), 0);
    }

    /// Creating from the picker: one POST carrying the title and the title being
    /// added, and the answer is a choice the sheet can close on.
    #[tokio::test]
    async fn creating_a_playlist_answers_the_playlist_it_made() {
        let (server, _dir, core) = testing::configured().await;
        let created = core
            .create_playlist("Ma playlist 4", &["deezer:9001".to_owned()])
            .await
            .expect("created");
        assert_eq!(created.title, "Ma playlist 4");
        assert_eq!(created.nb_tracks, 1);
        assert_eq!(server.routes(), ["POST /api/playlists"]);
        assert_eq!(
            core.last_playlist().map(|last| last.title),
            Some("Ma playlist 4".to_owned())
        );
    }

    /// The replacement sheet: ONE request for the candidates, with the source mark
    /// the sheet draws (a file already here, or a fallback's own id).
    #[tokio::test]
    async fn the_replacement_sheet_asks_its_candidates_once() {
        let (server, _dir, core) = testing::configured().await;
        let answer = core
            .replace_candidates("deezer:9001", "", false)
            .await
            .expect("candidates");
        assert_eq!(server.routes(), ["GET /api/replace/candidates/9001"]);
        assert_eq!(answer.query, "Ascenseur Ouverture");
        assert_eq!(answer.candidates.len(), 2);
        assert!(answer.candidates[0].local);
        assert_eq!(
            answer.candidates[0].key,
            "local:44444444-4444-4444-8444-444444444444"
        );
        assert_eq!(answer.candidates[1].source, "youtube");
        assert_eq!(answer.candidates[1].artists, "Ascenseur");
        assert_eq!(answer.fallback_auto, None);

        // A typed query replaces the server's own, and its fallback answer says
        // where it looked on its own.
        server.forget();
        let typed = core
            .replace_candidates("deezer:9001", "June", true)
            .await
            .expect("a typed search");
        assert_eq!(typed.query, "June");
        assert!(typed.candidates.is_empty());
        assert_eq!(typed.fallback_auto.as_deref(), Some("youtube"));
        assert_eq!(server.count(), 1, "one request: {:?}", server.routes());
        let query = server.hits()[0].query("q");
        assert_eq!(query.as_deref(), Some("June"));
        assert_eq!(server.hits()[0].query("fallbacks").as_deref(), Some("1"));
    }

    /// Replacing answers a job at once (rule 2: the rewrite is the server's), and
    /// the report names what it changed.
    #[tokio::test]
    async fn replacing_a_title_answers_a_job_then_its_report() {
        let (server, _dir, core) = testing::configured().await;
        let started = core
            .replace("deezer:9001", "44444444-4444-4444-8444-444444444444")
            .await
            .expect("accepted");
        assert_eq!(started.job.as_deref(), Some("job-replace"));
        assert!(started.running);
        let body = server.last_body("/api/replace").expect("a body");
        assert!(body.contains(r#""from":"9001""#), "{body}");

        let report = core.replace_status("job-replace").await.expect("a status");
        assert!(!report.running);
        assert!(report.ok);
        assert_eq!(report.playlists, 2);
        assert_eq!(report.favorites, 1);
    }

    /// A bulk deletion reports each title it kept WITH its cause, never a count.
    #[tokio::test]
    async fn a_bulk_deletion_names_every_title_it_kept() {
        let (server, _dir, core) = testing::configured().await;
        let report = core
            .delete_tracks(&["deezer:9001".to_owned(), "deezer:9002".to_owned()])
            .await
            .expect("accepted");
        assert_eq!(report.job.as_deref(), Some("job-delete"));
        assert_eq!(report.total, 2);
        assert_eq!(server.routes(), ["POST /api/tracks/delete"]);

        let finished = core.delete_status("job-delete").await.expect("a status");
        assert_eq!(finished.deleted, ["deezer:9001"]);
        assert_eq!(
            finished.kept,
            [Kept {
                id: "deezer:9002".to_owned(),
                reason: "playable".to_owned()
            }]
        );
    }

    /// Deleting an upload is one request per key, and a refusal keeps that key's
    /// cause while the others still go.
    #[tokio::test]
    async fn deleting_uploads_keeps_each_refusals_cause() {
        let (server, _dir, core) = testing::configured().await;
        let report = core
            .delete_uploads(&[
                "local:44444444-4444-4444-8444-444444444444".to_owned(),
                "local:55555555-5555-4555-8555-555555555555".to_owned(),
            ])
            .await
            .expect("walked");
        assert!(report.ok);
        assert_eq!(report.deleted.len(), 2);
        assert_eq!(
            server.routes(),
            [
                "DELETE /api/local/44444444-4444-4444-8444-444444444444",
                "DELETE /api/local/55555555-5555-4555-8555-555555555555",
            ]
        );
    }

    /// A playlist's link is minted on the first ask and reused after: one GET, then
    /// one POST the first time, one GET with nothing else when a link exists.
    #[tokio::test]
    async fn a_playlist_link_is_minted_once_and_then_only_read() {
        let (server, _dir, core) = testing::configured().await;
        let link = core
            .share_link("playlist", "11111111-1111-4111-8111-111111111111")
            .await
            .expect("a link");
        assert_eq!(link, format!("{}/shared/playlist/tok-1", server.base()));
        assert_eq!(
            server.routes(),
            [
                "GET /api/playlist/11111111-1111-4111-8111-111111111111/share",
                "POST /api/playlist/11111111-1111-4111-8111-111111111111/share",
            ]
        );
    }

    /// A title's share address costs no request: it is the file route of the
    /// server this app already speaks to.
    #[tokio::test]
    async fn a_titles_share_address_is_built_without_a_request() {
        let (server, _dir, core) = testing::configured().await;
        let link = core
            .share_link("track", "deezer:9001")
            .await
            .expect("a link");
        assert_eq!(link, format!("{}/api/share/file/9001", server.base()));
        let episode = core
            .share_link("episode", "66666666-6666-4666-8666-666666666666")
            .await
            .expect("a link");
        assert!(episode.ends_with("/api/share/file/66666666-6666-4666-8666-666666666666"));
        assert_eq!(server.count(), 0);
        assert_eq!(server.accepted(), 0);
    }

    /// The row key's id part is the reference a request names (the SPA's
    /// `trackkey.js#trackRef`), and a key that is already a reference is kept.
    #[test]
    fn a_row_key_names_the_reference_the_server_resolves() {
        assert_eq!(reference_of("deezer:9001"), "9001");
        assert_eq!(
            reference_of("local:44444444-4444-4444-8444-444444444444"),
            "44444444-4444-4444-8444-444444444444"
        );
        assert_eq!(reference_of("9001"), "9001");
        assert_eq!(reference_of("deezer:"), "deezer:");
    }

    /// A test's listening surface: what a screen would receive.
    #[derive(Default)]
    struct CountingSink {
        keys: std::sync::Mutex<Vec<String>>,
    }

    impl CountingSink {
        fn keys(&self) -> Vec<String> {
            self.keys
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        }
    }

    impl ActionSink for CountingSink {
        fn on_rows_changed(&self, keys: Vec<String>, _version: u64) {
            self.keys
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .extend(keys);
        }
    }
}
