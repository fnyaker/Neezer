//! The device's copy of each episode's chapters, and its LRU cap.
//!
//! `lib/chapters.js` keeps 40 episodes' chapter lists in memory and says why:
//! "keyed by episode id string, deduped, 40 entries" — the panel only ever holds
//! a handful, and a 2 000-episode show must not grow the store without bound.
//! The phone keeps the same 40, on disk, so a chapter list read once paints on
//! the next visit with **zero requests** (the screen standard's warm open) and
//! survives a process being killed in the background (rule 7 for the device's
//! own data).
//!
//! The SPA is explicit that chapters are NOT in the offline `/api` cache: a
//! chapter list read from a stale copy would be indistinguishable from a fresh
//! one. So [`cached`] answers with the copy AND its age, and the screen always
//! revalidates — the copy is what paints first, never what wins.

use serde_json::Value;

use super::store;
use super::views::{ChapterCard, Chapters};
use crate::core::Core;

/// How many episodes' chapter lists the device keeps (`chapters.js`
/// `MAX_ENTRIES`).
pub const MAX_ENTRIES: usize = 40;

/// The stored list for `episode_id`, with whether it came from disk (the screen
/// says « version mémorisée » on the second).
pub(super) fn cached(core: &Core, episode_id: &str) -> Option<Chapters> {
    let body = store::read(core, store::CHAPTERS, episode_id)?;
    let value: Value = serde_json::from_str(&body).ok()?;
    Some(from_value(&value))
}

/// Keeps one answer and prunes the store back to its cap.
pub(super) fn write(core: &Core, chapters: &Chapters) {
    let Ok(body) = serde_json::to_string(&to_value(chapters)) else {
        return;
    };
    if store::write(core, store::CHAPTERS, &chapters.episode_id, &body).is_err() {
        return;
    }
    prune(core, &chapters.episode_id);
}

/// Drops the oldest lists once there are more than [`MAX_ENTRIES`], never the
/// one just written.
fn prune(core: &Core, keep: &str) {
    let kept = store::path_of(core, store::CHAPTERS, keep);
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> =
        store::files(core, store::CHAPTERS)
            .into_iter()
            .filter(|path| *path != kept)
            .filter_map(|path| {
                let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
                Some((modified, path))
            })
            .collect();
    if files.len() < MAX_ENTRIES {
        return;
    }
    files.sort_by_key(|(modified, _)| *modified);
    let excess = files.len() + 1 - MAX_ENTRIES;
    for (_, path) in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}

/// The record as JSON: the stored body and nothing else.
fn to_value(chapters: &Chapters) -> Value {
    serde_json::json!({
        "episode_id": chapters.episode_id,
        "source": chapters.source,
        "archived": chapters.archived,
        "scanned": chapters.scanned,
        "duration": chapters.duration_s,
        "chapters": chapters
            .chapters
            .iter()
            .map(|chapter| serde_json::json!({
                "index": chapter.index,
                "start": chapter.start,
                "end": chapter.end,
                "title": chapter.title,
            }))
            .collect::<Vec<Value>>(),
    })
}

/// The stored body back into the record the screen reads.
fn from_value(value: &Value) -> Chapters {
    let chapters = value
        .get("chapters")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| ChapterCard {
                    index: row.get("index").and_then(Value::as_u64).unwrap_or(0) as u32,
                    start: row.get("start").and_then(Value::as_u64).unwrap_or(0) as u32,
                    end: row.get("end").and_then(Value::as_u64).map(|end| end as u32),
                    title: row
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();
    Chapters {
        episode_id: value
            .get("episode_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        chapters,
        source: value
            .get("source")
            .and_then(Value::as_str)
            .map(str::to_owned),
        archived: value
            .get("archived")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        scanned: value
            .get("scanned")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        duration_s: value.get("duration").and_then(Value::as_u64).unwrap_or(0) as u32,
    }
}
