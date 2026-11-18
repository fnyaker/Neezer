//! The search's wire: the three reads (`/api/search`, `/api/youtube/search`,
//! `/api/transcripts/search`), the rows the screen draws, the one file the
//! device keeps between two visits, and the account's recent searches.
//!
//! Every call here goes through `nz-api`'s TYPED endpoints, which run on
//! [`nz_api::Lane::User`] — the action just taken, never behind a prefetch or a
//! download (rule 1). Nothing in this file decides a rule of its own: the shapes
//! are `nz-model`'s (the same records the desktop decodes), the order and the
//! weaving are [`super::rank`]'s, and the counting is `nz-api`'s per-tag
//! counters (SC3) read around one screen open.

use std::path::PathBuf;

use nz_api::Api;
use nz_model::{
    Album, Artist, PodcastCard, SearchPlaylist, Track, TranscriptHit, TranscriptSearch,
};
use serde::{Deserialize, Serialize};

use super::{Core, CoreError, Row, Scope, TAG_SEARCH, TranscriptLine};

/// `Search.svelte`'s transcript page: 20 passages ("20 passages affichés").
pub const TRANSCRIPT_LIMIT: u32 = 20;

/// What one request cost, as `nz-api` counts it (SC3): requests really sent,
/// and the response bodies' bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Totals {
    pub requests: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

/// The tag's running totals on this client.
pub(super) fn totals(api: &Api) -> Totals {
    api.stats()
        .into_iter()
        .find(|row| row.tag == TAG_SEARCH)
        .map(|row| Totals {
            requests: row.requests,
            bytes_in: row.bytes_in,
            bytes_out: row.bytes_out,
        })
        .unwrap_or_default()
}

/// One search answer, as the server sent it: the five families of
/// `/api/search`, in the provider's own order. Library rows come FIRST
/// (`neezer/webui/__init__.py#search` puts them there), which is what
/// [`super::rank::rank_search_results`] keeps.
///
/// This is what the device stores: the rows the screen draws are built from it
/// (and the radio's seed needs the raw tracks' own ids, which a row flattens).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalogue {
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub artists: Vec<Artist>,
    #[serde(default)]
    pub albums: Vec<Album>,
    #[serde(default)]
    pub playlists: Vec<SearchPlaylist>,
    #[serde(default)]
    pub podcasts: Vec<PodcastCard>,
}

/// A transcript search's answer, flattened to what the screen prints: the
/// passages, the total, whether more exist, and how much of the library was
/// scanned (`TranscriptScope`'s three lines of copy).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscriptAnswer {
    pub lines: Vec<TranscriptLine>,
    pub total: u32,
    pub more: bool,
    pub episodes: u32,
    pub segments: Option<u32>,
    pub truncated: bool,
}

/// `GET /api/search?q=` — the five families in one answer.
///
/// The server's own `limit` default is 25, which is exactly the SPA's number
/// (`searchrank.js`'s fixtures and `Search.svelte` ask for 25 of each family), so
/// the phone sends none: the two screens spend the same bytes on the same query.
pub(super) async fn fetch_search(api: &Api, term: &str) -> Result<Catalogue, CoreError> {
    let answer = api.search(term).await?;
    Ok(Catalogue {
        tracks: answer.tracks,
        artists: answer.artists,
        albums: answer.albums,
        playlists: answer.playlists,
        podcasts: answer.podcasts,
    })
}

/// `GET /api/youtube/search?q=` — the fallback's hits, shaped as ordinary
/// tracks. A refusal is an error (`X-NS-Error`), never an empty list, so the
/// screen can say WHY it has nothing to add.
pub(super) async fn fetch_youtube(api: &Api, term: &str) -> Result<Vec<Track>, CoreError> {
    Ok(api.youtube_search(term).await?.tracks)
}

/// `GET /api/search/podcasts?q=` — the shows only (the admin's "Émissions").
pub(super) async fn fetch_shows(api: &Api, term: &str) -> Result<Vec<PodcastCard>, CoreError> {
    Ok(api.search_podcasts(term).await?.podcasts)
}

/// `GET /api/transcripts/search?q=&limit=` — the passages of every transcribed
/// episode.
///
/// A `404` is the server being older than the feature (rule 7): it crosses as
/// [`CoreError::Unsupported`], so the screen hides the transcript list and says
/// so instead of failing every time.
pub(super) async fn fetch_transcripts(
    api: &Api,
    term: &str,
    query: &str,
) -> Result<TranscriptAnswer, CoreError> {
    let answer: TranscriptSearch = match api.search_transcripts(term, TRANSCRIPT_LIMIT).await {
        Ok(answer) => answer,
        Err(nz_api::ApiError::Http { status: 404, .. }) => {
            return Err(CoreError::Unsupported {
                feature: "transcription".to_owned(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    Ok(transcript_answer(answer, query))
}

fn transcript_answer(answer: TranscriptSearch, query: &str) -> TranscriptAnswer {
    let lines = answer
        .hits
        .iter()
        .enumerate()
        .map(|(index, hit)| transcript_line(hit, index, query))
        .collect();
    TranscriptAnswer {
        lines,
        total: answer.total,
        more: answer.more,
        episodes: answer.scope.episodes,
        segments: answer.scope.segments,
        truncated: answer.scope.truncated,
    }
}

/// One passage, as the phone draws it: the moment, the episode, and the line
/// with the words that were typed marked (`TranscriptHits.svelte`'s `<mark>`).
fn transcript_line(hit: &TranscriptHit, index: usize, query: &str) -> TranscriptLine {
    let episode = hit.episode.as_ref();
    let title = episode
        .map(|episode| episode.title.clone())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Épisode".to_owned());
    let episode_id = hit
        .episode_id
        .as_ref()
        .map(ToString::to_string)
        .or_else(|| episode.map(|episode| episode.id.to_string()))
        .unwrap_or_default();
    TranscriptLine {
        key: format!("{episode_id}:{index}"),
        episode_id,
        title,
        channel: hit.channel_title.clone().unwrap_or_default(),
        start_s: hit.start.max(0.0) as u32,
        text: highlight_parts(&hit.text, query),
        cover: episode.and_then(|episode| episode.cover.clone()),
    }
}

/// One piece of a transcript line, marked or not (`transcript.js#highlightParts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Highlight {
    pub text: String,
    pub mark: bool,
}

/// Splits `text` into the pieces around the words that were typed.
///
/// The SPA's rule, ported: the WHOLE query first (a person who typed a phrase
/// sees the phrase), and only when it occurs nowhere, each word on its own.
/// Case is folded, accents are not — exactly what the SPA's `toLowerCase` does.
pub fn highlight_parts(text: &str, query: &str) -> Vec<Highlight> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let mut ranges = find_all(&chars, query.trim());
    if ranges.is_empty() {
        for word in query.split_whitespace() {
            ranges.extend(find_all(&chars, word));
        }
    }
    if ranges.is_empty() {
        return vec![Highlight {
            text: text.to_owned(),
            mark: false,
        }];
    }
    ranges.sort_by_key(|(start, end)| (*start, std::cmp::Reverse(*end)));
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start < last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    let mut out = Vec::with_capacity(merged.len() * 2 + 1);
    let mut at = 0;
    for (start, end) in merged {
        if start > at {
            out.push(Highlight {
                text: chars[at..start].iter().collect(),
                mark: false,
            });
        }
        out.push(Highlight {
            text: chars[start..end].iter().collect(),
            mark: true,
        });
        at = end;
    }
    if at < chars.len() {
        out.push(Highlight {
            text: chars[at..].iter().collect(),
            mark: false,
        });
    }
    out
}

/// Every occurrence of `needle` in `hay`, case folded, as char ranges.
fn find_all(hay: &[char], needle: &str) -> Vec<(usize, usize)> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() || needle.len() > hay.len() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut at = 0;
    while at + needle.len() <= hay.len() {
        let matches = hay[at..at + needle.len()]
            .iter()
            .zip(needle.iter())
            .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()));
        if matches {
            found.push((at, at + needle.len()));
            at += needle.len();
        } else {
            at += 1;
        }
    }
    found
}

/// Who performed the row, joined for a single line (`format.js#artistLine`).
pub(super) fn artist_line(track: &Track) -> String {
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

/// A catalogue or fallback title, as the screen draws and queues it.
///
/// The key and the id are the row's UUID when the library holds the track, else
/// the provider's id — the same value `nz-player`'s `Entry::key()` holds, so a
/// tap from the search and the same row in a playlist name one title.
pub(super) fn row(track: &Track, position: u32) -> Row {
    let row_id = track.id.as_ref().map(ToString::to_string);
    let deezer = track.deezer_id.to_string();
    let source = match track.source.as_deref() {
        Some("youtube") => "youtube",
        _ if track.local => "upload",
        _ => "deezer",
    };
    Row {
        key: row_id.clone().unwrap_or_else(|| deezer.clone()),
        id: row_id.unwrap_or(deezer),
        title: track.title.clone(),
        artists: artist_line(track),
        album: track.album.title.clone(),
        album_id: (!track.album.deezer_id.to_string().is_empty())
            .then(|| track.album.deezer_id.to_string()),
        duration_s: track.duration,
        cover: track.album.cover.clone(),
        explicit: track.explicit,
        liked: false,
        disliked: false,
        unavailable: track.unavailable,
        downloaded: track.is_archived(),
        source: source.to_owned(),
        position,
        played_at: 0,
        added_at: 0,
    }
}

/// An artist, an album, a playlist or a show, as one card draws it
/// (`Card.svelte`, `PodcastCard.svelte`): the title, the one line under it, the
/// cover, and the id its route is opened by.
pub(super) fn entity_row(
    id: &str,
    deezer_id: &str,
    title: &str,
    subtitle: &str,
    cover: Option<String>,
    kind: super::EntityKind,
) -> super::EntityRow {
    super::EntityRow {
        id: id.to_owned(),
        deezer_id: deezer_id.to_owned(),
        title: title.to_owned(),
        subtitle: subtitle.to_owned(),
        cover,
        kind,
    }
}

// --- the one file the device keeps -----------------------------------------

/// What the device remembers between two visits for ONE term: the provider's
/// own answer, so a warm paint costs nothing, and so the radio's seed and the
/// fallback's weaving work without asking the catalogue again.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Stored {
    pub term: String,
    pub scope: Scope,
    pub catalogue: Catalogue,
    /// The fallback's hits, once that source answered (empty before).
    #[serde(default)]
    pub youtube: Vec<Track>,
    /// Whether the fallback answered AT ALL for this term — an empty answer and
    /// no answer are not the same thing, and the screen says which.
    #[serde(default)]
    pub youtube_answered: bool,
    #[serde(default)]
    pub transcript: Option<TranscriptAnswer>,
    /// The shows-only answer (scope [`Scope::Shows`]).
    #[serde(default)]
    pub shows: Vec<PodcastCard>,
    /// Why the fallback's last answer was a failure, if it was.
    #[serde(default)]
    pub youtube_failure: Option<super::YoutubeFailure>,
}

pub(super) fn read_snapshot(core: &Core, term: &str, scope: Scope) -> Option<Stored> {
    let body = std::fs::read_to_string(snapshot_path(core, term, scope)).ok()?;
    let stored: Stored = serde_json::from_str(&body).ok()?;
    // A file written by another term (a hash collision, a hand-edited file) is
    // not this answer: the screen would draw another query's results.
    (stored.term == term.trim() && stored.scope == scope).then_some(stored)
}

pub(super) fn write_snapshot(core: &Core, stored: &Stored) -> Result<(), CoreError> {
    let path = snapshot_path(core, &stored.term, stored.scope);
    let body = serde_json::to_vec(stored).map_err(|error| CoreError::Internal {
        detail: format!("{} : {error}", path.display()),
    })?;
    let temporary = path.with_extension("part");
    std::fs::write(&temporary, &body).map_err(|error| CoreError::Internal {
        detail: format!("{} : {error}", temporary.display()),
    })?;
    std::fs::rename(&temporary, &path).map_err(|error| CoreError::Internal {
        detail: format!("{} : {error}", path.display()),
    })
}

/// `search-<account hash>-<scope>-<term hash>.json`, in the CACHE directory
/// (evictable, like every cache): two accounts never read each other's answers,
/// and the name is a hash of what the person typed, never a server string.
fn snapshot_path(core: &Core, term: &str, scope: Scope) -> PathBuf {
    core.cache_path(&format!(
        "search-{:016x}-{}-{:016x}.json",
        super::fnv1a(super::owner(core).as_bytes()),
        scope.slug(),
        super::fnv1a(term.trim().as_bytes()),
    ))
}

/// How many term files the device keeps: the five recent searches, and the one
/// being read. A sixth term evicts the oldest, so the cache never grows with
/// what somebody typed once.
pub(super) fn prune_snapshots(core: &Core) -> usize {
    let dir = core.config().cache_dir.clone();
    let prefix = format!(
        "search-{:016x}-",
        super::fnv1a(super::owner(core).as_bytes())
    );
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let mut kept: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(prefix.as_str())
                && entry.path().extension().is_some_and(|ext| ext == "json")
        })
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            Some((meta.modified().ok()?, entry.path()))
        })
        .collect();
    if kept.len() <= super::RECENT_MAX {
        return 0;
    }
    kept.sort_by_key(|(at, _)| *at);
    let mut removed = 0;
    for (_, path) in kept.iter().take(kept.len() - super::RECENT_MAX) {
        if std::fs::remove_file(path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Books one screen-open's traffic in the core's meter, so `NzCore::stats()`
/// answers what this screen cost without the UI counting anything itself.
pub(super) fn note_traffic(core: &Core, api: &Api, before: Totals) {
    let after = totals(api);
    let requests = after.requests.saturating_sub(before.requests);
    if requests == 0 {
        return;
    }
    // `Meter::record` counts ONE request and its bytes; the bytes of the whole
    // run go with the first, the rest are zeros.
    core.meter().record(
        TAG_SEARCH,
        after.bytes_in.saturating_sub(before.bytes_in),
        after.bytes_out.saturating_sub(before.bytes_out),
    );
    for _ in 1..requests {
        core.meter().record(TAG_SEARCH, 0, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transcript_line_marks_what_was_typed() {
        let parts = highlight_parts("la memoire de la ville", "memoire");
        let marked: Vec<&str> = parts
            .iter()
            .filter(|part| part.mark)
            .map(|part| part.text.as_str())
            .collect();
        assert_eq!(marked, ["memoire"]);
        assert_eq!(
            parts.iter().map(|p| p.text.as_str()).collect::<String>(),
            "la memoire de la ville",
            "the pieces rebuild the line exactly"
        );
    }

    #[test]
    fn a_phrase_is_marked_whole_and_one_word_at_a_time_only_without_it() {
        let whole = highlight_parts("la ville et la vie", "la ville");
        assert_eq!(
            whole
                .iter()
                .filter(|part| part.mark)
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>(),
            ["la ville"]
        );
        let words = highlight_parts("la ville et la vie", "ville vie");
        assert_eq!(
            words
                .iter()
                .filter(|part| part.mark)
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>(),
            ["ville", "vie"]
        );
        // Nothing typed: one unmarked piece.
        let none = highlight_parts("la ville", "  ");
        assert_eq!(none.len(), 1);
        assert!(!none[0].mark);
    }

    #[test]
    fn a_row_is_the_queue_s_own_row() {
        let mut track = Track {
            deezer_id: nz_model::TrackId::new("3000168"),
            title: "Ouverture".to_owned(),
            duration: 180,
            artist: nz_model::TrackArtist {
                deezer_id: nz_model::ArtistId::new("42"),
                name: "Ascenseur".to_owned(),
            },
            album: nz_model::TrackAlbum {
                deezer_id: nz_model::AlbumId::new("501"),
                title: "Le matin".to_owned(),
                cover: Some("/api/cover/501".to_owned()),
            },
            ..Track::default()
        };
        let first = row(&track, 3);
        assert_eq!(first.key, "3000168");
        assert_eq!(first.artists, "Ascenseur");
        assert_eq!(first.album, "Le matin");
        assert_eq!(first.album_id.as_deref(), Some("501"));
        assert_eq!(first.source, "deezer");
        assert_eq!(first.position, 3);

        track.id = Some(nz_model::TrackRowId::new(
            "11111111-1111-4111-8111-111111111111",
        ));
        let second = row(&track, 0);
        assert_eq!(second.key, "11111111-1111-4111-8111-111111111111");
        assert_eq!(second.id, second.key, "and that is the id /stream accepts");

        track.source = Some("youtube".to_owned());
        assert_eq!(row(&track, 0).source, "youtube");
        track.source = None;
        track.local = true;
        assert_eq!(row(&track, 0).source, "upload");
    }

    #[test]
    fn a_transcript_answer_flattens_its_scope() {
        let answer: TranscriptSearch = serde_json::from_str(
            r#"{"q":"memoire","hits":[{"idx":0,"start":600.5,"end":615,"text":"la memoire","score":1,
                 "episode_id":"5ef08406-6ef3-4adf-835b-c9e9927ba70e",
                 "episode":{"id":"5ef08406-6ef3-4adf-835b-c9e9927ba70e","title":"Épisode 1",
                            "cover":"/api/cover/x"},
                 "channel_title":"Les Histoires"}],
                "total":1,"more":true,
                "scope":{"episodes":4,"segments":240,"truncated":true}}"#,
        )
        .expect("a transcript answer");
        let flat = transcript_answer(answer, "memoire");
        assert_eq!(flat.total, 1);
        assert!(flat.more);
        assert_eq!(flat.episodes, 4);
        assert_eq!(flat.segments, Some(240));
        assert!(flat.truncated);
        assert_eq!(flat.lines.len(), 1);
        assert_eq!(flat.lines[0].title, "Épisode 1");
        assert_eq!(flat.lines[0].channel, "Les Histoires");
        assert_eq!(flat.lines[0].start_s, 600);
        assert!(flat.lines[0].text.iter().any(|part| part.mark));
    }
}

#[cfg(test)]
mod round_trip_tests {
    use super::*;

    /// The kept answer must survive a restart: the device reads it back exactly
    /// as it wrote it (a shape that cannot round-trip would make every warm visit
    /// a cold one, silently).
    #[test]
    fn a_kept_answer_round_trips_through_json() {
        let catalogue: Catalogue = serde_json::from_str(
            r#"{"tracks":[{"id":null,"deezer_id":"1001","title":"Get Lucky","duration":369,
                 "local":true,"source":null,"youtube_id":null,"bitrate":null,"suffix":null,
                 "quality":null,"gain":null,"explicit":false,"unavailable":false,
                 "artist":{"deezer_id":"27","name":"Daft Punk"},
                 "artists":[{"deezer_id":"27","name":"Daft Punk","role":"Main"}],
                 "display_artist":"Daft Punk",
                 "album":{"deezer_id":"501","title":"RAM","cover":"/api/cover/501"}}],
                "artists":[{"id":"aaaaaaaa-1111-4111-8111-111111111111","deezer_id":"27",
                            "name":"Daft Punk","picture":"/api/cover/a"}],
                "albums":[{"id":"bbbbbbbb-1111-4111-8111-111111111111","deezer_id":"501",
                           "title":"RAM","cover":"/api/cover/501","year":null,
                           "release_date":"2013-05-17","nb_tracks":13,
                           "artist":{"deezer_id":"27","name":"Daft Punk"}}],
                "playlists":[{"deezer_id":"7001","title":"French Touch","owner":"Nadia"}],
                "podcasts":[]}"#,
        )
        .expect("a catalogue");
        let stored = Stored {
            term: "daft punk".to_owned(),
            scope: Scope::Library,
            catalogue,
            youtube: Vec::new(),
            youtube_answered: true,
            transcript: None,
            shows: Vec::new(),
            youtube_failure: Some(super::super::YoutubeFailure::Unavailable),
        };
        let body = serde_json::to_string(&stored).expect("stored as JSON");
        let back: Stored = serde_json::from_str(&body).expect("read back");
        assert_eq!(back, stored);
    }
}
