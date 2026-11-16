//! « Ajouter à une playlist » : the picker a row's menu (and the library's
//! toolbar) opens.
//!
//! It is the SPA's `PlaylistPicker` without the guesswork: the playlists come
//! from `/api/my/playlists` (cached, so it opens offline), the filter folds the
//! same way the screens' search does, a title can be typed to create a playlist
//! that already holds the tracks, and the one mutation is
//! `POST /api/playlist/<id>/tracks` with NO version — like the SPA, this sheet
//! adds without claiming a version it never read, so it can never overwrite a
//! reorder somebody else made.
//!
//! A track that is already in the chosen playlist is added again on purpose:
//! the server allows duplicates and the SPA does the same (`playlist.py`).

use std::future::Future;

use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::{
    AppContext as _, Context, ElementId, Entity, EventEmitter, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, Window, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon, icon_button, input};

use nz_api::ApiError;
use nz_model::Playlist;

use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;
use crate::widgets::sheets::{
    SheetEvent, backdrop, card, error_line, field_label, is_forbidden, is_gone, note_line,
    push_toast,
};

/// Fold text for a search: lower case, accents dropped — `lib/format.js#fold`,
/// the same comparison the playlist search uses.
pub fn fold(text: &str) -> String {
    // Lower case FIRST, then strip the accents: `char::to_lowercase` turns `É`
    // into `é`, which the table below then folds to `e`. Doing it the other way
    // round left `éte` for « Été » and the filter missed it.
    text.to_lowercase()
        .chars()
        .flat_map(|character| match character {
            'à' | 'â' | 'ä' | 'á' | 'ã' | 'å' => vec!['a'],
            'ç' => vec!['c'],
            'é' | 'è' | 'ê' | 'ë' => vec!['e'],
            'î' | 'ï' | 'í' | 'ì' => vec!['i'],
            'ô' | 'ö' | 'ó' | 'ò' | 'õ' => vec!['o'],
            'û' | 'ü' | 'ú' | 'ù' => vec!['u'],
            'ÿ' => vec!['y'],
            'ñ' => vec!['n'],
            'œ' => vec!['o', 'e'],
            'æ' => vec!['a', 'e'],
            other => other.to_lowercase().collect(),
        })
        .collect()
}

/// The playlists whose title matches `query` (all of them for an empty query).
pub fn matching<'a>(playlists: &'a [Playlist], query: &str) -> Vec<&'a Playlist> {
    let needle = fold(query.trim());
    playlists
        .iter()
        .filter(|playlist| needle.is_empty() || fold(&playlist.title).contains(&needle))
        .collect()
}

/// Run `future` on the network runtime and hand its answer back on the UI
/// thread (the sheets' one path).
fn spawn_answer<S, T, F, G>(cx: &mut Context<S>, state: &Entity<AppState>, future: F, done: G)
where
    S: Render + 'static,
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
    G: FnOnce(&mut S, T, &mut Context<S>) + 'static,
{
    let runtime = state.read(cx).runtime.clone();
    let task = runtime.spawn(future);
    cx.spawn(async move |this, cx| {
        if let Ok(answer) = task.await {
            this.update(cx, |this, cx| done(this, answer, cx)).ok();
        }
    })
    .detach();
}

/// The sheet.
pub struct PlaylistPicker {
    state: Entity<AppState>,
    /// The track references the server resolves (`trackkey.js#trackRef`).
    refs: Vec<String>,
    label: SharedString,
    playlists: Option<Vec<Playlist>>,
    query: Entity<InputState>,
    creating: Entity<InputState>,
    creating_open: bool,
    busy: bool,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SheetEvent> for PlaylistPicker {}

impl PlaylistPicker {
    pub fn new(
        state: Entity<AppState>,
        refs: Vec<String>,
        label: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Filtrer mes playlists"));
        let creating = cx.new(|cx| InputState::new(window, cx).placeholder("Nom de la playlist"));
        let mut subscriptions = vec![
            cx.subscribe(&query, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe(&creating, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.create(cx);
                }
            }),
        ];
        let _ = window;
        let mut this = Self {
            state,
            refs,
            label: label.into(),
            playlists: None,
            query,
            creating,
            creating_open: false,
            busy: false,
            error: None,
            _subscriptions: Vec::new(),
        };
        this.load(cx);
        this._subscriptions = std::mem::take(&mut subscriptions);
        this
    }

    /// How many tracks this sheet adds.
    pub fn count(&self) -> usize {
        self.refs.len()
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        // Cache first: this sheet opens offline too (the paths are cacheable).
        if let Some(cache) = self.state.read(cx).cache.as_ref()
            && let Some(owner) = api.owner()
            && let Some(body) = cache.get(&owner, "/me/playlists")
            && let Ok(list) = serde_json::from_str::<nz_model::PlaylistList>(&body)
        {
            self.playlists = Some(list.playlists);
        }
        spawn_answer(
            cx,
            &self.state.clone(),
            async move { api.my_playlists().await },
            |this, result, cx| {
                match result {
                    Ok(list) => this.playlists = Some(list.playlists),
                    Err(error) => {
                        if this.playlists.is_none() {
                            this.error = Some(error.cause_fr().into());
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    fn fail(&mut self, error: &ApiError, cx: &mut Context<Self>) {
        self.busy = false;
        if is_forbidden(error) {
            self.error = Some("Vous ne pouvez pas modifier cette playlist.".into());
        } else if is_gone(error) {
            self.error = Some("Cette playlist ne vous est plus accessible.".into());
        } else {
            self.error = Some(error.cause_fr().into());
        }
        cx.notify();
    }

    fn add(&mut self, playlist: &Playlist, cx: &mut Context<Self>) {
        if self.busy || self.refs.is_empty() {
            return;
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let id = playlist.id.as_str().to_owned();
        let request_id = id.clone();
        let title = playlist.title.clone();
        let refs = self.refs.clone();
        let count = refs.len();
        self.busy = true;
        self.error = None;
        cx.notify();
        spawn_answer(
            cx,
            &self.state.clone(),
            async move { api.add_to_playlist(&request_id, &refs, None).await },
            move |this, result, cx| {
                this.busy = false;
                match result {
                    Ok(_) => {
                        push_toast(
                            cx,
                            &this.state,
                            ToastKind::Info,
                            format!(
                                "{count} {} dans « {title} »",
                                if count > 1 {
                                    "titres ajoutés"
                                } else {
                                    "titre ajouté"
                                }
                            ),
                        );
                        cx.emit(SheetEvent::Added {
                            playlist: id.clone(),
                            count,
                        });
                        cx.emit(SheetEvent::Close);
                    }
                    Err(error) => this.fail(&error, cx),
                }
                cx.notify();
            },
        );
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let title = self.creating.read(cx).value().trim().to_owned();
        if title.is_empty() {
            self.error = Some("Indiquez un nom de playlist.".into());
            cx.notify();
            return;
        }
        let Some(api) = self.state.read(cx).api() else {
            return;
        };
        let refs = self.refs.clone();
        let count = refs.len();
        self.busy = true;
        self.error = None;
        cx.notify();
        spawn_answer(
            cx,
            &self.state.clone(),
            async move { api.create_playlist(&title, &refs, None).await },
            move |this, result, cx| {
                this.busy = false;
                match result {
                    Ok(playlist) => {
                        push_toast(
                            cx,
                            &this.state,
                            ToastKind::Info,
                            format!(
                                "Playlist « {} » créée avec {count} {}",
                                playlist.title,
                                if count > 1 { "titres" } else { "titre" }
                            ),
                        );
                        cx.emit(SheetEvent::Added {
                            playlist: playlist.id.as_str().to_owned(),
                            count,
                        });
                        cx.emit(SheetEvent::Close);
                    }
                    Err(error) => this.fail(&error, cx),
                }
                cx.notify();
            },
        );
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(SheetEvent::Close);
    }
}

impl Render for PlaylistPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.query.read(cx).value().to_string();
        let busy = self.busy;
        let error = self.error.clone();
        let creating_open = self.creating_open;
        let playlists = self.playlists.clone();

        let mut body = div()
            .id("picker-list")
            .flex()
            .flex_col()
            .gap(rems(0.25))
            .max_h(rems(16.))
            .overflow_y_scroll()
            .min_h_0();
        match &playlists {
            None => body = body.child(note_line("Chargement de vos playlists…", cx)),
            Some(list) if list.is_empty() => {
                body = body.child(note_line(
                    "Aucune playlist pour l'instant : créez-en une ci-dessous.",
                    cx,
                ));
            }
            Some(list) => {
                let shown = matching(list, &query);
                if shown.is_empty() {
                    body = body.child(note_line(
                        format!("Aucune playlist ne correspond à « {query} »."),
                        cx,
                    ));
                }
                for playlist in shown {
                    let target = playlist.clone();
                    body = body.child(
                        div()
                            .id(ElementId::Name(
                                format!("picker-playlist-{}", playlist.id.as_str()).into(),
                            ))
                            .flex()
                            .items_center()
                            .gap(rems(0.5))
                            .px(rems(0.375))
                            .py(rems(0.25))
                            .hover(|style| style.bg(theme::selected(cx)))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .flex_shrink_0()
                                    .size(px(28.))
                                    .bg(theme::sidebar(cx))
                                    .child(icon(IconName::ListMusic).size(rems(0.875))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(rems(0.75))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(playlist.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(rems(0.625))
                                            .text_color(theme::secondary(cx))
                                            .child(format!(
                                                "{} titre{}",
                                                playlist.nb_tracks,
                                                if playlist.nb_tracks > 1 { "s" } else { "" }
                                            )),
                                    ),
                            )
                            .child(
                                button(
                                    ElementId::Name(
                                        format!("picker-add-{}", playlist.id.as_str()).into(),
                                    ),
                                    "Ajouter",
                                    ButtonVariant::Outline,
                                    cx,
                                )
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| this.add(&target, cx))),
                            ),
                    );
                }
            }
        }

        let mut sheet = card(
            "picker-sheet",
            "Ajouter à une playlist",
            Some(self.label.clone()),
            26.,
            cx,
        )
        .child(input("picker-filter", &self.query, window, cx))
        .child(body);

        if creating_open {
            sheet = sheet
                .child(field_label("Nouvelle playlist", cx))
                .child(input("picker-new", &self.creating, window, cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.375))
                        .child(
                            button("picker-create", "Créer", ButtonVariant::Outline, cx)
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.create(cx))),
                        )
                        .child(
                            button(
                                "picker-create-cancel",
                                "Annuler",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.creating_open = false;
                                cx.notify();
                            })),
                        ),
                );
        } else {
            sheet = sheet.child(
                button(
                    "picker-new-open",
                    "Nouvelle playlist…",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.creating_open = true;
                    cx.notify();
                })),
            );
        }
        if let Some(error) = &error {
            sheet = sheet.child(error_line(error, cx));
        }
        sheet = sheet.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.375))
                .child(div().flex_1())
                .child(
                    icon_button(
                        "picker-close",
                        IconName::Close,
                        "Fermer",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                ),
        );

        backdrop("picker-backdrop", cx)
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close(cx)),
            )
            .child(
                div()
                    .id("picker-plate")
                    .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation()
                    })
                    .child(sheet),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playlist(title: &str) -> Playlist {
        Playlist {
            id: nz_model::PlaylistId::new(title),
            title: title.to_owned(),
            ..Playlist::default()
        }
    }

    #[test]
    fn the_filter_ignores_case_and_accents() {
        let lists = vec![
            playlist("Été"),
            playlist("Route de nuit"),
            playlist("Chambre"),
        ];
        assert_eq!(matching(&lists, "ete").len(), 1, "Été matches « ete »");
        assert_eq!(matching(&lists, "ÉTÉ").len(), 1);
        assert_eq!(matching(&lists, "nuit").len(), 1);
        assert_eq!(matching(&lists, "  ").len(), 3, "an empty query shows all");
        assert_eq!(matching(&lists, "zzz").len(), 0);
    }
}
