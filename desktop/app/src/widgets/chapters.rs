//! Podcast chapters: one episode's own table of contents, its state, and the
//! rows that jump into it.
//!
//! The rules are `webapp/src/lib/chapters.js`'s, ported rather than re-derived:
//!
//! - **A chapter's `start` is SECONDS** while a marker's `position` is
//!   MILLISECONDS (`nz-model`'s two types say it; D1's fixtures caught it). This
//!   module never mixes them: [`at`] takes seconds, [`crate::widgets::markers`]
//!   takes milliseconds.
//! - **The list is normalised**: sorted by start, a negative start dropped, an
//!   `end` before its `start` dropped rather than believed, two chapters at the
//!   same second collapsed (a feed that repeats a row would show it twice).
//! - **One request per episode per visit, and it comes from the live routes.** A
//!   chapter list is written when the episode is archived, so `nz-api`'s
//!   `is_cacheable` refuses `/podcast/episode/<id>/chapters` on purpose: what is
//!   kept is this in-memory map, and a second expansion in the same visit costs
//!   **0 requests** (the acceptance test asserts both counts).
//! - **Only what is visible is built**: [`MAX_INLINE`] rows inline, the rest
//!   behind "Voir les N chapitres", exactly as `Show.svelte` does.
//!
//! Nothing here is async: a screen owns the network, calls [`Chapters::load`],
//! hands the answer to [`Chapters::resolved`] and repaints. That keeps the widget
//! usable from an entity test without a runtime.

use std::collections::HashMap;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement,
    Render, StatefulInteractiveElement as _, Styled, div, rems,
};
use gpui_omarchy::{IconName, icon};
use nz_api::ApiError;
use nz_model::{Chapter, EpisodeChapters};

use crate::state::AppState;
use crate::theme;
use crate::widgets::track_list::duration_label;

/// The rows `Show.svelte` shows inline before "Voir les N chapitres" (8 on a
/// desktop, 4 on a phone — a desktop window is the wide one).
pub const MAX_INLINE: usize = 8;

/// Where one episode's chapter list stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// Never asked for.
    #[default]
    Idle,
    /// One request in flight.
    Loading,
    /// The answer is here (possibly empty).
    Ready,
    /// The request failed; `error` says why.
    Failed,
}

/// One episode's chapters and what the answer said about the episode.
#[derive(Clone, Debug, Default)]
pub struct Entry {
    pub status: Status,
    pub chapters: Vec<Chapter>,
    /// The audio is on disk: chapters can be read out of the file's tags.
    pub archived: Option<bool>,
    /// The description was already scanned for chapters.
    pub scanned: bool,
    /// The episode's length in seconds, as the server counted it.
    pub duration: u32,
    /// `"id3"`, `"description"`, … — what the answer came from.
    pub source: Option<String>,
    pub error: Option<ApiError>,
    /// A cached copy is on screen and the live answer has not landed.
    pub stale: bool,
}

/// Every episode this screen has asked about, keyed by the episode's id as the
/// server spells it.
#[derive(Default)]
pub struct Chapters {
    by_episode: HashMap<String, Entry>,
}

impl Chapters {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, episode_id: &str) -> Option<&Entry> {
        self.by_episode.get(episode_id)
    }

    /// Should this episode be asked for, once? Idle and failed both mean "ask";
    /// asking and ready do not.
    pub fn wants(&self, episode_id: &str) -> bool {
        match self.by_episode.get(episode_id) {
            None => true,
            Some(entry) => matches!(entry.status, Status::Idle | Status::Failed),
        }
    }

    pub fn is_loading(&self, episode_id: &str) -> bool {
        self.by_episode
            .get(episode_id)
            .is_some_and(|entry| entry.status == Status::Loading)
    }

    /// Mark the episode as being asked for, so a second paint stays quiet.
    pub fn asking(&mut self, episode_id: &str) {
        self.by_episode.insert(
            episode_id.to_owned(),
            Entry {
                status: Status::Loading,
                ..Entry::default()
            },
        );
    }

    /// Ask for one episode's chapters on the network runtime.
    ///
    /// `done` runs on whatever thread answers (the tokio worker), which is why
    /// it only sends a message: a screen drains its own channel on the UI thread.
    pub fn load<F>(&mut self, state: &AppState, episode_id: &str, done: F)
    where
        F: FnOnce(String, Result<EpisodeChapters, ApiError>) + Send + 'static,
    {
        let (Some(api), Some(_cache)) = (state.api.clone(), state.cache.as_ref()) else {
            return;
        };
        self.asking(episode_id);
        let id = episode_id.to_owned();
        state.runtime.spawn(async move {
            let answer = api.episode_chapters(&id).await;
            done(id, answer);
        });
    }

    /// Adopt the answer. A failure keeps what was already known: a chapter list
    /// that is on screen is better than an empty one, and the cause is shown
    /// under it.
    pub fn resolved(&mut self, episode_id: &str, answer: Result<EpisodeChapters, ApiError>) {
        let entry = self.by_episode.entry(episode_id.to_owned()).or_default();
        match answer {
            Ok(page) => {
                entry.status = Status::Ready;
                entry.chapters = normalize(&page.chapters);
                entry.archived = Some(page.archived);
                entry.scanned = page.scanned;
                entry.duration = page.duration;
                entry.source = page.source;
                entry.error = None;
                entry.stale = false;
            }
            Err(error) => {
                entry.status = Status::Failed;
                entry.error = Some(error);
                if entry.chapters.is_empty() {
                    entry.stale = false;
                }
            }
        }
    }

    /// A screen's `Réessayer`: forget the failure so [`Chapters::wants`] is true.
    pub fn retry(&mut self, episode_id: &str) {
        self.by_episode.remove(episode_id);
    }

    /// Forget everything (a different show is open).
    pub fn clear(&mut self) {
        self.by_episode.clear();
    }
}

/// The SPA's `normalizeChapters`: sorted, sane, deduplicated.
pub fn normalize(chapters: &[Chapter]) -> Vec<Chapter> {
    let mut out: Vec<Chapter> = chapters
        .iter()
        .filter(|chapter| chapter.start.is_finite() && chapter.start >= 0.0)
        .map(|chapter| {
            let end = if chapter.end.is_finite() && chapter.end >= chapter.start {
                chapter.end
            } else {
                chapter.start
            };
            Chapter {
                index: chapter.index,
                start: chapter.start,
                end,
                title: chapter.title.trim().to_owned(),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut seen: Vec<f64> = Vec::with_capacity(out.len());
    out.retain(|chapter| {
        if seen
            .iter()
            .any(|start| (*start - chapter.start).abs() < 1e-9)
        {
            return false;
        }
        seen.push(chapter.start);
        true
    });
    out
}

/// Which chapter `seconds` falls in, or `None` when the playhead is before the
/// first one or past a chapter that named its own end (the SPA's `chapterAt`).
pub fn at(chapters: &[Chapter], seconds: f64) -> Option<usize> {
    let first = chapters.first()?;
    if !seconds.is_finite() || seconds < first.start {
        return None;
    }
    // The list is sorted and small (a feed carries tens, not thousands): a walk
    // beats a binary search for its own branch cost, and it is the same answer.
    let mut found = 0;
    for (index, chapter) in chapters.iter().enumerate() {
        if chapter.start <= seconds {
            found = index;
        } else {
            break;
        }
    }
    let chapter = &chapters[found];
    if chapter.end > chapter.start && seconds >= chapter.end {
        return None;
    }
    Some(found)
}

/// Where a chapter ends: its own `end`, else the next chapter's start, else the
/// episode's duration (the SPA's `chapterEnd`).
pub fn end_of(chapters: &[Chapter], index: usize, duration: f64) -> f64 {
    let Some(chapter) = chapters.get(index) else {
        return 0.0;
    };
    if chapter.end > chapter.start {
        return chapter.end;
    }
    if let Some(next) = chapters.get(index + 1) {
        return next.start;
    }
    if duration > chapter.start {
        duration
    } else {
        chapter.start
    }
}

/// "3 chapitres" / "1 chapitre" (`chapters.js#chapterCountLabel`).
pub fn count_label(count: u32) -> String {
    match count {
        0 => "Aucun chapitre".to_owned(),
        1 => "1 chapitre".to_owned(),
        other => format!("{other} chapitres"),
    }
}

/// "Chapitre 3 · Le départ" — one row's name.
pub fn label(index: usize, chapter: Option<&Chapter>) -> String {
    let base = format!("Chapitre {}", index + 1);
    match chapter {
        Some(chapter) if !chapter.title.is_empty() => format!("{base} · {}", chapter.title),
        _ => base,
    }
}

/// The chapter rows: a time, a name, and the one playing marked by an accent
/// edge (never by colour alone).
pub fn list<V: Render>(
    chapters: &[Chapter],
    current: Option<usize>,
    limit: usize,
    cx: &mut Context<V>,
    on_seek: impl Fn(&mut V, f64, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    let mut column = div().flex().flex_col().w_full().min_w_0();
    for (index, chapter) in chapters.iter().take(limit.max(1)).enumerate() {
        let is_now = current == Some(index);
        let start = chapter.start;
        let title = if chapter.title.is_empty() {
            format!("Chapitre {}", index + 1)
        } else {
            chapter.title.clone()
        };
        column = column.child(
            div()
                .id(("chapter-row", index))
                .debug_selector(move || format!("chapter-row-{index}"))
                .flex()
                .items_center()
                .gap(rems(0.5))
                .h(rems(2.))
                .w_full()
                .min_w_0()
                .pl(rems(0.5))
                .pr(rems(0.375))
                .border_l_2()
                .border_color(if is_now {
                    theme::accent(cx)
                } else {
                    theme::shell(cx)
                })
                .when(is_now, |row| row.bg(theme::selected(cx)))
                .cursor_pointer()
                .hover(|style| style.bg(theme::chrome(cx)))
                .on_click({
                    let on_seek = on_seek.clone();
                    cx.listener(move |this, _, _, cx| on_seek(this, start, cx))
                })
                .child(
                    div()
                        .flex_shrink_0()
                        .w(rems(3.))
                        .text_size(rems(0.6875))
                        .font_family(theme::mono(cx))
                        .text_color(theme::secondary(cx))
                        .child(duration_label(start.max(0.0) as u32)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(rems(0.75))
                        .text_color(if is_now {
                            theme::accent(cx)
                        } else {
                            theme::text(cx)
                        })
                        .child(title),
                )
                .when(is_now, |row| {
                    row.child(
                        div()
                            .flex_shrink_0()
                            .text_size(rems(0.625))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::accent(cx))
                            .child("En cours"),
                    )
                }),
        );
    }
    column.into_any_element()
}

/// The three placeholder rows a loading list shows: the height does not jump
/// when the answer lands.
pub fn loading_rows(cx: &gpui_kit::App) -> Div {
    let mut column = div().flex().flex_col().w_full();
    for index in 0..3usize {
        column = column.child(
            div()
                .id(("chapter-skeleton", index))
                .flex()
                .items_center()
                .gap(rems(0.5))
                .h(rems(2.))
                .pl(rems(0.5))
                .child(div().w(rems(3.)).h(rems(0.5)).bg(theme::border(cx)))
                .child(div().flex_1().h(rems(0.5)).bg(theme::border(cx))),
        );
    }
    column
}

/// The glyph a chapter section uses, kept here so the widget and its caller
/// cannot disagree (and so a missing bundle glyph is one place to fix).
pub fn glyph() -> IconName {
    IconName::List
}

/// A one-line note under a chapter list ("Chargement…", "Aucun chapitre…").
pub fn note(text: impl Into<gpui_kit::SharedString>, cx: &gpui_kit::App) -> Div {
    div()
        .px(rems(0.5))
        .py(rems(0.25))
        .text_size(rems(0.75))
        .text_color(theme::secondary(cx))
        .child(text.into())
}

/// The `Réessayer` of a failed chapter list, with the cause above it.
pub fn retry_row<V: Render>(
    cause: impl Into<gpui_kit::SharedString>,
    cx: &mut Context<V>,
    on_retry: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(rems(0.25))
        .px(rems(0.5))
        .py(rems(0.25))
        .child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child(cause.into()),
        )
        .child(
            div()
                .id("chapters-retry")
                .flex()
                .items_center()
                .gap(rems(0.25))
                .h(rems(1.75))
                .px(rems(0.5))
                .border_1()
                .border_color(theme::border(cx))
                .text_size(rems(0.75))
                .text_color(theme::accent(cx))
                .cursor_pointer()
                .hover(|style| style.bg(theme::chrome(cx)))
                .on_click(cx.listener(move |this, _, _, cx| on_retry(this, cx)))
                .child(icon(IconName::RefreshCw).size(rems(0.75)))
                .child("Réessayer"),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter(start: f64, end: f64, title: &str) -> Chapter {
        Chapter {
            index: 0,
            start,
            end,
            title: title.to_owned(),
        }
    }

    /// The SPA's normalisation, case by case: the order does not depend on the
    /// feed's, a broken end is dropped, a repeated second is one row.
    #[test]
    fn a_chapter_list_is_sorted_sane_and_deduplicated() {
        let raw = vec![
            chapter(120.0, 0.0, "  Deuxième "),
            chapter(-5.0, 3.0, "Négatif"),
            chapter(30.0, 90.0, "Premier"),
            chapter(120.0, 150.0, "Doublon"),
            chapter(90.0, f64::NAN, "Fin cassée"),
        ];
        let list = normalize(&raw);
        assert_eq!(
            list.iter().map(|c| c.start).collect::<Vec<_>>(),
            vec![30.0, 90.0, 120.0],
            "sorted, the negative one dropped, the repeated second collapsed"
        );
        assert_eq!(list[0].title, "Premier");
        assert_eq!(
            list[1].end, 90.0,
            "a NaN end falls back to the chapter's own start, never to 0"
        );
    }

    /// `at` is the playhead's chapter: before the first one there is none, and a
    /// chapter that named its end stops owning the playhead past it.
    #[test]
    fn the_playheads_chapter_follows_its_own_bounds() {
        let list = normalize(&[
            chapter(0.0, 60.0, "Intro"),
            chapter(60.0, 0.0, "Milieu"),
            chapter(600.0, 700.0, "Fin"),
        ]);
        assert_eq!(at(&list, -1.0), None);
        assert_eq!(at(&list, 0.0), Some(0));
        assert_eq!(at(&list, 59.9), Some(0));
        assert_eq!(at(&list, 60.0), Some(1));
        assert_eq!(at(&list, 300.0), Some(1), "no end: it runs to the next one");
        assert_eq!(at(&list, 650.0), Some(2));
        assert_eq!(at(&list, 750.0), None, "past a named end, no chapter");
        assert_eq!(at(&[], 10.0), None);
    }

    #[test]
    fn a_chapter_ends_where_its_own_end_says_else_at_the_next_one() {
        let list = normalize(&[chapter(0.0, 0.0, "A"), chapter(120.0, 180.0, "B")]);
        assert_eq!(end_of(&list, 0, 3_600.0), 120.0);
        assert_eq!(end_of(&list, 1, 3_600.0), 180.0);
        assert_eq!(end_of(&[], 0, 3_600.0), 0.0);
        // The last one without an end runs to the episode's own duration.
        let tail = normalize(&[chapter(3_500.0, 0.0, "Fin")]);
        assert_eq!(end_of(&tail, 0, 3_600.0), 3_600.0);
    }

    #[test]
    fn a_count_and_a_name_read_in_french() {
        assert_eq!(count_label(0), "Aucun chapitre");
        assert_eq!(count_label(1), "1 chapitre");
        assert_eq!(count_label(12), "12 chapitres");
        assert_eq!(label(0, None), "Chapitre 1");
        assert_eq!(
            label(2, Some(&chapter(0.0, 0.0, "Le départ"))),
            "Chapitre 3 · Le départ"
        );
        assert_eq!(label(2, Some(&chapter(0.0, 0.0, ""))), "Chapitre 3");
    }

    /// Asking twice is one request: `wants` is false while the answer is in
    /// flight and after it landed, and a `Réessayer` is what makes it true again.
    #[test]
    fn an_episode_is_asked_for_once_until_it_fails() {
        let mut chapters = Chapters::new();
        assert!(chapters.wants("e1"));
        chapters.asking("e1");
        assert!(!chapters.wants("e1"));
        assert!(chapters.is_loading("e1"));
        chapters.resolved(
            "e1",
            Ok(EpisodeChapters {
                archived: true,
                chapters: vec![chapter(0.0, 0.0, "A")],
                duration: 3_600,
                ..EpisodeChapters::default()
            }),
        );
        assert!(!chapters.wants("e1"));
        assert_eq!(chapters.get("e1").map(|e| e.chapters.len()), Some(1));
        assert_eq!(chapters.get("e1").and_then(|e| e.archived), Some(true));

        chapters.asking("e2");
        chapters.resolved(
            "e2",
            Err(ApiError::Http {
                status: 500,
                message: "boom".into(),
                code: None,
            }),
        );
        assert!(chapters.wants("e2"), "a failure offers Réessayer");
        chapters.retry("e2");
        assert!(chapters.wants("e2"));
    }
}
