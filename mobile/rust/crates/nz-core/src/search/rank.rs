//! `webapp/src/lib/searchrank.js`, ported: how the search's "Tout" tab is put
//! together, and how close a YouTube hit is to what somebody typed.
//!
//! Why this file exists at all rather than a server sort: the SPA decides the
//! order of a search's answer on the CLIENT (`rankSearchResults`), and the phone
//! must show the same list for the same answer. So the rules come across
//! verbatim — the bars, the version penalty, the cosmetic decorations, the
//! library-first order — and the tests here are the SPA's own
//! (`webapp/test/searchrank.test.mjs`), one for one.
//!
//! The four things it decides:
//!
//! - [`top_result`] — the ONE artist (or show) the query plainly names, shown
//!   first and large; only a clear match earns the place.
//! - [`mix_podcasts`] — the shows among the albums, after the first two, so a
//!   show the query names is not buried at the end.
//! - [`match_score`] — how much of what was typed a title and its artist account
//!   for, 0..1. [`YOUTUBE_MIN`] > [`CATALOGUE_MIN`]: a video only LEADS when it
//!   is strictly closer than the best catalogue answer.
//! - [`rank_search_results`] — the catalogue's order kept, library rows first,
//!   the fallback's hits woven into the SAME list, an identical recording
//!   dropped.
//!
//! `appcore::fold` is the one that folds text (accents, case, ligatures): the
//! same function the lists filter with, so "beyonce" finds "Beyoncé" here too.

use std::collections::BTreeSet;

use appcore::fold::fold_into;
use nz_model::{Album, Artist, PodcastCard, Track};

/// The main source's bar: below it, a catalogue row is not a comparable match.
pub const CATALOGUE_MIN: f64 = 0.5;
/// YouTube's bar: a video must be a real match, not just a word in its title.
pub const YOUTUBE_MIN: f64 = 0.75;

/// How many leading answers are candidates for the "best result" card
/// (`searchrank.js#topResult`'s `slice(0, 3)`: the catalogue lists by
/// popularity, so the third "Indila" is a tribute act and the first is not).
pub const TOP_WINDOW: usize = 3;
/// How many albums stay in front of the shows (`mixPodcasts`'s `after`).
pub const SHOWS_AFTER: usize = 2;

/// Words that name a DIFFERENT recording of a title: a plain query does not ask
/// for them, so a video carrying one is demoted below the studio version.
const VARIANT: [&str; 26] = [
    "cover",
    "live",
    "remix",
    "remixed",
    "mix",
    "edit",
    "version",
    "instrumental",
    "karaoke",
    "tribute",
    "acoustic",
    "demo",
    "reprise",
    "extended",
    "radio",
    "sped",
    "slowed",
    "nightcore",
    "bootleg",
    "mashup",
    "reaction",
    "8d",
    "orchestral",
    "piano",
    "a-capella",
    "acapella",
];

/// Words that DO change which recording it is, as `searchrank.js#VARIANT_RE`
/// lists them (the same set minus `8d`, `version`, `mix`, `edit` and the two
/// that are not words a title carries — the SPA's own two lists differ, and
/// porting the difference is the point).
const RECORDING_MARKERS: [&str; 20] = [
    "cover",
    "live",
    "remix",
    "remixed",
    "instrumental",
    "karaoke",
    "tribute",
    "acoustic",
    "demo",
    "reprise",
    "extended",
    "sped",
    "slowed",
    "nightcore",
    "bootleg",
    "mashup",
    "reaction",
    "orchestral",
    "acapella",
    "a-capella",
];

/// Decorations that do NOT change which recording it is: an official clip, a
/// lyric video, an audio upload. The SPA writes them as one regular expression
/// (`searchrank.js#COSMETIC_RE`); here the same alternatives are matched word by
/// word on the folded span, which is what `\b…\b` decides — "hd" matches "(HD)"
/// and not "(HDMI)", and the two spellings of each pair are both listed.
const COSMETIC: [&str; 16] = [
    "official",
    "officiel",
    "officielle",
    "clip",
    "video",
    "vidéo",
    "audio",
    "lyric",
    "lyrics",
    "paroles",
    "visualiser",
    "visualizer",
    "hd",
    "hq",
    "4k",
    "mv",
];

/// The folded form of `text`: what every comparison in this file is made on.
///
/// This is `lib/genre/rank.js#fold`, NOT the byte fold the lists filter with:
/// the SPA's search fold lowercases, drops accents and turns every run of
/// non-alphanumerics into ONE space, so "(Live)" becomes the word `live` (which
/// is what lets the version penalty see it) and "Dernière danse!" ends without
/// punctuation. `appcore::fold` provides the accent half (é → e, œ → oe, the
/// combining marks a decomposed string carries); the rest is the SPA's own rule.
pub fn fold(text: &str) -> String {
    let mut base = Vec::with_capacity(text.len());
    fold_into(text.as_bytes(), &mut base);
    let base = String::from_utf8(base).unwrap_or_default();
    let mut out = String::with_capacity(base.len());
    for ch in base.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim().to_owned()
}

/// The folded words of a text, in order (`searchrank.js#wordsOf`).
fn words_of(text: &str) -> Vec<String> {
    fold(text)
        .split(' ')
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

fn set_of(text: &str) -> BTreeSet<String> {
    words_of(text).into_iter().collect()
}

/// The name a track is credited to (the full credit line when there is one),
/// `searchrank.js#artistOf`.
pub fn artist_of(track: &Track) -> String {
    if let Some(line) = track
        .display_artist
        .as_deref()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        return line.to_owned();
    }
    let names: Vec<&str> = track
        .artists
        .iter()
        .map(|artist| artist.name.trim())
        .filter(|name| !name.is_empty())
        .collect();
    if !names.is_empty() {
        return names.join(", ");
    }
    track.artist.name.trim().to_owned()
}

/// F1 of two word sets: both "what was typed is there" and "nothing extra".
fn overlap_f1(asked: &BTreeSet<String>, got: &BTreeSet<String>) -> f64 {
    if asked.is_empty() || got.is_empty() {
        return 0.0;
    }
    let hit = asked.intersection(got).count() as f64;
    if hit == 0.0 {
        return 0.0;
    }
    let precision = hit / got.len() as f64;
    let recall = hit / asked.len() as f64;
    (2.0 * precision * recall) / (precision + recall)
}

/// A version marker the query did not ask for demotes a title. Capped at three
/// steps so a pile of suffixes cannot reach zero.
fn variant_penalty(title_words: &[String], asked: &BTreeSet<String>) -> f64 {
    let extra = title_words
        .iter()
        .filter(|word| VARIANT.contains(&word.as_str()) && !asked.contains(*word))
        .count();
    if extra == 0 {
        1.0
    } else {
        0.72f64.powi(extra.min(3) as i32)
    }
}

/// How much of `query` a track's title and artist account for, 0..1.
///
/// Half is coverage (every word typed is found somewhere in the credit), half is
/// the title's F1 against the words the artist's name does not explain — so
/// "Daft Punk" does not count against a title query, and a title with words
/// nobody typed ("(Live)", "Remix") is not as close as the plain one.
pub fn match_score(query: &str, track: &Track) -> f64 {
    let asked = set_of(query);
    if asked.is_empty() {
        return 0.0;
    }
    let title_words = words_of(&track.title);
    let title: BTreeSet<String> = title_words.iter().cloned().collect();
    let artist = set_of(&artist_of(track));
    let title_asked: BTreeSet<String> = asked.difference(&artist).cloned().collect();
    let title_score = if title_asked.is_empty() {
        1.0
    } else {
        overlap_f1(&title_asked, &title)
    };
    let covered = asked
        .iter()
        .filter(|word| title.contains(*word) || artist.contains(*word))
        .count();
    let coverage = covered as f64 / asked.len() as f64;
    let score = (coverage + title_score) / 2.0;
    (score * variant_penalty(&title_words, &asked)).clamp(0.0, 1.0)
}

/// Whether a bracketed span carries a word from `words`, on word boundaries
/// (`\b…\b` over the folded span: `hd` matches "(HD)" and not "(HDMI)").
fn span_has(span: &str, words: &[&str]) -> bool {
    words_of(span)
        .iter()
        .any(|word| words.contains(&word.as_str()))
}

/// A title without its cosmetic decorations ("(Clip officiel)", "(Audio)"), so
/// two uploads of the same recording compare equal. A LIVE or REMIX marker is
/// NOT removed: it names another recording on purpose.
fn canonical_title(track: &Track) -> String {
    let raw = track.title.as_str();
    let mut kept = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find(['(', '[']) {
        kept.push_str(&rest[..start]);
        let open = rest.as_bytes()[start] as char;
        let close = if open == '(' { ')' } else { ']' };
        match rest[start + 1..].find(close) {
            Some(end) => {
                let inside = &rest[start + 1..start + 1 + end];
                // Kept as a space either way, so the two sides never run into
                // each other's words.
                if !(span_has(inside, &COSMETIC) && !span_has(inside, &RECORDING_MARKERS)) {
                    kept.push(open);
                    kept.push_str(inside);
                    kept.push(close);
                } else {
                    kept.push(' ');
                }
                rest = &rest[start + 1 + end + 1..];
            }
            None => {
                // An unclosed bracket is text, not decoration.
                kept.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    kept.push_str(rest);
    fold(&kept)
}

/// The identity of a recording for de-duplication (`""` when there is no title).
pub fn recording_key(track: &Track) -> String {
    let title = canonical_title(track);
    if title.is_empty() {
        String::new()
    } else {
        format!("{title}\u{0}{}", fold(&artist_of(track)))
    }
}

/// Whether two rows are the same recording, decorations aside.
///
/// The rule `rank_search_results` applies internally; exposed because the SPA
/// exports it (`sameRecording`) and a later lot comparing two answers wants the
/// same verdict. Nothing in this lot calls it outside its test.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the SPA's own export, used by the port's tests")
)]
pub fn same_recording(a: &Track, b: &Track) -> bool {
    let key = recording_key(a);
    !key.is_empty() && key == recording_key(b)
}

/// The best result: the one artist (or show) the query plainly names, or
/// nothing.
///
/// Nothing is not a failure: a top card that is not what was typed pushes the
/// titles down for nothing, so a title search gets no artist on top.
#[derive(Debug, Clone, PartialEq)]
pub enum Top {
    Artist(Artist),
    /// A show, only for a screen that shows shows (the admin's).
    Show(PodcastCard),
}

/// `searchrank.js#topResult`.
pub fn top_result(
    query: &str,
    artists: &[Artist],
    shows: &[PodcastCard],
    with_podcasts: bool,
) -> Option<Top> {
    let exact = fold(query);
    if exact.is_empty() {
        return None;
    }
    // An artist named exactly, among the first few.
    if let Some(artist) = artists
        .iter()
        .take(TOP_WINDOW)
        .find(|artist| fold(&artist.name) == exact)
    {
        return Some(Top::Artist(artist.clone()));
    }
    if with_podcasts
        && let Some(show) = shows
            .iter()
            .take(TOP_WINDOW)
            .find(|show| fold(&show.title) == exact)
    {
        return Some(Top::Show(show.clone()));
    }
    // Else only the FIRST artist, and only when the query is the start of its
    // name: what somebody types before they have finished typing it. Folded
    // names are words joined by single spaces, so a prefix of the whole name
    // starts on a word: "daft" and "daft p" name Daft Punk, "aft" does not.
    let first = artists.first()?;
    let name = fold(&first.name);
    if exact.chars().count() >= 3 && name.starts_with(&exact) {
        return Some(Top::Artist(first.clone()));
    }
    None
}

/// One entry of the albums shelf: an album, or a show mixed in among them.
#[derive(Debug, Clone, PartialEq)]
pub enum Mixed {
    Album(Album),
    Show(PodcastCard),
}

/// The albums with the shows mixed in after the first two: what matched best
/// stays first, and a show the query names is not buried at the end
/// (`searchrank.js#mixPodcasts`).
pub fn mix_podcasts(albums: Vec<Album>, shows: Vec<PodcastCard>, after: usize) -> Vec<Mixed> {
    if shows.is_empty() {
        return albums.into_iter().map(Mixed::Album).collect();
    }
    let mut out = Vec::with_capacity(albums.len() + shows.len());
    let after = after.min(albums.len());
    let mut albums = albums;
    let tail = albums.split_off(after);
    out.extend(albums.into_iter().map(Mixed::Album));
    out.extend(shows.into_iter().map(Mixed::Show));
    out.extend(tail.into_iter().map(Mixed::Album));
    out
}

/// What [`rank_search_results`] did, so a test (and the PR) can state it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ranked {
    /// The list the screen draws: library rows, promoted videos, the catalogue,
    /// then the rest of the fallback's hits.
    pub tracks: Vec<Track>,
    pub promoted: usize,
    pub appended: usize,
    pub dropped: usize,
}

/// The catalogue's results with the YouTube hits in the SAME list.
///
/// The catalogue's own order is preserved and library rows (`local`) stay first.
/// A YouTube hit is dropped when it duplicates a catalogue row (same identity, or
/// the same recording once decorations are stripped) or when its score is under
/// [`YOUTUBE_MIN`]. A surviving hit that is STRICTLY closer than the best
/// comparable catalogue answer is promoted to just after the library rows; the
/// others follow the catalogue (the catalogue wins a tie).
///
/// `searchrank.js#rankSearchResults`.
pub fn rank_search_results(query: &str, catalogue: &[Track], youtube: &[Track]) -> Ranked {
    let mut ranked = Ranked::default();
    let mut best = 0.0f64;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for track in catalogue {
        let score = match_score(query, track);
        if score >= CATALOGUE_MIN && score > best {
            best = score;
        }
        let key = recording_key(track);
        if !key.is_empty() {
            seen.insert(key);
        }
        ids.insert(identity(track));
    }

    let mut kept: Vec<(Track, f64)> = Vec::new();
    for track in youtube {
        if ids.contains(&identity(track)) {
            ranked.dropped += 1;
            continue;
        }
        let key = recording_key(track);
        if !key.is_empty() && seen.contains(&key) {
            ranked.dropped += 1;
            continue;
        }
        let score = match_score(query, track);
        if score < YOUTUBE_MIN {
            ranked.dropped += 1;
            continue;
        }
        if !key.is_empty() {
            seen.insert(key);
        }
        kept.push((track.clone(), score));
    }
    // Strongest first, and stable: two videos with the same score keep the
    // order the fallback gave them.
    kept.sort_by(|a, b| b.1.total_cmp(&a.1));

    let mut promoted = Vec::new();
    let mut appended = Vec::new();
    for (track, score) in kept {
        if score > best {
            promoted.push(track);
        } else {
            appended.push(track);
        }
    }
    ranked.promoted = promoted.len();
    ranked.appended = appended.len();

    let mut out = Vec::with_capacity(catalogue.len() + promoted.len() + appended.len());
    out.extend(catalogue.iter().filter(|track| track.local).cloned());
    out.extend(promoted);
    out.extend(catalogue.iter().filter(|track| !track.local).cloned());
    out.extend(appended);
    ranked.tracks = out;
    ranked
}

/// A row's own identity, as the wire names it: its row UUID when the library
/// holds it, else the provider's id (`trackkey.js#trackKey`'s rule, without the
/// UUID hash: two forms of one row always share their [`recording_key`]).
pub fn identity(track: &Track) -> String {
    match track.id.as_ref() {
        Some(id) => id.to_string(),
        None => track.deezer_id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_model::{ArtistId, TrackAlbum, TrackArtist, TrackArtistRole, TrackId, TrackRowId};

    /// The SPA's own fixtures (`webapp/test/searchrank.test.mjs`), in Rust.
    fn artist(id: &str, name: &str) -> Artist {
        Artist {
            deezer_id: ArtistId::new(id),
            name: name.to_owned(),
            ..Artist::default()
        }
    }

    fn show(id: &str, title: &str) -> PodcastCard {
        PodcastCard {
            deezer_id: id.to_owned(),
            title: title.to_owned(),
            ..PodcastCard::default()
        }
    }

    fn album(id: &str) -> Album {
        Album {
            deezer_id: nz_model::AlbumId::new(id),
            title: format!("Album {id}"),
            ..Album::default()
        }
    }

    fn youtube(title: &str, artist_name: &str) -> Track {
        Track {
            deezer_id: TrackId::new(format!("yt-{title}")),
            title: title.to_owned(),
            artist: TrackArtist {
                deezer_id: ArtistId::new(""),
                name: artist_name.to_owned(),
            },
            display_artist: Some(artist_name.to_owned()),
            source: Some("youtube".to_owned()),
            local: true,
            ..Track::default()
        }
    }

    fn catalogue(title: &str, artist_name: &str) -> Track {
        Track {
            deezer_id: TrackId::new(format!("dz-{title}")),
            title: title.to_owned(),
            artist: TrackArtist {
                deezer_id: ArtistId::new(title.len().to_string()),
                name: artist_name.to_owned(),
            },
            artists: vec![TrackArtistRole {
                deezer_id: ArtistId::new(""),
                name: artist_name.to_owned(),
                role: "Main".to_owned(),
            }],
            album: TrackAlbum {
                title: "Album".to_owned(),
                ..TrackAlbum::default()
            },
            ..Track::default()
        }
    }

    const Q: &str = "derniere danse indila";

    #[test]
    fn an_artist_named_exactly_leads_accents_and_case_aside() {
        let artists = [artist("1", "Stromae Tribute"), artist("2", "Stromaé")];
        match top_result("stromae", &artists, &[], false) {
            Some(Top::Artist(found)) => assert_eq!(found.name, "Stromaé"),
            other => panic!("expected the exact artist, got {other:?}"),
        }
    }

    #[test]
    fn the_first_artist_leads_on_the_start_of_its_name_never_from_inside_a_word() {
        let artists = [artist("1", "Daft Punk"), artist("2", "Daftside")];
        let name = |query: &str| match top_result(query, &artists, &[], false) {
            Some(Top::Artist(found)) => found.name,
            _ => String::new(),
        };
        assert_eq!(name("daft"), "Daft Punk");
        assert_eq!(name("daft p"), "Daft Punk");
        assert_eq!(name("aft"), "", "never from inside a word");
        assert_eq!(name("da"), "", "two letters name nothing yet");
    }

    #[test]
    fn a_title_search_does_not_get_an_artist_on_top() {
        let artists = [artist("1", "Indila")];
        assert_eq!(
            top_result("derniere danse", &artists, &[], false),
            None,
            "the query is not the start of the artist's name"
        );
    }

    #[test]
    fn a_show_named_exactly_leads_only_where_podcasts_are_shown() {
        let shows = [show("1", "Les Pieds sur terre")];
        assert_eq!(top_result("les pieds sur terre", &[], &shows, false), None);
        assert!(matches!(
            top_result("les pieds sur terre", &[], &shows, true),
            Some(Top::Show(_))
        ));
    }

    #[test]
    fn shows_are_mixed_in_after_the_first_two_albums() {
        let albums = vec![album("a"), album("b"), album("c")];
        let mixed = mix_podcasts(albums, vec![show("p", "Show")], SHOWS_AFTER);
        assert_eq!(mixed.len(), 4);
        assert!(matches!(mixed[0], Mixed::Album(_)));
        assert!(matches!(mixed[1], Mixed::Album(_)));
        assert!(matches!(mixed[2], Mixed::Show(_)), "the show after two");
        assert!(matches!(mixed[3], Mixed::Album(_)));
        // No show at all: the albums untouched.
        assert_eq!(mix_podcasts(vec![album("a")], vec![], SHOWS_AFTER).len(), 1);
    }

    #[test]
    fn youtube_must_clear_a_higher_bar_than_a_catalogue_result() {
        const { assert!(YOUTUBE_MIN > CATALOGUE_MIN) };
    }

    #[test]
    fn a_title_exactly_as_typed_is_a_full_match() {
        assert_eq!(
            match_score("Dernière danse", &youtube("Dernière danse", "Indila")),
            1.0
        );
        assert_eq!(
            match_score("DERNIERE DANSE", &youtube("Dernière danse", "Indila")),
            1.0
        );
        assert_eq!(match_score(Q, &youtube("Dernière danse", "Indila")), 1.0);
    }

    #[test]
    fn a_partial_title_scores_above_zero_but_below_the_exact_one() {
        let exact = match_score("derniere danse", &youtube("Dernière danse", "Indila"));
        let short = match_score("derniere danse", &youtube("Dernière", "Indila"));
        assert!(short > 0.0);
        assert!(short < exact);
        assert!(match_score("derniere", &youtube("Dernière danse", "Indila")) > 0.6);
    }

    #[test]
    fn another_artist_does_not_clear_youtubes_bar() {
        let right = match_score(Q, &youtube("Dernière danse", "Indila"));
        let wrong = match_score(Q, &youtube("Dernière danse", "The Cover Band"));
        assert!(wrong < right);
        assert!(wrong < YOUTUBE_MIN, "the wrong artist scored {wrong}");
    }

    #[test]
    fn cover_live_remix_and_other_versions_are_demoted_below_the_plain_title() {
        let plain = match_score(Q, &youtube("Dernière danse", "Indila"));
        for title in [
            "Dernière danse (Live)",
            "Dernière danse (Remix)",
            "Dernière danse (Cover)",
            "Dernière danse (Reprise)",
            "Dernière danse (Instrumental)",
        ] {
            let score = match_score(Q, &youtube(title, "Indila"));
            assert!(score < plain, "{title}: {score} vs {plain}");
            assert!(score < YOUTUBE_MIN, "{title} scored {score}");
        }
        // Typing "live" asks for that recording, so it is not a version.
        assert!(
            match_score(
                "derniere danse live indila",
                &youtube("Dernière danse (Live)", "Indila")
            ) >= YOUTUBE_MIN
        );
    }

    #[test]
    fn the_cosmetic_decorations_of_an_upload_do_not_make_it_another_recording() {
        let deezer = catalogue("Dernière danse", "Indila");
        assert!(same_recording(
            &deezer,
            &youtube("Dernière danse (Clip officiel)", "Indila")
        ));
        assert!(same_recording(
            &deezer,
            &youtube("Dernière danse (Official Audio)", "Indila")
        ));
        assert!(
            !same_recording(&deezer, &youtube("Dernière danse (Live)", "Indila")),
            "a live version is another recording on purpose"
        );
    }

    #[test]
    fn an_exact_youtube_duplicate_is_never_a_second_line() {
        let deezer = catalogue("Dernière danse", "Indila");
        let out = rank_search_results(
            Q,
            std::slice::from_ref(&deezer),
            &[youtube("Dernière danse", "Indila")],
        );
        assert_eq!(out.tracks, vec![deezer.clone()]);
        assert_eq!(out.promoted, 0);
        assert_eq!(
            out.dropped, 1,
            "the identical video is dropped, not appended"
        );

        let clip = rank_search_results(
            Q,
            std::slice::from_ref(&deezer),
            &[youtube("Dernière danse (Clip officiel)", "Indila")],
        );
        assert_eq!(clip.tracks, vec![deezer]);
        assert_eq!(clip.dropped, 1);
    }

    #[test]
    fn a_closer_video_is_promoted_after_the_library_before_the_catalogue() {
        let mut local = catalogue("Autre chose", "Indila");
        local.local = true;
        let cat = catalogue("Dernière danse (Live)", "Indila");
        let out = rank_search_results(Q, &[local, cat], &[youtube("Dernière danse", "Indila")]);
        let titles: Vec<&str> = out.tracks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Autre chose", "Dernière danse", "Dernière danse (Live)"]
        );
        assert_eq!(out.promoted, 1);
        assert_eq!(out.appended, 0);
    }

    #[test]
    fn a_tie_keeps_the_catalogue_result_in_front() {
        let deezer = catalogue("Dernière danse 2024", "Indila");
        let video = youtube("Dernière danse 2025", "Indila");
        assert_eq!(match_score(Q, &deezer), match_score(Q, &video));
        let out = rank_search_results(
            Q,
            std::slice::from_ref(&deezer),
            std::slice::from_ref(&video),
        );
        assert_eq!(out.tracks, vec![deezer, video]);
        assert_eq!(out.promoted, 0, "equal is not closer: the catalogue wins");
        assert_eq!(out.appended, 1);
    }

    #[test]
    fn a_weak_or_wrong_artist_video_is_dropped_not_appended() {
        let deezer = catalogue("Dernière danse", "Indila");
        let out = rank_search_results(
            Q,
            std::slice::from_ref(&deezer),
            &[
                youtube("Dernière danse (Live)", "Indila"),
                youtube("Dernière danse", "The Cover Band"),
            ],
        );
        assert_eq!(out.tracks, vec![deezer]);
        assert_eq!(out.dropped, 2);
    }

    #[test]
    fn a_library_row_ahead_of_a_catalogue_row_in_the_same_answer() {
        // The server already orders local rows first; the fold keeps that order
        // and the SPA's rule is the belt to its braces.
        let live = catalogue("Dernière danse", "Indila");
        let mut mine = catalogue("Dernière danse", "Indila");
        mine.id = Some(TrackRowId::new("11111111-1111-4111-8111-111111111111"));
        mine.local = true;
        let out = rank_search_results(Q, &[live.clone(), mine.clone()], &[]);
        assert_eq!(out.tracks, vec![mine, live]);
    }
}
