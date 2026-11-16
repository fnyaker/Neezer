//! Tagging a whole album, playlist or artist — after a REVIEW.
//!
//! `components/GenreBulkSheet.svelte` and `/genre/bulk/preview` are the
//! reference, and the review is the whole point: a Frenchcore album is
//! Frenchcore nearly always, and "nearly" — the intro, the collab, the closing
//! ballad — is exactly what a blind bulk tag would teach the head to confuse.
//! So the sheet is two steps: a PREVIEW that says, for every track in scope,
//! what it already wears and what the head thinks, with the disagreements
//! UNTICKED; then [`Sheet::selection`] is applied to the tracks the person kept.
//!
//! The server does the judging (`genre_bulk_preview`): the head's opinion is a
//! disagreement only past the analysis's own gate AND for a tag the head knows,
//! compared at family level when it is only sure of the family. This file never
//! re-decides any of that; it shows what the server said and lets a person
//! untick.
//!
//! Applying is ONE request with an explicit list (`genre_bulk`): an artist scope
//! previews every library row of that artist but is never applied by scope, so
//! what was reviewed is what is tagged.

use std::collections::BTreeMap;

use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement as _, Styled, div, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};
use serde_json::Value;

use crate::theme;
use crate::widgets::genre_label::Tag;

/// What a bulk request names (`GenreBulkSheet`'s scope).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScopeKind {
    #[default]
    Album,
    Playlist,
    Artist,
}

impl ScopeKind {
    pub fn label(self) -> &'static str {
        match self {
            ScopeKind::Album => "album",
            ScopeKind::Playlist => "playlist",
            ScopeKind::Artist => "artiste",
        }
    }
}

/// The scope as the screen knows it: the tracks the page shows, plus the artist
/// id for an artist scope (whose tracks the SERVER resolves).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scope {
    pub kind: ScopeKind,
    pub title: String,
    /// The tracks' row refs (a Deezer id or a local UUID), in page order.
    pub tracks: Vec<String>,
    /// The artist's ref, for an artist scope; `None` otherwise.
    pub artist: Option<String>,
}

impl Scope {
    /// The body `/genre/bulk/preview` takes: the SPA's own scope object plus the
    /// tag, exactly as `api.js` spreads it.
    pub fn body(&self, tag: &str) -> Value {
        let mut body = serde_json::json!({
            "kind": self.kind.label(),
            "title": self.title,
            "tag": tag,
        });
        if let Some(artist) = &self.artist {
            body["artist"] = Value::from(artist.clone());
        } else {
            body["tracks"] = Value::from(self.tracks.clone());
        }
        body
    }
}

/// One row of the preview: what the track wears, what the head hears, and
/// whether the two disagree with the tag being applied.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// The tag the track already wears, if any.
    pub current: Option<String>,
    /// The head's own answer, when it had one.
    pub model: Option<Model>,
    /// `"tag"`, `"model"` or `None` — the server's `disagree` field.
    pub disagree: Option<String>,
    /// Already wears the tag being applied: nothing would change.
    pub same: bool,
    pub ticked: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub label: String,
    pub confidence: f64,
    /// `"label"`, `"family"` or `"ancestor"`.
    pub level: String,
}

/// The preview's totals, in the server's own words.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Counts {
    pub tracks: u32,
    pub missing: u32,
    pub same: u32,
    pub other_tag: u32,
    pub model: u32,
    pub no_vector: u32,
    pub unjudged: u32,
}

/// The sheet's own state.
#[derive(Clone, Debug, Default)]
pub struct Sheet {
    pub open: bool,
    pub scope: Scope,
    pub tag: Option<Tag>,
    pub rows: Vec<Row>,
    pub counts: Counts,
    pub model_active: bool,
    pub model_knows: bool,
    pub loading: bool,
    pub applying: bool,
    pub error: Option<SharedString>,
    /// What the server answered once the tag was applied.
    pub applied: Option<u32>,
}

impl Sheet {
    pub fn open(scope: Scope, tag: Tag) -> Self {
        Self {
            open: true,
            scope,
            tag: Some(tag),
            loading: true,
            ..Self::default()
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.loading = false;
        self.applying = false;
    }

    /// The tracks that will be sent: the ticked ones, in page order. The
    /// server's `genre_bulk` takes an explicit list and nothing else.
    pub fn selection(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter(|row| row.ticked && !row.same)
            .map(|row| row.id.clone())
            .collect()
    }

    pub fn toggle(&mut self, index: usize) {
        if let Some(row) = self.rows.get_mut(index)
            && !row.same
        {
            row.ticked = !row.ticked;
        }
    }

    /// The preview's answer. The server sends `{tag, tracks, counts, model}`;
    /// every disagreement starts UNTICKED (the review's whole purpose).
    pub fn answered(&mut self, body: &Value) {
        self.loading = false;
        self.error = None;
        self.tag = body
            .get("tag")
            .and_then(|tag| tag.as_object())
            .map(|tag| Tag {
                id: tag.get("id").and_then(Value::as_i64).unwrap_or(0),
                name: tag
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                color: tag.get("color").and_then(Value::as_str).map(str::to_owned),
                archetype: tag
                    .get("archetype")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                family: None,
            })
            .or_else(|| self.tag.take());
        self.rows = body
            .get("tracks")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(read_row).collect())
            .unwrap_or_default();
        let counts = body.get("counts").cloned().unwrap_or(Value::Null);
        let n = |key: &str| {
            counts
                .get(key)
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(u32::MAX as u64) as u32
        };
        self.counts = Counts {
            tracks: n("tracks"),
            missing: n("missing"),
            same: n("same"),
            other_tag: n("other_tag"),
            model: n("model"),
            no_vector: n("no_vector"),
            unjudged: n("unjudged"),
        };
        let model = body.get("model").cloned().unwrap_or(Value::Null);
        self.model_active = model
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.model_knows = model.get("knows").and_then(Value::as_bool).unwrap_or(false);
    }

    pub fn failed(&mut self, cause: impl Into<SharedString>) {
        self.loading = false;
        self.error = Some(cause.into());
    }

    /// The apply answer (`{tag, applied, missing}`).
    pub fn applied(&mut self, body: &Value) {
        self.applying = false;
        self.applied = Some(
            body.get("applied")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(u32::MAX as u64) as u32,
        );
        self.rows.clear();
    }
}

fn read_row(value: &Value) -> Option<Row> {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let id = value.get("id").and_then(Value::as_str)?.to_owned();
    let same = value.get("same").and_then(Value::as_bool).unwrap_or(false);
    let disagree = value
        .get("disagree")
        .and_then(Value::as_str)
        .map(str::to_owned);
    // A disagreement is unticked; a track already wearing the tag cannot be
    // ticked either (there is nothing to do).
    let ticked = !same && disagree.is_none();
    let model = value
        .get("model")
        .and_then(Value::as_object)
        .map(|model| Model {
            label: model
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            confidence: model.get("conf").and_then(Value::as_f64).unwrap_or(0.),
            level: model
                .get("level")
                .and_then(Value::as_str)
                .unwrap_or("label")
                .to_owned(),
        });
    Some(Row {
        id,
        title: text("title"),
        artist: text("artist"),
        album: text("album"),
        current: value
            .get("current")
            .and_then(Value::as_str)
            .map(str::to_owned),
        model,
        disagree,
        same,
        ticked,
    })
}

/// What the sheet asks the screen to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Toggle(usize),
    Apply,
    Close,
    /// Correct one track through the tag sheet, which is the review's other
    /// verdict ("this one is not what the album says").
    Correct(usize),
}

/// The bulk sheet as an overlay. The screen paints it above its page and owns
/// the one request per action.
pub fn view<V: Render>(
    sheet: &Sheet,
    cx: &mut Context<V>,
    on: impl Fn(&mut V, Action, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    let tag = sheet.tag.clone().unwrap_or_default();
    let counts = &sheet.counts;
    let header = div()
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
                        .text_size(rems(1.0))
                        .font_weight(FontWeight::BOLD)
                        .child(format!(
                            "Étiqueter {} « {} »",
                            sheet.scope.kind.label(),
                            sheet.scope.title
                        )),
                )
                .child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child(format!(
                            "{} titre(s) · revue avant application · « {} »",
                            counts.tracks, tag.name
                        )),
                ),
        )
        .child(
            div()
                .id("genre-bulk-close")
                .cursor_pointer()
                .on_click({
                    let on = on.clone();
                    cx.listener(move |this, _, _, cx| on(this, Action::Close, cx))
                })
                .child(
                    icon(IconName::Close)
                        .size(rems(1.))
                        .text_color(theme::secondary(cx)),
                ),
        );
    let mut panel = div()
        .id("genre-bulk-sheet")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::shell(cx).opacity(0.72))
        .on_mouse_down(gpui_kit::MouseButton::Left, {
            let on = on.clone();
            cx.listener(move |this, _, _, cx| on(this, Action::Close, cx))
        })
        .child(
            div()
                .id("genre-bulk-panel")
                .relative()
                .flex()
                .flex_col()
                .gap(rems(0.75))
                .w(rems(42.))
                .max_h(rems(40.))
                .p(rems(1.25))
                .border_1()
                .border_color(theme::border(cx))
                .bg(theme::chrome(cx))
                .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
                .child(header),
        );
    if sheet.loading {
        panel = panel.child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child("Le serveur compare chaque titre au modèle…"),
        );
    }
    if let Some(error) = &sheet.error {
        panel = panel.child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::danger(cx))
                .child(error.clone()),
        );
    }
    if let Some(applied) = sheet.applied {
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .text_size(rems(0.875))
                .text_color(theme::success(cx))
                .child(icon(IconName::CircleCheck).size(rems(1.125)))
                .child(format!("{} titre(s) étiqueté(s) « {} »", applied, tag.name)),
        );
    }
    if !sheet.rows.is_empty() {
        let mut list = div()
            .id("genre-bulk-rows")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        for (index, row) in sheet.rows.iter().enumerate() {
            let reason: (SharedString, _) = match (&row.disagree, row.same, &row.current) {
                (_, true, _) => ("Déjà étiqueté".into(), theme::secondary(cx)),
                (Some(kind), _, Some(current)) if kind == "tag" => {
                    (format!("Porte « {current} »").into(), theme::warning(cx))
                }
                (Some(_), _, Some(current)) => (
                    format!("Porte « {current} » · le modèle n'est pas d'accord").into(),
                    theme::warning(cx),
                ),
                (Some(_), _, None) => {
                    let label = row
                        .model
                        .as_ref()
                        .map(|model| model.label.clone())
                        .unwrap_or_else(|| "autre genre".to_owned());
                    (
                        format!("Le modèle entend « {label} »").into(),
                        theme::warning(cx),
                    )
                }
                (None, false, None) => ("Rien d'autre".into(), theme::secondary(cx)),
                (None, false, Some(current)) => {
                    (format!("Porte « {current} »").into(), theme::secondary(cx))
                }
            };
            let mut line = div()
                .id(("genre-bulk-row", index))
                .flex()
                .items_center()
                .gap(rems(0.5))
                .py(rems(0.25))
                .px(rems(0.25))
                .border_b_1()
                .border_color(theme::border(cx))
                .text_size(rems(0.75))
                .cursor_pointer()
                .hover(|style| style.bg(theme::selected(cx)))
                .on_click({
                    let on = on.clone();
                    cx.listener(move |this, _, _, cx| on(this, Action::Toggle(index), cx))
                })
                .child(
                    div()
                        .flex_shrink_0()
                        .w(rems(0.75))
                        .h(rems(0.75))
                        .border_1()
                        .border_color(if row.ticked {
                            theme::accent(cx)
                        } else {
                            theme::border(cx)
                        })
                        .bg(if row.ticked {
                            theme::accent(cx)
                        } else {
                            theme::chrome(cx)
                        })
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(if row.ticked {
                            icon(IconName::Check)
                                .size(rems(0.5))
                                .text_color(theme::shell(cx))
                                .into_any_element()
                        } else {
                            div().into_any_element()
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(rems(0.0625))
                        .child(
                            div()
                                .text_color(theme::text(cx))
                                .child(SharedString::from(row.title.clone())),
                        )
                        .child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::secondary(cx))
                                .child(SharedString::from(row.artist.clone())),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(rems(0.6875))
                        .text_color(reason.1)
                        .child(reason.0),
                )
                .child(
                    button("genre-bulk-correct", "Corriger", ButtonVariant::Outline, cx).on_click(
                        {
                            let on = on.clone();
                            cx.listener(move |this, _, _, cx| on(this, Action::Correct(index), cx))
                        },
                    ),
                );
            // A `same` row is not a button: there is nothing to change.
            if row.same {
                line = line.opacity(0.6);
            }
            list = list.child(line);
        }
        panel = panel.child(list);
        let mut footer = div()
            .flex()
            .items_center()
            .gap(rems(0.5))
            .text_size(rems(0.6875))
            .text_color(theme::secondary(cx))
            .child(format!(
                "{} sélectionné(s) · {} déjà étiqueté(s) · {} désaccord(s) · {} hors bibliothèque",
                sheet.selection().len(),
                counts.same,
                counts.other_tag + counts.model,
                counts.missing
            ));
        if counts.no_vector > 0 {
            footer = footer.child(format!(" · {} sans empreinte", counts.no_vector));
        }
        if !sheet.model_active {
            footer = footer.child(" · aucun modèle : seule l'étiquette existante est comparée");
        } else if !sheet.model_knows {
            footer = footer.child(" · le modèle ne connaît pas encore ce genre");
        }
        panel = panel.child(footer);
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(div().flex_1())
                .child(
                    button("genre-bulk-cancel", "Fermer", ButtonVariant::Outline, cx).on_click({
                        let on = on.clone();
                        cx.listener(move |this, _, _, cx| on(this, Action::Close, cx))
                    }),
                )
                .child(
                    button(
                        "genre-bulk-apply",
                        format!("Étiqueter {}", sheet.selection().len()),
                        ButtonVariant::Primary,
                        cx,
                    )
                    .disabled(sheet.selection().is_empty() || sheet.applying)
                    .on_click({
                        let on = on.clone();
                        cx.listener(move |this, _, _, cx| on(this, Action::Apply, cx))
                    }),
                ),
        );
    }
    panel.into_any_element()
}

/// The applies' own counts, kept as one map so a test reads them without a
/// render: the server answers `{applied, missing}` and both are shown.
pub fn applied_counts(body: &Value) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    for key in ["applied", "missing"] {
        out.insert(
            key.to_owned(),
            body.get(key)
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(u32::MAX as u64) as u32,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview() -> Value {
        serde_json::json!({
            "tag": {"id": 7, "name": "Frenchcore", "color": "#aa22ff", "archetype": "hard"},
            "counts": {"tracks": 3, "missing": 1, "same": 1, "other_tag": 1, "model": 0},
            "model": {"active": true, "knows": true},
            "tracks": [
                {"id": "a", "title": "Un", "artist": "X", "album": "A", "current": null,
                 "disagree": null, "same": false},
                {"id": "b", "title": "Deux", "artist": "X", "album": "A",
                 "current": "Frenchcore", "disagree": null, "same": true},
                {"id": "c", "title": "Trois", "artist": "X", "album": "A",
                 "current": "Hardtek", "disagree": "tag", "same": false}
            ]
        })
    }

    /// The review's rule: a disagreement is unticked, a track already wearing the
    /// tag cannot be ticked, and only the kept rows are sent.
    #[test]
    fn a_disagreement_starts_unticked_and_is_never_sent() {
        let mut sheet = Sheet::open(
            Scope {
                kind: ScopeKind::Album,
                title: "A".to_owned(),
                tracks: vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
                artist: None,
            },
            Tag::new(7, "Frenchcore"),
        );
        sheet.answered(&preview());
        assert_eq!(sheet.rows.len(), 3);
        assert!(sheet.rows[0].ticked);
        assert!(!sheet.rows[1].ticked, "already wears it");
        assert!(!sheet.rows[2].ticked, "another tag is a disagreement");
        assert_eq!(sheet.selection(), vec!["a".to_owned()]);
        assert_eq!(sheet.counts.missing, 1);
        // Ticking the disagreement is possible — the person knows better.
        sheet.toggle(2);
        assert_eq!(sheet.selection(), vec!["a".to_owned(), "c".to_owned()]);
        // A `same` row stays unticked whatever the click.
        sheet.toggle(1);
        assert!(!sheet.rows[1].ticked);
    }

    #[test]
    fn the_preview_body_is_the_spas_scope_object() {
        let scope = Scope {
            kind: ScopeKind::Playlist,
            title: "Nuit".to_owned(),
            tracks: vec!["1".to_owned(), "2".to_owned()],
            artist: None,
        };
        let body = scope.body("Hardtek");
        assert_eq!(body["kind"], "playlist");
        assert_eq!(body["tag"], "Hardtek");
        assert_eq!(body["tracks"], serde_json::json!(["1", "2"]));
        let artist = Scope {
            kind: ScopeKind::Artist,
            title: "X".to_owned(),
            tracks: vec!["1".to_owned()],
            artist: Some("42".to_owned()),
        };
        let body = artist.body("Hardtek");
        assert_eq!(body["artist"], "42");
        assert!(
            body.get("tracks").is_none(),
            "an artist scope sends the artist"
        );
    }

    #[test]
    fn an_apply_answer_says_how_many_landed_and_how_many_were_absent() {
        let counts = applied_counts(&serde_json::json!({"applied": 12, "missing": 2}));
        assert_eq!(counts["applied"], 12);
        assert_eq!(counts["missing"], 2);
    }
}
