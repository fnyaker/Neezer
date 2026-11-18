//! Home: hero, resume card, moods, releases card, recap card, Flow, energy picker.
//!
//! Owned by **MB5** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this file — its own `#[uniffi::export] impl` blocks, its own
//! records — and edits no other file of the crate.
//!
//! The home screen paints from the cache first (`cached_home`), then replaces it
//! — it never waits on the network to show something.
//!
//! # The shape of this boundary
//!
//! - **`cached_home`** is `Option<HomeData>`, synchronous and disk-only: the
//!   answer a warm start paints before a socket opens (`StartupBenchmark`'s
//!   TTFD ≤ 900 ms is measured on it).
//! - **`home`** is the SPA's seven requests on the `User` lane, counted under
//!   the `home` tag, assembled into ONE record so the screen repaints once
//!   instead of five times.
//! - **`flow_styles`** is its own call because the chips and Explorer share it
//!   ONCE per session (`StylePicker.svelte#loadStyleRows`): the home is six
//!   requests, the style row the seventh.
//! - **A row that a screen draws or queues is the shared `TrackRow`** — the same
//!   record the lists use, so "Pour toi", a mood's mix and a Flow pick hand the
//!   player what a playlist row hands it.
//! - **No listener of its own.** The home has no event to push: the Flow's
//!   changes are the player's `PlayerSnapshot` (MB3b), and the prefs are read
//!   when a screen asks. A listener here would be a second source of truth.
//!
//! Every body converts to `nz_core::home` and back; no rule lives in this file.

use nz_core::home as core;

use crate::core::Screens;
use crate::errors::NzError;
use crate::lists::TrackRow;

/// The person's name and the Flow's opening decision (`HomeHero.svelte`).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct HeroData {
    /// The signed-in person's name; `""` before `/me`.
    pub name: String,
    /// The notch the Flow starts from (`None`: "Libre").
    pub energy: Option<f64>,
    /// Its French name — the row never prints a number.
    pub energy_name: String,
    /// The six notches, in the row's order.
    pub energy_steps: Vec<EnergyStepData>,
    /// `discovery.js#stepOf`'s name for the tuning chip ("Voisin").
    pub discovery_name: String,
    /// The playing queue is this device's Flow.
    pub flow_live: bool,
}

/// One energy notch: the server's own words (`energy.py#STEPS`).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct EnergyStepData {
    pub name: String,
    /// `None` is "Libre".
    pub value: Option<f64>,
    pub hint: String,
}

impl From<core::EnergyStep> for EnergyStepData {
    fn from(step: core::EnergyStep) -> Self {
        EnergyStepData {
            name: step.name,
            value: step.value,
            hint: step.hint,
        }
    }
}

/// "Reprendre": what was playing, from the player's own saved queue.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ResumeCardData {
    pub key: String,
    pub id: String,
    pub title: String,
    pub artists: String,
    pub album: String,
    pub cover: Option<String>,
    pub duration_s: u32,
    /// Where the playhead sits, in seconds.
    pub position_s: f64,
}

/// One sleeve of "Nouveautés": a release, and where its first title sits.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ReleaseCardData {
    pub key: String,
    pub title: String,
    pub cover: Option<String>,
    pub artist: String,
    /// The index of its first title in [`ReleasesCardData::tracks`].
    pub track_index: u32,
}

/// "Nouveautés": the count, the unseen badge, the sleeves and what "Tout lire"
/// queues.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ReleasesCardData {
    pub count: u32,
    pub unseen: u32,
    pub pending: u32,
    pub releases: Vec<ReleaseCardData>,
    pub tracks: Vec<TrackRow>,
}

/// One favourite artist (`/me/artists`).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ArtistCardData {
    pub id: String,
    pub name: String,
    pub picture: Option<String>,
    pub tracks: u32,
    pub plays: u32,
}

/// "Ton année 2025, en cours".
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct RecapCardData {
    pub year: i32,
    pub listens: u32,
    pub minutes: u32,
    pub partial: bool,
}

/// One "Mix d'ambiance" card.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct MoodCardData {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub tracks: u32,
    pub ready: bool,
    pub cover: Option<String>,
}

/// The title the Flow is about to play.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct FlowNextData {
    pub key: String,
    pub title: String,
    pub artists: String,
}

/// What the Flow's picker, controls and "up next" note draw.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct FlowStateData {
    pub live: bool,
    pub energy: Option<f64>,
    pub energy_name: String,
    pub styles: Vec<String>,
    pub level: f64,
    /// "Techno, House +1 · Voisin" — the door's label.
    pub aim: String,
    /// "2 genres · Voisin" — the same, for a narrow button.
    pub aim_short: String,
    pub up_next: Option<FlowNextData>,
    /// Live, with nothing after the playing title: `FlowUpNext`'s sentence.
    pub waiting: bool,
}

impl From<core::FlowState> for FlowStateData {
    fn from(state: core::FlowState) -> Self {
        FlowStateData {
            live: state.live,
            energy: state.energy,
            energy_name: state.energy_name,
            styles: state.styles,
            level: state.level,
            aim: state.aim,
            aim_short: state.aim_short,
            up_next: state.up_next.map(|next| FlowNextData {
                key: next.key,
                title: next.title,
                artists: next.artists,
            }),
            waiting: state.waiting,
        }
    }
}

/// One stop of the diversity ladder, in the words the picker shows.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct DiscoveryStepData {
    pub name: String,
    pub value: f64,
    pub hint: String,
}

impl From<core::DiversityStep> for DiscoveryStepData {
    fn from(step: core::DiversityStep) -> Self {
        DiscoveryStepData {
            name: step.name,
            value: step.value,
            hint: step.hint,
        }
    }
}

/// The discovery level and its two ladders.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct DiscoveryStateData {
    pub level: f64,
    pub name: String,
    pub hint: String,
    /// The four steps the hero's chip names.
    pub steps: Vec<DiscoveryStepData>,
    /// The six the Flow's picker uses.
    pub flow_steps: Vec<DiscoveryStepData>,
}

impl From<core::Hero> for HeroData {
    fn from(hero: core::Hero) -> Self {
        HeroData {
            name: hero.name,
            energy: hero.energy,
            energy_name: hero.energy_name,
            energy_steps: hero.energy_steps.into_iter().map(Into::into).collect(),
            discovery_name: hero.discovery_name,
            flow_live: hero.flow_live,
        }
    }
}

impl From<core::DiscoveryState> for DiscoveryStateData {
    fn from(state: core::DiscoveryState) -> Self {
        DiscoveryStateData {
            level: state.level,
            name: state.name,
            hint: state.hint,
            steps: state.steps.into_iter().map(Into::into).collect(),
            flow_steps: state.flow_steps.into_iter().map(Into::into).collect(),
        }
    }
}

/// Everything the home draws, in one answer.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct HomeData {
    pub hero: HeroData,
    pub resume: Option<ResumeCardData>,
    pub releases: Option<ReleasesCardData>,
    pub artists: Vec<ArtistCardData>,
    /// "Pour toi": at most six are drawn, the answer holds the server's twenty.
    pub reco: Vec<TrackRow>,
    pub recap: Option<RecapCardData>,
    pub moods: Vec<MoodCardData>,
    pub flow: FlowStateData,
    pub discovery: DiscoveryStateData,
    /// How many of the three sections that can fail did.
    pub failures: u32,
}

impl From<core::HomeData> for HomeData {
    fn from(data: core::HomeData) -> Self {
        HomeData {
            hero: data.hero.into(),
            resume: data.resume.map(|resume| ResumeCardData {
                key: resume.key,
                id: resume.id,
                title: resume.title,
                artists: resume.artists,
                album: resume.album,
                cover: resume.cover,
                duration_s: resume.duration_s,
                position_s: resume.position_s,
            }),
            releases: data.releases.map(|card| ReleasesCardData {
                count: card.count,
                unseen: card.unseen,
                pending: card.pending,
                releases: card
                    .releases
                    .into_iter()
                    .map(|release| ReleaseCardData {
                        key: release.key,
                        title: release.title,
                        cover: release.cover,
                        artist: release.artist,
                        track_index: release.track_index,
                    })
                    .collect(),
                tracks: rows(card.tracks),
            }),
            artists: data
                .artists
                .into_iter()
                .map(|artist| ArtistCardData {
                    id: artist.id,
                    name: artist.name,
                    picture: artist.picture,
                    tracks: artist.tracks,
                    plays: artist.plays,
                })
                .collect(),
            reco: rows(data.reco),
            recap: data.recap.map(|card| RecapCardData {
                year: card.year,
                listens: card.listens,
                minutes: card.minutes,
                partial: card.partial,
            }),
            moods: data
                .moods
                .into_iter()
                .map(|mood| MoodCardData {
                    id: mood.id,
                    name: mood.name,
                    subtitle: mood.subtitle,
                    tracks: mood.tracks,
                    ready: mood.ready,
                    cover: mood.cover,
                })
                .collect(),
            flow: data.flow.into(),
            discovery: data.discovery.into(),
            failures: data.failures,
        }
    }
}

/// One genre of `/flow/styles`, as the chips draw it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct FlowStyleData {
    pub name: String,
    pub parent: Option<String>,
    /// Titles of this node alone.
    pub tracks: u32,
    /// Titles under it, this node included — what a family chip is worth.
    pub branch: u32,
}

impl From<nz_core::home::FlowStyleRow> for FlowStyleData {
    fn from(style: nz_core::home::FlowStyleRow) -> Self {
        FlowStyleData {
            name: style.name,
            parent: style.parent,
            tracks: style.tracks,
            branch: style.branch,
        }
    }
}

/// What "Lancer le Flow" answered: the session's id (the queue's context) and
/// the titles to play — the first and its reserve.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct FlowStartData {
    pub sid: String,
    pub tracks: Vec<TrackRow>,
}

/// One press of the Flow's controls (`FlowControls.svelte`).
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowPress {
    /// "Plus comme ça": its neighbours join the pool, the next is chosen again.
    More,
    /// "Moins comme ça": this title makes way for a fresh choice.
    Less,
    /// The notch or the level moved: choose again, keep this title playing.
    Reaim,
}

impl From<FlowPress> for core::FlowFeedback {
    fn from(press: FlowPress) -> Self {
        match press {
            FlowPress::More => core::FlowFeedback::More,
            FlowPress::Less => core::FlowFeedback::Less,
            FlowPress::Reaim => core::FlowFeedback::Reaim,
        }
    }
}

/// The home's own order, so a row's `position` is its index in what the screen
/// drew.
fn rows(cards: Vec<core::TrackCard>) -> Vec<TrackRow> {
    cards
        .into_iter()
        .enumerate()
        .map(|(position, card)| TrackRow::from(card.to_row(position as u32)))
        .collect()
}

/// The same, for a list that starts at zero on its own (a mood's mix).
fn rows_of(cards: Vec<core::TrackCard>) -> Vec<TrackRow> {
    rows(cards)
}

#[uniffi::export(async_runtime = "tokio")]
impl Screens {
    /// The home as this device last saw it — disk only, no request. `None` the
    /// first time, which is the one moment the hero has nothing under it.
    pub fn cached_home(&self) -> Option<HomeData> {
        self.core().cached_home().map(HomeData::from)
    }

    /// The home, refreshed: the SPA's requests on the `User` lane, counted under
    /// `home`. Fails fast when the link is dead (`NzError::Offline`), which is
    /// what the screen's "Hors ligne · version mémorisée" is read from.
    pub async fn home(&self) -> Result<HomeData, NzError> {
        self.core()
            .home()
            .await
            .map(HomeData::from)
            .map_err(NzError::from)
    }

    /// The genres a Flow can rest on, from the device's copy: what the chips
    /// paint before the network answers (and what Explorer shares).
    pub fn cached_flow_styles(&self) -> Vec<FlowStyleData> {
        self.core()
            .cached_flow_styles()
            .into_iter()
            .map(FlowStyleData::from)
            .collect()
    }

    /// `GET /api/flow/styles` — one request, kept for the next screen.
    pub async fn flow_styles(&self) -> Result<Vec<FlowStyleData>, NzError> {
        self.core()
            .flow_styles()
            .await
            .map(|rows| rows.into_iter().map(FlowStyleData::from).collect())
            .map_err(NzError::from)
    }

    /// The Flow's own state for the picker, the controls and "up next": no
    /// request, read from the device and the saved queue.
    pub fn home_flow_state(&self) -> FlowStateData {
        self.core().flow_state().into()
    }

    /// Remembers the styles the Flow rests on — the device's own, no request.
    pub fn set_flow_styles(&self, styles: Vec<String>) {
        self.core().save_styles(&styles);
    }

    /// The discovery level the device paints first.
    pub fn discovery_level(&self) -> f64 {
        self.core().discovery_level()
    }

    /// Moves it: the device answers at once, then `PUT /api/discover/prefs`.
    /// ONE call is ONE request — the slider calls this when the hand settles,
    /// never per pixel of a drag.
    pub async fn set_discovery_level(&self, level: f64) -> Result<f64, NzError> {
        self.core()
            .save_discovery_level(level)
            .await
            .map_err(NzError::from)
    }

    /// Moves the Flow's starting notch: the device first, then the account's
    /// copy.
    pub async fn set_energy(&self, energy: Option<f64>) -> Result<Option<f64>, NzError> {
        self.core().save_energy(energy).await.map_err(NzError::from)
    }

    /// One notch up (`direction > 0`) or down. Local only: a live Flow is
    /// re-aimed with [`Screens::flow_feedback`]`(Reaim)`.
    pub fn nudge_energy(&self, direction: i32) -> Option<f64> {
        self.core().nudge_energy(direction)
    }

    /// "Lancer le Flow": the SPA's own body — the notch, the level and the
    /// styles really steer the first pick. One request, two titles.
    pub async fn start_flow(&self) -> Result<FlowStartData, NzError> {
        self.core()
            .start_flow()
            .await
            .map(|start| FlowStartData {
                sid: start.sid,
                tracks: rows(start.tracks),
            })
            .map_err(NzError::from)
    }

    /// One press of the Flow's controls; answers the title to queue next, when
    /// the server had one.
    pub async fn flow_feedback(&self, press: FlowPress) -> Result<Option<TrackRow>, NzError> {
        self.core()
            .flow_feedback(press.into())
            .await
            .map(|card| card.map(|card| TrackRow::from(card.to_row(0))))
            .map_err(NzError::from)
    }

    /// `GET /api/moods/<id>` — the mix a card starts: ONE request per tap,
    /// exactly like `Home.svelte#playMood` (the catalog carries counts, not
    /// titles).
    pub async fn mood_tracks(&self, id: String) -> Result<Vec<TrackRow>, NzError> {
        self.core()
            .mood_tracks(&id)
            .await
            .map(rows)
            .map_err(NzError::from)
    }

    /// "Autres suggestions": `/recommend` again at the current level with a new
    /// seed (`Home.svelte#refreshReco`).
    pub async fn more_suggestions(&self) -> Result<Vec<TrackRow>, NzError> {
        self.core()
            .more_suggestions()
            .await
            .map(rows_of)
            .map_err(NzError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every record converts what the screen draws, and NOTHING more: the
    /// boundary stays screen-shaped (a `HomeData` carries no server JSON).
    #[test]
    fn a_home_crosses_as_screen_shaped_records() {
        let card = core::TrackCard {
            key: "11111111-1111-4111-8111-111111111111".to_owned(),
            id: "11111111-1111-4111-8111-111111111111".to_owned(),
            title: "Ouverture".to_owned(),
            artists: "Ascenseur".to_owned(),
            album: "Le matin".to_owned(),
            album_id: Some("501".to_owned()),
            duration_s: 180,
            cover: Some("/api/cover/501".to_owned()),
            explicit: false,
            unavailable: false,
            source: "deezer".to_owned(),
            release: Some("501".to_owned()),
        };
        let data = core::HomeData {
            releases: Some(core::ReleasesCard {
                count: 3,
                unseen: 2,
                pending: 0,
                releases: vec![core::ReleaseCard {
                    key: "501".to_owned(),
                    title: "Le matin".to_owned(),
                    cover: Some("/api/cover/501".to_owned()),
                    artist: "Ascenseur".to_owned(),
                    track_index: 0,
                }],
                tracks: vec![card.clone()],
            }),
            reco: vec![card],
            ..core::HomeData::default()
        };
        let crossed = HomeData::from(data);
        assert_eq!(crossed.releases.as_ref().map(|card| card.count), Some(3));
        assert_eq!(
            crossed.releases.as_ref().map(|card| card.tracks.len()),
            Some(1)
        );
        assert_eq!(crossed.reco.len(), 1);
        assert_eq!(crossed.reco[0].title, "Ouverture");
        assert_eq!(crossed.reco[0].position, 0, "a row knows where it sits");
        assert_eq!(crossed.reco[0].source, "deezer");
    }

    /// The energy ladder crosses with its `None` notch intact: "Libre" is an
    /// absence, not a zero, and a UI that read 0.0 would aim at the calmest
    /// music instead of at the person's own listening.
    #[test]
    fn free_energy_crosses_as_an_absence() {
        let hero: HeroData = core::Hero {
            name: "Nadia".to_owned(),
            energy: None,
            energy_name: "Libre".to_owned(),
            energy_steps: core::energy_steps(),
            discovery_name: "Voisin".to_owned(),
            flow_live: false,
        }
        .into();
        assert_eq!(hero.energy, None);
        assert_eq!(hero.energy_name, "Libre");
        assert_eq!(hero.energy_steps.len(), 6);
        assert_eq!(hero.energy_steps[0].value, None);
        assert_eq!(hero.energy_steps[2].value, Some(0.3));
    }

    /// A press is a value the core understands, in both directions.
    #[test]
    fn a_press_crosses_as_the_cores_own() {
        assert_eq!(
            core::FlowFeedback::from(FlowPress::More),
            core::FlowFeedback::More
        );
        assert_eq!(
            core::FlowFeedback::from(FlowPress::Less),
            core::FlowFeedback::Less
        );
        assert_eq!(
            core::FlowFeedback::from(FlowPress::Reaim),
            core::FlowFeedback::Reaim
        );
    }
}
