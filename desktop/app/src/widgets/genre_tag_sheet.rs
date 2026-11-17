//! The genre tag sheet: one track, the vocabulary ranked for it, one pick.
//!
//! `components/GenreTagSheet.svelte` is the reference. It exists because tagging
//! is reachable from wherever the track is — the studio is for a pass, but a
//! wrong genre is noticed while listening or scrolling. The studio opens it from
//! a candidate (to correct a label the model contests), from a bulk preview row
//! (to fix one title before applying) and from the tree's node list.
//!
//! The ranking comes from the server (`GET /genre/track/<id>`, one call, no
//! reading of the whole labelled set for one row) and is displayed in the page's
//! own order (`genre_label::rank`). One tag per track: picking replaces, and
//! `Retirer l'étiquette` clears it with `genre_label(track, null)`.

use std::collections::BTreeMap;

use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement as _, Styled, div, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};

use crate::theme;
use crate::widgets::genre_label::{self, Scored, Tag};

/// What the sheet knows between two renders.
#[derive(Clone, Debug, Default)]
pub struct Sheet {
    pub open: bool,
    /// The track's row id (a UUID), the key `/genre/track/<id>` takes.
    pub track: String,
    pub title: String,
    /// The tag the track wears, when it wears one.
    pub current: Option<Tag>,
    /// The server's ranking for this track: `[label, score, head, proto]`.
    pub scores: Vec<crate::widgets::genre_label::ScoreRow>,
    pub query: String,
    pub loading: bool,
    pub saving: bool,
    pub error: Option<SharedString>,
}

impl Sheet {
    /// Open on one track; the caller asks `/genre/track/<id>` and hands the
    /// answer to [`Sheet::answered`].
    pub fn open(track: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            open: true,
            track: track.into(),
            title: title.into(),
            loading: true,
            ..Self::default()
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.loading = false;
        self.saving = false;
        self.error = None;
    }

    pub fn path(&self) -> String {
        format!("/genre/track/{}", crate::widgets::card::encode(&self.track))
    }

    /// The server's answer, typed: `nz_model::GenreTrack` carries the tag as an
    /// OBJECT (`_tag_json`) and the ranking as the server's own
    /// `[[label, score, head, proto]]` tuples, so the sheet keeps no parser of
    /// its own — it had one because the model declared a string and a struct.
    pub fn answered(&mut self, answer: &nz_model::GenreTrack) {
        self.loading = false;
        self.error = None;
        self.current = answer
            .tag
            .as_ref()
            .map(|tag| Tag {
                id: tag.id,
                name: tag.name.clone(),
                color: tag.color.clone(),
                archetype: tag.archetype.clone(),
                family: None,
            })
            .filter(|tag| !tag.name.is_empty());
        self.scores = scores_of(&answer.scores);
    }

    pub fn failed(&mut self, cause: impl Into<SharedString>) {
        self.loading = false;
        self.error = Some(cause.into());
    }

    /// One pick: the sheet remembers it at once (the screen sends the request),
    /// so the row reads as chosen before the server answered.
    pub fn picked(&mut self, tag: &Tag) {
        self.current = Some(tag.clone());
        self.saving = true;
        self.error = None;
    }
}

/// The server's ranking as the page's row type: one tuple per genre, best
/// first, capped like the SPA (`genre_label::SCORES_MAX`).
pub fn scores_of(scores: &[nz_model::GenreScore]) -> Vec<genre_label::ScoreRow> {
    scores
        .iter()
        .take(genre_label::SCORES_MAX)
        .map(|row| (row.label.clone(), row.score, row.head, row.proto))
        .collect()
}

/// What the sheet asks the screen to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Pick(Tag),
    Clear,
    Close,
}

/// The sheet, as an overlay the screen draws above its page.
///
/// `on` is the screen's one handler; every chip, the clear button and the close
/// button call it, so a click and a key end in the same place (D8a's rule for
/// the row menu).
pub fn view<V: Render>(
    sheet: &Sheet,
    tags: &[Tag],
    counts: &BTreeMap<String, u32>,
    cx: &mut Context<V>,
    on: impl Fn(&mut V, Action, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    let ranked = genre_label::rank(tags, &sheet.scores, counts);
    let filtered: Vec<&Scored> = if sheet.query.trim().is_empty() {
        ranked.iter().take(24).collect()
    } else {
        genre_label::search(&ranked, &sheet.query)
            .into_iter()
            .take(30)
            .collect()
    };
    let mut panel = div()
        .id("genre-tag-sheet")
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
                .id("genre-tag-sheet-panel")
                .relative()
                .flex()
                .flex_col()
                .gap(rems(0.75))
                .w(rems(34.))
                .max_h(rems(38.))
                .p(rems(1.25))
                .border_1()
                .border_color(theme::border(cx))
                .bg(theme::chrome(cx))
                // The panel swallows the click that would close the sheet.
                .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
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
                                        .text_size(rems(1.0))
                                        .font_weight(FontWeight::BOLD)
                                        .child("Étiqueter le genre"),
                                )
                                .child(
                                    div()
                                        .text_size(rems(0.75))
                                        .text_color(theme::secondary(cx))
                                        .child(SharedString::from(sheet.title.clone())),
                                ),
                        )
                        .child(
                            div()
                                .id("genre-tag-close")
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
                        ),
                ),
        );
    if let Some(current) = &sheet.current {
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child("Actuellement")
                .child(genre_label::chip(
                    "genre-tag-current",
                    current,
                    None,
                    true,
                    cx,
                ))
                .child(div().flex_1())
                .child(
                    button(
                        "genre-tag-clear",
                        "Retirer l'étiquette",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click({
                        let on = on.clone();
                        cx.listener(move |this, _, _, cx| on(this, Action::Clear, cx))
                    }),
                ),
        );
    }
    if sheet.loading {
        panel = panel.child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child("Classement du vocabulaire…"),
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
    if !sheet.loading {
        let mut grid = div().flex().flex_wrap().gap(rems(0.375));
        for row in filtered {
            let selected = sheet
                .current
                .as_ref()
                .is_some_and(|tag| tag.name == row.tag.name);
            let trailing = row
                .score
                .map(genre_label::percent)
                .or_else(|| (row.used > 0).then(|| row.used.to_string()));
            let tag = row.tag.clone();
            grid = grid.child(
                genre_label::chip(
                    format!("genre-tag-{}", row.tag.id.max(0)),
                    &row.tag,
                    trailing,
                    selected,
                    cx,
                )
                .on_click({
                    let on = on.clone();
                    cx.listener(move |this, _, _, cx| on(this, Action::Pick(tag.clone()), cx))
                }),
            );
        }
        panel = panel.child(grid);
    }
    panel = panel.child(
        div()
            .text_size(rems(0.6875))
            .text_color(theme::secondary(cx))
            .child(
                "Un seul genre par titre : c'est ce qui apprend au modèle que deux genres \
                 veulent dire deux sons.",
            ),
    );
    panel.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The route's answer as `nz_model::GenreTrack` decodes it: the tag is an
    /// OBJECT (`_tag_json`) and each score is the server's
    /// `[label, score, head, proto]` tuple, `head`/`proto` possibly `null`.
    #[test]
    fn the_servers_answer_becomes_the_sheets_rows() {
        let answer: nz_model::GenreTrack = serde_json::from_str(
            r##"{"track":"u",
                "tag":{"id":4,"name":"Hardtek","color":"#ff0000","archetype":"hard"},
                "scores":[["Hardtek",0.62,0.7,0.4],["Ambient",0.1,null,0.1]]}"##,
        )
        .expect("the route's body");
        let mut sheet = Sheet::open("u", "Titre");
        sheet.answered(&answer);
        assert_eq!(
            sheet.current.as_ref().map(|tag| tag.name.as_str()),
            Some("Hardtek")
        );
        assert_eq!(
            sheet.current.as_ref().and_then(|tag| tag.color.as_deref()),
            Some("#ff0000"),
            "the chip keeps the tag's colour"
        );
        assert_eq!(
            sheet.scores[0],
            ("Hardtek".to_owned(), 0.62, Some(0.7), Some(0.4))
        );
        assert_eq!(
            sheet.scores[1],
            ("Ambient".to_owned(), 0.1, None, Some(0.1))
        );
        assert!(!sheet.loading);
    }

    #[test]
    fn a_track_without_a_tag_says_so() {
        let mut sheet = Sheet::open("u", "Titre");
        let answer: nz_model::GenreTrack =
            serde_json::from_str(r#"{"track":"u","tag":null,"scores":[]}"#).expect("body");
        sheet.answered(&answer);
        assert!(sheet.current.is_none());
        assert!(sheet.scores.is_empty());
    }
}
