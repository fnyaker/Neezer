//! The listen party across the boundary: host one from the queue that is
//! playing, join one by its link, and paint what the room is doing.
//!
//! Owned by **MB20** (`docs/agents/mobile.md`, "Screen lots, wave 3"): the note
//! pins "host, join `/party/<id>`, leave, members", and the guest's sync reads the
//! ENGINE's output clock (SC2's latency), never a wall clock. The protocol, the
//! line, the planner and both loops are `nz-party`'s (SC6); this crate only
//! converts, and `nz-core::party` is what starts them over the phone's own player.
//!
//! Three rules of the boundary are visible here:
//!
//! - **Screen-shaped records, not server JSON.** A row carries what the screens
//!   DRAW (a title, an artist, a cover route, a duration) and the sentences the
//!   shared crate wrote — never `serde_json::Value`, never a percentage of a
//!   buffer only the engine understands.
//! - **The playhead is a HOT read** (`position_s`), called by the screen each
//!   frame while its bar is on screen. Rust pushes a state only on CHANGE
//!   (`PartyListener`), never on a timer, so a sleeping screen costs nothing.
//! - **`PartyApi` holds no state.** MB0 declared it as a unit struct built by
//!   `NzCore::party()`, so it cannot carry `Arc<Core>`: `attach(core, player)`
//!   binds the process's ONE service (MB15's `offlineAttach` seam), and every
//!   method answers `NzError::Unsupported { feature: "party:attach" }` before it —
//!   a cause a screen prints, not a panic. Because the service is the process's,
//!   two `PartyApi` values are the same party, which is what "one party per
//!   device" means.

use std::sync::{Arc, Mutex, OnceLock};

use crate::core::NzCore;
use crate::errors::NzError;
use crate::player::PlayerApi;

/// What this device is in the party.
#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum PartyRole {
    None,
    Host {
        id: String,
        /// The address the guests open (`<serveur>/party/<id>`).
        link: String,
        /// The loops are running.
        active: bool,
        /// The server forgot the party and the id could not be taken back: the
        /// link in everybody's hands is dead.
        lost: bool,
    },
    Guest {
        id: String,
        host_name: String,
    },
}

/// Where the party screen stands (`Party.svelte`'s phases, plus the two a phone
/// needs: an older server, and a failure with a cause instead of a spinner).
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartyPhase {
    Idle,
    Landing,
    Joining,
    Live,
    Ended,
    Left,
    Missing,
    Unsupported,
    Failed,
}

/// One track, as the two screens draw it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PartyTrackData {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_s: f64,
    pub is_episode: bool,
    /// `/api/party/<pid>/cover/<id>`: a guest has no session, so its art only
    /// ever comes through the party.
    pub cover: Option<String>,
}

/// One member, as a list draws it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PartyMemberData {
    pub name: String,
    /// `host` or `guest`.
    pub role: String,
    /// The guest's own sync estimate, milliseconds (`?q=`), rounded as the wire
    /// rounds it.
    pub latency_ms: u32,
    /// The line under the name (`Listener::status`): « En phase · ±0.4 ms »,
    /// « Chargement… », « En pause », « Connecté ».
    pub status: String,
    /// The letter an avatar draws.
    pub initial: String,
    pub synced: bool,
    pub waiting: bool,
}

/// The party, as one screen paints it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PartyStateData {
    pub role: PartyRole,
    pub phase: PartyPhase,
    pub host: String,
    pub track: Option<PartyTrackData>,
    pub next: Option<PartyTrackData>,
    /// The host is still there (its heartbeat has not timed out).
    pub live: bool,
    pub playing: bool,
    pub loading: bool,
    /// The playhead at the instant of the read: for the bar, and re-read hot
    /// from `position_s` while the screen draws.
    pub position_s: f64,
    pub duration_s: f64,
    /// The status line, in French, and its tone: `ok`, `wait`, `warn`, `error`.
    pub status: String,
    pub tone: String,
    /// This device's clock estimate: `(spread, rtt)`, milliseconds.
    pub clock_spread_ms: Option<f64>,
    pub clock_rtt_ms: Option<f64>,
    pub offline: bool,
    pub members: Vec<PartyMemberData>,
    /// How many people are listening (a guest draws the count, never the names).
    pub listeners: u32,
    pub link: String,
    pub volume: f64,
    pub trim_ms: f64,
    /// The latency the OS itself reports, when it does (the *Décalage* card only
    /// offers the Bluetooth shortcut when this is under 100 ms).
    pub os_latency_ms: Option<i64>,
    pub error: Option<String>,
    /// The landing card's invitation, the live header, the next row and the big
    /// line — the shared crate's wording and the SPA's templates, so a Kotlin
    /// screen prints them and never rebuilds one.
    pub invite_line: String,
    pub live_line: String,
    pub next_line: Option<String>,
    pub title_line: String,
    /// « Personne à l'écoute », « 1 personne à l'écoute », …
    pub audience: String,
    pub phase_title: Option<String>,
    pub phase_body: Option<String>,
    /// The bar's share, 0…100.
    pub progress: f64,
    pub version: u64,
}

impl PartyStateData {
    fn of(view: &nz_core::party::PartyView) -> PartyStateData {
        PartyStateData {
            role: role_of(&view.role),
            phase: phase_of(view.phase),
            host: view.host.clone(),
            track: view.track.as_ref().map(track_of),
            next: view.next.as_ref().map(track_of),
            live: view.live,
            playing: view.playing,
            loading: view.loading,
            position_s: view.position_s,
            duration_s: view.duration_s,
            status: view.status_text.clone(),
            tone: tone_of(view),
            clock_spread_ms: view.clock.map(|(spread, _)| spread),
            clock_rtt_ms: view.clock.map(|(_, rtt)| rtt),
            offline: view.offline,
            members: view.members.iter().map(member_of).collect(),
            listeners: view.listeners,
            // The link lives in the role (that is where the screen reads it);
            // the flat field is the sheet's convenience and may lag a host that
            // was adopted without one.
            link: view.role.host_link().unwrap_or_else(|| view.link.clone()),
            volume: view.volume,
            trim_ms: view.trim_ms,
            os_latency_ms: view.os_latency_ms,
            error: view.error.clone(),
            invite_line: view.invite_line(),
            live_line: view.live_line(),
            next_line: view.next_line(),
            title_line: view.title_line(),
            audience: audience_of(view),
            phase_title: view.phase_title().map(str::to_owned),
            phase_body: view.phase_body().map(str::to_owned),
            progress: view.progress(),
            version: view.version,
        }
    }
}

fn track_of(track: &nz_core::party::TrackView) -> PartyTrackData {
    PartyTrackData {
        id: track.id.clone(),
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        duration_s: track.duration_s,
        is_episode: track.podcast,
        cover: track.cover.clone(),
    }
}

fn member_of(member: &nz_core::party::Member) -> PartyMemberData {
    PartyMemberData {
        name: member.name.clone(),
        role: member.role.clone(),
        latency_ms: member.q.map(|q| q.max(0.0).round() as u32).unwrap_or(0),
        status: member.status.clone(),
        initial: member.initial(),
        synced: member.synced(),
        waiting: member.waiting(),
    }
}

fn role_of(role: &nz_core::party::Role) -> PartyRole {
    match role {
        nz_core::party::Role::None => PartyRole::None,
        nz_core::party::Role::Host {
            id,
            link,
            active,
            lost,
        } => PartyRole::Host {
            id: id.clone(),
            link: link.clone(),
            active: *active,
            lost: *lost,
        },
        nz_core::party::Role::Guest { id, host_name } => PartyRole::Guest {
            id: id.clone(),
            host_name: host_name.clone(),
        },
    }
}

fn phase_of(phase: nz_core::party::Phase) -> PartyPhase {
    match phase {
        nz_core::party::Phase::Idle => PartyPhase::Idle,
        nz_core::party::Phase::Landing => PartyPhase::Landing,
        nz_core::party::Phase::Joining => PartyPhase::Joining,
        nz_core::party::Phase::Live => PartyPhase::Live,
        nz_core::party::Phase::Ended => PartyPhase::Ended,
        nz_core::party::Phase::Left => PartyPhase::Left,
        nz_core::party::Phase::Missing => PartyPhase::Missing,
        nz_core::party::Phase::Unsupported => PartyPhase::Unsupported,
        nz_core::party::Phase::Failed => PartyPhase::Failed,
    }
}

fn tone_of(view: &nz_core::party::PartyView) -> String {
    if view.error.is_some() {
        "error".to_owned()
    } else if view.status_warn {
        "warn".to_owned()
    } else if view.status_ok {
        "ok".to_owned()
    } else {
        "wait".to_owned()
    }
}

/// The sheet's audience line, `nz-party`'s own words.
fn audience_of(view: &nz_core::party::PartyView) -> String {
    match view.members.len() {
        0 => "Personne à l'écoute".to_owned(),
        1 => "1 personne à l'écoute".to_owned(),
        count => format!("{count} personnes à l'écoute"),
    }
}

/// The listener's three values, as one record.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PartyPrefs {
    pub name: String,
    pub volume: f64,
    pub trim_ms: f64,
}

/// What a screen hears: one call per CHANGE (never per frame, never on a timer).
#[uniffi::export(callback_interface)]
pub trait PartyListener: Send + Sync {
    fn on_changed(&self, state: PartyStateData);
}

/// The sink the service calls: forwards to whichever listener subscribed last,
/// so a screen that comes back replaces the one that left instead of doubling
/// the calls.
struct ListenerSink;

static LISTENER: Mutex<Option<Arc<dyn PartyListener>>> = Mutex::new(None);
static REGISTERED: OnceLock<()> = OnceLock::new();

impl nz_core::party::PartySink for ListenerSink {
    fn changed(&self, view: &nz_core::party::PartyView) {
        let listener = LISTENER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(listener) = listener {
            listener.on_changed(PartyStateData::of(view));
        }
    }
}

/// The party. MB0 declared it; every method delegates to the process's ONE
/// service and answers `Unsupported` before `attach`.
#[derive(uniffi::Object)]
pub struct PartyApi;

#[uniffi::export(async_runtime = "tokio")]
impl PartyApi {
    /// Binds the process's party service to the core and to its player. Called
    /// once by `:feature:party`'s `PartyCore`, after `core.player()` exists — the
    /// party plays through the PLAYER's engine, never a second one (CLAUDE.md).
    pub fn attach(&self, core: Arc<NzCore>, player: Arc<PlayerApi>) -> Result<(), NzError> {
        nz_core::party::attach(core.core(), player.player_handle());
        let service = nz_core::party::get().ok_or_else(|| NzError::Internal {
            detail: "l'écoute partagée n'a pas pu démarrer".to_owned(),
        })?;
        REGISTERED.get_or_init(|| service.subscribe(Arc::new(ListenerSink)));
        Ok(())
    }

    /// Whether this server has the party routes at all (rule 7): one `OPTIONS`,
    /// and a doubt never hides the door.
    pub async fn supported(&self) -> Option<bool> {
        self.service().ok()?.supported().await
    }

    /// The party, as a screen paints it right now (the playhead included).
    pub fn state(&self) -> PartyStateData {
        self.service()
            .map(|service| PartyStateData::of(&service.view()))
            .unwrap_or_else(|_| PartyStateData::of(&nz_core::party::PartyView::default()))
    }

    /// The playhead, seconds into the track: a HOT read for the screen's bar.
    pub fn position_s(&self) -> f64 {
        self.service()
            .map(|service| service.position_s())
            .unwrap_or(0.0)
    }

    pub fn state_version(&self) -> u64 {
        self.service()
            .map(|service| service.state_version())
            .unwrap_or(0)
    }

    pub fn subscribe(&self, listener: Box<dyn PartyListener>) {
        *LISTENER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(listener.into());
    }

    pub fn prefs(&self) -> PartyPrefs {
        let prefs = self
            .service()
            .map(|service| service.prefs())
            .unwrap_or_default();
        PartyPrefs {
            name: prefs.name.clone(),
            volume: prefs.volume,
            trim_ms: prefs.trim_ms,
        }
    }

    pub fn set_name(&self, name: String) {
        if let Ok(service) = self.service() {
            service.set_name(&name);
        }
    }

    /// The party's own volume (never the app's, which the person keeps).
    pub fn set_volume(&self, volume: f64) {
        if let Ok(service) = self.service() {
            service.set_volume(volume);
        }
    }

    /// The device's own output delay, milliseconds: positive plays earlier.
    pub fn set_trim(&self, ms: f64) {
        if let Ok(service) = self.service() {
            service.set_trim(ms);
        }
    }

    /// The landing card's remembered answer, disk only: what a re-open paints
    /// before the peek answers (rule 3).
    pub async fn cached_peek(&self, id: String) -> Option<PartyStateData> {
        self.service()
            .ok()
            .and_then(|service| service.cached_peek(&id))
            .map(|view| PartyStateData::of(&view))
    }

    /// `GET /api/party/<id>` — the card. `None` when the party never existed or
    /// the host ended it (the screen's `missing` phase, not a failure).
    pub async fn peek(&self, id: String) -> Result<Option<PartyStateData>, NzError> {
        let service = self.service()?;
        service
            .peek(&id)
            .await
            .map(|card| card.map(|view| PartyStateData::of(&view)))
            .map_err(NzError::from)
    }

    /// `POST /api/party` — start (or re-open) the party this person hosts.
    /// Returns the link the guests open.
    pub async fn host(&self) -> Result<String, NzError> {
        self.service()?.host().await.map_err(NzError::from)
    }

    /// Re-open the party this device was hosting before the app was killed —
    /// ONE request, and only for somebody who hosted before.
    pub async fn resume(&self) -> Result<Option<String>, NzError> {
        self.service()?.resume().await.map_err(NzError::from)
    }

    /// `POST /api/party/<id>/join`, then the guest session: the clock, the polls
    /// and the scheduler.
    pub async fn join(&self, id: String, name: String) -> Result<PartyStateData, NzError> {
        let service = self.service()?;
        service
            .join(&id, &name)
            .await
            .map(|view| PartyStateData::of(&view))
            .map_err(NzError::from)
    }

    /// Leave the party (a guest) or end it (a host, `DELETE /api/party/<id>`).
    pub async fn leave(&self) -> Result<(), NzError> {
        self.service()?.leave().await.map_err(NzError::from)
    }
}

/// The seam every exported method goes through. Not exported itself: a Rust
/// type never crosses the boundary.
impl PartyApi {
    /// The process's party service, or the cause a screen prints when
    /// `:feature:party` has not bound it yet.
    fn service(&self) -> Result<Arc<nz_core::party::Party>, NzError> {
        nz_core::party::get().ok_or_else(|| NzError::Unsupported {
            feature: "party:attach".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_core::party::{Member, PartyView, Phase, Role, TrackView};

    fn view() -> PartyView {
        PartyView {
            role: Role::Guest {
                id: "p1".to_owned(),
                host_name: "Camille".to_owned(),
            },
            phase: Phase::Live,
            host: "Camille".to_owned(),
            track: Some(TrackView {
                id: "3000001".to_owned(),
                title: "Balancing the Dark".to_owned(),
                artist: "Kaya Project".to_owned(),
                duration_s: 312.0,
                cover: Some("/api/party/p1/cover/3000001".to_owned()),
                ..TrackView::default()
            }),
            live: true,
            playing: true,
            position_s: 42.5,
            duration_s: 312.0,
            status_ok: true,
            status_text: "Calé sur Camille · ±0.4 ms".to_owned(),
            clock: Some((0.4, 8.0)),
            listeners: 2,
            members: vec![Member {
                name: "Salon".to_owned(),
                role: "guest".to_owned(),
                q: Some(0.42),
                st: Some("sync".to_owned()),
                status: "En phase · ±0.4 ms".to_owned(),
            }],
            volume: 0.8,
            trim_ms: 150.0,
            ..PartyView::default()
        }
    }

    /// A guest's state crosses whole: the role, the phase, the track's art, the
    /// tone of the line, the bar's share, and the four sentences a screen prints
    /// instead of rebuilding (`invite_line`, `live_line`, `title_line`,
    /// `audience`). The numbers are the SPA's: 42.5 s of 312 s is 13.6 %.
    #[test]
    fn a_guest_state_crosses_with_its_sentences() {
        let data = PartyStateData::of(&view());
        assert!(matches!(data.role, PartyRole::Guest { ref id, .. } if id == "p1"));
        assert_eq!(data.phase, PartyPhase::Live);
        assert_eq!(data.tone, "ok");
        assert_eq!(data.status, "Calé sur Camille · ±0.4 ms");
        assert_eq!(data.track.as_ref().map(|t| t.id.as_str()), Some("3000001"));
        assert_eq!(
            data.track.as_ref().and_then(|t| t.cover.as_deref()),
            Some("/api/party/p1/cover/3000001")
        );
        assert!(
            (data.progress - 13.621_794).abs() < 1e-3,
            "{}",
            data.progress
        );
        assert_eq!(data.clock_spread_ms, Some(0.4));
        assert_eq!(data.clock_rtt_ms, Some(8.0));
        assert_eq!(data.live_line, "En direct avec Camille · 2 à l'écoute");
        assert_eq!(data.audience, "1 personne à l'écoute");
        assert_eq!(data.title_line, "Balancing the Dark");
        assert_eq!(data.volume, 0.8);
        assert_eq!(data.trim_ms, 150.0);
        assert_eq!(data.members.len(), 1);
        // 0.42 ms, rounded to the wire's integer and never negative.
        assert_eq!(data.members[0].latency_ms, 0);
        assert_eq!(data.members[0].status, "En phase · ±0.4 ms");
        assert_eq!(data.members[0].initial, "S");
        assert!(data.members[0].synced);
        assert!(!data.members[0].waiting);
    }

    /// A host's state says so, with its link and its dead-link verdict.
    #[test]
    fn a_host_state_carries_the_link_and_its_verdict() {
        let mut host = view();
        host.role = Role::Host {
            id: "p1".to_owned(),
            link: "https://neezer.lan/party/p1".to_owned(),
            active: true,
            lost: true,
        };
        host.error = Some("Le serveur a redémarré et le lien a changé.".to_owned());
        let data = PartyStateData::of(&host);
        assert!(matches!(
            data.role,
            PartyRole::Host {
                active: true,
                lost: true,
                ..
            }
        ));
        assert_eq!(data.link, "https://neezer.lan/party/p1");
        // A cause outranks the tone of the line: the screen prints it as a failure.
        assert_eq!(data.tone, "error");
        assert!(data.error.is_some());
    }

    /// A card that never joined has no track and no members: the landing card
    /// still reads as an invitation, and nothing divides by zero.
    #[test]
    fn an_empty_card_reads_without_tracks_or_members() {
        let mut card = view();
        card.phase = Phase::Landing;
        card.track = None;
        card.members.clear();
        card.listeners = 0;
        card.playing = false;
        card.position_s = 0.0;
        card.duration_s = 0.0;
        let data = PartyStateData::of(&card);
        assert_eq!(data.phase, PartyPhase::Landing);
        assert!(data.track.is_none());
        assert_eq!(data.progress, 0.0);
        assert_eq!(data.title_line, "Rien en lecture pour l'instant");
        assert_eq!(data.audience, "Personne à l'écoute");
        assert!(data.invite_line.starts_with("Camille vous invite"));
    }

    /// The two dead-link pages carry their own sentences, and a live one carries
    /// none (a screen draws no empty heading).
    #[test]
    fn the_dead_link_pages_say_what_happened() {
        let mut card = view();
        card.phase = Phase::Ended;
        let data = PartyStateData::of(&card);
        assert_eq!(data.phase_title.as_deref(), Some("La party est terminée"));
        assert_eq!(
            data.phase_body.as_deref(),
            Some("L'hôte l'a arrêtée. Merci d'avoir écouté.")
        );
        card.phase = Phase::Missing;
        let data = PartyStateData::of(&card);
        assert_eq!(
            data.phase_title.as_deref(),
            Some("Cette party n'existe plus")
        );
        card.phase = Phase::Live;
        assert!(PartyStateData::of(&card).phase_title.is_none());
    }

    /// Every phase has a distinct FFI spelling: a copy-paste in `phase_of` would
    /// show the wrong page.
    #[test]
    fn every_phase_crosses_once() {
        let phases = [
            Phase::Idle,
            Phase::Landing,
            Phase::Joining,
            Phase::Live,
            Phase::Ended,
            Phase::Left,
            Phase::Missing,
            Phase::Unsupported,
            Phase::Failed,
        ];
        let mut seen: Vec<String> = phases
            .into_iter()
            .map(|phase| format!("{:?}", phase_of(phase)))
            .collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), phases.len());
    }
}
