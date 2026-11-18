//! The Récap's decisions, without a screen: card order, the French copy, the
//! numbers, the year chips and the poster lines.
//!
//! This is `webapp/src/lib/recap.js` PORTED, function for function, and
//! `webapp/test/recap.test.mjs` ported with it: the tests below assert the same
//! strings, character for character (a literal U+202F where the French thousands
//! separator belongs), so "the same year is not told two ways" is a test rather
//! than a claim. Nothing here touches a store, the network or a screen: the
//! slides are assembled by [`super::RecapData`] and drawn by Kotlin.

use nz_model::{Recap, RecapAlbumRow, RecapArtistRow, RecapTrackRow};

/// `recap.js#TOP`: a top card names five rows.
pub const TOP: usize = 5;
/// `recap.js#MOSAIC`: at most four sleeves on the opening card and the poster.
pub const MOSAIC: usize = 4;
/// `recap.js#POSTER_LINE_MAX`: one canvas line, ellipsis included.
pub const POSTER_LINE_MAX: usize = 38;

/// `recap.js#KNOWN_CARDS`: the cards a payload may name, in the fixed order of
/// the plan. A card absent from `cards` is NOT rendered — the client never
/// invents one.
pub const KNOWN_CARDS: [&str; 8] = [
    "year",
    "minutes",
    "artists",
    "tracks",
    "albums",
    "genres",
    "streak",
    "firstlast",
];

/// The twelve full month names (`recap.js#MONTHS`).
pub const MONTHS: [&str; 12] = [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre",
];

/// The twelve abbreviations a 360 px phone keeps legible
/// (`recap.js#MONTHS_SHORT`, the French typographic forms).
pub const MONTHS_SHORT: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
    "déc.",
];

/// The French thousands separator: a NARROW NO-BREAK SPACE (U+202F), written
/// once here so the poster's canvas and the tests agree with the SPA's.
pub const THIN_SPACE: char = '\u{202f}';

/// `recap.js#formatNumber`: digits grouped by three, with U+202F.
pub fn format_number(value: f64) -> String {
    let value = if value.is_finite() {
        value.trunc()
    } else {
        0.0
    };
    let sign = if value < 0.0 { "-" } else { "" };
    let digits = format!("{}", value.abs() as u64);
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(THIN_SPACE);
        }
        out.push(ch);
    }
    format!("{sign}{out}")
}

/// "8 540 minutes" / "1 minute" / "0 minute": French counts zero as singular
/// (`recap.js#formatMinutes`).
pub fn format_minutes(value: f64) -> String {
    let value = if value.is_finite() {
        value.trunc()
    } else {
        0.0
    };
    format!(
        "{} minute{}",
        format_number(value),
        if value > 1.0 { "s" } else { "" }
    )
}

/// A span of listened time at the precision that reads (`recap.js#formatSpan`):
/// minutes under an hour, hours and minutes under a day, days and hours above.
pub fn format_span(seconds: f64) -> String {
    let total = if seconds.is_finite() {
        seconds.round().max(0.0) as u64
    } else {
        0
    };
    if total < 60 {
        return "0 min".to_owned();
    }
    if total < 3600 {
        return format!("{} min", total / 60);
    }
    if total < 86_400 {
        let hours = total / 3600;
        let minutes = (total % 3600) / 60;
        return if minutes > 0 {
            format!("{hours} h {minutes:02}")
        } else {
            format!("{hours} h")
        };
    }
    let days = total / 86_400;
    let hours = (total % 86_400) / 3600;
    if hours > 0 {
        format!("{days} j {hours} h")
    } else {
        format!("{days} j")
    }
}

/// The server's order, filtered to the ids this screen knows, then the client's
/// own `share` card — appended only to a real story, never alone on an empty
/// year (`recap.js#cardIds`).
pub fn card_ids(recap: &Recap) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in &recap.cards {
        if !KNOWN_CARDS.contains(&id.as_str()) || out.iter().any(|kept| kept == id) {
            continue;
        }
        out.push(id.clone());
    }
    if recap.enough && !out.is_empty() {
        out.push("share".to_owned());
    }
    out
}

/// The big line of a card — the plan's exact copy (`recap.js#slideTitle`).
pub fn slide_title(recap: &Recap, id: &str) -> String {
    let year = recap.year;
    match id {
        "year" => format!("Ton année {year}"),
        "minutes" => format_minutes(f64::from(recap.totals.minutes)),
        "artists" => "Tes 5 artistes".to_owned(),
        "tracks" => "Tes 5 titres".to_owned(),
        "albums" => "Tes 5 albums".to_owned(),
        "genres" => "Tes styles".to_owned(),
        "streak" => {
            let days = recap.streak.as_ref().map(|s| s.days).unwrap_or(0);
            format!(
                "{} jour{} d'affilée",
                format_number(f64::from(days)),
                if days > 1 { "s" } else { "" }
            )
        }
        "firstlast" => "Ton premier titre de l'année".to_owned(),
        "share" => "Partage ton année".to_owned(),
        _ => String::new(),
    }
}

/// The small mono line above the title (`recap.js#slideKicker`): the opening
/// card names its year (and says when it is still running, from the payload's
/// own `partial`); every other card carries the reader's place.
pub fn slide_kicker(recap: &Recap, id: &str) -> String {
    let ids = card_ids(recap);
    let Some(at) = ids.iter().position(|kept| kept == id) else {
        return String::new();
    };
    if id == "year" {
        return if recap.partial {
            format!("{} · en cours", recap.year)
        } else {
            recap.year.to_string()
        };
    }
    format!("{} sur {}", at + 1, ids.len())
}

/// `YYYY-MM-DD`, when the value is one.
fn iso_day(value: &str) -> Option<(u32, u32, u32)> {
    let bytes = value.as_bytes();
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year = value.get(0..4)?.parse::<u32>().ok()?;
    let month = value.get(5..7)?.parse::<u32>().ok()?;
    let day = value.get(8..10)?.parse::<u32>().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

/// "1ᵉʳ janvier", "13 novembre": the article-less day a French date uses.
fn day_month(day: u32, month: u32) -> String {
    let name = MONTHS[(month as usize - 1).min(11)];
    if day == 1 {
        format!("1ᵉʳ {name}")
    } else {
        format!("{day} {name}")
    }
}

/// The year's display period (`recap.js#periodLabel`): a finished year is the
/// whole year, the running one what has happened so far.
pub fn period_label(recap: &Recap) -> String {
    let label = recap.year.to_string();
    let from = iso_day(&recap.period.from_date);
    let to = iso_day(&recap.period.to);
    match (recap.partial, from, to) {
        (true, Some((_, from_month, from_day)), Some((to_year, to_month, to_day))) => {
            format!(
                "du {} au {} {to_year}",
                day_month(from_day, from_month),
                day_month(to_day, to_month)
            )
        }
        _ => format!("toute l'année {label}").trim().to_owned(),
    }
}

/// One of the twelve bars (`recap.js#monthBars`): always twelve, zero-filled,
/// normalised to the busiest month so the tallest is exactly 100.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonthBar {
    pub month: u8,
    pub label: String,
    pub listens: u32,
    pub pct: u8,
}

pub fn month_bars(recap: &Recap) -> Vec<MonthBar> {
    let mut listens = [0u32; 12];
    for row in &recap.months {
        if (1..=12).contains(&row.month) {
            listens[row.month as usize - 1] = row.listens;
        }
    }
    let max = listens.iter().copied().max().unwrap_or(0);
    (0..12)
        .map(|index| {
            let value = listens[index];
            MonthBar {
                month: (index + 1) as u8,
                label: MONTHS_SHORT[index].to_owned(),
                listens: value,
                pct: if max > 0 {
                    ((u64::from(value) * 100 + u64::from(max) / 2) / u64::from(max)) as u8
                } else {
                    0
                },
            }
        })
        .collect()
}

/// The busiest month, named in full; ties go to the earliest month
/// (`recap.js#busiestMonth`).
pub fn busiest_month(recap: &Recap) -> Option<(String, u32)> {
    let mut best: Option<(usize, u32)> = None;
    for (index, bar) in month_bars(recap).into_iter().enumerate() {
        if bar.listens > 0 && best.is_none_or(|(_, listens)| bar.listens > listens) {
            best = Some((index, bar.listens));
        }
    }
    best.map(|(index, listens)| (MONTHS[index].to_owned(), listens))
}

/// The rows of a top, in the server's order, at most five; nothing invented
/// (`recap.js#rankRows`).
pub fn rank_tracks(recap: &Recap) -> Vec<RecapTrackRow> {
    recap.tops.tracks.iter().take(TOP).cloned().collect()
}

pub fn rank_artists(recap: &Recap) -> Vec<RecapArtistRow> {
    recap.tops.artists.iter().take(TOP).cloned().collect()
}

pub fn rank_albums(recap: &Recap) -> Vec<RecapAlbumRow> {
    recap.tops.albums.iter().take(TOP).cloned().collect()
}

/// The styles card's lines: nothing when the server could not vouch for the
/// measurement (`recap.js#genreRows`).
pub fn genre_rows(recap: &Recap) -> Vec<nz_model::RecapGenreRow> {
    if !recap.tops.genres.available {
        return Vec::new();
    }
    recap
        .tops
        .genres
        .rows
        .iter()
        .filter(|row| !row.name.is_empty())
        .take(TOP)
        .cloned()
        .collect()
}

/// Up to four unique sleeves, albums first, then the top titles' album art
/// (`recap.js#mosaicCovers`).
pub fn mosaic_covers(recap: &Recap) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |url: Option<&String>| {
        if out.len() >= MOSAIC {
            return;
        }
        if let Some(url) = url
            && !url.is_empty()
            && !out.iter().any(|kept| kept == url)
        {
            out.push(url.clone());
        }
    };
    for row in &recap.tops.albums {
        push(row.cover.as_ref());
    }
    for row in &recap.tops.tracks {
        push(row.track.album.cover.as_ref());
    }
    out
}

/// One chip of the year selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YearChip {
    pub year: i32,
    pub label: String,
}

/// `recap.js#yearChips`: one chip per year that holds an event, newest first,
/// and only the SERVER's `nowYear` may say "en cours".
pub fn year_chips(years: &[nz_model::RecapYearRow], now_year: Option<i32>) -> Vec<YearChip> {
    let mut out: Vec<YearChip> = Vec::new();
    let mut seen: Vec<i32> = Vec::new();
    for row in years {
        if seen.contains(&row.year) {
            continue;
        }
        if row.events == 0 {
            continue;
        }
        seen.push(row.year);
        out.push(YearChip {
            year: row.year,
            label: match now_year {
                Some(now) if now == row.year => format!("{} · en cours", row.year),
                _ => row.year.to_string(),
            },
        });
    }
    out.sort_by_key(|chip| -chip.year);
    out
}

/// `recap.js#advertisedYear`: the newest year the server judged substantial,
/// and only while it is this year or the one before.
pub fn advertised_year(
    years: &[nz_model::RecapYearRow],
    now_year: Option<i32>,
) -> Option<nz_model::RecapYearRow> {
    let mut best: Option<&nz_model::RecapYearRow> = None;
    for row in years {
        if !row.enough || row.events == 0 {
            continue;
        }
        if best.is_none_or(|kept| row.year > kept.year) {
            best = Some(row);
        }
    }
    let best = best?;
    let now = now_year?;
    (best.year == now || best.year == now - 1).then(|| best.clone())
}

/// Why a year has no story, in the person's own data's order
/// (`recap.js#emptyReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyReason {
    /// No event at all.
    None,
    /// Events, but not one listen.
    SkipsOnly,
    /// Listens the server judged too few.
    Thin,
}

impl EmptyReason {
    pub fn code(self) -> &'static str {
        match self {
            EmptyReason::None => "none",
            EmptyReason::SkipsOnly => "skips-only",
            EmptyReason::Thin => "thin",
        }
    }
}

pub fn empty_reason(recap: &Recap) -> Option<EmptyReason> {
    if recap.totals.events == 0 {
        return Some(EmptyReason::None);
    }
    if recap.totals.listens == 0 {
        return Some(EmptyReason::SkipsOnly);
    }
    if !recap.enough {
        return Some(EmptyReason::Thin);
    }
    None
}

/// One sentence for the share text fallback — tested to the character
/// (`recap.js#shareText`).
pub fn share_text(recap: &Recap) -> String {
    let listens = recap.totals.listens;
    let artist = recap
        .tops
        .artists
        .first()
        .map(|row| row.name.clone())
        .unwrap_or_default();
    let who = if artist.is_empty() {
        String::new()
    } else {
        format!(", avec {artist} en tête")
    };
    let label = if recap.partial {
        format!("Mon année {} (en cours)", recap.year)
    } else {
        format!("Mon année {}", recap.year)
    };
    format!(
        "{label} en musique : {} écoute{}, {}{who}.",
        format_number(f64::from(listens)),
        if listens > 1 { "s" } else { "" },
        format_minutes(f64::from(recap.totals.minutes))
    )
}

/// Cut a string to `max` characters, ellipsis included; ASCII layout whitespace
/// is folded, and the narrow no-break space is left alone so "8 540" never
/// breaks (`recap.js#ellipsis`).
pub fn ellipsis(text: &str, max: usize) -> String {
    let folded: String = {
        let mut out = String::with_capacity(text.len());
        let mut in_space = false;
        for ch in text.chars() {
            if matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{c}' | '\u{b}') {
                if !in_space {
                    out.push(' ');
                }
                in_space = true;
            } else {
                out.push(ch);
                in_space = false;
            }
        }
        out.trim().to_owned()
    };
    let limit = max.max(1);
    if folded.chars().count() <= limit {
        return folded;
    }
    if limit == 1 {
        return "…".to_owned();
    }
    let mut cut: String = folded.chars().take(limit - 1).collect();
    while cut.ends_with(' ') {
        cut.pop();
    }
    cut.push('…');
    cut
}

/// The six lines of the 1080×1350 poster, in reading order
/// (`recap.js#posterLines`): a thin year produces fewer lines rather than a
/// padded one.
pub fn poster_lines(recap: &Recap) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    if recap.year != 0 {
        lines.push(if recap.partial {
            format!("Ton année {}, en cours", recap.year)
        } else {
            format!("Ton année {}", recap.year)
        });
    }
    lines.push(format_minutes(f64::from(recap.totals.minutes)));
    let listens = recap.totals.listens;
    lines.push(format!(
        "{} écoute{}",
        format_number(f64::from(listens)),
        if listens > 1 { "s" } else { "" }
    ));
    if let Some(artist) = recap.tops.artists.first()
        && !artist.name.is_empty()
    {
        lines.push(format!("Artiste : {}", artist.name));
    }
    if let Some(row) = recap.tops.tracks.first()
        && !row.track.title.is_empty()
    {
        lines.push(format!("Titre : {}", row.track.title));
    }
    match busiest_month(recap) {
        Some((label, _)) => lines.push(format!("Ton mois le plus fort : {label}")),
        None => lines.push(period_label(recap)),
    }
    lines
        .into_iter()
        .take(6)
        .map(|line| ellipsis(&line, POSTER_LINE_MAX))
        .collect()
}

#[cfg(test)]
mod tests {
    //! `webapp/test/recap.test.mjs`, ported: the same payload, the same
    //! expected strings, character for character.

    use super::*;

    const TITLE_LONG: &str = "Le très long titre qui ne doit jamais déborder de l'affiche partagée";

    fn payload() -> Recap {
        let json = format!(
            r#"{{
            "year": 2026, "partial": true, "enough": true, "timezone": "server-local",
            "period": {{"from": "2026-01-01", "to": "2026-11-13"}},
            "totals": {{"events": 2412, "listens": 2180, "skips": 190, "neutrals": 42,
                        "seconds": 512400, "minutes": 8540, "days_active": 211}},
            "months": [
              {{"month": 1, "listens": 180}}, {{"month": 2, "listens": 165}},
              {{"month": 3, "listens": 240}}, {{"month": 4, "listens": 210}},
              {{"month": 5, "listens": 190}}, {{"month": 6, "listens": 170}},
              {{"month": 7, "listens": 300}}, {{"month": 8, "listens": 120}},
              {{"month": 9, "listens": 200}}, {{"month": 10, "listens": 225}},
              {{"month": 11, "listens": 180}}, {{"month": 12, "listens": 0}}],
            "streak": {{"days": 23, "from": "2026-03-01", "to": "2026-03-23"}},
            "biggest_day": {{"date": "2026-07-04", "listens": 61, "seconds": 14200}},
            "tops": {{
              "tracks": [
                {{"track": {{"id": "t1", "deezer_id": "1", "title": {TITLE_LONG:?},
                            "album": {{"deezer_id": "7", "cover": "cover-a"}}}}, "listens": 41}},
                {{"track": {{"id": "t2", "deezer_id": "2", "title": "Titre deux",
                            "album": {{"deezer_id": "7", "cover": "cover-b"}}}}, "listens": 34}},
                {{"track": {{"id": "t3", "deezer_id": "3", "title": "Titre trois",
                            "album": {{"deezer_id": "7", "cover": "cover-a"}}}}, "listens": 28}},
                {{"track": {{"id": "t4", "deezer_id": "4", "title": "Titre quatre",
                            "album": {{"deezer_id": "7", "cover": null}}}}, "listens": 22}},
                {{"track": {{"id": "t5", "deezer_id": "5", "title": "Titre cinq",
                            "album": {{"deezer_id": "7", "cover": "cover-c"}}}}, "listens": 19}},
                {{"track": {{"id": "t6", "deezer_id": "6", "title": "Titre six",
                            "album": {{"deezer_id": "7", "cover": "cover-d"}}}}, "listens": 12}}],
              "artists": [
                {{"id": "a1", "name": "June", "cover": "cover-a", "listens": 210, "tracks": 14}},
                {{"id": "a2", "name": "Avril", "cover": null, "listens": 190, "tracks": 9}}],
              "albums": [
                {{"id": "al1", "title": "Premier", "artist": "June", "cover": "cover-a", "listens": 96}},
                {{"id": "al2", "title": "Deuxième", "artist": "Avril", "cover": "cover-e", "listens": 80}}],
              "genres": {{
                "available": true, "coverage": 0.72, "measured_listens": 1570, "explored": 17,
                "rows": [{{"name": "Techno", "listens": 640, "share": 0.41}},
                         {{"name": "House", "listens": 380, "share": 0.24}}]}}
            }},
            "first": {{"at": "2026-01-02T08:11:00", "track": {{"deezer_id": "1", "title": "Titre un"}}}},
            "last": {{"at": "2026-11-13T20:58:00", "track": {{"deezer_id": "3", "title": "Titre trois"}}}},
            "cards": ["year", "minutes", "artists", "tracks", "albums", "genres", "streak", "firstlast"],
            "generated": 1794835151
        }}"#
        );
        serde_json::from_str(&json).expect("the SPA's own payload decodes")
    }

    #[test]
    fn format_number_groups_with_a_narrow_no_break_space() {
        assert_eq!(format_number(0.0), "0");
        assert_eq!(format_number(1.0), "1");
        assert_eq!(format_number(1000.0), "1\u{202f}000");
        assert_eq!(format_number(8540.0), "8\u{202f}540");
        assert_eq!(format_number(1_234_567.0), "1\u{202f}234\u{202f}567");
        assert_eq!(format_number(f64::NAN), "0");
    }

    #[test]
    fn format_minutes_words_the_count_and_counts_zero_as_singular() {
        assert_eq!(format_minutes(0.0), "0 minute");
        assert_eq!(format_minutes(1.0), "1 minute");
        assert_eq!(format_minutes(1000.0), "1\u{202f}000 minutes");
        assert_eq!(format_minutes(8540.0), "8\u{202f}540 minutes");
    }

    #[test]
    fn format_span_picks_minutes_hours_then_days() {
        assert_eq!(format_span(0.0), "0 min");
        assert_eq!(format_span(42.0 * 60.0), "42 min");
        assert_eq!(format_span(3.0 * 3600.0), "3 h");
        assert_eq!(format_span(3.0 * 3600.0 + 12.0 * 60.0), "3 h 12");
        assert_eq!(format_span(5.0 * 86_400.0), "5 j");
        assert_eq!(format_span(5.0 * 86_400.0 + 22.0 * 3600.0), "5 j 22 h");
        assert_eq!(format_span(f64::NAN), "0 min");
    }

    #[test]
    fn card_ids_keeps_the_servers_order_drops_unknown_ids_and_appends_share_last() {
        assert_eq!(
            card_ids(&payload()),
            vec![
                "year",
                "minutes",
                "artists",
                "tracks",
                "albums",
                "genres",
                "streak",
                "firstlast",
                "share"
            ]
        );
        let mut mixed = payload();
        mixed.cards = ["minutes", "year", "nope", "minutes"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(card_ids(&mixed), vec!["minutes", "year", "share"]);

        let mut none = payload();
        none.cards.clear();
        assert!(card_ids(&none).is_empty(), "a deck with no card is no deck");

        let mut thin = payload();
        thin.enough = false;
        assert_eq!(card_ids(&thin).len(), 8, "no share card on a thin year");
    }

    #[test]
    fn slide_title_carries_the_exact_french_copy_of_the_plan() {
        let p = payload();
        assert_eq!(slide_title(&p, "year"), "Ton année 2026");
        assert_eq!(slide_title(&p, "minutes"), "8\u{202f}540 minutes");
        assert_eq!(slide_title(&p, "artists"), "Tes 5 artistes");
        assert_eq!(slide_title(&p, "tracks"), "Tes 5 titres");
        assert_eq!(slide_title(&p, "albums"), "Tes 5 albums");
        assert_eq!(slide_title(&p, "genres"), "Tes styles");
        assert_eq!(slide_title(&p, "streak"), "23 jours d'affilée");
        assert_eq!(slide_title(&p, "firstlast"), "Ton premier titre de l'année");
        assert_eq!(slide_title(&p, "share"), "Partage ton année");
        assert_eq!(slide_title(&p, "unknown"), "");
    }

    #[test]
    fn slide_kicker_names_the_running_year_then_the_readers_place() {
        assert_eq!(slide_kicker(&payload(), "year"), "2026 · en cours");
        let mut finished = payload();
        finished.year = 2025;
        finished.partial = false;
        assert_eq!(slide_kicker(&finished, "year"), "2025");
        assert_eq!(slide_kicker(&payload(), "artists"), "3 sur 9");
        assert_eq!(slide_kicker(&payload(), "share"), "9 sur 9");
        assert_eq!(slide_kicker(&payload(), "unknown"), "");
    }

    #[test]
    fn month_bars_is_always_twelve_zero_filled_bars_normalised_to_the_busiest() {
        let bars = month_bars(&payload());
        assert_eq!(bars.len(), 12);
        assert_eq!(
            bars.iter().map(|bar| bar.month).collect::<Vec<_>>(),
            (1..=12).collect::<Vec<u8>>()
        );
        assert_eq!(bars.iter().map(|bar| bar.pct).max(), Some(100));
        assert_eq!(bars[6].listens, 300);
        assert_eq!(bars[6].pct, 100);
        assert_eq!(bars[11].listens, 0);
        assert_eq!(bars[11].pct, 0);

        let mut partial = payload();
        partial.months.truncate(1);
        let bars = month_bars(&partial);
        assert_eq!(bars.len(), 12);
        assert_eq!(bars[0].pct, 100);
        assert_eq!(bars[1].listens, 0);

        let mut empty = payload();
        empty.months.clear();
        assert!(month_bars(&empty).iter().all(|bar| bar.pct == 0));
    }

    #[test]
    fn busiest_month_names_the_strongest_month_in_full_or_nothing() {
        assert_eq!(busiest_month(&payload()), Some(("juillet".to_owned(), 300)));
        let mut empty = payload();
        empty.months.clear();
        assert_eq!(busiest_month(&empty), None);
    }

    #[test]
    fn rank_rows_never_invents_a_row() {
        let mut empty = payload();
        empty.tops.tracks.clear();
        empty.tops.artists.clear();
        empty.tops.albums.clear();
        assert!(rank_tracks(&empty).is_empty());
        assert!(rank_artists(&empty).is_empty());
        assert!(rank_albums(&empty).is_empty());
        assert_eq!(rank_tracks(&payload()).len(), 5);
        assert_eq!(rank_albums(&payload()).len(), 2);
    }

    #[test]
    fn genre_rows_is_empty_unless_the_server_vouched_for_the_measurement() {
        let mut p = payload();
        p.tops.genres.available = false;
        assert!(genre_rows(&p).is_empty());
        assert_eq!(genre_rows(&payload()).len(), 2);
        assert_eq!(genre_rows(&payload())[0].name, "Techno");
    }

    #[test]
    fn mosaic_covers_is_at_most_four_unique_sleeves_albums_then_titles() {
        let covers = mosaic_covers(&payload());
        assert!(covers.len() <= 4);
        let mut unique = covers.clone();
        unique.dedup();
        assert_eq!(unique, covers, "no sleeve twice");
        assert_eq!(covers, vec!["cover-a", "cover-e", "cover-b", "cover-c"]);
        let mut empty = payload();
        empty.tops.albums.clear();
        empty.tops.tracks.clear();
        assert!(mosaic_covers(&empty).is_empty());
    }

    fn year_row(year: i32, events: u32, enough: bool) -> nz_model::RecapYearRow {
        nz_model::RecapYearRow {
            year,
            events,
            listens: events,
            minutes: 0,
            enough,
            partial: false,
        }
    }

    #[test]
    fn year_chips_uses_the_servers_now_year_never_the_device_clock() {
        let rows = vec![
            year_row(2026, 2412, true),
            year_row(2025, 9120, true),
            year_row(2024, 0, false),
        ];
        assert_eq!(
            year_chips(&rows, Some(2026)),
            vec![
                YearChip {
                    year: 2026,
                    label: "2026 · en cours".to_owned()
                },
                YearChip {
                    year: 2025,
                    label: "2025".to_owned()
                }
            ],
            "the empty 2024 is not offered and the server says what is running"
        );
        assert_eq!(
            year_chips(&[year_row(2026, 1, true)], None),
            vec![YearChip {
                year: 2026,
                label: "2026".to_owned()
            }],
            "without the server's year nothing is called en cours"
        );
        assert!(year_chips(&[], Some(2026)).is_empty());
    }

    #[test]
    fn advertised_year_only_advertises_this_year_or_the_one_before() {
        let rows = vec![year_row(2026, 2412, true), year_row(2025, 9120, true)];
        assert_eq!(
            advertised_year(&rows, Some(2026)).map(|row| row.year),
            Some(2026)
        );
        let thin = vec![year_row(2026, 5, false), year_row(2025, 9120, true)];
        assert_eq!(
            advertised_year(&thin, Some(2026)).map(|row| row.year),
            Some(2025),
            "the current year is thin: last year's full story is the honest banner"
        );
        assert!(
            advertised_year(&rows, Some(2030)).is_none(),
            "no eternal banner"
        );
        assert!(advertised_year(&[], Some(2026)).is_none());
        assert!(advertised_year(&rows, None).is_none());
        assert!(advertised_year(&[year_row(2026, 5, false)], Some(2026)).is_none());
    }

    #[test]
    fn empty_reason_follows_the_persons_own_data() {
        let mut silent = payload();
        silent.totals.events = 0;
        silent.totals.listens = 0;
        assert_eq!(empty_reason(&silent), Some(EmptyReason::None));

        let mut skipped = payload();
        skipped.totals.events = 3;
        skipped.totals.listens = 0;
        skipped.enough = false;
        assert_eq!(empty_reason(&skipped), Some(EmptyReason::SkipsOnly));

        let mut thin = payload();
        thin.totals.events = 5;
        thin.totals.listens = 5;
        thin.enough = false;
        assert_eq!(empty_reason(&thin), Some(EmptyReason::Thin));

        assert_eq!(empty_reason(&payload()), None);
        assert_eq!(EmptyReason::SkipsOnly.code(), "skips-only");
    }

    #[test]
    fn period_label_distinguishes_a_finished_year_from_the_running_one() {
        let mut finished = payload();
        finished.year = 2025;
        finished.partial = false;
        finished.period.from_date = "2025-01-01".to_owned();
        finished.period.to = "2025-12-31".to_owned();
        assert_eq!(period_label(&finished), "toute l'année 2025");
        assert_eq!(
            period_label(&payload()),
            "du 1ᵉʳ janvier au 13 novembre 2026"
        );

        let mut may = payload();
        may.year = 2025;
        may.period.from_date = "2025-05-03".to_owned();
        may.period.to = "2025-12-23".to_owned();
        assert_eq!(period_label(&may), "du 3 mai au 23 décembre 2025");
    }

    #[test]
    fn share_text_is_one_exact_sentence() {
        assert_eq!(
            share_text(&payload()),
            "Mon année 2026 (en cours) en musique : 2\u{202f}180 écoutes, 8\u{202f}540 minutes, avec June en tête."
        );
        let mut one = payload();
        one.partial = false;
        one.year = 2025;
        one.totals.listens = 1;
        one.totals.minutes = 1;
        one.tops.artists.clear();
        assert_eq!(
            share_text(&one),
            "Mon année 2025 en musique : 1 écoute, 1 minute."
        );
    }

    #[test]
    fn poster_lines_gives_six_bounded_lines() {
        let lines = poster_lines(&payload());
        assert_eq!(lines.len(), 6);
        for line in &lines {
            assert!(
                line.chars().count() <= 38,
                "\"{line}\" is {} chars",
                line.chars().count()
            );
        }
        assert_eq!(lines[0], "Ton année 2026, en cours");
        assert_eq!(lines[1], "8\u{202f}540 minutes");
        assert_eq!(lines[2], "2\u{202f}180 écoutes");
        assert_eq!(lines[3], "Artiste : June");
        assert!(lines[4].starts_with("Titre : "));
        assert!(lines[4].ends_with('…'));
        assert_eq!(lines[5], "Ton mois le plus fort : juillet");
    }

    #[test]
    fn ellipsis_cuts_to_the_maximum_ellipsis_included() {
        assert_eq!(ellipsis("abcdef", 10), "abcdef");
        assert_eq!(ellipsis("abcdef", 4), "abc…");
        assert_eq!(ellipsis("abcdef", 1), "…");
        assert_eq!(ellipsis("  a\n b  ", 10), "a b");
        assert!(ellipsis(TITLE_LONG, 38).chars().count() <= 38);
        assert!(
            ellipsis("8\u{202f}540 minutes", 38).contains('\u{202f}'),
            "the thousands separator is never turned into a plain space"
        );
    }
}
