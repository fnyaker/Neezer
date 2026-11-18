//! The wire: the seven reads the home opens with, their counted traffic, and the
//! two files the device keeps between two visits.
//!
//! Every call here goes through `nz-api`'s TYPED endpoints, which run on
//! [`nz_api::Lane::User`] — the action just taken, never behind a prefetch or a
//! download (rule 1). Nothing in this file decides a rule: the shapes are
//! `nz-model`'s (the same records the desktop decodes), the ordering and the
//! filters are the SPA's, and the counting is `nz-api`'s own per-tag counters
//! (SC3) read around one screen open.

use std::path::PathBuf;

use nz_api::Api;
use nz_model::{
    DiscoverPrefs, FlowStyle, FlowStyles, Mood, Moods, MyArtists, RecapYearRow, RecapYears,
    Recommend, Releases, Track, TrackList,
};

use super::{Core, CoreError, TAG_HOME, TrackCard};

/// `flow.js#body`'s `exclude`: the queue's own tail, at most 400 ids.
pub(super) const EXCLUDE_MAX: usize = 400;

/// What one tag spent, as `nz-api` counts it (SC3): requests really sent, and
/// the response bodies' bytes.
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
        .find(|row| row.tag == TAG_HOME)
        .map(|row| Totals {
            requests: row.requests,
            bytes_in: row.bytes_in,
            bytes_out: row.bytes_out,
        })
        .unwrap_or_default()
}

/// `GET /api/releases` — the new titles, their count and the unseen badge.
pub(super) async fn fetch_releases(api: &Api) -> Result<Releases, CoreError> {
    Ok(api.releases().await?)
}

/// `GET /api/me/artists` — the followed artists, in the server's own order.
pub(super) async fn fetch_artists(api: &Api) -> Result<MyArtists, CoreError> {
    Ok(api.my_artists().await?)
}

/// `GET /api/moods` — the catalog.
pub(super) async fn fetch_moods(api: &Api) -> Result<Moods, CoreError> {
    Ok(api.moods().await?)
}

/// `GET /api/moods/<id>` — one mix's titles.
pub(super) async fn fetch_mood(api: &Api, id: &str) -> Result<Mood, CoreError> {
    Ok(api.mood(id).await?)
}

/// `GET /api/discover/prefs` — the account's level and starting notch.
pub(super) async fn fetch_prefs(api: &Api) -> Result<DiscoverPrefs, CoreError> {
    Ok(api.discover_prefs().await?)
}

/// `GET /api/recommend?d=<level>` — "Pour toi".
pub(super) async fn fetch_recommend(
    api: &Api,
    level: f64,
    seed: Option<&str>,
) -> Result<Recommend, CoreError> {
    Ok(api.recommend(level, seed).await?)
}

/// `GET /api/recap/years` — the years that hold at least one listen.
pub(super) async fn fetch_recap_years(api: &Api) -> Result<RecapYears, CoreError> {
    Ok(api.recap_years().await?)
}

/// `GET /api/flow/styles` — the genres a Flow can be set to.
pub(super) async fn fetch_styles(api: &Api) -> Result<FlowStyles, CoreError> {
    Ok(api.flow_styles().await?)
}

/// `POST /api/flow/next` — the bodies are `flow.js#body`'s own
/// (`discover.py#flow_next` reads exactly these keys).
pub(super) async fn flow_pick(
    api: &Api,
    body: &serde_json::Value,
) -> Result<Vec<TrackCard>, CoreError> {
    let answer: TrackList = api.flow_pick(body).await?;
    Ok(answer.tracks.iter().map(track_card).collect())
}

/// `PUT /api/discover/prefs` with the notch only.
pub(super) async fn put_energy(api: &Api, energy: Option<f64>) -> Result<(), CoreError> {
    api.set_flow_energy(energy).await?;
    Ok(())
}

/// `PUT /api/discover/prefs` with the level only.
pub(super) async fn put_discovery_level(api: &Api, level: f64) -> Result<(), CoreError> {
    api.set_discover_level(level).await?;
    Ok(())
}

/// The account's copy of the two Flow settings, as the home adopts them
/// (`discovery.js#loadDiscovery`, `flow.js#adoptPrefs`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct AdoptedPrefs {
    pub level: f64,
    /// `None` is a real answer ("Libre"), not "unknown".
    pub energy: Option<f64>,
}

/// One genre of `/flow/styles`, as the picker draws it (a family and its
/// sub-genres).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FlowStyleRow {
    pub name: String,
    pub parent: Option<String>,
    /// Titles of this node alone.
    pub tracks: u32,
    /// Titles under it, this node included — what a family chip is worth.
    pub branch: u32,
}

/// The rows `StylePicker.svelte#styleFamilies` builds its chips from.
pub(super) fn style_rows(answer: &FlowStyles) -> Vec<FlowStyleRow> {
    answer
        .styles
        .iter()
        .map(|style: &FlowStyle| FlowStyleRow {
            name: style.name.clone(),
            parent: style.parent.clone(),
            tracks: style.tracks,
            branch: style.branch,
        })
        .collect()
}

/// A catalogue title, as the home draws and queues it.
pub(super) fn track_card(track: &Track) -> TrackCard {
    let row_id = track.id.as_ref().map(|id| id.to_string());
    let deezer = track.deezer_id.to_string();
    let source = match track.source.as_deref() {
        Some("youtube") => "youtube",
        _ if track.local => "upload",
        _ => "deezer",
    };
    TrackCard {
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
        unavailable: track.unavailable,
        source: source.to_owned(),
        release: track
            .release
            .as_ref()
            .map(|release| release.deezer_id.to_string()),
    }
}

/// `format.js#artistLine`: who performed it, joined for a single line.
fn artist_line(track: &Track) -> String {
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
    track.artist.name.clone()
}

/// `recap.js#advertisedYear`: the newest year that can tell a story AND is this
/// year or the last one — never a three-year-old year on a home card.
pub(super) fn advertised_year(years: &RecapYears) -> Option<RecapYearRow> {
    let now = years.now_year;
    let mut best: Option<&RecapYearRow> = None;
    for row in &years.years {
        if !row.enough || row.events == 0 {
            continue;
        }
        if best.is_none_or(|kept| row.year > kept.year) {
            best = Some(row);
        }
    }
    let best = best?;
    if now == 0 {
        return None;
    }
    (best.year == now || best.year == now - 1).then(|| best.clone())
}

/// The seed "Autres suggestions" asks with: `Home.svelte#refreshReco` sends the
/// clock, so a second press draws a different set from the same pool.
pub(super) fn seed() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos() % 1_000_000_000)
        .unwrap_or(0);
    nanos.to_string()
}

// --- the two files the device keeps ----------------------------------------

/// The saved home, or `None` the first time this account opens it.
pub(super) fn read_snapshot(core: &Core) -> Option<super::HomeData> {
    let path = snapshot_path(core);
    let body = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&body).ok()
}

/// Writes it whole, through a temporary file and a rename: a process killed
/// mid-write leaves the previous copy, never half a JSON document.
pub(super) fn write_snapshot(core: &Core, data: &super::HomeData) -> Result<(), CoreError> {
    let path = snapshot_path(core);
    write_json(&path, data)
}

fn snapshot_path(core: &Core) -> PathBuf {
    super::snapshot_path(core, &super::owner(core))
}

/// The styles the last visit kept for this account.
pub(super) fn read_styles(core: &Core) -> Option<Vec<FlowStyleRow>> {
    let body = std::fs::read_to_string(styles_path(core)).ok()?;
    serde_json::from_str::<Vec<FlowStyleRow>>(&body).ok()
}

pub(super) fn write_styles(core: &Core, rows: &[FlowStyleRow]) -> Result<(), CoreError> {
    write_json(&styles_path(core), &rows)
}

fn styles_path(core: &Core) -> PathBuf {
    core.cache_path(&format!(
        "flow-styles-{:016x}.json",
        super::fnv1a(super::owner(core).as_bytes())
    ))
}

/// `write` + `rename`, so a reader never sees half a document.
fn write_json<T: serde::Serialize>(path: &PathBuf, value: &T) -> Result<(), CoreError> {
    let inner = |path: &PathBuf| -> Result<(), CoreError> {
        let body = serde_json::to_vec(value).map_err(|error| CoreError::Internal {
            detail: format!("{} : {error}", path.display()),
        })?;
        let temporary = path.with_extension("part");
        std::fs::write(&temporary, &body).map_err(|error| CoreError::Internal {
            detail: format!("{} : {error}", temporary.display()),
        })?;
        std::fs::rename(&temporary, path).map_err(|error| CoreError::Internal {
            detail: format!("{} : {error}", path.display()),
        })?;
        Ok(())
    };
    match inner(path) {
        Ok(()) => Ok(()),
        Err(cause) => {
            // A cache that cannot be written never costs the screen: the answer
            // is already in memory, and the next visit asks again.
            tracing::debug!(%cause, "a home cache file was not written");
            Err(cause)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_model::{RecapYearRow, RecapYears, Track, TrackAlbum, TrackArtist, TrackId};

    fn track(title: &str, artists: &[&str]) -> Track {
        Track {
            deezer_id: TrackId::new("3000168"),
            title: title.to_owned(),
            duration: 200,
            artist: TrackArtist {
                deezer_id: nz_model::ArtistId::new("42"),
                name: artists.first().copied().unwrap_or_default().to_owned(),
            },
            artists: artists
                .iter()
                .map(|name| nz_model::TrackArtistRole {
                    deezer_id: nz_model::ArtistId::new(""),
                    name: (*name).to_owned(),
                    role: "Main".to_owned(),
                })
                .collect(),
            album: TrackAlbum {
                deezer_id: nz_model::AlbumId::new("7"),
                title: "Album".to_owned(),
                cover: Some("/api/cover/7".to_owned()),
            },
            ..Track::default()
        }
    }

    /// The artist line is the server's own when it sent one, else the credited
    /// names joined — the same sentence every list shows.
    #[test]
    fn the_artist_line_is_one_sentence() {
        assert_eq!(artist_line(&track("T", &["A", "B"])), "A, B");
        let mut one = track("T", &["A"]);
        one.display_artist = Some("A feat. B".to_owned());
        assert_eq!(artist_line(&one), "A feat. B");
    }

    /// A row's key is the queue's own: the row's UUID when it has one, else the
    /// provider's id — the same value `nz-player`'s `Entry::key()` holds, so a
    /// tap from the home and a Flow pick name one row.
    #[test]
    fn a_catalogue_title_becomes_a_screen_row() {
        let card = track_card(&track("Titre", &["Artiste"]));
        assert_eq!(card.key, "3000168");
        assert_eq!(card.id, "3000168");
        assert_eq!(card.album, "Album");
        assert_eq!(card.cover.as_deref(), Some("/api/cover/7"));
        assert_eq!(card.duration_s, 200);
        assert_eq!(card.source, "deezer");

        let mut library = track("Titre", &["Artiste"]);
        library.id = Some(nz_model::TrackRowId::new(
            "11111111-1111-4111-8111-111111111111",
        ));
        let card = track_card(&library);
        assert_eq!(
            card.key, "11111111-1111-4111-8111-111111111111",
            "a library row is keyed by its UUID"
        );
        assert_eq!(
            card.id, "11111111-1111-4111-8111-111111111111",
            "and that is the id /stream accepts"
        );

        let mut upload = track("Titre", &["Artiste"]);
        upload.local = true;
        assert_eq!(track_card(&upload).source, "upload");
        let mut youtube = track("Titre", &["Artiste"]);
        youtube.source = Some("youtube".to_owned());
        assert_eq!(track_card(&youtube).source, "youtube");
    }

    /// The recap card is this year or the last one, and only when the year can
    /// tell a story (`recap.js#advertisedYear`).
    #[test]
    fn the_recap_card_is_this_year_or_the_last_one() {
        let row = |year: i32, enough: bool, events: u32| RecapYearRow {
            year,
            events,
            listens: events,
            minutes: 10,
            enough,
            partial: false,
        };
        let years = |rows: Vec<RecapYearRow>, now: i32| RecapYears {
            latest: rows.first().map(|row| row.year),
            years: rows,
            now_year: now,
            min_year: 2020,
            timezone: "server-local".to_owned(),
        };

        let last = years(vec![row(2025, true, 40)], 2026);
        assert_eq!(advertised_year(&last).map(|row| row.year), Some(2025));

        let two_years_old = years(vec![row(2024, true, 40)], 2026);
        assert!(advertised_year(&two_years_old).is_none());

        let thin = years(vec![row(2026, false, 2)], 2026);
        assert!(
            advertised_year(&thin).is_none(),
            "not enough to tell a story"
        );

        let silent = years(vec![row(2026, true, 0)], 2026);
        assert!(advertised_year(&silent).is_none(), "no event at all");
    }
}
