//! This person's podcast markers, kept on the device and written optimistically.
//!
//! `lib/markers.js` in one file: a store keyed by episode that every view reads,
//! every mutation applied to it FIRST and reconciled with the server's answer
//! after, and a rollback that puts the previous list back when the server says
//! no. The SPA keeps the map in memory and re-reads it at login; the phone keeps
//! it on disk too, because a marker is a place somebody chose and a process that
//! dies in the background must not lose it (rule 7 for the device's data).
//!
//! The unit is the server's: a marker's `position` is MILLISECONDS (a chapter's
//! `start` is seconds). [`super::views::Marker`] owns the one conversion, and
//! this store never mixes the two.
//!
//! The map is small by construction — the server caps markers at 100 per episode
//! and 5 000 per person (`webui/__init__.py`'s `_MAX_MARKERS*`) — so one file per
//! account holds all of them and a write is a single rename.

use std::collections::BTreeMap;

use super::store;
use super::views::Marker;
use crate::core::Core;

/// Every marker of this account, episode id → markers in position order.
pub(super) fn all(core: &Core) -> BTreeMap<String, Vec<Marker>> {
    let Some(body) = store::read(core, store::MARKERS, "") else {
        return BTreeMap::new();
    };
    serde_json::from_str(&body).unwrap_or_default()
}

/// One episode's markers, in position order.
pub(super) fn for_episode(core: &Core, episode_id: &str) -> Vec<Marker> {
    all(core).remove(episode_id).map(sorted).unwrap_or_default()
}

/// Replaces one episode's list (the answer to `GET /api/podcast/<id>/markers`
/// grouped them for us).
pub(super) fn put_episode(core: &Core, episode_id: &str, markers: Vec<Marker>) {
    let mut map = all(core);
    map.insert(episode_id.to_owned(), sorted(markers));
    save(core, &map);
}

/// Replaces every episode the server named, KEEPING the episodes it did not
/// (a show's answer covers the episodes it holds; another show's markers stay).
pub(super) fn merge_show(core: &Core, grouped: BTreeMap<String, Vec<Marker>>) {
    let mut map = all(core);
    for (episode, markers) in grouped {
        map.insert(episode, sorted(markers));
    }
    save(core, &map);
}

/// Adds one marker, optimistic: the list holds it before the request is sent,
/// and the caller puts the previous list back if the request fails.
pub(super) fn insert(core: &Core, marker: Marker) {
    let mut map = all(core);
    let entry = map.entry(marker.episode_id.clone()).or_default();
    entry.retain(|existing| existing.id != marker.id);
    entry.push(marker);
    let episode = entry.first().map(|marker| marker.episode_id.clone());
    if let Some(episode) = episode {
        let list = map.remove(&episode).unwrap_or_default();
        map.insert(episode, sorted(list));
    }
    save(core, &map);
}

/// Removes one marker by id, answering what was there (the rollback's copy).
pub(super) fn remove(core: &Core, marker_id: &str) -> Option<Marker> {
    let mut map = all(core);
    let mut removed = None;
    for markers in map.values_mut() {
        if let Some(at) = markers.iter().position(|marker| marker.id == marker_id) {
            removed = Some(markers.remove(at));
            break;
        }
    }
    save(core, &map);
    removed
}

/// Puts one marker back exactly where it was (a refused delete's rollback).
pub(super) fn restore(core: &Core, marker: Marker) {
    insert(core, marker);
}

fn sorted(mut markers: Vec<Marker>) -> Vec<Marker> {
    markers.sort_by_key(|marker| (marker.position_ms, marker.id.clone()));
    markers
}

fn save(core: &Core, map: &BTreeMap<String, Vec<Marker>>) {
    let Ok(body) = serde_json::to_string(map) else {
        return;
    };
    if store::write(core, store::MARKERS, "", &body).is_err() {
        tracing::debug!("the markers' copy was not written");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(id: &str, episode: &str, position_ms: u32) -> Marker {
        Marker {
            id: id.to_owned(),
            episode_id: episode.to_owned(),
            position_ms,
            label: String::new(),
        }
    }

    /// The store keeps the episodes apart and each list in position order, which
    /// is what the chips under an episode are drawn from.
    #[test]
    fn markers_are_kept_per_episode_in_position_order() {
        let dir = crate::podcasts::testing::TempDir::new("markers-order");
        let core = dir.core();
        put_episode(
            &core,
            "ep-1",
            vec![marker("b", "ep-1", 900_000), marker("a", "ep-1", 60_000)],
        );
        put_episode(&core, "ep-2", vec![marker("c", "ep-2", 1_000)]);
        let first = for_episode(&core, "ep-1");
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].id, "a", "the earlier marker first");
        assert_eq!(first[1].seconds(), 900);
        assert_eq!(for_episode(&core, "ep-2").len(), 1);
        assert!(for_episode(&core, "ep-3").is_empty());
    }

    /// A delete answers the marker it removed, so a refusal can put it back
    /// exactly where it was.
    #[test]
    fn a_removed_marker_can_come_back() {
        let dir = crate::podcasts::testing::TempDir::new("markers-rollback");
        let core = dir.core();
        put_episode(&core, "ep-1", vec![marker("a", "ep-1", 60_000)]);
        let removed = remove(&core, "a").expect("removed");
        assert!(for_episode(&core, "ep-1").is_empty());
        restore(&core, removed);
        assert_eq!(for_episode(&core, "ep-1").len(), 1);
        assert!(remove(&core, "missing").is_none());
    }

    /// A show's grouped answer replaces the episodes it names and keeps the
    /// ones it does not — another show's markers survive a re-read.
    #[test]
    fn a_show_merge_never_drops_another_shows_markers() {
        let dir = crate::podcasts::testing::TempDir::new("markers-merge");
        let core = dir.core();
        put_episode(&core, "elsewhere", vec![marker("z", "elsewhere", 5_000)]);
        let mut grouped = BTreeMap::new();
        grouped.insert("ep-1".to_owned(), vec![marker("a", "ep-1", 60_000)]);
        merge_show(&core, grouped);
        assert_eq!(for_episode(&core, "ep-1").len(), 1);
        assert_eq!(for_episode(&core, "elsewhere").len(), 1);
        put_episode(&core, "ep-1", Vec::new());
        assert!(for_episode(&core, "ep-1").is_empty());
        assert_eq!(for_episode(&core, "elsewhere").len(), 1);
    }
}
