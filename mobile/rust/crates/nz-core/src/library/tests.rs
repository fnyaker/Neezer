//! What this area is held to: one request per leaf, none before a warm open
//! paints, 8 000 favourites in ONE answer, the folder writes the SPA's own
//! optimistic shape, and a cause with « Réessayer » when the link is dead.
//!
//! Every number here is COUNTED (requests at the server, bytes in the meter),
//! and every wait is on a channel or an `await` — never on a clock.

use super::testing::{FAVORITES_ROWS, LibraryServer, TempDir, configured};
use super::*;
use crate::core::NetworkState;

/// The tag's own row in the core's meter.
fn library_stats(core: &Core) -> (u64, u64, u64) {
    core.stats()
        .into_iter()
        .find(|row| row.tag == TAG_LIBRARY)
        .map(|row| (row.requests, row.bytes_in, row.bytes_out))
        .unwrap_or_default()
}

/// The acceptance: **8 000 favourites, ONE request cold, ZERO before a warm open
/// paints**, and the rows are the server's own (key, title, position).
#[tokio::test]
async fn eight_thousand_favorites_cost_one_request_cold_and_none_warm() {
    let (server, _dir, core) = configured().await;

    let rows = core.favorites().await.expect("the list");
    assert_eq!(rows.len(), FAVORITES_ROWS);
    assert_eq!(server.count(), 1, "one route, one request");
    assert_eq!(server.routes(), ["GET /api/me/favorites"]);
    assert_eq!(rows[0].position, 0);
    assert_eq!(rows[FAVORITES_ROWS - 1].position, FAVORITES_ROWS as u32 - 1);
    assert!(
        rows.iter().all(|row| row.liked),
        "the favourites ARE the stars: every row's heart is on"
    );
    assert_eq!(rows[0].title, "Ouverture 0");
    assert_eq!(rows[1].title, "Sinnerman 1");
    assert_eq!(rows[1].artists, "Nina Simone");
    // The key is the row's UUID (`trackkey.js#trackKey`), which is what the
    // queue and every other list key their rows by.
    assert_eq!(rows[0].key, "11111111-1111-4111-8111-000000000000");
    assert!(rows[0].album_id.is_some());

    // The counted bytes are what the answer really weighed, not a guess.
    let (requests, bytes_in, _) = library_stats(&core);
    assert_eq!(requests, 1);
    assert!(
        bytes_in > 1_000_000,
        "8 000 rows are over a megabyte on the wire: {bytes_in} B"
    );

    // A WARM open: the copy answers with no request at all, and the rows are
    // the same list (the same converter read it back).
    server.forget();
    let cached = core.cached_favorites().expect("the copy of the visit");
    assert_eq!(cached.len(), FAVORITES_ROWS);
    assert_eq!(
        server.count(),
        0,
        "a warm paint asks nothing: the device's copy is the list"
    );
    assert_eq!(cached[1].title, rows[1].title);
    assert_eq!(cached[1].key, rows[1].key);
    assert_eq!(library_stats(&core).0, 1, "and the meter did not move");

    // The SPA's own refresh (`loadMyFavorites` → `refreshFavTracks`) is the one
    // request a warm open makes in the BACKGROUND, after the copy is drawn.
    let fresh = core.favorites().await.expect("the refresh");
    assert_eq!(fresh.len(), FAVORITES_ROWS);
    assert_eq!(server.count(), 1);
}

/// Rule 3: no link means no socket. The copy stays; the failure is a cause the
/// screen prints, never a spinner.
#[tokio::test]
async fn a_dead_link_opens_no_socket_and_keeps_the_copy() {
    let (server, _dir, core) = configured().await;
    core.favorites().await.expect("the first visit");
    core.library_playlists().await.expect("the first visit");
    let accepted = server.accepted();

    core.set_network(NetworkState::None);
    assert_eq!(
        core.favorites().await.err(),
        Some(CoreError::Offline),
        "the gate answers before a socket"
    );
    assert_eq!(
        core.library_playlists().await.err(),
        Some(CoreError::Offline)
    );
    assert_eq!(
        core.move_playlist_to("aaaaaaaa-1111-4111-8111-000000000002", None)
            .await
            .err(),
        Some(CoreError::Offline)
    );
    assert_eq!(
        server.accepted(),
        accepted,
        "not one socket was opened while offline"
    );
    // And the copy is still what the screen paints.
    assert_eq!(
        core.cached_favorites().map(|rows| rows.len()),
        Some(FAVORITES_ROWS)
    );
    assert!(core.cached_library_playlists().is_some());
}

/// A leaf that fails with no copy answers a CAUSE: the screen shows it and
/// « Réessayer » (rule 3), it never invents an empty list.
#[tokio::test]
async fn a_refused_leaf_answers_its_cause() {
    let (server, _dir, core) = configured().await;
    server.break_route("/api/me/albums", 500);

    let refused = core.albums().await.expect_err("the server said no");
    assert!(
        matches!(refused, CoreError::Server { status: 500, .. }),
        "{refused:?}"
    );
    assert!(core.cached_albums().is_none(), "nothing was kept");
    // The other leaves are untouched by one section's refusal.
    assert!(core.artists().await.is_ok());
}

/// The playlist tree is grouped the way `playlistfolders.js` groups it: the
/// folders in their order, then « Sans dossier », « Partagées avec moi » and
/// « Depuis Deezer » — each only when it has rows.
#[tokio::test]
async fn the_playlist_tree_is_grouped_like_the_spa() {
    let (server, _dir, core) = configured().await;
    let tree = core.library_playlists().await.expect("the tree");
    assert_eq!(server.routes(), ["GET /api/me/playlists"]);
    assert_eq!(tree.folders.len(), 2);
    assert_eq!(tree.playlists.len(), 4);

    let keys: Vec<&str> = tree
        .sections
        .iter()
        .map(|section| section.key.as_str())
        .collect();
    assert_eq!(
        keys,
        [
            "folder:ffffffff-1111-4111-8111-000000000001",
            "folder:ffffffff-1111-4111-8111-000000000002",
            "root",
            "shared",
            "imported"
        ],
        "a folder is a section even when empty; the four derived ones follow"
    );
    assert_eq!(tree.sections[0].title, "Voyage");
    assert_eq!(tree.sections[0].count, 1);
    assert_eq!(tree.sections[0].rows[0].title, "Route");
    assert_eq!(tree.sections[1].count, 0, "an empty folder still shows");
    assert_eq!(tree.sections[2].title, "Sans dossier");
    assert_eq!(tree.sections[2].rows.len(), 1);
    assert_eq!(tree.sections[2].rows[0].title, "Matin");
    assert_eq!(tree.sections[3].rows[0].title, "Avec June");
    assert_eq!(tree.sections[4].rows[0].title, "Import Deezer");

    // A search keeps only the sections that match (`buildSections(filtering)`).
    let filtered = sections(&tree.folders, &tree.playlists, true);
    assert!(
        filtered.iter().all(|section| !section.rows.is_empty()),
        "a filtered list never shows an empty heading"
    );

    // Warm: the copy answers, no request.
    server.forget();
    let cached = core.cached_library_playlists().expect("the copy");
    assert_eq!(cached.playlists.len(), 4);
    assert_eq!(cached.sections.len(), tree.sections.len());
    assert_eq!(server.count(), 0);
}

/// A folder write is the SPA's: the tree changes first, ONE request carries the
/// whole desired order, and the client-chosen id is what a retry would repeat.
#[tokio::test]
async fn a_folder_is_created_with_the_client_id_and_one_request() {
    let (server, _dir, core) = configured().await;
    core.library_playlists().await.expect("the tree");
    server.forget();

    let tree = core
        .create_folder("  Route de nuit  ")
        .await
        .expect("created");
    assert_eq!(server.routes(), ["POST /api/playlist-folders"]);
    let body = server.last_body("/api/playlist-folders").expect("the body");
    assert!(body.contains("\"title\":\"Route de nuit\""), "{body}");
    let created = tree.folders.last().expect("the new folder");
    assert_eq!(created.title, "Route de nuit");
    assert_eq!(created.count, 0);
    assert_eq!(created.position, 2, "appended after the two there were");
    assert!(
        body.contains(&created.id),
        "the body carries the id this client chose: {body}"
    );
    // The id is a v4 UUID: the server parses it with `uuid.UUID`.
    assert_eq!(created.id.len(), 36);
    assert_eq!(created.id.as_bytes()[14], b'4', "version 4: {}", created.id);
    assert!(
        matches!(created.id.as_bytes()[19], b'8' | b'9' | b'a' | b'b'),
        "variant 1: {}",
        created.id
    );
    // And it is on the device at once: a reopen paints it before any request.
    server.forget();
    assert_eq!(
        core.cached_library_playlists()
            .map(|tree| tree.folders.len()),
        Some(3)
    );
    assert_eq!(server.count(), 0);

    // A blank title is refused here, with no request at all.
    assert!(core.create_folder("   ").await.is_err());
    assert_eq!(server.count(), 0);
}

/// `actions.js#movePlaylistTo`: the row moves to the END of its new container
/// locally, and the ONE request is the destination's complete order. Offline it
/// is refused BEFORE the move — the acceptance's "rolls back with Réessayer".
#[tokio::test]
async fn a_move_changes_the_tree_first_and_sends_the_destination_order() {
    let (server, _dir, core) = configured().await;
    core.library_playlists().await.expect("the tree");
    server.forget();

    let folder = "ffffffff-1111-4111-8111-000000000002";
    let tree = core
        .move_playlist_to("aaaaaaaa-1111-4111-8111-000000000002", Some(folder))
        .await
        .expect("moved");
    assert_eq!(
        server
            .hits()
            .iter()
            .map(|hit| format!("{} {}", hit.method, hit.route()))
            .collect::<Vec<_>>(),
        ["PUT /api/playlist-folders/ffffffff-1111-4111-8111-000000000002/order"]
    );
    let hit = server
        .last_hit("/api/playlist-folders/ffffffff-1111-4111-8111-000000000002/order")
        .expect("the destination's order");
    assert!(
        hit.body.contains("aaaaaaaa-1111-4111-8111-000000000002"),
        "the moved row is in the order: {}",
        hit.body
    );
    let moved = tree.sections[1]
        .rows
        .iter()
        .find(|row| row.id == "aaaaaaaa-1111-4111-8111-000000000002")
        .expect("in its new folder");
    assert_eq!(
        moved.folder_id.as_deref(),
        Some(folder),
        "the local tree moved it before the answer"
    );
    assert_eq!(tree.sections[1].count, 1, "the folder's badge follows it");
    assert!(
        tree.sections[2].rows.iter().all(|row| row.id != moved.id),
        "and it left « Sans dossier »"
    );

    // Moving it where it already is asks nothing (`movePlaylistTo` returns true).
    server.forget();
    core.move_playlist_to(&moved.id, Some(folder))
        .await
        .expect("already there");
    assert_eq!(server.count(), 0);

    // Offline: refused, no request, and the device still holds the moved tree.
    core.set_network(NetworkState::None);
    assert_eq!(
        core.move_playlist_to("aaaaaaaa-1111-4111-8111-000000000003", None)
            .await
            .err(),
        Some(CoreError::Offline)
    );
    assert_eq!(server.count(), 0);
    let kept = core.cached_library_playlists().expect("the copy");
    let shared = kept
        .sections
        .iter()
        .find(|section| section.kind == SectionKind::Shared)
        .expect("« Partagées avec moi »");
    assert!(
        shared
            .rows
            .iter()
            .any(|row| row.id == "aaaaaaaa-1111-4111-8111-000000000003"),
        "the refused move did not happen"
    );
}

/// A rename answers in the tree, ONE `PATCH`; a delete releases its playlists to
/// « Sans dossier » and ONE `DELETE`.
#[tokio::test]
async fn a_rename_and_a_delete_are_one_request_each() {
    let (server, _dir, core) = configured().await;
    core.library_playlists().await.expect("the tree");
    let folder = "ffffffff-1111-4111-8111-000000000001";

    server.forget();
    let tree = core
        .rename_folder(folder, "Voyages")
        .await
        .expect("renamed");
    assert_eq!(
        server
            .hits()
            .iter()
            .map(|hit| format!("{} {}", hit.method, hit.route()))
            .collect::<Vec<_>>(),
        ["PATCH /api/playlist-folders/ffffffff-1111-4111-8111-000000000001"]
    );
    assert_eq!(tree.folders[0].title, "Voyages");
    assert_eq!(
        tree.sections[0].title, "Voyages",
        "the section's heading too"
    );

    server.forget();
    let tree = core.delete_folder(folder).await.expect("deleted");
    assert_eq!(
        server
            .hits()
            .iter()
            .map(|hit| format!("{} {}", hit.method, hit.route()))
            .collect::<Vec<_>>(),
        ["DELETE /api/playlist-folders/ffffffff-1111-4111-8111-000000000001"]
    );
    assert_eq!(tree.folders.len(), 1);
    let root = tree
        .sections
        .iter()
        .find(|section| section.kind == SectionKind::Root)
        .expect("« Sans dossier »");
    assert!(
        root.rows
            .iter()
            .any(|row| row.id == "aaaaaaaa-1111-4111-8111-000000000001"),
        "the folder's playlist moved to the root: {:?}",
        root.rows.iter().map(|row| &row.title).collect::<Vec<_>>()
    );
    assert_eq!(
        tree.playlists.len(),
        4,
        "a folder delete never loses a playlist"
    );

    // A folder this person does not have is a cause, not a silent success.
    assert!(core.rename_folder("inconnu", "X").await.is_err());
}

/// The folder order is the ids' order, and ONE `PUT` carries it; anything the
/// client left out keeps its place at the end (`orderFolders`).
#[tokio::test]
async fn the_folder_order_is_one_put_of_the_full_list() {
    let (server, _dir, core) = configured().await;
    core.library_playlists().await.expect("the tree");
    server.forget();

    let second = "ffffffff-1111-4111-8111-000000000002".to_owned();
    let first = "ffffffff-1111-4111-8111-000000000001".to_owned();
    let tree = core
        .reorder_folders(&[second.clone(), first.clone()])
        .await
        .expect("reordered");
    assert_eq!(server.routes(), ["PUT /api/playlist-folders/order"]);
    assert_eq!(tree.folders[0].id, second);
    assert_eq!(tree.folders[1].id, first);
    assert_eq!(tree.folders[0].position, 0);
    assert_eq!(tree.folders[1].position, 1);
    let body = server
        .last_body("/api/playlist-folders/order")
        .expect("body");
    assert!(body.contains(&second) && body.contains(&first), "{body}");

    // A partial list renumbers what it named and keeps the rest at the end.
    let ordered = order_folders(&tree.folders, std::slice::from_ref(&first));
    assert_eq!(ordered[0].id, first);
    assert_eq!(ordered[1].id, second);
    assert_eq!(ordered[1].position, 1);
}

/// Every leaf is ONE request cold and nothing warm, and the light route carries
/// four id arrays rather than titles.
#[tokio::test]
async fn every_leaf_costs_one_request_cold_and_none_warm() {
    let (server, _dir, core) = configured().await;

    macro_rules! leaf {
        ($name:literal, $route:literal, $fresh:ident, $cached:ident) => {{
            server.forget();
            let rows = core.$fresh().await.expect($name);
            assert_eq!(server.routes(), [format!("GET {}", $route)], $name);
            let warm = core.$cached().expect($name);
            assert!(!warm.is_empty() || rows.is_empty(), $name);
            assert_eq!(server.count(), 1, "{}: one request, never two", $name);
        }};
    }
    leaf!("the artists", "/api/me/artists", artists, cached_artists);
    leaf!("the albums", "/api/me/albums", albums, cached_albums);
    leaf!("the local files", "/api/me/local", local, cached_local);
    leaf!(
        "the dislikes",
        "/api/me/dislikes",
        dislikes,
        cached_dislikes
    );
    leaf!(
        "the unavailable rows",
        "/api/unavailable",
        unavailable,
        cached_unavailable
    );

    let artists = core.artists().await.expect("artists");
    server.forget();
    let _ = artists;
    assert_eq!(core.artists().await.expect("artists").len(), 2);
    assert_eq!(server.count(), 1, "a fresh read always asks once");

    // The artists carry what their grid draws, and the order the server sent
    // (« Les plus écoutés » is the server's own).
    let rows = core.cached_artists().expect("the copy");
    assert_eq!(rows[0].name, "Ascenseur");
    assert_eq!(rows[0].plays, 340);
    assert_eq!(rows[0].tracks, 120);
    assert_eq!(rows[1].starred, "2026-11-12T21:00:00");

    let albums = core.cached_albums().expect("the copy");
    assert_eq!(albums[0].title, "Le matin");
    assert_eq!(albums[0].artist, "Ascenseur");

    // The upload usage and the light ids are their own routes.
    server.forget();
    let usage = core.upload_usage().await.expect("usage");
    assert_eq!(usage.used, 1_073_741_824);
    assert_eq!(usage.quota, 5_368_709_120);
    assert!(!usage.unlimited);
    assert!((usage.percent() - 20.0).abs() < 0.001, "a fifth full");
    assert!(core.cached_upload_usage().is_some());
    assert_eq!(server.routes(), ["GET /api/upload/usage"]);

    server.forget();
    let ids = core.favorite_ids().await.expect("ids");
    assert_eq!(server.routes(), ["GET /api/me/favorite-ids"]);
    assert_eq!(ids.tracks.len(), 2);
    assert_eq!(ids.disliked, ["7001"], "a dislike is not a star");
    assert!(core.cached_favorite_ids().is_some());

    // A dislike removal is ONE mutation and it drops the kept list.
    server.forget();
    core.remove_dislike("7001").await.expect("removed");
    assert_eq!(server.routes(), ["POST /api/dislike"]);
    assert!(
        core.cached_dislikes().is_none(),
        "the kept copy cannot resurrect the row"
    );
}

/// The history is a keyset cursor: 50 per page, both halves together, and the
/// filter is the server's own `outcome`.
#[tokio::test]
async fn the_history_pages_by_cursor_and_keeps_its_first_page() {
    let (server, _dir, core) = configured().await;

    let page = core
        .history(None, None, None, HISTORY_PAGE)
        .await
        .expect("the first page");
    assert_eq!(server.routes(), ["GET /api/history"]);
    let hit = server.last_hit("/api/history").expect("the request");
    assert_eq!(hit.query("limit").as_deref(), Some("50"));
    assert_eq!(hit.query("before"), None, "no cursor on the first page");
    assert_eq!(page.events.len(), 2);
    assert_eq!(page.events[0].outcome, 1);
    assert_eq!(page.events[1].outcome, -1);
    assert_eq!(page.events[0].track.title, "Ouverture");
    assert_eq!(
        page.next,
        Some(HistoryCursorRow {
            before: "2026-11-17T20:58:00".to_owned(),
            before_id: 101
        })
    );
    assert_eq!(page.summary.listens, 41);
    assert_eq!(page.summary.top_artists[0].name, "Ascenseur");
    assert!(!page.summary.is_empty());

    // The next page carries BOTH halves, encoded.
    server.forget();
    let next = page.next.clone().expect("a cursor");
    core.history(None, Some(&next.before), Some(next.before_id), 50)
        .await
        .expect("the next page");
    let hit = server.last_hit("/api/history").expect("the request");
    assert_eq!(
        hit.query("before").as_deref(),
        Some("2026-11-17T20%3A58%3A00"),
        "the timestamp is one query value"
    );
    assert_eq!(hit.query("before_id").as_deref(), Some("101"));

    // A filter is the server's own `outcome`; the copy is kept per filter.
    server.forget();
    let skips = core
        .history(Some("skip"), None, None, 50)
        .await
        .expect("the skips");
    assert!(skips.events.is_empty());
    assert!(skips.summary.is_empty(), "an empty history says so");
    let hit = server.last_hit("/api/history").expect("the request");
    assert_eq!(hit.query("outcome").as_deref(), Some("skip"));

    server.forget();
    let kept = core.cached_history("skip").expect("the copy");
    assert!(kept.events.is_empty());
    assert_eq!(server.count(), 0);
    let all = core.cached_history("").expect("the unfiltered copy");
    assert_eq!(all.events.len(), 2, "each filter keeps its own page");

    // A page beyond the first is a position in a live log, not "the list".
    server.forget();
    core.history(Some("listen"), Some("2026-11-17T20:58:00"), Some(101), 50)
        .await
        .expect("a cursor page");
    assert_eq!(
        core.cached_history("listen"),
        None,
        "a cursor page is never kept"
    );
}

/// Rule 7: a server that predates the folder routes says so, and the screen
/// hides the folder affordances instead of failing. A doubt (`None`) hides
/// nothing.
#[tokio::test]
async fn an_older_server_hides_the_folders_instead_of_failing() {
    let (server, _dir, core) = configured().await;
    assert_eq!(
        core.folders_supported().await,
        Some(true),
        "the probe answers a registered route"
    );
    assert_eq!(
        server
            .last_hit("/api/playlist-folders")
            .map(|hit| hit.method),
        Some("OPTIONS".to_owned()),
        "the probe asks for the ROUTE's existence, never a GET"
    );

    let old = LibraryServer::start();
    old.break_route("/api/playlist-folders", 404);
    let other = TempDir::new("library-old");
    let core = other.core();
    core.set_server(&old.base()).await.expect("reachable");
    core.login("bench", "Bench1").await.expect("a session");
    assert_eq!(
        core.folders_supported().await,
        Some(false),
        "a 404 on the probe is an older server"
    );

    // A doubt is not a verdict: no link at all answers nothing, so nothing is
    // hidden.
    core.set_network(NetworkState::None);
    assert_eq!(core.folders_supported().await, None);
}

/// Worker rule 4: the tree's memory is COUNTED from its own strings, never read
/// off a process.
#[test]
fn the_tree_counts_its_own_bytes() {
    let folders = vec![FolderCard {
        id: "f".repeat(36),
        title: "Voyage".to_owned(),
        position: 0,
        count: 1,
    }];
    let playlists = vec![PlaylistCard {
        id: "p".repeat(36),
        title: "Route".to_owned(),
        owner: "Nadia".to_owned(),
        role: "owner".to_owned(),
        ..PlaylistCard::default()
    }];
    let tree = LibraryTree::new(folders, playlists);
    let bytes = tree.memory_bytes();
    assert!(bytes > 100, "the rows are counted one by one: {bytes}");
    assert!(bytes < 4_096, "two rows are not a megabyte: {bytes}");
    assert_eq!(tree.playlists.len(), 1);
    assert_eq!(tree.folders[0].count, 0, "recounted from its own rows");
}

/// `moveItem`, `orderFolders` and `orderContainer` are the SPA's own functions,
/// with the SPA's own edge cases (clamped ends, `Infinity` for a row the list
/// did not name, buckets by container).
#[test]
fn the_ordering_helpers_are_the_spas_own() {
    assert_eq!(move_item(&[0, 1, 2], 0, 2), [1, 2, 0]);
    assert_eq!(move_item(&[0, 1, 2], 0, 9), [1, 2, 0], "a clamped target");
    assert_eq!(move_item(&[0, 1, 2], 9, 1), [0, 2, 1], "a clamped source");
    assert_eq!(move_item::<u8>(&[], 0, 1), Vec::<u8>::new());

    let folders = vec![
        FolderCard {
            id: "a".to_owned(),
            title: "A".to_owned(),
            position: 0,
            count: 0,
        },
        FolderCard {
            id: "b".to_owned(),
            title: "B".to_owned(),
            position: 1,
            count: 0,
        },
    ];
    let playlists = vec![
        PlaylistCard {
            id: "one".to_owned(),
            folder_id: Some("a".to_owned()),
            position: Some(0),
            ..PlaylistCard::default()
        },
        PlaylistCard {
            id: "two".to_owned(),
            folder_id: None,
            ..PlaylistCard::default()
        },
        PlaylistCard {
            id: "three".to_owned(),
            folder_id: Some("b".to_owned()),
            position: Some(0),
            ..PlaylistCard::default()
        },
    ];
    // The root's order is its own; the folders keep their rows.
    let ordered = order_container(&playlists, &folders, None, &["two".to_owned()]);
    let ids: Vec<&str> = ordered.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, ["one", "three", "two"], "folders first, the root last");

    // Naming a row from ANOTHER container moves it there (`orderContainer`).
    let moved = order_container(&playlists, &folders, Some("a"), &["three".to_owned()]);
    let ids: Vec<&str> = moved.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, ["three", "one", "two"]);
    assert_eq!(moved[0].folder_id.as_deref(), Some("a"));
    assert_eq!(moved[0].position, Some(0));
    assert_eq!(moved[1].position, Some(1), "the resident follows it");
    assert_eq!(
        container_ids(&LibraryTree::new(folders.clone(), moved), Some("a")),
        ["three", "one"]
    );
}

/// A folder id is a UUID the server parses, and two presses never share one.
#[test]
fn a_folder_id_is_a_fresh_uuid_each_time() {
    let first = new_folder_id();
    let second = new_folder_id();
    assert_ne!(first, second);
    for id in [&first, &second] {
        assert_eq!(id.len(), 36);
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(
            parts.iter().map(|part| part.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(id.chars().all(|ch| ch == '-' || ch.is_ascii_hexdigit()));
        assert_eq!(&id[14..15], "4");
    }
}
