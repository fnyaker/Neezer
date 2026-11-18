//! The lyric card's RULES — `webapp/src/lib/lyriccard.js`, ported.
//!
//! The SPA splits its card in two: a canvas that draws 1080×1350 pixels
//! (`LyricCard.svelte`) and a pure module that decides everything a person can
//! feel — which lines a card can offer, which one it opens on, what a tap does at
//! the limit, the order of the words, the file's name. This file is the pure
//! half, so the decisions are held by Rust tests instead of by a screenshot.
//!
//! # Where the other half lives, and why
//!
//! The DRAWING is Android's (`Bitmap` + `Canvas` + the design system's own
//! faces), not Rust's. `wrapText`/`fitLyricLayout` measure text with the font's
//! real metrics, and the mobile Rust workspace carries no text rasteriser and no
//! image decoder; the fonts that must be measured are `:designsystem`'s Android
//! resources. The lot's section of `docs/agents/mobile.md` carries the measured
//! reason and the deviation it costs.
//!
//! What is ported here, one function per rule:
//!
//! - [`lines`] — the selectable lines of either lyric shape, marker lines
//!   dropped, and `source` keeping the index in the ORIGINAL array so the line
//!   being heard still resolves after a `♪` between two verses.
//! - [`default_line`] — the line being heard, else the first; a line that was
//!   skipped (an instrumental marker) resolves to the words just before it.
//! - [`toggle`] — one tap: add, remove, and the two honest refusals (`min`, one
//!   line always says something; `max`, three is the card's own limit).
//! - [`choose`] — the selected lines' text in LYRIC order, never tap order.
//! - [`filename`] — `neezer-<artist>-<title>.png`, safe on every filesystem and
//!   share target.

use super::Lyrics;

/// How many lines a card can carry (`MAX_LINES`).
pub const MAX_LINES: usize = 3;

/// One line a card can offer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CardLine {
    pub text: String,
    /// Milliseconds, for a synced answer; `None` for plain words.
    pub time_ms: Option<u32>,
    /// The index this line had in the ORIGINAL synced array (or in the plain
    /// text's lines). What [`default_line`] compares the playing line with.
    pub source: u32,
}

/// One tap's answer: the new selection (in lyric order) and, when nothing
/// changed, why — `"min"` or `"max"`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    pub indices: Vec<u32>,
    pub reason: Option<String>,
}

/// Is this line WORDS, or an instrumental marker?
///
/// `lyriccard.js` asks a regex for a letter or a number: `• • •` and `♪` are
/// markers, "Oh !" is words. `char::is_alphanumeric` is that class for every
/// script the app can meet (a title in Japanese is words, not a marker).
fn is_words(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
}

/// The lines a card can offer, from either lyric shape.
pub fn lines(lyrics: &Lyrics) -> Vec<CardLine> {
    let mut out = Vec::new();
    if !lyrics.synced.is_empty() {
        for (index, line) in lyrics.synced.iter().enumerate() {
            let text = line.text.trim();
            if text.is_empty() || !is_words(text) {
                continue;
            }
            out.push(CardLine {
                text: text.to_owned(),
                time_ms: Some(line.time_ms),
                source: index as u32,
            });
        }
        return out;
    }
    for (index, line) in lyrics.plain.split('\n').enumerate() {
        let text = line.trim().trim_end_matches('\r');
        if text.is_empty() || !is_words(text) {
            continue;
        }
        out.push(CardLine {
            text: text.to_owned(),
            time_ms: None,
            source: index as u32,
        });
    }
    out
}

/// Which line the card opens on: the one being heard, else the first.
pub fn default_line(lines: &[CardLine], active_source: i64) -> u32 {
    if lines.is_empty() {
        return 0;
    }
    if active_source < 0 {
        return 0;
    }
    let mut before = 0;
    for (index, line) in lines.iter().enumerate() {
        if i64::from(line.source) == active_source {
            return index as u32;
        }
        if i64::from(line.source) < active_source {
            before = index;
        } else {
            break;
        }
    }
    before as u32
}

/// One tap on `index`.
pub fn toggle(selected: &[u32], index: u32) -> Selection {
    let mut current: Vec<u32> = selected.to_vec();
    current.sort_unstable();
    current.dedup();
    if current.contains(&index) {
        if current.len() <= 1 {
            return Selection {
                indices: current,
                reason: Some("min".to_owned()),
            };
        }
        return Selection {
            indices: current.into_iter().filter(|kept| *kept != index).collect(),
            reason: None,
        };
    }
    if current.len() >= MAX_LINES {
        return Selection {
            indices: current,
            reason: Some("max".to_owned()),
        };
    }
    current.push(index);
    current.sort_unstable();
    Selection {
        indices: current,
        reason: None,
    }
}

/// The text of the selected lines, in LYRIC order, at most [`MAX_LINES`].
pub fn choose(lines: &[CardLine], selected: &[u32]) -> Vec<String> {
    let mut picked: Vec<u32> = selected.to_vec();
    picked.sort_unstable();
    picked.dedup();
    picked
        .into_iter()
        .filter(|index| (*index as usize) < lines.len())
        .take(MAX_LINES)
        .map(|index| lines[index as usize].text.clone())
        .collect()
}

/// A filename anything can accept: `neezer-<artist>-<title>.png`.
///
/// Neezer's own prefix, the two names as the person reads them, lowercased and
/// with the diacritics folded (`Beyoncé` → `beyonce`) so the file survives every
/// filesystem, mail client and messaging app. Path separators, reserved
/// characters and control characters become word boundaries; runs collapse; each
/// part is bounded so a 300-character title cannot produce an unopenable name.
///
/// The SPA normalises with `String.prototype.normalize("NFKD")`; std has no
/// Unicode tables, so the fold is the Latin one written out ([`fold`]) plus the
/// combining marks a decomposed input carries. A title in another script keeps
/// its letters — which is the point of the rule (a name that can be read).
pub fn filename(artist: &str, title: &str) -> String {
    let artist = slug(artist, 40);
    let title = slug(title, 64);
    let title = if title.is_empty() {
        "titre".to_owned()
    } else {
        title
    };
    match artist.is_empty() {
        true => format!("neezer-{title}.png"),
        false => format!("neezer-{artist}-{title}.png"),
    }
}

/// The lowercase, accent-folded, hyphen-joined form of `value`, at most `limit`
/// characters (and never ending on a hyphen).
fn slug(value: &str, limit: usize) -> String {
    let mut out = String::with_capacity(value.len());
    let mut pending_separator = false;
    for character in value.chars() {
        // Control characters are REMOVED, not turned into a word boundary: a
        // corrupted byte inside "Titre" must not become "t-itre".
        if character.is_control() {
            continue;
        }
        let folded = match fold(character) {
            Some(mapped) => mapped,
            None => continue,
        };
        if folded.is_alphanumeric() {
            if pending_separator && !out.is_empty() {
                out.push('-');
            }
            pending_separator = false;
            for lower in folded.to_lowercase() {
                out.push(lower);
            }
        } else {
            pending_separator = true;
        }
    }
    if out.chars().count() > limit {
        let cut: String = out.chars().take(limit).collect();
        out = cut;
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// The Latin fold (`é` → `e`, `ß` → `ss`, `œ` → `oe`), the combining marks being
/// dropped. `None` for a mark that only decorates the letter before it.
fn fold(character: char) -> Option<char> {
    // Combining diacritics (a decomposed input): the letter before it already
    // carries the sound.
    if ('\u{0300}'..='\u{036F}').contains(&character) {
        return None;
    }
    let folded = match character {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => 'e',
        'ğ' | 'ģ' => 'g',
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' => 'i',
        'ł' | 'ľ' | 'ĺ' => 'l',
        'ñ' | 'ń' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => 'o',
        'ŕ' | 'ř' => 'r',
        'ś' | 'š' | 'ş' => 's',
        'ť' | 'ţ' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' | 'ų' => 'u',
        'ý' | 'ÿ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        'æ' => 'a', // "ae" keeps its sound; one letter is what a slug needs
        'ß' => 's',
        'œ' => 'o',
        other => other,
    };
    Some(folded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nowplaying::LyricLine;

    fn synced(lines: &[(u32, &str)]) -> Lyrics {
        Lyrics {
            synced: lines
                .iter()
                .map(|(time, text)| LyricLine {
                    time_ms: *time,
                    text: (*text).to_owned(),
                })
                .collect(),
            plain: String::new(),
            source: "archive".to_owned(),
        }
    }

    /// A marker between two verses is not a line — and it must not shift the
    /// selection either, which is what `source` exists for.
    #[test]
    fn a_marker_is_not_a_line_and_keeps_its_index() {
        let lyrics = synced(&[(0, "Premier vers"), (2_000, "♪"), (4_000, "Second vers")]);
        let lines = lines(&lyrics);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].text, "Second vers");
        assert_eq!(lines[1].source, 2, "the ORIGINAL index, not the new one");
        // The playing line is the marker: the words just before it are chosen.
        assert_eq!(default_line(&lines, 1), 0);
        assert_eq!(default_line(&lines, 2), 1);
        assert_eq!(default_line(&lines, -1), 0);
        assert_eq!(default_line(&[], 3), 0);
    }

    /// Plain words are lines too, blank paragraphs dropped.
    #[test]
    fn plain_words_are_lines() {
        let lyrics = Lyrics {
            synced: Vec::new(),
            plain: "Une ligne\n\n  Une autre  \n• • •\nTrois".to_owned(),
            source: String::new(),
        };
        let lines = lines(&lyrics);
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Une ligne", "Une autre", "Trois"]
        );
        assert!(lines.iter().all(|line| line.time_ms.is_none()));
    }

    /// One tap: add (in lyric order), remove, and the two refusals.
    #[test]
    fn a_tap_adds_removes_and_refuses_at_the_two_limits() {
        let first = toggle(&[], 1);
        assert_eq!(first.indices, vec![1]);
        assert_eq!(first.reason, None);

        let second = toggle(&[1], 3);
        assert_eq!(second.indices, vec![1, 3]);
        let third = toggle(&[1, 3], 0);
        assert_eq!(third.indices, vec![0, 1, 3], "lyric order, not tap order");

        let full = toggle(&[0, 1, 3], 5);
        assert_eq!(full.indices, vec![0, 1, 3], "nothing changed");
        assert_eq!(full.reason.as_deref(), Some("max"));

        let last = toggle(&[2], 2);
        assert_eq!(last.indices, vec![2]);
        assert_eq!(last.reason.as_deref(), Some("min"));

        let back = toggle(&[0, 1, 3], 1);
        assert_eq!(back.indices, vec![0, 3]);
        assert_eq!(back.reason, None);
    }

    /// The texts are read in lyric order and bounded at three.
    #[test]
    fn chosen_texts_are_in_lyric_order() {
        let lyrics = synced(&[(0, "a"), (1, "b"), (2, "c"), (3, "d")]);
        let lines = lines(&lyrics);
        assert_eq!(choose(&lines, &[3, 0]), vec!["a", "d"]);
        // A fourth tap is refused by `toggle`; `choose` itself keeps the
        // FIRST three in lyric order (the SPA's `.slice(0, max)`).
        assert_eq!(choose(&lines, &[3, 2, 1, 0]), vec!["a", "b", "c"]);
        assert_eq!(choose(&lines, &[9]), Vec::<String>::new());
    }

    /// The file's name survives every filesystem and share target.
    #[test]
    fn a_filename_is_safe_and_readable() {
        assert_eq!(filename("Beyoncé", "Déjà Vu"), "neezer-beyonce-deja-vu.png");
        assert_eq!(
            filename("AC/DC", "Back in Black"),
            "neezer-ac-dc-back-in-black.png"
        );
        assert_eq!(filename("", ""), "neezer-titre.png");
        assert_eq!(
            filename("Sigur Rós", "Hoppípolla"),
            "neezer-sigur-ros-hoppipolla.png"
        );
        // Control characters vanish instead of splitting a word.
        assert_eq!(filename("Ti\u{7}tre", "x"), "neezer-titre-x.png");
        // A title in another script keeps its letters.
        assert_eq!(
            filename("坂本龍一", "戦場のメリークリスマス"),
            "neezer-坂本龍一-戦場のメリークリスマス.png"
        );
        // Bounded, and never ending on a hyphen.
        let long = filename(&"a".repeat(300), &"b".repeat(300));
        assert!(long.len() < 120, "{}", long.len());
        assert!(!long.contains("--"));
        assert!(long.ends_with(".png"));
    }

    /// The same two names twice give the same file (a share that is not a
    /// different file every time).
    #[test]
    fn a_filename_is_deterministic() {
        assert_eq!(filename("Artiste", "Titre"), filename("Artiste", "Titre"));
    }
}
