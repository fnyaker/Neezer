//! The model, proven against the REAL server's JSON.
//!
//! `desktop/tools/dump_fixtures.py` starts `tools/perf_api.py`, logs in as
//! `bench` and writes one body per read route of `webapp/src/lib/api.js` into
//! `tests/fixtures/` (the committed copy). This test deserializes each of them
//! with the type named in [`FIXTURES`], then twice more:
//!
//! - with an unknown field added to every object (an older or a newer server
//!   must still parse, rule 7);
//! - with every field the type does not truly need REMOVED — each key is tried
//!   and kept deleted when the payload still parses, which proves the
//!   `#[serde(default)]`/`Option` rule instead of trusting it.
//!
//! The bench's library is random on purpose, so nothing here compares VALUES:
//! the assertions are about shapes, nullability and tolerance.
//!
//! CI regenerates the directory from the server at the commit under test
//! (`NZ_FIXTURES_DIR`) and runs this again: a payload change that would break
//! the desktop fails the desktop job.

use nz_model::*;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Instant;

/// One fixture file and the type that must read it.
struct Fixture {
    name: &'static str,
    parse: fn(&str) -> Result<(), serde_json::Error>,
}

/// `fixtures! { "me" => Me, … }` — one line per file, no duplicated list.
macro_rules! fixtures {
    ($($name:literal => $ty:ty),* $(,)?) => {
        const FIXTURES: &[Fixture] = &[
            $(Fixture {
                name: $name,
                parse: |text| serde_json::from_str::<$ty>(text).map(|_| ()),
            }),*
        ];
    };
}

fixtures! {
    // auth, session, who may use what
    "me" => Me,
    "access" => Access,
    "my-accounts" => MyAccounts,
    // onboarding
    "onboarding" => OnboardingState,
    "onboarding-artists" => OnboardingArtists,
    "onboarding-search" => OnboardingSearch,
    "onboarding-account-options" => OnboardingAccountOptions,
    // library
    "my-playlists" => PlaylistList,
    "my-favorites" => MyFavorites,
    "my-artists" => MyArtists,
    "my-albums" => MyAlbums,
    "my-local" => MyLocal,
    "favorite-ids" => FavoriteIds,
    "my-dislikes" => MyDislikes,
    "unavailable" => Unavailable,
    // playlists
    "playlist" => PlaylistPage,
    "playlist-recommendations" => PlaylistRecommendations,
    "playlist-version" => PlaylistVersion,
    "playlist-members" => PlaylistMembers,
    "playlist-share" => PlaylistShare,
    "shared-playlist" => SharedPlaylist,
    // entities
    "artist" => ArtistPage,
    "artist-tracks" => TrackList,
    "artist-discography" => Discography,
    "album" => AlbumPage,
    "search" => SearchResults,
    "lyrics" => Lyrics,
    "gain" => Gain,
    "audio-edges" => AudioEdges,
    "gains" => Gains,
    "analyses" => Analyses,
    "track-analysis" => TrackAnalysis,
    "track-credits" => TrackCredits,
    "credits-person" => PersonCredits,
    // discovery
    "flow" => TrackList,
    "flow-styles" => FlowStyles,
    "recommend" => Recommend,
    "discover-prefs" => DiscoverPrefs,
    "history" => History,
    "recap-years" => RecapYears,
    "recap" => Recap,
    "moods" => Moods,
    "mood" => Mood,
    "releases" => Releases,
    "releases-settings" => ReleasesSettings,
    "radio-track" => TrackList,
    "radio-artist" => TrackList,
    "radio-genre" => TrackList,
    // podcasts and transcripts
    "podcasts" => PodcastList,
    "podcast" => PodcastPage,
    "search-podcasts" => PodcastSearch,
    "podcast-progress" => PodcastProgress,
    "show-markers" => ShowMarkers,
    "episode-markers" => EpisodeMarkers,
    "episode-chapters" => EpisodeChapters,
    "episode-transcript" => EpisodeTranscript,
    "show-transcript-search" => TranscriptSearch,
    "transcripts-search" => TranscriptSearch,
    "transcribe-status" => TranscribeStatus,
    "transcribe-settings" => TranscribeSettings,
    // sources, settings, server status
    "sources" => Sources,
    "settings" => ServerSettings,
    "deezer-status" => DeezerStatus,
    "storage" => Storage,
    "archive-status" => ArchiveStatus,
    "archive-rules" => ArchiveRules,
    "archive-cleanup-preview" => CleanupPreview,
    "archive-migrate" => MigrateStatus,
    "sync-status" => SyncStatus,
    "upload-usage" => UploadUsage,
    "version" => Version,
    "services" => Services,
    "export-formats" => ExportFormats,
    "automatch" => Automatch,
    "probe-track" => TrackProbe,
    "replace-candidates" => ReplaceCandidates,
    "stream-ready" => StreamReady,
    // administration
    "admin-users" => AdminUsers,
    "admin-invites" => AdminInvites,
    "genre-status" => GenreStatus,
    "genre-tree" => GenreTree,
    "genre-candidates" => GenreCandidates,
    "genre-labelled" => GenreLabelled,
    "genre-pool" => GenrePool,
    "genre-track" => GenreTrack,
    "genre-model" => GenreModel,
    "genre-relabel" => GenreRelabel,
    "genre-embed" => GenreEmbed,
    "genre-construction" => GenreConstruction,
    "genre-references" => GenreReferences,
    "genre-references-for" => GenreReferencesFor,
    "genre-extractor-test" => ExtractorTest,
    "genre-share-status" => GenreShareStatus,
    "similar-rebuild" => SimilarStatus,
    "enrich-backfill" => EnrichStatus,
    "credits-backfill" => CreditsBackfill,
    "analysis-plan" => AnalysisPlan,
    "analysis-todo" => AnalysisTodo,
    "analysis-backfill" => AnalysisBackfill,
    "explorer" => Explorer,
    // remote control and listen party
    "remote-links" => RemoteLinks,
    "party-mine" => PartyMine,
}

/// Where the fixtures are: the committed copy, or what CI just regenerated.
fn fixture_dir() -> PathBuf {
    match std::env::var_os("NZ_FIXTURES_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
    }
}

fn read(name: &str) -> String {
    let path = fixture_dir().join(format!("{name}.json"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

fn parse(fixture: &Fixture, value: &Value) -> Result<(), serde_json::Error> {
    let text = serde_json::to_string(value).expect("a JSON document serialises back");
    (fixture.parse)(&text)
}

/// How many fixture files the directory holds.
fn fixture_files() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(fixture_dir())
        .unwrap_or_else(|error| panic!("cannot list {}: {error}", fixture_dir().display()))
        .map(|entry| entry.expect("a directory entry").file_name())
        .filter_map(|name| name.to_str().map(str::to_owned))
        .filter(|name| name.ends_with(".json"))
        .map(|name| name.trim_end_matches(".json").to_owned())
        .collect();
    names.sort();
    names
}

/// Adds one unknown key to the root object and to every object inside an array.
///
/// This is what an older desktop meets on a newer server: a key it has never
/// heard of. `serde` ignores unknown fields by default, and this proves the model
/// never adds `deny_unknown_fields` (which would break every upgrade).
///
/// Objects reached through an object KEY are deliberately skipped: those are the
/// API's maps (`{"<track id>": verdict}`, `{"<genre>": count}`), where an extra
/// key is data of the map's own value type, not a struct field to ignore.
fn inject_unknown(value: &mut Value, struct_like: bool) {
    match value {
        Value::Object(map) if struct_like => {
            map.insert(
                "nz_unknown_probe".to_owned(),
                json!({"why": "an older model must ignore this", "n": [1, 2, 3]}),
            );
            for child in map.values_mut() {
                inject_unknown(child, false);
            }
        }
        Value::Object(map) => {
            for child in map.values_mut() {
                inject_unknown(child, false);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                inject_unknown(item, true);
            }
        }
        _ => {}
    }
}

/// One place inside a document: an object key, or an index in an array.
#[derive(Clone, Debug)]
enum Step {
    Key(String),
    Index(usize),
}

/// Every key of every object in the document, as a path from the root.
fn key_paths(value: &Value, base: &mut Vec<Step>, out: &mut Vec<Vec<Step>>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                base.push(Step::Key(key.clone()));
                out.push(base.clone());
                key_paths(child, base, out);
                base.pop();
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                base.push(Step::Index(index));
                key_paths(child, base, out);
                base.pop();
            }
        }
        _ => {}
    }
}

/// Removes the value at `path`, if it is still there.
fn remove_at(value: &mut Value, path: &[Step]) -> bool {
    let (last, rest) = match path.split_last() {
        Some(parts) => parts,
        None => return false,
    };
    let mut node = value;
    for step in rest {
        node = match (node, step) {
            (Value::Object(map), Step::Key(key)) => match map.get_mut(key) {
                Some(child) => child,
                None => return false,
            },
            (Value::Array(items), Step::Index(index)) => match items.get_mut(*index) {
                Some(child) => child,
                None => return false,
            },
            _ => return false,
        };
    }
    match (node, last) {
        (Value::Object(map), Step::Key(key)) => map.remove(key).is_some(),
        (Value::Array(items), Step::Index(index)) if *index < items.len() => {
            // A removed array slot would shift the paths recorded after it, so
            // arrays are kept whole: only keys are stripped.
            let _ = index;
            false
        }
        _ => false,
    }
}

/// Shrinks the arrays to their first element (a 200-track page and a 1-track
/// page prove the same types, and stripping a small document is fast).
fn shrink(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for child in map.values_mut() {
                shrink(child);
            }
        }
        Value::Array(items) => {
            items.truncate(1);
            for item in items.iter_mut() {
                shrink(item);
            }
        }
        _ => {}
    }
}

#[test]
fn the_folder_holds_one_file_per_listed_fixture_and_none_else() {
    let listed: Vec<&str> = FIXTURES.iter().map(|fixture| fixture.name).collect();
    let files = fixture_files();
    // The lot's floor: at least 40 real payloads (one per read route of api.js).
    assert!(
        FIXTURES.len() >= 40,
        "the table lists {} fixtures, the lot asks for at least 40",
        FIXTURES.len()
    );
    for name in &listed {
        let path = fixture_dir().join(format!("{name}.json"));
        let size = std::fs::metadata(&path)
            .unwrap_or_else(|error| panic!("missing fixture {}: {error}", path.display()))
            .len();
        assert!(size > 0, "fixture {} is empty", path.display());
    }
    let mut missing: Vec<&str> = files
        .iter()
        .map(String::as_str)
        .filter(|name| !listed.contains(name))
        .collect();
    missing.sort_unstable();
    assert!(
        missing.is_empty(),
        "{} fixture file(s) have no type in the table: {missing:?}",
        missing.len()
    );
    let unread: Vec<&str> = listed
        .iter()
        .copied()
        .filter(|name| !files.iter().any(|file| file == name))
        .collect();
    assert!(unread.is_empty(), "listed but absent: {unread:?}");
    println!(
        "{} fixtures, {} files in {}",
        FIXTURES.len(),
        files.len(),
        fixture_dir().display()
    );
}

#[test]
fn every_fixture_deserializes() {
    let started = Instant::now();
    let mut bytes = 0usize;
    for fixture in FIXTURES {
        let text = read(fixture.name);
        bytes += text.len();
        if let Err(error) = (fixture.parse)(&text) {
            panic!("{} does not parse: {error}", fixture.name);
        }
    }
    let elapsed = started.elapsed();
    println!(
        "{} fixtures ({bytes} bytes) parse in {:.1} ms",
        FIXTURES.len(),
        elapsed.as_secs_f64() * 1000.0
    );
    // A generous bound: a debug build on a cold cache is the slow case, and the
    // lot's `cargo test -p nz-model` budget is 2 s for the whole suite.
    assert!(
        elapsed.as_millis() < 500,
        "parsing every fixture took {elapsed:?}, the suite's budget is 2 s"
    );
}

#[test]
fn every_fixture_ignores_unknown_fields() {
    for fixture in FIXTURES {
        let mut value: Value = serde_json::from_str(&read(fixture.name))
            .unwrap_or_else(|error| panic!("{} is not JSON: {error}", fixture.name));
        inject_unknown(&mut value, true);
        if let Err(error) = parse(fixture, &value) {
            panic!(
                "{} rejects an unknown field, which breaks every server upgrade: {error}",
                fixture.name
            );
        }
    }
}

#[test]
fn every_fixture_parses_with_every_optional_field_removed() {
    let started = Instant::now();
    let mut stripped = 0usize;
    let mut tried = 0usize;
    for fixture in FIXTURES {
        let original: Value = serde_json::from_str(&read(fixture.name))
            .unwrap_or_else(|error| panic!("{} is not JSON: {error}", fixture.name));
        let mut value = original.clone();
        shrink(&mut value);
        // The shrunken document must still parse before anything is removed.
        parse(fixture, &value)
            .unwrap_or_else(|error| panic!("{} breaks when shrunk: {error}", fixture.name));

        let mut paths = Vec::new();
        key_paths(&value, &mut Vec::new(), &mut paths);
        for path in paths {
            let mut candidate = value.clone();
            if !remove_at(&mut candidate, &path) {
                continue;
            }
            tried += 1;
            // Kept deleted only when the payload still parses: what survives is
            // exactly what the model truly needs.
            if parse(fixture, &candidate).is_ok() {
                value = candidate;
                stripped += 1;
            }
        }
        if let Err(error) = parse(fixture, &value) {
            panic!(
                "{} stops parsing once its optional fields are gone: {error}",
                fixture.name
            );
        }
    }
    let elapsed = started.elapsed();
    println!(
        "stripped {stripped} of {tried} keys across {} fixtures in {:.1} ms",
        FIXTURES.len(),
        elapsed.as_secs_f64() * 1000.0
    );
    // A fixture whose every key is required would prove nothing about rule 7.
    assert!(
        stripped > tried / 2,
        "only {stripped} of {tried} keys were optional: the model is too strict"
    );
}
