//! MB25's acceptance, measured: what each node of the car's tree costs (one
//! request cold, none warm), what a tap plays, and what a hostile id cannot do.
//!
//! Every count is a number the fake server logged, not a duration; the one
//! expiry test ages the copy's modification time instead of sleeping.

use std::time::Duration;

use super::testing::{TempDir, configured};
use super::*;

/// The one copy a test wrote, so it can age it instead of waiting.
fn sole_copy(dir: &TempDir) -> std::path::PathBuf {
    let directory = dir.cache_dir().join("auto");
    let mut copies: Vec<std::path::PathBuf> = std::fs::read_dir(&directory)
        .expect("the copies' directory")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    assert_eq!(copies.len(), 1, "one browse wrote one copy");
    copies.remove(0)
}

/// One title really downloaded: the `.part` reserved, its bytes written, the
/// file published, and MB15's label kept under its `offline.set.` key.
fn publish_download(core: &Core, id: &str, title: &str, artist: &str) {
    let owner = core
        .me()
        .expect("the bench is signed in")
        .user
        .id
        .as_str()
        .to_owned();
    let track = nz_api::TrackId::new(id.to_owned());
    let cache = core.cache();
    let partial = cache
        .downloads()
        .reserve(&owner, &track, nz_api::Quality::Opus320)
        .expect("reserved");
    std::fs::write(&partial, b"OggS").expect("the bytes");
    cache
        .downloads()
        .complete(&owner, &track, 4)
        .expect("published");
    let label = crate::offline::TrackLabel {
        id: id.to_owned(),
        title: title.to_owned(),
        artist: artist.to_owned(),
        cover: None,
        duration_s: 210,
        explicit: false,
        added_at: 1_700_000_000_000,
    };
    let set = crate::offline::QueuedSet {
        title: "Mes téléchargements".to_owned(),
        tracks: vec![label],
    };
    let context = format!("track:{id}");
    cache
        .kv()
        .set(
            &format!("{}{context}", crate::offline::KEY_SET_PREFIX),
            &set,
        )
        .expect("the label");
    cache
        .kv()
        .set(crate::offline::KEY_SETS, &vec![context])
        .expect("the index");
}

/// The root is free: a car that connects sees five sections and the device has
/// spent **0 requests** before the person asks for anything.
#[tokio::test]
async fn the_root_names_the_five_sections_with_no_request() {
    let (server, _dir, core) = configured().await;
    let items = core.auto_children(ROOT).await.expect("the root");
    let titles: Vec<&str> = items.iter().map(|item| item.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Favoris",
            "Playlists",
            "Mon Flow",
            "Récents",
            "Téléchargements"
        ]
    );
    assert_eq!(server.count(), 0, "ouvrir l'arbre ne coûte aucune requête");
    assert!(
        items[0].browsable && items[0].playable,
        "Favoris se parcourt et se joue"
    );
    assert!(
        items[1].browsable && !items[1].playable,
        "Playlists se parcourt"
    );
    assert!(
        items[2].playable && !items[2].browsable,
        "Mon Flow se joue, il ne se parcourt pas"
    );
    assert!(items[3].browsable, "Récents se parcourt");
    assert!(items[4].browsable, "Téléchargements se parcourt");
    core.shutdown();
}

/// **The acceptance's number**: one request for the first browse of a node, and
/// none afterwards (its copy is on the device).
#[tokio::test]
async fn a_browse_asks_once_then_reads_the_device_copy() {
    let (server, _dir, core) = configured().await;
    let first = core.auto_children(FAVORITES).await.expect("Favoris");
    assert_eq!(first.len(), MAX_CHILDREN, "un nœud est plafonné à 100");
    assert_eq!(first[0].title, "Ouverture 0");
    assert!(first[0].liked, "un favori est une étoile par définition");
    assert_eq!(server.count_of("/api/me/favorites"), 1, "1 requête froide");
    assert!(
        server.hits()[0].background,
        "une lecture d'arbre part en Prefetch (X-NS-Background: 1), jamais en User"
    );

    let before = server.count();
    let again = core.auto_children(FAVORITES).await.expect("Favoris");
    assert_eq!(again, first, "la même liste, servie par la copie");
    assert_eq!(server.count(), before, "0 requête à chaud");

    let painted = core.auto_cached_children(FAVORITES);
    assert_eq!(painted.len(), MAX_CHILDREN);
    assert_eq!(server.count(), before, "la copie synchrone ne demande rien");

    // The core's own meter carries the number the PR quotes (`NzCore.stats`).
    let stats = core
        .stats()
        .into_iter()
        .find(|row| row.tag == TAG_AUTO)
        .expect("le tag auto");
    assert_eq!(stats.requests, 1, "une lecture froide = une requête comptée");
    assert!(stats.bytes_in > 0, "et ses octets");
    core.shutdown();
}

/// A copy older than [`FRESH`] is asked once again — the staleness rule, read
/// from the file's own modification time (no sleep).
#[tokio::test]
async fn a_copy_older_than_fifteen_minutes_is_asked_again() {
    let (server, dir, core) = configured().await;
    let _ = core.auto_children(FAVORITES).await.expect("Favoris");
    let copy = sole_copy(&dir);
    let aged = std::time::SystemTime::now() - FRESH - Duration::from_secs(1);
    std::fs::File::options()
        .write(true)
        .open(&copy)
        .expect("the copy")
        .set_modified(aged)
        .expect("aged");

    server.forget();
    let again = core.auto_children(FAVORITES).await.expect("Favoris");
    assert_eq!(again.len(), MAX_CHILDREN);
    assert_eq!(
        server.count_of("/api/me/favorites"),
        1,
        "une copie périmée se redemande UNE fois"
    );
    core.shutdown();
}

/// The playlist library: the person's own lists first, then the ones shared
/// with them, and one of them browses its titles then plays them for free.
#[tokio::test]
async fn the_playlists_come_own_first_and_one_plays_what_it_showed() {
    let (server, _dir, core) = configured().await;
    let lists = core.auto_children(PLAYLISTS).await.expect("Playlists");
    let titles: Vec<&str> = lists.iter().map(|item| item.title.as_str()).collect();
    assert_eq!(titles, ["Route", "Matin", "Avec June"]);
    assert_eq!(lists[0].subtitle, "12 titres");
    assert_eq!(lists[2].subtitle, "8 titres");
    assert!(lists[0].browsable && lists[0].playable);
    assert_eq!(server.count_of("/api/me/playlists"), 1);

    let route = lists[0].media_id.clone();
    let browsed = core.auto_children(&route).await.expect("une playlist");
    let titles: Vec<&str> = browsed.iter().map(|item| item.title.as_str()).collect();
    assert_eq!(titles, ["Départ", "Nationale 7", "Arrivée"]);
    assert_eq!(
        server.count_of("/api/playlist/aaaaaaaa-1111-4111-8111-000000000001"),
        1,
        "parcourir une playlist coûte une requête"
    );

    // **The tap**: what the car just showed IS the queue, so it costs nothing.
    let before = server.count();
    match core.auto_play(&route).await.expect("elle se joue") {
        Play::Queue(tracks) => assert_eq!(tracks, browsed),
        other => panic!("une playlist doit rendre ses titres, pas {other:?}"),
    }
    assert_eq!(
        server.count(),
        before,
        "0 requête pour jouer ce qui est affiché"
    );
    core.shutdown();
}

/// A tap on a node the device has never seen is the ACTION the person just
/// took: `User`, never `Prefetch` (rule 1).
#[tokio::test]
async fn a_play_on_a_cold_node_asks_on_the_user_lane() {
    let (server, _dir, core) = configured().await;
    match core.auto_play(FAVORITES).await.expect("Favoris se joue") {
        Play::Queue(tracks) => assert_eq!(tracks.len(), MAX_CHILDREN),
        other => panic!("{other:?}"),
    }
    assert_eq!(server.count_of("/api/me/favorites"), 1);
    assert!(
        !server.hits()[0].background,
        "le tap est l'action juste : voie User, sans X-NS-Background"
    );
    core.shutdown();
}

/// Mon Flow is the player's business: its queue asks for one title at a time
/// (`nz-core/src/player/flow.rs`), so the tree hands it the entity and spends
/// nothing.
#[tokio::test]
async fn mon_flow_is_the_players_own_entity() {
    let (server, _dir, core) = configured().await;
    assert_eq!(
        core.auto_play(FLOW).await.expect("le Flow se joue"),
        Play::Entity {
            kind: "flow".to_owned(),
            id: String::new()
        }
    );
    assert_eq!(server.count(), 0, "le Flow se choisit chez le lecteur");
    assert!(core.auto_children(FLOW).await.expect("rien").is_empty());
    core.shutdown();
}

/// « Récents » is the listening history: newest first, one row per title.
#[tokio::test]
async fn recents_keep_one_row_per_title_newest_first() {
    let (server, _dir, core) = configured().await;
    let items = core.auto_children(RECENT).await.expect("Récents");
    let titles: Vec<&str> = items.iter().map(|item| item.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Ouverture", "Suite"],
        "deux écoutes d'Ouverture = une ligne"
    );
    assert_eq!(server.count_of("/api/history"), 1);
    assert!(
        server.hits()[0].path.contains("limit=30"),
        "{:?}",
        server.hits()[0]
    );
    core.shutdown();
}

/// The downloads are on the device: **0 requests**, ever, and a tap plays what
/// is shown.
#[tokio::test]
async fn the_downloads_come_from_the_device_and_cost_no_request() {
    let (server, _dir, core) = configured().await;
    publish_download(&core, "deezer:1234", "Titre local", "Artiste local");
    let items = core
        .auto_children(DOWNLOADS)
        .await
        .expect("Téléchargements");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "Titre local");
    assert_eq!(items[0].subtitle, "Artiste local");
    assert_eq!(items[0].duration_s, 210);
    assert!(items[0].playable && !items[0].browsable);
    assert_eq!(server.count(), 0, "un téléchargement ne demande rien");

    let before = server.count();
    match core.auto_play(DOWNLOADS).await.expect("ils se jouent") {
        Play::Queue(tracks) => assert_eq!(tracks.len(), 1),
        other => panic!("{other:?}"),
    }
    assert_eq!(server.count(), before);
    core.shutdown();
}

/// An id the tree did not build names nothing: no route, no file, no query.
#[test]
fn a_media_id_the_tree_did_not_build_is_refused() {
    for id in [
        "",
        " ",
        "neezer:",
        "neezer:root:extra",
        "playlist:x",
        "neezer:playlist:../etc/passwd",
        "neezer:playlist:aaaaaaaa/bbbb",
        "neezer:playlist:aaaaaaaa?x=1",
        "neezer:playlist:",
        "neezer:track:a/b",
        "neezer:track:a#b",
    ] {
        assert_eq!(parse(id), None, "{id:?} doit être refusé");
    }
    assert_eq!(parse(ROOT), Some((Request::Root, "")));
    assert_eq!(
        parse("neezer:playlist:aaaaaaaa-1111-4111-8111-000000000001"),
        Some((Request::Playlist, "aaaaaaaa-1111-4111-8111-000000000001"))
    );
    assert_eq!(
        parse("neezer:track:deezer:123"),
        Some((Request::Track, "deezer:123"))
    );
    assert_eq!(
        parse(&track_media_id("11111111-1111-4111-8111-000000000001")),
        Some((Request::Track, "11111111-1111-4111-8111-000000000001"))
    );
}

/// A media id the tree does not know answers a French cause, not an empty list
/// that looks like "nothing here" (rule 3).
#[tokio::test]
async fn a_node_the_tree_does_not_know_answers_a_cause() {
    let (server, _dir, core) = configured().await;
    let error = core
        .auto_children("neezer:playlist:../x")
        .await
        .expect_err("refusé");
    assert!(matches!(error, CoreError::Invalid { .. }), "{error:?}");
    assert!(error.to_string().contains("Nœud inconnu"), "{error}");
    assert_eq!(server.count(), 0);
    core.shutdown();
}
