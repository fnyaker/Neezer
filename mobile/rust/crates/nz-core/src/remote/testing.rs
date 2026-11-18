//! Test support for MB19: a loopback server that speaks the remote-control
//! routes, and a fake player to lend.
//!
//! **Why a fake and not only `webapp/test/remote/server.py`.** That fixture is
//! the reference (SC5 drives it, and this lot's numbers were taken against it),
//! but it needs an interpreter that can `import peewee, neezer` and the mobile
//! CI's Rust job installs no Python at all — so the acceptance must also run
//! against a server built here, in process, on a loopback port. Both speak the
//! same wire: the shapes below are `neezer/webui/remote.py`'s own, field for
//! field, including the two answers the protocol turns on (`ack`, the last
//! command the player RAN, and `reset`/`chan`, a channel the player has not seen).
//!
//! **Nothing waits on the clock.** The server counts every request and signals a
//! channel for each one, so a test waits on the EVENT it is about (a command
//! arrived, a poll was answered, a link was cut) — never on a `sleep`.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nz_player::{Command, Entry, PlayerState};

use crate::core::{Core, CoreConfig};
use crate::lists::Row;
use crate::remote::host::{HostNow, HostPlayer};

/// `remote.py#CMD_TTL`: a command the player picks up later than this is dropped.
pub const CMD_TTL: f64 = 8.0;
/// `remote.py#HOST_ONLINE`: seconds since the player's last poll before it reads
/// as offline.
pub const HOST_ONLINE: f64 = 10.0;
/// `remote.py#CONTROLLER_TTL`.
pub const CONTROLLER_TTL: f64 = 20.0;

/// One live link, as the server holds it.
#[derive(Clone, Debug)]
pub struct Link {
    pub id: String,
    pub token: String,
    pub level: String,
    pub label: String,
    pub device: String,
    pub device_name: String,
    /// Seconds of life, `None` for unlimited.
    pub ttl: Option<u64>,
    pub created: Instant,
    pub revoked: bool,
}

impl Link {
    fn alive(&self) -> bool {
        !self.revoked
            && self
                .ttl
                .is_none_or(|ttl| self.created.elapsed() < Duration::from_secs(ttl))
    }
}

/// One command a controller posted.
#[derive(Clone, Debug)]
pub struct Queued {
    pub seq: u64,
    pub op: String,
    pub args: serde_json::Value,
    pub link: String,
    pub at: Instant,
}

/// One controlled player's channel (`remote.py`'s `_Channel`).
#[derive(Debug, Default)]
pub struct Channel {
    pub boot: String,
    pub state: Option<serde_json::Value>,
    pub queue: Vec<serde_json::Value>,
    pub qv: String,
    pub settings: serde_json::Value,
    pub sv: String,
    pub cmds: VecDeque<Queued>,
    pub seq: u64,
    pub ack: u64,
    pub host_seen: Option<Instant>,
    pub controllers: HashMap<String, (String, Instant)>,
}

impl Channel {
    fn online(&self, now: Instant) -> bool {
        self.host_seen
            .is_some_and(|seen| now.duration_since(seen).as_secs_f64() < HOST_ONLINE)
    }
}

#[derive(Debug, Clone)]
struct Session {
    user: String,
    /// `Some((link, device, controller id))` while this session is a loan.
    rc: Option<(String, String, String)>,
    /// The account this session had before the loan.
    prev: Option<String>,
}

#[derive(Debug, Default)]
struct World {
    sessions: HashMap<String, Session>,
    links: Vec<Link>,
    channels: HashMap<String, Channel>, // device -> channel
    next_link: u64,
    next_seq: u64,
}

/// A loopback server that answers the remote-control routes.
pub struct FakeServer {
    base: String,
    world: Arc<Mutex<World>>,
    requests: Arc<AtomicU64>,
    events: Sender<u64>,
    receiver: Mutex<Receiver<u64>>,
    /// The request number of the last answer that said "no link is left" to a
    /// poll or a publish — the EVENT after which a host must send nothing.
    inactive: Arc<AtomicU64>,
    /// The last few request lines, for a test that has to name the one it did not
    /// expect.
    log: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeServer {
    pub fn start() -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let base = format!("http://{}", listener.local_addr().expect("its address"));
        listener
            .set_nonblocking(true)
            .expect("a non-blocking listener");
        let world = Arc::new(Mutex::new(World::default()));
        let requests = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let inactive = Arc::new(AtomicU64::new(0));
        let log = Arc::new(Mutex::new(Vec::new()));
        let world_for_thread = Arc::clone(&world);
        let counting = Arc::clone(&requests);
        let stopping = Arc::clone(&stop);
        let events = sender.clone();
        let inactive_for_thread = Arc::clone(&inactive);
        let log_for_thread = Arc::clone(&log);
        let thread = std::thread::Builder::new()
            .name("nz-remote-fake".to_owned())
            .spawn(move || {
                while !stopping.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let count = counting.fetch_add(1, Ordering::SeqCst) + 1;
                            let _ = events.send(count);
                            answer(
                                stream,
                                &world_for_thread,
                                &inactive_for_thread,
                                count,
                                &log_for_thread,
                            );
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            })
            .expect("the fake server's thread");
        FakeServer {
            base,
            world,
            requests,
            events: sender,
            receiver: Mutex::new(receiver),
            inactive,
            log,
            stop,
            thread: Some(thread),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// How many requests have reached this server — the acceptance's strongest
    /// count ("0 requests" is measured AT THE PEER, not inferred).
    pub fn requests(&self) -> u64 {
        self.requests.load(Ordering::SeqCst)
    }

    /// Wait for the next request, up to `within`. The EVENT is the arrival; the
    /// deadline is what bounds a test that asserts nothing more comes.
    pub fn next_request(&self, within: Duration) -> Option<u64> {
        lock(&self.receiver).recv_timeout(within).ok()
    }

    /// Wait until `count` requests have been seen in total.
    pub fn wait_requests(&self, count: u64, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if self.requests() >= count {
                return true;
            }
            if self.next_request(Duration::from_millis(50)).is_none() && self.requests() >= count {
                return true;
            }
            if Instant::now() >= deadline {
                break;
            }
        }
        self.requests() >= count
    }

    /// Mint a link directly, as `POST /api/remote/links` would — for a test that
    /// needs a link without going through the sheet.
    pub fn create_link(&self, level: &str, device: &str, label: &str, ttl: Option<u64>) -> Link {
        let mut world = lock(&self.world);
        world.next_link += 1;
        let id = format!("{:016}", world.next_link)
            .chars()
            .map(|c| if c == '0' { 'a' } else { c })
            .collect::<String>();
        let token = format!("{id}.{}", "b".repeat(24));
        let link = Link {
            id: id.clone(),
            token: token.clone(),
            level: level.to_owned(),
            label: label.to_owned(),
            device: device.to_owned(),
            device_name: "Linux · Neezer".to_owned(),
            ttl,
            created: Instant::now(),
            revoked: false,
        };
        world.links.push(link.clone());
        let _ = &self.events;
        link
    }

    /// What one device's channel holds (`None` before its first poll/publish).
    pub fn with_channel<T>(&self, device: &str, read: impl FnOnce(&Channel) -> T) -> Option<T> {
        let world = lock(&self.world);
        world.channels.get(device).map(read)
    }

    pub fn commands(&self, device: &str) -> Vec<Queued> {
        self.with_channel(device, |channel| channel.cmds.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn ack(&self, device: &str) -> u64 {
        self.with_channel(device, |channel| channel.ack)
            .unwrap_or(0)
    }

    /// Queue one command as `POST /remote/cmd` would have (after its own level
    /// and liveness checks): what a test uses to drive the HOST half without a
    /// second phone.
    pub fn queue_command(&self, device: &str, op: &str, args: serde_json::Value) {
        let mut world = lock(&self.world);
        let link = world
            .links
            .iter()
            .find(|link| link.device == device && link.alive())
            .map(|link| link.id.clone())
            .unwrap_or_default();
        let boot = world.next_link + 1;
        let channel = world.channels.entry(device.to_owned()).or_default();
        if channel.boot.is_empty() {
            channel.boot = format!("boot-{boot}");
        }
        channel.seq += 1;
        let seq = channel.seq;
        channel.cmds.push_back(Queued {
            seq,
            op: op.to_owned(),
            args,
            link,
            at: Instant::now(),
        });
    }

    /// The last requests, oldest first — what a failing "nothing more came"
    /// assertion prints so the stray request names itself.
    pub fn log(&self) -> Vec<String> {
        lock(&self.log).clone()
    }

    /// Wait until the server answered "no link is left" (a poll or a publish) to
    /// request number `after` or later — the event after which a host must send
    /// nothing more. `after` is the count read right before the cut, so an answer
    /// that predates it cannot be mistaken for it.
    pub fn wait_inactive_after(&self, after: u64, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if self.inactive.load(Ordering::SeqCst) >= after {
                return true;
            }
            let _ = self.next_request(Duration::from_millis(50));
        }
        self.inactive.load(Ordering::SeqCst) >= after
    }

    /// Forget the requests already signalled: what a test does before it asserts
    /// that nothing more comes, so a queued signal is never read as a new one.
    pub fn drain_events(&self) {
        while lock(&self.receiver).try_recv().is_ok() {}
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Wake the accept loop's poll without touching anybody else's process.
        let _ = TcpStream::connect(self.base.trim_start_matches("http://"));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The routes, on one blocking connection.
fn answer(
    stream: TcpStream,
    world: &Arc<Mutex<World>>,
    inactive: &Arc<AtomicU64>,
    count: u64,
    log: &Arc<Mutex<Vec<String>>>,
) {
    let _ = stream.set_nonblocking(false);
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(clone) => clone,
        Err(_) => return,
    });
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let mut length = 0_usize;
    let mut cookie = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            return;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();
            if name == "content-length" {
                length = value.parse().unwrap_or(0);
            } else if name == "cookie" {
                cookie = value.to_owned();
            }
        }
    }
    let mut body = vec![0_u8; length];
    if length > 0 && reader.read_exact(&mut body).is_err() {
        return;
    }
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);

    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_owned(), query.to_owned()),
        None => (target.clone(), String::new()),
    };
    let sid = cookie
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(name, _)| *name == "sid")
        .map(|(_, value)| value.to_owned());
    {
        let mut held = lock(log);
        held.push(format!("{count} {method} {path}"));
        if held.len() > 32 {
            held.remove(0);
        }
    }
    let mut world = lock(world);
    let response = route(&mut world, &method, &path, &query, sid.as_deref(), &body);
    if response.contains("\"active\":false") {
        inactive.store(count, Ordering::SeqCst);
    }
    drop(world);
    let mut stream = stream;
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// One route, as `neezer/webui/remote.py` answers it.
fn route(
    world: &mut World,
    method: &str,
    path: &str,
    query: &str,
    sid: Option<&str>,
    body: &serde_json::Value,
) -> String {
    let params: HashMap<String, String> = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    let path = path.trim_start_matches("/api");
    match (method, path) {
        ("GET", "/version") => json(200, None, &serde_json::json!({"version": "fake-1.0"})),
        ("POST", "/login") => {
            let user = body
                .get("username")
                .and_then(|value| value.as_str())
                .unwrap_or("owner")
                .to_owned();
            let sid = format!("sid-{user}");
            world.sessions.insert(
                sid.clone(),
                Session {
                    user: user.clone(),
                    rc: None,
                    prev: None,
                },
            );
            json(
                200,
                Some(&format!("sid={sid}; Path=/; HttpOnly")),
                &serde_json::json!({"user": user_json(&user, false), "remote": null}),
            )
        }
        ("POST", "/logout") => json(200, None, &serde_json::json!({"ok": true})),
        ("GET", "/me") => match session(world, sid) {
            Some(session) => {
                let remote = session.rc.as_ref().map(|(link, _, _)| {
                    let held = world.links.iter().find(|held| &held.id == link);
                    serde_json::json!({
                        "level": held.map(|held| held.level.clone()).unwrap_or_default(),
                        "owner": session.user,
                        "device": held.map(|held| held.device_name.clone()).unwrap_or_default(),
                        "label": held.map(|held| held.label.clone()).unwrap_or_default(),
                        "expires": null,
                    })
                });
                json(
                    200,
                    None,
                    &serde_json::json!({
                        "user": user_json(&session.user, admin(&session.user)),
                        "remote": remote,
                    }),
                )
            }
            None => json(401, None, &serde_json::json!({"error": "session ended"})),
        },
        ("GET", "/remote/links") => match session(world, sid) {
            Some(_session) => {
                let device = params.get("device");
                let rows: Vec<serde_json::Value> = world
                    .links
                    .iter()
                    .filter(|link| link.alive())
                    .filter(|link| device.is_none_or(|device| &link.device == device))
                    .map(|link| link_json(world, link))
                    .collect();
                json(
                    200,
                    None,
                    &serde_json::json!({"links": rows, "levels": ["queue", "read", "full", "admin"]}),
                )
            }
            None => json(401, None, &serde_json::json!({"error": "session ended"})),
        },
        ("POST", "/remote/links") => match session(world, sid) {
            Some(session) => {
                let level = body
                    .get("level")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_owned();
                if !["queue", "read", "full", "admin"].contains(&level.as_str()) {
                    return json(400, None, &serde_json::json!({"error": "unknown level"}));
                }
                if level == "admin" && !admin(&session.user) {
                    return json(
                        403,
                        None,
                        &serde_json::json!({"error": "only an administrator can lend administration"}),
                    );
                }
                let device = body
                    .get("device")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_owned();
                if !(8..=40).contains(&device.len()) {
                    return json(400, None, &serde_json::json!({"error": "invalid device"}));
                }
                world.next_link += 1;
                let id = format!("{:016}", world.next_link)
                    .chars()
                    .map(|c| if c == '0' { 'a' } else { c })
                    .collect::<String>();
                let token = format!("{id}.{}", "b".repeat(24));
                let link = Link {
                    id,
                    token,
                    level,
                    label: body
                        .get("label")
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .to_owned(),
                    device,
                    device_name: body
                        .get("device_name")
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .to_owned(),
                    ttl: body.get("ttl").and_then(|value| value.as_u64()),
                    created: Instant::now(),
                    revoked: false,
                };
                let answer = link_json(world, &link);
                world.links.push(link);
                json(200, None, &serde_json::json!({"link": answer}))
            }
            None => json(401, None, &serde_json::json!({"error": "session ended"})),
        },
        ("DELETE", "/remote/links") => match session(world, sid) {
            Some(_) => {
                let device = params.get("device").cloned();
                let mut cut = 0;
                for link in world.links.iter_mut() {
                    if link.alive() && device.as_ref().is_none_or(|device| &link.device == device) {
                        link.revoked = true;
                        cut += 1;
                    }
                }
                json(200, None, &serde_json::json!({"revoked": cut}))
            }
            None => json(401, None, &serde_json::json!({"error": "session ended"})),
        },
        _ if method == "DELETE" && path.starts_with("/remote/links/") => {
            match session(world, sid) {
                Some(_) => {
                    let id = path.trim_start_matches("/remote/links/").to_owned();
                    let mut cut = 0;
                    for link in world.links.iter_mut() {
                        if link.id == id && link.alive() {
                            link.revoked = true;
                            cut += 1;
                        }
                    }
                    json(200, None, &serde_json::json!({"revoked": cut}))
                }
                None => json(401, None, &serde_json::json!({"error": "session ended"})),
            }
        }
        _ if method == "GET" && path.starts_with("/remote/host/") && path.ends_with("/poll") => {
            let device = path
                .trim_start_matches("/remote/host/")
                .trim_end_matches("/poll")
                .to_owned();
            match session(world, sid) {
                Some(session) if live_for(world, &session.user, &device) => {
                    let since = params
                        .get("since")
                        .and_then(|value| value.parse::<u64>().ok())
                        .unwrap_or(0);
                    let chan = params.get("chan").cloned().unwrap_or_default();
                    let now = Instant::now();
                    let channel = world.channels.entry(device.clone()).or_default();
                    if channel.boot.is_empty() {
                        channel.boot = format!("boot-{}", world.next_link + 1);
                    }
                    channel.host_seen = Some(now);
                    let reset = chan != channel.boot;
                    let from = if reset { 0 } else { since };
                    let controllers: Vec<serde_json::Value> = channel
                        .controllers
                        .values()
                        .filter_map(|(link, seen)| {
                            if now.duration_since(*seen).as_secs_f64() > CONTROLLER_TTL {
                                return None;
                            }
                            world.links.iter().find(|held| &held.id == link).map(|held| {
                                serde_json::json!({"link": held.id, "level": held.level, "label": held.label})
                            })
                        })
                        .collect();
                    let cmds: Vec<serde_json::Value> = channel
                        .cmds
                        .iter()
                        .filter(|command| command.seq > from)
                        .filter(|command| now.duration_since(command.at).as_secs_f64() < CMD_TTL)
                        .map(|command| {
                            serde_json::json!({
                                "seq": command.seq,
                                "op": command.op,
                                "args": command.args,
                                "link": command.link,
                            })
                        })
                        .collect();
                    json(
                        200,
                        None,
                        &serde_json::json!({
                            "active": true,
                            "chan": channel.boot,
                            "reset": reset,
                            "seq": channel.seq,
                            "cmds": cmds,
                            "controllers": controllers,
                            "has_state": channel.state.is_some(),
                        }),
                    )
                }
                _ => json(200, None, &serde_json::json!({"active": false})),
            }
        }
        _ if method == "POST" && path.starts_with("/remote/host/") => {
            let device = path.trim_start_matches("/remote/host/").to_owned();
            match session(world, sid) {
                Some(session) if live_for(world, &session.user, &device) => {
                    publish(world, &device, body).to_string();
                    let channel = world
                        .channels
                        .get(&device)
                        .expect("the publish created the channel");
                    json(
                        200,
                        None,
                        &serde_json::json!({
                            "active": true,
                            "chan": channel.boot,
                            "need_queue": channel.qv != text(body, "qv"),
                            "need_settings": channel.sv != text(body, "sv"),
                        }),
                    )
                }
                _ => json(200, None, &serde_json::json!({"active": false})),
            }
        }
        ("POST", "/remote/claim") => {
            let token = text(body, "token");
            let Some(link) = world.links.iter().find(|link| link.token == token).cloned() else {
                return json(
                    404,
                    None,
                    &serde_json::json!({"error": "this link is no longer valid"}),
                );
            };
            if !link.alive() {
                return json(
                    404,
                    None,
                    &serde_json::json!({"error": "this link is no longer valid"}),
                );
            }
            let sid = sid.unwrap_or("anonymous").to_owned();
            let prev = world.sessions.get(&sid).map(|session| session.user.clone());
            world.sessions.insert(
                sid,
                Session {
                    user: "owner".to_owned(),
                    rc: Some((
                        link.id.clone(),
                        link.device.clone(),
                        format!("cid-{}", link.id),
                    )),
                    prev,
                },
            );
            json(
                200,
                None,
                &serde_json::json!({
                    "remote": {
                        "level": link.level,
                        "owner": "owner",
                        "device": link.device_name,
                        "label": link.label,
                    },
                    "user": user_json("owner", true),
                }),
            )
        }
        ("POST", "/remote/leave") => match session(world, sid) {
            Some(session) => {
                let restored = session.prev.clone();
                let sid = sid.unwrap_or_default().to_owned();
                match restored.clone() {
                    Some(user) => {
                        world.sessions.insert(
                            sid,
                            Session {
                                user: user.clone(),
                                rc: None,
                                prev: None,
                            },
                        );
                        json(
                            200,
                            None,
                            &serde_json::json!({"ok": true, "restored": user_json(&user, false)}),
                        )
                    }
                    None => {
                        world.sessions.insert(
                            sid,
                            Session {
                                user: session.user.clone(),
                                rc: None,
                                prev: None,
                            },
                        );
                        json(
                            200,
                            None,
                            &serde_json::json!({"ok": true, "restored": null}),
                        )
                    }
                }
            }
            None => json(
                200,
                None,
                &serde_json::json!({"ok": true, "restored": null}),
            ),
        },
        ("GET", "/remote/state") => {
            let Some(session) = session(world, sid) else {
                return json(401, None, &serde_json::json!({"error": "session ended"}));
            };
            let Some((link_id, device, cid)) = session.rc.clone() else {
                return json(
                    404,
                    None,
                    &serde_json::json!({"error": "not a remote session"}),
                );
            };
            let Some(link) = world.links.iter().find(|held| held.id == link_id).cloned() else {
                return json(401, None, &serde_json::json!({"error": "remote ended"}));
            };
            if !link.alive() {
                return json(
                    401,
                    None,
                    &serde_json::json!({"error": "remote ended", "restored": user_json("friend", false)}),
                );
            }
            let now = Instant::now();
            let channel = world.channels.entry(device.clone()).or_default();
            channel
                .controllers
                .insert(cid.clone(), (link_id.clone(), now));
            let sv = params.get("sv").cloned().unwrap_or_default();
            let mut answer = serde_json::json!({
                "remote": {"level": link.level, "label": link.label, "device": link.device_name},
                "online": channel.online(now),
                "state": channel.state,
                "qv": channel.qv,
                "sv": channel.sv,
                "ack": channel.ack,
                "seq": channel.seq,
                "now": now_ms() as f64,
            });
            if sv != channel.sv {
                answer["settings"] = channel.settings.clone();
            }
            json(200, None, &answer)
        }
        ("GET", "/remote/queue") => {
            let Some(session) = session(world, sid) else {
                return json(401, None, &serde_json::json!({"error": "session ended"}));
            };
            let Some((_, device, _)) = session.rc.clone() else {
                return json(
                    404,
                    None,
                    &serde_json::json!({"error": "not a remote session"}),
                );
            };
            let channel = world.channels.entry(device).or_default();
            json(
                200,
                None,
                &serde_json::json!({"qv": channel.qv, "queue": channel.queue}),
            )
        }
        ("POST", "/remote/cmd") => {
            let Some(session) = session(world, sid) else {
                return json(401, None, &serde_json::json!({"error": "session ended"}));
            };
            let Some((link_id, device, cid)) = session.rc.clone() else {
                return json(
                    404,
                    None,
                    &serde_json::json!({"error": "not a remote session"}),
                );
            };
            let Some(link) = world.links.iter().find(|held| held.id == link_id).cloned() else {
                return json(401, None, &serde_json::json!({"error": "remote ended"}));
            };
            if !link.alive() {
                return json(
                    401,
                    None,
                    &serde_json::json!({"error": "remote ended", "restored": user_json("friend", false)}),
                );
            }
            let op = text(body, "op");
            let args = body.get("args").cloned().unwrap_or(serde_json::Value::Null);
            let Some(need) = op_level(&op, &args) else {
                return json(400, None, &serde_json::json!({"error": "unknown command"}));
            };
            if rank(&link.level) < rank(need) {
                return json(
                    403,
                    None,
                    &serde_json::json!({"error": "not allowed", "level": link.level}),
                );
            }
            let now = Instant::now();
            let channel = world.channels.entry(device.clone()).or_default();
            if !channel.online(now) {
                return json(409, None, &serde_json::json!({"error": "player offline"}));
            }
            channel.controllers.insert(cid, (link_id.clone(), now));
            world.next_seq += 1;
            let seq = world.next_seq;
            let channel = world.channels.entry(device).or_default();
            channel.seq += 1;
            let seq_here = channel.seq;
            channel.cmds.push_back(Queued {
                seq: seq_here,
                op,
                args,
                link: link_id,
                at: now,
            });
            let _ = seq;
            json(200, None, &serde_json::json!({"seq": seq_here}))
        }
        _ => json(404, None, &serde_json::json!({"error": "not found"})),
    }
}

/// Store a publish, exactly as `remote_host_publish` does.
fn publish(world: &mut World, device: &str, body: &serde_json::Value) -> serde_json::Value {
    let now = Instant::now();
    let qv = text(body, "qv");
    let sv = text(body, "sv");
    let queue = body.get("queue").cloned();
    let settings = body.get("settings").cloned();
    let mut state = body
        .get("state")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let channel = world.channels.entry(device.to_owned()).or_default();
    if channel.boot.is_empty() {
        channel.boot = format!("boot-{}", world.next_link + 1);
    }
    channel.host_seen = Some(now);
    let seq = channel.seq;
    if let Some(queue) = queue.and_then(|queue| queue.as_array().cloned()) {
        channel.queue = queue;
        channel.qv = qv;
    }
    if let Some(settings) = settings.filter(|settings| settings.is_object()) {
        channel.settings = settings;
        channel.sv = sv;
    }
    state["qv"] = serde_json::Value::String(channel.qv.clone());
    // A FLOAT, as Flask's `jsonify` writes `server_ms()` (a Python float):
    // `1795027294444.0068`. The crate parses `t` with `as_i64`, and a test whose
    // server was nicer than the real one hid that.
    state["t"] = serde_json::Value::from(now_ms() as f64);
    channel.state = Some(state);
    let ack = body
        .get("ack")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    channel.ack = channel.ack.max(ack.min(seq));
    serde_json::json!({"active": true})
}

/// `remote.py#op_level`: what each command needs, `None` for an op this table
/// does not have.
fn op_level(op: &str, args: &serde_json::Value) -> Option<&'static str> {
    Some(match op {
        "play" | "pause" | "toggle" | "next" | "prev" | "seek" | "jump" | "volume" | "mute" => {
            "queue"
        }
        "shuffle" | "repeat" | "remove" | "move" | "clear" | "add" | "play_next" | "play_queue"
        | "shuffle_play" => "read",
        "set" => {
            let key = args
                .get("key")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if key.starts_with("viz.") {
                "read"
            } else {
                "full"
            }
        }
        _ => return None,
    })
}

fn rank(level: &str) -> u8 {
    match level {
        "queue" => 0,
        "read" => 1,
        "full" => 2,
        "admin" => 3,
        _ => 0,
    }
}

fn live_for(world: &World, user: &str, device: &str) -> bool {
    let _ = user;
    world
        .links
        .iter()
        .any(|link| link.device == device && link.alive())
}

fn session(world: &World, sid: Option<&str>) -> Option<Session> {
    sid.and_then(|sid| world.sessions.get(sid).cloned())
}

/// An ISO-8601 instant `ttl` seconds from now, as the server's `isoformat()`.
fn iso_after(ttl: Option<u64>) -> serde_json::Value {
    match ttl {
        None => serde_json::Value::Null,
        Some(ttl) => {
            let then = now_ms() / 1000 + ttl;
            serde_json::Value::String(format_iso(then as i64))
        }
    }
}

/// `days_from_civil` backwards: a UTC instant as `2026-11-18T21:04:05`.
fn format_iso(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// Howard Hinnant's `civil_from_days`, the inverse of `expiry_label`'s own.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn link_json(world: &World, link: &Link) -> serde_json::Value {
    let controllers = world
        .channels
        .get(&link.device)
        .map(|channel| {
            channel
                .controllers
                .values()
                .filter(|(id, seen)| {
                    id == &link.id && seen.elapsed().as_secs_f64() < CONTROLLER_TTL
                })
                .count()
        })
        .unwrap_or(0);
    serde_json::json!({
        "id": link.id,
        "token": link.token,
        "level": link.level,
        "label": link.label,
        "device": link.device,
        "device_name": link.device_name,
        "created": null,
        "expires": iso_after(link.ttl),
        "last_used": null,
        "controllers": controllers,
    })
}

fn user_json(name: &str, admin: bool) -> serde_json::Value {
    serde_json::json!({
        "id": name,
        "name": name,
        "admin": admin,
        "enabled": true,
        "access": {},
        "sources": [],
        "onboarding": null,
    })
}

fn admin(user: &str) -> bool {
    user == "owner" || user == "bench"
}

fn text(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_owned()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn json(status: u16, cookie: Option<&str>, body: &serde_json::Value) -> String {
    let body = body.to_string();
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Error",
    };
    let cookie = cookie
        .map(|cookie| format!("Set-Cookie: {cookie}\r\n"))
        .unwrap_or_default();
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n{cookie}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

// --- a player to lend, without a device ------------------------------------

/// A `HostPlayer` for the tests: the real `nz_player::PlayerState`, whose effects
/// are recorded rather than played — the desktop's own state machine, which is
/// what `nz-remote` is proven against.
pub struct FakePlayer {
    state: Mutex<PlayerState>,
    queue_version: AtomicU64,
    pub applied: Mutex<Vec<Command>>,
    pub effects: Mutex<Vec<nz_player::Effect>>,
}

impl FakePlayer {
    /// A player with `queue` loaded, paused at 0.
    pub fn with_queue(queue: &[(&str, &str)]) -> Arc<FakePlayer> {
        let entries: Vec<Entry> = queue
            .iter()
            .map(|(id, title)| {
                let mut entry = Entry::new(*id);
                entry.title = (*title).to_owned();
                entry.duration = 40;
                entry
            })
            .collect();
        let mut state = PlayerState::new();
        state.apply(Command::play_queue(entries, 0, None), 0);
        state.apply(Command::pause(), 0);
        Arc::new(FakePlayer {
            state: Mutex::new(state),
            queue_version: AtomicU64::new(1),
            applied: Mutex::new(Vec::new()),
            effects: Mutex::new(Vec::new()),
        })
    }

    /// Plays, as the owner's own button would (through the machine, so its
    /// effects are produced exactly as on a device).
    pub fn play(&self) {
        let mut state = lock(&self.state);
        let effects = state.apply(Command::play(), 0);
        lock(&self.effects).extend(effects);
    }
}

impl HostPlayer for FakePlayer {
    fn now(&self) -> HostNow {
        let state = lock(&self.state);
        let entry = state.current();
        HostNow {
            index: state.index(),
            playing: state.playing(),
            p: state.current_time(),
            duration: state.duration(),
            volume: state.volume(),
            muted: state.muted(),
            shuffle: state.shuffle(),
            repeat: crate::remote::repeat_wire(state.repeat()).to_owned(),
            status: if entry.is_none() {
                "idle".to_owned()
            } else if state.playing() {
                "playing".to_owned()
            } else {
                "paused".to_owned()
            },
            current: entry.map(|entry| row_of(entry, state.index().max(0) as u32)),
        }
    }

    fn queue_rows(&self) -> Vec<Row> {
        lock(&self.state)
            .queue()
            .iter()
            .enumerate()
            .map(|(position, entry)| row_of(entry, position as u32))
            .collect()
    }

    fn queue_version(&self) -> u64 {
        self.queue_version.load(Ordering::Acquire)
    }

    fn apply(&self, commands: &[crate::player::Command]) {
        for command in commands {
            let (wire, extra) = match command {
                crate::player::Command::Play => (Some(Command::play()), None),
                crate::player::Command::Pause => (Some(Command::pause()), None),
                crate::player::Command::Toggle => (Some(Command::toggle()), None),
                crate::player::Command::Next => (Some(Command::next()), None),
                crate::player::Command::Prev => (Some(Command::prev()), None),
                crate::player::Command::Seek { s } => (Some(Command::seek(*s)), None),
                crate::player::Command::Jump { index, .. } => {
                    (Some(Command::jump(*index as usize)), None)
                }
                crate::player::Command::Remove { index, .. } => {
                    (Some(Command::remove_at(*index as usize)), None)
                }
                crate::player::Command::Move { from, to } => {
                    (Some(Command::move_to(*from as usize, *to as usize)), None)
                }
                crate::player::Command::ClearUpcoming => (Some(Command::clear_upcoming()), None),
                crate::player::Command::SetShuffle { on } => (
                    None,
                    if *on {
                        Some(Command::toggle_shuffle())
                    } else {
                        None
                    },
                ),
                crate::player::Command::CycleRepeat => (Some(Command::cycle_repeat()), None),
                crate::player::Command::SetVolume { v } => {
                    (Some(Command::set_volume(f64::from(*v))), None)
                }
                crate::player::Command::ToggleMute => (Some(Command::toggle_mute()), None),
                _ => (None, None),
            };
            for command in wire.into_iter().chain(extra) {
                let mut state = lock(&self.state);
                let effects = state.apply(command.clone(), 0);
                drop(state);
                lock(&self.applied).push(command);
                lock(&self.effects).extend(effects);
                self.queue_version.fetch_add(1, Ordering::AcqRel);
            }
        }
    }
}

fn row_of(entry: &Entry, position: u32) -> Row {
    let artists = entry
        .artists
        .iter()
        .map(|artist| artist.name.clone())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    Row {
        key: entry.key().to_owned(),
        id: entry.reference().unwrap_or(entry.key()).to_owned(),
        title: entry.title.clone(),
        artists: if artists.is_empty() {
            entry.artist_name().to_owned()
        } else {
            artists
        },
        album: entry
            .album
            .as_ref()
            .map(|album| album.title.clone())
            .unwrap_or_default(),
        album_id: entry
            .album
            .as_ref()
            .and_then(|album| album.id.clone().or_else(|| album.deezer_id.clone())),
        duration_s: entry.duration,
        cover: entry.cover.clone(),
        explicit: false,
        liked: false,
        disliked: false,
        unavailable: false,
        downloaded: false,
        source: "deezer".to_owned(),
        position,
        played_at: 0,
        added_at: 0,
    }
}

// --- a throwaway data + cache directory ------------------------------------

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// One test's two roots, removed on drop (a few kB of SQLite).
pub struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let number = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "nz-remote-core-{}-{tag}-{number}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("data")).expect("data dir");
        std::fs::create_dir_all(path.join("cache")).expect("cache dir");
        TempDir { path }
    }

    /// A core over this directory, built the way `NzBridge.install` builds it.
    pub fn core(&self, device: &str) -> Core {
        Core::new(CoreConfig {
            data_dir: self.path.join("data"),
            cache_dir: self.path.join("cache"),
            app_version: "0.0.0-test".to_owned(),
            version_code: 2_000_001,
            sdk_int: 35,
            device_name: device.to_owned(),
        })
        .expect("a core over a throwaway directory")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
