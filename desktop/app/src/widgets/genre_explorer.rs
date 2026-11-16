//! Explorer's parent styles: what this person actually listens to, as shelves
//! of the genre tree, and one style's discovery at a chosen breadth.
//!
//! `components/ExplorerStyles.svelte`, `lib/genrestation.js`,
//! `webui/explorer.py` and `deezer/explorer.discover_style` are the reference.
//! A style is never a terminal leaf (a leaf's name alone says nothing about the
//! shelf behind it): the server sends the parent styles with their weight and
//! the leaf below, and this panel merges leaves that name the same parent.
//!
//! Two requests and one rule:
//!
//! - `GET /explorer/styles` once, when the panel first paints, kept for the
//!   session — the list moves with the library, not with a click.
//! - `POST /explorer/discover {style, level}` only when the person presses
//!   Découvrir, then `GET /explorer/discover?style=&level=` every 2 s while the
//!   server works. **A GET never starts work** (the server answers `idle` when
//!   nothing is cached or in flight, and only a POST that starts a worker spends
//!   the person's budget): polling with a POST would be a request storm.
//! - The breadth is one of five notches (`style_limits`): 1 stays inside the
//!   style, 5 reaches everything Deezer relates to it. The names and hints are
//!   the page's own.
//!
//! In the SPA this panel lives on the Search page; D16's file list gives the
//! desktop one `genre_explorer.rs` and no `search.rs`, so the studio carries it
//! as its fifth tab. That is a placement difference, not a feature one.

use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement as _, Styled, div, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};
use serde_json::Value;

use crate::theme;

/// The five breadth notches, in the page's words (`ExplorerStyles.svelte#LEVELS`).
pub const LEVELS: [(u8, &str, &str); 5] = [
    (
        1,
        "Prudent",
        "Des playlists qui restent dans le style, sans surprise.",
    ),
    (2, "Mesuré", "Le style, avec quelques voisins immédiats."),
    (3, "Équilibré", "Le style et ce qui l'entoure."),
    (4, "Curieux", "Le style, ses voisins et des passerelles."),
    (5, "Large", "Tout ce que Deezer rapproche de ce style."),
];

/// How long a poll waits, and how many polls before the panel stops asking on
/// its own (two minutes: a real Deezer sweep on a slow link, per the page).
pub const POLL_MS: u64 = 2_000;
pub const MAX_POLLS: u32 = 60;

/// One parent style, as `/explorer/styles` sends it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub name: String,
    pub tracks: u32,
    pub weight: f64,
    pub confidence: f64,
    pub leaf: bool,
    pub children: Vec<String>,
    pub parent: Option<String>,
    pub family: Option<String>,
}

impl Style {
    pub fn read(value: &Value) -> Option<Self> {
        let name = value.get("name")?.as_str()?.trim();
        if name.is_empty() {
            return None;
        }
        let parent = value
            .get("parent")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|parent| !parent.is_empty())
            .map(str::to_owned);
        let family = value
            .get("family")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|family| !family.is_empty())
            .map(str::to_owned);
        let leaf = value.get("leaf").and_then(Value::as_bool).unwrap_or(false);
        // The page's own rule: a leaf is shown as its parent (or family), and a
        // leaf with neither is not a way in.
        let shown = if leaf {
            parent.clone().or_else(|| family.clone())?
        } else {
            name.to_owned()
        };
        Some(Self {
            name: shown,
            tracks: value.get("tracks").and_then(Value::as_u64).unwrap_or(0) as u32,
            weight: value.get("weight").and_then(Value::as_f64).unwrap_or(0.),
            confidence: value
                .get("confidence")
                .and_then(Value::as_f64)
                .unwrap_or(0.),
            leaf,
            children: value
                .get("children")
                .and_then(Value::as_array)
                .map(|children| {
                    children
                        .iter()
                        .filter_map(|child| child.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            parent,
            family,
        })
    }
}

/// The page's `parentStyles`: leaves merge into the parent they name, the list
/// ends most-listened first.
pub fn parent_styles(values: &[Value]) -> Vec<Style> {
    let mut merged: Vec<Style> = Vec::new();
    for value in values {
        let Some(style) = Style::read(value) else {
            continue;
        };
        match merged.iter_mut().find(|row| row.name == style.name) {
            Some(row) => {
                row.tracks += style.tracks;
                row.weight += style.weight;
                row.leaf |= style.leaf;
                let leaf_name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if leaf_name != row.name {
                    row.children.push(leaf_name);
                }
            }
            None => merged.push(style),
        }
    }
    merged.sort_by(|a, b| b.tracks.cmp(&a.tracks).then(a.name.cmp(&b.name)));
    merged
}

/// A discovered playlist: what the server verified of it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playlist {
    pub id: String,
    pub title: String,
    pub cover: Option<String>,
    pub nb_tracks: u32,
    /// How many of its titles the engine actually heard in the style — the only
    /// number worth printing (`matched` is `null` when nothing was checked).
    pub matched: Option<u32>,
    /// The kept titles, in the provider's shape: `deezer_id`, title, artist,
    /// album, duration.
    pub tracks: Vec<Value>,
}

impl Playlist {
    fn read(value: &Value) -> Option<Self> {
        let id = value.get("id").and_then(Value::as_str)?.to_owned();
        let title = value
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or("Playlist sans titre")
            .to_owned();
        Some(Self {
            id,
            title,
            cover: value
                .get("cover")
                .and_then(Value::as_str)
                .map(str::to_owned),
            nb_tracks: value.get("nb_tracks").and_then(Value::as_u64).unwrap_or(0) as u32,
            matched: value
                .get("matched")
                .and_then(Value::as_u64)
                .map(|n| n as u32),
            tracks: value
                .get("tracks")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        })
    }

    /// `« 3 vérifiés sur 40 »`, or the raw count when nothing was checked.
    pub fn verified_label(&self) -> String {
        let count = |n: u32| format!("{n} titre{}", if n == 1 { "" } else { "s" });
        match self.matched {
            None => {
                if self.nb_tracks > 0 {
                    count(self.nb_tracks)
                } else {
                    "Titres vérifiés".to_owned()
                }
            }
            Some(matched) if self.nb_tracks > matched => {
                format!("{} vérifiés sur {}", count(matched), self.nb_tracks)
            }
            Some(matched) => format!("{} vérifiés", count(matched)),
        }
    }
}

/// The panel's own state.
#[derive(Clone, Debug, Default)]
pub struct Explorer {
    pub loading: bool,
    pub loaded: bool,
    pub styles: Vec<Style>,
    pub selected: usize,
    pub level: u8,
    pub error: Option<SharedString>,
    /// Where the server is: `"idle"`, `"pending"`, `"ready"`, `"error"`.
    pub state: String,
    pub playlists: Vec<Playlist>,
    pub discovery_error: Option<String>,
    pub polls: u32,
    pub asking: bool,
}

impl Explorer {
    pub fn new() -> Self {
        Self {
            level: 3,
            ..Self::default()
        }
    }

    pub fn answered_styles(&mut self, body: &Value) {
        self.loading = false;
        self.loaded = true;
        self.error = None;
        self.styles = body
            .as_array()
            .map(|values| parent_styles(values))
            .unwrap_or_default();
        if self.selected >= self.styles.len() {
            self.selected = 0;
        }
    }

    pub fn failed_styles(&mut self, cause: impl Into<SharedString>) {
        self.loading = false;
        self.error = Some(cause.into());
    }

    pub fn style(&self) -> Option<&Style> {
        self.styles.get(self.selected)
    }

    /// `GET /explorer/discover?style=&level=`, the poll the panel makes while a
    /// sweep runs. Percent-encoded by `card::encode` (an accent in a style name
    /// must land on the same route).
    pub fn discover_path(&self) -> Option<String> {
        let style = self.style()?;
        Some(format!(
            "/explorer/discover?style={}&level={}",
            crate::widgets::card::encode(&style.name),
            self.level
        ))
    }

    /// `POST /explorer/discover`: the ONE call that may start work.
    pub fn discover_body(&self) -> Option<Value> {
        let style = self.style()?;
        Some(serde_json::json!({"style": style.name, "level": self.level}))
    }

    /// The server's answer, whether it came from the poll or the POST.
    pub fn answered(&mut self, body: &Value) {
        self.state = body
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("ready")
            .to_owned();
        self.discovery_error = body.get("error").and_then(Value::as_str).map(str::to_owned);
        match self.state.as_str() {
            "pending" => {
                self.asking = false;
            }
            "idle" => {
                self.asking = false;
                // The panel asked and the server has nothing: say so rather than
                // spin for two minutes (rule 3's endless spinner).
                if self.playlists.is_empty() {
                    self.discovery_error = Some("aucune découverte en cours".to_owned());
                }
            }
            _ => {
                self.asking = false;
                self.playlists = body
                    .get("playlists")
                    .and_then(Value::as_array)
                    .map(|rows| rows.iter().filter_map(Playlist::read).collect())
                    .unwrap_or_default();
            }
        }
    }

    pub fn failed_discover(&mut self, cause: impl Into<SharedString>) {
        self.asking = false;
        self.discovery_error = Some(cause.into().to_string());
    }

    /// The next poll is due: the server is still working and the panel has not
    /// spent its budget of polls.
    pub fn wants_poll(&self) -> bool {
        self.state == "pending" && self.polls < MAX_POLLS
    }

    pub fn polled(&mut self) {
        self.polls += 1;
    }

    pub fn selected_name(&self) -> Option<&str> {
        self.style().map(|style| style.name.as_str())
    }
}

/// What the panel asks the screen to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Select(usize),
    Level(u8),
    Discover,
    /// A poll is due (the screen owns the timer).
    Poll,
    /// Start a station on the selected style (the server's genre radio).
    Station,
    /// Play one found playlist's verified titles.
    Play(usize),
    Reload,
}

/// The panel.
pub fn view<V: Render>(
    explorer: &Explorer,
    cx: &mut Context<V>,
    on: impl Fn(&mut V, Action, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    let mut column = div()
        .id("genre-explorer")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .gap(rems(0.75))
        .child(
            div()
                .flex()
                .items_start()
                .gap(rems(0.5))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.125))
                        .flex_1()
                        .child(
                            div()
                                .text_size(rems(0.875))
                                .font_weight(FontWeight::BOLD)
                                .child("Explorer les styles"),
                        )
                        .child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::secondary(cx))
                                .child(
                                    "Les styles que vous écoutez, et ce que Deezer en \
                                     rapproche — mesuré par le serveur, jamais supposé.",
                                ),
                        ),
                )
                .child(
                    button(
                        "genre-explorer-reload",
                        "Recharger",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click({
                        let on = on.clone();
                        cx.listener(move |this, _, _, cx| on(this, Action::Reload, cx))
                    }),
                ),
        );
    if explorer.loading && !explorer.loaded {
        column = column.child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child("Chargement des styles…"),
        );
    }
    if let Some(error) = &explorer.error {
        column = column.child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::danger(cx))
                .child(error.clone()),
        );
    }
    if explorer.loaded && explorer.styles.is_empty() {
        column = column.child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child(
                    "Aucun style connu pour l'instant : les styles viennent de l'historique \
                     d'écoute et des verdicts du serveur.",
                ),
        );
    }
    if !explorer.styles.is_empty() {
        let mut tiles = div().flex().flex_wrap().gap(rems(0.375));
        for (index, style) in explorer.styles.iter().enumerate() {
            let selected = index == explorer.selected;
            let name = style.name.clone();
            tiles = tiles.child(
                div()
                    .id(("genre-explorer-style", index))
                    .flex()
                    .items_center()
                    .gap(rems(0.375))
                    .px(rems(0.5))
                    .py(rems(0.25))
                    .border_1()
                    .border_color(if selected {
                        theme::accent(cx)
                    } else {
                        theme::border(cx)
                    })
                    .bg(if selected {
                        theme::selected(cx)
                    } else {
                        theme::chrome(cx)
                    })
                    .text_size(rems(0.75))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::selected(cx)))
                    .on_click({
                        let on = on.clone();
                        cx.listener(move |this, _, _, cx| on(this, Action::Select(index), cx))
                    })
                    .child(SharedString::from(name))
                    .child(
                        div()
                            .text_size(rems(0.625))
                            .font_family(theme::mono(cx))
                            .text_color(theme::secondary(cx))
                            .child(format!("{}", style.tracks)),
                    ),
            );
        }
        column = column.child(tiles);
    }
    // The breadth: five notches, the words the person reads.
    let mut levels = div().flex().flex_wrap().items_center().gap(rems(0.375));
    for (number, name, _hint) in LEVELS {
        let selected = explorer.level == number;
        levels = levels.child(
            div()
                .id(("genre-explorer-level", number as usize))
                .px(rems(0.5))
                .py(rems(0.25))
                .border_1()
                .border_color(if selected {
                    theme::accent(cx)
                } else {
                    theme::border(cx)
                })
                .bg(if selected {
                    theme::selected(cx)
                } else {
                    theme::chrome(cx)
                })
                .text_size(rems(0.75))
                .cursor_pointer()
                .on_click({
                    let on = on.clone();
                    cx.listener(move |this, _, _, cx| on(this, Action::Level(number), cx))
                })
                .child(format!("{number} · {name}")),
        );
    }
    column = column.child(
        div()
            .flex()
            .items_center()
            .gap(rems(0.5))
            .flex_wrap()
            .child(levels)
            .child(div().flex_1())
            .child(
                button(
                    "genre-explorer-station",
                    "Lancer une station",
                    ButtonVariant::Outline,
                    cx,
                )
                .disabled(explorer.style().is_none())
                .on_click({
                    let on = on.clone();
                    cx.listener(move |this, _, _, cx| on(this, Action::Station, cx))
                }),
            )
            .child(
                button(
                    "genre-explorer-discover",
                    if explorer.state == "pending" {
                        "Recherche en cours…"
                    } else {
                        "Découvrir"
                    },
                    ButtonVariant::Primary,
                    cx,
                )
                .disabled(explorer.style().is_none() || explorer.state == "pending")
                .on_click({
                    let on = on.clone();
                    cx.listener(move |this, _, _, cx| on(this, Action::Discover, cx))
                }),
            ),
    );
    if explorer.state == "pending" {
        column = column.child(
            div()
                .text_size(rems(0.6875))
                .text_color(theme::secondary(cx))
                .child(format!(
                    "Le serveur écoute huit extraits de 30 s, à son rythme — il cède dès que \
                     vous écoutez quelque chose. Sondage {} / {}.",
                    explorer.polls, MAX_POLLS
                )),
        );
    }
    if let Some(error) = &explorer.discovery_error {
        let message = match error.as_str() {
            "provider" => "Deezer est injoignable : réessayez quand la connexion revient.",
            "failed" => "La découverte a échoué côté serveur.",
            other => other,
        };
        column = column.child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::warning(cx))
                .child(SharedString::from(message.to_owned())),
        );
    }
    if !explorer.playlists.is_empty() {
        let mut results = div()
            .id("genre-explorer-playlists")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .border_1()
            .border_color(theme::border(cx));
        for (index, playlist) in explorer.playlists.iter().enumerate() {
            results = results.child(
                div()
                    .id(("genre-explorer-playlist", index))
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .p(rems(0.5))
                    .border_b_1()
                    .border_color(theme::border(cx))
                    .bg(theme::chrome(cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(rems(0.0625))
                            .child(
                                div()
                                    .text_size(rems(0.8125))
                                    .child(SharedString::from(playlist.title.clone())),
                            )
                            .child(
                                div()
                                    .text_size(rems(0.6875))
                                    .text_color(theme::secondary(cx))
                                    .child(playlist.verified_label()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(rems(0.625))
                            .font_family(theme::mono(cx))
                            .text_color(theme::secondary(cx))
                            .child(playlist.id.to_string()),
                    )
                    .child(
                        button(
                            ("genre-explorer-play", index),
                            "Écouter",
                            ButtonVariant::Primary,
                            cx,
                        )
                        .disabled(playlist.tracks.is_empty())
                        .on_click({
                            let on = on.clone();
                            cx.listener(move |this, _, _, cx| on(this, Action::Play(index), cx))
                        }),
                    ),
            );
        }
        column = column.child(results);
    }
    if explorer.playlists.is_empty() && explorer.state == "ready" {
        column = column.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child(icon(IconName::Info).size(rems(1.)))
                .child("Rien de vérifié pour ce style et ce niveau : essayez un cran plus large."),
        );
    }
    column.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The page's merge rule: a leaf is shown as its parent, two leaves of one
    /// parent are one entry, and a leafless leaf is dropped.
    #[test]
    fn parent_styles_merge_leaves_and_sort_by_use() {
        let body = serde_json::json!([
            {"name": "Frenchcore", "leaf": true, "parent": "Hardcore", "family": "hardcore",
             "tracks": 12, "weight": 3.0},
            {"name": "Gabber", "leaf": true, "parent": "Hardcore", "tracks": 5, "weight": 1.0},
            {"name": "Jazz", "tracks": 20, "weight": 5.0, "family": "jazz"},
            {"name": "Boogie", "leaf": true, "tracks": 3},
            {"name": "  ", "tracks": 99}
        ]);
        let styles = parent_styles(body.as_array().expect("array"));
        let names: Vec<&str> = styles.iter().map(|style| style.name.as_str()).collect();
        assert_eq!(names, ["Jazz", "Hardcore"]);
        assert_eq!(styles[1].tracks, 17);
        // The page's merge only records the leaves that DIFFER from the shown
        // parent; the first leaf IS the row.
        assert_eq!(styles[1].children, vec!["Gabber"]);
        assert_eq!(styles[0].tracks, 20);
    }

    #[test]
    fn only_a_post_may_start_work_and_a_get_only_polls() {
        let mut explorer = Explorer::new();
        explorer.answered_styles(&serde_json::json!([{"name": "Hardcore", "tracks": 4}]));
        assert_eq!(
            explorer.discover_path().as_deref(),
            Some("/explorer/discover?style=Hardcore&level=3")
        );
        assert_eq!(
            explorer.discover_body(),
            Some(serde_json::json!({"style": "Hardcore", "level": 3}))
        );
        explorer.answered(&serde_json::json!({"state": "pending"}));
        assert!(explorer.wants_poll());
        explorer.answered(&serde_json::json!({"state": "idle"}));
        assert!(!explorer.wants_poll());
        assert_eq!(
            explorer.discovery_error.as_deref(),
            Some("aucune découverte en cours")
        );
    }

    #[test]
    fn a_discovered_playlist_says_what_was_verified() {
        let body = serde_json::json!({
            "state": "ready",
            "playlists": [
                {"id": "901", "title": "Hardcore classics", "nb_tracks": 40, "matched": 3,
                 "tracks": [{"deezer_id": "1", "title": "T", "artist": "A", "duration": 180}]},
                {"id": "902", "title": "Sans vérification", "nb_tracks": 20}
            ]
        });
        let mut explorer = Explorer::new();
        explorer.answered_styles(&serde_json::json!([{"name": "Hardcore", "tracks": 4}]));
        explorer.answered(&body);
        assert_eq!(explorer.state, "ready");
        assert_eq!(explorer.playlists.len(), 2);
        assert_eq!(
            explorer.playlists[0].verified_label(),
            "3 titres vérifiés sur 40"
        );
        assert_eq!(explorer.playlists[1].verified_label(), "20 titres");
        assert_eq!(explorer.playlists[0].tracks.len(), 1);
    }

    #[test]
    fn the_levels_are_the_pages_five_notches() {
        assert_eq!(
            LEVELS.map(|(_, name, _)| name),
            ["Prudent", "Mesuré", "Équilibré", "Curieux", "Large"]
        );
    }
}
