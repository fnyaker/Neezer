//! THIS device's player, lent: the controlled side.
//!
//! The protocol is `nz_remote::host`'s (`SC5`, extracted from the desktop's
//! `wiring/remote_host.rs`): the wire session (poll `since`/`chan`, publish,
//! `ack`, the queue's fingerprint, the settings a controller may read), the two
//! keys this device keeps (`remote.device`, `remote.hosted`) and the poll loop.
//! What lives here is the half that has to know THIS app — the same half the
//! desktop keeps in its own adapter, because it touches its player:
//!
//! - [`HostPlayer`], the seam to the phone's one player: what a controller sees
//!   it doing ([`HostPlayer::now`]), its queue's rows, and how a command it sent
//!   is applied (through `Player::command`, the ONE transport path a tap, a
//!   notification and the car card already take, so the engine, the crossfade
//!   and the queue top-ups follow as if the owner had tapped);
//! - [`HostNow`] → `nz_remote::host::Published`, so the projection from the
//!   player onto the wire is written once;
//! - [`tick`], the foreground step the service's own thread calls: drain what a
//!   poll queued, apply it, hand the queue over when it moved, publish the state.
//!
//! # The rules this file keeps
//!
//! - **Nothing is sent while nothing is lent.** The poll loop starts only when a
//!   link points here ([`Host::start`]), asks the server once at a launch of a
//!   device that was lent before ([`Host::resume`]), and stops the moment the
//!   server says no link is left. A phone nobody drives spends **0 requests**
//!   (the acceptance counts them on `NzCore::stats`).
//! - **A command is never retried.** It arrives in a poll, is applied once, and
//!   the next publish's `ack` says it ran. A command older than the server's own
//!   `CMD_TTL` never even arrives (`remote.py#remote_host_poll`).
//! - **Rule 1**: every loop request is `Lane::User` and tagged `remote`, and the
//!   queue is handed over only when its version moved, so a controller on a
//!   borrowed player costs the owner nothing he can hear.

use std::sync::Arc;

use nz_api::Api;
use nz_player::Command as WireCommand;
use nz_remote::host::{self, HostStatus, Published, RemoteHost};
use serde_json::{Value, json};

use crate::core::Core;
use crate::lists::Row;
use crate::player;

/// What a controller sees this player doing, projected from the player itself.
///
/// A plain struct on purpose: the phone's player is `nz-core::player`'s, the
/// desktop's is its own `AppState`, and both hand the same six numbers to the
/// wire. The screen-shaped rows travel as [`Row`]s, never as `Entry`s, because
/// this core's player exposes what a list DRAWS.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostNow {
    pub index: i64,
    pub playing: bool,
    /// The position the person HEARS: `nz-audio`'s clock, which already
    /// subtracts the device latency (D9's rule, and what a controller's seek bar
    /// must show).
    pub p: f64,
    pub duration: f64,
    pub volume: f64,
    pub muted: bool,
    pub shuffle: bool,
    /// `off`, `all`, `one` — the wire's spellings, not this machine's.
    pub repeat: String,
    /// `playing`, `paused` or `idle` (`playbackStatus`).
    pub status: String,
    /// The track that is playing, as the row a list draws it.
    pub current: Option<Row>,
}

/// What this module asks of the phone's player.
///
/// Implemented once, in `nz-ffi/src/remote.rs`, over the process's ONE
/// `PlayerApi` (`MB3c`'s rule): a second `Player` would hold a second queue and
/// sound nothing. The desktop's equivalent is its `AppState`, which is why the
/// trait lives in the core and not in the FFI file.
pub trait HostPlayer: Send + Sync {
    /// The state a controller sees, built from the player right now.
    fn now(&self) -> HostNow;

    /// The queue's rows. Called only when [`queue_version`](Self::queue_version)
    /// moved — cloning 4 000 rows every 120 ms is the cost this rule exists for.
    fn queue_rows(&self) -> Vec<Row>;

    /// The queue's composition version (`Player::queue_version`).
    fn queue_version(&self) -> u64;

    /// Apply what a controller asked, through the app's ONE transport path.
    /// `tap_ns` is 0: the click happened on the other device, so this device has
    /// no tap instant to attribute.
    fn apply(&self, commands: &[player::Command]);
}

/// The host side of the service.
pub(crate) struct Host {
    core: Arc<Core>,
    player: Arc<dyn HostPlayer>,
    /// The client every loop request goes through (tagged `remote`).
    api: Arc<Api>,
    /// The queue version whose rows were last handed to the wire: the ONE thing
    /// that decides whether 4 000 rows are cloned.
    held_queue: std::sync::Mutex<u64>,
}

impl Host {
    pub(crate) fn new(core: Arc<Core>, player: Arc<dyn HostPlayer>, api: Arc<Api>) -> Host {
        Host {
            core,
            player,
            api,
            held_queue: std::sync::Mutex::new(0),
        }
    }

    /// This device's stable remote id (`remote.device`, `lib/remote/device.js`).
    pub(crate) fn device(&self) -> String {
        host::device_id(self.core.cache())
    }

    /// What the chip paints.
    pub(crate) fn status(&self) -> HostStatus {
        RemoteHost::get()
            .map(|host| host.status())
            .unwrap_or_default()
    }

    /// Whether the loops are running.
    pub(crate) fn running(&self) -> bool {
        RemoteHost::get().is_some_and(|host| host.running())
    }

    /// Whether this device was ever lent — the key that makes a launch ask the
    /// server once instead of never or always.
    pub(crate) fn hosted_before(&self) -> bool {
        RemoteHost::hosted(self.core.cache())
    }

    /// Wire the process's host service to this core's client and cache, once per
    /// session (a re-login replaces the client, so the pointer decides).
    ///
    /// A device that was never lent starts NOTHING and asks NOTHING: the feature
    /// costs a launch zero requests until somebody uses it (rule 3), and no other
    /// area's tests inherit a loop they do not drive.
    pub(crate) fn ensure(&self, runtime: tokio::runtime::Handle) {
        let host = RemoteHost::ensure(
            Arc::clone(&self.api),
            self.core.cache().clone(),
            self.device(),
            runtime,
        );
        if self.hosted_before() {
            // The settings a controller may change: published once here, so its
            // own Réglages shows them instead of an empty card.
            host.hand_over_settings(self.core.cache());
        }
    }

    /// Ask the server whether a link still points here, and answer again if one
    /// does — the call a launch makes, so a restart with a live link does not
    /// strand a controller on "Lecteur hors ligne".
    pub(crate) fn resume(&self, runtime: tokio::runtime::Handle) {
        self.ensure(runtime);
        RemoteHost::get().unwrap().resume(self.hosted_before());
    }

    /// Start answering (a link was just made on this device, or one was found at
    /// a launch). Idempotent.
    pub(crate) fn start(&self, runtime: tokio::runtime::Handle) {
        self.ensure(runtime);
        if let Some(host) = RemoteHost::get() {
            host.hand_over_settings(self.core.cache());
            host.start();
        }
    }

    /// Stop answering. Nothing is sent anywhere afterwards.
    pub(crate) fn stop(&self) {
        if let Some(host) = RemoteHost::get() {
            host.stop();
        }
    }

    /// The owner's always-reachable button: stop the loops FIRST (nothing may be
    /// sent on a link that is being cut), then cut every link onto this device.
    pub(crate) async fn cut_all(&self) -> Result<(), crate::core::CoreError> {
        if let Some(host) = RemoteHost::get() {
            host.stop();
        }
        let answer = super::wire::revoke_device(&self.api, &self.device()).await;
        if let Some(host) = RemoteHost::get() {
            host.hand_over_settings(self.core.cache());
        }
        answer
    }

    /// One foreground step: apply what a poll queued, then hand over what this
    /// player is doing. Called by the service's own thread every 120 ms while the
    /// loops run, which is the desktop's `spawn_tick` cadence — and does NOTHING
    /// when nothing is lent.
    pub(crate) fn tick(&self) {
        let Some(host) = RemoteHost::get() else {
            return;
        };
        if !host.running() {
            return;
        }
        // 1. What a controller asked, applied through the one transport path.
        let pending = host.drain_pending();
        let ran = pending.len();
        let mut settings_changed = false;
        for item in pending {
            match item {
                host::Pending::Command(command) => self.player.apply(&core_commands(command)),
                host::Pending::Setting { key, value } => {
                    // The crate does not decide what THIS build lends: the key
                    // must be one of ours, and its value is validated by the
                    // same `check` Réglages uses. A refused write changes
                    // nothing, and the controller's card snaps back on its next
                    // read — the honest answer.
                    if let Some(applied) = super::settings::apply(&self.core, &key, &value) {
                        if let Err(error) = applied {
                            tracing::debug!(key, %error, "réglage de télécommande refusé");
                        }
                        settings_changed = true;
                    } else {
                        tracing::debug!(key, "réglage non prêté, ignoré");
                    }
                }
                host::Pending::Ignored(op) => {
                    tracing::debug!(op, "commande de contrôle ignorée");
                }
            }
        }
        if settings_changed {
            host.hand_over_settings(self.core.cache());
        }
        host.note_applied(ran);
        // 2. What this player is doing: the queue first when it moved, then the
        // state that describes it, so the two never travel out of step.
        let version = self.player.queue_version();
        {
            let mut held = lock(&self.held_queue);
            if *held != version {
                *held = version;
                let rows = self.player.queue_rows();
                let entries: Vec<Arc<nz_player::Entry>> =
                    rows.iter().map(entry_of).map(Arc::new).collect();
                host.hand_over_queue(version, &entries);
            }
        }
        host.publish_state(published(self.player.now()));
    }
}

/// What one publish carries, from the player's own numbers.
fn published(now: HostNow) -> Published {
    Published {
        index: now.index,
        playing: now.playing,
        p: now.p,
        duration: now.duration,
        volume: now.volume,
        muted: now.muted,
        shuffle: now.shuffle,
        repeat: now.repeat,
        status: now.status,
        track: now.current.as_ref().map(slim_track),
        // The phone's player does not keep a `Context` in its snapshot (a Flow
        // or a radio's seed): the field goes out empty, which the server treats
        // as "no context" and the controller's screen ignores. The desktop sends
        // its own; nothing else in the protocol depends on it.
        context: None,
    }
}

/// A track as the other side needs it: what the lists and the player DRAW, and
/// nothing else (`commands.js#slimTrack`).
///
/// A 5 000-track queue crosses in full whenever it changes, and a podcast
/// episode's description alone can be a paragraph: the keys below are the SPA's
/// own list, cut to what a [`Row`] carries.
fn slim_track(row: &Row) -> Value {
    let id = if playable(&row.id) {
        row.id.clone()
    } else {
        row.key.clone()
    };
    let mut out = json!({
        "id": id,
        "title": row.title,
        "duration": row.duration_s,
        "explicit": row.explicit,
        "cover": row.cover,
        "unavailable": row.unavailable,
        "local": row.source == "local",
    });
    if !row.artists.is_empty() {
        out["artist"] = json!({ "name": row.artists });
    }
    if !row.album.is_empty() || row.album_id.is_some() {
        out["album"] = json!({
            "id": row.album_id,
            "title": row.album,
            "cover": row.cover,
        });
    }
    out
}

/// Whether a string can stand in a URL path segment (`commands.js#playableId`).
fn playable(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// A queue row as the entry the wire's fingerprint and body are built from.
fn entry_of(row: &Row) -> nz_player::Entry {
    let id = if playable(&row.id) {
        row.id.clone()
    } else {
        row.key.clone()
    };
    let mut entry = nz_player::Entry::new(id.clone());
    entry.title = if row.title.is_empty() {
        id
    } else {
        row.title.clone()
    };
    entry.duration = row.duration_s;
    entry.cover = row.cover.clone();
    entry.local = (row.source == "local").then_some(true);
    entry.unavailable = row.unavailable.then_some(true);
    if !row.artists.is_empty() {
        entry.artist = Some(nz_player::EntryArtist {
            id: None,
            deezer_id: None,
            name: row.artists.clone(),
            role: None,
        });
    }
    if !row.album.is_empty() || row.album_id.is_some() {
        entry.album = Some(nz_player::EntryAlbum {
            id: row.album_id.clone(),
            deezer_id: None,
            title: row.album.clone(),
            cover: row.cover.clone(),
        });
    }
    entry
}

/// A command that came over the wire, as this player's own commands.
///
/// One wire op can be two of ours: `shuffle_play` replaces the queue AND turns
/// shuffle on, and `PlayerState::apply` cannot do both in one call. Everything
/// else is one for one — the wire's tags ARE `commands.js`'s op names, which is
/// why a remote command is DECODED by `nz-remote` and never re-dispatched here.
pub(crate) fn core_commands(command: WireCommand) -> Vec<player::Command> {
    use nz_player::Command as Wire;
    match command {
        Wire::Play { .. } => vec![player::Command::Play],
        Wire::Pause { .. } => vec![player::Command::Pause],
        Wire::Toggle { .. } => vec![player::Command::Toggle],
        Wire::Next { .. } => vec![player::Command::Next],
        Wire::Prev { .. } => vec![player::Command::Prev],
        Wire::Seek { args } => vec![player::Command::Seek { s: args.t.max(0.0) }],
        Wire::Jump { args } => vec![player::Command::Jump {
            index: args.i.max(0) as u32,
            key: args.id.unwrap_or_default(),
        }],
        Wire::RemoveAt { args } => vec![player::Command::Remove {
            index: args.i.max(0) as u32,
            key: args.id.unwrap_or_default(),
        }],
        Wire::Move { args } => vec![player::Command::Move {
            from: args.i.max(0) as u32,
            to: args.to.max(0) as u32,
        }],
        Wire::ClearUpcoming { .. } => vec![player::Command::ClearUpcoming],
        Wire::SetVolume { args } => vec![player::Command::SetVolume {
            v: args.v.clamp(0.0, 1.0) as f32,
        }],
        Wire::ToggleMute { .. } => vec![player::Command::ToggleMute],
        Wire::ToggleShuffle { .. } => vec![player::Command::SetShuffle { on: true }],
        Wire::CycleRepeat { .. } => vec![player::Command::CycleRepeat],
        Wire::AddToQueue { args } => vec![player::Command::AddToQueue {
            tracks: tracks_of(args.tracks),
        }],
        Wire::PlayNext { args } => vec![player::Command::PlayNext {
            tracks: tracks_of(args.tracks),
        }],
        Wire::PlayQueue { args } => vec![player::Command::PlayQueue {
            tracks: tracks_of(args.tracks),
            start: args.start as u32,
            context: args.context,
        }],
        Wire::ShufflePlay { args } => vec![
            player::Command::PlayQueue {
                tracks: tracks_of(args.tracks),
                start: 0,
                context: args.context,
            },
            player::Command::SetShuffle { on: true },
        ],
    }
}

/// The wire's entries, as the phone's queue names a track.
fn tracks_of(entries: Vec<nz_player::Entry>) -> Vec<player::TrackRef> {
    entries.iter().map(track_of).collect()
}

/// One wire entry as the row the phone's player draws and resolves by id.
fn track_of(entry: &nz_player::Entry) -> player::TrackRef {
    let artists = {
        let names = entry
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        if names.is_empty() {
            entry.artist_name().to_owned()
        } else {
            names
        }
    };
    player::TrackRef {
        key: entry.key().to_owned(),
        id: entry.reference().unwrap_or(entry.key()).to_owned(),
        title: entry.title.clone(),
        artists,
        album: entry
            .album
            .as_ref()
            .map(|album| album.title.clone())
            .unwrap_or_default(),
        album_id: entry
            .album
            .as_ref()
            .and_then(|album| album.id.clone().or_else(|| album.deezer_id.clone())),
        artist_id: entry
            .artist
            .as_ref()
            .and_then(|artist| artist.id.clone().or_else(|| artist.deezer_id.clone())),
        cover: entry.cover.clone(),
        duration_s: entry.duration,
        episode: entry.is_podcast(),
        liked: false,
    }
}

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}
