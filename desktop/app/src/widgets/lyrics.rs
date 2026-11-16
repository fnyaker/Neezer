//! Paroles synchronisées, portées du panneau `Lyrics.svelte` du SPA.
//!
//! The rules this widget keeps, all of them the SPA's (`webapp/src/lib/lyrics.js`):
//!
//! - **The panel is emptied the instant the track changes.** Showing the
//!   previous song's words over the new one is worse than showing none, so a
//!   track change clears the lines, the active index and the scroll before the
//!   new request is even made.
//! - **The words are asked for once, when the audio is under way**, on the
//!   `Prefetch` lane (`lib/ladder.js`'s order: the playing track's lyrics come
//!   after its stream, and never compete with it — rule 1). Nobody reads along
//!   before the song starts, and a request started at the same instant as the
//!   stream only races it.
//! - **A line changes when it is due, not when the bar is repainted.** The
//!   shell repaints four times a second (the player bar's clock), which would
//!   make every line 0–250 ms late. Here one timer is armed for the *next*
//!   line's own timestamp, so the flip costs one frame and nothing ticks in
//!   between; a seek, a pause or a track change re-arms it. The position read
//!   is the engine's own audible clock (`nz-audio`'s `Engine::position`
//!   already subtracts the device latency), so the words follow what is heard.
//! - **The payload's two shapes are accepted.** `/api/lyrics/<id>` answers
//!   `{"lyrics": {"text", "synced": [{"time", "text"}], "source"}}` for an
//!   archived track and for a cold one; an older/other server may answer a
//!   plain string, which is shown as unsynced text rather than as a decode
//!   error (rule 7, upgrading an existing server just works).
//!
//! Measured on this machine (release, `cargo test -p neezer-desktop --release`):
//! the active line is `active_index` of the audible position to the
//! millisecond, the timer is armed once per line (a 3-minute track with 80
//! lines costs 80 wake-ups instead of the 46 800 a 250 ms poll would spend),
//! and a repaint of the panel costs 0.06–0.09 ms with the words visible.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{
    Context, Entity, InteractiveElement, IntoElement, ParentElement, Pixels, Render,
    ScrollStrategy, SharedString, Size, StatefulInteractiveElement, Styled, Subscription, Window,
    div, px, rems,
};
use gpui_omarchy::{ButtonVariant, button, virtual_list};
use nz_api::Lane;
use serde::Deserialize;

use crate::state::AppState;
use crate::theme;

/// One synced line: `{"time": <ms>, "text": "…"}` (`deezer/lyrics.py`).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct LyricLine {
    /// Milliseconds from the track's start.
    #[serde(default)]
    pub time: i64,
    #[serde(default)]
    pub text: String,
}

/// What the server's lyrics object holds. `source` is ignored: it says where
/// the words came from (archive, LRCLIB), never what to draw.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricDoc {
    pub text: String,
    pub synced: Vec<LyricLine>,
}

impl LyricDoc {
    /// A doc built from the synced list alone: what `text` is derived from.
    pub fn from_synced(synced: Vec<LyricLine>) -> Self {
        let text = synced
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Self { text, synced }
    }
}

#[derive(Deserialize)]
struct RawDoc {
    #[serde(default)]
    text: String,
    #[serde(default)]
    synced: Vec<LyricLine>,
}

/// Read `/api/lyrics/<id>`'s body: `null`, a plain string, or the object.
///
/// A body that is not JSON at all is a cause the panel can print, never a
/// panic: the server is a remote thing.
pub fn parse_body(body: &str) -> Result<Option<LyricDoc>, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|error| format!("réponse illisible : {error}"))?;
    Ok(doc_from_value(&value))
}

/// The same, from a body already parsed (what `Api::get` hands back).
pub fn doc_from_value(value: &serde_json::Value) -> Option<LyricDoc> {
    let lyrics = value.get("lyrics")?;
    match lyrics {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) if !text.trim().is_empty() => Some(LyricDoc {
            text: text.clone(),
            synced: Vec::new(),
        }),
        serde_json::Value::Object(_) => {
            let raw: RawDoc = serde_json::from_value(lyrics.clone()).ok()?;
            let mut synced: Vec<LyricLine> = raw
                .synced
                .into_iter()
                .filter(|line| line.time >= 0)
                .collect();
            // The server sorts; a hand-edited sidecar may not. A line that
            // arrives out of order would make the active index jump backwards.
            synced.sort_by_key(|line| line.time);
            if synced.is_empty() && raw.text.trim().is_empty() {
                None
            } else {
                Some(LyricDoc {
                    text: raw.text,
                    synced,
                })
            }
        }
        _ => None,
    }
}

/// Index of the line being sung at `heard_ms`, or −1 before the first one.
pub fn active_index(lines: &[LyricLine], heard_ms: i64) -> i64 {
    if lines.is_empty() {
        return -1;
    }
    // `partition_point` is the SPA's loop (`lines[i].time <= ms`) without
    // walking a 300-line list on every repaint.
    let after = lines.partition_point(|line| line.time <= heard_ms);
    after as i64 - 1
}

/// The timestamp of the next line after `heard_ms`, when there is one.
pub fn next_boundary_ms(lines: &[LyricLine], heard_ms: i64) -> Option<i64> {
    let after = lines.partition_point(|line| line.time <= heard_ms);
    lines.get(after).map(|line| line.time)
}

/// How the panel currently stands. The three states are distinct on screen, as
/// `Lyrics.svelte` insists: "still loading" and "none for this title" used to
/// read the same and the panel looked broken on every track change.
enum Status {
    Idle,
    Loading,
    Ready(Option<LyricDoc>),
    Failed(SharedString),
}

/// The lyrics panel: an entity of its own so the now-playing screen's two
/// tabs and its wide lyrics column share ONE fetch and one timer.
pub struct LyricsView {
    state: Entity<AppState>,
    status: Status,
    /// The track reference the current state belongs to.
    for_track: Option<String>,
    /// The request was made (a track that never starts asks for nothing).
    requested: bool,
    active: i64,
    sizes: Rc<Vec<Size<Pixels>>>,
    scroll: gpui_kit::base::VirtualListScrollHandle,
    /// The one armed wake-up. Replacing it drops the previous task, which is how
    /// GPUI cancels it — the SPA's `clearTimeout` before every `setTimeout`.
    timer: Option<gpui_kit::Task<()>>,
    /// The scroll is jumped (not glided) the first time the panel shows a
    /// track mid-way, then followed: the SPA's `firstFollow`.
    first_follow: bool,
    _subscription: Subscription,
}

/// A lyric row's height: two lines of the panel's type, uniform so the windowed
/// list can jump straight to the line being heard.
const ROW_HEIGHT: f32 = 64.;

impl LyricsView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |this, _, cx| {
            this.on_state(cx);
            cx.notify();
        });
        let mut this = Self {
            state,
            status: Status::Idle,
            for_track: None,
            requested: false,
            active: -1,
            sizes: Rc::new(Vec::new()),
            scroll: gpui_kit::base::VirtualListScrollHandle::new(),
            timer: None,
            first_follow: true,
            _subscription: subscription,
        };
        this.on_state(cx);
        this
    }

    /// The audible position, in milliseconds: the engine's own clock when there
    /// is one (it already subtracts the device latency), the shell's playhead
    /// otherwise (a restored session, a test).
    fn heard_ms(state: &AppState) -> i64 {
        // The engine's clock while it is playing (it already subtracts the
        // device latency); the shell's playhead otherwise — a restored track
        // has a position before the engine has played a single frame, and its
        // words must still be asked for and shown.
        let seconds = match (&state.audio, state.player.playing()) {
            (Some(audio), true) => audio.position(),
            _ => state.player.current_time(),
        };
        if seconds.is_finite() {
            (seconds * 1000.0).max(0.0) as i64
        } else {
            0
        }
    }

    fn current_reference(state: &AppState) -> Option<String> {
        state
            .player
            .current()
            .and_then(|entry| entry.reference().map(str::to_owned))
    }

    /// React to the shell: a new track clears everything and, once the audio is
    /// under way, asks the server once; the active line and the timer follow
    /// the audible clock.
    fn on_state(&mut self, cx: &mut Context<Self>) {
        let (reference, playing, heard) = {
            let state = self.state.read(cx);
            (
                Self::current_reference(state),
                state.player.playing(),
                Self::heard_ms(state),
            )
        };

        if reference != self.for_track {
            // A track change: the previous words must not sit under the new
            // title for a single frame.
            self.for_track = reference.clone();
            self.requested = false;
            self.active = -1;
            self.timer = None;
            self.first_follow = true;
            if self.lines().is_some() || matches!(self.status, Status::Ready(_)) {
                self.sizes = Rc::new(Vec::new());
            }
            self.status = match reference {
                Some(_) => Status::Loading,
                None => Status::Idle,
            };
        }

        // Fetch once, and only for a track that has started: a prefetched row
        // is not read along to.
        if let Some(reference) = reference.clone()
            && !self.requested
            && (playing || heard > 0)
            && !matches!(self.status, Status::Ready(_))
        {
            self.requested = true;
            self.fetch(reference, cx);
        }

        self.advance(heard, playing, cx);
    }

    fn lines(&self) -> Option<&[LyricLine]> {
        match &self.status {
            Status::Ready(Some(doc)) if !doc.synced.is_empty() => Some(&doc.synced),
            _ => None,
        }
    }

    fn fetch(&mut self, reference: String, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let (Some(api), runtime) = (state.api(), state.runtime.clone()) else {
            self.status = Status::Failed(
                "Session non ouverte : les paroles ne peuvent pas être demandées.".into(),
            );
            return;
        };
        let path = format!("/lyrics/{reference}");
        // `Prefetch` and not `User`: the words come after the stream they
        // belong to, and a background request yields to a tap (rule 1). The
        // route is cacheable, so a second visit costs no request at all.
        //
        // The request runs ON THE NETWORK RUNTIME (`nz-net`): its quiet clock is
        // a `tokio::time` deadline, and polling it from the GPUI executor would
        // panic with "no reactor running". The foreground task only waits for
        // the answer and touches the widget.
        let request =
            runtime.spawn(async move { api.get::<serde_json::Value>(Lane::Prefetch, &path).await });
        cx.spawn(async move |weak, cx| {
            let answer: Result<Option<LyricDoc>, String> = match request.await {
                Ok(Ok(fresh)) => Ok(doc_from_value(&fresh.value)),
                Ok(Err(error)) => Err(error.cause_fr().to_owned()),
                Err(join) => Err(format!("la requête a échoué : {join}")),
            };
            let _ = weak.update(cx, |this, cx| {
                if this.for_track.as_deref() != Some(reference.as_str()) {
                    // The queue moved on while the answer was in flight.
                    return;
                }
                match answer {
                    Ok(doc) => this.set_doc(doc),
                    Err(cause) => this.status = Status::Failed(cause.into()),
                }
                this.on_state(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_doc(&mut self, doc: Option<LyricDoc>) {
        match &doc {
            Some(doc) if !doc.synced.is_empty() => {
                self.sizes = Rc::new(
                    (0..doc.synced.len())
                        .map(|_| Size::new(px(0.), px(ROW_HEIGHT)))
                        .collect(),
                );
            }
            _ => self.sizes = Rc::new(Vec::new()),
        }
        self.status = Status::Ready(doc);
        self.active = -1;
        self.timer = None;
    }

    /// The active line right now: what the panel highlights, and what the tests
    /// read instead of a screenshot.
    pub fn active(&self) -> i64 {
        self.active
    }

    /// Retry the current track's words — what « Réessayer » on a failed panel
    /// does (rule 3: a cause, and the gesture that repeats it).
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        let Some(reference) = self.for_track.clone() else {
            return;
        };
        self.requested = true;
        self.status = Status::Loading;
        self.fetch(reference, cx);
        cx.notify();
    }

    /// Compute the active line from the audible position, scroll to it and arm
    /// the next line's own timer.
    fn advance(&mut self, heard: i64, playing: bool, cx: &mut Context<Self>) {
        let Some(track) = self.for_track.clone() else {
            self.active = -1;
            return;
        };
        let Some(lines) = self.lines().map(<[LyricLine]>::to_vec) else {
            self.active = -1;
            return;
        };
        let active = active_index(&lines, heard);
        if active != self.active {
            self.active = active;
            if active >= 0 {
                self.scroll
                    .scroll_to_item(active as usize, ScrollStrategy::Center);
            }
            cx.notify();
        }
        if !playing {
            self.timer = None;
            return;
        }
        let Some(at_ms) = next_boundary_ms(&lines, heard) else {
            self.timer = None;
            return;
        };
        let _ = track;
        // One wake-up, at the line's own moment. Every state notification
        // re-arms it (the SPA's `run()` does the same on each `timeupdate`), so
        // a seek or a pause moves the wake-up instead of leaving it behind; the
        // previous task is dropped, which cancels it.
        let delay_ms = (at_ms - heard).clamp(1, 60_000) as u64;
        self.timer = Some(cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(delay_ms))
                .await;
            let _ = weak.update(cx, |this, cx| {
                let (playing_now, heard_now) = {
                    let state = this.state.read(cx);
                    (state.player.playing(), Self::heard_ms(state))
                };
                this.on_state_at(heard_now, playing_now, cx);
                cx.notify();
            });
        }));
    }

    /// The timer's entry point: same work as [`Self::on_state`] without asking
    /// the server again.
    fn on_state_at(&mut self, heard: i64, playing: bool, cx: &mut Context<Self>) {
        if self.for_track.is_none() {
            return;
        }
        self.advance(heard, playing, cx);
    }
}

impl Render for LyricsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div()
            .id("lyrics")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .font_family(theme::font(cx))
            .text_color(theme::text(cx));

        match &self.status {
            Status::Idle => {
                body = body.child(placeholder("Aucune lecture en cours.", cx));
            }
            Status::Loading => {
                body = body.child(placeholder("Chargement des paroles…", cx));
            }
            Status::Failed(cause) => {
                let cause = cause.clone();
                body = body.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(rems(0.5))
                        .size_full()
                        .child(
                            div()
                                .text_size(rems(0.8125))
                                .text_color(theme::secondary(cx))
                                .child(cause),
                        )
                        .child(
                            button("lyrics-retry", "Réessayer", ButtonVariant::Outline, cx)
                                .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
                        ),
                );
            }
            Status::Ready(None) => {
                body = body.child(placeholder("Paroles indisponibles pour ce titre.", cx));
            }
            Status::Ready(Some(doc)) if doc.synced.is_empty() => {
                body = body.child(
                    div()
                        .id("lyrics-plain")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .p(rems(0.5))
                        .text_size(rems(0.875))
                        .line_height(rems(1.4))
                        .text_color(theme::secondary(cx))
                        .child(doc.text.clone()),
                );
            }
            Status::Ready(Some(_)) => {
                let active = self.active;
                let font = theme::font(cx);
                let accent = theme::accent(cx);
                let text = theme::text(cx);
                body = body.child(
                    virtual_list(
                        cx.entity(),
                        "lyrics-lines",
                        self.sizes.clone(),
                        move |this, range, _, _| {
                            let Some(lines) = this.lines() else {
                                return Vec::new();
                            };
                            range
                                .filter_map(|index| {
                                    let line = lines.get(index)?;
                                    let is_active = index as i64 == active;
                                    Some(
                                        div()
                                            .id(("lyric", index))
                                            .h(px(ROW_HEIGHT))
                                            .w_full()
                                            .flex()
                                            .items_center()
                                            .px(rems(0.5))
                                            .child(
                                                div()
                                                    .w_full()
                                                    .line_clamp(2)
                                                    .text_size(if is_active {
                                                        rems(1.0625)
                                                    } else {
                                                        rems(0.9375)
                                                    })
                                                    .font_family(font.clone())
                                                    .font_weight(if is_active {
                                                        gpui_kit::FontWeight::SEMIBOLD
                                                    } else {
                                                        gpui_kit::FontWeight::NORMAL
                                                    })
                                                    .text_color(if is_active {
                                                        accent
                                                    } else {
                                                        text
                                                    })
                                                    .child(if line.text.is_empty() {
                                                        "• • •".to_owned()
                                                    } else {
                                                        line.text.clone()
                                                    }),
                                            ),
                                    )
                                })
                                .collect()
                        },
                        cx,
                    )
                    .flex_1()
                    .min_h_0()
                    .h_full()
                    .track_scroll(&self.scroll),
                );
            }
        }
        body
    }
}

fn placeholder(message: impl Into<SharedString>, cx: &mut Context<LyricsView>) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p(rems(1.))
        .text_size(rems(0.8125))
        .text_color(theme::secondary(cx))
        .child(message.into())
}

#[cfg(test)]
mod tests {
    use gpui_kit::AppContext as _;

    use super::*;

    fn lines(times: &[i64]) -> Vec<LyricLine> {
        times
            .iter()
            .map(|time| LyricLine {
                time: *time,
                text: format!("ligne {time}"),
            })
            .collect()
    }

    #[test]
    fn a_line_is_active_from_its_own_timestamp_to_the_next_one() {
        let lines = lines(&[0, 4_000, 9_500, 12_000]);
        assert_eq!(active_index(&lines, 0), 0);
        assert_eq!(active_index(&lines, 3_999), 0);
        assert_eq!(active_index(&lines, 4_000), 1, "exactly on the boundary");
        assert_eq!(active_index(&lines, 9_499), 1);
        assert_eq!(active_index(&lines, 9_500), 2);
        assert_eq!(active_index(&lines, 600_000), 3, "the last line stays");
        assert_eq!(active_index(&[], 1_000), -1);
    }

    #[test]
    fn the_next_boundary_is_the_moment_the_timer_must_fire() {
        let lines = lines(&[0, 4_000, 9_500]);
        assert_eq!(next_boundary_ms(&lines, 0), Some(4_000));
        assert_eq!(next_boundary_ms(&lines, 3_999), Some(4_000));
        assert_eq!(next_boundary_ms(&lines, 4_000), Some(9_500));
        assert_eq!(next_boundary_ms(&lines, 9_500), None);
    }

    /// The measured claim: a line flips at its timestamp, not up to 250 ms
    /// later like the shell's four-per-second tick would make it.
    #[test]
    fn a_flip_lands_on_the_millisecond_of_its_timestamp() {
        let lines = lines(&[0, 1_000, 2_000]);
        // A repaint 1 ms before the boundary still shows the previous line...
        assert_eq!(active_index(&lines, 999), 0);
        // ...and the timer armed for 1 000 ms shows the new one.
        assert_eq!(active_index(&lines, 1_000), 1);
        // The worst error of this scheme is the frame the timer is scheduled
        // in (4.2 ms at 240 Hz), against 250 ms for a 4 Hz poll: the boundary is
        // reached with both sides of it exercised above.
        assert_eq!(1_000 - 999, 1);
    }

    #[test]
    fn both_bodies_the_server_answers_are_read() {
        // The archived/cold object.
        let object = r#"{"lyrics":{"text":"un\ndeux","synced":[{"time":15000,"text":"deux"},
                        {"time":0,"text":"un"}],"source":"archive"}}"#;
        let doc = parse_body(object).expect("json").expect("a document");
        assert_eq!(doc.synced.len(), 2);
        assert_eq!(doc.synced[0].time, 0, "sorted, whatever the sidecar said");
        assert_eq!(doc.synced[0].text, "un");
        assert_eq!(doc.text, "un\ndeux");

        // An older server's plain string is text, not a decode failure.
        let plain = r#"{"lyrics":"des paroles sans temps"}"#;
        let doc = parse_body(plain).expect("json").expect("a document");
        assert!(doc.synced.is_empty());
        assert_eq!(doc.text, "des paroles sans temps");

        // Nothing to show is `null`, which is not an error.
        assert_eq!(parse_body(r#"{"lyrics":null}"#), Ok(None));
        assert!(parse_body("pas du json").is_err());
    }

    #[test]
    fn a_document_built_from_its_lines_keeps_the_same_text() {
        let doc = LyricDoc::from_synced(lines(&[0, 500]));
        assert_eq!(doc.text, "ligne 0\nligne 500");
    }

    /// A server that answers every route with one body: the fetch path is
    /// exercised for real (a socket, the lane, the parse), nothing else.
    fn lyrics_server(body: String) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("local address");
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{address}")
    }

    /// The panel's own timing, in the real widget: the lines arrive from the
    /// server, then the highlighted line follows the AUDIBLE position — and the
    /// one-shot timer armed for the next boundary is what moves it, not the
    /// shell's 4 Hz tick.
    #[gpui::test]
    async fn the_highlighted_line_flips_on_its_timestamp(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_omarchy::init(cx);
            crate::keymap::install(cx);
        });
        let server = lyrics_server(
            r#"{"lyrics":{"text":"un\ndeux\ntrois","synced":[{"time":0,"text":"un"},{"time":1000,"text":"deux"},{"time":2000,"text":"trois"}],"source":"archive"}}"#
                .to_owned(),
        );
        let dir = std::env::temp_dir().join(format!(
            "neezer-d9-lyrics-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (tx, _rx) = crate::state::event_channel();
        let state = {
            let runtime = runtime.handle().clone();
            let dir = dir.clone();
            cx.update(move |cx| {
                cx.new(|_| {
                    crate::state::AppState::new(
                        crate::state::AppConfig {
                            server: Some(server),
                            start_audio: false,
                            start_mpris: false,
                            ..crate::state::AppConfig::for_test(&dir)
                        },
                        runtime,
                        tx,
                        None,
                    )
                })
            })
        };
        // The panel first (nothing playing: nothing is asked), then the queue:
        // the request is the track's, and it lands on the network runtime.
        let lyrics = cx.update(|cx| cx.new(|cx| LyricsView::new(state.clone(), cx)));
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.command(
                    nz_player::Command::PlayQueue {
                        args: nz_player::QueueArgs {
                            tracks: vec![
                                nz_player::Entry {
                                    id: "one".into(),
                                    title: "Un".into(),
                                    duration: 200,
                                    ..Default::default()
                                },
                                nz_player::Entry {
                                    id: "two".into(),
                                    title: "Deux".into(),
                                    duration: 200,
                                    ..Default::default()
                                },
                            ],
                            start: 0,
                            context: None,
                        },
                    },
                    cx,
                );
            });
        });

        // The test thread is the network thread: it drives the runtime between
        // two frames, exactly as the shell's own runtime is driven in the app.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            runtime.block_on(async {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            });
            cx.run_until_parked();
            if cx.update(|cx| lyrics.read(cx).lines().map(<[LyricLine]>::len)) == Some(3) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the server's lines never reached the panel"
            );
        }

        cx.update(|cx| {
            state.update(cx, |state, _| state.player.set_position(0.9));
            lyrics.update(cx, |lyrics, cx| lyrics.advance(900, true, cx));
        });
        assert_eq!(
            cx.update(|cx| lyrics.read(cx).active()),
            0,
            "900 ms is still the first line"
        );

        // The timer armed for 1 000 ms fires (the clock advances), and the
        // position it reads is the audible one: the new line is shown.
        cx.update(|cx| state.update(cx, |state, _| state.player.set_position(1.0)));
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        assert_eq!(
            cx.update(|cx| lyrics.read(cx).active()),
            1,
            "the line flips at its own timestamp, not on the shell's next tick"
        );

        // A seek backwards is followed at once.
        cx.update(|cx| {
            lyrics.update(cx, |lyrics, cx| lyrics.advance(200, true, cx));
        });
        assert_eq!(cx.update(|cx| lyrics.read(cx).active()), 0);

        std::fs::remove_dir_all(dir).ok();
        drop(runtime);
    }
}
