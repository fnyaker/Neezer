//! Podcast markers: a person's own bookmarked moments inside an episode, and
//! the one call that loads them for a whole show.
//!
//! Ported from `webapp/src/lib/markers.js`, with its two rules:
//!
//! - **A marker's `position` is MILLISECONDS.** `nz-model`'s `Marker` says so and
//!   D1's fixtures proved it: a chapter's `start` is seconds, a marker's is a
//!   thousand times that. [`format`] divides exactly once, here.
//! - **Every mutation is optimistic and reconciled.** Adding a marker paints it
//!   in the same frame (a click must answer at once, rule 4) with a temporary id;
//!   the server's answer replaces it, and a failure removes it again and names
//!   the cause (rule 3). Removing paints the row gone first and puts it back if
//!   the server refused.
//!
//! The show page loads every marker of a show in ONE request
//! (`GET /podcast/<id>/markers`, grouped by episode) rather than one per episode,
//! and the list is `fail-soft`: no markers is a page that works, so a failed
//! read is remembered only to offer "Réessayer".

use std::collections::BTreeMap;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, ElementId, InteractiveElement as _, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement as _, Styled, div, rems,
};
use gpui_omarchy::{IconName, icon};
use nz_api::ApiError;
use nz_model::{Marker, ShowMarkers};

use crate::state::AppState;
use crate::theme;
use crate::widgets::track_list::duration_label;

/// How many markers one row shows before it says "et N autres": an episode with
/// twenty bookmarks must not turn a list row into a wall.
pub const MAX_INLINE: usize = 4;

/// Every marker of one show, by episode id as the server spells it.
#[derive(Default)]
pub struct Markers {
    by_episode: BTreeMap<String, Vec<Marker>>,
    /// The show this map is for: opening another one drops it.
    channel: Option<String>,
    loaded: bool,
    loading: bool,
    error: Option<ApiError>,
    /// The id of the next optimistic marker (the server's ids are uuids).
    pending: u64,
}

impl Markers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn for_episode(&self, episode_id: &str) -> &[Marker] {
        self.by_episode
            .get(episode_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&ApiError> {
        self.error.as_ref()
    }

    /// For one show, once: a different show starts empty.
    pub fn wants(&self, channel_id: &str) -> bool {
        self.channel.as_deref() != Some(channel_id) || (!self.loaded && !self.loading)
    }

    /// Read every marker of `channel_id` in one call, fail-soft.
    pub fn load<F>(&mut self, state: &AppState, channel_id: &str, done: F)
    where
        F: FnOnce(String, Result<ShowMarkers, ApiError>) + Send + 'static,
    {
        let Some(api) = state.api.clone() else {
            return;
        };
        if self.channel.as_deref() != Some(channel_id) {
            self.by_episode.clear();
            self.loaded = false;
        }
        self.channel = Some(channel_id.to_owned());
        self.loading = true;
        self.error = None;
        let id = channel_id.to_owned();
        state.runtime.spawn(async move {
            let answer = api.show_markers(&id).await;
            done(id, answer);
        });
    }

    pub fn resolved(&mut self, channel_id: &str, answer: Result<ShowMarkers, ApiError>) {
        if self.channel.as_deref() != Some(channel_id) {
            return; // another show answered late: it is not this page's
        }
        self.loading = false;
        match answer {
            Ok(page) => {
                self.loaded = true;
                self.error = None;
                self.by_episode = page
                    .markers
                    .into_iter()
                    .map(|(episode, mut markers)| {
                        sort(&mut markers);
                        (episode, markers)
                    })
                    .filter(|(_, markers)| !markers.is_empty())
                    .collect();
            }
            Err(error) => {
                // The markers already on screen stay: they are what a person put
                // there, and the cause is a line under them.
                self.error = Some(error);
            }
        }
    }

    /// Put a marker at `position_ms` (milliseconds, the row's own unit). The chip
    /// appears at once with a temporary id; `done` carries the server's marker.
    pub fn add<F>(
        &mut self,
        state: &AppState,
        episode_id: &str,
        position_ms: u32,
        label: Option<String>,
        done: F,
    ) -> Option<String>
    where
        F: FnOnce(String, String, Result<Marker, ApiError>) + Send + 'static,
    {
        let api = state.api.clone()?;
        self.pending += 1;
        let temporary = format!("pending-{}", self.pending);
        let marker = Marker {
            id: temporary.clone(),
            episode_id: episode_id.into(),
            position: position_ms,
            label: label.clone().unwrap_or_default(),
            created: None,
        };
        let list = self.by_episode.entry(episode_id.to_owned()).or_default();
        list.push(marker);
        sort(list);
        let episode = episode_id.to_owned();
        let label = label.unwrap_or_default();
        state.runtime.spawn(async move {
            let answer = api
                .add_marker(
                    &episode,
                    position_ms,
                    (!label.is_empty()).then_some(label.as_str()),
                )
                .await;
            done(episode, temporary, answer);
        });
        Some(position_ms.to_string())
    }

    /// Adopt the server's marker in place of the temporary one, or drop it.
    pub fn added(&mut self, episode_id: &str, temporary: &str, answer: Result<Marker, ApiError>) {
        let Some(list) = self.by_episode.get_mut(episode_id) else {
            return;
        };
        let at = list.iter().position(|marker| marker.id == temporary);
        match answer {
            Ok(marker) => {
                if let Some(at) = at {
                    list[at] = marker;
                } else {
                    list.push(marker);
                }
                sort(list);
            }
            Err(_) => {
                if let Some(at) = at {
                    list.remove(at);
                }
                if list.is_empty() {
                    self.by_episode.remove(episode_id);
                }
            }
        }
    }

    /// Remove a marker now, and put it back if the server refused.
    pub fn remove<F>(&mut self, state: &AppState, marker: &Marker, done: F)
    where
        F: FnOnce(Marker, Result<(), ApiError>) + Send + 'static,
    {
        let Some(api) = state.api.clone() else {
            return;
        };
        let episode = marker.episode_id.as_str().to_owned();
        if let Some(list) = self.by_episode.get_mut(&episode) {
            list.retain(|held| held.id != marker.id);
            if list.is_empty() {
                self.by_episode.remove(&episode);
            }
        }
        let removed = marker.clone();
        state.runtime.spawn(async move {
            let answer = api.delete_marker(&removed.id).await;
            done(removed, answer);
        });
    }

    /// The server refused: the chip comes back where it was.
    pub fn restore(&mut self, marker: Marker) {
        let list = self
            .by_episode
            .entry(marker.episode_id.as_str().to_owned())
            .or_default();
        if !list.iter().any(|held| held.id == marker.id) {
            list.push(marker);
            sort(list);
        }
    }

    /// Forget everything (a `Réessayer` that starts over, a logout).
    pub fn clear(&mut self) {
        self.by_episode.clear();
        self.channel = None;
        self.loaded = false;
        self.loading = false;
        self.error = None;
    }
}

fn sort(list: &mut [Marker]) {
    list.sort_by_key(|marker| marker.position);
}

/// `12:34`: a marker's whole visible identity, from milliseconds.
pub fn format(position_ms: u32) -> String {
    duration_label(position_ms / 1_000)
}

/// The chips of one episode's markers: a jump, its name, and the ✕ that removes
/// it. `None` when there is nothing to draw — a row with no marker says nothing.
pub fn chips<V: Render>(
    episode_id: &str,
    markers: &[Marker],
    limit: usize,
    cx: &mut Context<V>,
    on_jump: impl Fn(&mut V, u32, &mut Context<V>) + Clone + 'static,
    on_delete: impl Fn(&mut V, Marker, &mut Context<V>) + Clone + 'static,
) -> Option<AnyElement> {
    if markers.is_empty() {
        return None;
    }
    let mut row = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(rems(0.25))
        .min_w_0();
    for (index, marker) in markers.iter().take(limit.max(1)).enumerate() {
        let position = marker.position;
        let held = marker.clone();
        row = row.child(
            div()
                .id(ElementId::Name(SharedString::from(format!(
                    "marker-{episode_id}-{index}-{}",
                    marker.id
                ))))
                .flex()
                .items_center()
                .gap(rems(0.1875))
                .h(rems(1.375))
                .pl(rems(0.25))
                .border_1()
                .border_color(theme::border(cx))
                .text_size(rems(0.625))
                .font_family(theme::mono(cx))
                .text_color(theme::secondary(cx))
                .child(
                    div()
                        .id(ElementId::Name(SharedString::from(format!(
                            "marker-jump-{episode_id}-{index}"
                        ))))
                        .flex()
                        .items_center()
                        .gap(rems(0.1875))
                        .h_full()
                        .pr(rems(0.25))
                        .cursor_pointer()
                        .hover(|style| style.text_color(theme::text(cx)))
                        .on_click({
                            let on_jump = on_jump.clone();
                            cx.listener(move |this, _, _, cx| on_jump(this, position, cx))
                        })
                        .child(
                            icon(IconName::Bookmark)
                                .size(rems(0.625))
                                .text_color(theme::accent(cx)),
                        )
                        .child(format(position))
                        .when(!marker.label.is_empty(), |chip| {
                            chip.child(
                                div()
                                    .max_w(rems(9.))
                                    .truncate()
                                    .font_family(theme::font(cx))
                                    .child(marker.label.clone()),
                            )
                        }),
                )
                .child(
                    div()
                        .id(ElementId::Name(SharedString::from(format!(
                            "marker-delete-{episode_id}-{index}-{}",
                            marker.id
                        ))))
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(rems(1.))
                        .h_full()
                        .cursor_pointer()
                        .text_color(theme::secondary(cx))
                        .hover(|style| style.text_color(theme::danger(cx)))
                        .on_click({
                            let on_delete = on_delete.clone();
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                on_delete(this, held.clone(), cx)
                            })
                        })
                        .child(icon(IconName::Close).size(rems(0.5625))),
                ),
        );
    }
    let hidden = markers.len().saturating_sub(limit.max(1));
    if hidden > 0 {
        row = row.child(
            div()
                .text_size(rems(0.625))
                .text_color(theme::secondary(cx))
                .child(format!(
                    "et {hidden} autre{}",
                    if hidden > 1 { "s" } else { "" }
                )),
        );
    }
    Some(row.into_any_element())
}

/// The "add a marker here" control: a bookmark and the moment it would mark.
/// `position_s` is the playhead in SECONDS (what the player hands over); the
/// conversion to the row's milliseconds happens in [`Markers::add`]'s caller, in
/// one place — [`build_position`].
pub fn add_button<V: Render>(
    id: &'static str,
    position_s: f64,
    enabled: bool,
    cx: &mut Context<V>,
    on_add: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(rems(0.25))
        .h(rems(1.75))
        .px(rems(0.375))
        .border_1()
        .border_color(theme::border(cx))
        .text_size(rems(0.6875))
        .text_color(if enabled {
            theme::text(cx)
        } else {
            theme::secondary(cx)
        })
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(|style| style.bg(theme::chrome(cx)))
                .on_click(cx.listener(move |this, _, _, cx| on_add(this, cx)))
        })
        .child(icon(IconName::BookmarkPlus).size(rems(0.75)))
        .child(format!(
            "Marqueur à {}",
            duration_label(position_s.max(0.0) as u32)
        ))
        .into_any_element()
}

/// Seconds → the milliseconds a marker stores, rounded once.
pub fn build_position(position_s: f64) -> u32 {
    if !position_s.is_finite() || position_s <= 0.0 {
        return 0;
    }
    (position_s * 1_000.0).round().min(u32::MAX as f64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_model::EpisodeId;

    fn marker(id: &str, position: u32) -> Marker {
        Marker {
            id: id.to_owned(),
            episode_id: EpisodeId::new("e1"),
            position,
            label: String::new(),
            created: None,
        }
    }

    /// The number a person reads comes from milliseconds, divided exactly once.
    #[test]
    fn a_markers_position_is_milliseconds_and_reads_as_a_clock() {
        assert_eq!(format(0), "0:00");
        assert_eq!(format(600_000), "10:00");
        assert_eq!(format(3_723_000), "1:02:03");
        assert_eq!(build_position(95.4), 95_400);
        assert_eq!(build_position(0.0), 0);
        assert_eq!(build_position(f64::NAN), 0);
        assert_eq!(
            build_position(-3.0),
            0,
            "a negative position is not a marker"
        );
    }

    /// An optimistic add is visible at once, replaced by the server's row, and
    /// gone again when the server refused.
    #[test]
    fn an_optimistic_marker_is_reconciled_with_the_servers_answer() {
        let mut markers = Markers::new();
        let temporary = {
            // No runtime needed: the store's local half is what is asserted.
            markers.pending += 1;
            let temporary = format!("pending-{}", markers.pending);
            markers
                .by_episode
                .entry("e1".to_owned())
                .or_default()
                .push(marker(&temporary, 95_400));
            temporary
        };
        assert_eq!(markers.for_episode("e1").len(), 1);

        markers.added(
            "e1",
            &temporary,
            Ok(Marker {
                id: "srv-1".into(),
                ..marker("srv-1", 95_400)
            }),
        );
        let list = markers.for_episode("e1");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "srv-1");

        markers.added("e1", "srv-1", Err(ApiError::Offline));
        assert!(markers.for_episode("e1").is_empty(), "refused: gone again");
    }

    /// A show's markers arrive grouped, and the list is ordered by position
    /// whatever the server's order was.
    #[test]
    fn a_show_answer_is_grouped_sorted_and_empties_dropped() {
        let mut markers = Markers::new();
        markers.loading = true;
        markers.channel = Some("c1".into());
        let mut page = ShowMarkers::default();
        page.markers
            .insert("e1".into(), vec![marker("b", 200_000), marker("a", 10_000)]);
        page.markers.insert("e2".into(), Vec::new());
        markers.resolved("c1", Ok(page));
        assert!(markers.is_loaded());
        assert_eq!(
            markers
                .for_episode("e1")
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert!(markers.for_episode("e2").is_empty());
        assert!(!markers.wants("c1"), "loaded once is enough");
        assert!(markers.wants("c2"), "another show is another map");

        // An answer for a show the page left is dropped, not merged.
        markers.resolved("c2", Ok(ShowMarkers::default()));
        assert_eq!(markers.for_episode("e1").len(), 2);
    }
}
