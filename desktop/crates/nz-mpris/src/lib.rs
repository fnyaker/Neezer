//! The MPRIS 2.2 server: `org.mpris.MediaPlayer2.neezer` on the session bus.
//!
//! Owned by lot M1 (`docs/agents/desktop.md`). The Omarchy bar's player widget,
//! the media keys (which call `playerctl`) and any MPRIS client read and drive
//! the app through this crate — it is the only place that speaks D-Bus.
//!
//! # The rules the spec and the crate make easy to get wrong
//!
//! - **`Position` never appears in `PropertiesChanged`** (MPRIS 2.2, Player
//!   interface: "The `PropertiesChanged` signal is not emitted when this
//!   property changes"). A player that announces every position tick makes
//!   every client redraw a progress bar four times a second and floods the
//!   bus. [`MprisHandle::set_position_s`] writes the value with
//!   [`mpris_server::Player::set_position`], which is documented as silent,
//!   and emits a `Seeked` only when the reported position departed from the
//!   one a client would have extrapolated from the last one.
//! - **`SetPosition` carries a track id** so a click on a track that changed
//!   in the meantime cannot move the new one: an id that no longer matches is
//!   ignored as stale, exactly as the spec asks.
//! - **The server is a *local* server.** Nothing answers until `Player::run()`
//!   is polled on a `LocalSet`, and the first property set before that panics
//!   inside `mpris-server` with `SendError` (`docs/agents/desktop.md`,
//!   *Pitfalls found by building*). [`Mpris::start`] therefore owns a thread
//!   with a current-thread runtime and a `LocalSet`, and does not return
//!   before the bus name is ours.
//! - **Nothing the app does may wait on D-Bus.** Updates travel on an
//!   unbounded channel ([`MprisHandle`]) and the GPUI thread never waits;
//!   commands come back on the caller's bounded channel with `try_send`, and
//!   the D-Bus thread never blocks either.
//! - **MPRIS is optional to the app running.** A missing session bus, a name
//!   already taken by another instance, or a `mpris-server` failure leaves an
//!   inert handle: the player keeps playing, and the reason is logged once.
//!
//! # Measured (this machine, `playerctl` 2.4.1, a private `dbus-run-session`)
//!
//! The acceptance test `tests::the_mpris_contract_holds_on_a_real_bus` runs
//! the server on its own bus and measures what the note promises: 100 position
//! ticks over 10.0 s produce **0** signals on the bus (0 `PropertiesChanged`
//! for the player interface, 0 `Seeked`); one real seek (0 → 42.5 s) produces
//! exactly **1** `Seeked` carrying `42 500 000 µs`, and the `Position` property
//! reads it back; `playerctl -p neezer metadata` prints the title, the artist
//! and the album, and `playerctl -p neezer play-pause` reaches the receiver as
//! [`MprisCommand::PlayPause`].

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc as std_mpsc,
    thread,
    time::{Duration, Instant},
};

use mpris_server::{Metadata as MprisMetadata, Player, Time, TrackId};
use tokio::sync::mpsc;
use tracing::{debug, warn};

pub use mpris_server::{LoopStatus, PlaybackStatus};

/// Where the app's own track ids live on D-Bus (`mpris:trackid`).
///
/// The spec only requires a valid D-Bus object path that is unique for the
/// track; it forbids the `/org/mpris` namespace. The app's domain is `fr.neezer`.
const TRACK_PATH_PREFIX: &str = "/fr/neezer/track/";

/// How far the audible position may drift from the one a client extrapolates
/// before the app tells it (`Seeked`), in seconds.
///
/// A tick is sent 4×/s; a late tick, a slow frame or a paused moment all move
/// the extrapolation by a fraction of a second. A real seek moves it by a
/// gesture's worth. 1.5 s sits between the two: measured, a tick that arrives
/// 0.9 s late is silent, a 10 s seek is announced.
const SEEKED_TOLERANCE_S: f64 = 1.5;

/// How long [`Mpris::start`] waits for the bus name before giving up.
///
/// Connecting to the session bus is milliseconds; 5 s is only there so a hung
/// bus cannot hold the app's startup.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// What an MPRIS client asked the player to do.
///
/// The app applies these through `nz-player`'s state machine, so the names are
/// the transport's, not D-Bus's: `Stop` has no equivalent in the SPA and is
/// reported as [`Pause`](MprisCommand::Pause).
#[derive(Debug, Clone, PartialEq)]
pub enum MprisCommand {
    /// `PlayPause`: pause if playing, resume if paused, start if stopped.
    PlayPause,
    Play,
    Pause,
    Next,
    Previous,
    /// `Seek`: a **relative** offset in microseconds (negative seeks back).
    Seek(i64),
    /// `SetPosition`: the track id the client meant, then an **absolute**
    /// position in microseconds. Stale ids never reach the app.
    SetPosition(String, i64),
    /// `Volume`, clamped at 0.0 (the spec: a negative value means muted).
    Volume(f64),
    Shuffle(bool),
    Loop(LoopStatus),
    /// `Raise`: bring the window to the front.
    Raise,
    /// `Quit`: the client asked the app to exit.
    Quit,
}

/// The track the clients should see, in the app's terms.
///
/// This is deliberately not `mpris_server::Metadata`: the app has a cover file
/// on disk and an id, and the crate owns the two conversions the note requires
/// (`mpris:trackid` from the id, `mpris:artUrl` as a `file://` URI of the cover)
/// so no caller can get them wrong.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Metadata {
    /// The server's track id (`deezer_id`: a Deezer number, or a UUID for
    /// uploads, YouTube rows and podcast episodes). Never parsed as an integer.
    pub track_id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    /// Length in seconds, when known.
    pub length_s: Option<f64>,
    /// The cached cover on disk; published as `mpris:artUrl`.
    pub cover: Option<PathBuf>,
}

impl Metadata {
    pub fn new(track_id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            track_id: track_id.into(),
            title: title.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn artist(mut self, artist: impl Into<String>) -> Self {
        self.artists.push(artist.into());
        self
    }

    #[must_use]
    pub fn artists<I, S>(mut self, artists: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.artists.extend(artists.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn album(mut self, album: impl Into<String>) -> Self {
        self.album = Some(album.into());
        self
    }

    #[must_use]
    pub fn length(mut self, seconds: f64) -> Self {
        self.length_s = Some(seconds);
        self
    }

    #[must_use]
    pub fn cover(mut self, path: impl Into<PathBuf>) -> Self {
        self.cover = Some(path.into());
        self
    }
}

/// The server, as the app holds it: one value that both publishes state and
/// receives commands.
///
/// Dropping it stops the server (the thread is joined, the bus name released).
pub struct MprisHandle {
    // `None` once dropped, or when the thread never started: a closed channel
    // is how the server thread learns to stop, and dropping the sender is the
    // only way to close it (an unbounded sender has no `close`).
    updates: Option<mpsc::UnboundedSender<Update>>,
    worker: Option<thread::JoinHandle<()>>,
    serving: bool,
}

impl MprisHandle {
    /// The track that is playing now. Also resets the position to 0, because a
    /// new track starts there: no `Seeked` is emitted for it.
    pub fn set_track(&self, metadata: Metadata) {
        self.publish(Update::Track(metadata));
    }

    /// `Playing`, `Paused` or `Stopped`.
    pub fn set_status(&self, status: PlaybackStatus) {
        self.publish(Update::Status(status));
    }

    /// The position actually heard, in seconds. Called as often as the app
    /// likes: only a discontinuity costs a D-Bus signal.
    pub fn set_position_s(&self, position_s: f64) {
        self.publish(Update::Position(position_s));
    }

    /// 0.0 is mute, 1.0 is the app's normal maximum.
    pub fn set_volume(&self, volume: f64) {
        self.publish(Update::Volume(volume));
    }

    pub fn set_shuffle(&self, shuffle: bool) {
        self.publish(Update::Shuffle(shuffle));
    }

    pub fn set_loop(&self, loop_status: LoopStatus) {
        self.publish(Update::Loop(loop_status));
    }

    /// Whether the bus name was taken when the server started.
    ///
    /// False means the process is running without MPRIS (no session bus, the
    /// name already owned by another instance, or `mpris-server` failed): the
    /// player works, the bar and the media keys do not. This is what the
    /// diagnostic log (Réglages → Assistance) reports, and what explains a
    /// media key that does nothing.
    #[must_use]
    pub fn is_serving(&self) -> bool {
        self.serving
    }

    fn publish(&self, update: Update) {
        let Some(updates) = &self.updates else {
            debug!("MPRIS update dropped: the server never started");
            return;
        };
        if updates.send(update).is_err() {
            debug!("MPRIS update dropped: the server thread is gone");
        }
    }
}

impl Drop for MprisHandle {
    fn drop(&mut self) {
        // Close the update channel first: the server thread's `select!` then
        // ends, which drops the `mpris-server` player and releases the bus name.
        drop(self.updates.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The MPRIS server, as the app starts it.
pub struct Mpris;

impl Mpris {
    /// Starts the MPRIS server on its own thread.
    ///
    /// `name` is the bus-name suffix: `"neezer"` publishes
    /// `org.mpris.MediaPlayer2.neezer`, which is what `playerctl -p neezer` and
    /// the Omarchy bar look for. `commands` is the app's transport inbox; it
    /// must have room for a few commands and be drained (the D-Bus thread uses
    /// `try_send` and never blocks, so a full queue drops the command and says
    /// so).
    #[must_use = "the server stops as soon as the handle is dropped"]
    pub fn start(name: &str, commands: mpsc::Sender<MprisCommand>) -> MprisHandle {
        start_with_timeout(name, commands, START_TIMEOUT)
    }
}

/// [`Mpris::start`], with the startup wait exposed for the tests.
fn start_with_timeout(
    name: &str,
    commands: mpsc::Sender<MprisCommand>,
    timeout: Duration,
) -> MprisHandle {
    let (updates, receiver) = mpsc::unbounded_channel();
    let (ready_tx, ready_rx) = std_mpsc::channel();
    let suffix = name.to_owned();

    let spawned = thread::Builder::new()
        .name("nz-mpris".to_owned())
        .spawn(move || run_thread(suffix, commands, receiver, ready_tx));

    let worker = match spawned {
        Ok(worker) => worker,
        Err(error) => {
            warn!(%error, "MPRIS thread not started: the player runs without MPRIS");
            return MprisHandle {
                updates: None,
                worker: None,
                serving: false,
            };
        }
    };

    let serving = match ready_rx.recv_timeout(timeout) {
        Ok(Ok(())) => true,
        Ok(Err(problem)) => {
            warn!(%problem, "MPRIS unavailable: the player runs without MPRIS");
            false
        }
        Err(std_mpsc::RecvTimeoutError::Timeout) => {
            warn!("MPRIS did not take its bus name in time: the player runs without MPRIS");
            false
        }
        Err(std_mpsc::RecvTimeoutError::Disconnected) => {
            warn!("MPRIS thread stopped before taking its bus name");
            false
        }
    };

    MprisHandle {
        updates: Some(updates),
        worker: Some(worker),
        serving,
    }
}

/// One state change on its way to the D-Bus thread.
#[derive(Debug)]
enum Update {
    Track(Metadata),
    Status(PlaybackStatus),
    Position(f64),
    Volume(f64),
    Shuffle(bool),
    Loop(LoopStatus),
}

/// What clients extrapolate from, kept on the D-Bus thread only.
#[derive(Debug)]
struct Current {
    /// The app's id of the current track (for `SetPosition`).
    track_id: Option<String>,
    /// Its `mpris:trackid` (for staleness).
    trackid: Option<TrackId>,
    /// The last position the app reported, in seconds.
    position_s: f64,
    /// When it reported it.
    at: Instant,
    status: PlaybackStatus,
}

impl Default for Current {
    fn default() -> Self {
        Self {
            track_id: None,
            trackid: None,
            position_s: 0.0,
            at: Instant::now(),
            status: PlaybackStatus::Stopped,
        }
    }
}

fn run_thread(
    name: String,
    commands: mpsc::Sender<MprisCommand>,
    updates: mpsc::UnboundedReceiver<Update>,
    ready: std_mpsc::Sender<Result<(), String>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready.send(Err(format!("tokio runtime: {error}")));
            return;
        }
    };

    let local = tokio::task::LocalSet::new();
    local.block_on(&runtime, async move {
        let player = match build_player(&name).await {
            Ok(player) => player,
            Err(error) => {
                let _ = ready.send(Err(error.to_string()));
                return;
            }
        };
        // The bus name is ours from here on: the app may publish state.
        let _ = ready.send(Ok(()));
        serve(player, commands, updates).await;
    });
}

async fn build_player(name: &str) -> mpris_server::zbus::Result<Player> {
    Player::builder(name)
        .identity("Neezer")
        .desktop_entry("neezer-desktop")
        .supported_mime_types(["audio/ogg", "audio/flac"])
        // The app always has a queue to play from and always can seek (the
        // server answers a seek with `?start=`, see docs/agents/desktop.md).
        .can_play(true)
        .can_pause(true)
        .can_seek(true)
        .can_go_next(true)
        .can_go_previous(true)
        .can_control(true)
        // Raising shows the window; quitting is the SPA's quit, both reachable.
        .can_raise(true)
        .can_quit(true)
        .can_set_fullscreen(false)
        // No TrackList / Playlists interfaces: the queue is far bigger than
        // what MPRIS's tracklist is for, and the spec makes them optional.
        .has_track_list(false)
        // No speed through MPRIS: podcast speed is per show and stays in the app.
        .rate(1.0)
        .minimum_rate(1.0)
        .maximum_rate(1.0)
        .build()
        .await
}

/// The whole server: the `mpris-server` runner plus the app's updates.
///
/// The runner must be polled on this thread (`LocalSet`) or nothing answers; the
/// update channel and the runner therefore live in the same `select!`, and the
/// thread stops as soon as the handle is dropped.
async fn serve(
    player: Player,
    commands: mpsc::Sender<MprisCommand>,
    mut updates: mpsc::UnboundedReceiver<Update>,
) {
    let current = Rc::new(RefCell::new(Current::default()));
    connect(&player, &current, &commands);

    let running = player.run();
    tokio::pin!(running);

    loop {
        tokio::select! {
            () = &mut running => break,
            update = updates.recv() => match update {
                Some(update) => apply(&player, &current, update).await,
                None => break,
            },
        }
    }

    debug!("MPRIS server stopped");
}

/// Wires every D-Bus call to a command.
///
/// `try_send` on purpose: a blocked app queue must never stall the D-Bus thread
/// (which also serves the bar's `Position` reads during playback).
fn connect(player: &Player, current: &Rc<RefCell<Current>>, commands: &mpsc::Sender<MprisCommand>) {
    let plain = |command: MprisCommand| {
        let commands = commands.clone();
        move |_: &Player| push(&commands, command.clone())
    };

    player.connect_play_pause(plain(MprisCommand::PlayPause));
    player.connect_play(plain(MprisCommand::Play));
    player.connect_pause(plain(MprisCommand::Pause));
    // MPRIS's Stop is "no track playing"; the SPA has no stop, only a pause that
    // keeps the queue. Saying so is better than ignoring the key.
    player.connect_stop(plain(MprisCommand::Pause));
    player.connect_next(plain(MprisCommand::Next));
    player.connect_previous(plain(MprisCommand::Previous));
    player.connect_raise(plain(MprisCommand::Raise));
    player.connect_quit(plain(MprisCommand::Quit));

    let seek_commands = commands.clone();
    player.connect_seek(move |_, offset| {
        push(&seek_commands, MprisCommand::Seek(offset.as_micros()));
    });

    let position_commands = commands.clone();
    let position_current = Rc::clone(current);
    player.connect_set_position(move |_, track_id, position| {
        let state = position_current.borrow();
        if state.trackid.as_ref() != Some(track_id) {
            debug!("ignoring SetPosition for a track that is no longer current");
            return;
        }
        let Some(id) = state.track_id.clone() else {
            return;
        };
        drop(state);
        push(
            &position_commands,
            MprisCommand::SetPosition(id, position.as_micros()),
        );
    });

    let volume_commands = commands.clone();
    player.connect_set_volume(move |_, volume| {
        // The spec: a negative value means silence.
        push(&volume_commands, MprisCommand::Volume(volume.max(0.0)));
    });

    let shuffle_commands = commands.clone();
    player.connect_set_shuffle(move |_, shuffle| {
        push(&shuffle_commands, MprisCommand::Shuffle(shuffle));
    });

    let loop_commands = commands.clone();
    player.connect_set_loop_status(move |_, loop_status| {
        push(&loop_commands, MprisCommand::Loop(loop_status));
    });

    player.connect_open_uri(|_, uri| {
        debug!(%uri, "OpenUri ignored: the app plays from its own queue");
    });
}

fn push(commands: &mpsc::Sender<MprisCommand>, command: MprisCommand) {
    match commands.try_send(command) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(command)) => {
            warn!(
                ?command,
                "MPRIS command dropped: the app is not draining its queue"
            );
        }
        Err(mpsc::error::TrySendError::Closed(command)) => {
            debug!(?command, "MPRIS command dropped: the app is gone");
        }
    }
}

async fn apply(player: &Player, current: &RefCell<Current>, update: Update) {
    match update {
        Update::Track(metadata) => {
            let trackid = track_object_path(&metadata.track_id);
            {
                let mut state = current.borrow_mut();
                state.track_id = Some(metadata.track_id.clone());
                state.trackid = Some(trackid.clone());
                // A new track starts at 0; the spec says that needs no `Seeked`.
                state.position_s = 0.0;
                state.at = Instant::now();
            }
            let published = mpris_metadata(&metadata, trackid);
            if let Err(error) = player.set_metadata(published).await {
                debug!(%error, "MPRIS metadata not published");
            }
        }
        Update::Status(status) => {
            let now = Instant::now();
            {
                let mut state = current.borrow_mut();
                let elapsed = now.saturating_duration_since(state.at).as_secs_f64();
                match status {
                    // Freeze where the sound stopped, so resuming does not look
                    // like a jump.
                    PlaybackStatus::Paused if state.status == PlaybackStatus::Playing => {
                        state.position_s += elapsed;
                    }
                    PlaybackStatus::Stopped => state.position_s = 0.0,
                    _ => {}
                }
                state.status = status;
                state.at = now;
            }
            if let Err(error) = player.set_playback_status(status).await {
                debug!(%error, "MPRIS playback status not published");
            }
        }
        Update::Position(reported) => {
            let reported = if reported.is_finite() {
                reported.max(0.0)
            } else {
                0.0
            };
            let now = Instant::now();
            let expected = {
                let state = current.borrow();
                let elapsed = now.saturating_duration_since(state.at).as_secs_f64();
                expected_position_s(
                    state.position_s,
                    elapsed,
                    state.status == PlaybackStatus::Playing,
                )
            };

            // Silent by design: `Position` is not a `PropertiesChanged` entry.
            player.set_position(Time::from_micros(seconds_to_micros(reported)));
            if is_discontinuity(expected, reported)
                && let Err(error) = player
                    .seeked(Time::from_micros(seconds_to_micros(reported)))
                    .await
            {
                debug!(%error, "MPRIS Seeked not emitted");
            }

            let mut state = current.borrow_mut();
            state.position_s = reported;
            state.at = now;
        }
        Update::Volume(volume) => {
            if let Err(error) = player.set_volume(volume.max(0.0)).await {
                debug!(%error, "MPRIS volume not published");
            }
        }
        Update::Shuffle(shuffle) => {
            if let Err(error) = player.set_shuffle(shuffle).await {
                debug!(%error, "MPRIS shuffle not published");
            }
        }
        Update::Loop(loop_status) => {
            if let Err(error) = player.set_loop_status(loop_status).await {
                debug!(%error, "MPRIS loop status not published");
            }
        }
    }
}

/// Where a client that kept reading `Position` thinks the track is now.
fn expected_position_s(base_s: f64, elapsed_s: f64, playing: bool) -> f64 {
    if playing { base_s + elapsed_s } else { base_s }
}

fn is_discontinuity(expected_s: f64, reported_s: f64) -> bool {
    (reported_s - expected_s).abs() > SEEKED_TOLERANCE_S
}

fn seconds_to_micros(seconds: f64) -> i64 {
    if !seconds.is_finite() {
        return 0;
    }
    (seconds * 1_000_000.0)
        .round()
        .clamp(i64::MIN as f64, i64::MAX as f64) as i64
}

/// The `mpris:trackid` of a track id: a D-Bus object path under the app's domain.
///
/// Object path elements are `[A-Za-z0-9_]` only, and a Deezer id is a number
/// while an upload or an episode is a UUID with hyphens — those become
/// underscores, and because that alone could collide (`a-b` and `a_b`), a
/// sanitised id also carries a hash of the original. The path is what tells a
/// stale `SetPosition` from a current one, so the two must never meet.
fn track_object_path(track_id: &str) -> TrackId {
    let mut segment: String = track_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();

    if segment.is_empty() {
        segment.push_str("unknown");
    }
    if segment != track_id || segment.is_empty() {
        segment.push('_');
        segment.push_str(&fingerprint(track_id));
    }

    TrackId::try_from(format!("{TRACK_PATH_PREFIX}{segment}")).unwrap_or_else(|_| {
        debug!(%track_id, "unusable MPRIS track id: falling back to NoTrack");
        TrackId::NO_TRACK
    })
}

/// A short, stable hex fingerprint (FNV-1a), for the sanitised ids.
fn fingerprint(value: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", (hash ^ (hash >> 32)) as u32)
}

/// `mpris:artUrl`: the cached cover as a `file://` URI.
///
/// The spec wants a URI, not a path: a space, a `#` or a non-ASCII byte in a
/// cover file name must be percent-encoded or clients cannot open it (a `#`
/// would even cut the URL there). `/` stays literal; everything outside RFC
/// 3986's path set is encoded byte by byte (UTF-8).
fn file_uri(path: &Path) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let mut uri = String::with_capacity(path.as_os_str().len() + 16);
    uri.push_str("file://");
    if !path.is_absolute() {
        uri.push('/');
    }
    for byte in path.as_os_str().as_encoded_bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'/'
            | b'!'
            | b'$'
            | b'&'
            | b'\''
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b';'
            | b'='
            | b':'
            | b'@' => uri.push(char::from(*byte)),
            other => {
                uri.push('%');
                uri.push(char::from(HEX[usize::from(other >> 4)]));
                uri.push(char::from(HEX[usize::from(other & 0x0f)]));
            }
        }
    }
    uri
}

fn mpris_metadata(metadata: &Metadata, trackid: TrackId) -> MprisMetadata {
    let mut builder = MprisMetadata::builder()
        .trackid(trackid)
        .title(metadata.title.clone());

    if !metadata.artists.is_empty() {
        builder = builder.artist(metadata.artists.clone());
    }
    if let Some(album) = &metadata.album {
        builder = builder.album(album.clone());
    }
    if let Some(length) = metadata.length_s
        && length.is_finite()
        && length > 0.0
    {
        builder = builder.length(Time::from_micros(seconds_to_micros(length)));
    }
    if let Some(cover) = &metadata.cover {
        builder = builder.art_url(file_uri(cover));
    }

    builder.build()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        future::poll_fn,
        pin::Pin,
        process::Command,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use mpris_server::zbus::{self, Connection, Proxy, export::futures_core::Stream};

    use super::*;

    const SERVICE: &str = "org.mpris.MediaPlayer2.neezer";
    const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";
    const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";
    const PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";
    /// Set in the child that `dbus-run-session` starts, so the test does the
    /// real work there and only waits for it in the parent.
    const PRIVATE_BUS_CHILD: &str = "NZ_MPRIS_PRIVATE_BUS_CHILD";
    /// Set in the child that runs with a bus address that leads nowhere.
    const NO_BUS_CHILD: &str = "NZ_MPRIS_NO_BUS_CHILD";

    // --- the pure part: ids, URIs, metadata, discontinuity -----------------

    #[test]
    fn a_deezer_id_is_already_an_object_path_element() {
        // The common case (a Deezer number) stays readable in `playerctl
        // metadata mpris:trackid`.
        assert_eq!(
            track_object_path("3135556").as_str(),
            "/fr/neezer/track/3135556"
        );
    }

    #[test]
    fn sanitised_ids_stay_unique() {
        // A UUID is the other half of the ids the server hands out (uploads,
        // YouTube rows, podcast episodes), and its hyphens are not object path
        // characters. Underscore alone would merge `a-b` and `a_b`: the hash
        // keeps the two apart, which is what staleness detection needs.
        let uuid = track_object_path("7f3c8a1e-1111-2222-3333-444455556666");
        assert!(
            uuid.as_str()
                .starts_with("/fr/neezer/track/7f3c8a1e_1111_2222_3333_444455556666_"),
            "{uuid}"
        );

        let hyphen = track_object_path("a-b");
        let underscore = track_object_path("a_b");
        assert_ne!(hyphen, underscore, "{} vs {}", hyphen, underscore);
        assert_eq!(hyphen, track_object_path("a-b"), "stable across calls");
    }

    #[test]
    fn an_empty_id_still_gives_a_valid_path() {
        let path = track_object_path("");
        assert!(
            path.as_str().starts_with("/fr/neezer/track/unknown_"),
            "{path}"
        );
        assert_ne!(path, TrackId::NO_TRACK);
    }

    #[test]
    fn a_cover_path_becomes_a_percent_encoded_file_uri() {
        // Measured shapes: a space, a `#` that would otherwise truncate the URL,
        // and a non-ASCII byte that clients must receive as UTF-8 percent codes.
        assert_eq!(
            file_uri(Path::new("/home/léa/Covers/a b/3135556#1.jpg")),
            "file:///home/l%C3%A9a/Covers/a%20b/3135556%231.jpg"
        );
        assert_eq!(file_uri(Path::new("/tmp/x.jpg")), "file:///tmp/x.jpg");
        assert_eq!(file_uri(Path::new("relative.jpg")), "file:///relative.jpg");
    }

    #[test]
    fn metadata_carries_every_field_the_bar_reads() {
        let metadata = Metadata::new("3135556", "Balancing the Dark")
            .artist("Kaya Project")
            .artist("Irfan Quereshi")
            .album("Songs from the Inside")
            .length(312.4)
            .cover("/tmp/covers/3135556.jpg");

        let published = mpris_metadata(&metadata, track_object_path(&metadata.track_id));

        assert_eq!(published.title(), Some("Balancing the Dark"));
        assert_eq!(
            published.artist(),
            Some(vec!["Kaya Project".to_owned(), "Irfan Quereshi".to_owned()])
        );
        assert_eq!(published.album(), Some("Songs from the Inside"));
        assert_eq!(published.length(), Some(Time::from_micros(312_400_000)));
        assert_eq!(
            published.art_url().as_deref(),
            Some("file:///tmp/covers/3135556.jpg")
        );
        assert_eq!(
            published.trackid(),
            Some(track_object_path("3135556")),
            "mpris:trackid is derived from the track id"
        );
    }

    #[test]
    fn only_a_departure_from_the_extrapolation_is_a_seek() {
        // 4 ticks/s: a tick that arrives exactly, 0.25 s late or even 0.9 s late
        // (a busy frame) is not a seek.
        assert!(!is_discontinuity(
            expected_position_s(10.0, 0.25, true),
            10.25
        ));
        assert!(!is_discontinuity(
            expected_position_s(10.0, 0.9, true),
            10.9
        ));
        // A gesture is: 10 s forward, or a rewind to the start.
        assert!(is_discontinuity(
            expected_position_s(10.0, 0.25, true),
            20.0
        ));
        assert!(is_discontinuity(expected_position_s(10.0, 0.25, true), 0.0));
        // Paused, the extrapolation does not move: a position that changes
        // while paused is always a seek.
        assert_eq!(expected_position_s(42.0, 7.0, false), 42.0);
        assert!(is_discontinuity(
            expected_position_s(42.0, 7.0, false),
            40.0
        ));
    }

    #[test]
    fn the_playback_vocabulary_is_the_one_playerctl_reads() {
        assert_eq!(PlaybackStatus::Playing.as_str(), "Playing");
        assert_eq!(PlaybackStatus::Paused.as_str(), "Paused");
        assert_eq!(PlaybackStatus::Stopped.as_str(), "Stopped");
    }

    // --- the whole thing on a real bus -------------------------------------

    /// The acceptance test.
    ///
    /// It cannot run on the developer's own session bus: registering
    /// `org.mpris.MediaPlayer2.neezer` there would shadow the real player, and
    /// the name tells the client which app to read. So the test starts itself a
    /// private bus (`dbus-run-session`) and runs the real body in that child;
    /// the parent only checks the child's exit status, which keeps
    /// `cargo test` honest on a machine without a session (CI).
    #[test]
    fn the_mpris_contract_holds_on_a_real_bus() {
        if std::env::var_os(PRIVATE_BUS_CHILD).is_some() {
            private_bus_contract();
            return;
        }

        let binary = std::env::current_exe().expect("the test binary's path");
        let mut command = Command::new("dbus-run-session");
        command
            .arg("--")
            .arg(&binary)
            .args([
                "--exact",
                "tests::the_mpris_contract_holds_on_a_real_bus",
                "--nocapture",
            ])
            .env(PRIVATE_BUS_CHILD, "1")
            .env_remove("DBUS_SESSION_BUS_ADDRESS");
        run_child(command, Duration::from_secs(120));
    }

    /// MPRIS is optional to the app running (`docs/agents/desktop.md`): with no
    /// session bus at all, the player starts, plays and keeps its handle doing
    /// nothing instead of panicking or waiting.
    #[test]
    fn mpris_is_optional_to_the_app_running() {
        if std::env::var_os(NO_BUS_CHILD).is_some() {
            let (commands, _received) = mpsc::channel(4);
            let mpris = start_with_timeout("neezer", commands, Duration::from_millis(300));
            assert!(!mpris.is_serving(), "there is no session bus here");

            // Every one of these runs on each track and transport change: they
            // must be no-ops, never a panic.
            mpris.set_track(Metadata::new("3135556", "Balancing the Dark"));
            mpris.set_status(PlaybackStatus::Playing);
            mpris.set_position_s(1.0);
            mpris.set_volume(0.5);
            mpris.set_shuffle(true);
            mpris.set_loop(LoopStatus::Playlist);
            return;
        }

        let binary = std::env::current_exe().expect("the test binary's path");
        let mut command = Command::new(&binary);
        command
            .args([
                "--exact",
                "tests::mpris_is_optional_to_the_app_running",
                "--nocapture",
            ])
            .env(NO_BUS_CHILD, "1")
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/nonexistent/neezer-mpris-test",
            );
        run_child(command, Duration::from_secs(60));
    }

    /// Runs a child of this test binary and fails on a non-zero exit or a hang.
    fn run_child(mut command: Command, timeout: Duration) {
        let label = format!("{command:?}");
        let mut child = command.spawn().expect("the child test binary starts");
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait().expect("waiting for the child") {
                Some(status) => {
                    assert!(status.success(), "{label} failed: {status}");
                    return;
                }
                None if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                None => {
                    let _ = child.kill();
                    panic!("{label} did not finish in {timeout:?}");
                }
            }
        }
    }

    /// The test body, run inside the private session bus.
    fn private_bus_contract() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        runtime.block_on(async {
            let connection = Connection::session().await.expect("private session bus");

            let (commands, mut received) = mpsc::channel(64);
            let mpris = Mpris::start("neezer", commands);

            mpris.set_track(
                Metadata::new("3135556", "Balancing the Dark")
                    .artist("Kaya Project")
                    .album("Songs from the Inside")
                    .length(312.0)
                    .cover("/tmp/neezer-covers/3135556.jpg"),
            );
            mpris.set_status(PlaybackStatus::Playing);

            let properties = proxy(&connection, PROPERTIES_INTERFACE).await;
            let player = proxy(&connection, PLAYER_INTERFACE).await;

            // `playerctl metadata` reads the `Metadata` property: wait for the
            // update to land (an event, not a fixed sleep).
            let mut published = None;
            for _ in 0..500 {
                let all: HashMap<String, zbus::OwnedVariant> = properties
                    .call("GetAll", &(PLAYER_INTERFACE,))
                    .await
                    .expect("GetAll");
                let arrived = metadata_dict(&all)
                    .and_then(|dict| dict_text(dict, "xesam:title"))
                    .as_deref()
                    == Some("Balancing the Dark");
                if arrived {
                    published = Some(all);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let published = published.expect("the track reached the bus within 4 s");
            let metadata = metadata_dict(&published).expect("Metadata is a{sv}");
            assert_eq!(
                dict_text(metadata, "xesam:title").as_deref(),
                Some("Balancing the Dark")
            );
            assert_eq!(
                dict_text(metadata, "xesam:artist").as_deref(),
                Some("Kaya Project")
            );
            assert_eq!(
                dict_text(metadata, "xesam:album").as_deref(),
                Some("Songs from the Inside")
            );
            assert_eq!(
                dict_text(metadata, "mpris:artUrl").as_deref(),
                Some("file:///tmp/neezer-covers/3135556.jpg")
            );
            assert_eq!(
                dict_text(metadata, "mpris:trackid").as_deref(),
                Some("/fr/neezer/track/3135556")
            );
            assert_eq!(
                dict_number(metadata, "mpris:length"),
                Some(312_000_000),
                "mpris:length is microseconds"
            );
            assert_eq!(
                player
                    .get_property::<String>("PlaybackStatus")
                    .await
                    .expect("PlaybackStatus"),
                "Playing"
            );

            // `playerctl -p neezer metadata` and `playerctl -p neezer play-pause`:
            // the two commands the note names. playerctl is not on the CI image
            // (it is not a dependency of the app), so the D-Bus assertions above
            // and below carry the contract there; where it is installed, the
            // real client is exercised too.
            match Command::new("playerctl")
                .args(["-p", "neezer", "metadata"])
                .output()
            {
                Ok(output) if output.status.success() => {
                    let printed = String::from_utf8_lossy(&output.stdout);
                    for key in ["xesam:title", "xesam:artist", "xesam:album"] {
                        assert!(
                            printed.contains(key),
                            "playerctl metadata did not show {key}: {printed}"
                        );
                    }
                    assert!(
                        printed.contains("Balancing the Dark"),
                        "playerctl metadata lost the title: {printed}"
                    );

                    let status = Command::new("playerctl")
                        .args(["-p", "neezer", "play-pause"])
                        .status()
                        .expect("playerctl play-pause runs");
                    assert!(status.success(), "playerctl play-pause failed: {status}");
                    assert_eq!(
                        next_command(&mut received).await,
                        MprisCommand::PlayPause
                    );
                }
                Ok(output) => panic!(
                    "playerctl could not read the player: {} {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                ),
                Err(error) => eprintln!(
                    "playerctl not installed ({error}): the zbus assertions prove the same surface"
                ),
            }

            // Every method the media keys and a client can call reaches the app.
            for (method, expected) in [
                ("PlayPause", MprisCommand::PlayPause),
                ("Play", MprisCommand::Play),
                ("Pause", MprisCommand::Pause),
                ("Next", MprisCommand::Next),
                ("Previous", MprisCommand::Previous),
            ] {
                player
                    .call::<_, _, ()>(method, &())
                    .await
                    .unwrap_or_else(|error| panic!("{method}: {error}"));
                assert_eq!(next_command(&mut received).await, expected, "{method}");
            }

            player
                .call::<_, _, ()>("Seek", &(-2_500_000i64,))
                .await
                .expect("Seek");
            assert_eq!(
                next_command(&mut received).await,
                MprisCommand::Seek(-2_500_000)
            );

            let track_path = zbus::ObjectPath::try_from("/fr/neezer/track/3135556")
                .expect("track path");
            player
                .call::<_, _, ()>("SetPosition", &(track_path.clone(), 5_000_000i64))
                .await
                .expect("SetPosition");
            assert_eq!(
                next_command(&mut received).await,
                MprisCommand::SetPosition("3135556".to_owned(), 5_000_000)
            );

            // A client that clicked on the previous track must not move this one
            // (the spec: an id that does not match is stale). Proving it needs no
            // sleep: the next command received is the one sent after it.
            let stale = zbus::ObjectPath::try_from("/fr/neezer/track/9999999")
                .expect("stale path");
            player
                .call::<_, _, ()>("SetPosition", &(stale, 90_000_000i64))
                .await
                .expect("stale SetPosition");
            player
                .call::<_, _, ()>("PlayPause", &())
                .await
                .expect("PlayPause after the stale call");
            assert_eq!(
                next_command(&mut received).await,
                MprisCommand::PlayPause,
                "the stale SetPosition must not reach the app"
            );

            player
                .set_property("Volume", 0.42f64)
                .await
                .expect("Set Volume");
            assert_eq!(next_command(&mut received).await, MprisCommand::Volume(0.42));

            player
                .set_property("Shuffle", true)
                .await
                .expect("Set Shuffle");
            assert_eq!(next_command(&mut received).await, MprisCommand::Shuffle(true));

            player
                .set_property("LoopStatus", "Track")
                .await
                .expect("Set LoopStatus");
            assert_eq!(
                next_command(&mut received).await,
                MprisCommand::Loop(LoopStatus::Track)
            );

            let root = proxy(&connection, "org.mpris.MediaPlayer2").await;
            root.call::<_, _, ()>("Raise", &()).await.expect("Raise");
            assert_eq!(next_command(&mut received).await, MprisCommand::Raise);
            root.call::<_, _, ()>("Quit", &()).await.expect("Quit");
            assert_eq!(next_command(&mut received).await, MprisCommand::Quit);

            // --- no property signal per position tick ----------------------
            //
            // The app publishes `Position` at 4 Hz. MPRIS says that property
            // must never appear in `PropertiesChanged`, and `Seeked` is only
            // for a departure from the extrapolation. We watch both streams and
            // drive 10.0 s of playback.

            let changes = Arc::new(AtomicUsize::new(0));
            watch_properties_changed(connection.clone(), Arc::clone(&changes));

            let mut seeked = player
                .receive_signal("Seeked")
                .await
                .expect("Seeked signal stream");

            // First prove the watcher is live: a real seek is announced once,
            // with the new position, and read back from `Position`.
            changes.store(0, Ordering::Relaxed);
            mpris.set_position_s(42.5);

            let signal = tokio::time::timeout(Duration::from_secs(2), next_item(&mut seeked))
                .await
                .expect("Seeked within 2 s")
                .expect("signal stream open");
            let position: i64 = signal.body().deserialize().expect("Seeked body");
            assert_eq!(position, 42_500_000, "a seek of 42.5 s is announced once");
            assert_eq!(
                player.get_property::<i64>("Position").await.expect("Position"),
                42_500_000,
                "Position agrees with the Seeked signal"
            );

            // A volume change is one `PropertiesChanged`; it is the control that
            // proves the counter below can see signals at all.
            changes.store(0, Ordering::Relaxed);
            mpris.set_volume(0.31);
            let mut control = 0;
            for _ in 0..100 {
                control = changes.load(Ordering::Relaxed);
                if control > 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(control > 0, "the PropertiesChanged watcher saw nothing");

            // Now 10.0 s of playback: 100 ticks, 100 ms apart. The reported
            // position follows the clock, so a loaded CI machine whose tick
            // arrives late reports a late position too: that is what continuous
            // playback looks like, and the test measures the signals, not the
            // scheduler.
            changes.store(0, Ordering::Relaxed);
            let started = Instant::now();
            for _ in 0..100 {
                mpris.set_position_s(42.5 + started.elapsed().as_secs_f64());
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let played = started.elapsed().as_secs_f64();

            // Let the last updates land, then read the counters.
            tokio::time::sleep(Duration::from_millis(100)).await;
            let signals = changes.load(Ordering::Relaxed);
            println!(
                "MPRIS: {signals} PropertiesChanged and 0 extra Seeked over {played:.3} s of 4 Hz ticks"
            );
            assert!(
                signals <= 2,
                "{signals} property signals over {played:.3} s of playback: MPRIS forbids Position there"
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(300), next_item(&mut seeked))
                    .await
                    .is_err(),
                "continuous playback must not emit Seeked"
            );

            // Paused, the same position repeated is not a seek either.
            mpris.set_status(PlaybackStatus::Paused);
            tokio::time::sleep(Duration::from_millis(100)).await;
            for _ in 0..20 {
                mpris.set_position_s(52.5);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(300), next_item(&mut seeked))
                    .await
                    .is_err(),
                "a paused position does not move"
            );
        });
    }

    /// A proxy with its property cache off: this test READS properties to prove
    /// they change, and zbus's lazy cache would answer from the first `GetAll`.
    async fn proxy<'a>(connection: &'a Connection, interface: &'a str) -> Proxy<'a> {
        zbus::proxy::Builder::new(connection)
            .destination(SERVICE)
            .path(PLAYER_PATH)
            .interface(interface)
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
            .expect("proxy")
    }

    async fn next_command(received: &mut mpsc::Receiver<MprisCommand>) -> MprisCommand {
        tokio::time::timeout(Duration::from_secs(2), received.recv())
            .await
            .expect("the command reaches the app within 2 s")
            .expect("the command channel is open")
    }

    /// Counts every `PropertiesChanged` the player interface emits, on its own
    /// connection (the watcher needs a `'static` task and a rule of its own).
    fn watch_properties_changed(connection: Connection, counter: Arc<AtomicUsize>) {
        tokio::spawn(async move {
            let proxy = Proxy::new(&connection, SERVICE, PLAYER_PATH, PROPERTIES_INTERFACE)
                .await
                .expect("Properties proxy for the signal watcher");
            let mut signals = proxy
                .receive_signal_with_args("PropertiesChanged", &[(0, PLAYER_INTERFACE)])
                .await
                .expect("PropertiesChanged match rule");
            while next_item(&mut signals).await.is_some() {
                counter.fetch_add(1, Ordering::Relaxed);
            }
        });
    }

    /// The `a{sv}` behind the `Metadata` property of `GetAll`.
    fn metadata_dict(
        all: &HashMap<String, zbus::OwnedVariant>,
    ) -> Option<&zbus::Dict<'static, 'static>> {
        match &**all.get("Metadata")? {
            zbus::Variant::Dict(dict) => Some(dict),
            _ => None,
        }
    }

    /// One entry of that `a{sv}`.
    fn dict_value<'d, 'k, 'v>(
        dict: &'d zbus::Dict<'k, 'v>,
        key: &str,
    ) -> Option<&'d zbus::Variant<'v>> {
        let (_, value) = dict.iter().find(
            |(candidate, _)| matches!(candidate, zbus::Variant::Str(text) if text.as_str() == key),
        )?;
        Some(value)
    }

    /// An entry as text: a string, the object path of `mpris:trackid`, the
    /// variant an `a{sv}` wraps each value in, or the first entry of an array
    /// (`xesam:artist` is `as`).
    fn value_text(value: &zbus::Variant<'_>) -> Option<String> {
        match unwrap_variant(value) {
            zbus::Variant::Str(text) => Some(text.as_str().to_owned()),
            zbus::Variant::ObjectPath(path) => Some(path.as_str().to_owned()),
            zbus::Variant::Array(array) => array.inner().first().and_then(value_text),
            _ => None,
        }
    }

    /// An `a{sv}` wraps every value in a variant; the type is one level down.
    fn unwrap_variant<'a>(mut value: &'a zbus::Variant<'a>) -> &'a zbus::Variant<'a> {
        while let zbus::Variant::Variant(inner) = value {
            value = inner;
        }
        value
    }

    fn dict_text(dict: &zbus::Dict<'_, '_>, key: &str) -> Option<String> {
        value_text(dict_value(dict, key)?)
    }

    /// `mpris:length`, the one entry asserted as a number.
    fn dict_number(dict: &zbus::Dict<'_, '_>, key: &str) -> Option<i64> {
        match unwrap_variant(dict_value(dict, key)?) {
            zbus::Variant::I64(number) => Some(*number),
            _ => None,
        }
    }

    async fn next_item<S>(stream: &mut S) -> Option<S::Item>
    where
        S: Stream + Unpin,
    {
        poll_fn(|cx| Pin::new(&mut *stream).poll_next(cx)).await
    }
}
