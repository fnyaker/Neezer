//! Explorer: the ten parent styles, the all-stations list, one style's
//! discovery, and a genre station.
//!
//! This is `webapp/src/lib/genrestation.js` and the pure half of
//! `ExplorerStyles.svelte` PORTED: which rows are tiles, how a style search
//! folds, what a station is called and what may be played. The component's
//! timers (2 s poll, 60 polls, the 450 ms settle) are the ViewModel's — a core
//! never owns a timer (`docs/agents/mobile.md`, "Hot reads").

use appcore::fold;
use nz_model::{FlowStyles, Track};

/// `genrestation.js#STATION_KIND`: a station is its own playback context, never
/// the Flow's saved aim.
pub const STATION_KIND: &str = "genre-radio";
/// `genrestation.js#STATION_PREFIX`.
pub const STATION_PREFIX: &str = "Station ";
/// `ExplorerStyles.svelte#TILES_MAX`: ten parent styles read as one shelf.
pub const TILES_MAX: usize = 10;
/// `ExplorerStyles.svelte#LEVELS.len()`: the breadth slider's five notches.
pub const LEVELS: u32 = 5;
/// `ExplorerStyles.svelte#MAX_POLLS`: two minutes of polling, then the panel
/// hands the person a way out instead of an endless skeleton.
pub const MAX_POLLS: u32 = 60;
/// `ExplorerStyles.svelte#POLL_MS`.
pub const POLL_MS: u64 = 2_000;

/// One tile of "Parcourir par style" (`ExplorerStyles.svelte#parentStyles`): the
/// parent a row is DISPLAYED as, with the titles under it.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StyleTile {
    pub name: String,
    pub tracks: u32,
    /// The summed weight of the rows that merged into this tile, 0…1.
    pub weight: f64,
    /// All the rows that merged here were leaves of this parent.
    pub leaf: bool,
    /// The leaf names that merged into this parent, in the server's order.
    pub children: Vec<String>,
}

/// `ExplorerStyles.svelte#parentStyles`: the parents of the taste, merged, most
/// listened first. A leaf with no parent at all is not a way in and is dropped.
pub fn parent_styles(rows: &[super::wire::ExplorerStyleRow]) -> Vec<StyleTile> {
    let mut by_name: Vec<StyleTile> = Vec::new();
    for row in rows {
        if row.name.trim().is_empty() {
            continue;
        }
        let parent = row
            .parent
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty());
        let family = row
            .family
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty());
        if row.leaf && parent.is_none() && family.is_none() {
            continue;
        }
        let name = if row.leaf {
            parent.or(family).unwrap_or(row.name.as_str())
        } else {
            row.name.trim()
        };
        let tile = match by_name.iter_mut().find(|tile| tile.name == name) {
            Some(tile) => tile,
            None => {
                by_name.push(StyleTile {
                    name: name.to_owned(),
                    ..StyleTile::default()
                });
                by_name.last_mut().expect("just pushed")
            }
        };
        tile.tracks = tile.tracks.saturating_add(row.tracks);
        tile.weight += row.weight;
        if row.leaf {
            tile.leaf = true;
        }
        if row.name.trim() != name && !tile.children.iter().any(|child| child == row.name.trim()) {
            tile.children.push(row.name.trim().to_owned());
        }
    }
    // Most listened first: the server already sorts, but a merge can reorder it.
    by_name.sort_by(|a, b| b.tracks.cmp(&a.tracks).then_with(|| a.name.cmp(&b.name)));
    by_name.truncate(TILES_MAX);
    by_name
}

/// The biggest tile's count, for the bar under each name (`maxTracks`).
pub fn max_tracks(tiles: &[StyleTile]) -> u32 {
    tiles
        .iter()
        .map(|tile| tile.tracks)
        .max()
        .unwrap_or(0)
        .max(1)
}

/// `ExplorerStyles.svelte#countLabel`: "12 titres", "1 titre".
pub fn count_label(tracks: u32) -> String {
    format!("{tracks} titre{}", if tracks == 1 { "" } else { "s" })
}

/// One row of "Toutes les stations" (`genrestation.js#stylesOf`).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StationRow {
    pub name: String,
    pub parent: Option<String>,
    /// Titles under it, this node included — what a station is worth.
    pub branch: u32,
    /// Titles whose own style is this node.
    pub tracks: u32,
}

/// `genrestation.js#stylesOf`: junk dropped, names trimmed, a row kept ONCE (a
/// kept copy from an older build can hold it twice; the first reading wins).
pub fn station_rows(styles: &FlowStyles) -> Vec<StationRow> {
    let mut out: Vec<StationRow> = Vec::new();
    for style in &styles.styles {
        let name = style.name.trim();
        if name.is_empty() || out.iter().any(|row| row.name == name) {
            continue;
        }
        out.push(StationRow {
            name: name.to_owned(),
            parent: style
                .parent
                .as_deref()
                .map(str::trim)
                .filter(|parent| !parent.is_empty())
                .map(str::to_owned),
            branch: style.branch,
            tracks: style.tracks,
        });
    }
    out
}

/// Folds a string the way the library search does (`appcore::fold`).
pub fn fold_text(text: &str) -> String {
    let mut bytes = Vec::with_capacity(text.len());
    fold::fold_into(text.as_bytes(), &mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `genretree.js#flat`: lowercase, accents removed, letters and digits only —
/// so "Disco / funk" and "discofunk" are one style, which is what the SPA's own
/// style search compares.
pub fn flat_genre(text: &str) -> String {
    fold_text(text)
        .chars()
        .filter(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        .collect()
}

/// `genrestation.js#filterStyles`: the styles whose name or parent holds what
/// was typed, accents and punctuation set aside ("electro" finds "Électro",
/// "disco funk" finds "Disco / funk").
pub fn filter_styles(rows: &[StationRow], query: &str) -> Vec<StationRow> {
    let needle = flat_genre(query);
    if needle.is_empty() {
        return rows.to_vec();
    }
    let haystack = needle.as_bytes();
    rows.iter()
        .filter(|row| {
            fold::contains(flat_genre(&row.name).as_bytes(), haystack)
                || fold::contains(
                    flat_genre(row.parent.as_deref().unwrap_or("")).as_bytes(),
                    haystack,
                )
        })
        .cloned()
        .collect()
}

/// The queue context of a station (`genrestation.js#stationContext`): its OWN
/// kind, the server's canonical node, and a title a person reads. `None` when
/// neither the asked style nor the answer names one — then there is nothing to
/// play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StationContext {
    pub kind: String,
    pub id: String,
    pub title: String,
}

pub fn station_context(style: &str, answer: &super::wire::StationAnswer) -> Option<StationContext> {
    let asked = style.trim();
    let canonical = {
        let answered = answer.style.trim();
        if answered.is_empty() { asked } else { answered }
    };
    if canonical.is_empty() {
        return None;
    }
    let name = {
        let answered = answer.name.trim();
        if answered.is_empty() {
            canonical
        } else {
            answered
        }
    };
    Some(StationContext {
        kind: STATION_KIND.to_owned(),
        id: canonical.to_owned(),
        title: format!("{STATION_PREFIX}{name}"),
    })
}

/// `genrestation.js#stationTracks`: the rows a station may play — a catalogue
/// title by its provider id, a library title by its own.
pub fn station_tracks(answer: &super::wire::StationAnswer) -> Vec<Track> {
    answer
        .tracks
        .iter()
        .filter(|track| !track.deezer_id.to_string().is_empty() || track.id.is_some())
        .cloned()
        .collect()
}

/// The five breadth notches, as the slider names them
/// (`ExplorerStyles.svelte#LEVELS`): what the server takes and what the person
/// reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub level: u32,
    pub name: &'static str,
    pub hint: &'static str,
}

pub const LEVEL_NAMES: [Level; 5] = [
    Level {
        level: 1,
        name: "Prudent",
        hint: "Des playlists qui restent dans le style, sans surprise.",
    },
    Level {
        level: 2,
        name: "Mesuré",
        hint: "Le style, avec quelques voisins immédiats.",
    },
    Level {
        level: 3,
        name: "Équilibré",
        hint: "Le style et ce qui l'entoure.",
    },
    Level {
        level: 4,
        name: "Curieux",
        hint: "Le style, ses voisins et des passerelles.",
    },
    Level {
        level: 5,
        name: "Large",
        hint: "Tout ce que Deezer rapproche de ce style.",
    },
];

/// The notch, clamped to the five the server accepts.
pub fn clamp_level(level: u32) -> u32 {
    level.clamp(1, LEVELS)
}

/// The name and sentence of one notch.
pub fn level(level: u32) -> Level {
    LEVEL_NAMES[(clamp_level(level) - 1) as usize]
}

/// `ExplorerStyles.svelte#verifiedLabel`: what a playlist's count is worth.
pub fn verified_label(playlist: &super::wire::DiscoverPlaylist) -> String {
    match playlist.matched {
        None => {
            if playlist.nb_tracks > 0 {
                count_label(playlist.nb_tracks)
            } else {
                "Titres vérifiés".to_owned()
            }
        }
        Some(matched) => {
            if playlist.nb_tracks > matched {
                format!(
                    "{} vérifiés sur {}",
                    count_label(matched),
                    format_thousands(playlist.nb_tracks)
                )
            } else {
                format!("{} vérifiés", count_label(matched))
            }
        }
    }
}

/// `Number(n).toLocaleString("fr-FR")` for the counts this screen prints: the
/// SPA's own separator, a NARROW NO-BREAK SPACE.
pub fn format_thousands(value: u32) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(super::recap::THIN_SPACE);
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::wire::{DiscoverPlaylist, ExplorerStyleRow};

    fn row(name: &str, parent: Option<&str>, tracks: u32, leaf: bool) -> ExplorerStyleRow {
        ExplorerStyleRow {
            name: name.to_owned(),
            parent: parent.map(str::to_owned),
            family: None,
            tracks,
            weight: 0.5,
            confidence: 0.9,
            leaf,
            children: 0,
        }
    }

    /// A leaf is displayed under its parent, two leaves of one parent merge into
    /// ONE tile, and a leaf with nothing above it is not a way in.
    #[test]
    fn parent_styles_merges_leaves_under_their_parent_and_drops_orphans() {
        let rows = vec![
            row("Techno", None, 10, false),
            row("Detroit", Some("Techno"), 2, true),
            row("Minimal", Some("Techno"), 3, true),
            row("Inconnu", None, 5, true),
        ];
        let tiles = parent_styles(&rows);
        assert_eq!(tiles.len(), 1, "the orphan leaf is not a way in");
        assert_eq!(tiles[0].name, "Techno");
        assert_eq!(tiles[0].tracks, 15, "its own plus both leaves");
        assert!(tiles[0].leaf);
        assert_eq!(tiles[0].children, vec!["Detroit", "Minimal"]);
        assert_eq!(max_tracks(&tiles), 15);
    }

    #[test]
    fn the_tiles_are_capped_at_ten_most_listened_first() {
        let rows: Vec<ExplorerStyleRow> = (0..14)
            .map(|n| row(&format!("Style {n}"), None, n, false))
            .collect();
        let tiles = parent_styles(&rows);
        assert_eq!(tiles.len(), TILES_MAX);
        assert_eq!(tiles[0].name, "Style 13", "the most listened leads");
    }

    /// `genrestation.js#stylesOf`: junk dropped, duplicates kept once, names and
    /// parents trimmed.
    #[test]
    fn a_station_row_is_clean_and_unique() {
        let styles: FlowStyles = serde_json::from_str(
            r#"{"styles":[
                {"name":" Techno ","parent":null,"tracks":10,"branch":12},
                {"name":"Techno","parent":null,"tracks":1,"branch":1},
                {"name":"   ","parent":null,"tracks":9,"branch":9},
                {"name":"Detroit","parent":" Techno ","tracks":2,"branch":2}]}"#,
        )
        .expect("styles");
        let rows = station_rows(&styles);
        assert_eq!(rows.len(), 2, "the blank row and the duplicate are dropped");
        assert_eq!(rows[0].name, "Techno");
        assert_eq!(rows[0].tracks, 10, "the first reading wins");
        assert_eq!(rows[1].parent.as_deref(), Some("Techno"));
    }

    /// The style search folds accents and spacing, as `filterStyles` does.
    #[test]
    fn the_style_search_folds_accents_and_spacing() {
        let rows = vec![
            StationRow {
                name: "Électro".to_owned(),
                ..StationRow::default()
            },
            StationRow {
                name: "Disco / funk".to_owned(),
                ..StationRow::default()
            },
            StationRow {
                name: "Detroit".to_owned(),
                parent: Some("Techno".to_owned()),
                ..StationRow::default()
            },
        ];
        assert_eq!(
            filter_styles(&rows, "").len(),
            3,
            "an empty query keeps all"
        );
        assert_eq!(filter_styles(&rows, "electro")[0].name, "Électro");
        assert_eq!(filter_styles(&rows, "disco funk")[0].name, "Disco / funk");
        assert_eq!(
            filter_styles(&rows, "techno")[0].name,
            "Detroit",
            "a parent matches too"
        );
        assert!(filter_styles(&rows, "zzz").is_empty());
    }

    /// A station's context names the SERVER's canonical node, and an empty
    /// answer has no context at all.
    #[test]
    fn a_station_context_uses_the_servers_canonical_node() {
        let answer = crate::discovery::wire::StationAnswer {
            style: "Techno".to_owned(),
            name: "Techno".to_owned(),
            tracks: Vec::new(),
        };
        let context = station_context("techno", &answer).expect("a context");
        assert_eq!(context.kind, STATION_KIND);
        assert_eq!(context.id, "Techno", "the node a refill takes back");
        assert_eq!(context.title, "Station Techno");
        assert_eq!(
            station_context("Techno", &crate::discovery::wire::StationAnswer::default()),
            Some(StationContext {
                kind: STATION_KIND.to_owned(),
                id: "Techno".to_owned(),
                title: "Station Techno".to_owned(),
            }),
            "the asked style is kept when the answer names nothing"
        );
        assert_eq!(
            station_context("  ", &crate::discovery::wire::StationAnswer::default()),
            None
        );
    }

    /// A station plays a catalogue row by its provider id or a library row by
    /// its own: nothing else (`stationTracks`).
    #[test]
    fn a_station_keeps_only_playable_rows() {
        let answer: crate::discovery::wire::StationAnswer = serde_json::from_str(
            r#"{"style":"Techno","name":"Techno","tracks":[
                {"deezer_id":"9001","title":"Catalogue"},
                {"id":"11111111-1111-4111-8111-111111111111","deezer_id":"","title":"Bibliothèque"},
                {"deezer_id":"","title":"Rien"}]}"#,
        )
        .expect("station");
        assert_eq!(station_tracks(&answer).len(), 2);
    }

    /// The five notches are the component's own words, clamped.
    #[test]
    fn the_five_breadths_are_named() {
        assert_eq!(level(1).name, "Prudent");
        assert_eq!(level(3).name, "Équilibré");
        assert_eq!(level(5).name, "Large");
        assert_eq!(clamp_level(0), 1);
        assert_eq!(clamp_level(9), 5);
        assert_eq!(count_label(1), "1 titre");
        assert_eq!(count_label(40), "40 titres");
        assert_eq!(format_thousands(1_570), "1\u{202f}570");
    }

    /// `verifiedLabel`: what the count is worth, in the panel's own words.
    #[test]
    fn a_playlist_says_how_many_of_its_titles_were_verified() {
        let playlist = |matched: Option<u32>, nb_tracks: u32| DiscoverPlaylist {
            matched,
            nb_tracks,
            ..DiscoverPlaylist::default()
        };
        assert_eq!(
            verified_label(&playlist(Some(12), 60)),
            "12 titres vérifiés sur 60"
        );
        assert_eq!(
            verified_label(&playlist(Some(12), 12)),
            "12 titres vérifiés"
        );
        assert_eq!(verified_label(&playlist(None, 40)), "40 titres");
        assert_eq!(verified_label(&playlist(None, 0)), "Titres vérifiés");
    }
}
