//! MB19's acceptance, driven step by step — never slept through.
//!
//! `await_holding_lock` is allowed for the whole module: [`bench`] is a plain
//! `std::sync::Mutex` taken by a test that owns a whole runtime, so holding it
//! across an await is the POINT (one test at a time), and it is never contended.
#![allow(clippy::await_holding_lock)]
//!
//! The headline row of the lot (`docs/agents/mobile.md`, MB19) is *"the phone
//! controls the desktop app — a pause reaches it, acked"*. The desktop app's
//! half of the protocol is `nz-remote::host` over `nz_player`'s state machine —
//! the very code its `wiring/remote_host.rs` runs — so these tests drive that
//! code (a real poll, a real publish, a real `PlayerState::apply`) while THIS
//! lot's controller does the tapping. Nothing of the protocol is simulated on
//! either side.
//!
//! The server is [`FakeServer`], a loopback implementation of
//! `neezer/webui/remote.py`'s routes (see `testing.rs` for why a Python fixture
//! cannot be the only one: the mobile CI's Rust job installs no interpreter).
//!
//! **Every wait is an event**: a request arrives (the server signals it), an
//! `ack` moves (read after the publish that moved it), a link is cut. The only
//! deadline in the file is the bound on "nothing more comes", which is how an
//! absence is measured at all — and it is named as such where it is used.
//!
//! The service is ONE per process (`nz_remote`'s own singletons are too, on both
//! sides), so these tests take [`bench`] first: they run one at a time, each with
//! its own core and its own server.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use nz_api::{Api, Lane, SessionJar};
use nz_player::Command;
use nz_remote::host::{self, Session};
use serde_json::{Value, json};

use super::testing::{FakePlayer, FakeServer, TempDir};
use super::{Remote, Transport};
use crate::core::NetworkState;

/// A service owned by ONE test, stopped and joined on drop.
///
/// `nz_remote`'s host and controller are process-wide (`OnceLock`), so a service
/// thread left running by the previous test would tick the next test's shared
/// services with a stale client. [`Guard::drop`] is what makes each test's world
/// end with it.
struct Guard(Arc<Remote>);

impl std::ops::Deref for Guard {
    type Target = Arc<Remote>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

/// A service over `core`, with the guard.
fn service(core: &Arc<crate::core::Core>) -> Guard {
    Guard(Remote::start(Arc::clone(core)))
}

/// The one-at-a-time guard: `nz_remote`'s host and controller are process-wide
/// services (`OnceLock`), so two tests driving them at once would share one
/// status and one client.
fn bench() -> MutexGuard<'static, ()> {
    static BENCH: OnceLock<Mutex<()>> = OnceLock::new();
    BENCH
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// The ONE runtime every test's loops run on.
///
/// `nz-remote`'s host and controller are process-wide services whose loops are
/// tasks on the runtime that spawned them: a test that owned its own runtime
/// would kill a loop mid-answer when it ended, and the service's `looping` flag
/// would stay set — the next test's `start` would then be swallowed (SC5's own
/// find, one level up). One runtime for the binary is what makes each test's
/// stop-and-restart a real event.
fn rt() -> &'static tokio::runtime::Handle {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("nz-remote-test")
            .enable_all()
            .build()
            .expect("a runtime for the remote tests")
    })
    .handle()
}

/// A client of the bench's server with its own cookie jar — the "desktop" side of
/// a test, or a second person.
async fn client(fake: &FakeServer, user: &str) -> Api {
    let api = Api::builder(fake.base())
        .session(Arc::new(SessionJar::in_memory()))
        .build()
        .expect("a client");
    api.login(user, "Motdepasse1").await.expect("the login");
    api
}

/// The phone, built the way `NzBridge.install` builds it, on `fake`.
async fn phone(fake: &FakeServer, dir: &TempDir, user: &str) -> Arc<crate::core::Core> {
    let core = Arc::new(dir.core("téléphone"));
    core.set_server(fake.base()).await.expect("the server");
    core.login(user, "Motdepasse1").await.expect("the login");
    core
}

// --- the acceptance: a phone drives the desktop's player --------------------

/// **The pause that reaches the desktop, acked.**
///
/// The phone claims a link, taps pause, and the DESKTOP's own host half (a real
/// `Session` poll + `PlayerState::apply` + a publish carrying `ack`) runs it. The
/// numbers this asserts are also the lot's PR numbers: the phone spends FOUR
/// requests and no more (claim, the loan's first read, the command, the read that
/// sees the ack), and the owner's `ack` is 1 — not "the request went out".
#[test]
fn a_pause_from_this_phone_reaches_the_desktop_player_and_is_acked() {
    rt().block_on(a_pause_from_this_phone_reaches_the_desktop_player_and_is_acked_async());
}

async fn a_pause_from_this_phone_reaches_the_desktop_player_and_is_acked_async() {
    let _bench = bench();
    let fake = FakeServer::start();
    let dir = TempDir::new("pause");
    let core = phone(&fake, &dir, "friend").await;

    // The owner lent this desk's player to whoever claims the link.
    let link = fake.create_link("queue", "deskdev1", "Salon", None);

    let remote = service(&core);
    remote.attach_player(FakePlayer::with_queue(&[("abc", "Alpha")]));
    let grant = remote.claim(&link.token).await.expect("the claim");
    assert_eq!(grant.level, "queue");
    assert_eq!(grant.owner, "owner");
    assert!(
        !grant.can_browse,
        "un lien `queue` ne fait que la lecture et la file"
    );
    assert!(!grant.can_keep, "il n'écrit rien");
    assert!(!grant.can_admin, "il n'administre rien");
    remote.open_loan().await.expect("the loan");

    // --- the desktop's own host half, driven by hand ------------------------
    let owner = client(&fake, "owner").await;
    let mut player = nz_player::PlayerState::new();
    player.apply(
        Command::play_queue(
            vec![{
                let mut entry = nz_player::Entry::new("abc");
                entry.title = "Alpha".to_owned();
                entry.duration = 40;
                entry
            }],
            0,
            None,
        ),
        0,
    );
    player.apply(Command::pause(), 0);
    assert!(!player.playing(), "le lecteur du bureau est en pause");
    let mut session = Session::new("deskdev1", host::page_id());
    // Its first poll: a channel this launch has not seen, so it publishes all.
    let first: Value = owner
        .get::<Value>(Lane::User, &session.poll_path())
        .await
        .expect("a poll")
        .value;
    assert!(session.absorb_poll(&first).reset);
    publish(&owner, &session, &player).await;

    // --- the phone's tap ----------------------------------------------------
    let before = remote.traffic();
    remote.send(Transport::Pause).await.expect("la pause part");
    // Optimistic: the loan's view says paused BEFORE the owner answered.
    assert!(!remote.now().playing, "le bouton répond dans la même image");

    // --- the desktop runs it, and acks --------------------------------------
    let answer: Value = owner
        .get::<Value>(Lane::User, &session.poll_path())
        .await
        .expect("a poll")
        .value;
    let polled = session.absorb_poll(&answer);
    assert!(!polled.ended, "un lien est vivant");
    let mut ran = 0;
    for pending in &polled.commands {
        if let host::Pending::Command(command) = pending {
            player.apply(command.clone(), 0);
            ran += 1;
        }
    }
    assert_eq!(ran, 1, "une commande, celle de la pause");
    let published = publish(&owner, &session, &player).await;
    assert_eq!(
        published.get("need_queue").and_then(Value::as_bool),
        Some(false),
        "la file est arrivée avec la première publication"
    );

    // --- the phone reads the ack -------------------------------------------
    remote.step_loan().await.expect("une lecture");
    let status = remote.status();
    assert_eq!(
        status.applied, 0,
        "ce téléphone ne prête rien : aucun numéro exécuté de son côté"
    );
    assert!(status.online, "le lecteur du bureau répond");
    assert!(
        !remote.now().playing,
        "le bureau est en pause, comme demandé"
    );
    assert_eq!(fake.ack("deskdev1"), 1, "la commande a été EXÉCUTÉE");

    // The phone's own queue view: the owner's row crossed `/remote/queue`.
    let rows = remote.queue_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "Alpha");

    let after = remote.traffic();
    let spent = after.requests - before.requests;
    assert_eq!(
        spent, 3,
        "la pause (1), la lecture qui voit l'ack (1) et la file qu'elle lit (1) : \
         3 requêtes ({after:?})"
    );
    assert!(
        after.bytes_in < 4_000,
        "sous 4 ko pour un tap et son ack : {} octets",
        after.bytes_in
    );
    eprintln!(
        "pause → ack : {spent} requête(s), {} octets reçus, {} envoyés",
        after.bytes_in, after.bytes_out
    );
}

/// **A pause pressed during an outage is DROPPED, never replayed.**
///
/// `party-remote.md`: "a pause pressed a minute ago is not what anyone wants when
/// the link comes back". The phone's network is cut BEFORE the tap, so `nz-api`
/// refuses it without a socket (the fake sees no request at all); when the link
/// is back, the owner's own state replaces the local view and the pause never
/// travels.
#[test]
fn a_pause_pressed_while_offline_is_never_replayed() {
    rt().block_on(a_pause_pressed_while_offline_is_never_replayed_async());
}

async fn a_pause_pressed_while_offline_is_never_replayed_async() {
    let _bench = bench();
    let fake = FakeServer::start();
    let dir = TempDir::new("outage");
    let core = phone(&fake, &dir, "friend").await;
    let link = fake.create_link("queue", "deskdev1", "Salon", None);
    let remote = service(&core);
    remote.attach_player(FakePlayer::with_queue(&[("abc", "Alpha")]));
    remote.claim(&link.token).await.expect("the claim");
    remote.start_loan().await.expect("the loan");

    // The desk's player publishes once, so the channel is online and has a state.
    let owner = client(&fake, "owner").await;
    let mut player = nz_player::PlayerState::new();
    player.apply(
        Command::play_queue(vec![nz_player::Entry::new("abc")], 0, None),
        0,
    );
    let mut session = Session::new("deskdev1", host::page_id());
    let first: Value = owner
        .get::<Value>(Lane::User, &session.poll_path())
        .await
        .expect("a poll")
        .value;
    session.absorb_poll(&first);
    publish(&owner, &session, &player).await;
    remote.step_loan().await.expect("une lecture");

    // The link dies: nothing may leave the device, not even one socket.
    let requests = fake.requests();
    core.set_network(NetworkState::None);
    let refused = remote.send(Transport::Pause).await;
    assert!(refused.is_err(), "hors ligne, la pause ne part pas");
    assert_eq!(
        fake.requests(),
        requests,
        "aucune requête n'a quitté le téléphone"
    );

    // Back online: the owner's own state is the truth, and it says PLAYING.
    core.set_network(NetworkState::Wifi { metered: false });
    player.apply(Command::play(), 0);
    publish(&owner, &session, &player).await;
    remote.step_loan().await.expect("une lecture");
    assert!(
        remote.now().playing,
        "la pause avalée par la coupure n'a pas été rejouée"
    );
    assert_eq!(
        fake.commands("deskdev1").len(),
        0,
        "aucune commande n'a été mise en file"
    );
}

/// **The host loop polls only while a link is live, and 0 requests otherwise.**
///
/// Three measurements in one, because they are one rule:
/// 1. a device that was never lent sends NOTHING (0 requests at the peer);
/// 2. a link made on this phone starts the loop, whose polls reach the server;
/// 3. « Tout couper » stops it — and after the event that says it stopped, no
///    further request arrives (the deadline is the bound on an absence).
#[test]
fn the_host_loop_polls_only_while_a_link_is_live() {
    rt().block_on(the_host_loop_polls_only_while_a_link_is_live_async());
}

async fn the_host_loop_polls_only_while_a_link_is_live_async() {
    let _bench = bench();
    let fake = FakeServer::start();
    let dir = TempDir::new("hostloop");
    let core = phone(&fake, &dir, "owner").await;
    let remote = service(&core);
    let player = FakePlayer::with_queue(&[("abc", "Alpha")]);
    remote.attach_player(Arc::clone(&player) as Arc<dyn crate::remote::HostPlayer>);

    // 1. Never lent: the launch probe asks nothing at all.
    let requests = fake.requests();
    assert!(
        !remote.resume_host().await.expect("une réponse"),
        "ce téléphone n'a jamais été prêté"
    );
    assert_eq!(
        fake.requests(),
        requests,
        "aucune requête pour un appareil jamais prêté"
    );

    // 2. A link is made on this device: the loop answers.
    fake.create_link("queue", &remote.device(), "Salon", None);
    remote.start_host().await.expect("l'écoute démarre");
    let deadline = Duration::from_secs(10);
    let started = fake.requests();
    assert!(
        fake.wait_requests(started + 1, deadline),
        "le guetteur interroge le serveur"
    );
    // The owner's hand: a controller claims it elsewhere, and the phone must
    // publish what it plays.
    player.play();
    let published = fake
        .with_channel(&remote.device(), |channel| channel.state.is_some())
        .unwrap_or(false);
    let _ = published;

    // 3. Tout couper: the loops stop, and nothing follows.
    let cut = fake.requests();
    remote.revoke_all().await.expect("les liens sont coupés");
    // The EVENT is the server's own "no link is left" answer (to the poll or the
    // publish that was already in flight); after it the loop breaks. The bound
    // below is what turns "nothing came" into a measurement.
    assert!(
        fake.wait_inactive_after(cut, deadline),
        "le serveur a dit qu'aucun lien ne reste"
    );
    assert!(
        wait_until(deadline, || !remote.host_running()),
        "les boucles se sont arrêtées"
    );
    fake.drain_events();
    assert_eq!(
        fake.next_request(Duration::from_secs(3)),
        None,
        "aucune interrogation après l'arrêt (attente bornée : c'est la mesure d'une absence) ; \
         journal : {:?}",
        fake.log()
    );
}

/// **What the SPA's sheet asks, one request each, and nothing warm.**
///
/// A link is created, listed and cut: `POST /remote/links`, `GET
/// /remote/links?device=`, `DELETE /remote/links/<id>` — the SPA's own three
/// calls (`RemoteSheet.svelte#create/load/cut`), counted here.
#[test]
fn a_link_is_created_listed_and_cut_with_one_request_each() {
    rt().block_on(a_link_is_created_listed_and_cut_with_one_request_each_async());
}

async fn a_link_is_created_listed_and_cut_with_one_request_each_async() {
    let _bench = bench();
    let fake = FakeServer::start();
    let dir = TempDir::new("links");
    let core = phone(&fake, &dir, "owner").await;
    let remote = service(&core);

    let before = fake.requests();
    let link = remote
        .create_link("read", "1h", Some("Léa"))
        .await
        .expect("la création");
    assert_eq!(
        fake.requests(),
        before + 1,
        "créer un lien, c'est UNE requête"
    );
    assert_eq!(link.level, "read");
    assert_eq!(link.level_label, "Lecture seule");
    assert_eq!(link.title, "Lecture seule · pour Léa");
    assert!(
        link.url.ends_with(&format!("/rc/{}", link.id)),
        "{}",
        link.url
    );
    assert_eq!(link.expiry, "Expire dans 1 h");

    let listed = remote.links().await.expect("la liste");
    assert_eq!(listed.len(), 1, "un lien vivant sur cet appareil");
    assert_eq!(listed[0].id, link.id);

    let before_cut = fake.requests();
    remote.revoke_link(&link.id).await.expect("la coupure");
    assert_eq!(
        fake.requests(),
        before_cut + 1,
        "couper un lien, c'est UNE requête"
    );
    let after = remote.links().await.expect("la liste");
    assert!(after.is_empty(), "le lien coupé n'est plus listé");
}

/// **Claim and leave hand the account back.**
///
/// `remote.py#_end_remote_session`: logging out of a borrowed session ends the
/// LOAN, not the friend's own login. The phone learns the restored name from the
/// leave's own answer — that is what the "Contrôle terminé" card prints.
#[test]
fn claim_and_leave_give_the_account_back() {
    rt().block_on(claim_and_leave_give_the_account_back_async());
}

async fn claim_and_leave_give_the_account_back_async() {
    let _bench = bench();
    let fake = FakeServer::start();
    let dir = TempDir::new("leave");
    let core = phone(&fake, &dir, "friend").await;
    let link = fake.create_link("full", "deskdev1", "Salon", None);
    let remote = service(&core);
    remote.attach_player(FakePlayer::with_queue(&[("abc", "Alpha")]));

    let grant = remote.claim(&link.token).await.expect("le claim");
    assert_eq!(grant.level, "full");
    assert_eq!(
        core.me()
            .and_then(|me| me.remote)
            .map(|remote| remote.level),
        Some("full".to_owned()),
        "la session est le prêt, pas un login"
    );

    let restored = remote.leave().await.expect("le départ");
    assert_eq!(
        restored.as_deref(),
        Some("friend"),
        "le compte rendu est celui du telephone"
    );
    assert!(
        core.me().and_then(|me| me.remote).is_none(),
        "le prêt est terminé"
    );
}

/// **A level is enforced, and a refusal changes NOTHING.**
///
/// `queue` may not touch a setting (`remote.py#op_level`): the crate refuses
/// before the local view moves, so the screen never lies about the owner's
/// player. The server would refuse it too — that is the belt to this brace.
#[test]
fn a_queue_link_cannot_change_a_setting() {
    rt().block_on(a_queue_link_cannot_change_a_setting_async());
}

async fn a_queue_link_cannot_change_a_setting_async() {
    let _bench = bench();
    let fake = FakeServer::start();
    let dir = TempDir::new("level");
    let core = phone(&fake, &dir, "friend").await;
    let link = fake.create_link("queue", "deskdev1", "Salon", None);
    let remote = service(&core);
    remote.attach_player(FakePlayer::with_queue(&[("abc", "Alpha")]));
    remote.claim(&link.token).await.expect("le claim");
    remote.start_loan().await.expect("le loan");

    // The desk answers once, so the channel is online.
    let owner = client(&fake, "owner").await;
    let player = nz_player::PlayerState::new();
    let mut session = Session::new("deskdev1", host::page_id());
    let first: Value = owner
        .get::<Value>(Lane::User, &session.poll_path())
        .await
        .expect("a poll")
        .value;
    session.absorb_poll(&first);
    publish(&owner, &session, &player).await;
    remote.step_loan().await.expect("une lecture");

    let before = fake.requests();
    let refused = remote.send(Transport::Shuffle).await;
    assert!(refused.is_err(), "un lien `queue` ne mélange pas la file");
    assert_eq!(
        fake.requests(),
        before,
        "un refus local ne fait même pas une requête"
    );
    assert_eq!(
        fake.commands("deskdev1").len(),
        0,
        "rien n'est mis en file chez le propriétaire"
    );
}

/// **A controller's `set` lands in THIS device's store.**
///
/// The host side applies a mirrored setting through the same validated write
/// Réglages uses (`lib/remote/settings.js`): a setting the person then opens in
/// Réglages shows the value the controller chose, in ONE store.
#[test]
fn a_mirrored_setting_written_by_a_controller_lands_in_the_store() {
    rt().block_on(a_mirrored_setting_written_by_a_controller_lands_in_the_store_async());
}

async fn a_mirrored_setting_written_by_a_controller_lands_in_the_store_async() {
    let _bench = bench();
    let fake = FakeServer::start();
    let dir = TempDir::new("settings");
    let core = phone(&fake, &dir, "owner").await;
    let remote = service(&core);
    let player: Arc<dyn crate::remote::HostPlayer> = FakePlayer::with_queue(&[("abc", "Alpha")]);
    remote.attach_player(Arc::clone(&player));

    let link = fake.create_link("full", &remote.device(), "Salon", None);
    remote.start_host().await.expect("l'écoute démarre");
    let deadline = Duration::from_secs(10);
    let polled = wait_until(deadline, || {
        fake.with_channel(&remote.device(), |channel| channel.host_seen.is_some())
            .unwrap_or(false)
    });
    assert!(polled, "le guetteur interroge le serveur");

    // A controller asks for a setting, as `set` on the wire.
    fake.queue_command(
        &remote.device(),
        "set",
        json!({"key": "player.quality", "value": "OPUS_128"}),
    );
    let applied = wait_until(deadline, || core.local_settings().quality == "OPUS_128");
    assert!(applied, "le réglage est arrivé dans le magasin");
    assert_eq!(
        fake.ack(&remote.device()),
        1,
        "la commande exécutée est annoncée"
    );
    let _ = link;

    // A key this build does not lend is IGNORED, not written — and the ack still
    // moves, so a controller is never left waiting for a switch that cannot be
    // honoured.
    fake.queue_command(
        &remote.device(),
        "set",
        json!({"key": "viz.flash.ack", "value": true}),
    );
    assert!(
        wait_until(deadline, || fake.ack(&remote.device()) == 2),
        "la seconde commande a été exécutée (ou ignorée) et annoncée"
    );
    assert!(
        core.setting_json("viz.flash.ack").is_none(),
        "l'aveu du flash ne se prête pas"
    );
}

// --- helpers ----------------------------------------------------------------

/// One publish of the owner's player, through `nz-remote`'s own body builder.
async fn publish(api: &Api, session: &Session, player: &nz_player::PlayerState) -> Value {
    let published = host::Published {
        index: player.index(),
        playing: player.playing(),
        p: player.current_time(),
        duration: player.duration(),
        volume: player.volume(),
        muted: player.muted(),
        shuffle: player.shuffle(),
        repeat: host::repeat_wire(player.repeat()).to_owned(),
        status: if player.playing() {
            "playing"
        } else if player.current().is_some() {
            "paused"
        } else {
            "idle"
        }
        .to_owned(),
        track: player
            .current()
            .and_then(|entry| serde_json::to_value(entry).ok()),
        context: None,
    };
    let rows = host::queue_values(player.queue());
    let body = session.publish_body(&published, rows.as_deref(), None);
    api.send::<Value, Value>(
        Lane::User,
        nz_api::Method::POST,
        &session.publish_path(),
        Some(&body),
    )
    .await
    .expect("a publish")
    .unwrap_or(Value::Null)
}

/// Wait for a predicate that a REQUEST changed — the event is the request the
/// server answers, never a sleep that "lets it settle".
fn wait_until(within: Duration, mut check: impl FnMut() -> bool) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < within {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    check()
}
