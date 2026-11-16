//! One genre, drawn once: the tag model, the page's ranking of a track's
//! vocabulary, and the chip every genre screen composes.
//!
//! `webapp/src/lib/genre/rank.js` is the reference, ported value for value: the
//! server ranks the genres it has an opinion on (`scores`: `[label, score,
//! head, proto]`), and the rest follow by how much this library uses them, then
//! by name. The order is what a person sees, so it is pinned to the JavaScript's
//! own output in the tests below.
//!
//! A tag's colour is the server's `#rrggbb`, parsed here (a bad colour falls back
//! to the theme's accent rather than painting nothing), and a chip always shows
//! its name: colour is a second cue, never the only one (an accessibility rule,
//! and the reason tagging stays readable with `NO_COLOR` themes).

use std::collections::BTreeMap;

use gpui_kit::{
    App, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    Styled, div, rems,
};
use nz_model::GenreNode;

use crate::theme;

/// How many genres a track's ranking carries (`deezer/webui/genre.py#SCORES_MAX`).
pub const SCORES_MAX: usize = 24;

/// One genre of the vocabulary, as `/genre/status` and `/genre/tree` send it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tag {
    /// The tag row's id, `0` for a tree-only node (`id: null` on the wire).
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub archetype: Option<String>,
    /// The engine family the tree resolves this node to, when it has one.
    pub family: Option<String>,
}

impl Tag {
    pub fn from_node(node: &GenreNode) -> Self {
        Self {
            id: node.id,
            name: node.name.clone(),
            color: node.color.clone(),
            archetype: node.archetype.clone(),
            family: node.family.clone(),
        }
    }

    pub fn new(id: i64, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            ..Self::default()
        }
    }
}

/// One row of the ranking: the tag, the track's score when the server has an
/// opinion, and how much this library uses it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scored {
    pub tag: Tag,
    /// `null` in the page when neither the head nor the prototypes said
    /// anything about the track.
    pub score: Option<f64>,
    pub head: Option<f64>,
    pub proto: Option<f64>,
    pub used: u32,
    /// The folded name, so a search never refolds on every keystroke.
    pub key: String,
}

/// `rank.js#fold`: case, accents and punctuation folded away. The JavaScript
/// normalises NFD, drops every mark, lowercases, then keeps `[a-z0-9]` words;
/// a non-Latin name folds to nothing, exactly as there.
pub fn fold(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut gap = false;
    for character in value.chars() {
        // Lowercase FIRST: `É` lowers to `é`, which the accent arms know.
        let character = character.to_lowercase().next().unwrap_or(character);
        let folded = match character {
            'à' | 'â' | 'ä' | 'á' | 'ã' | 'å' | 'ā' => Some('a'),
            'é' | 'è' | 'ê' | 'ë' | 'ē' => Some('e'),
            'î' | 'ï' | 'í' | 'ì' | 'ī' => Some('i'),
            'ô' | 'ö' | 'ó' | 'ò' | 'õ' | 'ō' => Some('o'),
            'û' | 'ü' | 'ú' | 'ù' | 'ū' => Some('u'),
            'ç' | 'ć' | 'č' => Some('c'),
            'ñ' | 'ń' => Some('n'),
            'ß' | 'œ' | 'æ' | 'ø' => None,
            other => other.is_ascii_alphanumeric().then_some(other),
        };
        match folded {
            Some(c) if c.is_ascii_alphanumeric() => {
                if gap && !out.is_empty() {
                    out.push(' ');
                }
                gap = false;
                out.push(c);
            }
            _ => gap = true,
        }
    }
    out
}

/// One row of a server ranking: `[label, score, head, proto]` (`rank.js`).
pub type ScoreRow = (String, f64, Option<f64>, Option<f64>);

/// `rank.js#rankTags`: opinions first, best first; then use; then name.
pub fn rank(tags: &[Tag], scores: &[ScoreRow], counts: &BTreeMap<String, u32>) -> Vec<Scored> {
    let mut by_name: BTreeMap<&str, &ScoreRow> = BTreeMap::new();
    for row in scores {
        by_name.entry(row.0.as_str()).or_insert(row);
    }
    let mut rows: Vec<Scored> = tags
        .iter()
        .map(|tag| {
            let hit = by_name.get(tag.name.as_str()).copied();
            Scored {
                tag: tag.clone(),
                score: hit.map(|row| if row.1.is_finite() { row.1 } else { 0. }),
                head: hit.and_then(|row| row.2),
                proto: hit.and_then(|row| row.3),
                used: counts.get(&tag.name).copied().unwrap_or(0),
                key: fold(&tag.name),
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        if a.score.is_some() != b.score.is_some() {
            return if a.score.is_some() {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }
        if let (Some(x), Some(y)) = (a.score, b.score)
            && x != y
        {
            return y.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal);
        }
        if a.used != b.used {
            return b.used.cmp(&a.used);
        }
        a.key.cmp(&b.key)
    });
    rows
}

/// `rank.js#searchTags`: a whole-name match, then a word starting with the
/// query, then the query anywhere — the rank order is kept inside each tier,
/// nothing is re-alphabetised.
pub fn search<'a>(ranked: &'a [Scored], query: &str) -> Vec<&'a Scored> {
    let q = fold(query);
    if q.is_empty() {
        return ranked.iter().collect();
    }
    let compact = q.replace(' ', "");
    let (mut exact, mut word, mut inside) = (Vec::new(), Vec::new(), Vec::new());
    for row in ranked {
        if row.key == q {
            exact.push(row);
        } else if row.key.starts_with(&q) || row.key.contains(&format!(" {q}")) {
            word.push(row);
        } else if row.key.replace(' ', "").contains(&compact) {
            inside.push(row);
        }
    }
    exact.append(&mut word);
    exact.append(&mut inside);
    exact
}

/// The score as the page prints it: a whole percentage, `0 %` under half a
/// point (a share of a thousandth is noise, not a promise).
pub fn percent(score: f64) -> String {
    let pct = (score * 100.).round();
    if pct <= 0. {
        "0 %".to_owned()
    } else {
        format!("{} %", pct as i64)
    }
}

/// A tag's own colour, or the theme's accent when the server sent none (or
/// something that is not `#rrggbb`).
pub fn color(tag: &Tag, cx: &App) -> Hsla {
    tag.color
        .as_deref()
        .and_then(parse_hex)
        .unwrap_or_else(|| theme::accent(cx))
}

fn parse_hex(value: &str) -> Option<Hsla> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let rgb = u32::from_str_radix(hex, 16).ok()?;
    Some(Hsla::from(gpui_kit::rgb(rgb)))
}

/// The coloured dot every chip starts with.
pub fn dot(tag: &Tag, size: gpui_kit::Pixels, cx: &App) -> impl IntoElement + use<> {
    div()
        .flex_shrink_0()
        .w(size)
        .h(size)
        .rounded_full()
        .bg(color(tag, cx))
}

/// One tag, as a chip: a dot, the name, and an optional trailing count or
/// percentage. `selected` draws the border the theme's selection uses, so a
/// pressed chip reads without relying on colour alone.
pub fn chip<V: gpui_kit::Render>(
    id: impl Into<gpui_kit::ElementId>,
    tag: &Tag,
    trailing: Option<String>,
    selected: bool,
    cx: &mut gpui_kit::Context<V>,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let mut chip = div()
        .id(id)
        .flex()
        .items_center()
        .gap(rems(0.375))
        .px(rems(0.5))
        .py(rems(0.25))
        .border_1()
        .border_color(if selected {
            color(tag, cx)
        } else {
            theme::border(cx)
        })
        .bg(if selected {
            theme::selected(cx)
        } else {
            theme::chrome(cx)
        })
        .text_size(rems(0.75))
        .text_color(theme::text(cx))
        .cursor_pointer()
        .hover(|style| style.bg(theme::selected(cx)))
        .child(dot(tag, gpui_kit::px(7.), cx))
        .child(div().child(SharedString::from(tag.name.clone())));
    if let Some(trailing) = trailing {
        chip = chip.child(
            div()
                .font_family(theme::mono(cx))
                .text_size(rems(0.625))
                .text_color(theme::secondary(cx))
                .child(SharedString::from(trailing)),
        );
    }
    chip
}

/// A candidate card's heading line: the title, then who made it.
pub fn track_line(title: &str, artist: &str, album: &str, cx: &App) -> impl IntoElement + use<> {
    let mut line = div()
        .flex()
        .flex_col()
        .gap(rems(0.125))
        .child(
            div()
                .text_size(rems(1.125))
                .font_weight(FontWeight::BOLD)
                .text_color(theme::text(cx))
                .child(SharedString::from(title.to_owned())),
        )
        .child(
            div()
                .text_size(rems(0.8125))
                .text_color(theme::secondary(cx))
                .child(SharedString::from(artist.to_owned())),
        );
    if !album.is_empty() {
        line = line.child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child(SharedString::from(album.to_owned())),
        );
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The JavaScript's own answers, produced by running `lib/genre/rank.js`
    /// under Node on this machine (`node --input-type=module -e …` with the
    /// inputs below). The order is what a person sees, so it is pinned, not
    /// "reasonable".
    #[test]
    fn fold_matches_the_web_apps_own_output() {
        for (input, expected) in [
            ("Été", "ete"),
            ("Drum & bass", "drum bass"),
            ("Deutscher Krach", "deutscher krach"),
            ("  Hardtek  ", "hardtek"),
            ("Jazz / acoustique", "jazz acoustique"),
            ("Нирвана", ""),
            ("!!!", ""),
            ("Rap FR", "rap fr"),
            ("Café-concert", "cafe concert"),
        ] {
            assert_eq!(fold(input), expected, "fold({input:?})");
        }
    }

    fn tags(names: &[&str]) -> Vec<Tag> {
        names
            .iter()
            .enumerate()
            .map(|(index, name)| Tag::new(index as i64 + 1, *name))
            .collect()
    }

    /// `rankTags` on a track the head has one opinion about: the opinion leads
    /// even though the genre is barely used, then the library's own order.
    #[test]
    fn an_opinion_leads_then_use_then_name() {
        let tags = tags(&["Ambient", "Frenchcore", "Hardtek"]);
        let counts = BTreeMap::from([
            ("Ambient".to_owned(), 30),
            ("Frenchcore".to_owned(), 2),
            ("Hardtek".to_owned(), 7),
        ]);
        let scores = vec![("Hardtek".to_owned(), 0.62, Some(0.7), Some(0.4))];
        let ranked = rank(&tags, &scores, &counts);
        assert_eq!(
            ranked
                .iter()
                .map(|row| row.tag.name.as_str())
                .collect::<Vec<_>>(),
            ["Hardtek", "Ambient", "Frenchcore"]
        );
        assert_eq!(ranked[0].score, Some(0.62));
        assert_eq!(ranked[0].head, Some(0.7));
        assert_eq!(ranked[0].proto, Some(0.4));
        assert_eq!(ranked[0].used, 7);
        assert_eq!(ranked[1].score, None);
    }

    /// Without any opinion the order is use first, then the folded name — the
    /// page's fallback so the wall opens on the genres this library tags.
    #[test]
    fn without_an_opinion_the_order_is_use_then_name() {
        let tags = tags(&["Ambient", "Acid", "Frenchcore", "Dub"]);
        let counts = BTreeMap::from([
            ("Frenchcore".to_owned(), 12),
            ("Acid".to_owned(), 3),
            ("Dub".to_owned(), 3),
        ]);
        let ranked = rank(&tags, &[], &counts);
        assert_eq!(
            ranked
                .iter()
                .map(|row| row.tag.name.as_str())
                .collect::<Vec<_>>(),
            ["Frenchcore", "Acid", "Dub", "Ambient"]
        );
    }

    /// A whole-name match, then a word that starts with it, then it anywhere —
    /// inside each tier the rank survives ("core" reaches Frenchcore, then
    /// Hardcore, in the track's order, never alphabetically).
    #[test]
    fn a_search_keeps_the_rank_inside_each_tier() {
        let tags = tags(&["Frenchcore", "Hardcore", "Core", "Breakcore"]);
        let counts = BTreeMap::from([
            ("Frenchcore".to_owned(), 9),
            ("Hardcore".to_owned(), 4),
            ("Breakcore".to_owned(), 2),
        ]);
        let ranked = rank(&tags, &[], &counts);
        let found: Vec<&str> = search(&ranked, "core")
            .into_iter()
            .map(|row| row.tag.name.as_str())
            .collect();
        assert_eq!(found, ["Core", "Frenchcore", "Hardcore", "Breakcore"]);
        // A multi-word query reaches the name it prefixes, accents folded.
        let found: Vec<&str> = search(&ranked, "french core")
            .into_iter()
            .map(|row| row.tag.name.as_str())
            .collect();
        assert_eq!(found, ["Frenchcore"]);
    }

    #[test]
    fn a_colour_is_the_servers_own_or_the_themes_accents() {
        assert!(parse_hex("#ff8800").is_some());
        assert!(parse_hex("ff8800").is_none());
        assert!(parse_hex("#ff88").is_none());
        assert!(parse_hex("#gg0000").is_none());
        assert_eq!(percent(0.624), "62 %");
        assert_eq!(percent(0.0004), "0 %");
    }
}
