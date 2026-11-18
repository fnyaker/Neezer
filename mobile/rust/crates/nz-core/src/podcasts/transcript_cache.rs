//! The device's copy of transcript pages — the phone's `transcriptcache.js`.
//!
//! The SPA's IndexedDB store, ported with its numbers: **50 pages**, **5 MB**,
//! **4 pages per episode**, key `episode:limit:offset`, `ready` pages only, and
//! a local search over what is held so a search works with the link down. The
//! point is the same one: *a ready transcript costs 0 requests while listening*
//! — the panel follows the audio through pages it already has, and the network
//! is asked for a page or a search and nothing else.
//!
//! Two adaptations, both named in the lot's section of `docs/agents/mobile.md`:
//!
//! - **one file per episode** instead of one IndexedDB store, so a page read is
//!   a `read_to_string` and never a database open (the phone already opens SQLite
//!   for its own stores; a second engine for 5 MB of text is not worth its
//!   startup);
//! - **eviction drops whole episodes, oldest first**, rather than single pages.
//!   The caps are the same three numbers, the per-episode cap is exact, and the
//!   only thing that changes is that a partly-cached episode is evicted whole
//!   instead of page by page — which is what keeps the file count bounded too.

use serde::{Deserialize, Serialize};

use super::store;
use super::text;
use super::views::{Segment, TranscriptPage};
use crate::core::Core;

/// How many pages the whole cache holds (`transcriptcache.js#MAX_PAGES`).
pub const MAX_PAGES: usize = 50;
/// How many bytes the whole cache holds (`MAX_BYTES`, 5 MB).
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;
/// How many pages one episode keeps (`MAX_PER_EPISODE`).
pub const MAX_PER_EPISODE: usize = 4;

/// One stored page: the key's three parts, when it was written, and the record.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoredPage {
    offset: u32,
    limit: u32,
    /// Unix milliseconds of the write — the LRU's order and the "newest wins".
    at: u64,
    /// The page's own size in this file, for the byte cap.
    bytes: u64,
    #[serde(default)]
    language: Option<String>,
    duration_ms: u64,
    total: u32,
    segments: Vec<StoredSegment>,
    /// The page arrived as the answer to a search, and its lines are excerpts.
    #[serde(default)]
    query: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoredSegment {
    start: f64,
    end: f64,
    text: String,
}

/// One episode's file: its pages, and the time of the newest write.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct EpisodeFile {
    /// The episode these pages belong to, stored INSIDE the body: the file name
    /// is a hash of it (`store`'s rule), so the id is never parsed back out of
    /// a name.
    #[serde(default)]
    episode_id: String,
    /// Unix milliseconds of the newest page: the whole-episode LRU's order.
    #[serde(default)]
    at: u64,
    #[serde(default)]
    pages: Vec<StoredPage>,
}

impl EpisodeFile {
    fn bytes(&self) -> u64 {
        self.pages.iter().map(|page| page.bytes).sum()
    }

    fn read(page: &StoredPage) -> TranscriptPage {
        TranscriptPage {
            status: "ready".to_owned(),
            archived: true,
            language: page.language.clone(),
            duration_ms: page.duration_ms,
            done_ms: page.duration_ms,
            progress: 1.0,
            total: page.total,
            offset: page.offset,
            limit: page.limit,
            query: page.query.clone(),
            segments: page
                .segments
                .iter()
                .map(|segment| Segment {
                    start: segment.start,
                    end: segment.end,
                    text: segment.text.clone(),
                })
                .collect(),
            next: {
                let consumed = page.offset + page.segments.len() as u32;
                (consumed < page.total).then_some(consumed)
            },
            ..TranscriptPage::default()
        }
    }
}

/// The page at `(limit, offset)`, or `None` when it was never read (or was
/// evicted).
pub(super) fn read_page(
    core: &Core,
    episode_id: &str,
    limit: u32,
    offset: u32,
) -> Option<TranscriptPage> {
    let file = load(core, episode_id)?;
    let page = file
        .pages
        .iter()
        .find(|page| page.offset == offset && page.limit == limit)?;
    Some(EpisodeFile::read(page))
}

/// The page that COVERS `seconds` — the one whose first line starts at or before
/// it and whose last starts after (`transcriptcache.js#readAt`), so a panel that
/// reopens mid-episode paints the right lines without a request.
pub(super) fn read_at(
    core: &Core,
    episode_id: &str,
    limit: u32,
    seconds: f64,
) -> Option<TranscriptPage> {
    let file = load(core, episode_id)?;
    let mut pages: Vec<&StoredPage> = file
        .pages
        .iter()
        .filter(|page| page.limit == limit)
        .collect();
    pages.sort_by_key(|page| page.offset);
    if pages.is_empty() {
        return None;
    }
    if !seconds.is_finite() {
        return Some(EpisodeFile::read(pages[0]));
    }
    let mut found: Option<&StoredPage> = None;
    for page in &pages {
        let first = page.segments.first().map(|segment| segment.start);
        if first.is_some_and(|start| start <= seconds) {
            found = Some(page);
        }
    }
    Some(EpisodeFile::read(found.unwrap_or(pages[0])))
}

/// Keeps a ready page. A `ready` page with no segments, or any other status, is
/// never stored: the panel must not paint "nothing" as if it were the text.
pub(super) fn write_page(core: &Core, page: &TranscriptPage, at_ms: u64) -> bool {
    if page.status != "ready" || page.segments.is_empty() || page.episode_id.is_empty() {
        return false;
    }
    let offset = page.offset;
    let limit = page.limit.max(1);
    let mut file = load(core, &page.episode_id).unwrap_or_default();
    // A total that changed means the transcript was redone: the old pages
    // describe a text that no longer exists.
    let changed = file
        .pages
        .iter()
        .any(|stored| stored.limit == limit && stored.total != page.total);
    if changed {
        file.pages.retain(|stored| stored.limit != limit);
    }
    let entry = StoredPage {
        offset,
        limit,
        at: at_ms,
        bytes: 0,
        language: page.language.clone(),
        duration_ms: page.duration_ms,
        total: page.total,
        segments: page
            .segments
            .iter()
            .map(|segment| StoredSegment {
                start: segment.start,
                end: segment.end,
                text: segment.text.clone(),
            })
            .collect(),
        query: page.query.clone(),
    };
    file.pages
        .retain(|stored| !(stored.offset == offset && stored.limit == limit));
    file.pages.push(entry);
    // Per-episode cap: this episode's four newest pages, as the SPA keeps them.
    file.pages
        .sort_by_key(|stored| std::cmp::Reverse(stored.at));
    file.pages.truncate(MAX_PER_EPISODE);
    for stored in &mut file.pages {
        stored.bytes = stored
            .segments
            .iter()
            .map(|segment| segment.text.len() as u64 + 20)
            .sum::<u64>()
            + std::mem::size_of::<StoredPage>() as u64;
    }
    file.episode_id = page.episode_id.clone();
    file.at = file
        .pages
        .iter()
        .map(|stored| stored.at)
        .max()
        .unwrap_or(at_ms);
    save(core, &page.episode_id, &file)
}

/// Forgets one episode's pages (a deleted or restarted transcript).
pub(super) fn forget(core: &Core, episode_id: &str) {
    store::forget(core, store::TRANSCRIPT, episode_id);
}

/// What this cache holds, for the panel's own line and a test: pages, bytes and
/// episodes.
pub(super) fn stats(core: &Core) -> (usize, u64, usize) {
    let mut pages = 0usize;
    let mut bytes = 0u64;
    for path in store::files(core, store::TRANSCRIPT) {
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(file) = serde_json::from_str::<EpisodeFile>(&body) else {
            continue;
        };
        pages += file.pages.len();
        bytes += file.bytes();
    }
    (pages, bytes, store::files(core, store::TRANSCRIPT).len())
}

fn load(core: &Core, episode_id: &str) -> Option<EpisodeFile> {
    let body = store::read(core, store::TRANSCRIPT, episode_id)?;
    serde_json::from_str(&body).ok()
}

fn save(core: &Core, episode_id: &str, file: &EpisodeFile) -> bool {
    let Ok(body) = serde_json::to_string(file) else {
        return false;
    };
    if store::write(core, store::TRANSCRIPT, episode_id, &body).is_err() {
        return false;
    }
    prune(core, episode_id);
    true
}

/// The whole-cache caps: 50 pages and 5 MB, dropping the episodes with the
/// oldest newest-write first, never the one just saved.
fn prune(core: &Core, keep: &str) {
    let kept = store::path_of(core, store::TRANSCRIPT, keep);
    let mut files: Vec<(u64, u64, std::path::PathBuf)> = Vec::new();
    let mut pages = 0usize;
    let mut bytes = 0u64;
    for path in store::files(core, store::TRANSCRIPT) {
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(file) = serde_json::from_str::<EpisodeFile>(&body) else {
            continue;
        };
        pages += file.pages.len();
        bytes += file.bytes();
        if path != kept {
            files.push((file.at, file.pages.len() as u64, path));
        }
    }
    if pages <= MAX_PAGES && bytes <= MAX_BYTES {
        return;
    }
    files.sort_by_key(|(at, _, _)| *at);
    for (_, count, path) in files {
        if pages <= MAX_PAGES && bytes <= MAX_BYTES {
            break;
        }
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        let size = serde_json::from_str::<EpisodeFile>(&body)
            .map(|file| file.bytes())
            .unwrap_or(0);
        if std::fs::remove_file(&path).is_ok() {
            pages = pages.saturating_sub(count as usize);
            bytes = bytes.saturating_sub(size);
        }
    }
}

/// A local search inside ONE episode's held pages (`TranscriptPanel#localSearch`):
/// the lines whose folded text holds every word, an excerpt around the match.
pub(super) fn search_episode(
    core: &Core,
    episode_id: &str,
    query: &str,
    limit: usize,
) -> Vec<Segment> {
    let words: Vec<String> = text::fold_text(query)
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    if words.is_empty() {
        return Vec::new();
    }
    let Some(file) = load(core, episode_id) else {
        return Vec::new();
    };
    let mut pages: Vec<&StoredPage> = file.pages.iter().collect();
    pages.sort_by_key(|page| page.offset);
    let mut hits = Vec::new();
    for page in pages {
        for segment in &page.segments {
            let folded = text::fold_text(&segment.text);
            if !words.iter().all(|word| folded.contains(word.as_str())) {
                continue;
            }
            hits.push(Segment {
                start: segment.start,
                end: segment.end,
                text: text::excerpt_around(&segment.text, query, 240),
            });
        }
    }
    hits.truncate(limit.max(1));
    hits
}
