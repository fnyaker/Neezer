//! Where the sound goes, and the verdict on the output it is going to
//! (`webapp/src/lib/audio/output.js`, `android/.../AudioRoute.kt`).
//!
//! # Why a phone needs this at all
//!
//! In the SPA the choice is real: `direct` hands the element to the browser's
//! own media pipeline, `graph` routes it through Web Audio, and `auto` starts
//! processed and falls back to direct **on the connection that measured a
//! fault**. The native app has ONE path — its Rust player — so the decision is
//! always "processed", and the card says so with the reason the SPA already has
//! for its own native player (`plan.why === "native"`, « Lecteur Android »).
//!
//! What does NOT disappear on the phone is the rest of the policy, and it is
//! ported whole because it is what the card explains and what a later direct
//! path will obey:
//!
//! - **a route** — kind, name and the CONNECTION number `AudioRoute.kt` bumps
//!   every time the output comes back (a verdict is about one connection of one
//!   output, never about "Bluetooth" in general);
//! - **a verdict per connection**, with the doubling backoff (15 min → 4 h), the
//!   probe, and the six good windows in a row that close it;
//! - **"Réessayer maintenant"**, which marks the verdict probing at once.
//!
//! The verdicts live in `kv` (the SPA's `audio.glitchRoutes`), so a phone that
//! measured a fault on a car radio yesterday still remembers it today — and
//! forgets it after a week (`VERDICT_MAX_AGE`).

use serde::{Deserialize, Serialize};

/// The first retry of the processed path after a verdict (`RETRY_FIRST`).
pub const RETRY_FIRST_MS: f64 = 15.0 * 60.0 * 1000.0;
/// The ceiling the doubling stops at (`RETRY_MAX`): 4 h.
pub const RETRY_MAX_MS: f64 = 4.0 * 3600.0 * 1000.0;
/// Clean windows in a row, on the processed path, that close a verdict.
pub const GOOD_TO_CLEAR: u32 = 6;
/// A verdict nobody has touched in this long is dropped (`VERDICT_MAX_AGE`).
pub const VERDICT_MAX_AGE_MS: f64 = 7.0 * 24.0 * 3600.0 * 1000.0;
/// A context that reports this much output latency on Android is on a wireless
/// link (`BT_LATENCY`): a phone's own speaker or a wired headset sits at
/// 20-60 ms, Bluetooth at 150-300 ms.
pub const BT_LATENCY_S: f64 = 0.15;

/// The outputs, as `AudioRoute.kt` names them.
pub const KINDS: [&str; 7] = [
    "bluetooth",
    "wired",
    "usb",
    "speaker",
    "hdmi",
    "other",
    "unknown",
];

/// The French name of a kind (`AudioEffects.svelte#KIND_LABEL`).
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "bluetooth" => "Bluetooth",
        "wired" => "Filaire",
        "usb" => "USB",
        "speaker" => "Haut-parleur",
        "hdmi" => "HDMI",
        "other" => "Autre sortie",
        _ => "",
    }
}

/// One output, as the rest of the app reads it (`normalizeRoute`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub kind: String,
    pub name: String,
    pub key: String,
    pub conn: String,
    /// `native` when the app's own player answered, `latency` for a route
    /// inferred from the context's latency, `none` otherwise.
    pub source: String,
}

impl Default for Route {
    fn default() -> Self {
        Route {
            kind: "unknown".to_owned(),
            name: String::new(),
            key: "default".to_owned(),
            conn: String::new(),
            source: "none".to_owned(),
        }
    }
}

impl Route {
    /// The card's own sentence for the output playing now: « Bluetooth ·
    /// KMM-BT309 », « Bluetooth probable (latence de sortie) », or nothing when
    /// the platform could not say.
    pub fn describe(&self) -> String {
        if self.kind == "unknown" {
            return String::new();
        }
        if self.source == "latency" {
            return "Bluetooth probable (latence de sortie)".to_owned();
        }
        let label = kind_label(&self.kind);
        if self.name.is_empty() {
            label.to_owned()
        } else {
            format!("{label} · {}", self.name)
        }
    }
}

/// A route as the rest of the app reads it, whatever the platform sent: the kind
/// off the known list, a bounded one-line name (a Bluetooth device names
/// itself), a key derived from the kind and the name.
pub fn normalize_route(kind: &str, name: &str, conn: &str, source: &str) -> Route {
    let kind = if KINDS.contains(&kind) { kind } else { "other" };
    let name: String = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(60)
        .collect();
    let key = if kind == "unknown" {
        "default".to_owned()
    } else {
        format!("{kind}:{}", name.to_lowercase())
    };
    Route {
        kind: kind.to_owned(),
        name,
        key,
        conn: conn.chars().take(40).collect(),
        source: match source {
            "latency" => "latency".to_owned(),
            "native" => "native".to_owned(),
            _ => "none".to_owned(),
        },
    }
}

/// The mode the person chose (`stores.js#audioOutput`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Auto,
    Direct,
    Graph,
}

impl Mode {
    pub fn parse(value: &str) -> Mode {
        match value {
            "direct" => Mode::Direct,
            "graph" => Mode::Graph,
            _ => Mode::Auto,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Direct => "direct",
            Mode::Graph => "graph",
        }
    }
}

/// Which path the sound takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    /// Through the app's processor: the effects apply.
    Processed,
    /// Untouched, straight to the system: the effects are off.
    Direct,
}

impl Path {
    pub fn as_str(self) -> &'static str {
        match self {
            Path::Processed => "processed",
            Path::Direct => "direct",
        }
    }
}

/// Why the path is the one it is (`output.js`'s `why`), plus the app's own
/// player, which has no graph to choose from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// The person's setting.
    Setting,
    /// A fault was measured on this connection and is not being probed.
    Glitch,
    /// Nothing measured: the default.
    Default,
    /// The verdict exists and is being retried.
    Probing,
    /// The app's own player: the sound never leaves the engine.
    Native,
}

impl Why {
    pub fn as_str(self) -> &'static str {
        match self {
            Why::Setting => "setting",
            Why::Glitch => "glitch",
            Why::Default => "default",
            Why::Probing => "probing",
            Why::Native => "native",
        }
    }
}

/// The decision, as a value (`decideOutput`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plan {
    pub path: Path,
    pub why: Why,
    /// When the processed path is due for another try (`why == Glitch`).
    pub retry_at: Option<f64>,
}

/// One measured fault, about ONE connection of one output (`glitchRoutes`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    /// When it was measured, ms since the epoch.
    pub at: f64,
    /// The connection it is about: `AudioRoute.kt`'s `<boot>-<count>`.
    #[serde(default)]
    pub conn: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    /// What the detector said, one line (a diagnostic, never shown as a cause).
    #[serde(default)]
    pub why: String,
    #[serde(default)]
    pub ratio: Option<f64>,
    #[serde(default = "one")]
    pub count: u32,
    /// How long until the next try, doubling up to `RETRY_MAX_MS`.
    #[serde(default = "first")]
    pub backoff: f64,
    /// `at + backoff`: when a retry is due.
    #[serde(default)]
    pub retry_at: f64,
    /// The verdict is being retried right now (the processed path is in use).
    #[serde(default)]
    pub probing: bool,
    /// Consecutive windows that kept time while probing.
    #[serde(default)]
    pub good: u32,
}

fn one() -> u32 {
    1
}

fn first() -> f64 {
    RETRY_FIRST_MS
}

/// A stored verdict still describes the output playing now: same output, same
/// CONNECTION of it, and not stale (`sameConnection`).
pub fn same_connection(verdict: Option<&Verdict>, route: &Route, now: f64) -> bool {
    match verdict {
        None => false,
        Some(v) => {
            now - v.at < VERDICT_MAX_AGE_MS && v.at <= now + 60_000.0 && v.conn == route.conn
        }
    }
}

/// The decision, as a value. `native` is the app's own player: it has no graph to
/// route sound through, so it always processes, and the mode is only reported.
pub fn decide_output(
    native: bool,
    mode: Mode,
    route: &Route,
    verdicts: &Verdicts,
    now: f64,
) -> Plan {
    if native {
        return Plan {
            path: Path::Processed,
            why: Why::Native,
            retry_at: None,
        };
    }
    let verdict = verdicts.get(&route.key);
    match mode {
        Mode::Direct => Plan {
            path: Path::Direct,
            why: Why::Setting,
            retry_at: None,
        },
        Mode::Graph => Plan {
            path: Path::Processed,
            why: Why::Setting,
            retry_at: None,
        },
        Mode::Auto => {
            let live = same_connection(verdict, route, now);
            if live && !verdict.is_some_and(|v| v.probing) {
                Plan {
                    path: Path::Direct,
                    why: Why::Glitch,
                    retry_at: verdict.map(|v| v.retry_at),
                }
            } else if live {
                Plan {
                    path: Path::Processed,
                    why: Why::Probing,
                    retry_at: None,
                }
            } else {
                Plan {
                    path: Path::Processed,
                    why: Why::Default,
                    retry_at: None,
                }
            }
        }
    }
}

/// The verdicts, one per output key (`glitchRoutes` is the SPA's map).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Verdicts(std::collections::BTreeMap<String, Verdict>);

impl std::ops::Deref for Verdicts {
    type Target = std::collections::BTreeMap<String, Verdict>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Verdicts {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Read the stored map; a malformed row is dropped rather than fought over.
pub fn parse_verdicts(json: Option<&str>) -> Verdicts {
    let Some(json) = json else {
        return Verdicts::default();
    };
    Verdicts(serde_json::from_str(json).unwrap_or_default())
}

pub fn verdicts_json(verdicts: &Verdicts) -> String {
    serde_json::to_string(&verdicts.0).unwrap_or_else(|_| "{}".to_owned())
}

impl Verdicts {
    /// Write one output's verdict, dropping every expired one on the way
    /// (`writeVerdict`).
    pub fn write(&mut self, key: &str, entry: Option<Verdict>, now: f64) {
        self.retain(|_, v| now - v.at < VERDICT_MAX_AGE_MS);
        match entry {
            Some(verdict) => {
                self.insert(key.to_owned(), verdict);
            }
            None => {
                self.remove(key);
            }
        }
    }

    /// The verdicts still on record, newest first, for the settings screen
    /// (`activeGlitches`).
    pub fn active(&self, now: f64) -> Vec<(String, Verdict)> {
        let mut rows: Vec<(String, Verdict)> = self
            .iter()
            .filter(|(_, v)| now - v.at < VERDICT_MAX_AGE_MS && v.at <= now + 60_000.0)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        rows.sort_by(|a, b| {
            b.1.at
                .partial_cmp(&a.1.at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        rows
    }
}

/// What a detector hands in when it measured a fault.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Glitch {
    pub why: String,
    pub ratio: Option<f64>,
}

/// The timekeeper's verdict: the processed path lost audio on the current
/// output. Direct for this connection until a retry; a fault found again by a
/// retry waits twice as long for the next one. Answers true when that changes
/// what plays in `auto` mode (`reportGlitch`).
pub fn report_glitch(
    verdicts: &mut Verdicts,
    route: &Route,
    info: Glitch,
    now: f64,
    mode: Mode,
) -> bool {
    let prev = verdicts.get(&route.key).cloned();
    let again = same_connection(prev.as_ref(), route, now);
    let backoff = if again {
        let prev = prev.as_ref().map(|v| v.backoff).unwrap_or(RETRY_FIRST_MS);
        (prev * 2.0).clamp(RETRY_FIRST_MS, RETRY_MAX_MS)
    } else {
        RETRY_FIRST_MS
    };
    let count = if again {
        prev.as_ref().map(|v| v.count).unwrap_or(1) + 1
    } else {
        1
    };
    verdicts.write(
        &route.key,
        Some(Verdict {
            at: now,
            conn: route.conn.clone(),
            name: route.name.clone(),
            kind: route.kind.clone(),
            why: info.why.chars().take(120).collect(),
            ratio: info.ratio.map(|r| (r * 10_000.0).round() / 10_000.0),
            count,
            backoff,
            retry_at: now + backoff,
            probing: false,
            good: 0,
        }),
        now,
    );
    mode == Mode::Auto
}

/// At a track change: if the current output's verdict is due for a retry, try
/// the processed path again — the verdict STAYS, marked as being probed, until
/// the timekeeper either clears it or finds the fault again. Answers true when
/// it flipped the path (`retryIfDue`).
pub fn retry_if_due(verdicts: &mut Verdicts, route: &Route, now: f64, mode: Mode) -> bool {
    if mode != Mode::Auto {
        return false;
    }
    let Some(verdict) = verdicts.get(&route.key).cloned() else {
        return false;
    };
    if !same_connection(Some(&verdict), route, now) || verdict.probing || now < verdict.retry_at {
        return false;
    }
    verdicts.write(
        &route.key,
        Some(Verdict {
            probing: true,
            good: 0,
            ..verdict
        }),
        now,
    );
    true
}

/// A window that kept time on the processed path. While a verdict is being
/// probed, `GOOD_TO_CLEAR` of them in a row close it: the fault is gone
/// (`noteGoodWindow`).
pub fn note_good_window(verdicts: &mut Verdicts, route: &Route, now: f64) {
    let Some(verdict) = verdicts.get(&route.key).cloned() else {
        return;
    };
    if !same_connection(Some(&verdict), route, now) || !verdict.probing {
        return;
    }
    let good = verdict.good + 1;
    let entry = if good >= GOOD_TO_CLEAR {
        None
    } else {
        Some(Verdict { good, ..verdict })
    };
    verdicts.write(&route.key, entry, now);
}

/// "Réessayer maintenant" on the card: probe the processed path at once
/// (`retryNow`).
pub fn retry_now(verdicts: &mut Verdicts, key: &str, now: f64) {
    let Some(verdict) = verdicts.get(key).cloned() else {
        return;
    };
    verdicts.write(
        key,
        Some(Verdict {
            probing: true,
            good: 0,
            ..verdict
        }),
        now,
    );
}

/// Forget one output's verdict altogether (`forgetGlitch`).
pub fn forget(verdicts: &mut Verdicts, key: &str, now: f64) {
    verdicts.write(key, None, now);
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_800_000_000_000.0;

    fn bt() -> Route {
        normalize_route("bluetooth", "KMM  BT309 ", "12-3", "native")
    }

    /// The route: a known kind, a bounded one-line name, the key the verdicts
    /// live under, and the connection number the card's verdicts are scoped to.
    #[test]
    fn the_route_is_normalized_and_bounded() {
        let route = bt();
        assert_eq!(route.kind, "bluetooth");
        assert_eq!(route.name, "KMM BT309", "whitespace runs collapse");
        assert_eq!(route.key, "bluetooth:kmm bt309");
        assert_eq!(route.conn, "12-3");
        assert_eq!(route.source, "native");
        assert_eq!(route.describe(), "Bluetooth · KMM BT309");

        // A hostile name is bounded and stays one line.
        let long = "z".repeat(200);
        let route = normalize_route("wired", &long, "", "none");
        assert_eq!(route.name.chars().count(), 60);
        assert_eq!(route.describe(), format!("Filaire · {}", "z".repeat(60)));
        // An unknown kind is `other`, an unreadable one is `unknown`.
        assert_eq!(normalize_route("telepathy", "", "", "").kind, "other");
        assert_eq!(normalize_route("unknown", "", "", "").kind, "unknown");
        assert_eq!(Route::default().describe(), "");
        assert_eq!(
            normalize_route("bluetooth", "", "9-1", "latency").describe(),
            "Bluetooth probable (latence de sortie)"
        );
    }

    /// The app's own player always processes, and says why (`native`) — the SPA's
    /// own branch for its native player, not a new rule.
    #[test]
    fn the_apps_player_always_processes_the_sound() {
        let route = bt();
        let verdicts = Verdicts::default();
        for mode in [Mode::Auto, Mode::Direct, Mode::Graph] {
            let plan = decide_output(true, mode, &route, &verdicts, NOW);
            assert_eq!(plan.path, Path::Processed);
            assert_eq!(plan.why, Why::Native);
            assert_eq!(plan.retry_at, None);
        }
    }

    /// `auto` with nothing measured is the default path; a setting pins it; and a
    /// verdict on ANOTHER connection says nothing about this one.
    #[test]
    fn the_decision_follows_the_setting_and_the_connection() {
        let route = bt();
        let mut verdicts = Verdicts::default();
        assert_eq!(
            decide_output(false, Mode::Auto, &route, &verdicts, NOW),
            Plan {
                path: Path::Processed,
                why: Why::Default,
                retry_at: None
            }
        );
        assert_eq!(
            decide_output(false, Mode::Direct, &route, &verdicts, NOW).why,
            Why::Setting
        );
        assert_eq!(
            decide_output(false, Mode::Graph, &route, &verdicts, NOW).path,
            Path::Processed
        );

        // A fault on this connection: direct until the retry is due.
        report_glitch(
            &mut verdicts,
            &route,
            Glitch {
                why: "ratio 0.98".to_owned(),
                ratio: Some(0.98234),
            },
            NOW,
            Mode::Auto,
        );
        let plan = decide_output(false, Mode::Auto, &route, &verdicts, NOW);
        assert_eq!(plan.why, Why::Glitch);
        assert_eq!(plan.path, Path::Direct);
        assert_eq!(plan.retry_at, Some(NOW + RETRY_FIRST_MS));

        // The same radio reconnected is a DIFFERENT connection: the default path.
        let reconnected = normalize_route("bluetooth", "KMM BT309", "12-4", "native");
        assert_eq!(
            decide_output(false, Mode::Auto, &reconnected, &verdicts, NOW).why,
            Why::Default
        );
    }

    /// The backoff doubles on a fault found again by a retry, up to 4 h, and the
    /// count says how many times this connection has been caught.
    #[test]
    fn a_repeated_fault_waits_twice_as_long_up_to_the_ceiling() {
        let route = bt();
        let mut verdicts = Verdicts::default();
        let mut now = NOW;
        let mut expected = RETRY_FIRST_MS;
        for count in 1..=8 {
            report_glitch(&mut verdicts, &route, Glitch::default(), now, Mode::Auto);
            let verdict = verdicts.get(&route.key).cloned().expect("a verdict");
            assert_eq!(verdict.count, count);
            assert!(
                (verdict.backoff - expected).abs() < 1.0,
                "count {count}: backoff {} vs {expected}",
                verdict.backoff
            );
            expected = (expected * 2.0).min(RETRY_MAX_MS);
            now += verdict.backoff;
        }
        assert_eq!(
            verdicts.get(&route.key).unwrap().backoff,
            RETRY_MAX_MS,
            "the ceiling is 4 h"
        );
    }

    /// A retry at a track change, then six good windows close the verdict — and
    /// a fault found again mid-probe doubles the wait.
    #[test]
    fn a_retry_is_probed_then_cleared_by_six_good_windows() {
        let route = bt();
        let mut verdicts = Verdicts::default();
        report_glitch(&mut verdicts, &route, Glitch::default(), NOW, Mode::Auto);
        // Not due yet: nothing to retry at a track change.
        assert!(!retry_if_due(
            &mut verdicts,
            &route,
            NOW + 1_000.0,
            Mode::Auto
        ));
        let due = NOW + RETRY_FIRST_MS;
        assert!(retry_if_due(&mut verdicts, &route, due, Mode::Auto));
        // Probing: the processed path is in use, and the card says so.
        assert_eq!(
            decide_output(false, Mode::Auto, &route, &verdicts, due).why,
            Why::Probing
        );
        for good in 1..GOOD_TO_CLEAR {
            note_good_window(&mut verdicts, &route, due + 1_000.0);
            assert_eq!(verdicts.get(&route.key).unwrap().good, good);
        }
        note_good_window(&mut verdicts, &route, due + 1_000.0);
        assert!(
            verdicts.get(&route.key).is_none(),
            "six good windows close the verdict"
        );
    }

    /// "Réessayer maintenant" probes at once, and a verdict older than a week is
    /// dropped on the next write.
    #[test]
    fn retry_now_and_expiry() {
        let route = bt();
        let mut verdicts = Verdicts::default();
        report_glitch(&mut verdicts, &route, Glitch::default(), NOW, Mode::Auto);
        retry_now(&mut verdicts, &route.key, NOW + 10.0);
        assert!(verdicts.get(&route.key).unwrap().probing);
        assert_eq!(
            decide_output(false, Mode::Auto, &route, &verdicts, NOW + 10.0).path,
            Path::Processed
        );
        // A verdict from eight days ago belongs to no output of today.
        report_glitch(&mut verdicts, &route, Glitch::default(), NOW, Mode::Auto);
        assert!(!same_connection(
            verdicts.get(&route.key),
            &route,
            NOW + 8.0 * 24.0 * 3600.0 * 1000.0
        ));
        verdicts.write("other", None, NOW + 8.0 * 24.0 * 3600.0 * 1000.0);
        assert!(
            verdicts.is_empty(),
            "an expired verdict is dropped on the way"
        );
    }

    /// The stored map round-trips, and a malformed row is dropped rather than
    /// crashing the card.
    #[test]
    fn verdicts_round_trip_through_kv() {
        let route = bt();
        let mut verdicts = Verdicts::default();
        report_glitch(
            &mut verdicts,
            &route,
            Glitch {
                why: "x".repeat(300),
                ratio: Some(0.5),
            },
            NOW,
            Mode::Auto,
        );
        let json = verdicts_json(&verdicts);
        let back = parse_verdicts(Some(&json));
        assert_eq!(back, verdicts);
        assert!(back[&route.key].why.chars().count() <= 120);
        assert_eq!(parse_verdicts(Some("nope")).len(), 0);
        assert_eq!(parse_verdicts(None).len(), 0);
        // The screen's list is newest first.
        let mut two = Verdicts::default();
        two.write(
            "a",
            Some(Verdict {
                at: NOW,
                conn: "1".into(),
                name: String::new(),
                kind: "wired".into(),
                why: String::new(),
                ratio: None,
                count: 1,
                backoff: RETRY_FIRST_MS,
                retry_at: NOW,
                probing: false,
                good: 0,
            }),
            NOW,
        );
        two.write(
            "b",
            Some(Verdict {
                at: NOW + 5_000.0,
                conn: "1".into(),
                name: String::new(),
                kind: "usb".into(),
                why: String::new(),
                ratio: None,
                count: 1,
                backoff: RETRY_FIRST_MS,
                retry_at: NOW,
                probing: false,
                good: 0,
            }),
            NOW + 5_000.0,
        );
        let rows = two.active(NOW + 10_000.0);
        assert_eq!(rows[0].0, "b");
        assert_eq!(rows[1].0, "a");
        forget(&mut two, "b", NOW + 20_000.0);
        assert_eq!(two.active(NOW + 20_000.0).len(), 1);
    }
}
