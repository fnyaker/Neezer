//! Raw whisper.cpp segments → the absolute, bounded, de-duplicated lines the
//! server stores.
//!
//! A line-by-line port of `neezer/transcribe/engine.py` (`_milliseconds`,
//! `clean_text`, `normalize_segments`), because a transcript produced here and
//! one produced by the server must be the same text, not merely similar: an
//! episode transcribed on this laptop and the same episode transcribed on the
//! server can both land in the same database, and the SPA's transcript search
//! folds over whatever came first.
//!
//! The rules, and why each exists:
//!
//! - A chunk is [`CHUNK_SECONDS`] of audio and its first [`OVERLAP_SECONDS`]
//!   belong to the previous chunk, which already emitted them: a segment that
//!   *ends* before the cutoff is dropped, or a resumed run repeats two seconds
//!   of text. Chunk 0 has no previous chunk, so a cutoff of 0 keeps what a
//!   segment at the very beginning says.
//! - Everything is clamped to `[chunk_start, chunk_start + duration]`, so one
//!   hallucinated timestamp cannot put a line ten minutes past the end of the
//!   episode.
//! - Text is cleaned as `clean_text` cleans it: control characters become
//!   spaces, whitespace runs collapse, a line that is only a tag (`[Musique]`,
//!   `[]`) is dropped, and 2 000 characters is the ceiling.
//! - At most [`MAX_SEGMENTS_PER_CHUNK`] lines per chunk, which is what
//!   `job.py#apply_chunk` accepts.
//!
//! `tests/golden/` holds this file to the Python functions byte for byte; the
//! cases are written by `desktop/tools/transcribe_golden.py`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Audio per chunk, in seconds (`engine.CHUNK_SECONDS`, `worker.py`'s same
/// constant, and what `POST …/chunk`'s `duration_ms` must not exceed).
pub const CHUNK_SECONDS: i64 = 600;

/// Seconds a chunk shares with the previous one (`engine.OVERLAP_SECONDS`).
pub const OVERLAP_SECONDS: i64 = 2;

/// [`OVERLAP_SECONDS`] in milliseconds: the default overlap of a chunk.
pub const OVERLAP_MS: i64 = OVERLAP_SECONDS * 1000;

/// The engine's sample rate: 16 kHz mono is what whisper.cpp takes and what the
/// `engine` child's PCM file holds (`engine.SAMPLE_RATE`).
pub const SAMPLE_RATE: u32 = 16_000;

/// Lines kept per chunk; `job.py#apply_chunk` refuses more (`MAX_SEGMENTS_PER_CHUNK`).
pub const MAX_SEGMENTS_PER_CHUNK: usize = 2000;

/// Characters kept per line (`engine.MAX_TEXT`, `sidecar.MAX_TEXT`).
pub const MAX_TEXT: usize = 2000;

/// One timed line, the shape of a chunk's `segments`, of the sidecar's
/// `segments` and of a database row (`PodcastTranscriptSegment`).
///
/// Serialized as `{"start_ms":…,"end_ms":…,"text":…}` in that order, which is
/// also the order the server's own JSON uses: the golden files compare bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Segment {
    /// Absolute position of the first word, in milliseconds from the start of
    /// the episode (never from the start of the chunk).
    pub start_ms: i64,
    /// Absolute end of the last word, in milliseconds; always `> start_ms`.
    pub end_ms: i64,
    /// The line, cleaned and at most [`MAX_TEXT`] characters.
    pub text: String,
}

impl Segment {
    /// A segment, for tests and for the port's callers that already hold clean
    /// values.
    pub fn new(start_ms: i64, end_ms: i64, text: impl Into<String>) -> Self {
        Segment {
            start_ms,
            end_ms,
            text: text.into(),
        }
    }

    /// Milliseconds of speech this line covers.
    pub fn duration_ms(&self) -> i64 {
        self.end_ms - self.start_ms
    }
}

/// Raw whisper segments of one chunk → absolute, clamped, clean lines.
///
/// `items` is the `transcription` array of whisper.cpp's JSON (`-oj`), passed as
/// JSON values on purpose: `engine.py` reads it untyped (`item["offsets"]["from"]`),
/// a malformed entry is skipped instead of failing the chunk, and a value that is
/// not a number (a string, a boolean) is skipped too.
///
/// `chunk_start_ms` is the chunk's absolute position, `chunk_duration_ms` its
/// length (clamped to at least 1 ms, as Python does), and `overlap_ms` the part
/// of it the previous chunk already emitted ([`OVERLAP_MS`] in normal use).
pub fn normalize_segments(
    items: &[Value],
    chunk_start_ms: i64,
    chunk_duration_ms: i64,
    overlap_ms: i64,
) -> Vec<Segment> {
    let chunk_start = chunk_start_ms.max(0);
    let chunk_end = chunk_start.saturating_add(chunk_duration_ms.max(1));
    // Chunk 0 has no previous chunk: its head is the episode's head.
    let cutoff = if chunk_start > 0 {
        chunk_start.saturating_add(overlap_ms.max(0))
    } else {
        0
    };
    let mut out: Vec<Segment> = Vec::new();
    for item in items {
        if out.len() >= MAX_SEGMENTS_PER_CHUNK {
            break;
        }
        // `item.get` on a non-object is `None`, which is Python's
        // `not isinstance(item, dict)` guard.
        let Some(offsets) = item.get("offsets").and_then(Value::as_object) else {
            continue;
        };
        let (Some(from), Some(to)) = (
            milliseconds(offsets.get("from")),
            milliseconds(offsets.get("to")),
        ) else {
            continue;
        };
        let start = from.max(0).saturating_add(chunk_start);
        let end = to.max(0).saturating_add(chunk_start);
        if end <= start {
            continue;
        }
        if cutoff != 0 && end <= cutoff {
            continue;
        }
        let start = start.max(chunk_start);
        let end = end.min(chunk_end);
        if end <= start {
            continue;
        }
        let Some(text) = clean_text(item.get("text")) else {
            continue;
        };
        out.push(Segment {
            start_ms: start,
            end_ms: end,
            text,
        });
    }
    out.sort_by_key(|segment| (segment.start_ms, segment.end_ms));
    out
}

/// One line of a transcript, or `None` when there is nothing to keep.
///
/// Exactly `engine.py#clean_text`: `[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]` → space
/// (tabs, newlines and carriage returns are *not* in that class; the whitespace
/// pass below collapses them, as in Python), whitespace runs → one space, then
/// strip; a line that is only a bracketed tag is not speech; the result is cut
/// at [`MAX_TEXT`] characters (code points, as Python slices) and stripped
/// again, and an empty result is `None`.
pub fn clean_text(raw: Option<&Value>) -> Option<String> {
    let raw = raw?.as_str()?;
    let mut text = String::with_capacity(raw.len());
    for character in raw.chars() {
        match character {
            '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{7f}' => text.push(' '),
            other => text.push(other),
        }
    }
    let collapsed = collapse_whitespace(&text);
    if collapsed.is_empty() || is_tag_only(&collapsed) {
        return None;
    }
    let cut: String = collapsed.chars().take(MAX_TEXT).collect();
    let trimmed = cut.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// `re.sub(r"\s+", " ", text).strip()`: every whitespace run becomes one space,
/// and the string loses its own leading and trailing whitespace.
///
/// Rust's [`char::is_whitespace`] and Python's `str.isspace()` agree on every
/// character that can still be here (the control class above already turned
/// `U+001C`–`U+001F` into spaces, the one place they used to differ).
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space_due = false;
    for character in text.chars() {
        if character.is_whitespace() {
            space_due = !out.is_empty();
        } else {
            if space_due {
                out.push(' ');
                space_due = false;
            }
            out.push(character);
        }
    }
    out
}

/// `^\[[^\]]*\]$`: the whole line is one bracketed tag, so it is not speech.
fn is_tag_only(text: &str) -> bool {
    text.starts_with('[')
        && text.ends_with(']')
        && text.len() >= 2
        && !text[1..text.len() - 1].contains(']')
}

/// `engine.py#_milliseconds`: a JSON number as whole milliseconds.
///
/// Bools and strings are not numbers (Python's `isinstance(value, (int, float))`
/// is false for a bool on purpose), and rounding is `round()`'s: HALF TO EVEN
/// (`round(1500.5) == 1500`, `round(1501.5) == 1502`). Whisper's own offsets are
/// integers, but a float offset that rounds the other way would move a line by a
/// millisecond and break the byte equality the golden files check.
fn milliseconds(value: Option<&Value>) -> Option<i64> {
    let number = value?.as_f64()?;
    if !number.is_finite() {
        return None;
    }
    Some(number.round_ties_even() as i64)
}

#[cfg(test)]
mod tests {
    use super::{MAX_TEXT, OVERLAP_MS, Segment, clean_text, normalize_segments};
    use serde_json::{Value, json};

    fn items(texts: &[(&str, i64, i64)]) -> Vec<Value> {
        texts
            .iter()
            .map(|(text, from, to)| json!({"offsets": {"from": from, "to": to}, "text": text}))
            .collect()
    }

    /// The overlap rule, as `tests/test_transcribe.py` states it: the head of a
    /// chunk repeats the tail of the previous one and is dropped, the rest is
    /// absolute and clamped, and 2 000 characters is the ceiling.
    #[test]
    fn a_chunk_drops_the_previous_chunks_tail_and_stays_inside_itself() {
        let raw = items(&[
            ("dans l'overlap", 0, 1900),
            ("à cheval", 2500, 5000),
            ("[Musique]", 5000, 9000),
            ("  Une   phrase.  ", 9000, 12_000),
            ("hors du chunk", 601_000, 602_000),
            ("", 1, 2),
            (&"x".repeat(5000), 3000, 4000),
        ]);
        let out = normalize_segments(&raw, 600_000, 600_000, OVERLAP_MS);
        assert_eq!(
            out.iter()
                .map(|s| (s.start_ms, s.end_ms))
                .collect::<Vec<_>>(),
            [(602_500, 605_000), (603_000, 604_000), (609_000, 612_000)]
        );
        assert_eq!(
            out.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
            ["à cheval", "x".repeat(MAX_TEXT).as_str(), "Une phrase."]
        );
        assert!(out.iter().all(|s| s.end_ms <= 1_200_000));
    }

    /// Chunk 0 is the episode's head: nothing of it can belong to a previous
    /// chunk, so its first words are kept even with the overlap set.
    #[test]
    fn the_first_chunk_keeps_its_first_words() {
        let raw = items(&[("Bonjour", 0, 500)]);
        let out = normalize_segments(&raw, 0, 600_000, OVERLAP_MS);
        assert_eq!(out, [Segment::new(0, 500, "Bonjour")]);
    }

    /// Python rounds halves to even; rounding away from zero (Rust's `round`)
    /// moves lines by a millisecond and the sidecar stops matching the server's.
    #[test]
    fn a_fractional_offset_rounds_half_to_even() {
        let raw = vec![
            json!({"offsets": {"from": 1500.5, "to": 1501.5}, "text": "a"}),
            json!({"offsets": {"from": 2500.5, "to": 4000.5}, "text": "b"}),
        ];
        let out = normalize_segments(&raw, 0, 60_000, OVERLAP_MS);
        assert_eq!(
            out.iter()
                .map(|s| (s.start_ms, s.end_ms))
                .collect::<Vec<_>>(),
            [(1500, 1502), (2500, 4000)]
        );
    }

    /// A tag is a bracketed line with no bracket inside; two tags on one line are
    /// speech as far as the rule is concerned, and so is text around a bracket.
    #[test]
    fn only_a_whole_line_in_brackets_is_a_tag() {
        let tag = |text: &str| clean_text(Some(&json!(text)));
        assert_eq!(tag("[Musique]"), None);
        assert_eq!(tag("[]"), None);
        assert_eq!(tag("[ Applaudissements ]"), None);
        assert_eq!(tag("[a][b]").as_deref(), Some("[a][b]"));
        assert_eq!(tag("Musique]").as_deref(), Some("Musique]"));
        assert_eq!(tag("["), Some("[".to_owned()));
        assert_eq!(tag("[rire] bonjour").as_deref(), Some("[rire] bonjour"));
    }

    /// Control characters become spaces, whitespace collapses, and the cut is in
    /// characters, not bytes.
    #[test]
    fn text_is_cleaned_the_way_the_server_cleans_it() {
        let clean = |raw: Value| clean_text(Some(&raw));
        assert_eq!(clean(json!("a\u{0}b\u{7f}c")).as_deref(), Some("a b c"));
        assert_eq!(clean(json!("  \t\n  ")), None);
        assert_eq!(
            clean(json!("é".repeat(2500))).unwrap().chars().count(),
            2000
        );
        // Whitespace collapses before the cut, so 2 000 spaces in front of a word
        // leave the word alone (the cut can never empty a collapsed line).
        assert_eq!(
            clean(json!(format!("{}{}", " ".repeat(2000), "mot"))).as_deref(),
            Some("mot")
        );
        // A bool is not text: `isinstance(raw, str)` is false.
        assert_eq!(clean(json!(true)), None);
        assert_eq!(clean(json!(42)), None);
    }

    /// Malformed entries are skipped, never fatal: the engine's JSON is a log of
    /// what it heard, and one broken line must not lose the chunk.
    #[test]
    fn a_broken_entry_is_skipped_not_fatal() {
        let raw = vec![
            json!(42),
            json!({"text": "sans offsets"}),
            json!({"offsets": "nope", "text": "x"}),
            json!({"offsets": {"from": "12", "to": 20}, "text": "x"}),
            json!({"offsets": {"from": true, "to": 20}, "text": "x"}),
            json!({"offsets": {"from": 10, "to": 10}, "text": "x"}),
            json!({"offsets": {"from": 30, "to": 20}, "text": "x"}),
            json!({"offsets": {"from": 100, "to": 200}, "text": "la seule"}),
        ];
        let out = normalize_segments(&raw, 0, 60_000, OVERLAP_MS);
        assert_eq!(out, [Segment::new(100, 200, "la seule")]);
    }

    /// The cap `job.py#apply_chunk` enforces: 2 001 lines in, 2 000 out.
    #[test]
    fn a_chunk_keeps_at_most_two_thousand_lines() {
        let raw: Vec<Value> = (0..2001)
            .map(|i| json!({"offsets": {"from": i * 10, "to": i * 10 + 5}, "text": "a"}))
            .collect();
        let out = normalize_segments(&raw, 0, 600_000, OVERLAP_MS);
        assert_eq!(out.len(), 2000);
        assert_eq!(out[0].start_ms, 0);
        assert_eq!(out[1999].start_ms, 19_990);
    }
}
