//! What the remote-control screens draw, screen-shaped.
//!
//! Every type here is built from `nz-remote`'s own (`SC5`) or from this core's
//! player, and carries exactly what a row prints — never a server payload and
//! never a token. `nz-ffi`'s `remote.rs` converts these to UniFFI records and
//! adds nothing: a change of wording happens in ONE place (`nz-remote`'s
//! `level_label`, `expiry_label`, `HostStatus::who`), so the phone cannot say
//! something the desktop says differently.

use nz_remote::controller::{Grant, Phase};
use nz_remote::host::LinkRow;

use crate::lists::Row;

/// One link this device made, as the sheet lists it (`RemoteSheet.svelte`'s
/// `links` section): the URL the QR carries, the level it lends, who claimed it
/// and when it expires — all already said in French.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkView {
    pub id: String,
    pub url: String,
    pub level: String,
    /// `File d'attente`, `Lecture seule`, `Complet`, `Administrateur`.
    pub level_label: String,
    /// The owner's name for it (« Salon »), empty when there is none.
    pub label: String,
    /// `« Complet · pour Léa »` — the row's own title (`LinkRow::title`).
    pub title: String,
    /// `Sans limite`, `Expire dans 3 h`, `Expiré`.
    pub expiry: String,
    /// How many controllers hold it right now.
    pub controllers: u32,
}

impl From<LinkRow> for LinkView {
    fn from(row: LinkRow) -> Self {
        LinkView {
            level_label: nz_remote::host::level_label(&row.level).to_owned(),
            title: row.title(),
            id: row.id,
            url: row.url,
            level: row.level,
            label: row.label,
            expiry: row.expiry,
            controllers: row.controllers,
        }
    }
}

/// One controller driving THIS device, as the chip names it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ControllerView {
    pub link: String,
    pub level: String,
    pub level_label: String,
    /// The name the owner gave that link (« Léa »), empty when there is none.
    pub label: String,
}

impl From<nz_remote::host::Controller> for ControllerView {
    fn from(controller: nz_remote::host::Controller) -> Self {
        ControllerView {
            level_label: controller.level_label().to_owned(),
            link: controller.link,
            level: controller.level,
            label: controller.label,
        }
    }
}

/// The grant a loan runs under: whose player this is, and what may be done to it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrantView {
    pub level: String,
    pub level_label: String,
    pub owner: String,
    pub device: String,
    pub label: String,
    /// The three questions `mode.js#CAN_BROWSE/CAN_KEEP/CAN_ADMIN` answer: the
    /// UI hides what the level cannot do, the server enforces it.
    pub can_browse: bool,
    pub can_keep: bool,
    pub can_admin: bool,
}

impl From<&Grant> for GrantView {
    fn from(grant: &Grant) -> Self {
        use nz_model::RemoteLevel;
        GrantView {
            level: grant.level.wire().to_owned(),
            level_label: grant.level_label().to_owned(),
            owner: grant.owner.clone(),
            device: grant.device.clone(),
            label: grant.label.clone(),
            can_browse: grant.level >= RemoteLevel::Read,
            can_keep: grant.level >= RemoteLevel::Full,
            can_admin: grant.level >= RemoteLevel::Admin,
        }
    }
}

/// Everything the remote screens paint at once (`RemoteHostChip` and
/// `RemoteBanner`), with a version so a repaint happens once per CHANGE.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusView {
    /// This device's player is answering a controller (the chip).
    pub host_active: bool,
    /// `« Contrôlé par Léa »`, `« Contrôlé par 2 appareils »` — the chip's line.
    pub who: String,
    pub controllers: Vec<ControllerView>,
    /// The level line the chip prints when exactly one controller holds it.
    pub host_level: Option<String>,
    /// The last command number this device RAN (the `ack` a controller waits on).
    pub applied: u32,
    /// Why the host loops stopped, when they stopped on a failure.
    pub host_error: Option<String>,
    /// This device drives somebody else's player (the banner).
    pub phase: Phase,
    pub online: bool,
    pub grant: Option<GrantView>,
    /// The controller's last cause, in French (`nz_error_text` prints it).
    pub error: Option<String>,
    /// `Connexion…`, `En direct · Complet`, `Reconnexion…` — the banner's line,
    /// `nz-remote`'s own wording, empty when this device is not a controller.
    pub line: String,
    /// The account the server handed back when the loan ended, if it named one.
    pub restored: Option<String>,
    /// Bumped on every change.
    pub version: u64,
}

impl Default for StatusView {
    fn default() -> Self {
        StatusView {
            host_active: false,
            who: String::new(),
            controllers: Vec::new(),
            host_level: None,
            applied: 0,
            host_error: None,
            phase: Phase::Connecting,
            online: false,
            grant: None,
            error: None,
            line: String::new(),
            restored: None,
            version: 0,
        }
    }
}

impl StatusView {
    /// Nothing is happening: no loan, nobody driving this player.
    pub fn idle(&self) -> bool {
        !self.host_active && self.phase == Phase::Connecting && self.grant.is_none()
    }
}

/// The owner's player, as this device drives it: the "Contrôle distant" screen's
/// data (`player_bar` + the transport, pointed at another player).
///
/// The position is the owner's published line extrapolated on the server's clock
/// (`nz-remote`'s `Anchor`), which is why it is read per frame while the screen
/// shows a progress bar and never pushed.
#[derive(Debug, Clone, Default)]
pub struct NowView {
    /// The track that is playing there, as a list row draws it.
    pub track: Option<Row>,
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
    /// The version of the queue those rows belong to (`/remote/queue`'s `qv`):
    /// the screen re-windows its handle only when this moves.
    pub queue_version: u64,
}
