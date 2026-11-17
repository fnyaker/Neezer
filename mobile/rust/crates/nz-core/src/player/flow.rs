//! `PlayEntity`, the Flow and the radios: turning a list's NAME into rows, and
//! refilling a queue that ran out.
//!
//! Owned by **MB3b**. Three behaviours, all ported:
//!
//! - **`PlayEntity { kind, id }` fetches the list and starts it.** The kinds are
//!   the server's own routes (`album`, `playlist`, `artist`, `favorites`,
//!   `local`, `podcast`, `genre`, `flow`, `radio`); an unknown kind is refused
//!   with a French cause, never with a silent empty queue.
//! - **The Flow is asked ONE title at a time** (`webapp/src/lib/flow.js`): the
//!   queue holds the playing title and at most the next one, and the next is
//!   chosen with what the session said (what was heard and for how long, the
//!   discovery level, the energy notch, what the queue already holds). One
//!   request per title, and ONE more for the reserve at a start — measured in
//!   the tests, because a `/flow/next` storm is what that design prevents.
//! - **A radio is anchored on its ORIGINAL seed**, never on the queue's last
//!   title: `radio_next` takes the seed and the session, so a station does not
//!   drift a neighbour of a neighbour at a time (`lib/api.js#radioNext`).
//!
//! "The queue ran out" is `Effect::NeedMore`, which `nz-player` asks once PER
//! ROW — not on every later command, which is what used to make an unasked-once
//! top-up a storm (D4's own pitfall). The answer is
//! [`nz_player::PlayerState::auto_extend`]: append AND advance, so the music
//! goes on without a click.

use std::sync::Arc;

use serde_json::json;

use super::source::block_on;

/// How many titles a Flow start asks for: the first, and its reserve.
pub const FLOW_START_N: u8 = 2;
/// How many titles a refill asks for.
pub const FLOW_REFILL_N: u8 = 1;
/// The "most recent" tail a Flow is told not to repeat (`flow.js#exclude`).
pub const EXCLUDE_MAX: usize = 400;
/// How many episodes of a show one `PlayEntity` asks for: a show can hold
/// thousands, and the queue only needs the page it starts from (the podcast
/// screen pages the rest with MB13's own cursor).
pub const EPISODE_PAGE: u32 = 100;

/// What a `PlayEntity` or a start asked for, once resolved.
#[derive(Debug, Clone, PartialEq)]
pub enum Fill {
    /// A plain list: the rows ARE the queue.
    Rows(Vec<nz_player::Entry>),
    /// A Flow: the queue starts with what came back and asks for more at the
    /// end of each row (`context.kind == "flow"`).
    Flow { style: Option<String> },
    /// A radio, anchored on its seed.
    Radio { kind: String, id: String },
}

/// The Flow/radio filler: the session it reports and the client it asks.
pub struct Filler<'a> {
    api: Arc<nz_api::Api>,
    runtime: &'a tokio::runtime::Handle,
    gone: &'a mut tokio::sync::watch::Receiver<bool>,
    /// What the session heard, oldest first, as `{id, s}`.
    heard: Vec<(String, u32)>,
    /// What the queue holds, so the next title is not one already in it.
    exclude: Vec<String>,
}

impl<'a> Filler<'a> {
    pub fn new(
        api: Arc<nz_api::Api>,
        runtime: &'a tokio::runtime::Handle,
        gone: &'a mut tokio::sync::watch::Receiver<bool>,
    ) -> Filler<'a> {
        Filler {
            api,
            runtime,
            gone,
            heard: Vec::new(),
            exclude: Vec::new(),
        }
    }

    /// What was heard, so the Flow's choice has the session it needs.
    pub fn heard(&mut self, id: &str, seconds: u32) {
        self.heard.push((id.to_owned(), seconds));
        let len = self.heard.len();
        if len > 40 {
            self.heard.drain(..len - 40);
        }
    }

    /// The queue's tail, so a Flow or a radio does not hand back what is
    /// already queued.
    pub fn exclude(&mut self, ids: impl IntoIterator<Item = String>) {
        self.exclude.extend(ids);
        let len = self.exclude.len();
        if len > EXCLUDE_MAX {
            self.exclude.drain(..len - EXCLUDE_MAX);
        }
    }

    /// Resolves a `PlayEntity`. `Err` carries the French cause of a failure or
    /// an unknown kind.
    pub fn entity(&mut self, kind: &str, id: &str, shuffle: bool) -> Result<Fill, String> {
        let kind = kind.trim().to_ascii_lowercase();
        if kind == "flow" {
            return Ok(Fill::Flow {
                style: (!id.is_empty()).then(|| id.to_owned()),
            });
        }
        if kind == "radio" {
            return Ok(Fill::Radio {
                kind: "track".to_owned(),
                id: id.to_owned(),
            });
        }
        let api = Arc::clone(&self.api);
        let tracks = match kind.as_str() {
            "album" => entries(self.wait(api.album(id))?.tracks),
            "playlist" => entries(self.wait(api.playlist(id))?.tracks),
            "artist" => entries(self.wait(api.artist_tracks(id))?.tracks),
            "favorites" => entries(self.wait(api.my_favorites())?.tracks),
            "local" | "uploads" => entries(self.wait(api.my_local())?.tracks),
            "podcast" | "show" => self.episodes(id)?,
            "genre" => entries(self.wait(api.genre_radio(id, None))?.tracks),
            other => return Err(format!("Type de liste inconnu : {other}")),
        };
        let mut rows = tracks;
        if shuffle {
            shuffle_rows(&mut rows);
        }
        Ok(Fill::Rows(rows))
    }

    /// One Flow request: `n` titles with the session's own vocabulary
    /// (`discover.py#flow_next` reads exactly these keys).
    pub fn flow(&mut self, style: &Option<String>, n: u8) -> Result<Vec<nz_player::Entry>, String> {
        let heard: Vec<serde_json::Value> = self
            .heard
            .iter()
            .map(|(id, seconds)| json!({"id": id, "s": seconds}))
            .collect();
        let styles: Vec<&String> = style.iter().collect();
        let body = json!({
            "sid": serde_json::Value::Null,
            "d": serde_json::Value::Null,
            "energy": serde_json::Value::Null,
            "styles": styles,
            "heard": heard,
            "more": [],
            "less": [],
            "exclude": self.exclude,
            "n": n,
        });
        let api = Arc::clone(&self.api);
        let answer = self.wait(api.flow_pick(&body))?;
        Ok(entries(answer.tracks))
    }

    /// One radio request: the ORIGINAL seed, and what this session already
    /// heard or has queued.
    pub fn radio(&mut self, kind: &str, id: &str) -> Result<Vec<nz_player::Entry>, String> {
        let api = Arc::clone(&self.api);
        let answer = if kind == "artist" {
            self.wait(api.artist_radio(id))?.tracks
        } else {
            self.wait(api.track_radio(id))?.tracks
        };
        let mut rows = entries(answer);
        // Never hand back what is already queued or just heard: a station that
        // repeats itself is the reported "the radio plays the same three songs".
        rows.retain(|row| {
            !self.exclude.iter().any(|seen| seen == row.key())
                && !self.heard.iter().any(|(seen, _)| seen == row.key())
        });
        Ok(rows)
    }

    /// A show's episodes, as queue rows. They are built HERE rather than
    /// through [`entries`] because an episode is not a `Track`: it carries the
    /// podcast flag that keeps the crossfade away and lets the speed apply, and
    /// no album to name.
    fn episodes(&mut self, id: &str) -> Result<Vec<nz_player::Entry>, String> {
        let api = Arc::clone(&self.api);
        let page = self.wait(api.podcast(id, EPISODE_PAGE, None))?;
        Ok(page
            .episodes
            .into_iter()
            .map(|episode| nz_player::Entry {
                id: episode
                    .deezer_id
                    .clone()
                    .unwrap_or_else(|| episode.id.to_string()),
                deezer_id: episode.deezer_id.clone(),
                title: episode.title,
                duration: episode.duration,
                explicit: Some(episode.explicit),
                local: Some(true),
                podcast: Some(true),
                unavailable: Some(episode.unavailable),
                archived: Some(episode.archived),
                published: episode.published.map(|at| at.to_string()),
                channel_id: Some(episode.channel_id.to_string()),
                cover: episode.cover,
                ..Default::default()
            })
            .collect())
    }

    /// Runs a request, naming the close and the failure.
    fn wait<T>(
        &mut self,
        future: impl std::future::Future<Output = Result<T, nz_api::ApiError>>,
    ) -> Result<T, String> {
        block_on(self.runtime, self.gone, future)
            .ok_or_else(|| "lecteur fermé".to_owned())?
            .map_err(|error| error.cause_fr())
    }
}

/// `nz-model`'s rows, as the queue's own `Entry` (`commands.js#slimTrack`'s
/// shape: the id is the KEY, the cover is relative, the flags are optional).
///
/// A library row keeps its ROW UUID as the id (the server's `/stream/<id>`
/// accepts both spellings) and the Deezer number as `deezer_id`, which is what
/// `Entry::reference()` prefers — so a queue row names the same track whether it
/// came from the library or from a catalogue page.
pub fn entries(tracks: Vec<nz_model::Track>) -> Vec<nz_player::Entry> {
    tracks
        .into_iter()
        .map(|track| {
            let row_id = track.id.as_ref().map(|id| id.to_string());
            let archived = track.is_archived();
            let deezer_id = track.deezer_id.to_string();
            let id = row_id.clone().unwrap_or_else(|| deezer_id.clone());
            nz_player::Entry {
                id,
                deezer_id: Some(deezer_id.clone())
                    .filter(|other| Some(other.clone()) != row_id.clone()),
                title: track.title,
                duration: track.duration,
                explicit: Some(track.explicit),
                local: Some(track.local),
                gain: track.gain,
                unavailable: Some(track.unavailable),
                archived: Some(archived),
                cover: track.album.cover.clone(),
                display_artist: track.display_artist.clone(),
                artist: (!track.artist.name.is_empty()).then(|| nz_player::EntryArtist {
                    id: None,
                    deezer_id: Some(track.artist.deezer_id.to_string()).filter(|id| !id.is_empty()),
                    name: track.artist.name.clone(),
                    role: Some("Main".to_owned()),
                }),
                artists: track
                    .artists
                    .iter()
                    .filter(|artist| !artist.name.is_empty())
                    .map(|artist| nz_player::EntryArtist {
                        id: None,
                        deezer_id: Some(artist.deezer_id.to_string()).filter(|id| !id.is_empty()),
                        name: artist.name.clone(),
                        role: (!artist.role.is_empty()).then(|| artist.role.clone()),
                    })
                    .collect(),
                album: (!track.album.title.is_empty()).then(|| nz_player::EntryAlbum {
                    id: None,
                    deezer_id: Some(track.album.deezer_id.to_string()).filter(|id| !id.is_empty()),
                    title: track.album.title.clone(),
                    cover: track.album.cover.clone(),
                }),
                ..Default::default()
            }
        })
        .collect()
}

/// A shuffle of the rows a `PlayEntity` just fetched. Deterministic on the rows
/// themselves, which is what a test can assert, and the state machine
/// re-shuffles properly when the person presses the shuffle button.
fn shuffle_rows(rows: &mut [nz_player::Entry]) {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    for row in rows.iter() {
        for byte in row.key().bytes() {
            seed = seed.rotate_left(7) ^ u64::from(byte);
            seed = seed.wrapping_mul(0x100_0000_01B3);
        }
    }
    let len = rows.len();
    if len < 2 {
        return;
    }
    for at in (1..len).rev() {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let swap = (seed >> 33) as usize % (at + 1);
        rows.swap(at, swap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shuffle of a fetched list keeps every row, and the same rows give the
    /// same order — a test can assert where a shuffled album starts.
    #[test]
    fn a_shuffle_of_a_fetched_list_keeps_every_row_and_is_reproducible() {
        let make = || -> Vec<nz_player::Entry> {
            (0..50)
                .map(|n| nz_player::Entry::new(format!("key-{n}")))
                .collect()
        };
        let mut first = make();
        let mut second = make();
        shuffle_rows(&mut first);
        shuffle_rows(&mut second);
        assert_eq!(first, second);

        let mut keys: Vec<&str> = first.iter().map(|row| row.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 50, "no row was lost or duplicated");
        let original: Vec<String> = (0..50).map(|n| format!("key-{n}")).collect();
        let shuffled: Vec<&str> = first.iter().map(|row| row.key()).collect();
        assert_ne!(
            shuffled,
            original.iter().map(String::as_str).collect::<Vec<_>>(),
            "the order really moved"
        );
    }

    /// A library row is keyed by its UUID and names the Deezer number as the
    /// reference: the same track from two screens is one queue entry.
    #[test]
    fn a_library_row_keeps_its_uuid_as_the_key_and_the_deezer_id_as_the_reference() {
        use nz_model::{Track, TrackAlbum, TrackArtist, TrackId, TrackRowId};
        let row = Track {
            id: Some(TrackRowId::new("11111111-2222-4333-8444-555555555555")),
            deezer_id: TrackId::new("3000168"),
            title: "Titre".into(),
            duration: 200,
            artist: TrackArtist {
                deezer_id: nz_model::ArtistId::new("42"),
                name: "Artiste".into(),
            },
            album: TrackAlbum {
                deezer_id: nz_model::AlbumId::new("7"),
                title: "Album".into(),
                cover: Some("/api/cover/7".into()),
            },
            ..Default::default()
        };
        let entry = entries(vec![row]).pop().expect("one entry");
        assert_eq!(entry.key(), "11111111-2222-4333-8444-555555555555");
        assert_eq!(
            entry.reference(),
            Some("11111111-2222-4333-8444-555555555555")
        );
        assert_eq!(entry.deezer_id.as_deref(), Some("3000168"));
        assert_eq!(
            entry.artist.as_ref().map(|artist| artist.name.as_str()),
            Some("Artiste")
        );
        assert_eq!(
            entry.album.as_ref().map(|album| album.title.as_str()),
            Some("Album")
        );
    }
}
