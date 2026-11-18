//! The pure rules the podcast screens ported from the SPA — and nothing else.
//!
//! Two files of `webapp/src/lib` decide everything a podcast screen computes
//! without a request, and both are ported here function for function so a
//! behaviour cannot drift:
//!
//! | This module | The SPA |
//! |---|---|
//! | [`fold_text`] | `lib/transcript.js#foldText` (through `appcore::fold`) |
//! | [`language_label`] | `lib/transcript.js#languageLabel` |
//! | [`reason_text`] | `lib/transcript.js#reasonText` |
//! | [`is_active_status`], [`is_terminal_status`] | `lib/transcript.js#ACTIVE_STATUSES`/`#TERMINAL_STATUSES` |
//! | [`segment_at`] | `lib/transcript.js#segmentAt` |
//! | [`page_info`] | `lib/transcript.js#pageInfo` |
//! | [`batch_options`] | `lib/transcript.js#batchOptions` |
//! | [`highlight_parts`], [`excerpt_around`] | `lib/transcript.js#highlightParts`/`#excerptAround` |
//! | [`chapter_at`], [`next_chapter`], [`chapter_end`], [`chapter_label`], [`chapter_count_label`] | `lib/chapters.js` |
//! | [`normalize_chapters`] | `lib/chapters.js#normalizeChapters` |
//! | [`date_label`], [`duration_label`] | `lib/chapters.js#dateLabel`, `lib/format.js#duration` |
//!
//! Those functions hold the SPA's numbers (a transcript page is 200 lines, the
//! batch offers 5/10/20, an excerpt is 240 characters), so a screen that reads
//! them cannot invent a fourth behaviour. Nothing here touches the network or a
//! type that crosses the FFI.

use appcore::fold::fold_into;

/// The languages the transcription picker offers, `auto` first
/// (`transcript.js#LANGUAGES`).
pub const LANGUAGES: [&str; 7] = ["auto", "fr", "en", "es", "de", "it", "pt"];

/// A transcription in the server's queue (`ACTIVE_STATUSES`).
pub const ACTIVE_STATUSES: [&str; 3] = ["queued", "leased", "running"];

/// A transcription that will not move by itself (`TERMINAL_STATUSES`).
pub const TERMINAL_STATUSES: [&str; 3] = ["ready", "error", "stopped"];

/// Case- and accent-blind text, so a search typed from a phone keyboard finds
/// « Beyoncé » (`transcript.js#foldText`, through `appcore::fold` — the fold the
/// lists already use, so a search and a list filter agree).
pub fn fold_text(value: &str) -> String {
    let mut out = Vec::with_capacity(value.len());
    fold_into(value.as_bytes(), &mut out);
    String::from_utf8_lossy(&out).into_owned()
}

/// « Français », « Détection automatique », or the code in capitals
/// (`languageLabel`).
pub fn language_label(code: &str) -> String {
    match code.to_ascii_lowercase().as_str() {
        "" => String::new(),
        "auto" => "Détection automatique".to_owned(),
        "fr" => "Français".to_owned(),
        "en" => "Anglais".to_owned(),
        "es" => "Espagnol".to_owned(),
        "de" => "Allemand".to_owned(),
        "it" => "Italien".to_owned(),
        "pt" => "Portugais".to_owned(),
        other => other.to_uppercase(),
    }
}

/// Why the server cannot transcribe, in the SPA's own sentence
/// (`reasonText`). An unknown reason says the generic one, never a code.
pub fn reason_text(reason: &str) -> String {
    match reason {
        "disabled" => "La transcription est désactivée sur ce serveur.".to_owned(),
        "engine_missing" => {
            "Le moteur de transcription n'est pas installé sur le serveur.".to_owned()
        }
        "model_missing" => "Le modèle de transcription est introuvable sur le serveur.".to_owned(),
        "model_invalid" => {
            "Le fichier de modèle n'est pas un modèle whisper.cpp valide.".to_owned()
        }
        _ => "La transcription n'est pas configurée sur ce serveur.".to_owned(),
    }
}

/// A status the poller waits for.
pub fn is_active_status(status: &str) -> bool {
    ACTIVE_STATUSES.contains(&status)
}

/// A status that is over: the panel stops polling on it.
pub fn is_terminal_status(status: &str) -> bool {
    TERMINAL_STATUSES.contains(&status)
}

/// The last segment that started at or before `seconds` (`segmentAt`): a binary
/// search, because a one-hour transcript is 600 lines and this runs on every
/// position tick that moves.
pub fn segment_at(starts: &[f64], seconds: f64) -> Option<usize> {
    if starts.is_empty() || !seconds.is_finite() {
        return None;
    }
    let mut low = 0usize;
    let mut high = starts.len() - 1;
    let mut found: Option<usize> = None;
    while low <= high {
        let middle = (low + high) / 2;
        let start = starts[middle];
        if start.is_finite() && start <= seconds {
            found = Some(middle);
            low = middle + 1;
        } else if middle == 0 {
            break;
        } else {
            high = middle - 1;
        }
    }
    found
}

/// One page of a paged read (`pageInfo`): which page, how many, and the rows it
/// covers. `page` is 1-based, as the panel writes « page 2 sur 5 ».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PageInfo {
    pub page: u32,
    pub pages: u32,
    pub from: u32,
    pub to: u32,
}

/// `pageInfo(offset, limit, total)`, arithmetic and clamps included.
pub fn page_info(offset: u32, limit: u32, total: u32) -> PageInfo {
    let step = limit.max(1);
    let pages = total.div_ceil(step).max(1);
    let raw = offset / step + 1;
    let page = raw.clamp(1, pages);
    let from = if total == 0 { 0 } else { (page - 1) * step };
    let to = total.min(from.saturating_add(step));
    PageInfo {
        page,
        pages,
        from,
        to,
    }
}

/// The batch sizes the transcription card offers (`batchOptions`): 5, then 10
/// and 20 once there are that many eligible episodes.
pub fn batch_options(eligible: u32) -> Vec<u32> {
    if eligible == 0 {
        return Vec::new();
    }
    let mut out = vec![eligible.min(5)];
    if eligible >= 10 {
        out.push(10);
    }
    if eligible >= 20 {
        out.push(20);
    }
    out
}

/// One piece of a line with the search's words marked (`highlightParts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPart {
    pub text: String,
    pub marked: bool,
}

/// `highlightParts(text, query)`: the whole query first, then each word, the
/// ranges merged, so a line never gets two overlapping marks.
pub fn highlight_parts(text: &str, query: &str) -> Vec<TextPart> {
    let plain = |text: &str| {
        vec![TextPart {
            text: text.to_owned(),
            marked: false,
        }]
    };
    if text.is_empty() {
        return plain(text);
    }
    let words: Vec<&str> = query
        .split_whitespace()
        .filter(|word| !word.is_empty())
        .collect();
    if words.is_empty() {
        return plain(text);
    }
    // Lower-cased text that keeps a map back to the ORIGINAL byte offsets, so a
    // match can be marked in the text the person reads. `to_lowercase` can
    // change a character's byte length (`İ` becomes two chars), which is why the
    // map is per byte rather than per character.
    let mut lowered = String::with_capacity(text.len());
    let mut map: Vec<usize> = Vec::with_capacity(text.len() + 1);
    for (at, character) in text.char_indices() {
        for lower in character.to_lowercase() {
            let mut buffer = [0u8; 4];
            let encoded = lower.encode_utf8(&mut buffer);
            for _ in 0..encoded.len() {
                map.push(at);
            }
            lowered.push(lower);
        }
    }
    map.push(text.len());
    let occurrences = |needle: &str| -> Vec<(usize, usize)> {
        let mut found = Vec::new();
        if needle.is_empty() {
            return found;
        }
        let mut from = 0usize;
        while from + needle.len() <= lowered.len() {
            let Some(at) = lowered[from..].find(needle) else {
                break;
            };
            let start = from + at;
            let end = start + needle.len();
            found.push((map[start], map[end]));
            from = end;
        }
        found
    };
    let mut ranges = occurrences(&query.trim().to_lowercase());
    if ranges.is_empty() {
        for word in &words {
            ranges.extend(occurrences(&word.to_lowercase()));
        }
    }
    if ranges.is_empty() {
        return plain(text);
    }
    ranges.sort_by(|left, right| left.0.cmp(&right.0).then(right.1.cmp(&left.1)));
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.0 < last.1 => last.1 = last.1.max(range.1),
            _ => merged.push(range),
        }
    }
    let mut parts = Vec::new();
    let mut at = 0usize;
    for (start, end) in merged {
        if start > at {
            parts.push(TextPart {
                text: text[at..start].to_owned(),
                marked: false,
            });
        }
        parts.push(TextPart {
            text: text[start..end].to_owned(),
            marked: true,
        });
        at = end;
    }
    if at < text.len() {
        parts.push(TextPart {
            text: text[at..].to_owned(),
            marked: false,
        });
    }
    parts
}

/// `excerptAround(text, query, max)`: at most `max` characters, centred on the
/// first matching word, with `…` on the side that was cut.
pub fn excerpt_around(text: &str, query: &str, max: usize) -> String {
    let size = max.max(8);
    let length = text.chars().count();
    if length <= size {
        return text.to_owned();
    }
    let lower = text.to_lowercase();
    let mut found: Option<usize> = None;
    for word in query.split_whitespace().filter(|word| !word.is_empty()) {
        if let Some(byte_at) = lower.find(&word.to_lowercase()) {
            let at = lower[..byte_at].chars().count();
            if found.is_none_or(|current| at < current) {
                found = Some(at);
            }
        }
    }
    let Some(found) = found else {
        let head: String = text.chars().take(size - 1).collect();
        return format!("{head}…");
    };
    let mut start = found.saturating_sub(size / 2);
    let mut end = (start + size).min(length);
    start = end.saturating_sub(size);
    let ellipses = usize::from(start > 0) + usize::from(end < length);
    if ellipses > 0 {
        let room = size.saturating_sub(ellipses);
        end = (start + room).min(length);
        start = end.saturating_sub(room);
    }
    let body: String = text.chars().skip(start).take(end - start).collect();
    format!(
        "{}{body}{}",
        if start > 0 { "…" } else { "" },
        if end < length { "…" } else { "" }
    )
}

/// A published date as the SPA writes it (`dateLabel`: `toLocaleDateString`
/// with `fr-FR`'s short month) — derived from the Unix seconds, no locale data
/// needed for the twelve month names a French screen uses.
pub fn date_label(unix_seconds: i64) -> String {
    if unix_seconds <= 0 {
        return String::new();
    }
    let days = unix_seconds.div_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let names = [
        "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
        "déc.",
    ];
    let name = names
        .get((month as usize).saturating_sub(1))
        .copied()
        .unwrap_or("");
    format!("{day} {name} {year}")
}

/// Days since the Unix epoch → `(year, month, day)`, Howard Hinnant's
/// `civil_from_days` (the algorithm every date library uses, 20 lines and no
/// dependency).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

/// `format.js#duration`: `m:ss`, or `h:mm:ss` past the hour — an episode is
/// rarely under one, and « 92:15 » is not a duration anyone reads.
pub fn duration_label(seconds: u32) -> String {
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let rest = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{rest:02}")
    } else {
        format!("{minutes}:{rest:02}")
    }
}

/// One chapter, normalised the way `normalizeChapters` does: whole seconds, a
/// sort by start, and the first chapter of two that start together wins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterCard {
    /// The chapter's own index, in the order the server sent it.
    pub index: u32,
    pub start: u32,
    /// `None` when the server sent no end, or one before the start.
    pub end: Option<u32>,
    pub title: String,
}

/// `normalizeChapters`: whole seconds, sorted, one per start, a title trimmed.
///
/// `raw` is `(index, start, end, title)` exactly as the route sends it: an
/// `end` may be `null`, and a start arrives as a float of seconds.
pub fn normalize_chapters(raw: &[(u32, f64, Option<f64>, String)]) -> Vec<ChapterCard> {
    let mut out: Vec<ChapterCard> = Vec::with_capacity(raw.len());
    for (index, start, end, title) in raw {
        if !start.is_finite() || *start < 0.0 {
            continue;
        }
        let start = start.round() as u32;
        let end = end
            .filter(|value| value.is_finite())
            .map(|value| value.round().max(0.0) as u32)
            .filter(|value| *value >= start);
        out.push(ChapterCard {
            index: *index,
            start,
            end,
            title: title.trim().to_owned(),
        });
    }
    out.sort_by_key(|chapter| chapter.start);
    let mut seen = std::collections::HashSet::new();
    out.retain(|chapter| seen.insert(chapter.start));
    out
}

/// The chapter playing at `seconds` (`chapterAt`): the last one that started at
/// or before it, and none once it has passed that chapter's own end.
pub fn chapter_at(chapters: &[ChapterCard], seconds: f64) -> Option<(usize, &ChapterCard)> {
    if chapters.is_empty() || !seconds.is_finite() {
        return None;
    }
    if seconds < f64::from(chapters[0].start) {
        return None;
    }
    let mut best: Option<usize> = None;
    for (index, chapter) in chapters.iter().enumerate() {
        if f64::from(chapter.start) <= seconds {
            best = Some(index);
        } else {
            break;
        }
    }
    let index = best?;
    let chapter = &chapters[index];
    if let Some(end) = chapter.end {
        if seconds >= f64::from(end) {
            return None;
        }
    }
    Some((index, chapter))
}

/// The chapter after `seconds` (`nextChapter`): what the panel says « dans … ».
pub fn next_chapter(chapters: &[ChapterCard], seconds: f64) -> Option<(usize, &ChapterCard)> {
    let at = chapters
        .iter()
        .position(|chapter| f64::from(chapter.start) > seconds)?;
    chapters.get(at).map(|chapter| (at, chapter))
}

/// Where a chapter ends (`chapterEnd`): its own end, else the next chapter's
/// start, else the episode's duration when that is past its start.
pub fn chapter_end(chapters: &[ChapterCard], index: usize, fallback: u32) -> u32 {
    let Some(chapter) = chapters.get(index) else {
        return 0;
    };
    if let Some(end) = chapter.end {
        return end;
    }
    if let Some(after) = chapters.get(index + 1) {
        return after.start;
    }
    if fallback > chapter.start {
        fallback
    } else {
        chapter.start
    }
}

/// « Chapitre 3 · Le nom » (`chapterLabel`).
pub fn chapter_label(chapters: &[ChapterCard], index: usize) -> String {
    let Some(chapter) = chapters.get(index) else {
        return String::new();
    };
    let base = format!("Chapitre {}", index + 1);
    if chapter.title.is_empty() {
        base
    } else {
        format!("{base} · {}", chapter.title)
    }
}

/// « 3 chapitres » / « 1 chapitre » (`chapterCountLabel`).
pub fn chapter_count_label(count: u32) -> String {
    if count > 1 {
        format!("{count} chapitres")
    } else {
        format!("{count} chapitre")
    }
}

/// `podcastAbout`: the episode has something to say beyond its line — a
/// description, chapters or an archived file.
pub fn podcast_about(description: &str, chapter_count: u32, archived: bool) -> bool {
    !description.trim().is_empty() || chapter_count > 0 || archived
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The numbers the SPA holds: a transcript page is 200 lines, the batch's
    /// sizes are 5/10/20, an excerpt is 240 characters.
    #[test]
    fn the_spas_numbers_are_the_ported_ones() {
        assert_eq!(batch_options(0), Vec::<u32>::new());
        assert_eq!(batch_options(3), vec![3]);
        assert_eq!(batch_options(7), vec![5]);
        assert_eq!(batch_options(10), vec![5, 10]);
        assert_eq!(batch_options(25), vec![5, 10, 20]);
        assert_eq!(
            excerpt_around(&"a".repeat(300), "a", 240).chars().count(),
            240
        );
        // 240 characters of excerpt, one ellipsis: 239 of text.
        let long = "mot ".repeat(100);
        let cut = excerpt_around(&long, "mot", 240);
        assert!(cut.starts_with('…') || cut.ends_with('…'), "{cut}");
        assert!(cut.chars().count() <= 240, "{}", cut.chars().count());
    }

    /// A page is 1-based and clamped, exactly as `pageInfo` clamps it.
    #[test]
    fn page_info_is_one_based_and_clamped() {
        assert_eq!(
            page_info(0, 200, 0),
            PageInfo {
                page: 1,
                pages: 1,
                from: 0,
                to: 0
            }
        );
        assert_eq!(
            page_info(0, 200, 450),
            PageInfo {
                page: 1,
                pages: 3,
                from: 0,
                to: 200
            }
        );
        assert_eq!(
            page_info(200, 200, 450),
            PageInfo {
                page: 2,
                pages: 3,
                from: 200,
                to: 400
            }
        );
        assert_eq!(
            page_info(10_000, 200, 450),
            PageInfo {
                page: 3,
                pages: 3,
                from: 400,
                to: 450
            }
        );
    }

    /// A search typed without accents finds an accented line, and the fold is
    /// the lists' own (`appcore::fold`).
    #[test]
    fn folding_finds_an_accented_line_from_a_plain_keyboard() {
        assert_eq!(fold_text("Beyoncé — ÉTÉ"), "beyonce - ete");
        assert!(fold_text("L'émission").contains("l'emission"));
    }

    /// The current segment is the last one that started at or before the
    /// position; before the first line there is none.
    #[test]
    fn segment_at_is_a_binary_search_on_the_starts() {
        let starts = [0.0, 12.5, 30.0, 61.0];
        assert_eq!(segment_at(&starts, 0.0), Some(0));
        assert_eq!(segment_at(&starts, 12.5), Some(1));
        assert_eq!(segment_at(&starts, 40.0), Some(2));
        assert_eq!(segment_at(&starts, 1_000.0), Some(3));
        assert_eq!(segment_at(&[], 4.0), None);
        assert_eq!(segment_at(&starts, f64::NAN), None);
    }

    /// A chapter's unit is SECONDS and a start tie keeps the first, which is
    /// what `normalizeChapters` promises (a marker's unit is milliseconds —
    /// that conversion is `views`'s, not this module's).
    #[test]
    fn chapters_normalise_sort_and_dedupe() {
        let raw = vec![
            (1u32, 30.4f64, Some(59.6f64), " Deux ".to_owned()),
            (0, 0.0, None, "".to_owned()),
            (2, 30.2, Some(10.0), "Trop court".to_owned()),
        ];
        let chapters = normalize_chapters(&raw);
        assert_eq!(chapters.len(), 2, "one chapter per start");
        assert_eq!(chapters[0].start, 0);
        assert_eq!(chapters[1].start, 30);
        assert_eq!(chapters[1].title, "Deux");
        // The end the FIRST of the two carried survives: `normalizeChapters`
        // dedupes by start, never by end.
        assert_eq!(chapters[1].end, Some(60));
        assert_eq!(chapter_end(&chapters, 1, 3_600), 60);
        // An end before the start is no end at all.
        let backwards = normalize_chapters(&[(0, 10.0, Some(2.0), String::new())]);
        assert_eq!(backwards[0].end, None);
        assert_eq!(chapter_label(&chapters, 0), "Chapitre 1");
        assert_eq!(chapter_label(&chapters, 1), "Chapitre 2 · Deux");
        assert_eq!(chapter_count_label(1), "1 chapitre");
        assert_eq!(chapter_count_label(3), "3 chapitres");
    }

    /// The playing chapter, and the one after it, at four moments.
    #[test]
    fn a_chapter_is_current_only_inside_its_own_span() {
        let chapters = normalize_chapters(&[
            (0, 0.0, Some(60.0), "Intro".to_owned()),
            (1, 60.0, Some(120.0), "Suite".to_owned()),
        ]);
        assert_eq!(chapter_at(&chapters, 30.0).map(|(at, _)| at), Some(0));
        assert_eq!(chapter_at(&chapters, 60.0).map(|(at, _)| at), Some(1));
        assert_eq!(chapter_at(&chapters, 59.5).map(|(at, _)| at), Some(0));
        assert_eq!(next_chapter(&chapters, 30.0).map(|(at, _)| at), Some(1));
        assert_eq!(next_chapter(&chapters, 130.0), None);
        // A chapter with a declared end stops being current past it.
        let ended = normalize_chapters(&[(0, 0.0, Some(10.0), String::new())]);
        assert_eq!(chapter_at(&ended, 11.0).map(|(at, _)| at), None);
    }

    /// A search's marks cover the whole query first, then each word, merged.
    #[test]
    fn highlights_merge_and_never_overlap() {
        // The WHOLE query is marked first, as `highlightParts` does: « chat
        // noir » is one range, not two words with a gap between them.
        let parts = highlight_parts("le chat noir", "chat noir");
        assert_eq!(
            parts,
            vec![
                TextPart {
                    text: "le ".to_owned(),
                    marked: false
                },
                TextPart {
                    text: "chat noir".to_owned(),
                    marked: true
                },
            ]
        );
        // A query that is not a substring falls back to each word.
        let parts = highlight_parts("le chat noir", "noir chat");
        assert_eq!(
            parts.iter().filter(|part| part.marked).count(),
            2,
            "each word marked: {parts:?}"
        );
        assert_eq!(
            highlight_parts("rien ici", "zzz"),
            vec![TextPart {
                text: "rien ici".to_owned(),
                marked: false
            }]
        );
        // Multi-byte text is cut on a character boundary, never inside one.
        let parts = highlight_parts("Été à Paris", "paris");
        assert_eq!(parts.last().unwrap().text, "Paris");
    }

    /// A French date, and a duration past the hour, as the SPA writes them.
    #[test]
    fn dates_and_durations_read_as_french() {
        // 2026-11-18 (UTC): the day the note was written.
        assert_eq!(date_label(1_794_960_000), "18 nov. 2026");
        assert_eq!(date_label(0), "");
        assert_eq!(duration_label(95), "1:35");
        assert_eq!(duration_label(3_725), "1:02:05");
    }

    /// A bilingual sentence for every reason the server can name, so no code
    /// ever reaches a screen.
    #[test]
    fn every_reason_has_a_french_sentence() {
        for reason in [
            "disabled",
            "engine_missing",
            "model_missing",
            "model_invalid",
            "something_else",
        ] {
            let text = reason_text(reason);
            assert!(text.ends_with('.'), "{reason}: {text}");
            assert!(!text.contains('_'), "{reason} leaked a code: {text}");
        }
        assert_eq!(language_label("fr"), "Français");
        assert_eq!(language_label("pt"), "Portugais");
        assert_eq!(language_label("xx"), "XX");
        assert!(is_active_status("running"));
        assert!(!is_active_status("ready"));
        assert!(is_terminal_status("stopped"));
    }
}
