//! The frozen extractor's card: is it installed, what does it read, and the
//! self-test that proves it is not producing healthy-looking nonsense.
//!
//! `components/GenreExtractor.svelte` and `webui/genre.py`'s extractor routes are
//! the reference. The big model is a third-party artefact with its own licence
//! (CC BY-NC-ND): it is never vendored and never fetched silently, so the
//! operator supplies a copy they obtained themselves. The page takes it from a
//! file picker; a desktop has no browser picker, so the card takes an absolute
//! PATH (`desktop/` has no file-dialog dependency) and uploads it once, in
//! `Lane::Bulk` — installing a model is not the action a person is waiting on.
//!
//! What this card must never do is imply that "it loaded" means "it works". The
//! failure mode is a front-end/model mismatch that embeds healthy vectors which
//! mean nothing, so the card offers the server's own self-test (`two halves of
//! the same track must embed closer than two different tracks`) and shows each
//! line it printed, plus the reason when there is nothing to test.

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement as _, Styled, div, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};
use nz_api::SessionJar;
use nz_model::ExtractorInfo;

use crate::theme;

/// The card's own state, beside the server's `extractor` object.
#[derive(Clone, Debug, Default)]
pub struct Card {
    /// The `.onnx` path as typed: file paths are the one thing a desktop keeps
    /// in a field rather than a picker here.
    pub path: String,
    pub uploading: bool,
    pub deleting: bool,
    pub test_running: bool,
    pub test_ok: Option<bool>,
    pub test_error: Option<SharedString>,
    pub test_progress: Vec<String>,
    pub error: Option<SharedString>,
    /// The self-test's own verdict, one sentence.
    pub result: Option<String>,
}

/// What the card asks the screen to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Upload the path in the field.
    Upload,
    Delete,
    Test,
    /// Poll `/genre/extractor/test` while it runs.
    Poll,
}

impl Card {
    pub fn answered_upload(&mut self, body: &serde_json::Value) {
        self.uploading = false;
        if let Some(error) = body.get("error").and_then(|error| error.as_str()) {
            self.error = Some(SharedString::from(error.to_owned()));
            return;
        }
        self.error = None;
        self.path.clear();
    }

    pub fn answered_delete(&mut self, body: &serde_json::Value) {
        self.deleting = false;
        if let Some(error) = body.get("error").and_then(|error| error.as_str()) {
            self.error = Some(SharedString::from(error.to_owned()));
        }
    }

    /// The self-test's progress: the job's own lines, the running flag and the
    /// verdict once it lands.
    pub fn answered_test(&mut self, body: &serde_json::Value) {
        self.test_running = body
            .get("running")
            .and_then(|running| running.as_bool())
            .unwrap_or(false);
        self.test_ok = body.get("ok").and_then(|ok| ok.as_bool());
        self.test_error = body
            .get("error")
            .and_then(|error| error.as_str())
            .map(|error| SharedString::from(error.to_owned()));
        self.test_progress = body
            .get("progress")
            .and_then(|progress| progress.as_array())
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(|line| line.as_str())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        self.result = body
            .get("result")
            .and_then(|result| result.as_object())
            .map(|result| {
                let mut parts = Vec::new();
                for key in ["same", "different", "margin"] {
                    if let Some(value) = result.get(key) {
                        parts.push(format!("{key} {value}"));
                    }
                }
                parts.join(" · ")
            })
            .filter(|text| !text.is_empty());
    }

    pub fn failed(&mut self, cause: impl Into<SharedString>) {
        self.uploading = false;
        self.deleting = false;
        self.test_running = false;
        self.error = Some(cause.into());
    }
}

/// The card. `info` is `/genre/status`'s `extractor` object (D1's fixtures prove
/// `onnxruntime` is a boolean, not a version).
pub fn view<V: Render>(
    card: &Card,
    info: &ExtractorInfo,
    cx: &mut Context<V>,
    on: impl Fn(&mut V, Action, &mut Context<V>) + Clone + 'static,
    field: Option<AnyElement>,
) -> AnyElement {
    let mut body = div()
        .id("genre-extractor")
        .flex()
        .flex_col()
        .gap(rems(0.5))
        .p(rems(0.75))
        .border_1()
        .border_color(theme::border(cx))
        .bg(theme::chrome(cx))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    icon(if info.available {
                        IconName::CircleCheck
                    } else {
                        IconName::CircleAlert
                    })
                    .size(rems(1.))
                    .text_color(if info.available {
                        theme::success(cx)
                    } else {
                        theme::warning(cx)
                    }),
                )
                .child(
                    div()
                        .text_size(rems(0.8125))
                        .font_weight(FontWeight::BOLD)
                        .child("Extracteur (discogs-effnet)"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .child(if info.available {
                            format!("{} · {} dimensions", "installé", info.dim)
                        } else {
                            "absent".to_owned()
                        }),
                ),
        );
    if !info.available {
        body = body.child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child(
                    "Le modèle n'est pas livré avec Neezer : téléchargez l'export \
                     « dynamic batch » depuis Essentia, puis indiquez son chemin ici. \
                     Il reste la propriété de son auteur (CC BY-NC-ND) et n'est jamais \
                     envoyé ailleurs qu'à votre serveur.",
                ),
        );
        if let Some(reason) = &info.reason {
            body = body.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::warning(cx))
                    .child(SharedString::from(reason.clone())),
            );
        }
        if info.uploadable {
            let mut row = div().flex().items_center().gap(rems(0.5));
            row = row.child(div().flex_1().child(field.unwrap_or_else(|| {
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child("/chemin/vers/discogs-effnet-bsdynamic-1.onnx")
                    .into_any_element()
            })));
            row = row.child(
                button(
                    "genre-extractor-upload",
                    "Installer",
                    ButtonVariant::Primary,
                    cx,
                )
                .disabled(card.uploading || card.path.trim().is_empty())
                .on_click({
                    let on = on.clone();
                    cx.listener(move |this, _, _, cx| on(this, Action::Upload, cx))
                }),
            );
            body = body.child(row);
        } else {
            body = body.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(
                        "L'envoi d'un modèle est réservé à l'administrateur du serveur \
                         (ou pointez [deezer] embed_model vers votre copie).",
                    ),
            );
        }
    } else {
        if let Some(model) = &info.model {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(SharedString::from(model.clone()))
                    .child(div().flex_1())
                    .child(
                        button(
                            "genre-extractor-delete",
                            "Retirer",
                            ButtonVariant::Danger,
                            cx,
                        )
                        .disabled(card.deleting)
                        .on_click({
                            let on = on.clone();
                            cx.listener(move |this, _, _, cx| on(this, Action::Delete, cx))
                        }),
                    ),
            );
        }
        if let Some(state) = &info.session_state {
            body = body.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(if state == "ok" {
                        theme::success(cx)
                    } else {
                        theme::warning(cx)
                    })
                    .child(match &info.session_error {
                        Some(error) => format!("session : {state} · {error}"),
                        None => format!("session : {state}"),
                    }),
            );
        }
        let mut test = div().flex().items_center().gap(rems(0.5));
        test = test.child(
            button(
                "genre-extractor-test",
                if card.test_running {
                    "Test en cours…"
                } else {
                    "Tester sur de vrais titres"
                },
                ButtonVariant::Outline,
                cx,
            )
            .disabled(card.test_running)
            .on_click({
                let on = on.clone();
                cx.listener(move |this, _, _, cx| on(this, Action::Test, cx))
            }),
        );
        if let Some(ok) = card.test_ok {
            test = test.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.25))
                    .text_size(rems(0.6875))
                    .text_color(if ok {
                        theme::success(cx)
                    } else {
                        theme::danger(cx)
                    })
                    .child(
                        icon(if ok {
                            IconName::CircleCheck
                        } else {
                            IconName::TriangleAlert
                        })
                        .size(rems(0.875)),
                    )
                    .child(if ok {
                        "Le test passe : deux moitiés d'un même titre sont plus proches \
                         que deux titres différents."
                    } else {
                        "Le test échoue : ce modèle ne sépare rien. Vérifiez la version \
                         « bsdynamic »."
                    }),
            );
        }
        body = body.child(test);
        if let Some(result) = &card.result {
            body = body.child(
                div()
                    .text_size(rems(0.625))
                    .font_family(theme::mono(cx))
                    .text_color(theme::secondary(cx))
                    .child(SharedString::from(result.clone())),
            );
        }
        if !card.test_progress.is_empty() {
            let mut lines = div()
                .id("genre-extractor-test-lines")
                .flex()
                .flex_col()
                .gap(rems(0.125))
                .max_h(rems(6.))
                .overflow_y_scroll();
            for line in card.test_progress.iter().rev().take(12) {
                lines = lines.child(
                    div()
                        .text_size(rems(0.625))
                        .font_family(theme::mono(cx))
                        .text_color(theme::secondary(cx))
                        .child(SharedString::from(line.clone())),
                );
            }
            body = body.child(lines);
        }
    }
    if let Some(error) = &card.error {
        body = body.child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::danger(cx))
                .child(error.clone()),
        );
    }
    body.into_any_element()
}

/// Upload one `.onnx` the operator obtained themselves. A plain multipart POST
/// with the app's own session cookie — the same jar every other request uses, so
/// the server sees the administrator who is signed in.
///
/// This is the one request `nz-api` cannot make (its `multipart` feature is not
/// in the workspace's pins; D2's note says the extractor upload lands with D16
/// together with that pin). The pin is on the app, not on the client crate, so
/// `nz-api` stays JSON-only and this file owns the one form.
pub async fn upload(
    server: &str,
    session: Arc<SessionJar>,
    path: PathBuf,
) -> Result<serde_json::Value, String> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "extractor.onnx".to_owned());
    if !name.to_lowercase().ends_with(".onnx") {
        return Err("le modèle doit être un fichier .onnx".to_owned());
    }
    let bytes = std::fs::read(&path).map_err(|error| format!("lecture impossible : {error}"))?;
    if bytes.is_empty() {
        return Err("le fichier est vide".to_owned());
    }
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(name)
        .mime_str("application/octet-stream")
        .map_err(|error| error.to_string())?;
    let form = reqwest::multipart::Form::new().part("files", part);
    let client = reqwest::Client::builder()
        .cookie_provider(session)
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(format!("{server}/api/genre/extractor"))
        .multipart(form)
        .send()
        .await
        .map_err(|error| format!("envoi impossible : {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("réponse illisible : {error}"))?;
    let value: serde_json::Value =
        serde_json::from_str(&body).unwrap_or(serde_json::json!({"error": body}));
    if !status.is_success() {
        return Err(value
            .get("error")
            .and_then(|error| error.as_str())
            .unwrap_or("envoi refusé")
            .to_owned());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_upload_names_its_cause() {
        let mut card = Card::default();
        card.answered_upload(&serde_json::json!({"error": "the extractor must be a .onnx file"}));
        assert!(!card.uploading);
        assert_eq!(
            card.error.as_deref().map(|error| error.to_string()),
            Some("the extractor must be a .onnx file".to_owned())
        );
    }

    #[test]
    fn the_self_test_reads_its_progress_and_verdict() {
        let mut card = Card::default();
        card.answered_test(&serde_json::json!({
            "running": true, "ok": null, "progress": ["Balancing the Dark", "Sirens"]
        }));
        assert!(card.test_running);
        assert_eq!(card.test_progress.len(), 2);
        card.answered_test(&serde_json::json!({
            "running": false, "ok": true,
            "result": {"same": 0.91, "different": 0.42, "margin": 0.49},
            "progress": []
        }));
        assert!(!card.test_running);
        assert_eq!(card.test_ok, Some(true));
        assert_eq!(
            card.result.as_deref(),
            Some("same 0.91 · different 0.42 · margin 0.49")
        );
    }
}
