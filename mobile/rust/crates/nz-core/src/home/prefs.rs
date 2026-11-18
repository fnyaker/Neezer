//! The Flow's steering: the energy notch, the styles it rests on, and the
//! discovery level — `webapp/src/lib/flow.js` and `lib/discovery.js`, ported.
//!
//! Owned by **MB5** (docs/agents/mobile.md, "Module and file ownership").
//!
//! Three rules these values keep, each one learned on the SPA:
//!
//! - **The device copy paints first, the account's copy wins.** The notch and
//!   the discovery level are kept by the SERVER (`PUT /api/discover/prefs`) so
//!   the phone and the desktop agree, and a copy lives on the device so the
//!   hero's notches are filled on the first frame instead of after a request
//!   (`discovery.js#readLocal`, `flow.js#readNotch`). [`Home::adopt`] is where
//!   the server's answer replaces the local one.
//! - **The styles are the DEVICE's, never the account's** ("it is a mood of the
//!   moment, not a setting of the account"): `flow.styles` in the kv store only.
//! - **A value from an older build still lands on a notch.** The discovery
//!   level is one number from 0 to 1 and the six names are a ladder over it
//!   (`diversity_index` is the SPA's nearest-stop, ties upward), so 0.35 reads
//!   "Voisin" rather than a notch that does not exist.
//!
//! The two ladders are the server's own words: the energy notches are
//! `neezer/energy.py#STEPS` and the diversity names are `flow.js`'s. Neither is
//! invented here.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::Core;

/// `flow.energy` — the notch the Flow starts from (`null`: "Libre").
pub const KEY_ENERGY: &str = "flow.energy";
/// `flow.styles` — the genre names the Flow rests on (`[]`: the whole taste).
pub const KEY_STYLES: &str = "flow.styles";
/// `discover.level` — how far the Flow and "Pour toi" stray (0…1).
pub const KEY_LEVEL: &str = "discover.level";
/// `flow.session` — the Flow session's own steering (`more`/`less`).
pub const KEY_SESSION: &str = "flow.session";

/// `discovery.js#DEFAULT_LEVEL`: the level a person who never moved the slider
/// has.
pub const DEFAULT_LEVEL: f64 = 0.35;
/// `flow.js#STYLES_MAX`: six genres is what two short rows hold on a phone.
pub const STYLES_MAX: usize = 6;
/// `flow.js#PRESSES_MAX`: how many "plus / moins comme ça" a session remembers.
pub const PRESSES_MAX: usize = 20;
/// `flow.js#HEARD_MAX`: how many titles the server reads as what was heard.
pub const HEARD_MAX: usize = 40;
/// `flow.js#PICK_AHEAD`: the next title is chosen this many seconds before the
/// end (`FlowUpNext.svelte` says it in words).
pub const PICK_AHEAD_S: u32 = 20;

/// One energy notch, as the server names it (`energy.py#STEPS`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnergyStep {
    pub name: String,
    /// `None` is "Libre" — the Flow follows the person's own listening.
    pub value: Option<f64>,
    pub hint: String,
}

/// The six notches of `flow.js#ENERGY_STEPS`, in the order the row draws them.
pub fn energy_steps() -> Vec<EnergyStep> {
    [
        ("Libre", None, "Le Flow suit votre écoute"),
        ("Très calme", Some(0.1), "Doux, posé"),
        ("Calme", Some(0.3), "Tranquille"),
        ("Modéré", Some(0.5), "Entre les deux"),
        ("Énergique", Some(0.7), "Ça bouge"),
        ("Intense", Some(0.9), "À fond"),
    ]
    .into_iter()
    .map(|(name, value, hint)| EnergyStep {
        name: name.to_owned(),
        value,
        hint: hint.to_owned(),
    })
    .collect()
}

/// The notches a nudge can step through (`flow.js#NOTCHES`).
pub const NOTCHES: [f64; 5] = [0.1, 0.3, 0.5, 0.7, 0.9];

/// The name of the notch `value` is nearest to, ties upward — the SPA's
/// `energyName`, so a measured 0.62 reads "Énergique" and never a number.
pub fn energy_name(value: Option<f64>) -> String {
    let steps = energy_steps();
    let Some(value) = value else {
        return steps[0].name.clone();
    };
    let mut best = 1;
    for (index, step) in steps.iter().enumerate().skip(1) {
        let Some(candidate) = step.value else {
            continue;
        };
        let Some(best_value) = steps[best].value else {
            continue;
        };
        if (candidate - value).abs() < (best_value - value).abs() {
            best = index;
        }
    }
    steps[best].name.clone()
}

/// One stop of the Flow's diversity ladder (`flow.js#DIVERSITY_STEPS`): the same
/// discovery number, in six words a thumb can hit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiversityStep {
    pub name: String,
    pub value: f64,
    pub hint: String,
}

/// The six stops of `flow.js#DIVERSITY_STEPS`.
pub fn diversity_steps() -> Vec<DiversityStep> {
    [
        (
            "Prudent",
            0.0,
            "Presque uniquement ce que vous écoutez déjà.",
        ),
        (
            "Proche",
            0.2,
            "Votre bibliothèque et ses voisins immédiats.",
        ),
        ("Voisin", 0.4, "Des artistes proches de vos favoris."),
        (
            "Exploration",
            0.6,
            "Des genres voisins, des artistes à deux pas.",
        ),
        ("Aventure", 0.8, "Loin de vos habitudes."),
        ("Large", 1.0, "Le plus d'inconnus possible, sans boussole."),
    ]
    .into_iter()
    .map(|(name, value, hint)| DiversityStep {
        name: name.to_owned(),
        value,
        hint: hint.to_owned(),
    })
    .collect()
}

/// `flow.js#diversityIndex`: the stop a level falls on, the NEAREST one and ties
/// upward. A value from an older build (0.35) lands on "Voisin".
pub fn diversity_index(level: f64) -> usize {
    let value = if level.is_finite() {
        level.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let steps = diversity_steps();
    let mut best = 0;
    for index in 1..steps.len() {
        if (steps[index].value - value).abs() <= (steps[best].value - value).abs() {
            best = index;
        }
    }
    best
}

/// The stop `level` reads as (`flow.js#diversityStep`).
pub fn diversity_step(level: f64) -> DiversityStep {
    diversity_steps()[diversity_index(level)].clone()
}

/// `discovery.js#STEPS`: the four names the home's tuning chip and Explorer use
/// for the same number ("the two ends are Explorer's words", `flow.js`).
pub fn discovery_steps() -> Vec<DiversityStep> {
    [
        (
            "Proche",
            0.0,
            "Des titres qui sonnent comme ce que vous écoutez",
        ),
        ("Voisin", 0.25, "Les artistes proches de vos favoris"),
        (
            "Exploration",
            0.5,
            "Des genres voisins, des artistes à deux pas",
        ),
        ("Aventure", 0.75, "Loin de vos habitudes"),
    ]
    .into_iter()
    .map(|(name, value, hint)| DiversityStep {
        name: name.to_owned(),
        value,
        hint: hint.to_owned(),
    })
    .collect()
}

/// `discovery.js#stepOf`: the last step whose `from` the level reached.
pub fn discovery_step(level: f64) -> DiversityStep {
    let level = if level.is_finite() {
        level.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut out = discovery_steps().into_iter().next().expect("four steps");
    for step in discovery_steps() {
        if level >= step.value {
            out = step;
        }
    }
    out
}

/// `flow.js#flowAimSummary`: "Techno, House +1 · Voisin" — two names at most,
/// and never the word "genres" when there are none.
pub fn aim_summary(styles: &[String], level: f64) -> String {
    let names: Vec<&String> = styles
        .iter()
        .filter(|name| !name.is_empty())
        .take(STYLES_MAX)
        .collect();
    let label = if names.is_empty() {
        "Tous les styles".to_owned()
    } else if names.len() <= 2 {
        names
            .iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        format!(
            "{} +{}",
            names[0..2]
                .iter()
                .map(|name| name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            names.len() - 2
        )
    };
    format!("{label} · {}", diversity_step(level).name)
}

/// `flow.js#flowAimShort`: for a button ~150 px wide, the genre NAMES give way
/// and the breadth never does ("Voisin" must survive the ellipsis).
pub fn aim_short(styles: &[String], level: f64) -> String {
    let names: Vec<&String> = styles
        .iter()
        .filter(|name| !name.is_empty())
        .take(STYLES_MAX)
        .collect();
    let label = if names.is_empty() {
        "Tous les styles".to_owned()
    } else if names.len() <= 2 {
        names
            .iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        format!("{} genres", names.len())
    };
    format!("{label} · {}", diversity_step(level).name)
}

/// The value a flow-relevant `level`/`energy` from the wire is stored as: the
/// SPA clamps the level and refuses a non-number (`_energy_value`), and so does
/// this, so a hostile body cannot write `NaN` into the device's settings.
pub fn clamp_level(raw: f64) -> f64 {
    if raw.is_finite() {
        raw.clamp(0.0, 1.0)
    } else {
        DEFAULT_LEVEL
    }
}

/// The Flow session's own memory: the id the server keys it by and the two
/// feedback lists (`flow.js#flowSession`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub sid: String,
    #[serde(default)]
    pub more: Vec<String>,
    #[serde(default)]
    pub less: Vec<String>,
}

impl Session {
    /// Records one press, keeping the SPA's window.
    pub fn press(&mut self, more: bool, id: &str) {
        let list = if more { &mut self.more } else { &mut self.less };
        list.push(id.to_owned());
        let len = list.len();
        if len > PRESSES_MAX {
            list.drain(..len - PRESSES_MAX);
        }
    }

    /// A fresh session id: 8 random bytes, hex (`flow.js#newSid`).
    pub fn new_sid() -> String {
        let mut bytes = [0u8; 8];
        // `rand` is not a dependency of this crate; `/dev/urandom` is what the
        // platform guarantee actually is, and a fallback keeps a broken
        // environment from panicking on a button press.
        if std::fs::File::open("/dev/urandom")
            .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut bytes))
            .is_err()
        {
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or(0);
            let mut value = seed ^ (std::process::id() as u128) << 32;
            for byte in &mut bytes {
                byte.clone_from(&(value as u8));
                value = value.rotate_left(11) ^ (value >> 7);
            }
        }
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

/// What the device remembers of the Flow's steering.
#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    /// The starting notch (`None`: "Libre").
    pub energy: Option<f64>,
    /// The genres the Flow rests on, `[]` for the whole taste.
    pub styles: Vec<String>,
    /// How far it strays, 0…1.
    pub level: f64,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            energy: None,
            styles: Vec::new(),
            level: DEFAULT_LEVEL,
        }
    }
}

impl Core {
    /// The device's copy: no request, no lock held long enough to matter — it is
    /// what the hero paints before anything else.
    pub fn home_prefs(&self) -> Prefs {
        let energy = self
            .cache()
            .kv()
            .get::<Option<f64>>(KEY_ENERGY)
            .flatten()
            .map(clamp_level);
        let styles = self
            .cache()
            .kv()
            .get::<Vec<String>>(KEY_STYLES)
            .unwrap_or_default()
            .into_iter()
            .filter(|name| !name.is_empty())
            .take(STYLES_MAX)
            .collect();
        let level = self
            .cache()
            .kv()
            .get::<f64>(KEY_LEVEL)
            .map(clamp_level)
            .unwrap_or(DEFAULT_LEVEL);
        Prefs {
            energy,
            styles,
            level,
        }
    }

    /// Stores the notch on the device (the caller sends it to the server — see
    /// [`Core::save_energy`]).
    pub fn store_energy(&self, energy: Option<f64>) {
        let _ = self.cache().kv().set(KEY_ENERGY, &energy.map(clamp_level));
    }

    /// Stores the chosen styles; never more than the six the server accepts.
    pub fn store_styles(&self, styles: &[String]) {
        let kept: Vec<String> = styles
            .iter()
            .filter(|name| !name.is_empty())
            .take(STYLES_MAX)
            .cloned()
            .collect();
        let _ = self.cache().kv().set(KEY_STYLES, &kept);
    }

    /// Stores the discovery level on the device.
    pub fn store_level(&self, level: f64) {
        let _ = self.cache().kv().set(KEY_LEVEL, &clamp_level(level));
    }

    /// The Flow session's memory, or a fresh one.
    pub fn flow_session(&self) -> Session {
        self.cache()
            .kv()
            .get::<Session>(KEY_SESSION)
            .unwrap_or_default()
    }

    /// Writes it back (small: an id and two lists of at most 20 ids).
    pub fn store_flow_session(&self, session: &Session) {
        let _ = self.cache().kv().set(KEY_SESSION, session);
    }
}

/// The in-memory guard around [`Core::flow_session`]: two presses in a row must
/// not lose one another between the read and the write.
static SESSION_LOCK: Mutex<()> = Mutex::new(());

/// Runs `f` with the session read and written under one lock.
pub fn update_session(core: &Core, f: impl FnOnce(&mut Session)) -> Session {
    let _guard = SESSION_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let mut session = core.flow_session();
    f(&mut session);
    core.store_flow_session(&session);
    session
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A level from an older build lands on a notch, never on a number.
    #[test]
    fn a_level_reads_as_the_nearest_stop() {
        assert_eq!(diversity_index(0.35), 2, "0.35 is 'Voisin', ties upward");
        assert_eq!(diversity_index(0.0), 0);
        assert_eq!(diversity_index(0.3), 1, "0.3 is nearer 0.2 than 0.4");
        assert_eq!(diversity_index(0.9), 5);
        assert_eq!(diversity_index(7.0), 5, "clamped");
        assert_eq!(diversity_index(f64::NAN), 0, "a NaN is the first stop");
    }

    /// The home's chip and the Flow's panel name the same number differently on
    /// purpose ("the two ends are Explorer's words"): four steps there, six here.
    #[test]
    fn the_two_ladders_name_the_same_level_their_own_way() {
        assert_eq!(discovery_step(0.35).name, "Voisin");
        assert_eq!(discovery_step(0.0).name, "Proche");
        assert_eq!(discovery_step(1.0).name, "Aventure");
        assert_eq!(diversity_step(0.35).name, "Voisin");
        assert_eq!(diversity_step(0.0).name, "Prudent");
        assert_eq!(discovery_steps().len(), 4);
        assert_eq!(diversity_steps().len(), 6);
    }

    /// The notch's name, never its number; "Libre" is the absence of a notch.
    #[test]
    fn a_notch_is_named_the_way_the_row_draws_it() {
        assert_eq!(energy_name(None), "Libre");
        assert_eq!(energy_name(Some(0.1)), "Très calme");
        assert_eq!(energy_name(Some(0.62)), "Énergique");
        assert_eq!(energy_name(Some(0.9)), "Intense");
        assert_eq!(energy_steps().len(), 6);
    }

    /// The aim, in a chip's width: two names at most, the breadth always.
    #[test]
    fn the_aim_says_two_genres_and_never_hides_the_breadth() {
        let none: Vec<String> = vec![];
        assert_eq!(aim_summary(&none, 0.4), "Tous les styles · Voisin");
        assert_eq!(aim_short(&none, 0.4), "Tous les styles · Voisin");
        let two = vec!["Techno".to_owned(), "House".to_owned()];
        assert_eq!(aim_summary(&two, 0.4), "Techno, House · Voisin");
        let four = vec![
            "Techno".to_owned(),
            "House".to_owned(),
            "Disco".to_owned(),
            "Funk".to_owned(),
        ];
        assert_eq!(aim_summary(&four, 0.4), "Techno, House +2 · Voisin");
        assert_eq!(
            aim_short(&four, 0.4),
            "4 genres · Voisin",
            "a narrow button keeps the breadth and counts the names"
        );
    }

    /// A press is remembered, and the window is the SPA's twenty.
    #[test]
    fn the_feedback_window_is_the_one_the_server_reads() {
        let mut session = Session::default();
        for n in 0..25 {
            session.press(true, &format!("t{n}"));
        }
        assert_eq!(session.more.len(), PRESSES_MAX);
        assert_eq!(session.more.first().map(String::as_str), Some("t5"));
        assert!(session.less.is_empty());
    }

    /// A session id is eight hex bytes, and two of them differ.
    #[test]
    fn a_session_id_is_sixteen_hex_characters() {
        let first = Session::new_sid();
        let second = Session::new_sid();
        assert_eq!(first.len(), 16);
        assert!(first.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }
}
