//! Android Auto's browse tree — the thin `UniFFI` surface over
//! `nz_core::auto` (docs/agents/mobile.md, "MB25 — Android Auto").
//!
//! Owned by **MB25**: this file converts the core's [`nz_core::auto::Item`]s
//! into the records the Kotlin tree draws, and nothing else. **No rule lives
//! here** — the five nodes, their lanes (a browse is `Prefetch`, a tap is
//! `User`), their device copy and their play plans are the core's, where they
//! are tested against a real HTTP server.
//!
//! # What a head unit asks, and what it gets
//!
//! - `auto_cached_children(parent)` — **synchronous, 0 requests**: what
//!   `playback/.../auto/**` paints on the session's own thread, because a
//!   `MediaLibrarySession.Callback` must answer at once (MB3c's `LibraryTree`).
//! - `auto_children(parent)` — the read behind it: the copy while it is younger
//!   than fifteen minutes, else ONE request on the `Prefetch` lane.
//! - `auto_play(media_id)` — what the car's play button must queue: the titles
//!   the node showed (0 requests), or the entity the PLAYER resolves (Mon Flow).
//!
//! A media id is `neezer:<node>` or `neezer:track:<key>`; one the tree did not
//! build is an `NzError::Invalid` with a French sentence, never an empty list
//! that looks like "nothing here" (rule 3).

use nz_core::auto as core;

use crate::core::Screens;
use crate::errors::NzError;

/// What one row of the car's tree is, for the glyph and the layout.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoKind {
    /// A fixed node (Favoris, Playlists, Mon Flow, Récents, Téléchargements).
    Section,
    /// One of the person's playlists.
    Playlist,
    /// One title, played by the same player as everywhere else.
    Track,
}

impl From<core::Kind> for AutoKind {
    fn from(kind: core::Kind) -> Self {
        match kind {
            core::Kind::Section => AutoKind::Section,
            core::Kind::Playlist => AutoKind::Playlist,
            core::Kind::Track => AutoKind::Track,
        }
    }
}

/// One row of the tree, shaped the way the car draws it — plus what the queue
/// needs when a title is tapped (`id`, `album_id`, `duration_s`, …), because the
/// `MediaItem` that carries it crosses a Binder and `NzPlayer.trackOf` rebuilds
/// the player's `TrackRef` from exactly these fields.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct AutoItem {
    /// `neezer:<node>` or `neezer:track:<key>`: stable across restarts, which is
    /// what lets a head unit remember where the person was.
    pub media_id: String,
    pub title: String,
    /// The line under the title: the artists of a title, the count of a list.
    pub subtitle: String,
    pub kind: AutoKind,
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

impl From<core::Item> for AutoItem {
    fn from(item: core::Item) -> Self {
        AutoItem {
            media_id: item.media_id,
            title: item.title,
            subtitle: item.subtitle,
            kind: item.kind.into(),
            browsable: item.browsable,
            playable: item.playable,
            cover: item.cover,
            id: item.id,
            artists: item.artists,
            album: item.album,
            album_id: item.album_id,
            duration_s: item.duration_s,
            liked: item.liked,
            explicit: item.explicit,
            track_count: item.track_count,
        }
    }
}

/// What a tap on a node of the tree must play.
#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum AutoPlay {
    /// These titles ARE the queue (the ones the car just showed): the Kotlin
    /// side turns them into `MediaItem`s and hands them to the session, which
    /// sets them on the ONE player (`NzPlayer` → `PlayQueue`).
    Queue { tracks: Vec<AutoItem> },
    /// The PLAYER resolves it (`PlayerCommand::PlayEntity`): Mon Flow is asked
    /// one title at a time by the queue's own refill, as everywhere else in the
    /// app, so the Kotlin side must NOT replace the queue.
    Entity { kind: String, id: String },
}

impl From<core::Play> for AutoPlay {
    fn from(plan: core::Play) -> Self {
        match plan {
            core::Play::Queue(tracks) => AutoPlay::Queue {
                tracks: tracks.into_iter().map(AutoItem::from).collect(),
            },
            core::Play::Entity { kind, id } => AutoPlay::Entity { kind, id },
        }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Screens {
    /// The children of `parent` from the DEVICE's copy: synchronous, and it
    /// never asks the network (`0` requests). This is what a
    /// `MediaLibrarySession.Callback` answers with on the session's thread.
    pub fn auto_cached_children(&self, parent: String) -> Vec<AutoItem> {
        self.core()
            .auto_cached_children(&parent)
            .into_iter()
            .map(AutoItem::from)
            .collect()
    }

    /// The children of `parent`: the device's copy while it is fresh, else ONE
    /// read on the `Prefetch` lane — never `User` (rule 1: the phone's own
    /// screen comes first, and nobody there asked for this list).
    pub async fn auto_children(&self, parent: String) -> Result<Vec<AutoItem>, NzError> {
        self.core()
            .auto_children(&parent)
            .await
            .map(|items| items.into_iter().map(AutoItem::from).collect())
            .map_err(NzError::from)
    }

    /// What the car's play button must play: the titles the node showed, or the
    /// entity the player resolves. A node the device has never seen is the
    /// action the person just took, so it asks on the `User` lane.
    pub async fn auto_play(&self, media_id: String) -> Result<AutoPlay, NzError> {
        self.core()
            .auto_play(&media_id)
            .await
            .map(AutoPlay::from)
            .map_err(NzError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{CoreConfig, NzCore};

    struct Dir(std::path::PathBuf);

    impl Dir {
        fn new(tag: &str) -> Dir {
            let path =
                std::env::temp_dir().join(format!("nz-ffi-auto-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("data")).expect("data");
            std::fs::create_dir_all(path.join("cache")).expect("cache");
            Dir(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn core(dir: &Dir) -> std::sync::Arc<NzCore> {
        NzCore::new(CoreConfig {
            data_dir: dir.0.join("data").display().to_string(),
            cache_dir: dir.0.join("cache").display().to_string(),
            app_version: "0.0.0".into(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: "test".into(),
        })
        .expect("a core")
    }

    /// The root crosses as five screen-shaped rows, with what the car's layout
    /// reads: a title, a line under it, and whether the node is browsed or
    /// played. It costs nothing: this test has no server at all.
    #[tokio::test]
    async fn the_root_crosses_as_five_screen_shaped_rows() {
        let dir = Dir::new("root");
        let core = core(&dir);
        let screens = core.screens();
        let items = screens
            .auto_children(core::ROOT.to_owned())
            .await
            .expect("the root");
        let titles: Vec<&str> = items.iter().map(|item| item.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Favoris",
                "Playlists",
                "Mon Flow",
                "Récents",
                "Téléchargements"
            ]
        );
        assert_eq!(items[0].kind, AutoKind::Section);
        assert!(items[0].browsable && items[0].playable);
        assert!(items[2].playable && !items[2].browsable);
        // The synchronous path answers the same five rows with no request.
        assert_eq!(screens.auto_cached_children(core::ROOT.to_owned()), items);
        core.shutdown();
    }

    /// A media id the tree did not build crosses as a French cause — what
    /// `LoadFailed` shows — and never as an empty list.
    #[tokio::test]
    async fn a_hostile_id_crosses_as_a_french_cause() {
        let dir = Dir::new("hostile");
        let core = core(&dir);
        let screens = core.screens();
        let error = screens
            .auto_children("neezer:playlist:../etc/passwd".to_owned())
            .await
            .expect_err("refused");
        assert!(matches!(error, NzError::Invalid { .. }), "{error:?}");
        let sentence = crate::errors::nz_error_text(&error);
        assert!(sentence.contains("Nœud inconnu"), "{sentence}");
        assert!(
            screens
                .auto_cached_children("neezer:track:a/b".to_owned())
                .is_empty()
        );
        core.shutdown();
    }
}
