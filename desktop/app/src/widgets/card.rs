//! The browse kit D8a's five screens compose from: a card, a section head, a
//! chip row, a skeleton, a failure and an empty state.
//!
//! They are in ONE file because they are one design language: the home's
//! shelves, the search's results, an artist's discography and an album's header
//! must not each invent their own spacing. Everything here reads the live
//! Omarchy theme through [`crate::theme`], uses no emoji and no hard-coded
//! colour, and is drawn with gpui-omarchy's primitives.

use std::path::PathBuf;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement,
    Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled, Window, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};
use nz_api::ApiError;
use serde::de::DeserializeOwned;

use crate::state::AppState;
use crate::theme;
use crate::widgets::cover::{self, Kind};

/// The cached body of one cacheable path, read once when a screen opens.
///
/// This is the SPA's `api.swr` first half (`lib/api.js`): a page the device has
/// already seen paints at once and the fresh answer corrects it. It costs ONE
/// SQLite read — **0.18 ms while another thread holds the write lock** (D3's own
/// measurement, WAL) — taken at construction, never on a paint path; the
/// alternative is a skeleton frame on every revisit, which rule 3 and D8a's
/// acceptance forbid.
pub fn cached<T: DeserializeOwned>(state: &AppState, path: &str) -> Option<T> {
    let cache = state.cache.as_ref()?;
    let owner = state.api.as_ref()?.owner()?;
    let body = cache.get(&owner, path)?;
    match serde_json::from_str(&body) {
        Ok(value) => Some(value),
        Err(error) => {
            // Rule 7: an older copy this build reads differently is dropped, not
            // painted or panicked on.
            tracing::debug!(%error, path, "copie hors ligne illisible");
            None
        }
    }
}

/// One id, percent-encoded exactly as `nz-api` encodes it (`encodeURIComponent`'s
/// unreserved set), so the path a screen caches is the path it requests: a cache
/// key that differed by one byte would never be found again.
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The path a person's credits page is cached under: the SPA's
/// `api.personCreditsPath`, so both clients hit the same route AND the same
/// cache entry shape.
pub fn person_credits_path(name: &str) -> String {
    format!("/credits/person?name={}", encode(name))
}

/// The `LoadFailed.svelte` equivalent for a screen: what failed, why, and what
/// the person can do (rule 3: a cause and « Réessayer », never a spinner).
pub fn failed<V: Render>(
    title: &str,
    cause: impl Into<SharedString>,
    cx: &mut Context<V>,
    retry: impl Fn(&mut V, &mut Window, &mut Context<V>) + Clone + 'static,
) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(rems(0.75))
        .p(rems(2.))
        .bg(theme::shell(cx))
        .font_family(theme::font(cx))
        .child(
            icon(IconName::TriangleAlert)
                .size(rems(1.75))
                .text_color(theme::danger(cx)),
        )
        .child(
            div()
                .text_size(rems(1.125))
                .font_weight(FontWeight::BOLD)
                .text_color(theme::text(cx))
                .child(title.to_owned()),
        )
        .child(
            div()
                .max_w(rems(34.))
                .text_center()
                .text_size(rems(0.8125))
                .text_color(theme::secondary(cx))
                .child(cause.into()),
        )
        .child(
            button("reload", "Réessayer", ButtonVariant::Primary, cx)
                .on_click(cx.listener(move |this, _, window, cx| retry(this, window, cx))),
        )
}

/// A screen that has nothing to show, and what to do about it.
pub fn empty<V: Render>(
    glyph: IconName,
    title: &str,
    body: &str,
    cx: &mut Context<V>,
    action: Option<AnyElement>,
) -> Div {
    let _ = cx;
    let mut surface = div()
        .flex()
        .flex_col()
        .items_start()
        .gap(rems(0.625))
        .p(rems(1.75))
        .border_1()
        .border_color(theme::border(cx))
        .bg(theme::chrome(cx))
        .max_w(rems(38.))
        .child(icon(glyph).size(rems(1.5)).text_color(theme::secondary(cx)))
        .child(
            div()
                .text_size(rems(1.))
                .font_weight(FontWeight::BOLD)
                .text_color(theme::text(cx))
                .child(title.to_owned()),
        )
        .child(
            div()
                .text_size(rems(0.8125))
                .text_color(theme::secondary(cx))
                .child(body.to_owned()),
        );
    if let Some(control) = action {
        surface = surface.child(control);
    }
    surface
}

/// What a section is: a title, an optional count, an optional trailing action
/// (`Tout afficher`, `Autres suggestions`).
pub struct Section<'a> {
    pub title: &'a str,
    pub count: Option<String>,
    pub action: Option<(&'a str, IconName)>,
}

impl<'a> Section<'a> {
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            count: None,
            action: None,
        }
    }

    pub fn count(mut self, count: usize) -> Self {
        self.count = Some(count.to_string());
        self
    }

    pub fn action(mut self, label: &'a str, glyph: IconName) -> Self {
        self.action = Some((label, glyph));
        self
    }

    /// Draw the head; the action's click is the caller's, because only it knows
    /// what "Tout afficher" means here.
    pub fn render<V: Render>(
        &self,
        cx: &mut Context<V>,
        on_action: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    ) -> Div {
        let mut head = div()
            .flex()
            .w_full()
            .items_center()
            .gap(rems(0.5))
            .pb(rems(0.5))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .text_size(rems(0.875))
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme::text(cx))
                            .child(SharedString::from(self.title)),
                    )
                    .when_some(self.count.clone(), |row, count| {
                        row.child(
                            div()
                                .text_size(rems(0.75))
                                .font_family(theme::mono(cx))
                                .text_color(theme::secondary(cx))
                                .child(count),
                        )
                    }),
            )
            .child(div().flex_1());
        if let Some((label, glyph)) = self.action {
            head = head.child(
                div()
                    .id(SharedString::from(format!("section-action-{}", self.title)))
                    .flex()
                    .items_center()
                    .gap(rems(0.25))
                    .h(rems(1.75))
                    .px(rems(0.5))
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .cursor_pointer()
                    .hover(|style| style.text_color(theme::text(cx)))
                    .on_click(cx.listener(move |this, _, window, cx| on_action(this, window, cx)))
                    .child(icon(glyph).size(rems(0.875)))
                    .child(SharedString::from(label)),
            );
        }
        head
    }
}

/// A tile: a sleeve, a title and one line under it. `kind` decides whether the
/// sleeve is round (an artist) or square.
#[allow(clippy::too_many_arguments)]
pub fn card<V: Render>(
    id: SharedString,
    title: &str,
    subtitle: &str,
    cover_file: Option<&PathBuf>,
    cover_url: Option<&str>,
    kind: Kind,
    cx: &mut Context<V>,
    on_open: impl Fn(&mut V, &mut Window, &mut Context<V>) + Clone + 'static,
    on_menu: impl Fn(&mut V, &mut Window, &mut Context<V>) + Clone + 'static,
    on_play: Option<impl Fn(&mut V, &mut Window, &mut Context<V>) + Clone + 'static>,
) -> AnyElement {
    let art_size = px(148.);
    let open = on_open.clone();
    let mut tile = div()
        .id(SharedString::from(format!("card-{id}")))
        .w(px(164.))
        .flex()
        .flex_col()
        .gap(rems(0.5))
        .p(rems(0.5))
        .cursor_pointer()
        .hover(|style| style.bg(theme::chrome(cx)))
        .on_click(cx.listener(move |this, _, window, cx| open(this, window, cx)))
        .on_mouse_down(gpui_kit::MouseButton::Right, {
            let on_menu = on_menu.clone();
            cx.listener(move |this, _, window, cx| {
                on_menu(this, window, cx);
            })
        })
        .child(
            div()
                .relative()
                .child(cover::view(
                    cover_file, cover_url, title, kind, art_size, cx,
                ))
                .when_some(on_play, |art, play| {
                    art.child(
                        div()
                            .id(SharedString::from(format!("card-play-{id}")))
                            .absolute()
                            .right(px(6.))
                            .bottom(px(6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(rems(2.))
                            .h(rems(2.))
                            .bg(theme::chrome(cx))
                            .border_1()
                            .border_color(theme::border(cx))
                            .cursor_pointer()
                            .hover(|style| style.bg(theme::selected(cx)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                play(this, window, cx);
                            }))
                            .child(
                                icon(IconName::Play)
                                    .size(rems(1.))
                                    .text_color(theme::text(cx)),
                            ),
                    )
                }),
        )
        .child(
            div()
                .truncate()
                .text_size(rems(0.8125))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text(cx))
                .child(title.to_owned()),
        );
    if !subtitle.is_empty() {
        tile = tile.child(
            div()
                .truncate()
                .text_size(rems(0.6875))
                .text_color(theme::secondary(cx))
                .child(subtitle.to_owned()),
        );
    }
    tile.into_any_element()
}

/// A chip: the pill a source switch, a tab or a role filter is.
pub fn chip<V: Render>(
    id: SharedString,
    label: &str,
    active: bool,
    cx: &mut Context<V>,
    on_click: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
) -> AnyElement {
    div()
        .id(SharedString::from(format!("chip-{id}")))
        .flex()
        .items_center()
        .gap(rems(0.25))
        .h(rems(2.))
        .px(rems(0.75))
        .border_1()
        .border_color(if active {
            theme::accent(cx)
        } else {
            theme::border(cx)
        })
        .bg(if active {
            theme::selected(cx)
        } else {
            theme::chrome(cx)
        })
        .text_size(rems(0.75))
        .text_color(if active {
            theme::text(cx)
        } else {
            theme::secondary(cx)
        })
        .cursor_pointer()
        .hover(|style| style.text_color(theme::text(cx)))
        .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
        .child(label.to_owned())
        .into_any_element()
}

/// One dimension: the hero's sleeve or a page's, scaled by role.
pub fn hero_sleeve() -> Pixels {
    px(180.)
}

/// The share of a page a TAIL may take before it scrolls on its own: the
/// provider's half under an album's tracks, the suggestions under a playlist.
///
/// A tail is a SECONDARY list. Unbounded it is a page whose own list is
/// squeezed to nothing: measured — a 12-row « Titres recommandés » strip took
/// the whole 800 px page and left a 4 000-row playlist **0 px** tall, so the
/// playlist could not be seen, let alone scrolled (v1.6.1).
pub const TAIL_SHARE: f32 = 0.4;

/// A neutral band behind a header, the same on the artist, the album and the
/// credits pages (`§6`'s "a plain band under a hairline, not a gradient").
pub fn hero_band<V: Render>(cx: &mut Context<V>) -> Div {
    div()
        .w_full()
        .flex()
        .items_end()
        .gap(rems(1.75))
        .p(rems(1.75))
        .border_b_1()
        .border_color(theme::border(cx))
        .bg(theme::chrome(cx))
}

/// The little facts line a header carries (`Artiste · 42 titres · 3 albums`).
pub fn facts(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The cause a screen prints for an `ApiError`, in the app's French.
pub fn cause(error: &ApiError) -> SharedString {
    error.cause_fr().into()
}

/// A screen's own error line (a row action that failed), as the toast API is the
/// shell's. `retry` is the gesture rule 3 asks for, offered by the caller.
pub fn inline_error<V: Render>(
    message: impl Into<SharedString>,
    cx: &mut Context<V>,
    on_retry: impl Fn(&mut V, &mut Window, &mut Context<V>) + Clone + 'static,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(rems(0.5))
        .p(rems(0.5))
        .border_1()
        .border_color(theme::danger(cx))
        .bg(theme::chrome(cx))
        .text_size(rems(0.75))
        .text_color(theme::danger(cx))
        .child(icon(IconName::CircleAlert).size(rems(1.)))
        .child(div().flex_1().child(message.into()))
        .child(
            button("retry", "Réessayer", ButtonVariant::Outline, cx)
                .on_click(cx.listener(move |this, _, window, cx| on_retry(this, window, cx))),
        )
}

/// A row of skeletons, so a shelf that is loading reserves its height instead
/// of pushing the page around when the answer lands.
pub fn skeleton(count: usize, height: Pixels, cx: &App) -> Div {
    let mut row = div().flex().w_full().gap(rems(0.75));
    for _ in 0..count {
        row = row.child(
            div()
                .flex_shrink_0()
                .w(px(164.))
                .h(height)
                .bg(theme::chrome(cx)),
        );
    }
    row
}

/// A chip row that wraps: sources, tabs, roles.
pub fn chip_row(children: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(rems(0.375))
        .children(children)
}

/// A shelf of cards: they wrap onto the next line rather than hiding behind a
/// sideways scroller, because a desktop window is wide and a mouse's wheel does
/// not scroll sideways.
pub fn shelf(children: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .flex_wrap()
        .w_full()
        .gap(rems(0.25))
        .children(children)
}

/// The state a screen's data is in, in one place so the three states cannot
/// drift: loading paints a skeleton, failed paints the cause and « Réessayer »,
/// ready paints the page.
#[derive(Clone, Debug, Default)]
pub enum Phase<T> {
    #[default]
    Loading,
    Ready(T),
    Failed(ApiError),
}

impl<T> Phase<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Phase::Ready(value) => Some(value),
            _ => None,
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Phase::Loading)
    }
}

/// A tiny HTTP server for the browse screens' tests: real sockets, real
/// requests, and a count of each path, which is how "one request per settled
/// term" and "the artist page asks nothing before it paints" become assertions
/// instead of claims.
#[cfg(test)]
pub mod stub {
    use gpui_kit::AppContext as _;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// One answered request: the path without its query, and what was asked.
    #[derive(Clone, Debug)]
    pub struct Hit {
        pub path: String,
        pub query: String,
    }

    struct Inner {
        routes: HashMap<String, Vec<u8>>,
        hits: Vec<Hit>,
        closed: AtomicBool,
    }

    /// The server: `Stub::start` gives it answers per path prefix.
    pub struct Stub {
        base: String,
        inner: Arc<Mutex<Inner>>,
    }

    impl Stub {
        /// `routes` maps a path (`/api/search`, `/api/album/1`) to its JSON body.
        /// A request no route answers is `404` with `{}`.
        pub fn start(routes: &[(&str, &str)]) -> Self {
            Self::start_bytes(
                &routes
                    .iter()
                    .map(|(path, body)| (*path, body.as_bytes().to_vec()))
                    .collect::<Vec<_>>(),
            )
        }

        /// The same server, with raw bodies: a sleeve is a JPEG, not a string
        /// (`/api/cover/<album>` answers the bytes the person's device saves).
        pub fn start_bytes(routes: &[(&str, Vec<u8>)]) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let address = listener.local_addr().expect("address");
            let mut table = HashMap::new();
            for (path, body) in routes {
                table.insert((*path).to_owned(), body.clone());
            }
            let inner = Arc::new(Mutex::new(Inner {
                routes: table,
                hits: Vec::new(),
                closed: AtomicBool::new(false),
            }));
            let served = Arc::clone(&inner);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    if served.lock().expect("stub").closed.load(Ordering::Relaxed) {
                        break;
                    }
                    answer(stream, &served);
                }
            });
            Self {
                base: format!("http://{address}"),
                inner,
            }
        }

        pub fn base(&self) -> String {
            self.base.clone()
        }

        /// Every path asked for, in order.
        pub fn hits(&self) -> Vec<Hit> {
            self.inner.lock().expect("stub").hits.clone()
        }

        /// How many times a path (without its query) was asked for.
        pub fn count(&self, path: &str) -> usize {
            self.hits().iter().filter(|hit| hit.path == path).count()
        }

        /// The queries a path was asked with.
        pub fn queries(&self, path: &str) -> Vec<String> {
            self.hits()
                .into_iter()
                .filter(|hit| hit.path == path)
                .map(|hit| hit.query)
                .collect()
        }

        pub fn total(&self) -> usize {
            self.hits().len()
        }

        pub fn stop(&self) {
            self.inner
                .lock()
                .expect("stub")
                .closed
                .store(true, Ordering::Relaxed);
            // One connection wakes the accept loop so it can see the flag.
            let _ = TcpStream::connect(self.base.trim_start_matches("http://"));
        }
    }

    /// Run one test in a CHILD process, and give back what it printed.
    ///
    /// Some tests hold megabytes on purpose (a 4 000-row list is ~12 MB) and
    /// another test in this binary asserts on the RSS delta of a screen: two
    /// threads measuring one process's memory cannot both be right — measured on
    /// CI, the settings test read 8.4 MB against its 2 MB budget while a list was
    /// being built in a sibling thread. `NZ_TEST_CHILD` marks the child, which
    /// runs the body itself.
    pub fn in_child(test_path: &str) -> Option<String> {
        if std::env::var_os("NZ_TEST_CHILD").is_some() {
            return None;
        }
        let exe = std::env::current_exe().expect("the test binary");
        let output = std::process::Command::new(exe)
            .args(["--exact", test_path, "--nocapture", "--test-threads=1"])
            .env("NZ_TEST_CHILD", "1")
            .output()
            .expect("a child test process");
        let printed = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success(), "the child test failed:\n{printed}");
        Some(printed)
    }

    /// A logged-in app pointed at `stub`, and the runtime the test must drive.
    ///
    /// The runtime is CURRENT-THREAD and the test thread pumps it: a real tokio
    /// worker may not wake a GPUI task, or gpui-pre's deterministic scheduler
    /// panics (D7's pitfall). The deadline only turns a hang into a failure.
    pub fn logged_in(
        cx: &mut gpui::TestAppContext,
        stub: &Stub,
    ) -> (
        gpui_kit::Entity<crate::state::AppState>,
        tokio::runtime::Runtime,
    ) {
        use crate::state::{AppConfig, AppState, event_channel};
        let dir = std::env::temp_dir().join(format!(
            "neezer-d8a-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        cx.update(|cx| {
            gpui_omarchy::init(cx);
            crate::keymap::install(cx);
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (tx, rx) = event_channel();
        let config = AppConfig {
            server: Some(stub.base()),
            start_audio: false,
            start_mpris: false,
            ..AppConfig::for_test(&dir)
        };
        let state = {
            let runtime = runtime.handle().clone();
            cx.update(|cx| cx.new(move |_| AppState::new(config, runtime, tx, None)))
        };
        // The shell drains the app's event channel in a foreground task; a screen
        // test has no shell, so the harness drains it here — the same call, so
        // what a test exercises is what the app runs.
        {
            let state = state.clone();
            cx.update(move |cx| {
                cx.spawn(async move |cx| {
                    while let Some(event) = rx.next().await {
                        state.update(cx, |state, cx| state.on_event(event, cx));
                    }
                })
                .detach();
            });
        }
        cx.update(|cx| {
            state.update(cx, |state, _| {
                state.login(stub.base(), "bench".into(), "Bench1".into())
            })
        });
        // The session must be the one the screen asks with: a screen built before
        // the login would request with no owner and its answer would not be kept
        // for the device (the cache is keyed by account).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            runtime.block_on(async {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            });
            cx.run_until_parked();
            if cx.update(|cx| state.read(cx).auth.is_logged_in()) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the session did not arrive"
            );
        }
        (state, runtime)
    }

    /// Pump the network and the UI until `done` holds, bounded.
    pub fn until(
        runtime: &tokio::runtime::Runtime,
        cx: &mut gpui::VisualTestContext,
        what: &str,
        mut done: impl FnMut(&gpui_kit::App) -> bool,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            runtime.block_on(async {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            });
            cx.run_until_parked();
            if cx.update(|_, cx| done(cx)) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
        }
    }

    fn answer(mut stream: TcpStream, inner: &Arc<Mutex<Inner>>) {
        let mut buffer = [0u8; 8192];
        let read = stream.read(&mut buffer).unwrap_or(0);
        if read == 0 {
            return;
        }
        let request = String::from_utf8_lossy(&buffer[..read]).to_string();
        let first = request.lines().next().unwrap_or_default();
        let target = first.split_whitespace().nth(1).unwrap_or("/");
        let (path, query) = match target.split_once('?') {
            Some((path, query)) => (path.to_owned(), query.to_owned()),
            None => (target.to_owned(), String::new()),
        };
        let body = {
            let mut inner = inner.lock().expect("stub");
            inner.hits.push(Hit {
                path: path.clone(),
                query: query.clone(),
            });
            inner.routes.get(&path).cloned()
        };
        let (status, body) = match body {
            Some(body) => ("200 OK", body),
            None => ("404 Not Found", b"{}".to_vec()),
        };
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {}\r\n\
             Set-Cookie: session=stub; Path=/\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n",
            if path.starts_with("/api/cover/") {
                "image/jpeg"
            } else {
                "application/json"
            },
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(&body);
        let _ = stream.write_all(&response);
        let _ = stream.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The path a screen caches must be the path it requests: one byte apart and
    /// the offline copy is never found again.
    #[test]
    fn an_id_is_encoded_like_nz_api_encodes_it() {
        assert_eq!(encode("3000168"), "3000168");
        assert_eq!(encode("a b"), "a%20b");
        assert_eq!(encode("é"), "%C3%A9");
        assert_eq!(encode("a/b"), "a%2Fb");
        assert_eq!(encode("it's"), "it's");
        assert_eq!(
            person_credits_path("Camille"),
            "/credits/person?name=Camille"
        );
        assert_eq!(
            person_credits_path("Étienne Daho"),
            "/credits/person?name=%C3%89tienne%20Daho"
        );
    }

    #[test]
    fn a_facts_line_drops_the_empty_parts() {
        assert_eq!(
            facts(&[
                "Album".to_owned(),
                String::new(),
                "12 titres".to_owned(),
                "48:12".to_owned()
            ]),
            "Album · 12 titres · 48:12"
        );
    }
}
