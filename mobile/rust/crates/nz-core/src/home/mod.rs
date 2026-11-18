//! Home: hero, resume card, moods, releases card, recap card, Flow picker and
//! controls, energy picker. Owned by **MB5** (docs/agents/mobile.md, "Module and
//! file ownership"): the lot fills this directory and edits no other file of the
//! crate.
//!
//! The behaviour is `webapp/src/routes/Home.svelte` and the components it draws,
//! PORTED — not re-invented. What that means, section by section:
//!
//! | This module | The SPA |
//! |---|---|
//! | [`HomeData`], [`Core::home`] | `routes/Home.svelte#loadAll` |
//! | [`Core::cached_home`] | the device's copy a warm start paints |
//! | [`prefs`] (notch, styles, level) | `lib/flow.js`, `lib/discovery.js` |
//! | [`Core::start_flow`], [`Core::flow_feedback`] | `flow.js#startFlow`, `#moreLikeThis`, `#lessLikeThis` |
//! | [`Core::mood_tracks`] | `Home.svelte#playMood` |
//! | [`Core::more_suggestions`] | `Home.svelte#refreshReco` |
//!
//! # The request budget (rule 3)
//!
//! A cold home asks **seven** requests, the SPA's own list and nothing more:
//! `/releases`, `/me/artists`, `/moods`, `/discover/prefs`, `/recommend`,
//! `/recap/years`, `/flow/styles`. They run CONCURRENTLY on the `User` lane
//! (the SPA fires them together too) and the tag is [`TAG_HOME`], so
//! `NzCore::stats()` answers what this screen cost ([`Core::note_home_traffic`]).
//! Covers are not on this list on purpose: a sleeve is drawn by
//! [`Core::cover_path`], counted under `cover`, and asked for by the view that
//! shows it.
//!
//! # What is kept, and where
//!
//! [`Core::cached_home`] answers from `home-<user hash>.json` in the CACHE
//! directory (evictable, like every cache): the sections as they were, plus the
//! device's own Flow state. A warm open therefore paints before any request, and
//! a dead link still shows today's home, flagged "Hors ligne · version
//! mémorisée" by the screen. The file name is a hash of the account, never a
//! server string, and it lives beside the covers rather than in the shared
//! SQLite schema (a lot never edits `nz-cache`'s migrations).

mod wire;

#[cfg(test)]
mod testing;

pub mod prefs;

use std::path::PathBuf;
use std::sync::Arc;

use nz_api::Api;
use serde::{Deserialize, Serialize};

pub use prefs::{
    DEFAULT_LEVEL, DiversityStep, EnergyStep, NOTCHES, PICK_AHEAD_S, Prefs, STYLES_MAX, Session,
    aim_short, aim_summary, discovery_step, diversity_index, diversity_step, energy_name,
    energy_steps,
};

use crate::core::{Core, CoreError};
use crate::lists::Row;

pub use wire::FlowStyleRow;

/// The tag every request of this screen is counted under (`NzCore::stats`).
pub const TAG_HOME: &str = "home";
/// `Home.svelte#NEW_ROWS`: six sleeves, one per release, at a glance.
pub const NEW_ROWS: usize = 6;
/// `Home.svelte#RECO_ROWS`: "Pour toi" shows six titles; the answer holds 20.
pub const RECO_ROWS: usize = 6;
/// `Home.svelte#ARTISTS_SHOWN`: twelve portraits before "Tout afficher".
pub const ARTISTS_SHOWN: usize = 12;

/// The person's name and the Flow's opening decision (`HomeHero.svelte`).
///
/// The weekday under "Bonjour" is the DEVICE's clock, so it is built by the UI:
/// this core owns no calendar and the greeting is the only thing that needs one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Hero {
    /// The signed-in person's name, `""` before `/me`.
    pub name: String,
    /// The notch the Flow starts from (`None`: "Libre").
    pub energy: Option<f64>,
    /// Its French name, so the row never prints a number.
    pub energy_name: String,
    /// The six notches, in the row's order.
    pub energy_steps: Vec<EnergyStep>,
    /// `discovery.js#stepOf`'s name for the tuning chip ("Voisin").
    pub discovery_name: String,
    /// The playing queue is this device's Flow (`flow.js#flowLive`).
    pub flow_live: bool,
}

/// "Reprendre": what was playing, from the player's OWN saved queue — no
/// request, and true after a reload (`ResumeCard.svelte`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResumeCard {
    pub key: String,
    pub id: String,
    pub title: String,
    /// Already joined for display ("A, B").
    pub artists: String,
    pub album: String,
    pub cover: Option<String>,
    pub duration_s: u32,
    /// Where the playhead sits, in seconds — the card's "0:42 · 3:10".
    pub position_s: f64,
}

/// One sleeve of "Nouveautés": a RELEASE, and the index of its first title in
/// the list "Tout lire" plays (`Home.svelte#newShown`, `#playRelease`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReleaseCard {
    /// The release's id (`deezer_id`), the card's key.
    pub key: String,
    pub title: String,
    pub cover: Option<String>,
    pub artist: String,
    /// Where its first title sits in [`ReleasesCard::tracks`].
    pub track_index: u32,
}

/// "Nouveautés": the count of new titles, the unseen badge, and the sleeves.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReleasesCard {
    /// Every new title — the count the section's header states.
    pub count: u32,
    /// Releases the person has not opened yet (`/releases`' `unseen`).
    pub unseen: u32,
    /// Releases the server is still checking (`/releases`' `pending`).
    pub pending: u32,
    pub releases: Vec<ReleaseCard>,
    /// What "Tout lire" queues, in order.
    pub tracks: Vec<TrackCard>,
}

/// One favourite artist (`/me/artists`), drawn as a portrait and a name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtistCard {
    pub id: String,
    pub name: String,
    /// `/api/cover/<id>`, or `null` for the initialled disc.
    pub picture: Option<String>,
    /// How many titles of theirs the library holds, and how much was played.
    pub tracks: u32,
    pub plays: u32,
}

/// A title the screen draws and queues: the screen-shaped row, not the server's
/// JSON (`TrackRow` in `nz-ffi`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackCard {
    /// The queue's own key: the row's UUID when it has one, else the provider's
    /// id — the same value `nz-player`'s `Entry::key()` holds, so a tap and a
    /// Flow pick name one row (`trackkey.js#trackRef`).
    pub key: String,
    pub id: String,
    pub title: String,
    pub artists: String,
    pub album: String,
    pub album_id: Option<String>,
    pub duration_s: u32,
    pub cover: Option<String>,
    pub explicit: bool,
    pub unavailable: bool,
    /// `deezer`, `youtube`, `upload` — what the source chip says.
    pub source: String,
    /// The release it came from, when the server named one — "Nouveautés" groups
    /// by it.
    pub release: Option<String>,
}

/// "Ton année 2025, en cours · 1 204 écoutes · 3 890 minutes".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecapCard {
    pub year: i32,
    pub listens: u32,
    pub minutes: u32,
    /// The running year: the numbers grow until 31 December.
    pub partial: bool,
}

/// One "Mix d'ambiance" card (`MoodCard.svelte`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MoodCard {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    /// The matching titles, a COUNT — the card's "40 titres".
    pub tracks: u32,
    /// Enough measured titles to offer the card at all.
    pub ready: bool,
    pub cover: Option<String>,
}

/// The title the Flow is about to play, when it has one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FlowNext {
    pub key: String,
    pub title: String,
    pub artists: String,
}

/// What the Flow's picker, controls and "up next" note draw
/// (`FlowControls.svelte`, `FlowPicker.svelte`, `FlowUpNext.svelte`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FlowState {
    /// The playing queue is this device's Flow: the steering row is shown.
    pub live: bool,
    pub energy: Option<f64>,
    pub energy_name: String,
    /// The genres the Flow rests on (`[]`: the whole taste).
    pub styles: Vec<String>,
    /// How far it may stray, 0…1.
    pub level: f64,
    /// "Techno, House +1 · Voisin" — the door's label.
    pub aim: String,
    /// "2 genres · Voisin" — the same, for a 150 px button.
    pub aim_short: String,
    /// The queue's next row, when there is one ("the Flow holds the playing
    /// title and AT MOST the next").
    pub up_next: Option<FlowNext>,
    /// Live, with nothing after the playing title: `FlowUpNext`'s sentence.
    pub waiting: bool,
}

/// The discovery level and its ladder (the hero's tuning panel, `FlowPicker`'s
/// "Diversité", Réglages › Découverte).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryState {
    pub level: f64,
    /// `discovery.js#stepOf`'s name ("Voisin").
    pub name: String,
    /// That step's sentence.
    pub hint: String,
    /// The four steps the chip names, and the six the Flow's panel uses.
    pub steps: Vec<DiversityStep>,
    pub flow_steps: Vec<DiversityStep>,
}

/// Everything the home draws, in one answer (`HomeData` in `nz-ffi`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HomeData {
    pub hero: Hero,
    pub resume: Option<ResumeCard>,
    pub releases: Option<ReleasesCard>,
    pub artists: Vec<ArtistCard>,
    /// "Pour toi": at most [`RECO_ROWS`] rows are drawn, the answer holds the
    /// server's twenty (`Home.svelte#refreshReco`).
    pub reco: Vec<TrackCard>,
    pub recap: Option<RecapCard>,
    pub moods: Vec<MoodCard>,
    pub flow: FlowState,
    pub discovery: DiscoveryState,
    /// How many of the three sections that CAN fail did (`Home.svelte#failures`):
    /// releases, artists, "Pour toi". Moods and the recap card never condemn the
    /// home, which is why they are not counted.
    pub failures: u32,
}

impl HomeData {
    /// `Home.svelte#empty`: nothing to draw at all.
    pub fn is_empty(&self) -> bool {
        self.releases
            .as_ref()
            .is_none_or(|card| card.tracks.is_empty())
            && self.artists.is_empty()
            && self.reco.is_empty()
            && self.moods.is_empty()
    }

    /// `Home.svelte#allFailed`: nothing to draw AND three sections refused —
    /// the "l'accueil n'a pas pu se charger" card with "Réessayer".
    pub fn all_failed(&self) -> bool {
        self.is_empty() && self.failures >= 3
    }

    /// Counted bytes of what this answer holds in memory, for the lot's own
    /// measure (worker rule 4): the sum of the strings' bytes and a fixed cost
    /// per record, never a process RSS.
    pub fn memory_bytes(&self) -> u64 {
        let mut total = std::mem::size_of::<HomeData>() as u64;
        let strings = [
            &self.hero.name,
            &self.hero.energy_name,
            &self.hero.discovery_name,
        ];
        for text in strings {
            total += text.len() as u64;
        }
        for step in &self.hero.energy_steps {
            total += record_bytes(&step.name, &step.hint);
        }
        if let Some(resume) = &self.resume {
            total += record_bytes(&resume.title, &resume.artists)
                + record_bytes(&resume.album, resume.cover.as_deref().unwrap_or(""))
                + 64;
        }
        if let Some(releases) = &self.releases {
            for card in &releases.releases {
                total += record_bytes(&card.title, &card.artist)
                    + record_bytes(card.cover.as_deref().unwrap_or(""), "")
                    + 48;
            }
            for track in &releases.tracks {
                total += track.memory_bytes();
            }
        }
        for artist in &self.artists {
            total += record_bytes(&artist.name, artist.picture.as_deref().unwrap_or("")) + 32;
        }
        for track in &self.reco {
            total += track.memory_bytes();
        }
        for mood in &self.moods {
            total += record_bytes(&mood.name, &mood.subtitle)
                + record_bytes(mood.cover.as_deref().unwrap_or(""), "")
                + 40;
        }
        if let Some(next) = &self.flow.up_next {
            total += record_bytes(&next.title, &next.artists) + 24;
        }
        for style in &self.flow.styles {
            total += style.len() as u64;
        }
        total
    }
}

impl TrackCard {
    /// A title's own counted bytes.
    pub fn memory_bytes(&self) -> u64 {
        record_bytes(&self.title, &self.artists)
            + record_bytes(&self.album, self.cover.as_deref().unwrap_or(""))
            + record_bytes(&self.key, "")
            + record_bytes(&self.id, "")
            + 96
    }

    /// The queue's own row: what a tap hands the player.
    pub fn to_row(&self, position: u32) -> Row {
        Row {
            key: self.key.clone(),
            id: self.id.clone(),
            title: self.title.clone(),
            artists: self.artists.clone(),
            album: self.album.clone(),
            album_id: self.album_id.clone(),
            duration_s: self.duration_s,
            cover: self.cover.clone(),
            explicit: self.explicit,
            liked: false,
            disliked: false,
            unavailable: self.unavailable,
            downloaded: false,
            source: if self.source.is_empty() {
                "deezer".to_owned()
            } else {
                self.source.clone()
            },
            position,
            played_at: 0,
            added_at: 0,
        }
    }
}

fn record_bytes(a: &str, b: &str) -> u64 {
    (a.len() + b.len()) as u64
}

/// What `start_flow` answered: the session's id (the queue's context) and the
/// titles to play — the first, and the reserve, exactly as `flow.js#startFlow`
/// asks for two.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlowStart {
    pub sid: String,
    pub tracks: Vec<TrackCard>,
}

/// One press of the Flow's controls (`FlowControls.svelte`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowFeedback {
    /// "Plus comme ça": its neighbours join the pool, and the next is chosen again.
    More,
    /// "Moins comme ça": the same title makes way for a fresh choice.
    Less,
    /// The notch or the level moved: choose again, keep playing this title.
    Reaim,
}

impl std::fmt::Display for FlowFeedback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            FlowFeedback::More => "more",
            FlowFeedback::Less => "less",
            FlowFeedback::Reaim => "reaim",
        })
    }
}

impl Core {
    /// The home as this device last saw it — the CACHE directory's copy, no
    /// request at all. `None` the first time, which is what makes the hero paint
    /// from local state while the sections load.
    ///
    /// The Flow's steering is overlaid from the device's own prefs, so the
    /// hero's notches are the person's even on the very first frame.
    pub fn cached_home(&self) -> Option<HomeData> {
        let mut data = wire::read_snapshot(self)?;
        let prefs = self.home_prefs();
        self.apply_local(&mut data, &prefs);
        Some(data)
    }

    /// The home, refreshed: the seven requests of `Home.svelte#loadAll`, on the
    /// `User` lane, concurrently, counted under [`TAG_HOME`].
    ///
    /// A dead link answers `Offline` before a socket is opened (the screen keeps
    /// its cached copy); a section that fails costs its own sentence, never the
    /// whole screen, except when there is nothing else to draw
    /// ([`HomeData::all_failed`]).
    pub async fn home(&self) -> Result<HomeData, CoreError> {
        // The gate: offline answers in zero sockets, and the screen keeps the
        // copy it already painted.
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_HOME));
        let before = wire::totals(&api);
        let prefs = self.home_prefs();

        let (releases, artists, moods, recap, prefs_and_reco) = tokio::join!(
            self.fetch_releases(&api),
            self.fetch_artists(&api),
            self.fetch_moods(&api),
            self.fetch_recap(&api),
            self.fetch_prefs_then_reco(&api),
        );
        self.note_home_traffic(&api, before);

        let mut failures = 0;
        let releases = match releases {
            Ok(card) => card,
            Err(cause) => {
                tracing::debug!(%cause, "the releases shelf failed");
                failures += 1;
                None
            }
        };
        let artists = match artists {
            Ok(list) => list,
            Err(cause) => {
                tracing::debug!(%cause, "the artists row failed");
                failures += 1;
                Vec::new()
            }
        };
        // Moods and the recap card are accessories: a failure leaves the row
        // absent and the home whole (`Home.svelte`'s own comment).
        let moods = moods.unwrap_or_else(|cause| {
            tracing::debug!(%cause, "the moods row failed");
            Vec::new()
        });
        let recap = recap.unwrap_or_else(|cause| {
            tracing::debug!(%cause, "the recap card failed");
            None
        });
        let (adopted, reco) = match prefs_and_reco {
            Ok((prefs, reco)) => (prefs, reco),
            Err(cause) => {
                tracing::debug!(%cause, "the discovery prefs or 'pour toi' failed");
                failures += 1;
                (None, Vec::new())
            }
        };

        // The account's copy wins over the device's — but only for what it
        // really said (`discovery.js#loadDiscovery`, `flow.js#adoptPrefs`).
        let prefs = match adopted {
            Some(adopted) => {
                self.store_level(adopted.level);
                self.store_energy(adopted.energy);
                self.home_prefs()
            }
            None => prefs,
        };

        let mut data = HomeData {
            releases,
            artists,
            reco,
            recap,
            moods,
            failures,
            ..HomeData::default()
        };
        self.apply_local(&mut data, &prefs);
        if let Err(error) = wire::write_snapshot(self, &data) {
            // A cache that cannot be written never costs the screen: the fresh
            // answer is already in memory.
            tracing::debug!(%error, "the home snapshot was not written");
        }
        Ok(data)
    }

    /// The genres a Flow can rest on, from the device's copy (asked once per
    /// session by the SPA, `StylePicker.svelte#loadStyleRows`).
    pub fn cached_flow_styles(&self) -> Vec<FlowStyleRow> {
        wire::read_styles(self).unwrap_or_default()
    }

    /// `GET /flow/styles` — one request; the answer is kept for the next screen
    /// (the home's chips and Explorer's tiles name the same styles).
    pub async fn flow_styles(&self) -> Result<Vec<FlowStyleRow>, CoreError> {
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_HOME));
        let before = wire::totals(&api);
        let answer = async {
            let styles = wire::fetch_styles(&api).await?;
            let rows = wire::style_rows(&styles);
            wire::write_styles(self, &rows)?;
            Ok::<_, CoreError>(rows)
        }
        .await;
        self.note_home_traffic(&api, before);
        answer
    }

    /// The discovery level the device paints first.
    pub fn discovery_level(&self) -> f64 {
        self.home_prefs().level
    }

    /// The Flow's own state for the picker, the controls and "up next": no
    /// request, read from the device and the saved queue.
    pub fn flow_state(&self) -> FlowState {
        self.flow_state_with(&self.home_prefs())
    }

    /// Moves the discovery level: the device first (the slider answers at once),
    /// then `PUT /api/discover/prefs`, which is what the desktop reads too.
    ///
    /// One call is ONE request: the slider calls this when the hand settles
    /// (`discovery.js#SETTLE_MS` is 450 ms), never per pixel of a drag.
    pub async fn save_discovery_level(&self, level: f64) -> Result<f64, CoreError> {
        let level = prefs::clamp_level(level);
        self.store_level(level);
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_HOME));
        let before = wire::totals(&api);
        let answer = async {
            wire::put_discovery_level(&api, level).await?;
            Ok::<_, CoreError>(level)
        }
        .await;
        self.note_home_traffic(&api, before);
        answer
    }

    /// Moves the Flow's starting notch: the device first, then the account's
    /// copy (`flow.js#setStartEnergy`).
    pub async fn save_energy(&self, energy: Option<f64>) -> Result<Option<f64>, CoreError> {
        let energy = energy.map(prefs::clamp_level);
        self.store_energy(energy);
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_HOME));
        let before = wire::totals(&api);
        let answer = async {
            wire::put_energy(&api, energy).await?;
            Ok::<_, CoreError>(energy)
        }
        .await;
        self.note_home_traffic(&api, before);
        answer
    }

    /// Remembers the styles the Flow rests on — the device's own, no request
    /// (`flow.js#setStartStyles`).
    pub fn save_styles(&self, styles: &[String]) {
        self.store_styles(styles);
    }

    /// One notch up (`direction > 0`) or down, from the session's own notch or —
    /// from "Libre" — the playing title's energy when it has one
    /// (`flow.js#nudgeEnergy`). Local only: the caller re-aims with
    /// [`Core::flow_feedback`] when a Flow is live.
    pub fn nudge_energy(&self, direction: i32) -> Option<f64> {
        if direction == 0 {
            return self.home_prefs().energy;
        }
        let session = self.flow_state();
        let from = session
            .energy
            .unwrap_or_else(|| self.playing_energy().unwrap_or(0.5));
        let mut index = 0usize;
        for (candidate, value) in NOTCHES.iter().enumerate() {
            if (value - from).abs() < (NOTCHES[index] - from).abs() {
                index = candidate;
            }
        }
        // `flow.js#nudgeEnergy`: an aimed notch always steps; from "Libre" the
        // step is taken in the asked direction unless the nearest notch is
        // already past it.
        let mut step = index as i32;
        if session.energy.is_some()
            || NOTCHES[index] == from
            || (NOTCHES[index] - from) * (direction as f64) <= 0.0
        {
            step += direction.signum();
        }
        let step = step.clamp(0, NOTCHES.len() as i32 - 1) as usize;
        let value = NOTCHES[step];
        self.store_energy(Some(value));
        Some(value)
    }

    /// "Lancer le Flow": the SPA's OWN body (`flow.js#startFlow`), so the notch,
    /// the level and the styles really steer the first pick. One request, two
    /// titles (the first and the reserve).
    ///
    /// The queue then holds the playing title and AT MOST the next; the player's
    /// own refill asks for one more at the end of each row, with the level the
    /// server now holds for this account.
    pub async fn start_flow(&self) -> Result<FlowStart, CoreError> {
        let prefs = self.home_prefs();
        let session = prefs::update_session(self, |session| {
            session.sid = Session::new_sid();
            session.more.clear();
            session.less.clear();
        });
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_HOME));
        let before = wire::totals(&api);
        let answer = async {
            let body = self.flow_body(&prefs, &session, 2, &[]);
            let tracks = wire::flow_pick(&api, &body).await?;
            Ok::<_, CoreError>(FlowStart {
                sid: session.sid.clone(),
                tracks,
            })
        }
        .await;
        self.note_home_traffic(&api, before);
        answer
    }

    /// One press of the Flow's controls. Answers the title to play next, when
    /// the server had one — the caller replaces what is queued (and, for "moins
    /// comme ça", skips the playing title).
    pub async fn flow_feedback(&self, kind: FlowFeedback) -> Result<Option<TrackCard>, CoreError> {
        let prefs = self.home_prefs();
        let playing = self.playing_row();
        let key = playing.as_ref().map(|row| row.key().to_owned());
        let session = prefs::update_session(self, |session| {
            if session.sid.is_empty() {
                session.sid = Session::new_sid();
            }
            if let Some(key) = key.as_deref() {
                match kind {
                    FlowFeedback::More => session.press(true, key),
                    FlowFeedback::Less => session.press(false, key),
                    FlowFeedback::Reaim => {}
                }
            }
        });
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_HOME));
        let before = wire::totals(&api);
        let answer = async {
            let exclude = self.queued_keys();
            let body = self.flow_body(&prefs, &session, 1, &exclude);
            let tracks = wire::flow_pick(&api, &body).await?;
            Ok::<_, CoreError>(tracks.into_iter().next())
        }
        .await;
        self.note_home_traffic(&api, before);
        answer
    }

    /// `GET /api/moods/<id>` — the mix a card starts, one request per tap
    /// (`Home.svelte#playMood`). A mix that is not ready answers nothing, and the
    /// screen says why.
    pub async fn mood_tracks(&self, id: &str) -> Result<Vec<TrackCard>, CoreError> {
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_HOME));
        let before = wire::totals(&api);
        let answer = async {
            let mood = wire::fetch_mood(&api, id).await?;
            Ok::<_, CoreError>(mood.tracks.iter().map(wire::track_card).collect())
        }
        .await;
        self.note_home_traffic(&api, before);
        answer
    }

    /// "Autres suggestions": `/recommend` again, at the current level and with a
    /// new seed, exactly like `Home.svelte#refreshReco`.
    pub async fn more_suggestions(&self) -> Result<Vec<TrackCard>, CoreError> {
        let level = self.discovery_level();
        let api = self.api()?;
        let api = Arc::new(api.with_tag(TAG_HOME));
        let before = wire::totals(&api);
        let answer = async {
            let seed = wire::seed();
            let answer = wire::fetch_recommend(&api, level, Some(&seed)).await?;
            Ok::<_, CoreError>(answer.tracks.iter().map(wire::track_card).collect())
        }
        .await;
        self.note_home_traffic(&api, before);
        answer
    }

    // -- internals ------------------------------------------------------------

    /// Fills what the device knows: the hero, the resume card, the Flow and the
    /// discovery ladder. Never touches the network.
    fn apply_local(&self, data: &mut HomeData, prefs: &Prefs) {
        let name = self
            .me()
            .map(|me| me.user.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_default();
        let (resume, _, live) = self.queue_view();
        data.hero = Hero {
            name,
            energy: prefs.energy,
            energy_name: energy_name(prefs.energy),
            energy_steps: energy_steps(),
            discovery_name: discovery_step(prefs.level).name,
            flow_live: live,
        };
        data.resume = resume;
        data.flow = self.flow_state_with(prefs);
        data.discovery = DiscoveryState {
            level: prefs.level,
            name: discovery_step(prefs.level).name,
            hint: discovery_step(prefs.level).hint,
            steps: prefs::discovery_steps(),
            flow_steps: prefs::diversity_steps(),
        };
    }

    fn flow_state_with(&self, prefs: &Prefs) -> FlowState {
        let (_, up_next, live) = self.queue_view();
        FlowState {
            live,
            energy: prefs.energy,
            energy_name: energy_name(prefs.energy),
            styles: prefs.styles.clone(),
            level: prefs.level,
            aim: aim_summary(&prefs.styles, prefs.level),
            aim_short: aim_short(&prefs.styles, prefs.level),
            waiting: live && up_next.is_none(),
            up_next,
        }
    }

    /// What the saved queue says: the resume card, the row after the playing
    /// one, and whether the queue is this device's Flow. Disk only.
    fn queue_view(&self) -> (Option<ResumeCard>, Option<FlowNext>, bool) {
        let Some(snapshot) = self
            .cache()
            .queue()
            .load::<nz_player::Entry>()
            .ok()
            .flatten()
        else {
            return (None, None, false);
        };
        if snapshot.is_empty() {
            return (None, None, false);
        }
        let live = snapshot
            .context
            .as_deref()
            .is_some_and(|context| context.starts_with("flow"));
        let current = snapshot.entries.get(snapshot.index);
        let resume = current.map(|entry| ResumeCard {
            key: entry.key().to_owned(),
            id: entry.id.clone(),
            title: entry.title.clone(),
            artists: entry_artists(entry),
            album: entry
                .album
                .as_ref()
                .map(|album| album.title.clone())
                .unwrap_or_default(),
            cover: entry
                .cover
                .clone()
                .or_else(|| entry.album.as_ref().and_then(|album| album.cover.clone())),
            duration_s: entry.duration,
            position_s: snapshot.position_ms as f64 / 1000.0,
        });
        let up_next = snapshot
            .entries
            .get(snapshot.index + 1)
            .map(|entry| FlowNext {
                key: entry.key().to_owned(),
                title: entry.title.clone(),
                artists: entry_artists(entry),
            });
        (resume, up_next, live)
    }

    /// The playing entry, for the feedback's `more`/`less` ids.
    fn playing_row(&self) -> Option<nz_player::Entry> {
        let snapshot = self.cache().queue().load::<nz_player::Entry>().ok()??;
        snapshot.entries.get(snapshot.index).cloned()
    }

    /// The playing title's measured energy, when the queue row carries one
    /// (`flow.js#nudgeEnergy`'s starting point from "Libre").
    fn playing_energy(&self) -> Option<f64> {
        // `Entry` carries no energy: the server's measured value travels with a
        // Flow pick only. A Flow that started from a pick therefore nudges from
        // what the person asked, never from a number the row does not have.
        None
    }

    /// What the queue holds, so a Flow does not hand back what it already has
    /// (`flow.js#body`'s `exclude`, the SPA's own 400).
    fn queued_keys(&self) -> Vec<String> {
        let Ok(Some(snapshot)) = self.cache().queue().load::<nz_player::Entry>() else {
            return Vec::new();
        };
        let mut keys: Vec<String> = snapshot
            .entries
            .iter()
            .rev()
            .take(wire::EXCLUDE_MAX)
            .map(|entry| entry.key().to_owned())
            .collect();
        keys.reverse();
        keys
    }

    /// The body `discover.py#flow_next` reads, built the way `flow.js#body`
    /// builds it — this is the whole point of the lot: the notch, the level and
    /// the styles the person chose really reach the pick.
    fn flow_body(
        &self,
        prefs: &Prefs,
        session: &Session,
        n: u8,
        exclude: &[String],
    ) -> serde_json::Value {
        // Two decimals: the server rounds to its own notch (`flow.js#body`
        // sends `Number(level.toFixed(2))`).
        let level = (prefs.level * 100.0).round() / 100.0;
        serde_json::json!({
            "sid": if session.sid.is_empty() { serde_json::Value::Null } else { session.sid.clone().into() },
            "d": level,
            "energy": prefs.energy,
            "styles": prefs.styles,
            "heard": Vec::<serde_json::Value>::new(),
            "more": session.more,
            "less": session.less,
            "exclude": exclude,
            "n": n,
        })
    }

    /// Books one screen-open's traffic in the core's meter, so `NzCore::stats()`
    /// answers what this screen cost without the UI counting anything itself.
    fn note_home_traffic(&self, api: &Api, before: wire::Totals) {
        let after = wire::totals(api);
        let requests = after.requests.saturating_sub(before.requests);
        if requests == 0 {
            return;
        }
        // `Meter::record` counts ONE request and its bytes; the bytes of the
        // whole run go with the first, the rest are zeros.
        self.meter().record(
            TAG_HOME,
            after.bytes_in.saturating_sub(before.bytes_in),
            after.bytes_out.saturating_sub(before.bytes_out),
        );
        for _ in 1..requests {
            self.meter().record(TAG_HOME, 0, 0);
        }
    }

    async fn fetch_releases(&self, api: &Api) -> Result<Option<ReleasesCard>, CoreError> {
        let answer = wire::fetch_releases(api).await?;
        // One card per release, newest first: its first title is what the card
        // plays (`Home.svelte#newShown`).
        let tracks: Vec<TrackCard> = answer.tracks.iter().map(wire::track_card).collect();
        let mut seen = std::collections::BTreeSet::new();
        let mut cards = Vec::new();
        for (index, track) in tracks.iter().enumerate() {
            let release = track
                .release
                .clone()
                .unwrap_or_else(|| track.album_id.clone().unwrap_or_else(|| track.id.clone()));
            if !seen.insert(release.clone()) {
                continue;
            }
            cards.push(ReleaseCard {
                key: release,
                title: if track.album.is_empty() {
                    track.title.clone()
                } else {
                    track.album.clone()
                },
                cover: track.cover.clone(),
                artist: track.artists.clone(),
                track_index: index as u32,
            });
            if cards.len() >= NEW_ROWS {
                break;
            }
        }
        Ok(Some(ReleasesCard {
            count: tracks.len() as u32,
            unseen: answer.unseen,
            pending: answer.pending,
            releases: cards,
            tracks,
        }))
    }

    async fn fetch_artists(&self, api: &Api) -> Result<Vec<ArtistCard>, CoreError> {
        let answer = wire::fetch_artists(api).await?;
        Ok(answer
            .artists
            .iter()
            .take(ARTISTS_SHOWN)
            .map(|artist| ArtistCard {
                id: artist
                    .id
                    .clone()
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(|| artist.deezer_id.to_string()),
                name: artist.name.clone(),
                picture: artist.picture.clone(),
                tracks: artist.tracks.unwrap_or(0),
                plays: artist.plays.unwrap_or(0),
            })
            .collect())
    }

    async fn fetch_moods(&self, api: &Api) -> Result<Vec<MoodCard>, CoreError> {
        let answer = wire::fetch_moods(api).await?;
        // Only the mixes that are ready: an unavailable card is a promise the
        // server cannot keep (`Home.svelte#readyMoods`).
        Ok(answer
            .moods
            .iter()
            .filter(|mood| mood.ready)
            .map(|mood| MoodCard {
                id: mood.id.clone(),
                name: mood.name.clone(),
                subtitle: mood.subtitle.clone(),
                tracks: mood.tracks,
                ready: mood.ready,
                cover: mood.cover.clone(),
            })
            .collect())
    }

    async fn fetch_recap(&self, api: &Api) -> Result<Option<RecapCard>, CoreError> {
        let answer = wire::fetch_recap_years(api).await?;
        Ok(wire::advertised_year(&answer).map(|row| RecapCard {
            year: row.year,
            listens: row.listens,
            minutes: row.minutes,
            partial: row.partial,
        }))
    }

    /// `/discover/prefs` then `/recommend`: the second asks with the level the
    /// first just answered, which is what `Home.svelte#loadAll` chains.
    async fn fetch_prefs_then_reco(
        &self,
        api: &Api,
    ) -> Result<(Option<wire::AdoptedPrefs>, Vec<TrackCard>), CoreError> {
        let adopted = match wire::fetch_prefs(api).await {
            Ok(answer) => wire::AdoptedPrefs {
                level: prefs::clamp_level(answer.level),
                energy: answer.energy.map(prefs::clamp_level),
            },
            Err(cause) => {
                tracing::debug!(%cause, "the discovery prefs failed; the device's level is used");
                return Ok((None, Vec::new()));
            }
        };
        let answer = wire::fetch_recommend(api, adopted.level, None).await?;
        Ok((
            Some(adopted),
            answer.tracks.iter().map(wire::track_card).collect(),
        ))
    }
}

/// Where the snapshot lives: the CACHE directory (evictable), named after a hash
/// of the account so two accounts never read each other's home (rule 3: a path
/// from the server is never trusted; this one is a hash of the person's id).
fn snapshot_name(owner: &str) -> String {
    format!("home-{:016x}.json", fnv1a(owner.as_bytes()))
}

fn snapshot_path(core: &Core, owner: &str) -> PathBuf {
    core.cache_path(&snapshot_name(owner))
}

/// The account the device's copy belongs to: `/me`'s id, `""` before a session.
fn owner(core: &Core) -> String {
    core.me()
        .map(|me| me.user.id.to_string())
        .filter(|id| !id.is_empty())
        .unwrap_or_default()
}

/// A queue entry's performer line: what it was given, else its first credited
/// artist (`format.js#artistLine`).
fn entry_artists(entry: &nz_player::Entry) -> String {
    if let Some(line) = entry
        .display_artist
        .as_deref()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        return line.to_owned();
    }
    if let Some(artist) = entry.artist.as_ref().map(|artist| artist.name.trim())
        && !artist.is_empty()
    {
        return artist.to_owned();
    }
    let names: Vec<&str> = entry
        .artists
        .iter()
        .map(|artist| artist.name.trim())
        .filter(|name| !name.is_empty())
        .collect();
    names.join(", ")
}

/// FNV-1a, 64 bits — the same rule `nz-cache` uses for a file name: stable
/// across runs and platforms, which is what keeps a snapshot readable after an
/// upgrade.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two empty states, as `Home.svelte` decides them: an empty library is
    /// a sentence with a door, a refused one is the cause with "Réessayer".
    #[test]
    fn an_empty_home_says_whether_it_is_empty_or_broken() {
        let empty = HomeData::default();
        assert!(empty.is_empty());
        assert!(!empty.all_failed(), "nothing failed: the library is empty");

        let refused = HomeData {
            failures: 3,
            ..HomeData::default()
        };
        assert!(refused.is_empty());
        assert!(
            refused.all_failed(),
            "three sections refused and nothing to draw"
        );

        let partly = HomeData {
            failures: 3,
            artists: vec![ArtistCard {
                name: "Nina".into(),
                ..ArtistCard::default()
            }],
            ..HomeData::default()
        };
        assert!(!partly.is_empty());
        assert!(!partly.all_failed(), "one section is enough for a home");
    }

    /// The snapshot's name is a hash, stable across runs, and one account never
    /// reads another's.
    #[test]
    fn a_snapshot_is_named_after_a_hash_of_the_account() {
        let a = snapshot_name("user-a");
        assert_eq!(a, snapshot_name("user-a"), "stable");
        assert_ne!(a, snapshot_name("user-b"));
        assert!(a.starts_with("home-") && a.ends_with(".json"));
        assert!(a.chars().skip(5).take(16).all(|ch| ch.is_ascii_hexdigit()));
    }

    /// Counted bytes: the measure of "what the home holds in memory" is a sum of
    /// real string lengths, never a process RSS (worker rule 4).
    #[test]
    fn memory_is_counted_from_what_the_answer_really_holds() {
        let card = TrackCard {
            key: "deezer:1".into(),
            id: "1".into(),
            title: "Titre".into(),
            artists: "Artiste".into(),
            album: "Album".into(),
            cover: Some("/api/cover/1".into()),
            ..TrackCard::default()
        };
        let one = card.memory_bytes();
        assert!(one >= 96 + 5 + 7 + 5 + 13);

        let data = HomeData {
            reco: vec![card.clone(), card],
            ..HomeData::default()
        };
        let bytes = data.memory_bytes();
        assert!(bytes > 2 * one, "the rows are counted, one by one: {bytes}");
        assert!(
            bytes < 4_096,
            "two rows are a few hundred bytes, not a megabyte: {bytes}"
        );
    }
}

/// The lot's own tests: the screen's request budget, its failure shapes and its
/// Flow — against a server that COUNTS, never against a duration.
#[cfg(test)]
mod lot_tests {
    use super::*;
    use crate::core::NetworkState;

    /// The seven routes `Home.svelte#loadAll` asks for, sorted.
    const SPA_ROUTES: [&str; 7] = [
        "GET /api/discover/prefs",
        "GET /api/flow/styles",
        "GET /api/me/artists",
        "GET /api/moods",
        "GET /api/recap/years",
        "GET /api/recommend",
        "GET /api/releases",
    ];

    fn requests_for(core: &Core, tag: &str) -> u64 {
        core.stats()
            .into_iter()
            .find(|row| row.tag == tag)
            .map(|row| row.requests)
            .unwrap_or(0)
    }

    /// A cold home costs the SPA's SEVEN requests and not one more (measured on
    /// `Home.svelte`'s endpoint list — `webapp/test/requests/run.mjs` has no home
    /// bench), and a warm one costs ZERO: the snapshot is on the device.
    ///
    /// Numbers this test holds: 6 requests for the sections + 1 for the style
    /// chips = 7 cold, 0 warm, and one `home` row in `NzCore::stats()` with 7.
    #[tokio::test]
    async fn a_cold_home_asks_the_spas_seven_requests_and_a_warm_one_asks_none() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();

        // The screen's two calls, in the order the ViewModel makes them.
        let data = core.home().await.expect("a home");
        let styles = core.flow_styles().await.expect("the style chips");
        assert_eq!(styles.len(), 4);
        let mut asked = server.routes();
        asked.sort();
        assert_eq!(
            asked,
            SPA_ROUTES,
            "exactly the SPA's list: 7 requests ({})",
            server.count()
        );
        assert_eq!(data.failures, 0, "nothing to apologise for");
        assert_eq!(data.hero.name, "Nadia", "the greeting names the person");
        assert_eq!(
            data.releases.as_ref().map(|card| card.releases.len()),
            Some(2),
            "two releases, three titles"
        );
        assert_eq!(data.releases.as_ref().map(|card| card.count), Some(3));
        assert_eq!(data.releases.as_ref().map(|card| card.unseen), Some(2));
        assert_eq!(
            data.releases
                .as_ref()
                .map(|card| card.releases[0].track_index),
            Some(0),
            "the first card plays the first title"
        );
        assert_eq!(data.artists.len(), 2);
        assert_eq!(
            data.artists[0].picture.as_deref(),
            Some("/api/cover/aaaaaaaa-1111-4111-8111-111111111111")
        );
        assert_eq!(data.reco.len(), 1);
        assert_eq!(data.reco[0].title, "Pour toi");
        assert_eq!(
            data.moods.len(),
            1,
            "the mix that is not ready is not offered"
        );
        assert_eq!(data.moods[0].id, "calme");
        assert_eq!(data.moods[0].tracks, 40);
        assert_eq!(
            data.recap.as_ref().map(|card| card.year),
            Some(2025),
            "the last enough year, not the empty 2024"
        );
        assert_eq!(
            data.discovery.level, 0.6,
            "the account's level, not the device's 0.35"
        );
        assert_eq!(data.hero.energy, Some(0.3), "the account's notch");
        assert_eq!(data.hero.energy_steps.len(), 6);

        let row = core
            .stats()
            .into_iter()
            .find(|row| row.tag == TAG_HOME)
            .expect("one home row");
        assert_eq!(row.requests, 7, "NzCore::stats() counts the same seven");
        assert!(
            row.bytes_in > 0,
            "the response bodies are counted, not timed: {} bytes",
            row.bytes_in
        );
        println!(
            "MEASURE home cold: {} requests, {} bytes in, {} bytes out",
            row.requests, row.bytes_in, row.bytes_out
        );

        // A warm open paints from the device: no request at all.
        server.forget();
        let cached = core.cached_home().expect("the snapshot the visit wrote");
        assert_eq!(server.count(), 0, "a warm home asks nothing");
        assert_eq!(cached.releases.as_ref().map(|card| card.count), Some(3));
        assert_eq!(cached.hero.name, "Nadia");
        assert_eq!(
            cached.hero.energy,
            Some(0.3),
            "the local copy carries the notch the account gave"
        );
    }

    /// A dead link fails fast and keeps what the person already saw: the cache
    /// answers, and NO socket is opened to learn there is no link.
    #[tokio::test]
    async fn a_dead_link_opens_no_socket_and_the_snapshot_still_paints() {
        let (server, _dir, core) = testing::configured().await;
        core.home().await.expect("one good visit");
        let warm = core.cached_home().expect("the visit's copy");
        assert!(!warm.is_empty());

        server.forget();
        let before = server.accepted();
        core.set_network(NetworkState::None);
        assert_eq!(
            core.home().await.expect_err("offline"),
            CoreError::Offline,
            "the cause is the link, not a timeout"
        );
        assert_eq!(server.accepted(), before, "not one socket was opened");
        assert_eq!(server.count(), 0);
        assert!(
            core.cached_home().is_some(),
            "the copy is what the screen keeps"
        );
    }

    /// A section that refuses costs its own sentence: the moods row (an
    /// accessory) never condemns the home, and only when the three real sections
    /// are down does the screen say "l'accueil n'a pas pu se charger".
    #[tokio::test]
    async fn a_refused_section_leaves_the_rest_of_the_home_standing() {
        let (server, _dir, core) = testing::configured().await;
        server.break_route("/api/moods", 500);
        let data = core.home().await.expect("the home still opens");
        assert!(data.moods.is_empty(), "the row is simply absent");
        assert_eq!(data.failures, 0, "a mood is not a section that condemns it");
        assert_eq!(data.artists.len(), 2);

        let (server, _dir, core) = testing::configured().await;
        server.break_route("/api/releases", 503);
        server.break_route("/api/me/artists", 503);
        server.break_route("/api/recommend", 503);
        // The SPA's `empty` also needs no mood: a home with a mix row is not an
        // empty home, even when its three real sections refused.
        server.break_route("/api/moods", 503);
        let data = core.home().await.expect("the answer, not an error");
        assert_eq!(data.failures, 3);
        assert!(data.is_empty());
        assert!(
            data.all_failed(),
            "three sections refused and nothing to draw: the card with Réessayer"
        );
    }

    /// The notch is the person's: it reaches the account's copy in ONE request,
    /// and a restart still paints it.
    #[tokio::test]
    async fn the_notch_reaches_the_server_in_one_request_and_survives_a_restart() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        core.save_energy(Some(0.7)).await.expect("saved");
        let hits = server.routes();
        assert_eq!(
            hits,
            vec!["PUT /api/discover/prefs".to_owned()],
            "one request"
        );
        let body = server.last_body("/api/discover/prefs").expect("a body");
        assert!(body.contains("0.7"), "the notch travels: {body}");
        assert_eq!(core.home_prefs().energy, Some(0.7));
        assert_eq!(prefs::energy_name(core.home_prefs().energy), "Énergique");
    }

    /// The discovery level is one PUT per released drag, and the Flow's panel
    /// reads it back.
    #[tokio::test]
    async fn the_discovery_level_is_one_round_trip_and_reads_back() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let level = core.save_discovery_level(0.9).await.expect("saved");
        assert_eq!(level, 0.9);
        assert_eq!(server.routes(), vec!["PUT /api/discover/prefs".to_owned()]);
        assert_eq!(core.discovery_level(), 0.9);
        assert_eq!(core.flow_state().aim_short, "Tous les styles · Large");
        assert_eq!(core.save_discovery_level(4.0).await.expect("clamped"), 1.0);
    }

    /// "Lancer le Flow" sends the SPA's OWN body: the notch, the level and the
    /// styles the person chose — which is the whole reason this lot owns them.
    #[tokio::test]
    async fn the_flow_start_carries_the_notch_the_level_and_the_styles() {
        let (server, _dir, core) = testing::configured().await;
        core.save_styles(&["Techno".to_owned(), "House".to_owned()]);
        core.store_energy(Some(0.7));
        core.store_level(0.8);
        server.forget();

        let start = core.start_flow().await.expect("a Flow");
        assert_eq!(
            server.routes(),
            vec!["POST /api/flow/next".to_owned()],
            "one request"
        );
        let body = server.last_body("/api/flow/next").expect("a body");
        let sent: serde_json::Value = serde_json::from_str(&body).expect("the body is JSON");
        assert_eq!(sent["energy"], 0.7, "the notch steers the first pick");
        assert_eq!(sent["d"], 0.8, "the level travels as two decimals");
        assert_eq!(sent["styles"], serde_json::json!(["Techno", "House"]));
        assert_eq!(sent["n"], 2, "the first title and its reserve");
        assert_eq!(
            sent["sid"].as_str().map(str::len),
            Some(16),
            "a session id, so the server can steer the rest"
        );
        assert_eq!(start.tracks.len(), 2);
        assert_eq!(start.tracks[0].title, "Flow 1");
        assert_eq!(start.sid, sent["sid"].as_str().unwrap_or_default());
    }

    /// "Plus / moins comme ça" sends the session's own vocabulary and answers the
    /// next title; the queue's tail is excluded so it does not come back.
    #[tokio::test]
    async fn the_flow_feedback_records_the_press_and_asks_one_title() {
        let (server, _dir, core) = testing::configured().await;
        core.cache()
            .queue()
            .save(&nz_cache::QueueSnapshot::new(
                0,
                42_000,
                Some("flow:abcdef0123456789".to_owned()),
                vec![
                    nz_player::Entry::new("11111111-1111-4111-8111-555555555555"),
                    nz_player::Entry::new("11111111-1111-4111-8111-666666666666"),
                ],
            ))
            .expect("a saved queue");

        let state = core.flow_state();
        assert!(state.live, "the queue is this device's Flow");
        assert_eq!(
            state.up_next.as_ref().map(|next| next.key.as_str()),
            Some("11111111-1111-4111-8111-666666666666"),
            "the Flow holds the playing title and at most the next"
        );
        assert!(!state.waiting);

        server.forget();
        let next = core
            .flow_feedback(FlowFeedback::More)
            .await
            .expect("an answer");
        assert!(next.is_some(), "a title to queue");
        assert_eq!(server.routes(), vec!["POST /api/flow/next".to_owned()]);
        let body: serde_json::Value =
            serde_json::from_str(&server.last_body("/api/flow/next").expect("a body"))
                .expect("JSON");
        assert_eq!(
            body["more"],
            serde_json::json!(["11111111-1111-4111-8111-555555555555"]),
            "the playing title is what 'plus comme ça' names"
        );
        assert_eq!(body["n"], 1);
        assert_eq!(
            body["exclude"],
            serde_json::json!([
                "11111111-1111-4111-8111-555555555555",
                "11111111-1111-4111-8111-666666666666"
            ]),
            "what the queue holds never comes back"
        );

        core.flow_feedback(FlowFeedback::Less)
            .await
            .expect("an answer");
        let body: serde_json::Value =
            serde_json::from_str(&server.last_body("/api/flow/next").expect("a body"))
                .expect("JSON");
        assert_eq!(
            body["less"],
            serde_json::json!(["11111111-1111-4111-8111-555555555555"])
        );
    }

    /// The resume card is the player's own saved queue: no request, and true
    /// after a restart (`ResumeCard.svelte`).
    #[tokio::test]
    async fn the_resume_card_reads_the_saved_queue_without_a_request() {
        let (server, _dir, core) = testing::configured().await;
        let mut entry = nz_player::Entry::new("11111111-1111-4111-8111-555555555555");
        entry.title = "Titre en cours".to_owned();
        entry.duration = 210;
        entry.cover = Some("/api/cover/501".to_owned());
        entry.album = Some(nz_player::EntryAlbum {
            title: "Le matin".to_owned(),
            cover: Some("/api/cover/501".to_owned()),
            ..Default::default()
        });
        core.cache()
            .queue()
            .save(&nz_cache::QueueSnapshot::new(
                0,
                42_000,
                Some("album:501".to_owned()),
                vec![entry],
            ))
            .expect("a saved queue");

        server.forget();
        let data = core.home().await.expect("a home");
        let resume = data.resume.expect("the card");
        assert_eq!(resume.title, "Titre en cours");
        assert_eq!(resume.album, "Le matin");
        assert_eq!(resume.position_s, 42.0, "the playhead, in seconds");
        assert_eq!(resume.duration_s, 210);
        assert!(!data.flow.live, "an album queue is not a Flow");
        assert_eq!(server.count(), 6, "the queue cost no request of its own");
    }

    /// A mood tap is ONE request (`Home.svelte#playMood`), and the mix arrives
    /// as queue rows.
    #[tokio::test]
    async fn a_mood_tap_is_one_request_and_a_finite_list() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let tracks = core.mood_tracks("calme").await.expect("the mix");
        assert_eq!(server.routes(), vec!["GET /api/moods/calme".to_owned()]);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title, "Mix 1");
        let row = tracks[0].to_row(0);
        assert_eq!(row.source, "deezer");
        assert_eq!(row.cover.as_deref(), Some("/api/cover/501"));
    }

    /// The styles are asked ONCE for the session and kept: the home's chips and
    /// Explorer's tiles must not each pay for them (`StylePicker.svelte`).
    #[tokio::test]
    async fn the_styles_are_asked_once_and_kept_on_the_device() {
        let (server, _dir, core) = testing::configured().await;
        assert!(core.cached_flow_styles().is_empty(), "nothing yet");
        server.forget();
        let rows = core.flow_styles().await.expect("the genres");
        assert_eq!(server.routes(), vec!["GET /api/flow/styles".to_owned()]);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].name, "Techno");
        assert_eq!(rows[3].parent.as_deref(), Some("Techno"));

        server.forget();
        let kept = core.cached_flow_styles();
        assert_eq!(kept.len(), 4);
        assert_eq!(server.count(), 0, "the second screen pays nothing");
    }

    /// "Autres suggestions" asks again with a seed, as `Home.svelte#refreshReco`
    /// does: one request, and it carries the level.
    #[tokio::test]
    async fn another_suggestion_is_one_request_at_the_current_level() {
        let (server, _dir, core) = testing::configured().await;
        core.store_level(0.55);
        server.forget();
        let rows = core.more_suggestions().await.expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(server.routes(), vec!["GET /api/recommend".to_owned()]);
        let hit = server.hits().pop().expect("the hit");
        assert_eq!(hit.query("d").as_deref(), Some("0.55"));
        assert!(
            hit.query("seed").is_some(),
            "a new seed, so the pool is redrawn"
        );
    }

    /// The hero's notches step one at a time, and a step stores the notch.
    #[test]
    fn a_notch_nudge_steps_one_notch_and_stores_it() {
        let dir = testing::TempDir::new("home-nudge");
        let core = dir.core();
        // From "Libre" with nothing playing the SPA starts at the middle notch
        // (0.5) and takes one step UP from it (`flow.js#nudgeEnergy`).
        assert_eq!(core.nudge_energy(1), Some(0.7), "from Libre, up from 0.5");
        assert_eq!(core.home_prefs().energy, Some(0.7));
        assert_eq!(core.nudge_energy(1), Some(0.9));
        assert_eq!(core.nudge_energy(-1), Some(0.7));
        core.store_energy(Some(0.9));
        assert_eq!(core.nudge_energy(1), Some(0.9), "the loudest notch holds");
        core.store_energy(Some(0.1));
        assert_eq!(core.nudge_energy(-1), Some(0.1), "the softest notch holds");
    }
}
