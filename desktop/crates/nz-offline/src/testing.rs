//! The test harness the rules of this crate (and the desktop's engine benches)
//! are proven with: one local server whose link has a speed, a scratch store, a
//! panic recorder.
//!
//! **Always compiled, not `cfg(test)`**: the desktop app's `wiring::engine`
//! reuses this one server and store rather than growing a second copy that
//! would drift (its own words), and a crate cannot export `cfg(test)` items to
//! the app's tests without a feature — a feature would mean editing the app's
//! manifest, which one lot owns. The module is inert: no thread starts until a
//! test calls [`TestServer::start`], and the linker drops what nothing calls.
//!
//! Nothing here needs an audio engine, so it stays out of the library's real
//! dependency list; the two benches that play real audio stayed in the desktop
//! app with its engine adapter.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nz_api::{Api, SessionJar};
use nz_cache::Cache;

use crate::service::Shared;
use crate::state::Choice;

// --- a local server whose link has a speed ----------------------------------

/// A token bucket shared by every connection: the "link". Without one, a
/// loopback test measures the kernel and the rule 1 criterion is unmeasurable.
pub struct Link {
    rate: f64,
    state: std::sync::Mutex<(Instant, f64)>,
}

impl Link {
    pub fn new(rate: Option<f64>) -> Arc<Link> {
        Arc::new(Link {
            rate: rate.unwrap_or(0.0),
            state: std::sync::Mutex::new((Instant::now(), 16.0 * 1024.0)),
        })
    }

    /// Wait until `bytes` may go on the wire.
    fn take(&self, bytes: usize) {
        if self.rate <= 0.0 {
            return;
        }
        loop {
            let wait = {
                let mut state = Shared::lock(&self.state);
                let now = Instant::now();
                let cap = 16.0 * 1024.0;
                state.1 =
                    (state.1 + now.duration_since(state.0).as_secs_f64() * self.rate).min(cap);
                state.0 = now;
                let need = bytes as f64;
                if state.1 >= need {
                    state.1 -= need;
                    return;
                }
                (need - state.1) / self.rate
            };
            std::thread::sleep(Duration::from_secs_f64(wait.max(0.001)));
        }
    }
}

/// One stream's body. A generated body costs NO heap: this bench shares the
/// process with other tests, and a bench that parks 20 MB of filler inflates
/// every memory assertion running beside it.
pub enum Payload {
    /// A generated pattern of `len` bytes.
    Filler { len: usize, seed: u8 },
    /// Real bytes (the FLAC an engine decodes).
    Bytes(Vec<u8>),
}

impl Payload {
    fn len(&self) -> usize {
        match self {
            Payload::Filler { len, .. } => *len,
            Payload::Bytes(bytes) => bytes.len(),
        }
    }
}

/// What the server answers for one track before it serves its body.
#[derive(Clone)]
pub struct Script {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

pub struct TestServer {
    base: String,
    /// Kept alive for the server thread's connections.
    pub link: Arc<Link>,
    stop: Arc<AtomicBool>,
    delivered: Arc<std::sync::Mutex<std::collections::HashMap<String, u64>>>,
    hits: Arc<std::sync::Mutex<std::collections::HashMap<String, usize>>>,
    scripts: Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<Script>>>>,
}

impl TestServer {
    /// `bodies` maps a stream id to its byte count (filled with a pattern).
    pub fn start(
        link: Arc<Link>,
        bodies: std::collections::HashMap<String, Payload>,
    ) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let delivered: Arc<std::sync::Mutex<std::collections::HashMap<String, u64>>> =
            Arc::default();
        let hits: Arc<std::sync::Mutex<std::collections::HashMap<String, usize>>> = Arc::default();
        let scripts: Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<Script>>>> =
            Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let bodies = Arc::new(bodies);
        let link_for_thread = Arc::clone(&link);
        let delivered_for_thread = Arc::clone(&delivered);
        let hits_for_thread = Arc::clone(&hits);
        let scripts_for_thread = Arc::clone(&scripts);
        let stop_for_thread = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("nz-test-server".to_owned())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else {
                        if stop_for_thread.load(Ordering::Acquire) {
                            break;
                        }
                        continue;
                    };
                    let bodies = Arc::clone(&bodies);
                    let link = Arc::clone(&link_for_thread);
                    let delivered = Arc::clone(&delivered_for_thread);
                    let hits = Arc::clone(&hits_for_thread);
                    let scripts = Arc::clone(&scripts_for_thread);
                    let stop = Arc::clone(&stop_for_thread);
                    std::thread::spawn(move || {
                        serve(stream, &bodies, &link, &delivered, &hits, &scripts, &stop);
                    });
                }
            })
            .expect("a server thread");
        TestServer {
            base,
            link,
            stop,
            delivered,
            hits,
            scripts,
        }
    }

    /// Script the reply for `id` (one shot, in order).
    pub fn script(&self, id: &str, script: Script) {
        Shared::lock(&self.scripts)
            .entry(id.to_owned())
            .or_default()
            .push(script);
    }

    pub fn hits(&self, id: &str) -> usize {
        Shared::lock(&self.hits).get(id).copied().unwrap_or(0)
    }

    /// How many bytes of `id`'s body have left the server: the proof a close
    /// bench needs that a body is IN FLIGHT, and the byte count a resume bench
    /// counts with.
    pub fn delivered(&self, id: &str) -> u64 {
        Shared::lock(&self.delivered).get(id).copied().unwrap_or(0)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn serve(
    mut stream: TcpStream,
    bodies: &std::collections::HashMap<String, Payload>,
    link: &Link,
    delivered: &std::sync::Mutex<std::collections::HashMap<String, u64>>,
    hits: &std::sync::Mutex<std::collections::HashMap<String, usize>>,
    scripts: &std::sync::Mutex<std::collections::HashMap<String, Vec<Script>>>,
    stop: &AtomicBool,
) {
    let mut buffer = [0u8; 4096];
    let read = stream.read(&mut buffer).unwrap_or(0);
    let request = String::from_utf8_lossy(&buffer[..read]).to_string();
    let path = request
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/")
        .to_owned();
    let id = path.rsplit('/').next().unwrap_or_default().to_owned();
    *Shared::lock(hits).entry(id.clone()).or_default() += 1;

    if path == "/api/version" {
        let body = r#"{"version":"9.9.9"}"#;
        let _ = stream.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        );
        return;
    }

    let scripted = Shared::lock(scripts)
        .get_mut(&id)
        .and_then(|queue| (!queue.is_empty()).then(|| queue.remove(0)));
    if let Some(script) = scripted {
        let headers: String = script
            .headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"))
            .collect();
        let _ = stream.write_all(
            format!(
                "HTTP/1.1 {} X\r\n{headers}Content-Length: {}\r\n\
                 Connection: close\r\n\r\n{}",
                script.status,
                script.body.len(),
                script.body
            )
            .as_bytes(),
        );
        return;
    }

    let Some(body) = bodies.get(&id) else {
        let _ = stream.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\nConnection: close\r\n\r\nno",
        );
        return;
    };

    let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: audio/ogg\r\nAccept-Ranges: none\r\n\
          Connection: close\r\n\r\n",
    );
    let len = body.len();
    let mut buffer = vec![0u8; 16 * 1024];
    let mut sent = 0usize;
    while sent < len {
        if stop.load(Ordering::Acquire) {
            return;
        }
        let take = (len - sent).min(buffer.len());
        let chunk = match body {
            Payload::Filler { seed, .. } => {
                for (index, byte) in buffer[..take].iter_mut().enumerate() {
                    *byte = seed.wrapping_add((sent + index) as u8);
                }
                &buffer[..take]
            }
            Payload::Bytes(bytes) => &bytes[sent..sent + take],
        };
        link.take(chunk.len());
        if stream.write_all(chunk).is_err() {
            return;
        }
        *Shared::lock(delivered).entry(id.clone()).or_default() += chunk.len() as u64;
        sent += take;
    }
}

// --- the scaffolding --------------------------------------------------------

pub struct Scratch(std::path::PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "neezer-nzoffline-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        Scratch(dir)
    }

    pub fn dir(&self) -> &std::path::PathBuf {
        &self.0
    }

    pub fn store(&self) -> Cache {
        Cache::open_with_cache_dir(self.0.join("data"), self.0.join("cache"))
            .expect("the local store")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("nz-test-net")
        .enable_all()
        .build()
        .expect("a runtime")
}

pub fn api_for(server: &TestServer, store: &Cache, scratch: &Scratch) -> Arc<Api> {
    let api = Api::builder(&server.base)
        .cache(Arc::new(store.clone()))
        .session(Arc::new(SessionJar::at(scratch.dir().join("session.json"))))
        .build()
        .expect("a client");
    api.set_owner(Some("42".to_owned()));
    Arc::new(api)
}

pub fn body_map(ids: &[&str], size: usize) -> std::collections::HashMap<String, Payload> {
    ids.iter()
        .enumerate()
        .map(|(index, id)| {
            (
                (*id).to_owned(),
                Payload::Filler {
                    len: size,
                    seed: index as u8,
                },
            )
        })
        .collect()
}

pub fn filler(size: usize, seed: u8) -> Payload {
    Payload::Filler { len: size, seed }
}

/// Run a heavy bench in a process of its OWN.
///
/// `cargo test` runs a binary's tests in threads of one process, and other
/// suites assert on memory or install a process-wide panic hook: a bench that
/// streams 200 tracks beside them makes those measurements flaky. A child
/// process keeps both numbers true, and the bench is still part of
/// `cargo test --workspace`.
pub fn in_its_own_process(name: &str) {
    let exe = std::env::current_exe().expect("the test binary");
    let status = std::process::Command::new(exe)
        .args([
            "--exact",
            name,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .status()
        .expect("the child test process");
    assert!(
        status.success(),
        "the isolated bench {name} failed — its output is above"
    );
}

/// Poll `done` until it says so or `timeout`: a test waits on the outcome, never
/// on a clock.
pub fn wait_for(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    done()
}

/// Record every thread panic of THIS process, so a test can prove there was
/// none.
///
/// A panicking worker prints its message and the suite still passes — that is
/// exactly how the `nz-bulk` shutdown panic reached master's CI log. A panic
/// hook is process-wide, so the test that installs one runs in a process of its
/// own ([`in_its_own_process`]); the default hook still runs, so the message is
/// not hidden from a person reading the log.
pub struct Panics(Arc<std::sync::Mutex<Vec<String>>>);

impl Panics {
    pub fn install() -> Panics {
        let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
        let previous = std::panic::take_hook();
        let recorded = Arc::clone(&seen);
        std::panic::set_hook(Box::new(move |info| {
            Shared::lock(&recorded).push(info.to_string());
            previous(info);
        }));
        Panics(seen)
    }

    /// Everything recorded since the last call.
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut *Shared::lock(&self.0))
    }
}

pub fn choices(ids: &[&str]) -> Vec<Choice> {
    ids.iter()
        .map(|id| Choice::new(*id, format!("Titre {id}")))
        .collect()
}
