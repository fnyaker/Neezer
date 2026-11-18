//! The equalizer's presets and the rules around them (`eqpresets.js`).
//!
//! A preset captures the WHOLE listening setup — the equalizer's switch, the ten
//! band gains, the bass lift and the normalization level — because "my setup for
//! headphones" or "my setup for the car" is a complete state, not a curve. The
//! five built-ins ship in the code and are never writable; the person's own are
//! rows in `kv` and are the only kind "Mettre à jour" touches.
//!
//! Two rules here are the ones the SPA learned the hard way (their comments in
//! `eqpresets.js` and `docs/agents/playback.md` are the source):
//!
//! - **"Mettre à jour" writes the ten bands and the bass lift, and only the
//!   fields the preset already carries**: a preset stored by an older build
//!   without a curve keeps its shape instead of gaining a half-defined one; its
//!   EQ flag and normalization level describe how it is APPLIED, not the curve
//!   the person just shaped.
//! - **What looks dirty is what gets written**: the card's "modifié" and the
//!   button's "would this change anything" are the same comparison.

use serde::{Deserialize, Serialize};

/// The ten sliders, low to high: the count every rule here assumes.
pub const EQ_BAND_COUNT: usize = 10;

/// The equalizer's range, ±16 dB (`graph.js#EQ_MIN_DB/EQ_MAX_DB`, and the DSP's
/// own clamp).
pub const EQ_MIN_DB: f32 = -16.0;
pub const EQ_MAX_DB: f32 = 16.0;

/// One built-in curve: its id, its French name, its ten gains.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Builtin {
    pub id: &'static str,
    pub name: &'static str,
    pub bands: [f32; EQ_BAND_COUNT],
}

/// The curves shipped with the app (`BUILTIN_PRESETS`), in the card's order.
pub const BUILTIN_PRESETS: [Builtin; 5] = [
    Builtin {
        id: "flat",
        name: "Plat",
        bands: [0.0; 10],
    },
    Builtin {
        id: "bass",
        name: "Basses+",
        bands: [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    },
    Builtin {
        id: "vocal",
        name: "Voix",
        bands: [-2.0, -1.0, 0.0, 2.0, 4.0, 4.0, 3.0, 1.0, 0.0, -1.0],
    },
    Builtin {
        id: "treble",
        name: "Aigus+",
        bands: [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 4.0, 5.0, 6.0],
    },
    Builtin {
        id: "loud",
        name: "Loudness",
        bands: [6.0, 4.0, 2.0, 0.0, -1.0, -1.0, 0.0, 2.0, 4.0, 6.0],
    },
];

/// A preset as the card draws it: built-in or the person's own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    #[serde(default)]
    pub id: String,
    /// The name in the chip; the built-ins' is their French label.
    #[serde(default)]
    pub name: String,
    /// True for a shipped curve: no stored row, never writable.
    #[serde(default)]
    pub builtin: bool,
    /// The equalizer's switch as the preset applies it.
    #[serde(default)]
    pub eq: bool,
    /// The ten band gains. `None` on a row stored by a build that had no curve.
    #[serde(default)]
    pub bands: Option<Vec<f32>>,
    /// The bass lift, 0..1.
    #[serde(default)]
    pub bass: Option<f32>,
    /// `off` | `low` | `medium` | `high`.
    #[serde(default = "default_norm")]
    pub norm: String,
}

fn default_norm() -> String {
    "off".to_owned()
}

/// The chain a preset restores, and what "the preset I am on" is compared to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chain<'a> {
    pub eq: bool,
    pub bands: &'a [f32],
    pub bass: f32,
    pub norm: &'a str,
}

/// The list the card renders: the five built-ins (flagged) then the person's
/// own, both with a `name` so the markup has one field to read. Malformed rows
/// from an old storage are dropped rather than painted as broken chips.
pub fn preset_items(custom: &[Preset]) -> Vec<Preset> {
    let mut items: Vec<Preset> = BUILTIN_PRESETS
        .iter()
        .map(|b| Preset {
            id: b.id.to_owned(),
            name: b.name.to_owned(),
            builtin: true,
            eq: true,
            bands: Some(b.bands.to_vec()),
            bass: None,
            norm: default_norm(),
        })
        .collect();
    items.extend(
        custom
            .iter()
            .filter(|p| !p.id.is_empty())
            .cloned()
            .map(|p| {
                let mut p = p;
                p.builtin = false;
                if p.name.trim().is_empty() {
                    p.name = "Sans nom".to_owned();
                }
                p
            }),
    );
    items
}

/// A built-in cannot be updated or deleted: it lives in the code, not storage.
pub fn is_builtin(preset: &Preset) -> bool {
    preset.builtin || BUILTIN_PRESETS.iter().any(|b| b.id == preset.id)
}

/// A stored preset the person owns: the only kind "Mettre à jour" touches.
pub fn is_user_preset(preset: &Preset) -> bool {
    !is_builtin(preset) && !preset.id.is_empty()
}

/// Same ten gains, in order. False for a curve of the wrong length.
pub fn bands_equal(a: &[f32], b: &[f32]) -> bool {
    a.len() == EQ_BAND_COUNT && b.len() == EQ_BAND_COUNT && a.iter().zip(b).all(|(x, y)| x == y)
}

/// A stored curve this build can read (`bandArray`): exactly ten finite gains.
pub fn band_array(bands: Option<&Vec<f32>>) -> Option<&[f32]> {
    bands
        .map(Vec::as_slice)
        .filter(|b| b.len() == EQ_BAND_COUNT && b.iter().all(|v| v.is_finite()))
}

fn same_bass(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.005
}

/// A saved preset is the current chain when it agrees on everything it restores:
/// the equalizer's switch, the curve, the bass lift and the normalization level.
pub fn preset_matches(preset: &Preset, chain: Chain<'_>) -> bool {
    if preset.eq != chain.eq || preset.norm != chain.norm {
        return false;
    }
    if !same_bass(preset.bass.unwrap_or(0.0), chain.bass) {
        return false;
    }
    match band_array(preset.bands.as_ref()) {
        Some(bands) => bands_equal(bands, chain.bands),
        None => false,
    }
}

/// The preset the card highlights. A user preset wins over a built-in with the
/// same curve: it also carries the switch and the level, so it is the more
/// precise answer. A built-in matches on its curve alone, which is all it
/// prescribes.
pub fn active_preset_id(items: &[Preset], chain: Chain<'_>) -> Option<String> {
    if let Some(custom) = items
        .iter()
        .find(|p| !is_builtin(p) && preset_matches(p, chain))
    {
        return Some(custom.id.clone());
    }
    items
        .iter()
        .find(|p| is_builtin(p) && preset_matches_builtin(p, chain))
        .map(|p| p.id.clone())
}

fn preset_matches_builtin(preset: &Preset, chain: Chain<'_>) -> bool {
    match band_array(preset.bands.as_ref()) {
        Some(bands) => bands_equal(bands, chain.bands),
        None => false,
    }
}

/// Does the preset carry anything "Mettre à jour" could write? (`presetUpdatable`)
pub fn preset_updatable(preset: &Preset) -> bool {
    if !is_user_preset(preset) {
        return false;
    }
    band_array(preset.bands.as_ref()).is_some() || preset.bass.is_some_and(|b| b.is_finite())
}

/// Would "Mettre à jour" change the preset? Only the two fields it writes are
/// compared — a preset whose normalization level differs from the live chain is
/// not "modified" for this button, since pressing it would not change that.
pub fn preset_needs_update(preset: &Preset, chain: Chain<'_>) -> bool {
    if !is_user_preset(preset) {
        return false;
    }
    if let Some(bands) = band_array(preset.bands.as_ref())
        && !bands_equal(bands, chain.bands)
    {
        return true;
    }
    if let Some(bass) = preset.bass
        && bass.is_finite()
        && !same_bass(bass, chain.bass)
    {
        return true;
    }
    false
}

/// The preset to store for "Mettre à jour", or `None` when it cannot be written.
/// Pure: the caller hands the result back to the store.
pub fn updated_preset(preset: &Preset, chain: Chain<'_>) -> Option<Preset> {
    if !preset_updatable(preset) {
        return None;
    }
    let mut next = preset.clone();
    if band_array(preset.bands.as_ref()).is_some() && chain.bands.len() == EQ_BAND_COUNT {
        next.bands = Some(
            chain
                .bands
                .iter()
                .map(|v| if v.is_finite() { *v } else { 0.0 })
                .collect(),
        );
    }
    if let Some(bass) = preset.bass
        && bass.is_finite()
        && chain.bass.is_finite()
    {
        next.bass = Some(chain.bass.clamp(0.0, 1.0));
    }
    Some(next)
}

/// The snapshot "Enregistrer comme…" stores (`saveFxPreset`): the whole chain,
/// under a name of at most 40 characters. Saving under an existing name
/// overwrites that row — same id — rather than piling up near-duplicates.
pub fn save_preset(
    custom: &mut Vec<Preset>,
    name: &str,
    chain: Chain<'_>,
    id: &str,
) -> Option<Preset> {
    let clean = name.trim();
    if clean.is_empty() {
        return None;
    }
    let clean: String = clean.chars().take(40).collect();
    let snapshot = Preset {
        id: id.to_owned(),
        name: clean.clone(),
        builtin: false,
        eq: chain.eq,
        bands: Some(chain.bands.to_vec()),
        bass: Some(chain.bass.clamp(0.0, 1.0)),
        norm: chain.norm.to_owned(),
    };
    match custom
        .iter()
        .position(|p| p.name.to_lowercase() == clean.to_lowercase())
    {
        Some(index) => {
            // Re-saving keeps the stored row's id, so a card that selected it by
            // id keeps selecting it.
            let kept = Preset {
                id: custom[index].id.clone(),
                ..snapshot.clone()
            };
            custom[index] = kept.clone();
            Some(kept)
        }
        None => {
            custom.push(snapshot.clone());
            Some(snapshot)
        }
    }
}

/// Apply a preset's chain (the caller then writes the settings it names).
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedChain {
    pub eq: bool,
    pub bands: Vec<f32>,
    pub bass: f32,
    pub norm: String,
}

/// What a preset restores (`applyFxPreset`): its curve if it has a readable one,
/// its switch, its bass lift and its level. An unreadable field leaves the
/// caller's value alone, which is what the SPA does (it only sets what it reads).
pub fn apply_preset(preset: &Preset, current_bands: &[f32]) -> AppliedChain {
    let bands = match band_array(preset.bands.as_ref()) {
        Some(bands) => bands.to_vec(),
        None => current_bands.to_vec(),
    };
    let bass = preset
        .bass
        .filter(|b| b.is_finite())
        .map(|b| b.clamp(0.0, 1.0))
        .unwrap_or(0.0);
    let norm = match preset.norm.as_str() {
        "low" | "medium" | "high" => preset.norm.clone(),
        _ => "off".to_owned(),
    };
    AppliedChain {
        eq: preset.eq,
        bands,
        bass,
        norm,
    }
}

/// A stored row this build can read; a malformed one is dropped (`presetItems`).
pub fn parse_presets(json: Option<&str>) -> Vec<Preset> {
    let Some(json) = json else {
        return Vec::new();
    };
    let parsed: Vec<Preset> = serde_json::from_str(json).unwrap_or_default();
    parsed.into_iter().filter(|p| !p.id.is_empty()).collect()
}

pub fn presets_json(presets: &[Preset]) -> String {
    serde_json::to_string(presets).unwrap_or_else(|_| "[]".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain<'a>(bands: &'a [f32], bass: f32, eq: bool, norm: &'a str) -> Chain<'a> {
        Chain {
            eq,
            bands,
            bass,
            norm,
        }
    }

    fn user(id: &str, name: &str, bands: [f32; 10], bass: f32, eq: bool) -> Preset {
        Preset {
            id: id.to_owned(),
            name: name.to_owned(),
            builtin: false,
            eq,
            bands: Some(bands.to_vec()),
            bass: Some(bass),
            norm: "off".to_owned(),
        }
    }

    /// The five built-ins are the SPA's, in its order, with its curves.
    #[test]
    fn the_builtins_are_the_spas() {
        let items = preset_items(&[]);
        assert_eq!(items.len(), 5);
        assert_eq!(
            items.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["Plat", "Basses+", "Voix", "Aigus+", "Loudness"]
        );
        assert_eq!(items[0].bands.as_ref().unwrap(), &[0.0; 10]);
        assert_eq!(
            items[1].bands.as_ref().unwrap(),
            &[6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
        );
        assert!(items.iter().all(is_builtin));
    }

    /// The card highlights the preset the chain matches; a user preset wins over
    /// a built-in with the same curve (it also carries the switch and the level).
    #[test]
    fn the_active_preset_is_the_one_the_chain_matches() {
        let bands = [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let mine = user("p1", "Casque", bands, 0.0, true);
        let items = preset_items(&[mine]);
        assert_eq!(
            active_preset_id(&items, chain(&bands, 0.0, true, "off")).as_deref(),
            Some("p1"),
            "the person's own preset wins over the built-in it copies"
        );
        // Without it, the built-in with that curve is the answer.
        let items = preset_items(&[]);
        assert_eq!(
            active_preset_id(&items, chain(&bands, 0.0, true, "off")).as_deref(),
            Some("bass")
        );
        // A curve nobody ships matches nothing.
        assert_eq!(
            active_preset_id(&items, chain(&[1.0; 10], 0.0, true, "off")),
            None
        );
        // Bass and level are part of a user preset's identity, not a built-in's.
        let items = preset_items(&[user("p2", "Auto", [0.0; 10], 0.4, true)]);
        assert_eq!(
            active_preset_id(&items, chain(&[0.0; 10], 0.2, true, "off")),
            Some("flat".to_owned()),
            "the built-in matches its curve alone; the user preset does not match"
        );
    }

    /// "Mettre à jour" writes the curve and the bass lift, never the switch nor
    /// the level, and "modifié" is the same comparison.
    #[test]
    fn an_update_writes_the_curve_and_the_lift_only() {
        let stored = user("p1", "Voiture", [0.0; 10], 0.0, true);
        let live = chain(&[3.0; 10], 0.5, true, "high");
        assert!(preset_needs_update(&stored, live));
        let next = updated_preset(&stored, live).expect("a user preset is writable");
        assert_eq!(next.bands.as_deref(), Some(&[3.0f32; 10][..]));
        assert_eq!(next.bass, Some(0.5));
        assert_eq!(
            next.norm, "off",
            "the level the preset applies is untouched"
        );
        assert!(!preset_needs_update(&next, live), "clean after the write");

        // A built-in has no row: nothing to update, and no delete.
        let flat = preset_items(&[]).into_iter().next().unwrap();
        assert_eq!(updated_preset(&flat, live), None);
        // A preset whose stored curve is unreadable keeps its shape.
        let mut old = stored.clone();
        old.bands = Some(vec![1.0, 2.0]);
        assert_eq!(
            updated_preset(&old, live).unwrap().bands,
            Some(vec![1.0, 2.0])
        );
        // Only a bass-only row takes the lift alone.
        let mut bass_only = user("p3", "Grave", [0.0; 10], 0.1, true);
        bass_only.bands = None;
        let next = updated_preset(&bass_only, live).unwrap();
        assert_eq!(next.bands, None);
        assert_eq!(next.bass, Some(0.5));
    }

    /// Saving: a name is trimmed and bounded, a re-save keeps the stored row's
    /// id, and an empty name saves nothing at all.
    #[test]
    fn saving_a_preset_keeps_one_row_per_name() {
        let mut custom = Vec::new();
        let live = chain(&[2.0; 10], 0.3, true, "low");
        let first = save_preset(&mut custom, "  Casque  ", live, "p1").expect("saved");
        assert_eq!(first.name, "Casque");
        assert_eq!(first.norm, "low");
        assert_eq!(custom.len(), 1);
        // The same name again overwrites, with the FIRST id kept.
        let again = save_preset(&mut custom, "casque", live, "p2").expect("saved");
        assert_eq!(again.id, "p1");
        assert_eq!(custom.len(), 1);
        // Bounded at 40 characters, and an empty name is refused.
        let long = "x".repeat(60);
        let saved = save_preset(&mut custom, &long, live, "p3").expect("saved");
        assert_eq!(saved.name.chars().count(), 40);
        assert_eq!(save_preset(&mut custom, "   ", live, "p4"), None);
    }

    /// Applying a preset restores what it carries and leaves the rest alone.
    #[test]
    fn applying_a_preset_restores_its_chain() {
        let preset = user("p1", "Nuit", [-4.0; 10], 1.4, false);
        let applied = apply_preset(&preset, &[1.0; 10]);
        assert_eq!(applied.bands, vec![-4.0f32; 10]);
        assert_eq!(applied.bass, 1.0, "clamped to 0..1");
        assert!(!applied.eq);
        assert_eq!(applied.norm, "off");
        // A curve the build cannot read leaves the live one in place.
        let mut broken = preset.clone();
        broken.bands = Some(vec![1.0; 3]);
        assert_eq!(apply_preset(&broken, &[1.0; 10]).bands, vec![1.0f32; 10]);
        // An unknown level is "off", never a level this build cannot apply.
        broken.norm = "maximum".to_owned();
        assert_eq!(apply_preset(&broken, &[0.0; 10]).norm, "off");
    }

    /// Old storage: malformed rows are dropped, and a row without a curve or a
    /// level still reads (an older build stored less).
    #[test]
    fn stored_presets_are_read_defensively() {
        let json = r#"[
            {"id":"p1","name":"Casque","eq":true,"bands":[0,1,2,3,4,5,6,7,8,9],"bass":0.5},
            {"id":"","name":"broken"},
            {"name":"no id either"}
        ]"#;
        let rows = parse_presets(Some(json));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Casque");
        assert_eq!(rows[0].norm, "off", "a missing level is off");
        assert_eq!(parse_presets(Some("not json")).len(), 0);
        assert_eq!(parse_presets(None).len(), 0);
        // And a round trip keeps them.
        let back = parse_presets(Some(&presets_json(&rows)));
        assert_eq!(back, rows);
    }
}
