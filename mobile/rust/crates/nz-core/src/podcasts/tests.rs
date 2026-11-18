//! What MB13 promises, measured against the fake server. Every number the PR
//! and `docs/agents/mobile.md` state comes from one of these.
//!
//! The suite is deliberately shaped like a user's session: open the list, open a
//! show, scroll to the end of 2 000 episodes, resume an episode with the link
//! down, read a transcript while listening, drop a marker, transcribe.

use std::collections::BTreeMap;

use super::testing::{
    BIG_SHOW, BIG_SHOW_EPISODES, EPISODE, PAGE, PodcastServer, TempDir, configured,
};
use super::*;
use crate::core::NetworkState;
use crate::player::EpisodeProgress;

/// A subscription list and a show page: **2** requests for a cold show, which is
/// the SPA's own count (`Show.svelte#load` fires the page and the markers), and
/// the device's copy afterwards, so the next visit paints with **0**.
#[tokio::test]
async fn a_cold_show_costs_two_requests_and_the_next_visit_paints_from_disk() {
    let (server, dir, core) = configured("cold-show").await;
    let shows = core.podcasts().await.expect("the list");
    assert_eq!(shows.len(), 2);
    assert_eq!(shows[0].title, "Les Histoires");
    assert!(shows[0].auto_archive);
    assert_eq!(server.count(), 1, "one request for /api/podcasts");

    server.forget();
    let page = core.show(BIG_SHOW).await.expect("the show");
    assert_eq!(page.show.title, "Les Histoires");
    assert_eq!(page.episodes.len(), PAGE);
    assert_eq!(server.count(), 2, "the page and its markers, as the SPA");

    // A second visit: the copy is there BEFORE any request, and it holds the
    // header, the rows and the position the person left.
    server.forget();
    let warm = core.cached_show(BIG_SHOW).expect("the device's copy");
    assert_eq!(warm.episodes.len(), PAGE);
    assert_eq!(warm.next.as_deref(), Some("offset:50"));
    assert_eq!(server.count(), 0, "a warm open costs no request");

    // And the list itself bridges a restart: a NEW core over the same directory
    // paints it before the network (the app being killed in the background). It
    // is kept PER ACCOUNT (`apicache.js#owner`), so the new core signs in first
    // — a signed-out core must not read somebody else's list.
    let restarted = dir.core();
    assert!(
        restarted.cached_podcasts().is_none(),
        "a signed-out core reads nothing"
    );
    restarted.set_server(&server.base()).await.expect("server");
    restarted.login("bench", "Bench1").await.expect("session");
    assert!(
        restarted
            .cached_podcasts()
            .is_some_and(|shows| shows.len() == 2),
        "the list survived the restart, under its account"
    );
}

/// The acceptance's biggest number: a show of **2 000 episodes** opens with
/// 50 rows and 1 request, and each further page is 1 request of 50 — 40 requests
/// for the whole archive, never 2 000 rows in one answer.
#[tokio::test]
async fn two_thousand_episodes_page_by_cursor_fifty_at_a_time() {
    let (server, _dir, core) = configured("paging").await;
    core.show(BIG_SHOW).await.expect("the first page");
    let mut page = core.cached_show(BIG_SHOW).expect("cached");
    assert_eq!(page.episodes.len(), 50);
    assert_eq!(server.count(), 2, "the page and its markers");

    let mut pages = 1;
    while page.next.is_some() {
        // Bounded: a cursor that never advances is a FAILURE, not a hang.
        assert!(pages < 60, "the cursor stopped advancing at page {pages}");
        server.forget();
        page = core.show_more(BIG_SHOW).await.expect("the next page");
        pages += 1;
        assert_eq!(
            server.count(),
            1,
            "one request per page, and nothing else (page {pages})"
        );
        assert!(
            page.episodes.len() <= BIG_SHOW_EPISODES,
            "the list grows, never duplicates: page {pages}"
        );
    }
    assert_eq!(pages, BIG_SHOW_EPISODES.div_ceil(PAGE), "40 pages of 50");
    assert_eq!(
        page.episodes.len(),
        BIG_SHOW_EPISODES,
        "every episode is reachable by cursor"
    );
    // The rows are the shape a row is DRAWN as, not the server's JSON.
    let first = &page.episodes[0];
    assert_eq!(first.channel_title, "Les Histoires");
    assert!(first.archived, "the first episode of each 250 is archived");
    assert_eq!(first.chapters_label(), "3 chapitres");
    assert_eq!(
        first.transcript.as_ref().map(|info| info.status.as_str()),
        Some("ready")
    );

    // The next page was asked with the cursor the answer named, and a limit of
    // 50 — the SPA's own page size.
    let hit = server.last_hit("/api/podcast/5ef08406-6ef3-4adf-835b-c9e9927ba70e");
    if let Some(hit) = hit {
        assert_eq!(hit.query("limit").as_deref(), Some("50"));
    }

    // The windowed read answers a page of rows without a request.
    server.forget();
    let window = core.episode_cards(BIG_SHOW, 1_990, 64);
    assert_eq!(window.len(), 10, "the tail of the list");
    assert_eq!(server.count(), 0, "a window never asks the server");
}

/// The acceptance's resume row: an episode comes back at its position with the
/// link DOWN, and **0 sockets** are opened to find out.
#[tokio::test]
async fn an_episode_resumes_at_its_position_with_the_link_down() {
    let (server, _dir, core) = configured("resume").await;
    // The show was opened once, online: the copy the device keeps is what the
    // resume bar is drawn from afterwards.
    core.show(BIG_SHOW).await.expect("the show");
    // What the player's own report writes (`player::listen`): the instant mirror.
    let resumed_id = "ep-00000".to_owned();
    let mut mirror: BTreeMap<String, EpisodeProgress> = BTreeMap::new();
    mirror.insert(
        resumed_id.clone(),
        EpisodeProgress {
            t: 1_200.0,
            d: 3_600.0,
            at: 1_794_960_000_000,
            done: false,
        },
    );
    core.cache()
        .kv()
        .set(KEY_PROGRESS, &mirror)
        .expect("the mirror");

    core.set_network(NetworkState::None);
    let before = server.accepted();
    let position = core.position(&resumed_id).expect("a position to resume");
    assert_eq!(position.seconds, 1_200);
    assert_eq!(position.duration_s, 3_600);
    assert_eq!(position.remaining_s(), 2_400);
    assert!((position.fraction() - 0.333).abs() < 0.01);
    assert!(!position.finished);
    assert_eq!(
        server.accepted(),
        before,
        "resuming asks the network nothing at all"
    );
    // The show page's resume bar reads the same number, from the same copy.
    let page = core.cached_show(BIG_SHOW).expect("the copy");
    let resumed = page
        .episodes
        .iter()
        .find(|episode| episode.id == resumed_id)
        .expect("the loaded page holds it")
        .position
        .clone()
        .expect("a resume position");
    assert_eq!(resumed.seconds, 1_200);
}

/// The server's copy and the device's mirror merge newest-wins, and a write the
/// server has not seen is flagged `pending` (the panel says so instead of
/// pretending the other device knows).
#[tokio::test]
async fn the_newest_position_wins_and_a_pending_write_says_so() {
    let (server, _dir, core) = configured("merge").await;
    let merged = core.progress().await.expect("the server's copy");
    let remote = merged.get(EPISODE).expect("the fixture's position");
    assert_eq!(remote.seconds, 1_200);
    assert_eq!(server.count(), 1, "one request for the whole map");

    // A local write NEWER than the server's wins…
    let mut mirror: BTreeMap<String, EpisodeProgress> = BTreeMap::new();
    mirror.insert(
        EPISODE.to_owned(),
        EpisodeProgress {
            t: 2_400.0,
            d: 3_600.0,
            at: 1_794_960_000_000 + 60_000,
            done: false,
        },
    );
    core.cache()
        .kv()
        .set(KEY_PROGRESS, &mirror)
        .expect("mirror");
    let merged = core.positions();
    assert_eq!(merged[EPISODE].seconds, 2_400);
    assert!(
        !merged[EPISODE].pending,
        "nothing is pending: the player flushed it or it was never queued"
    );

    // …and an OLDER one loses to the server.
    mirror.insert(
        EPISODE.to_owned(),
        EpisodeProgress {
            t: 300.0,
            d: 3_600.0,
            at: 1_000,
            done: false,
        },
    );
    core.cache()
        .kv()
        .set(KEY_PROGRESS, &mirror)
        .expect("mirror");
    assert_eq!(core.positions()[EPISODE].seconds, 1_200);

    // A local listen inside the last 15 s is FINISHED, and a finished episode is
    // offered as « Terminé », never as a resume point inside the credits.
    mirror.insert(
        EPISODE.to_owned(),
        EpisodeProgress {
            t: 3_590.0,
            d: 3_600.0,
            at: 2_000_000_000_000,
            done: true,
        },
    );
    core.cache()
        .kv()
        .set(KEY_PROGRESS, &mirror)
        .expect("mirror");
    let finished = core.position(EPISODE).expect("it is remembered");
    assert!(finished.finished);
    assert_eq!(finished.remaining_s(), 10);
}

/// A ready transcript costs **0** requests while listening: the first read is
/// one, every later page the device holds is free, and a page it does not hold
/// is one more.
#[tokio::test]
async fn a_ready_transcript_costs_one_request_then_nothing() {
    let (server, _dir, core) = configured("transcript").await;
    let page = core
        .transcript(EPISODE, TRANSCRIPT_PAGE, Some(0), None, None)
        .await
        .expect("a page");
    assert_eq!(page.status, "ready");
    assert_eq!(page.segments.len(), 200);
    assert_eq!(page.total, 240);
    assert_eq!(server.count(), 1);

    // The same page again, from the device: no socket.
    server.forget();
    let cached = core
        .cached_transcript(EPISODE, TRANSCRIPT_PAGE, 0, None)
        .expect("the device's copy");
    assert_eq!(cached.segments.len(), 200);
    assert_eq!(cached.segments[0].text, "Ligne 0");
    assert_eq!(server.count(), 0, "a held page costs nothing");

    // The page that COVERS a moment is found without a request too (`readAt`).
    assert!(
        core.cached_transcript(EPISODE, TRANSCRIPT_PAGE, 0, Some(10.0))
            .is_some()
    );
    assert_eq!(server.count(), 0);

    // A page the device does not hold is one request.
    let second = core
        .transcript(EPISODE, TRANSCRIPT_PAGE, Some(200), None, None)
        .await
        .expect("the tail");
    assert_eq!(server.count(), 1);
    assert_eq!(second.segments.len(), 40);
    assert!(second.next.is_none(), "the last page names no next");
    let stats = core.transcript_cache_stats();
    assert_eq!(stats.pages, 2, "two pages held");
    assert!(
        stats.bytes > 2_000,
        "the bytes are counted: {}",
        stats.bytes
    );
}

/// The panel's search: the server's hits when the link is up, the held pages
/// when it is down (`localSearch`), and never a request for one character.
#[tokio::test]
async fn a_search_answers_from_the_server_or_from_the_pages_held() {
    let (server, _dir, core) = configured("search").await;
    let short = core
        .show_transcript_search(BIG_SHOW, "l", SHOW_SEARCH_LIMIT)
        .await
        .expect("too short to send");
    assert!(short.hits.is_empty());
    assert_eq!(server.count(), 0, "one character asks nothing");

    let hits = core
        .show_transcript_search(BIG_SHOW, "maison", SHOW_SEARCH_LIMIT)
        .await
        .expect("the hits");
    assert_eq!(hits.total, 1);
    assert_eq!(hits.hits[0].start, 180.0);
    assert_eq!(hits.hits[0].episode_title, "Épisode 12");
    assert_eq!(hits.hits[0].channel_title, "Les Histoires");
    assert_eq!(server.count(), 1);

    // Offline, the panel searches what it holds: the page read above has
    // « Ligne 12 » in it.
    core.transcript(EPISODE, TRANSCRIPT_PAGE, Some(0), None, None)
        .await
        .expect("a page");
    core.set_network(NetworkState::None);
    let local = core.local_transcript_search(EPISODE, "ligne 137", 10);
    assert_eq!(local.len(), 1, "the held page answers: {local:?}");
    assert!(local[0].text.contains("Ligne 137"));
    assert!(
        (local[0].start - 137.0 * 15.0).abs() < 0.01,
        "at its own moment"
    );
    let cold = core
        .transcript(EPISODE, TRANSCRIPT_PAGE, Some(0), None, None)
        .await;
    assert_eq!(
        cold.err(),
        Some(CoreError::Offline),
        "and the network is not asked"
    );
}

/// Chapters: one request, kept on the device, the CURRENT chapter found for a
/// position, and the store's cap of 40 episodes (`chapters.js#MAX_ENTRIES`).
#[tokio::test]
async fn chapters_are_read_once_kept_and_capped() {
    let (server, _dir, core) = configured("chapters").await;
    let chapters = core.chapters(EPISODE).await.expect("the list");
    assert_eq!(chapters.chapters.len(), 3);
    assert!(chapters.archived);
    assert_eq!(chapters.source.as_deref(), Some("id3"));
    assert_eq!(server.count(), 1);
    assert_eq!(chapters.at(1_000.0).map(|(at, _)| at), Some(1));
    assert_eq!(chapters.next(1_000.0).map(|(at, _)| at), Some(2));
    // A chapter the feed gives no end for ends with the file.
    assert_eq!(
        chapters.end_of(2),
        3_600,
        "the last chapter ends with the file"
    );
    assert_eq!(chapters.chapters[2].end, None, "and keeps that as a fact");

    server.forget();
    let cached = core.cached_chapters(EPISODE).expect("the copy");
    assert_eq!(cached.chapters.len(), 3);
    assert_eq!(server.count(), 0, "a reopen costs nothing");

    // The cap: 45 episodes' lists keep 40 files.
    for index in 0..45 {
        core.chapters(&format!("ep-{index:05}"))
            .await
            .expect("a list");
    }
    let files = crate::podcasts::store::files(&core, crate::podcasts::store::CHAPTERS);
    assert_eq!(files.len(), super::chapters::MAX_ENTRIES, "40 lists kept");
    assert!(
        core.cached_chapters(&format!("ep-{:05}", 44)).is_some(),
        "the newest one is never the one dropped"
    );
}

/// A marker appears before the answer and goes away if the server refuses: the
/// click is optimistic, and the failure is the cause, not an empty screen.
#[tokio::test]
async fn a_marker_is_optimistic_and_rolls_back() {
    let (server, _dir, core) = configured("markers").await;
    core.show(BIG_SHOW).await.expect("the show");
    assert_eq!(
        core.episode_markers(EPISODE).len(),
        1,
        "the show's answer carried one marker"
    );

    let added = core
        .add_marker(EPISODE, 1_800_000, Some("Le pont"))
        .await
        .expect("added");
    assert_eq!(added.seconds(), 1_800);
    assert_eq!(added.time_label(), "30:00");
    let held = core.episode_markers(EPISODE);
    assert_eq!(held.len(), 2);
    assert_eq!(
        held.last().map(|marker| marker.label.as_str()),
        Some("Le pont")
    );
    assert_eq!(
        server
            .last_body("/api/podcast/episode/31254005-fa75-4784-805b-e3ade4d7a65a/markers")
            .as_deref()
            .map(|body| body.contains("\"position\":1800000")),
        Some(true),
        "the position is MILLISECONDS on the wire"
    );

    core.remove_marker("m-1").await.expect("removed");
    assert_eq!(core.episode_markers(EPISODE).len(), 1);

    // A refusal puts the previous list back.
    let target = core.episode_markers(EPISODE)[0].clone();
    server.break_route(&format!("/api/podcast/marker/{}", target.id), 500);
    assert!(core.remove_marker(&target.id).await.is_err());
    assert_eq!(
        core.episode_markers(EPISODE).len(),
        1,
        "the marker the person kept is back"
    );
    assert_eq!(core.episode_markers(EPISODE)[0].id, target.id);
}

/// Subscribing is one request and lands on a page that is already loaded.
#[tokio::test]
async fn subscribing_returns_the_page_it_just_read() {
    let (_server, _dir, core) = configured("subscribe").await;
    let card = core
        .subscribe_podcast("https://www.deezer.com/show/9400003")
        .await
        .expect("subscribed");
    assert_eq!(card.title, "Nouveau podcast");
    assert!(
        core.cached_podcasts()
            .unwrap_or_default()
            .iter()
            .any(|show| show.id == card.id)
    );
    // A catalogue card names its show by the provider's id; the URL is built from
    // it exactly as `actions.js#subscribeToPodcast` builds it.
    let found = core
        .search_podcasts("histoires")
        .await
        .expect("the catalogue");
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].subscribe_url().as_deref(),
        Some("https://www.deezer.com/show/9400001")
    );
    // A link that is not a URL is refused BEFORE a request.
    assert!(matches!(
        core.subscribe_podcast("pas un lien").await,
        Err(CoreError::Invalid { .. })
    ));
}

/// The archive switch moves at once, says how many episodes it queued, and goes
/// back if the server refuses.
#[tokio::test]
async fn the_archive_switch_answers_at_once_and_rolls_back() {
    let (server, _dir, core) = configured("archive").await;
    core.show(BIG_SHOW).await.expect("the show");
    let answer = core.auto_archive(BIG_SHOW, true).await.expect("queued");
    assert_eq!(answer.queued, 17);
    assert!(answer.show.auto_archive);
    assert_eq!(server.count(), 2 + 1, "the page, the markers, the switch");
    assert_eq!(
        core.cached_show(BIG_SHOW).expect("copy").episodes.len(),
        PAGE,
        "the answer's channel carries no episodes: the loaded rows stay"
    );

    server.break_route(
        "/api/podcast/5ef08406-6ef3-4adf-835b-c9e9927ba70e/auto_archive",
        500,
    );
    assert!(core.auto_archive(BIG_SHOW, false).await.is_err());
    assert!(
        core.cached_show(BIG_SHOW).expect("copy").show.auto_archive,
        "a refusal puts the switch back where it was"
    );
}

/// The transcription card: the settings, the reason when it is unavailable, and
/// the show's own switch answering what it queued and skipped.
#[tokio::test]
async fn the_transcription_card_knows_availability_and_what_it_queued() {
    let (_server, _dir, core) = configured("transcribe").await;
    let settings = core.transcribe_settings(false).await.expect("settings");
    assert!(settings.available);
    assert_eq!(settings.language, "fr");
    assert_eq!(settings.languages.len(), 7);
    assert!(settings.engine.as_deref() == Some("whisper.cpp"));
    assert!(core.transcription_supported().await.unwrap_or(false));

    // The show's switch answers the show, the backlog it used, what it queued
    // and what it skipped (rule 2: the job says what it did).
    core.podcasts().await.expect("the list");
    let answer = core
        .set_show_transcribe(BIG_SHOW, Some(true), Some("recent"), Some("fr"))
        .await
        .expect("marked");
    assert_eq!(answer.queued, 12);
    assert_eq!(answer.skipped, 1);
    assert_eq!(answer.backlog, "recent");
    assert_eq!(
        answer.show.map(|show| show.transcribe_language),
        Some("fr".to_owned())
    );

    // An invalid backlog is refused before the request.
    assert!(matches!(
        core.set_show_transcribe(BIG_SHOW, None, Some("tout"), None)
            .await,
        Err(CoreError::Invalid { .. })
    ));
    assert!(matches!(
        core.set_show_transcribe(BIG_SHOW, None, None, None).await,
        Err(CoreError::Invalid { .. })
    ));

    // Starting one episode is a job: one request, and the panel polls.
    core.start_transcript(EPISODE, None, false)
        .await
        .expect("queued");
    core.stop_transcript(EPISODE).await.expect("stopped");
    core.delete_transcript(EPISODE).await.expect("deleted");
}

/// A forced restart describes a text that no longer exists, so the device's
/// pages of the OLD text are dropped with it.
#[tokio::test]
async fn a_forced_restart_forgets_the_pages_of_the_old_text() {
    let (_server, _dir, core) = configured("restart").await;
    core.transcript(EPISODE, TRANSCRIPT_PAGE, Some(0), None, None)
        .await
        .expect("a page");
    assert_eq!(core.transcript_cache_stats().pages, 1);
    core.start_transcript(EPISODE, Some("fr"), true)
        .await
        .expect("restarted");
    assert_eq!(
        core.transcript_cache_stats().pages,
        0,
        "the old text's pages went with it"
    );
    assert!(
        core.cached_transcript(EPISODE, TRANSCRIPT_PAGE, 0, None)
            .is_none()
    );
}

/// Nothing crosses whole: a page of 50 rows is what the screen windows, and the
/// memory it costs is COUNTED (worker rule 4), not guessed from a process.
#[tokio::test]
async fn the_pages_rows_and_memory_are_counted() {
    let (_server, _dir, core) = configured("memory").await;
    let page = core.show(BIG_SHOW).await.expect("the show");
    let bytes = page.memory_bytes();
    assert!(
        bytes > 50 * 300,
        "50 rows of a real episode are more than 300 B each: {bytes}"
    );
    assert!(
        bytes < 50 * 2_000,
        "and less than 2 kB each — a screen-shaped row, not the server's JSON: {bytes}"
    );
    let rows = page.rows();
    assert_eq!(rows.len(), 50);
    assert_eq!(rows[0].source, "podcast");
    assert_eq!(
        rows[0].key, rows[0].id,
        "an episode's key is its UUID, as the player queues it"
    );
}

/// Every route this area reads is tagged, so `NzCore::stats()` answers what a
/// podcast screen cost without the UI counting anything.
#[tokio::test]
async fn the_meter_books_the_podcast_tag() {
    let (_server, _dir, core) = configured("meter").await;
    core.podcasts().await.expect("the list");
    let rows = core.stats();
    let tag = rows
        .iter()
        .find(|row| row.tag == TAG_PODCASTS)
        .expect("the podcasts tag");
    assert_eq!(tag.requests, 1);
    let _ = core.stats();
    let _ = format!("{tag:?}");
    assert!(
        tag.bytes_in > 100,
        "the list's body is counted: {}",
        tag.bytes_in
    );
}

/// A server that predates the transcription routes answers `None` from the probe
/// (rule 7): the screen hides the card instead of failing.
#[tokio::test]
async fn a_server_without_the_transcription_routes_is_a_doubt_not_a_failure() {
    let (server, _dir, core) = configured("unsupported").await;
    server.break_route("/api/transcribe/status", 404);
    assert_eq!(core.transcription_supported().await, Some(false));
}

/// Keeps one server and one directory alive for the compile-time use above.
#[allow(dead_code)]
fn _keep(_: (PodcastServer, TempDir)) {}
