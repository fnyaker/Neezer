//! `webapp/src/lib/queryradio.js`, ported: a radio started from a SEARCH, not
//! from a title or an artist page. The results already found are the opening
//! titles, and the player's own refill fills the queue from the best one.
//!
//! Why a threshold of results at all: a search that found two answers is a
//! lookup, and a radio built from two titles is a shuffle of those two.
//! [`RADIO_MIN_RESULTS`] (30) is the point where "play what this is about"
//! replaces "open this". The count is what the screen shows across its tabs,
//! each entry once: a video that duplicates a catalogue row is not a second
//! result.
//!
//! Why the queue is filtered by score: [`super::rank::rank_search_results`]
//! answers "which order should the list show"; a radio needs "which titles are
//! really about this". The catalogue's bar is `CATALOGUE_MIN` and the fallback's
//! `YOUTUBE_MIN`, exactly the bars the search already applies to decide what
//! deserves a line — a title under them is another result in the list, never a
//! seed for the radio.
//!
//! **One difference from the SPA, on purpose, and it is cheaper**: the SPA's
//! launch asks `POST /api/radio/next` to top a short queue up, because its
//! `Player.svelte` only refills a Flow. The phone's player OWNS the top-up of a
//! `radio` context (`nz-core::player#need_more`), so a launch here costs **zero
//! requests**: the first title plays at once, from the answer the screen is
//! already drawing, and the queue grows by one request per title as it is needed
//! (rule 1a before 1c).
//!
//! The context this module builds is the one the player already understands
//! (`player/mod.rs#split_context`: `radio:track:<ref>` or `radio:artist:<id>`).

use std::collections::BTreeSet;

use nz_model::Track;

use super::rank::{
    CATALOGUE_MIN, Top, YOUTUBE_MIN, match_score, rank_search_results, recording_key, top_result,
};
use super::wire::Catalogue;

/// How many results a search must show before it deserves a radio.
pub const RADIO_MIN_RESULTS: u32 = 30;

/// Whether a result count is broad enough for the button to appear. The screen
/// keeps its other gates (a term in the box, results for THAT term, the library
/// scope, no pasted playlist link) next to its own state.
pub fn offer_radio(total: u32) -> bool {
    total >= RADIO_MIN_RESULTS
}

/// The identities a track can be reached by: the reference the server streams
/// (`id`, else `deezer_id`) and the recording itself — so the same song found
/// twice, or found again on the fallback under another id, is ONE result.
fn track_keys(track: &Track) -> Vec<String> {
    let mut keys = Vec::with_capacity(2);
    let reference = track
        .id
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| track.deezer_id.to_string());
    if !reference.is_empty() {
        keys.push(format!("r:{reference}"));
    }
    let recording = recording_key(track);
    if !recording.is_empty() {
        keys.push(format!("s:{recording}"));
    }
    keys
}

/// `{id, deezer_id}` — an artist, an album, a playlist or a show. Entities have
/// no `name` key on purpose: two artists can share a name and still be two rows.
fn entity_keys(id: Option<&str>, deezer_id: &str) -> Vec<String> {
    let mut keys = Vec::with_capacity(2);
    if let Some(id) = id.filter(|id| !id.is_empty()) {
        keys.push(format!("i:{id}"));
    }
    if !deezer_id.is_empty() {
        keys.push(format!("d:{deezer_id}"));
    }
    keys
}

/// Adds every key of one item; `true` when one was already seen (a duplicate).
fn add_keys(seen: &mut BTreeSet<String>, keys: &[String]) -> bool {
    if keys.is_empty() {
        return false;
    }
    if keys.iter().any(|key| seen.contains(key)) {
        return true;
    }
    for key in keys {
        seen.insert(key.clone());
    }
    false
}

fn count_unique<T>(
    items: &[T],
    keys_of: impl Fn(&T) -> Vec<String>,
    seen: &mut BTreeSet<String>,
) -> u32 {
    let mut count = 0;
    for item in items {
        let keys = keys_of(item);
        if keys.is_empty() || add_keys(seen, &keys) {
            continue;
        }
        count += 1;
    }
    count
}

/// How many distinct results the search found, across every tab
/// (`queryradio.js#countSearchResults`).
///
/// The fallback's hits are passed only once that source has answered: an
/// unanswered source must not change the count under the person's eyes. Shows
/// are counted only for a screen that shows them (the admin's).
pub fn count_search_results(
    query: &str,
    catalogue: &Catalogue,
    youtube: &[Track],
    with_podcasts: bool,
) -> u32 {
    let ranked = rank_search_results(query.trim(), &catalogue.tracks, youtube);
    let mut seen = BTreeSet::new();
    let mut total = count_unique(&ranked.tracks, track_keys, &mut seen);
    total += count_unique(
        &catalogue.artists,
        |artist| {
            let deezer = artist.deezer_id.to_string();
            entity_keys(artist.id.as_deref(), &deezer)
        },
        &mut seen,
    );
    total += count_unique(
        &catalogue.albums,
        |album| {
            let deezer = album.deezer_id.to_string();
            let row = album.id.as_ref().map(ToString::to_string);
            entity_keys(row.as_deref(), &deezer)
        },
        &mut seen,
    );
    total += count_unique(
        &catalogue.playlists,
        |playlist| entity_keys(None, &playlist.deezer_id),
        &mut seen,
    );
    if with_podcasts {
        total += count_unique(
            &catalogue.podcasts,
            |show| entity_keys(None, &show.deezer_id),
            &mut seen,
        );
    }
    total
}

/// The titles worth opening a radio with: above the search's own bar for their
/// source, de-duplicated by id AND by recording, strongest first. A library row
/// wins a tie (it plays from disk now); the search's own order breaks the rest,
/// so two equally close titles keep the order the screen showed.
pub fn relevant_tracks(query: &str, catalogue: &Catalogue, youtube: &[Track]) -> Vec<Track> {
    let term = query.trim();
    if term.is_empty() {
        return Vec::new();
    }
    let ranked = rank_search_results(term, &catalogue.tracks, youtube).tracks;
    let mut seen = BTreeSet::new();
    let mut kept: Vec<(Track, f64, bool, usize)> = Vec::new();
    for (order, track) in ranked.iter().enumerate() {
        let fallback = track.source.as_deref() == Some("youtube");
        let score = match_score(term, track);
        let bar = if fallback { YOUTUBE_MIN } else { CATALOGUE_MIN };
        if score < bar {
            continue;
        }
        let keys = track_keys(track);
        if keys.is_empty() {
            continue; // nothing playable
        }
        if add_keys(&mut seen, &keys) {
            continue;
        }
        kept.push((track.clone(), score, track.local && !fallback, order));
    }
    kept.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.3.cmp(&b.3))
    });
    kept.into_iter().map(|(track, ..)| track).collect()
}

/// What the radio is anchored on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seed {
    /// A catalogue or library title; `artist` lets the server radio on the
    /// artist when the row is not in the library yet.
    Track { id: String, artist: Option<String> },
    /// An artist the search names, for a search with no title to start from.
    Artist { id: String },
}

impl Seed {
    pub fn kind(&self) -> &'static str {
        match self {
            Seed::Track { .. } => "track",
            Seed::Artist { .. } => "artist",
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Seed::Track { id, .. } | Seed::Artist { id } => id,
        }
    }

    pub fn artist(&self) -> Option<&str> {
        match self {
            Seed::Track { artist, .. } => artist.as_deref(),
            Seed::Artist { .. } => None,
        }
    }
}

/// The name a link names an entity by: its row `id`, else the provider's id
/// (`entity.js#entityRef`).
fn entity_ref(id: Option<&str>, deezer_id: &str) -> Option<String> {
    id.filter(|id| !id.is_empty())
        .map(str::to_owned)
        .or_else(|| (!deezer_id.is_empty()).then(|| deezer_id.to_owned()))
}

/// The artist a search names, for a radio that has no title to start from: the
/// best result's exact/prefix match first, then the catalogue's own first
/// answer. `None` when the search found no artist at all.
fn best_artist(query: &str, catalogue: &Catalogue) -> Option<String> {
    if let Some(Top::Artist(artist)) =
        top_result(query, &catalogue.artists, &catalogue.podcasts, false)
    {
        let deezer = artist.deezer_id.to_string();
        if let Some(reference) = entity_ref(artist.id.as_deref(), &deezer) {
            return Some(reference);
        }
    }
    catalogue.artists.iter().find_map(|artist| {
        let deezer = artist.deezer_id.to_string();
        entity_ref(artist.id.as_deref(), &deezer)
    })
}

/// The radio's seed: the best title when there is one, so the radio sounds like
/// what was typed; otherwise the artist the search names.
///
/// A catalogue title absent from the library still radios on its artist —
/// `/api/radio/next` does that fallback, and only when the seed carries
/// `artist`.
pub fn radio_seed(query: &str, catalogue: &Catalogue, tracks: &[Track]) -> Option<Seed> {
    let term = query.trim();
    if let Some(best) = tracks.first() {
        let id = best
            .id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| best.deezer_id.to_string());
        if !id.is_empty() {
            // The artist id the row is credited to (a YouTube row has none),
            // else the artist the search itself names.
            let credited = best.artist.deezer_id.to_string();
            let artist = (!credited.is_empty())
                .then_some(credited)
                .or_else(|| best_artist(term, catalogue));
            return Some(Seed::Track { id, artist });
        }
    }
    best_artist(term, catalogue).map(|id| Seed::Artist { id })
}

/// What the player carries while this radio plays (`queryradio.js#radioContext`):
/// the honest name of the context ("Radio de patrick sébastien"), and the id the
/// player splits into its seed for every top-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioContext {
    /// Always `"radio"`: the player's own context kind, whose refill asks for the
    /// next title of the seed in `id`.
    pub kind: String,
    /// `track:<ref>` or `artist:<id>` — what `player/mod.rs#split_context` reads.
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
}

pub fn radio_context(query: &str, seed: &Seed) -> RadioContext {
    RadioContext {
        kind: "radio".to_owned(),
        id: format!("{}:{}", seed.kind(), seed.id()),
        title: format!("Radio de {}", query.trim()),
        artist: seed.artist().map(str::to_owned),
    }
}

/// The titles of `incoming` that are not in `existing` yet, by id or by
/// recording (`queryradio.js#mergeRadioTracks`).
///
/// Nothing in this lot calls it outside its test: the phone's top-up is the
/// PLAYER's own refill (`player/mod.rs#need_more`), which already drops what the
/// queue holds. It stays because it is the ported rule — the day a lot appends a
/// `radio_next` answer itself (MB20's party, a station screen), the de-duplication
/// is here instead of being reinvented.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the ported merge a later lot appends through")
)]
pub fn merge_radio_tracks(existing: &[Track], incoming: &[Track]) -> Vec<Track> {
    let mut seen = BTreeSet::new();
    for track in existing {
        add_keys(&mut seen, &track_keys(track));
    }
    let mut out = Vec::new();
    for track in incoming {
        let keys = track_keys(track);
        if keys.is_empty() || add_keys(&mut seen, &keys) {
            continue;
        }
        out.push(track.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_model::{
        Album, AlbumId, Artist, ArtistId, PodcastCard, SearchPlaylist, TrackArtist,
        TrackArtistRole, TrackId, TrackRowId,
    };

    const Q: &str = "patrick sebastien";

    fn dz(number: u32, title: Option<&str>, artist: &str, local: bool) -> Track {
        Track {
            deezer_id: TrackId::new(format!("dz-{number}")),
            title: title
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Titre {number}")),
            artist: TrackArtist {
                deezer_id: ArtistId::new("42"),
                name: artist.to_owned(),
            },
            artists: vec![TrackArtistRole {
                deezer_id: ArtistId::new("42"),
                name: artist.to_owned(),
                role: "Main".to_owned(),
            }],
            display_artist: Some(artist.to_owned()),
            local,
            ..Track::default()
        }
    }

    fn yt(number: u32, title: Option<&str>, artist: &str) -> Track {
        let mut track = dz(number, title, artist, true);
        track.deezer_id = TrackId::new(format!("yt-{number}"));
        track.source = Some("youtube".to_owned());
        track
    }

    fn entity(number: u32) -> Artist {
        Artist {
            deezer_id: ArtistId::new(format!("e-{number}")),
            name: format!("Entité {number}"),
            ..Artist::default()
        }
    }

    fn lots<T>(count: u32, make: impl Fn(u32) -> T) -> Vec<T> {
        (0..count).map(make).collect()
    }

    fn catalogue(tracks: Vec<Track>) -> Catalogue {
        Catalogue {
            tracks,
            ..Catalogue::default()
        }
    }

    fn ids(tracks: &[Track]) -> Vec<String> {
        tracks.iter().map(|t| t.deezer_id.to_string()).collect()
    }

    #[test]
    fn twenty_nine_results_are_a_lookup_and_thirty_are_a_radio() {
        let below = count_search_results(
            Q,
            &catalogue(lots(29, |i| dz(i, None, "Patrick Sébastien", false))),
            &[],
            true,
        );
        let at = count_search_results(
            Q,
            &catalogue(lots(30, |i| dz(i, None, "Patrick Sébastien", false))),
            &[],
            true,
        );
        assert_eq!(below, 29);
        assert_eq!(at, 30);
        assert!(!offer_radio(below));
        assert!(offer_radio(at));
    }

    #[test]
    fn every_tab_counts_and_each_entry_once() {
        let cat = Catalogue {
            tracks: lots(10, |i| dz(i, None, "Patrick Sébastien", false)),
            artists: lots(5, entity),
            albums: lots(8, |i| Album {
                deezer_id: AlbumId::new(format!("al-{i}")),
                ..Album::default()
            }),
            playlists: lots(4, |i| SearchPlaylist {
                deezer_id: format!("pl-{i}"),
                ..SearchPlaylist::default()
            }),
            podcasts: lots(3, |i| PodcastCard {
                deezer_id: format!("po-{i}"),
                ..PodcastCard::default()
            }),
        };
        assert_eq!(count_search_results(Q, &cat, &[], true), 30);
        assert_eq!(
            count_search_results(Q, &cat, &[], false),
            27,
            "shows are counted only where the screen shows them"
        );
    }

    #[test]
    fn a_duplicate_row_is_not_a_second_result() {
        let tracks = lots(25, |i| dz(i, None, "Patrick Sébastien", false));
        // The same 25 recordings found again by the fallback, and the same
        // artist repeated under its UUID: 25 results + 1 artist, not 50 or 26+.
        let videos: Vec<Track> = tracks
            .iter()
            .enumerate()
            .map(|(i, track)| {
                let mut hit = yt(i as u32, Some(track.title.as_str()), "Patrick Sébastien");
                hit.deezer_id = track.deezer_id.clone();
                hit
            })
            .collect();
        let mut repeated = entity(1);
        repeated.id = Some("uuid-e-1".to_owned());
        let cat = Catalogue {
            tracks,
            artists: vec![entity(1), repeated],
            ..Catalogue::default()
        };
        assert_eq!(count_search_results(Q, &cat, &videos, true), 26);
    }

    #[test]
    fn a_fallback_answer_is_counted_only_when_it_is_passed_in() {
        let cat = catalogue(lots(20, |i| dz(i, None, "Patrick Sébastien", false)));
        let videos = lots(10, |i| {
            yt(i, Some(format!("Autre {i}").as_str()), "Patrick Sébastien")
        });
        assert_eq!(count_search_results(Q, &cat, &[], true), 20);
        assert_eq!(count_search_results(Q, &cat, &videos, true), 30);
    }

    #[test]
    fn only_titles_above_their_sources_bar_open_the_radio() {
        let cat = catalogue(vec![
            dz(1, Some("Autre"), "Autre", false),
            dz(2, None, "Patrick Sébastien", false),
        ]);
        let videos = vec![
            // Another artist covering the title: under YOUTUBE_MIN.
            yt(3, Some("Ivanov"), "The Cover Band"),
            // A real match on the fallback, kept.
            yt(4, Some("Patrick Sébastien en tournée"), "Patrick Sébastien"),
        ];
        assert_eq!(ids(&relevant_tracks(Q, &cat, &videos)), ["dz-2", "yt-4"]);
    }

    #[test]
    fn a_title_found_twice_is_one_row_in_the_queue() {
        let mut twice = dz(1, None, "Patrick Sébastien", false);
        twice.id = Some(TrackRowId::new("11111111-1111-4111-8111-111111111111"));
        let cat = catalogue(vec![dz(1, None, "Patrick Sébastien", false), twice]);
        assert_eq!(relevant_tracks(Q, &cat, &[]).len(), 1);
        // And the same recording through the fallback is dropped by the ranking.
        let videos = vec![yt(9, Some("Titre 1"), "Patrick Sébastien")];
        assert_eq!(
            relevant_tracks(
                Q,
                &catalogue(vec![dz(1, None, "Patrick Sébastien", false)]),
                &videos
            )
            .len(),
            1
        );
    }

    #[test]
    fn the_strongest_match_leads_and_a_library_row_wins_a_tie() {
        let weak = dz(1, Some("Patrick"), "Autre", false);
        let strong = dz(2, None, "Patrick Sébastien", false);
        let local = dz(3, None, "Patrick Sébastien", true);
        let queue = relevant_tracks(Q, &catalogue(vec![weak, strong, local]), &[]);
        assert_eq!(ids(&queue), ["dz-3", "dz-2", "dz-1"]);
    }

    #[test]
    fn the_best_title_seeds_the_radio_with_its_artist() {
        let tracks = vec![
            dz(1, None, "Patrick Sébastien", false),
            dz(2, None, "Patrick Sébastien", false),
        ];
        let cat = catalogue(tracks.clone());
        let seed = radio_seed(Q, &cat, &tracks).expect("a seed");
        assert_eq!(
            seed,
            Seed::Track {
                id: "dz-1".to_owned(),
                artist: Some("42".to_owned())
            }
        );
        let context = radio_context(Q, &seed);
        assert_eq!(context.kind, "radio");
        assert_eq!(context.id, "track:dz-1", "the player splits this pair");
        assert_eq!(context.artist.as_deref(), Some("42"));
        assert_eq!(context.title, "Radio de patrick sebastien");
    }

    #[test]
    fn with_no_title_to_start_from_the_artist_the_search_names_seeds_it() {
        let cat = Catalogue {
            tracks: Vec::new(),
            artists: vec![entity(1)],
            ..Catalogue::default()
        };
        // The query is not the start of "Entité 1", so the best result names
        // nothing and the catalogue's own first answer seeds it.
        let seed = radio_seed(Q, &cat, &[]).expect("a seed");
        assert_eq!(
            seed,
            Seed::Artist {
                id: "e-1".to_owned()
            }
        );
        assert_eq!(radio_context(Q, &seed).id, "artist:e-1");
    }

    #[test]
    fn a_search_with_neither_a_matching_title_nor_an_artist_has_no_seed() {
        assert_eq!(radio_seed(Q, &Catalogue::default(), &[]), None);
    }

    #[test]
    fn the_top_up_only_adds_what_is_not_already_queued() {
        let existing = vec![
            dz(1, None, "Patrick Sébastien", false),
            dz(2, None, "Patrick Sébastien", false),
        ];
        let add = merge_radio_tracks(
            &existing,
            &[
                dz(1, None, "Patrick Sébastien", false),
                dz(3, None, "Patrick Sébastien", false),
                yt(1, Some("Titre 2"), "Patrick Sébastien"),
            ],
        );
        assert_eq!(ids(&add), ["dz-3"]);
    }
}
