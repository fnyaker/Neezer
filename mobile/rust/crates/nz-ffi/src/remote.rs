//! The remote-control FFI surface — lot MB19.
//!
//! Owned by **MB19** (docs/agents/mobile.md, "Module and file ownership"): the
//! records the remote screens draw, the listener that pushes a status on CHANGE,
//! and `RemoteApi`'s methods. No rule lives here — the two roles, the levels, the
//! request budget and the French wording are `nz_core::remote`'s (`nz-remote`'s
//! protocol under it, `SC5`), and this file only converts.
//!
//! # Why `remoteAttach(core)`
//!
//! `NzCore::remote()` answers `Arc::new(RemoteApi)` — MB0 declared the object as
//! a unit struct with its final SHAPE, so it carries no `Arc<Core>` the way
//! `PlayerApi::open(core)` does. The service therefore lives in
//! `nz_core::remote`'s ONE per-process slot, and the Kotlin side binds it once:
//!
//! ```kotlin
//! remoteAttach(NzBridge.core!!)      // :feature:remote's RemoteCore
//! val api = NzBridge.core!!.remote() // then every call answers
//! ```
//!
//! Before that call every method here answers `NzError::Unsupported`, whose
//! sentence names the cause (« la télécommande n'est pas reliée au noyau ») — a
//! screen that somehow reached it shows a cause and « Réessayer », never a blank
//! button (rule 3).
//!
//! The attach is also what hands the HOST side the phone's ONE player: this file
//! owns the adapter ([`FfiPlayer`]) because `PlayerApi` is this crate's object and
//! `nz-core` may not name it. It subscribes once to the player's own listener, so
//! a 120 ms publish step reads a cached snapshot and an atomic queue version
//! instead of cloning a 4 000-row queue.
//!
//! # What crosses the boundary
//!
//! Screen-shaped values only: a link row as the sheet prints it, the chip and the
//! banner's status, the loan's view (the owner's player, its queue behind a
//! [`ListHandle`] — 4 000 rows never cross whole). Nothing carries a token except
//! [`RemoteLinkRow::url`], which is what a person shares.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nz_core::player as core_player;
use nz_core::remote::{
    self as service, ControllerView, GrantView, HostNow, HostPlayer, LevelView, LinkView, NowView,
};
use nz_core::remote::{StatusView, Transport};
use nz_core::settings::LocalSettings;

use crate::core::NzCore;
use crate::errors::NzError;
use crate::lists::ListHandle;
use crate::player::{
    NowPlaying as FfiNowPlaying, PlayContext, PlayerApi, PlayerCommand, PlayerListener,
    PlayerSnapshot, TrackRef,
};

/// This device's role in a remote session.
#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum RemoteRole {
    /// Nobody is controlling this player, and this device drives nobody.
    None,
    /// Another device drives this one (the chip names who).
    Controlled { host_name: String },
    /// This device drives another one (the banner names whose).
    Controller { host_name: String },
}

/// One link this device made, as the sheet lists it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct RemoteLink {
    pub id: String,
    /// The `/rc/<token>` URL to share, built from the configured origin.
    pub url: String,
    /// `queue`, `read`, `full`, `admin`.
    pub level: String,
    /// `File d'attente`, `Lecture seule`, `Complet`, `Administrateur`.
    pub level_label: String,
    /// The owner's own name for it (« Salon »), empty when there is none.
    pub label: String,
    /// `« Complet · pour Léa »` — the row's title.
    pub title: String,
    /// `Sans limite`, `Expire dans 3 h`, `Expiré`.
    pub expiry: String,
    /// How many controllers hold it right now.
    pub controllers: u32,
}

impl From<LinkView> for RemoteLink {
    fn from(row: LinkView) -> Self {
        RemoteLink {
            id: row.id,
            url: row.url,
            level: row.level,
            level_label: row.level_label,
            label: row.label,
            title: row.title,
            expiry: row.expiry,
            controllers: row.controllers,
        }
    }
}

/// One controller driving THIS device, as the chip names it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct RemoteController {
    pub link: String,
    pub level: String,
    pub level_label: String,
    /// The name the owner gave that link, empty when there is none.
    pub label: String,
}

impl From<ControllerView> for RemoteController {
    fn from(controller: ControllerView) -> Self {
        RemoteController {
            link: controller.link,
            level: controller.level,
            level_label: controller.level_label,
            label: controller.label,
        }
    }
}

/// Where a loan stands, as the banner paints it (`RemoteBanner.svelte`).
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemotePhase {
    Connecting,
    Live,
    Reconnecting,
    Ended,
}

impl From<service::Phase> for RemotePhase {
    fn from(phase: service::Phase) -> Self {
        use service::Phase;
        match phase {
            Phase::Connecting => RemotePhase::Connecting,
            Phase::Live => RemotePhase::Live,
            Phase::Reconnecting => RemotePhase::Reconnecting,
            Phase::Ended => RemotePhase::Ended,
        }
    }
}

/// The grant a loan runs under: whose player this is, and what may be done to it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct RemoteGrant {
    pub level: String,
    pub level_label: String,
    pub owner: String,
    pub device: String,
    pub label: String,
    /// `mode.js#CAN_BROWSE/CAN_KEEP/CAN_ADMIN`: the UI hides what the level
    /// cannot do; the server enforces it on every request.
    pub can_browse: bool,
    pub can_keep: bool,
    pub can_admin: bool,
}

impl From<GrantView> for RemoteGrant {
    fn from(grant: GrantView) -> Self {
        RemoteGrant {
            level: grant.level,
            level_label: grant.level_label,
            owner: grant.owner,
            device: grant.device,
            label: grant.label,
            can_browse: grant.can_browse,
            can_keep: grant.can_keep,
            can_admin: grant.can_admin,
        }
    }
}

/// The chip and the banner, in one record: what both roles are doing right now,
/// with a version so a repaint happens once per CHANGE.
#[derive(uniffi::Record, Debug, Clone)]
pub struct RemoteStatus {
    pub role: RemoteRole,
    /// This device's player is answering a controller (the chip).
    pub host_active: bool,
    /// `« Contrôlé par Léa »` — the chip's first line.
    pub who: String,
    pub controllers: Vec<RemoteController>,
    /// The level line under the chip's first line, when exactly one drives.
    pub host_level: Option<String>,
    /// The last command number this device RAN (the `ack` a controller waits on).
    pub applied: u32,
    /// Why the host loops stopped, when they stopped on a failure.
    pub host_error: Option<String>,
    /// This device drives somebody else's player (the banner).
    pub phase: RemotePhase,
    pub online: bool,
    pub grant: Option<RemoteGrant>,
    /// `Connexion…`, `En direct · Complet`, `Reconnexion…` — the banner's line.
    pub line: String,
    /// The controller's last cause, in French.
    pub error: Option<String>,
    /// The account the server handed back when the loan ended, when it named one.
    pub restored: Option<String>,
    pub version: u64,
}

impl From<StatusView> for RemoteStatus {
    fn from(status: StatusView) -> Self {
        let role = if status.host_active || !status.controllers.is_empty() {
            RemoteRole::Controlled {
                host_name: status.who.clone(),
            }
        } else if status.grant.is_some() || status.phase != service::Phase::Connecting {
            RemoteRole::Controller {
                host_name: status
                    .grant
                    .as_ref()
                    .map(|grant| grant.owner.clone())
                    .unwrap_or_default(),
            }
        } else {
            RemoteRole::None
        };
        RemoteStatus {
            role,
            host_active: status.host_active,
            who: status.who,
            controllers: status.controllers.into_iter().map(Into::into).collect(),
            host_level: status.host_level,
            applied: status.applied,
            host_error: status.host_error,
            phase: status.phase.into(),
            online: status.online,
            grant: status.grant.map(Into::into),
            line: status.line,
            error: status.error,
            restored: status.restored.filter(|name| !name.is_empty()),
            version: status.version,
        }
    }
}

/// The owner's player, as this device drives it: the "Contrôle distant" screen.
#[derive(uniffi::Record, Debug, Clone)]
pub struct RemoteNow {
    pub track: Option<crate::lists::TrackRow>,
    pub playing: bool,
    pub index: i64,
    pub len: u32,
    pub position_s: f64,
    pub duration_s: f64,
    pub volume: f64,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: String,
    /// `playing`, `paused`, `idle` — the owner's own status word.
    pub status: String,
    /// The owner's player answers.
    pub online: bool,
    /// The version of the queue those rows belong to: the screen re-windows its
    /// handle only when this moves.
    pub queue_version: u64,
}

impl From<NowView> for RemoteNow {
    fn from(view: NowView) -> Self {
        RemoteNow {
            track: view.track.map(Into::into),
            playing: view.playing,
            index: view.index,
            len: view.len,
            position_s: view.position_s,
            duration_s: view.duration_s,
            volume: view.volume,
            muted: view.muted,
            shuffle: view.shuffle,
            repeat: view.repeat,
            status: view.status,
            online: view.online,
            queue_version: view.queue_version,
        }
    }
}

/// One level the sheet offers, with the words it prints.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct RemoteLevel {
    /// `queue`, `read`, `full`, `admin` — what `create_link` takes.
    pub id: String,
    pub label: String,
    /// The SPA's own one-line blurb under the level's name.
    pub blurb: String,
    pub short: String,
}

impl From<LevelView> for RemoteLevel {
    fn from(level: LevelView) -> Self {
        RemoteLevel {
            id: level.id,
            label: level.label,
            blurb: level.blurb,
            short: level.short,
        }
    }
}

/// One link duration the sheet offers (`RemoteSheet.svelte#DURATIONS`).
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct RemoteDuration {
    /// `1h`, `1d`, `7d`, `none` — what `create_link` takes.
    pub id: String,
    pub label: String,
}

/// This device's own settings, for the loan's Réglages (`mode.js`: a controller
/// keeps its settings apart).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct RemoteLocalSettings {
    pub theme: String,
    pub viz_mode: String,
    pub viz_eco: bool,
    pub quality: String,
}

impl From<LocalSettings> for RemoteLocalSettings {
    fn from(settings: LocalSettings) -> Self {
        RemoteLocalSettings {
            theme: settings.theme,
            viz_mode: settings.viz_mode,
            viz_eco: settings.eco,
            quality: settings.quality,
        }
    }
}

/// One transport command from the loan's screen (`commands.js#OPS`, cut to what
/// this screen offers).
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq)]
pub enum RemoteCommand {
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
    Seek { s: f64 },
    Jump { index: u32 },
    Volume { v: f64 },
    Mute,
    Shuffle,
    Repeat,
}

impl From<RemoteCommand> for Transport {
    fn from(command: RemoteCommand) -> Self {
        match command {
            RemoteCommand::Play => Transport::Play,
            RemoteCommand::Pause => Transport::Pause,
            RemoteCommand::Toggle => Transport::Toggle,
            RemoteCommand::Next => Transport::Next,
            RemoteCommand::Prev => Transport::Prev,
            RemoteCommand::Seek { s } => Transport::Seek(s),
            RemoteCommand::Jump { index } => Transport::Jump(index),
            RemoteCommand::Volume { v } => Transport::Volume(v),
            RemoteCommand::Mute => Transport::Mute,
            RemoteCommand::Shuffle => Transport::Shuffle,
            RemoteCommand::Repeat => Transport::Repeat,
        }
    }
}

/// Pushed on a Rust thread when the composed status CHANGED, never on a timer.
/// Kotlin hands it to a Flow and returns at once.
#[uniffi::export(callback_interface)]
pub trait RemoteListener: Send + Sync {
    fn on_status(&self, status: RemoteStatus);
    /// The loan ended (a cut, an expiry, the owner logging out). `restored` is
    /// the account the server handed back, when it named one.
    fn on_ended(&self, restored: Option<String>);
}

/// The Kotlin listener as the core's sink: it runs on the thread that fired it,
/// holds no lock, and returns at once.
struct Sink(Arc<dyn RemoteListener>);

impl service::StatusSink for Sink {
    fn status(&self, status: &StatusView) {
        self.0.on_status(status.clone().into());
        if status.phase == service::Phase::Ended {
            self.0
                .on_ended(status.restored.clone().filter(|name| !name.is_empty()));
        }
    }
}

/// The remote-control API — one per process (`NzCore::remote()`).
#[derive(uniffi::Object)]
pub struct RemoteApi;

/// The service, or the French cause a screen prints before `remoteAttach`.
fn service() -> Result<Arc<service::Remote>, NzError> {
    service::get().ok_or_else(|| NzError::Unsupported {
        feature: "remote:attach".to_owned(),
    })
}

/// The phone's player as the host side needs it: a cached snapshot (the player
/// pushes on CHANGE), an atomic queue version, and the one transport path.
struct FfiPlayer {
    player: Arc<PlayerApi>,
    /// The last snapshot the player pushed. Read by the 120 ms publish step, so
    /// that step locks nothing of the player's.
    snapshot: Mutex<PlayerSnapshot>,
    /// `PlayerListener::on_queue_changed`'s last number: what decides whether
    /// 4 000 rows are cloned.
    queue_version: AtomicU64,
    /// Whether a snapshot has ever arrived (before it, ask the player directly).
    seeded: AtomicU64,
}

impl FfiPlayer {
    fn new(player: Arc<PlayerApi>) -> Arc<FfiPlayer> {
        Arc::new(FfiPlayer {
            player,
            snapshot: Mutex::new(PlayerSnapshot {
                state: crate::player::PlayState::Stopped,
                current: None,
                index: None,
                len: 0,
                shuffle: false,
                repeat: crate::player::Repeat::Off,
                volume: 1.0,
                muted: false,
                streaming: false,
                sleep: None,
                error: None,
                fault: None,
                version: 0,
            }),
            queue_version: AtomicU64::new(0),
            seeded: AtomicU64::new(0),
        })
    }

    fn view(&self) -> PlayerSnapshot {
        if self.seeded.load(Ordering::Acquire) == 0 {
            // The first step of a fresh attach: the player's own answer is the
            // only truth, and the listener takes over from the next change.
            return self.player.snapshot();
        }
        lock(&self.snapshot).clone()
    }
}

/// The listener this adapter subscribes to the player. A struct of its own
/// because `PlayerApi::subscribe` takes the listener by value and the adapter
/// must keep reading what arrived.
struct Feed(Arc<FfiPlayer>);

impl PlayerListener for Feed {
    fn on_snapshot(&self, snapshot: PlayerSnapshot) {
        *lock(&self.0.snapshot) = snapshot;
        self.0.seeded.store(1, Ordering::Release);
    }

    fn on_queue_changed(&self, version: u64) {
        self.0.queue_version.store(version, Ordering::Release);
    }

    fn on_fault(&self, _cause: String) {
        // A fault on the owner's own player: the controller sees the player's
        // state go to `buffering`/`paused` with its cause in `fault`, which the
        // published state carries (`status`).
    }
}

impl HostPlayer for FfiPlayer {
    fn now(&self) -> HostNow {
        let snapshot = self.view();
        let position = self.player.position();
        HostNow {
            index: snapshot.index.map(i64::from).unwrap_or(-1),
            playing: matches!(snapshot.state, crate::player::PlayState::Playing),
            p: position.position_s.max(0.0),
            duration: position.duration_s.max(0.0),
            volume: f64::from(snapshot.volume),
            muted: snapshot.muted,
            shuffle: snapshot.shuffle,
            repeat: service::repeat_wire(core_repeat(snapshot.repeat)).to_owned(),
            status: match snapshot.state {
                crate::player::PlayState::Playing => "playing",
                crate::player::PlayState::Buffering => "loading",
                crate::player::PlayState::Paused => "paused",
                crate::player::PlayState::Stopped | crate::player::PlayState::Ended => "idle",
            }
            .to_owned(),
            current: snapshot
                .current
                .map(|now| row_of(now, snapshot.index.unwrap_or(0))),
        }
    }

    fn queue_rows(&self) -> Vec<nz_core::lists::Row> {
        self.player
            .queue_rows()
            .into_iter()
            .map(|track| nz_core::lists::Row {
                key: track.key,
                id: track.id,
                title: track.title,
                artists: track.artists,
                album: track.album,
                album_id: track.album_id,
                duration_s: track.duration_s,
                cover: track.cover,
                explicit: false,
                liked: track.liked,
                disliked: false,
                unavailable: false,
                downloaded: false,
                source: "deezer".to_owned(),
                position: 0,
                played_at: 0,
                added_at: 0,
            })
            .collect()
    }

    fn queue_version(&self) -> u64 {
        self.queue_version.load(Ordering::Acquire)
    }

    fn apply(&self, commands: &[core_player::Command]) {
        for command in commands {
            self.player.command(player_command(command.clone()), 0);
        }
    }
}

/// A track as the list row the wire publishes.
fn row_of(track: FfiNowPlaying, index: u32) -> nz_core::lists::Row {
    nz_core::lists::Row {
        key: track.key,
        id: track.id,
        title: track.title,
        artists: track.artists,
        album: track.album,
        album_id: track.album_id,
        duration_s: track.duration_s as u32,
        cover: track.cover,
        explicit: false,
        liked: track.liked,
        disliked: false,
        unavailable: false,
        downloaded: false,
        source: track.source,
        position: index,
        played_at: 0,
        added_at: 0,
    }
}

/// The FFI's repeat as the machine's (the wire spellings are the host's).
fn core_repeat(repeat: crate::player::Repeat) -> core_player::Repeat {
    match repeat {
        crate::player::Repeat::Off => core_player::Repeat::Off,
        crate::player::Repeat::One => core_player::Repeat::One,
        crate::player::Repeat::All => core_player::Repeat::All,
    }
}

/// `nz-core`'s command as this crate's enum — the mirror of `nz-ffi/src/player.rs`'s
/// `core_command`, kept here because that one is private and this file may not
/// edit it. Every variant is listed, so a command added there stops compiling
/// here instead of silently not crossing.
fn player_command(command: core_player::Command) -> PlayerCommand {
    match command {
        core_player::Command::Play => PlayerCommand::Play,
        core_player::Command::Pause => PlayerCommand::Pause,
        core_player::Command::Toggle => PlayerCommand::Toggle,
        core_player::Command::Next => PlayerCommand::Next,
        core_player::Command::Prev => PlayerCommand::Prev,
        core_player::Command::Seek { s } => PlayerCommand::Seek { s },
        core_player::Command::Jump { index, key } => PlayerCommand::Jump { index, key },
        core_player::Command::PlayQueue {
            tracks,
            start,
            context,
        } => PlayerCommand::PlayQueue {
            tracks: tracks.into_iter().map(track_ref).collect(),
            start,
            context: context.map(|context| PlayContext {
                kind: context.kind,
                id: context.id.unwrap_or_default(),
            }),
        },
        core_player::Command::PlayEntity { kind, id, shuffle } => {
            PlayerCommand::PlayEntity { kind, id, shuffle }
        }
        core_player::Command::AddToQueue { tracks } => PlayerCommand::AddToQueue {
            tracks: tracks.into_iter().map(track_ref).collect(),
        },
        core_player::Command::PlayNext { tracks } => PlayerCommand::PlayNext {
            tracks: tracks.into_iter().map(track_ref).collect(),
        },
        core_player::Command::Remove { index, key } => PlayerCommand::Remove { index, key },
        core_player::Command::Move { from, to } => PlayerCommand::Move { from, to },
        core_player::Command::ClearUpcoming => PlayerCommand::ClearUpcoming,
        core_player::Command::SetShuffle { on } => PlayerCommand::SetShuffle { on },
        core_player::Command::CycleRepeat => PlayerCommand::CycleRepeat,
        core_player::Command::SetVolume { v } => PlayerCommand::SetVolume { v },
        core_player::Command::ToggleMute => PlayerCommand::ToggleMute,
        core_player::Command::StartRadio { kind, id } => PlayerCommand::StartRadio { kind, id },
        core_player::Command::StartFlow { style } => PlayerCommand::StartFlow { style },
    }
}

/// A core track reference as the FFI's own record.
fn track_ref(track: core_player::TrackRef) -> TrackRef {
    TrackRef {
        key: track.key,
        id: track.id,
        title: track.title,
        artists: track.artists,
        album: track.album,
        album_id: track.album_id,
        artist_id: track.artist_id,
        cover: track.cover,
        duration_s: track.duration_s,
        episode: track.episode,
        liked: track.liked,
    }
}

/// Bind the process's remote-control service AND the phone's player to it.
///
/// One call from `:feature:remote`'s own door (`RemoteCore`), which every call of
/// this file goes through. It resumes what the session already is: a device that
/// was lent before asks the server ONCE whether a link still points here; a
/// session that is a loan starts driving. Nothing else is sent.
#[uniffi::export(async_runtime = "tokio")]
pub async fn remote_attach(core: Arc<NzCore>) -> Result<(), NzError> {
    let service = service::attach(core.core());
    if !service.has_player() {
        let player = core.player();
        let adapter = FfiPlayer::new(Arc::clone(&player));
        player.subscribe(Box::new(Feed(Arc::clone(&adapter))));
        service.attach_player(adapter);
    }
    // This device's id, the one a link points at: created on first attach, so a
    // link made later names a phone the server already knows.
    let _ = service.device();
    if service.grant().is_some() {
        service.start_loan().await.map_err(NzError::from)?;
    } else {
        service.resume_host().await.map_err(NzError::from)?;
    }
    Ok(())
}

/// Detach: stop both roles and join the service's thread. The app never needs it
/// (Android kills the process); a test that rebuilds a core does.
#[uniffi::export]
pub fn remote_shutdown() {
    service::shutdown();
}

#[uniffi::export]
impl RemoteApi {
    /// The chip and the banner, composed. Synchronous and cheap: the screen
    /// reads it once per pushed change, and on the repaint that follows.
    pub fn status(&self) -> Result<RemoteStatus, NzError> {
        Ok(service()?.status().into())
    }

    /// The loan's view of the owner's player. The position inside it is the
    /// owner's published line extrapolated — see [`RemoteApi::position`] for the
    /// per-frame read.
    pub fn now(&self) -> Result<RemoteNow, NzError> {
        Ok(service()?.now().into())
    }

    /// Where the owner's track is right now: the HOT read a progress bar calls
    /// each visible frame (an atomic plus the published line), and NOTHING while
    /// the screen is off.
    pub fn position(&self) -> Result<f64, NzError> {
        Ok(service()?.position())
    }

    /// The owner's queue, windowed (`ListHandle`): 4 000 rows never cross whole.
    /// Ask again when [`RemoteNow::queue_version`] moved.
    pub fn queue(&self) -> Result<Arc<ListHandle>, NzError> {
        let service = service()?;
        let core = Arc::clone(service.core());
        Ok(crate::lists::open_rows(&core, service.queue_rows()))
    }

    /// The links this device made, as last read — memory only: a token is a
    /// capability, and the SPA keeps no offline copy either.
    pub fn cached_links(&self) -> Result<Vec<RemoteLink>, NzError> {
        Ok(service()?
            .cached_links()
            .into_iter()
            .map(RemoteLink::from)
            .collect())
    }

    /// `GET /api/remote/links?device=` — every live link onto this device. ONE
    /// request on the `User` lane, counted under `remote`.
    pub async fn links(&self) -> Result<Vec<RemoteLink>, NzError> {
        let rows = service()?.links().await.map_err(NzError::from)?;
        Ok(rows.into_iter().map(RemoteLink::from).collect())
    }

    /// Makes a claim link and returns it, so the sheet can show its QR code and
    /// share it. `duration` is one of [`RemoteApi::durations`]' ids.
    pub async fn create_link(
        &self,
        level: String,
        duration: String,
        label: Option<String>,
    ) -> Result<RemoteLink, NzError> {
        let row = service()?
            .create_link(&level, &duration, label.as_deref())
            .await
            .map_err(NzError::from)?;
        Ok(row.into())
    }

    /// Cuts ONE link: the controller it let in ends on its very next request.
    pub async fn revoke_link(&self, id: String) -> Result<(), NzError> {
        service()?.revoke_link(&id).await.map_err(NzError::from)
    }

    /// « Tout couper » — every link onto this device.
    pub async fn revoke_all(&self) -> Result<(), NzError> {
        service()?.revoke_all().await.map_err(NzError::from)
    }

    /// Claims `/rc/<token>`: from now on this device controls that player.
    pub async fn claim(&self, token: String) -> Result<RemoteGrant, NzError> {
        let grant = service()?.claim(&token).await.map_err(NzError::from)?;
        Ok(grant.into())
    }

    /// Starts driving the owner's player (after a claim, or at the attach of a
    /// session that is a loan).
    pub async fn start_loan(&self) -> Result<(), NzError> {
        service()?.start_loan().await.map_err(NzError::from)
    }

    /// One step of the loan's traffic, for a test that owns the events: read
    /// `/api/remote/state`, adopt it, follow the line. The app's own loop does
    /// this itself, several times a second.
    pub async fn step_loan(&self) -> Result<(), NzError> {
        service()?.step_loan().await.map_err(NzError::from)
    }

    /// Hands the loan back. Returns the account the server restored, when it
    /// named one.
    pub async fn leave(&self) -> Result<Option<String>, NzError> {
        service()?.leave().await.map_err(NzError::from)
    }

    /// One transport command for the owner's player: optimistic (the view changes
    /// in the same frame), then POSTed. A level that refuses it changes NOTHING
    /// and its French sentence is the error's.
    pub async fn send(&self, command: RemoteCommand) -> Result<(), NzError> {
        service()?.send(command.into()).await.map_err(NzError::from)
    }

    /// Starts answering a controller: called right after a link is made, and
    /// again at the attach of a device that was lent before.
    pub async fn start_host(&self) -> Result<(), NzError> {
        service()?.start_host().await.map_err(NzError::from)
    }

    /// Asks the server once whether a link still points here. `false` when this
    /// device was never lent — the case that must cost ZERO requests.
    pub async fn resume_host(&self) -> Result<bool, NzError> {
        service()?.resume_host().await.map_err(NzError::from)
    }

    /// Stops answering (a logout, the person's own switch).
    pub fn stop_host(&self) -> Result<(), NzError> {
        service()?.stop_host();
        Ok(())
    }

    /// Whether this phone was ever lent: what decides whether a launch probes.
    pub fn hosted_before(&self) -> Result<bool, NzError> {
        Ok(service()?.hosted_before())
    }

    /// The levels the sheet offers, `admin` only to an administrator.
    pub fn levels(&self) -> Result<Vec<RemoteLevel>, NzError> {
        Ok(service()?
            .levels()
            .into_iter()
            .map(RemoteLevel::from)
            .collect())
    }

    /// The four durations the sheet offers (`RemoteSheet.svelte#DURATIONS`).
    pub fn durations(&self) -> Result<Vec<RemoteDuration>, NzError> {
        Ok(service::DURATIONS
            .iter()
            .map(|id| RemoteDuration {
                id: (*id).to_owned(),
                label: duration_label(id).to_owned(),
            })
            .collect())
    }

    /// This device's own settings, resolved: the loan's Réglages keeps them apart
    /// from the owner's (`mode.js`).
    pub fn local_settings(&self) -> Result<RemoteLocalSettings, NzError> {
        Ok(service()?.local_settings().into())
    }

    /// Registers a listener: a status is pushed on every CHANGE (a controller
    /// arrived, a tap was acked, the loan ended), never on a timer.
    pub fn subscribe(&self, listener: Box<dyn RemoteListener>) -> Result<(), NzError> {
        service()?.subscribe(Arc::new(Sink(Arc::from(listener))));
        Ok(())
    }
}

/// `RemoteSheet.svelte#DURATIONS`'s labels.
fn duration_label(id: &str) -> &'static str {
    match id {
        "1h" => "1 heure",
        "1d" => "1 jour",
        "7d" => "7 jours",
        _ => "Sans limite",
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire's repeat spellings and the machine's are one vocabulary: a
    /// mismatch would make a controller's repeat button do nothing visible.
    #[test]
    fn repeat_crosses_as_the_wire_spells_it() {
        for (machine, wire) in [
            (core_player::Repeat::Off, "off"),
            (core_player::Repeat::All, "all"),
            (core_player::Repeat::One, "one"),
        ] {
            assert_eq!(service::repeat_wire(machine), wire);
            assert_eq!(
                core_repeat(crate::player::Repeat::Off),
                core_player::Repeat::Off
            );
        }
    }

    /// Every duration the sheet offers has a French label and a TTL the core
    /// accepts — a chip with no label would print an empty pill.
    #[test]
    fn every_duration_has_a_label_and_a_ttl() {
        for id in service::DURATIONS {
            assert!(service::ttl_for(id).is_some(), "{id} a un TTL");
            assert!(!duration_label(id).is_empty(), "{id} a un libellé");
        }
        assert_eq!(service::ttl_for("1h"), Some(Some(3600)));
        assert_eq!(service::ttl_for("none"), Some(None));
        assert_eq!(service::ttl_for("2h"), None);
    }

    /// A `PlayQueue` crosses with its rows, so a controller's "play this album"
    /// is not a queue of phantom rows.
    #[test]
    fn a_queue_command_carries_its_rows() {
        let command = core_player::Command::PlayQueue {
            tracks: vec![core_player::TrackRef {
                key: "row-1".to_owned(),
                id: "3135556".to_owned(),
                title: "Titre".to_owned(),
                artists: "Artiste".to_owned(),
                duration_s: 200,
                ..core_player::TrackRef::default()
            }],
            start: 0,
            context: None,
        };
        match player_command(command) {
            PlayerCommand::PlayQueue { tracks, .. } => {
                assert_eq!(tracks.len(), 1);
                assert_eq!(tracks[0].title, "Titre");
                assert_eq!(tracks[0].id, "3135556");
            }
            other => panic!("une file attendue, reçu {other:?}"),
        }
    }
}
