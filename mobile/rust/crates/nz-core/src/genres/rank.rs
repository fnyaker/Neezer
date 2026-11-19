//! The vocabulary in the order a person tagging ONE track wants it — the port
//! of `webapp/src/lib/genre/rank.js`.
//!
//! The studio seeds ~220 genres. Listed alphabetically the one you want is
//! somewhere in a wall of names, so the order is the TRACK's own: the genres the
//! server has an opinion on (`scores`, the trained head's calibrated probability
//! blended with the nearest prototype's share) first, best first; then what this
//! library tags most; then by name. With no model at all the list still opens on
//! the genres this person actually uses.
//!
//! Ported rather than re-invented because the phone must open on the SAME
//! candidate as the page: the first row is what `Entrée` takes, and a different
//! tie-break would make the same library answer differently depending on the
//! device. `The ranking is the page's` holds it, and the fold is `appcore`'s
//! (`fold`, the web app's own table: "beyonce" finds "Beyoncé").

use appcore::fold::fold_into;

/// `appcore`'s fold as an owned `String` (the table is `fold_into`'s, so a
/// genre named "Électro" is reached by "electro" — the same bytes the lists
/// search on).
fn fold(text: &str) -> String {
    let mut out = Vec::with_capacity(text.len());
    fold_into(text.as_bytes(), &mut out);
    String::from_utf8(out).unwrap_or_default()
}

/// One genre, with whatever either opinion had to say about it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RankedTag {
    /// The server's tag id (the input of every write).
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub archetype: Option<String>,
    /// How many library tracks carry it (`/genre/status`'s `counts`).
    pub used: u32,
    /// The blended score, `None` when neither opinion exists.
    pub score: Option<f64>,
    /// The trained head's calibrated probability, when it has one.
    pub head: Option<f64>,
    /// The nearest prototype's share, when it has one.
    pub proto: Option<f64>,
    /// The folded name, for [search].
    key: String,
}

/// A tag of the vocabulary, as `rank` needs it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub archetype: Option<String>,
    /// How many library tracks carry it.
    pub used: u32,
}

/// One row of a `/genre/track/<id>` ranking: `[label, score, head, proto]`.
pub type Score = (String, f64, Option<f64>, Option<f64>);

/// `rank.js#rankTags`: best score first, then the library's own use, then the
/// folded name — a total order, so two devices never disagree.
pub fn rank(tags: &[Tag], scores: &[Score]) -> Vec<RankedTag> {
    let mut rows: Vec<RankedTag> = tags
        .iter()
        .map(|tag| {
            let found = scores.iter().find(|(label, ..)| *label == tag.name);
            RankedTag {
                id: tag.id,
                name: tag.name.clone(),
                color: tag.color.clone(),
                archetype: tag.archetype.clone(),
                used: tag.used,
                score: found.map(|(_, score, _, _)| *score),
                head: found.and_then(|(_, _, head, _)| *head),
                proto: found.and_then(|(_, _, _, proto)| *proto),
                key: fold(&tag.name),
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        // A genre with an opinion outranks one without, whatever the score.
        let by_score = match (a.score, b.score) {
            (Some(left), Some(right)) => Some(
                right
                    .partial_cmp(&left)
                    .unwrap_or(std::cmp::Ordering::Equal),
            ),
            (Some(_), None) => Some(std::cmp::Ordering::Less),
            (None, Some(_)) => Some(std::cmp::Ordering::Greater),
            (None, None) => None,
        };
        by_score.unwrap_or_else(|| {
            b.used
                .cmp(&a.used)
                .then_with(|| a.key.cmp(&b.key))
                .then_with(|| a.id.cmp(&b.id))
        })
    });
    rows
}

/// `rank.js#searchTags`: whole-name matches, then a word that starts with the
/// query, then the query anywhere — each tier in the rank order, nothing
/// re-sorted alphabetically ("core" reaches Frenchcore and Hardcore in the order
/// this track would take them).
pub fn search<'a>(ranked: &'a [RankedTag], query: &str) -> Vec<&'a RankedTag> {
    let query = fold(query);
    if query.is_empty() {
        return ranked.iter().collect();
    }
    let squeezed = query.replace(' ', "");
    let (mut exact, mut word, mut inside) = (Vec::new(), Vec::new(), Vec::new());
    for row in ranked {
        if row.key == query {
            exact.push(row);
        } else if row.key.starts_with(&query) || row.key.contains(&format!(" {query}")) {
            word.push(row);
        } else if row.key.replace(' ', "").contains(&squeezed) {
            inside.push(row);
        }
    }
    exact.extend(word);
    exact.extend(inside);
    exact
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(id: i64, name: &str, used: u32) -> Tag {
        Tag {
            id,
            name: name.to_owned(),
            used,
            ..Tag::default()
        }
    }

    /// The page's order: opinion first, then use, then the name. The phone opens
    /// on the same first row the page does (`Entrée` takes it).
    #[test]
    fn the_ranking_is_the_pages() {
        let tags = vec![
            tag(1, "Acid", 0),
            tag(2, "Hardcore", 40),
            tag(3, "Frenchcore", 12),
            tag(4, "Zebda", 3),
        ];
        let scores = vec![
            ("Frenchcore".to_owned(), 0.6, Some(0.5), None),
            ("Zebda".to_owned(), 0.2, Some(0.1), Some(0.4)),
        ];
        let ranked = rank(&tags, &scores);
        let names: Vec<&str> = ranked.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["Frenchcore", "Zebda", "Hardcore", "Acid"]);
        assert_eq!(ranked[0].score, Some(0.6));
        assert_eq!(ranked[0].head, Some(0.5));
        assert_eq!(ranked[2].score, None);
    }

    /// The fold is `appcore`'s, so accents and case never decide a tie.
    #[test]
    fn the_tie_break_folds_accents_and_case() {
        let tags = vec![tag(1, "Électro", 0), tag(2, "Easy", 0)];
        let ranked = rank(&tags, &[]);
        assert_eq!(ranked[0].name, "Easy");
        assert_eq!(search(&ranked, "ELECTRO")[0].name, "Électro");
    }

    /// Three tiers, rank order kept inside each: the page's own promise.
    #[test]
    fn a_search_keeps_the_rank_inside_each_tier() {
        let tags = vec![
            tag(1, "Hardcore", 5),
            tag(2, "Frenchcore", 9),
            tag(3, "Core", 1),
            tag(4, "Corelli", 0),
        ];
        let ranked = rank(&tags, &[]);
        let found: Vec<&str> = search(&ranked, "core")
            .iter()
            .map(|row| row.name.as_str())
            .collect();
        assert_eq!(found, ["Core", "Corelli", "Frenchcore", "Hardcore"]);
    }
}
