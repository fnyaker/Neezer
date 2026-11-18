//! Resolving a download context — `playlist:42`, `album:7`, `show:3`,
//! `track:3135556` — into the titles it holds (lot MB15).
//!
//! The SPA's download button sits on a page that ALREADY has the track list in
//! memory (`lib/offline.js#downloadTrack` is handed `track` objects). The phone's
//! `DownloadButton(context)` only carries the context, so the core asks the
//! server for it here: one read for a playlist or an album (the SPA's own page
//! routes), pages of episodes for a show (`/api/podcast/<id>` is cursor-paged on
//! purpose), and nothing at all for a single title.
//!
//! **This read is the action just taken** (rule 1): it runs on `Lane::User`
//! under the `offline` tag, and only the BYTES it queues afterwards are `Bulk`.
//! A playlist of 4 000 rows is still ONE request — its page carries its
//! tracklist.

use nz_api::Api;
use nz_model::{Episode, Track};

use crate::core::CoreError;

use super::view::TrackLabel;

/// How many episodes one request may pull from a show.
///
/// The server bounds a page itself; this is the phone's own ceiling, and 100 is
/// large enough that a 2 000-episode archive costs 20 requests and small enough
/// that one page never parks a megabyte of JSON in front of a tapped track.
pub const EPISODE_PAGE: u32 = 100;

/// How many pages a show may cost in one download request: 20 000 episodes is
/// beyond any real feed, and a feed that answers a cursor forever must not turn
/// one tap into an endless loop (rule 3: nothing fans out unbounded).
pub const EPISODE_PAGES_MAX: u32 = 200;

/// One title a set wants, with what a row draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wanted {
    pub id: String,
    pub label: TrackLabel,
}

/// A resolved context: what it is called, and every title it holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    pub title: String,
    pub tracks: Vec<Wanted>,
}

impl Resolved {
    fn push(&mut self, track: &Track) {
        let id = track.deezer_id.as_str().to_owned();
        self.tracks.push(Wanted {
            id: id.clone(),
            label: TrackLabel {
                id,
                title: track.title.clone(),
                artist: display_artist(track),
                cover: track.album.cover.clone(),
                duration_s: track.duration,
                explicit: track.explicit,
                added_at: super::now_ms(),
            },
        });
    }

    fn push_episode(&mut self, episode: &Episode, show: &str, cover: Option<&String>) {
        // An episode's stream id is its universal id, which is what
        // `/api/stream/<id>` takes (`nz-model`'s `Episode`).
        let id = episode.deezer_id.clone().unwrap_or_default();
        if id.is_empty() {
            return;
        }
        self.tracks.push(Wanted {
            id: id.clone(),
            label: TrackLabel {
                id,
                title: episode.title.clone(),
                artist: show.to_owned(),
                cover: episode.cover.clone().or_else(|| cover.cloned()),
                duration_s: episode.duration,
                explicit: episode.explicit,
                added_at: super::now_ms(),
            },
        });
    }
}

/// The one display line a row shows: the server's own joined form when it sent
/// one, the main artist otherwise (`webapp.md`: the list prints one line).
fn display_artist(track: &Track) -> String {
    if let Some(line) = track
        .display_artist
        .as_ref()
        .filter(|line| !line.is_empty())
    {
        return line.clone();
    }
    let main = track.artist.name.trim();
    if main.is_empty() {
        return String::new();
    }
    // The server joins the credits itself when it sent them; a row that carries
    // only the main artist is titled with that artist alone, never "A, ".
    main.to_owned()
}

/// Parse a context (`kind:id`) into its two halves. The id is everything after
/// the FIRST colon, so a server id that contains one is kept whole.
pub fn split_context(context: &str) -> Option<(&str, &str)> {
    let (kind, id) = context.split_once(':')?;
    if kind.is_empty() || id.is_empty() {
        return None;
    }
    Some((kind, id))
}

/// Whether the core knows how to download this kind. Anything else is a bug in
/// the caller, and `NzError::Unsupported` says so instead of fetching nothing.
pub fn known_kind(kind: &str) -> bool {
    matches!(kind, "playlist" | "album" | "show" | "track")
}

/// Resolve `kind:id` into the titles it holds.
pub async fn resolve(
    api: &Api,
    kind: &str,
    id: &str,
    title_hint: &str,
    artist_hint: &str,
) -> Result<Resolved, CoreError> {
    match kind {
        "playlist" => {
            let page = api.playlist(id).await.map_err(CoreError::from)?;
            let mut resolved = Resolved {
                title: page.playlist.title.clone(),
                ..Resolved::default()
            };
            for track in &page.tracks {
                resolved.push(track);
            }
            Ok(resolved)
        }
        "album" => {
            let page = api.album(id).await.map_err(CoreError::from)?;
            let mut resolved = Resolved {
                title: page.album.title.clone(),
                ..Resolved::default()
            };
            for track in &page.tracks {
                resolved.push(track);
            }
            Ok(resolved)
        }
        "show" => {
            let mut resolved = Resolved::default();
            let mut cursor: Option<String> = None;
            for page_number in 0..EPISODE_PAGES_MAX {
                let page = api
                    .podcast(id, EPISODE_PAGE, cursor.as_deref())
                    .await
                    .map_err(CoreError::from)?;
                if page_number == 0 {
                    resolved.title = page.show.title.clone();
                }
                let cover = page.show.cover.clone();
                for episode in &page.episodes {
                    resolved.push_episode(episode, &page.show.title, cover.as_ref());
                }
                match page.next.filter(|next| !next.is_empty()) {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
            Ok(resolved)
        }
        // A single title: the caller's own row is the only place its labels
        // exist (the phone has no route that turns a track id back into a
        // track), so they arrive as hints and an empty label still downloads.
        "track" => Ok(Resolved {
            title: if title_hint.is_empty() {
                id.to_owned()
            } else {
                title_hint.to_owned()
            },
            tracks: vec![Wanted {
                id: id.to_owned(),
                label: TrackLabel {
                    id: id.to_owned(),
                    title: title_hint.to_owned(),
                    artist: artist_hint.to_owned(),
                    cover: None,
                    duration_s: 0,
                    explicit: false,
                    added_at: super::now_ms(),
                },
            }],
        }),
        other => Err(CoreError::Unsupported {
            feature: format!("téléchargement de « {other} »"),
        }),
    }
}
