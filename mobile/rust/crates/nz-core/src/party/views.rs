//! What the party screens DRAW: one value, its phases, and the French lines
//! that are the PHONE's.
//!
//! The SPA's two screens are `routes/Party.svelte` (the guest: the landing card,
//! then the live view) and `components/PartySheet.svelte` (the host: start it,
//! hand the link out, see who listens, end it).
//!
//! **The rules already implemented by `nz-party` are NOT re-written here.** The
//! guest's status line (`GuestView::status_line`, `Party.svelte#statusLine`), the
//! host's audience line (`HostStatus::audience`) and a guest's own words on the
//! host's list (`Listener::status`, `PartySheet#statusOf`) live in the shared
//! crate, tested there; the FFI carries their sentences as plain strings so a
//! Kotlin screen prints them and never rebuilds one. What is here is what a
//! SERVER-shaped value does not say: the landing card's two invitations, the
//! live header, the next-track row, an avatar's letter, and the two dead-link
//! pages.

/// What this device is in the party.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Role {
    #[default]
    None,
    Host {
        id: String,
        /// The address the guests open (`<serveur>/party/<id>`).
        link: String,
        /// Whether the loops are running (`HostStatus::active`).
        active: bool,
        /// The server forgot the party and the id could not be taken back: the
        /// link in everybody's hands is dead, and the sheet says so.
        lost: bool,
    },
    Guest {
        id: String,
        /// The host's display name, as the server publishes it.
        host_name: String,
    },
}

impl Role {
    /// The link a hosted party hands out, when this device hosts one.
    pub fn host_link(&self) -> Option<String> {
        match self {
            Role::Host { link, .. } if !link.is_empty() => Some(link.clone()),
            _ => None,
        }
    }
}

/// Where the party screen stands: the SPA's `phase` (`Party.svelte`), plus the
/// two states a phone needs (the door on an older server, and a failure with a
/// cause instead of an endless spinner — rule 3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// No party on this device: the player's door, not a screen.
    #[default]
    Idle,
    /// The landing card: who invites you, what is playing, one tap to join.
    Landing,
    /// The tap happened, the seat is being taken.
    Joining,
    Live,
    /// The host ended it (the server answered `404` to a poll).
    Ended,
    /// The listener left; the screen offers « Rejoindre à nouveau ».
    Left,
    /// The link never was one (`404` on the peek).
    Missing,
    /// The server predates the feature (rule 7): `NzError::Unsupported` hides the
    /// door instead of failing behind it.
    Unsupported,
    /// The peek failed and nothing was remembered: the screen says why and
    /// offers « Réessayer ».
    Failed,
}

/// The party, composed from `nz-party`'s two views.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartyView {
    pub role: Role,
    pub phase: Phase,
    /// The host's display name (a guest) or this device's own (a host).
    pub host: String,
    pub track: Option<TrackView>,
    pub next: Option<TrackView>,
    /// The host is still there (its heartbeat has not timed out).
    pub live: bool,
    /// The host's transport says play.
    pub playing: bool,
    /// Loading or rebuffering: audible but not at its position yet.
    pub loading: bool,
    /// The party's own playhead, seconds into the track.
    pub position_s: f64,
    pub duration_s: f64,
    /// The status line's tone and sentence, as `nz-party` wrote them.
    pub status_ok: bool,
    pub status_warn: bool,
    pub status_text: String,
    /// `(spread, rtt)` of this device's clock estimate, milliseconds.
    pub clock: Option<(f64, f64)>,
    /// The link has been lost for six seconds: the music continues, and the line
    /// says so instead of pretending (`OFFLINE_AFTER_MS`).
    pub offline: bool,
    pub members: Vec<Member>,
    /// How many people are listening: the guest list's length for a host, the
    /// public state's names for a guest (which draws a count, never the names).
    pub listeners: u32,
    /// The link to hand out, when this device hosts.
    pub link: String,
    /// The three values the listener owns.
    pub volume: f64,
    pub trim_ms: f64,
    /// The output latency the OS itself reports, when it does: the *Décalage*
    /// card only offers the Bluetooth shortcut when this is under 100 ms,
    /// because a link the OS already counts must not be counted twice.
    pub os_latency_ms: Option<i64>,
    /// The last cause, in French; a screen prints it and offers « Réessayer ».
    pub error: Option<String>,
    pub version: u64,
}

/// One track of the party, as the two screens draw it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackView {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_s: f64,
    pub podcast: bool,
    /// The party's cover route (`/api/party/<pid>/cover/<id>`): a guest has no
    /// session, so its art only ever comes through the party.
    pub cover: Option<String>,
}

impl TrackView {
    pub fn of(track: &nz_model::PartyTrack) -> TrackView {
        TrackView {
            id: track.id.clone(),
            title: if track.title.is_empty() {
                "Sans titre".to_owned()
            } else {
                track.title.clone()
            },
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration_s: track.duration,
            podcast: track.podcast,
            cover: track.cover.clone(),
        }
    }
}

/// One member, as a list draws it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Member {
    pub name: String,
    /// `host` or `guest`.
    pub role: String,
    /// The guest's own sync estimate, milliseconds (`?q=`).
    pub q: Option<f64>,
    /// `sync`, `load`, `pause`, `idle` — its word for what it does.
    pub st: Option<String>,
    /// The line under its name, `nz-party`'s `Listener::status`.
    pub status: String,
}

impl Member {
    /// The host's row, from `nz-party`'s own listener.
    pub fn of_host(listener: &nz_party::Listener) -> Member {
        Member {
            name: if listener.name.is_empty() {
                "Invité".to_owned()
            } else {
                listener.name.clone()
            },
            role: "guest".to_owned(),
            q: listener.q,
            st: listener.st.clone(),
            status: listener.status(),
        }
    }

    /// The first letter the avatar draws (`l.name.trim().charAt(0)`).
    pub fn initial(&self) -> String {
        self.name
            .trim()
            .chars()
            .next()
            .map(|first| first.to_uppercase().collect::<String>())
            .unwrap_or_else(|| "?".to_owned())
    }

    /// Whether the row's status is the green one (`l.st === "sync"`).
    pub fn synced(&self) -> bool {
        self.st.as_deref() == Some("sync")
    }

    /// Whether the row's status is the amber one (`l.st === "load"`).
    pub fn waiting(&self) -> bool {
        self.st.as_deref() == Some("load")
    }
}

impl PartyView {
    /// The landing card's first sentence, the SPA's own template
    /// (`Party.svelte`): the host's name, then one clause per count.
    pub fn invite_line(&self) -> String {
        if self.host.is_empty() {
            return "Vous êtes invité à écouter".to_owned();
        }
        let mut line = format!("{} vous invite à écouter", self.host);
        if self.listeners > 0 {
            let people = if self.listeners > 1 {
                "autres personnes"
            } else {
                "autre personne"
            };
            line.push_str(&format!(" avec {} {people}", self.listeners));
        }
        line
    }

    /// `« En direct avec Camille · 3 à l'écoute »`, the live view's header — the
    /// count only once there is more than one guest (`listeners > 1`).
    pub fn live_line(&self) -> String {
        let mut line = format!("En direct avec {}", self.host);
        if self.listeners > 1 {
            line.push_str(&format!(" · {} à l'écoute", self.listeners));
        }
        line
    }

    /// The row under the playhead: the next handover, when the host announced
    /// one (`Party.svelte`'s `.next`).
    pub fn next_line(&self) -> Option<String> {
        self.next.as_ref().map(|next| {
            if next.artist.is_empty() {
                next.title.clone()
            } else {
                format!("{} — {}", next.title, next.artist)
            }
        })
    }

    /// The title the big line draws, or the page's own sentence when the room is
    /// between two tracks.
    pub fn title_line(&self) -> String {
        match &self.track {
            Some(track) => track.title.clone(),
            None => "Rien en lecture pour l'instant".to_owned(),
        }
    }

    /// The proportion of the track heard, for the bar (`Math.min(100, pos/dur*100)`).
    pub fn progress(&self) -> f64 {
        if self.duration_s <= 0.0 {
            return 0.0;
        }
        (self.position_s / self.duration_s * 100.0).clamp(0.0, 100.0)
    }

    /// The two dead-link pages (`Party.svelte`'s `missing` and `ended`).
    pub fn phase_title(&self) -> Option<&'static str> {
        match self.phase {
            Phase::Ended => Some("La party est terminée"),
            Phase::Missing => Some("Cette party n'existe plus"),
            Phase::Left => Some("Vous avez quitté la party"),
            _ => None,
        }
    }

    /// …and their sentence.
    pub fn phase_body(&self) -> Option<&'static str> {
        match self.phase {
            Phase::Ended => Some("L'hôte l'a arrêtée. Merci d'avoir écouté."),
            Phase::Missing => {
                Some("Le lien a expiré, ou l'hôte l'a arrêtée. Demandez-lui un nouveau lien.")
            }
            Phase::Left => None,
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(host: &str, listeners: u32) -> PartyView {
        PartyView {
            host: host.to_owned(),
            listeners,
            members: (0..listeners)
                .map(|index| Member {
                    name: format!("Invité {index}"),
                    role: "guest".to_owned(),
                    ..Member::default()
                })
                .collect(),
            ..PartyView::default()
        }
    }

    /// The landing card's sentences, the SPA's own template (`Party.svelte`'s
    /// `invite`: one clause per count, and no clause at zero).
    #[test]
    fn the_invitation_names_the_host_and_the_people_already_there() {
        assert_eq!(
            view("Camille", 0).invite_line(),
            "Camille vous invite à écouter"
        );
        assert_eq!(
            view("Camille", 1).invite_line(),
            "Camille vous invite à écouter avec 1 autre personne"
        );
        assert_eq!(
            view("Camille", 3).invite_line(),
            "Camille vous invite à écouter avec 3 autres personnes"
        );
        // A peek that has not answered yet cannot name the host: the card still
        // reads as an invitation instead of «  vous invite à écouter ».
        assert_eq!(view("", 0).invite_line(), "Vous êtes invité à écouter");
    }

    /// The live header counts the room, singular and plural (`Party.svelte`).
    #[test]
    fn the_live_header_counts_the_room() {
        assert_eq!(view("Camille", 0).live_line(), "En direct avec Camille");
        assert_eq!(view("Camille", 1).live_line(), "En direct avec Camille");
        assert_eq!(
            view("Camille", 3).live_line(),
            "En direct avec Camille · 3 à l'écoute"
        );
    }

    /// The next row is « Titre — Artiste », or the title alone when the server
    /// published no artist (never a dangling dash).
    #[test]
    fn the_next_row_joins_the_title_and_the_artist() {
        let mut party = view("Camille", 1);
        assert!(party.next_line().is_none());
        party.next = Some(TrackView {
            title: "Suivant".to_owned(),
            artist: "Kaya Project".to_owned(),
            ..TrackView::default()
        });
        assert_eq!(party.next_line().as_deref(), Some("Suivant — Kaya Project"));
        party.next = Some(TrackView {
            title: "Suivant".to_owned(),
            ..TrackView::default()
        });
        assert_eq!(party.next_line().as_deref(), Some("Suivant"));
    }

    /// The bar never divides by zero and never passes 100 (`Math.min(100, …)`).
    #[test]
    fn the_progress_bar_is_bounded() {
        let mut party = view("Camille", 1);
        assert_eq!(party.progress(), 0.0);
        party.duration_s = 300.0;
        party.position_s = 42.5;
        assert!(
            (party.progress() - 14.166_666).abs() < 1e-4,
            "{}",
            party.progress()
        );
        // A stalled line can overshoot (a host rewinds while a guest extrapolates).
        party.position_s = 900.0;
        assert_eq!(party.progress(), 100.0);
    }

    /// An avatar's letter survives an empty or oddly-spaced name.
    #[test]
    fn an_avatar_draws_the_first_letter_it_has() {
        let mut member = Member {
            name: "  salon".to_owned(),
            ..Member::default()
        };
        assert_eq!(member.initial(), "S");
        member.name = "éloïse".to_owned();
        assert_eq!(member.initial(), "É");
        member.name = "   ".to_owned();
        assert_eq!(member.initial(), "?");
    }
}
