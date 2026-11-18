//! `nz-lan` — the LAN exchange between the person's own devices and the devices
//! of the friends they trust (docs/agents/lan-nodes.md).
//!
//! The person is on a trip. Their PC has the files and their phone has little
//! room; on the same Wi-Fi the two apps find each other, prove who they are
//! against the server's trust chain, and move audio between themselves — with
//! no server in the path once the device is enrolled.
//!
//! # What lives where
//!
//! - [`types`] — the identity of a device and of a person, the tolerated
//!   qualities, the source of a track, and [`LanError`].
//! - [`proto`] — every wire frame (requests, responses, the control stream), in
//!   its final shape: a later lot changes a body, never a frame.
//! - [`cred`] / [`keys`] — the server's signed credential, roster and revocation
//!   list, and the device's own Ed25519 key file.
//! - [`codec`] / [`transport`] / [`auth`] — QUIC, the exporter-bound proof
//!   exchange and the connection table.
//! - [`discovery`] / [`discovery_mdns`] — DNS-SD, the platform feed, the
//!   last-known addresses.
//! - [`store`] / [`index`] / [`hashing`] / [`scope`] / [`serve`] — what this node
//!   has, what a peer may see of it, and how it answers.
//! - [`peers`] / [`decide`] / [`fetch`] — the other direction: the replicated
//!   inventories, the instant source decision, the transfer.
//! - [`enroll`] / [`gossip`] / [`settings`] — the server round trip, the trust
//!   exchange between peers, the per-device settings.
//! - [`service`] — [`Lan`], the one handle the apps hold.
//! - [`opener`] / [`downloader`] / [`sets`] / [`import`] — the adapters that
//!   make playback and downloads go to a peer first.
//!
//! # LN0's shape, and why it exists
//!
//! Every declaration above is written in its FINAL shape before any body is
//! filled, so that no later lot edits a manifest, this module list or a shared
//! type: a lot replaces the body of a function in ITS OWN file, and stops and
//! asks if a contract turns out wrong rather than forking a parallel one.
//!
//! The bodies LN0 leaves answer `Err(`[`LanError::Unsupported`]`)`, an empty
//! collection or `None`; nothing in the app calls this crate yet. The one thing
//! that is already true and tested here is the shape of the wire frames
//! ([`proto`]) and of the shared types ([`types`]), because those are what the
//! other lots compile against.
//!
//! # Rule 1, from the first line
//!
//! A transfer carries an [`Urgency`] and takes `nz-api`'s own
//! [`nz_api::lanes::Gates`] ([`nz_api::Api::gates`]) for `Bulk` work: one Bulk
//! slot across the server AND the LAN, so a playlist pulled from a peer waits
//! behind the tapped track exactly like a download does.

pub mod auth;
pub mod codec;
pub mod cred;
pub mod decide;
pub mod discovery;
#[cfg(feature = "mdns")]
pub mod discovery_mdns;
pub mod downloader;
pub mod enroll;
pub mod fetch;
pub mod gossip;
pub mod hashing;
pub mod import;
pub mod index;
pub mod keys;
pub mod opener;
pub mod peers;
pub mod proto;
pub mod scope;
pub mod serve;
pub mod service;
pub mod sets;
pub mod settings;
pub mod store;
pub mod transport;
pub mod types;

pub use cred::{
    CredError, Credential, Revocations, Role, Roster, Share, SharedPlaylist, TrustKeys, Truster,
    auth_message,
};
pub use keys::DeviceKey;
pub use proto::{Control, Request, Response};
pub use service::{
    Activity, Direction, DiscoveryKind, Lan, LanConfig, LanSettings, LanSnapshot, LanState,
    PeerSet, PeerView, TransferView,
};
pub use store::Store;
pub use transport::{Endpoint, Identity, PeerConn, PeerInfo, SharedTrust, TrustState};
pub use types::{
    DevicePub, Kid, LanError, Relation, Source, Subject, Tolerance, Urgency, WireQuality,
};
