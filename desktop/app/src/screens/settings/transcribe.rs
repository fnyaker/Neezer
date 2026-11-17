//! Réglages → Transcription locale: this PC's GPU, the models, the queue and
//! the worker (lot T4).
//!
//! The engine lives in its own process (`neezer-transcribe`); this screen never
//! runs whisper, never maps a model and never blocks on one. It draws
//! [`crate::wiring::transcribe::Snapshot`] — a few kilobytes cloned under one
//! mutex — and asks the service to do things. The service is what talks to the
//! child, the network and the model files.
//!
//! Three rules shape it:
//!
//! - **Rule 1, the priority is a sentence, not a number.** « Très basse » pauses
//!   the job when the sound or a painted frame needs the machine; « Basse » only
//!   for the sound; « Normale » only when the sound actually broke. The card says
//!   what each one yields to, and the job card says why it is paused right now.
//! - **Rule 2, every wait has a cause.** A running job shows its phase
//!   (téléchargement, transcription, vérification), its percent and the reason it
//!   is paused; a failure lists its code and its French sentence, with
//!   « Réessayer ». A model download shows its bytes, then its SHA-256 check.
//! - **Rule 6, everything here is a setting.** The model, the language, Vulkan or
//!   the CPU and the priority are `nv`-cache `kv` values, saved on change and read
//!   back by the next job — no config file, no restart.
//!
//! Contract for `screens/settings/mod.rs` (D14a): `transcribe::body(self, window,
//! cx)` for `SettingsSection::Transcribe`, the one arm a delivered section adds.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::{
    AnyElement, Context, ElementId, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement as _, Styled, Window, div, px, relative, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon, icon_button, separator, switch, toggle};
use nz_transcribe::backend::{Backend, FallbackCause};

use super::{Screen, card, card_title, hint, value_line};
use crate::theme;
use crate::widgets::transcript_panel::{LANGUAGES, language_label};
use crate::wiring::transcribe::{
    self, DownloadPhase, Engine, MODEL_KEY, ModelRow, Phase, Snapshot, bytes_label,
};

/// The priorities, as the chips read them. The first is the default and the
/// reason rule 1 exists: the person's music and their next click come first.
pub const PRIORITIES: [(&str, &str, &str); 3] = [
    ("lowest", "Très basse", "cède à la lecture et à l'interface"),
    ("low", "Basse", "cède à la lecture"),
    ("normal", "Normale", "cède seulement si le son se coupe"),
];

/// How often the card re-reads the service while it is open: one mutex and one
/// comparison, and no repaint when nothing moved (a 500 ms tick).
const POLL: Duration = Duration::from_millis(500);

/// One watcher for the process: the screen is rebuilt with every route change,
/// and the loop below ends by itself when its entity is gone.
static WATCHING: AtomicBool = AtomicBool::new(false);

pub fn body(screen: &mut Screen, _window: &mut Window, cx: &mut Context<Screen>) -> AnyElement {
    let state = screen.state.clone();
    let service = transcribe::ensure(state.read(cx));
    let snapshot = service.snapshot();
    // A standing probe is cheap (one child) and cached for five minutes; the
    // first paint of this page is what asks for it.
    service.probe(false);
    watch(cx);

    let mut column = div()
        .id("transcribe-settings")
        .debug_selector(|| "transcribe-settings".to_owned())
        .flex()
        .flex_col()
        .gap(rems(0.75))
        .max_w(px(1_800.))
        .min_w_0();

    if let Some(note) = &snapshot.note {
        column = column.child(note_line(note, cx));
    }

    column = column.child(engine_card(&snapshot, &service, cx));
    column = column.child(models_card(&snapshot, &service, cx));
    column = column.child(preferences_card(&snapshot, &service, cx));
    if let Some(card) = job_card(&snapshot, &service, cx) {
        column = column.child(card);
    }
    if !snapshot.failures.is_empty() {
        column = column.child(failures_card(&snapshot, &service, cx));
    }
    if screen.is_admin(cx) {
        if snapshot.worker.enrolled {
            service.refresh_worker();
        }
        column = column.child(worker_card(&snapshot, &service, cx));
    }
    column.into_any_element()
}

/// One repaint loop, started the first time the page is drawn and ended when
/// its entity is gone (a route change).
fn watch(cx: &mut Context<Screen>) {
    if WATCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(POLL).await;
            let alive = this.update(cx, |_, cx| cx.notify());
            if alive.is_err() {
                WATCHING.store(false, Ordering::SeqCst);
                break;
            }
        }
    })
    .detach();
}

fn note_line(note: &str, cx: &mut Context<Screen>) -> AnyElement {
    div()
        .id("transcribe-note")
        .flex()
        .items_start()
        .gap(rems(0.5))
        .p(rems(0.625))
        .border_1()
        .border_color(theme::border(cx))
        .bg(theme::chrome(cx))
        .text_size(rems(0.75))
        .text_color(theme::text(cx))
        .child(
            icon(IconName::Info)
                .size(rems(1.))
                .text_color(theme::secondary(cx)),
        )
        .child(div().flex_1().min_w_0().child(note.to_owned()))
        .into_any_element()
}

// --- the engine --------------------------------------------------------------

fn engine_card(
    snapshot: &Snapshot,
    service: &std::sync::Arc<transcribe::Service>,
    cx: &mut Context<Screen>,
) -> AnyElement {
    let engine = &snapshot.engine;
    let (glyph, colour) = match engine {
        Engine::Vulkan { .. } => (IconName::Zap, theme::accent(cx)),
        Engine::Cpu { .. } => (IconName::Cpu, theme::warning(cx)),
        Engine::Probing => (IconName::Activity, theme::secondary(cx)),
        Engine::Unknown | Engine::Unavailable { .. } => (IconName::CircleAlert, theme::danger(cx)),
    };
    let mut card = card("transcribe-engine", cx)
        .child(card_title("Moteur", cx))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(icon(glyph).size(rems(1.25)).text_color(colour))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(rems(0.8125))
                        .text_color(theme::text(cx))
                        .child(engine.label_fr()),
                )
                .child({
                    let service = service.clone();
                    button(
                        "transcribe-probe",
                        "Revérifier",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(move |_, _, _, _| {
                        service.probe(true);
                        service.publish_models();
                        service.refresh_worker();
                    }))
                }),
        );
    if let Some(device) = match engine {
        Engine::Vulkan { device } => device.as_deref(),
        _ => None,
    } {
        card = card.child(value_line("Appareil", device.to_owned(), cx));
    }
    if let Engine::Cpu {
        cause: FallbackCause::UserForcedCpu,
    } = engine
    {
        card = card.child(hint(
            "Le processeur est forcé par le réglage ci-dessous. Désactivez-le pour réessayer \
             le GPU : whisper.cpp recompile ses nuances au premier passage.",
            cx,
        ));
    }
    card.child(hint(
        "La transcription tourne dans un processus séparé, à priorité basse : elle ne prend \
         ni le son ni l'interface. Vulkan est utilisé par défaut ; si le GPU ne peut pas \
         démarrer, la cause exacte est écrite ci-dessus et le processeur prend le relais.",
        cx,
    ))
    .into_any_element()
}

// --- the models --------------------------------------------------------------

fn models_card(
    snapshot: &Snapshot,
    service: &std::sync::Arc<transcribe::Service>,
    cx: &mut Context<Screen>,
) -> AnyElement {
    let mut card = card("transcribe-models", cx).child(card_title("Modèles", cx));
    let free = snapshot
        .free_memory
        .map(bytes_label)
        .unwrap_or_else(|| "inconnue".to_owned());
    card = card.child(hint(
        format!(
            "Rien ne se télécharge sans votre clic. Chaque fichier est vérifié par sa taille \
             puis son SHA-256 après le transfert ; un fichier qui ne correspond pas est \
             supprimé. Mémoire libre : {free}."
        ),
        cx,
    ));
    if let Some(recommended) = snapshot.models.iter().find(|row| row.recommended) {
        card = card.child(hint(
            format!(
                "Recommandé pour cette machine : {} ({}).",
                recommended.name,
                bytes_label(recommended.bytes)
            ),
            cx,
        ));
    }

    let chosen = snapshot
        .models
        .iter()
        .find(|row| row.chosen)
        .map(|row| row.name.to_owned());
    for row in snapshot.models.iter().filter(|row| !is_vad(row.name)) {
        card = card.child(model_row(row, &chosen, snapshot, service, cx));
    }
    card.child(hint(
        "La détection de voix (VAD) n'est ni proposée ni utilisée : sur un processeur elle \
         coûte plus de temps qu'elle n'en fait gagner (98× la durée de l'audio, mesuré au \
         lot T2), et le moteur reste donc sur son découpage habituel.",
        cx,
    ))
    .into_any_element()
}

fn model_row(
    row: &ModelRow,
    chosen: &Option<String>,
    snapshot: &Snapshot,
    service: &std::sync::Arc<transcribe::Service>,
    cx: &mut Context<Screen>,
) -> AnyElement {
    let downloading = snapshot
        .download
        .as_ref()
        .filter(|download| download.name == row.name);
    let mut line = div()
        .id(ElementId::Name(format!("model-{}", row.name).into()))
        .flex()
        .items_center()
        .gap(rems(0.5))
        .w_full()
        .min_w_0()
        .py(rems(0.125));
    line = line.child(
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(rems(0.125))
            .child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::text(cx))
                    .child(SharedString::from(row.name.to_owned())),
            )
            .child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(SharedString::from(bytes_label(row.bytes))),
            ),
    );

    if let Some(download) = downloading {
        match &download.phase {
            DownloadPhase::Downloading => {
                line = line.child(
                    div()
                        .flex()
                        .flex_col()
                        .w(rems(12.))
                        .gap(rems(0.125))
                        .child(progress_bar(download.percent() as f32 / 100.0, cx))
                        .child(
                            div()
                                .text_size(rems(0.625))
                                .font_family(theme::mono(cx))
                                .text_color(theme::secondary(cx))
                                .child(SharedString::from(format!(
                                    "{} sur {} · {} %",
                                    bytes_label(download.done),
                                    bytes_label(download.total),
                                    download.percent()
                                ))),
                        ),
                );
                let service = service.clone();
                line = line.child(
                    button("model-cancel", "Annuler", ButtonVariant::Secondary, cx)
                        .on_click(cx.listener(move |_, _, _, _| service.cancel_download())),
                );
            }
            DownloadPhase::Verifying => {
                line = line.child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .child("Vérification du SHA-256…"),
                );
            }
            DownloadPhase::Done => {
                line = line.child(state_chip("Installé", true, cx));
                let service = service.clone();
                line = line.child(
                    icon_button(
                        "model-dismiss",
                        IconName::Check,
                        "Fermer",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(move |_, _, _, _| service.dismiss_download())),
                );
            }
            DownloadPhase::Failed(cause) => {
                line = line
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(rems(0.6875))
                            .text_color(theme::danger(cx))
                            .child(SharedString::from(cause.clone())),
                    )
                    .child({
                        let service = service.clone();
                        let name = row.name;
                        button("model-retry", "Réessayer", ButtonVariant::Secondary, cx)
                            .on_click(cx.listener(move |_, _, _, _| service.download_model(name)))
                    });
            }
        }
        return line.into_any_element();
    }

    if row.installed && !row.invalid {
        let selected = chosen.as_deref() == Some(row.name);
        line = line.child(state_chip(
            if selected { "Utilisé" } else { "Installé" },
            selected,
            cx,
        ));
        if !selected {
            let service = service.clone();
            let name = row.name;
            line = line.child(
                toggle(
                    ElementId::Name(format!("model-choose-{name}").into()),
                    "Utiliser",
                    false,
                    cx,
                )
                .on_click(cx.listener(move |_, _, _, cx| {
                    service.set_setting(MODEL_KEY, &serde_json::Value::from(name));
                    cx.notify();
                })),
            );
        }
    } else if row.invalid {
        line = line
            .child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::danger(cx))
                    .child("Fichier invalide"),
            )
            .child({
                let service = service.clone();
                let name = row.name;
                button(
                    "model-redownload",
                    "Retélécharger",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(move |_, _, _, _| service.download_model(name)))
            });
    }

    if !row.installed || row.invalid {
        let service = service.clone();
        let name = row.name;
        line = line.child(
            button("model-download", "Télécharger", ButtonVariant::Outline, cx)
                .on_click(cx.listener(move |_, _, _, _| service.download_model(name))),
        );
    }
    if row.recommended {
        line = line.child(
            div()
                .text_size(rems(0.625))
                .text_color(theme::accent(cx))
                .child("Conseillé"),
        );
    }
    line.into_any_element()
}

/// The VAD files are pinned by the crate for completeness but are not offered:
/// whisper.cpp's own VAD path is CPU-only and unbatched, and T2 measured it at
/// 98× the cost of the audio it removes (rule 1). A button that downloads a file
/// nothing uses would be a button that does nothing (rule 2).
fn is_vad(name: &str) -> bool {
    name == nz_transcribe::models::VAD || name == nz_transcribe::models::VAD_FOR_TESTS
}

fn state_chip(label: &str, on: bool, cx: &mut Context<Screen>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .h(rems(1.5))
        .px(rems(0.375))
        .border_1()
        .border_color(if on {
            theme::accent(cx)
        } else {
            theme::border(cx)
        })
        .text_size(rems(0.6875))
        .text_color(if on {
            theme::accent(cx)
        } else {
            theme::secondary(cx)
        })
        .child(SharedString::from(label.to_owned()))
        .into_any_element()
}

fn progress_bar(fraction: f32, cx: &mut Context<Screen>) -> AnyElement {
    div()
        .relative()
        .w_full()
        .h(rems(0.1875))
        .bg(theme::border(cx))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .w(relative(fraction.clamp(0.0, 1.0)))
                .bg(theme::accent(cx)),
        )
        .into_any_element()
}

// --- the settings ------------------------------------------------------------

fn preferences_card(
    snapshot: &Snapshot,
    service: &std::sync::Arc<transcribe::Service>,
    cx: &mut Context<Screen>,
) -> AnyElement {
    let settings = &snapshot.settings;
    let mut card = card("transcribe-preferences", cx)
        .child(card_title("Préférences", cx))
        .child(hint(
            "La langue et la priorité s'appliquent au prochain passage ; un travail en cours \
             garde celles avec lesquelles il a commencé.",
            cx,
        ));

    // The language: the server's own list (`transcribe.LANGUAGES`), labelled the
    // way the transcript panel labels a segment's verdict.
    let mut languages = div().flex().flex_wrap().items_center().gap(rems(0.25));
    for code in LANGUAGES {
        let selected = settings.language == code;
        let service = service.clone();
        languages = languages.child(
            toggle(
                (
                    "transcribe-language",
                    LANGUAGES.iter().position(|c| *c == code).unwrap_or(0),
                ),
                language_label(code),
                selected,
                cx,
            )
            .on_click(cx.listener(move |_, _, _, cx| {
                service.set_setting(transcribe::LANGUAGE_KEY, &serde_json::Value::from(code));
                cx.notify();
            })),
        );
    }
    card = card
        .child(value_line("Langue", language_label(&settings.language), cx))
        .child(languages);

    card = card.child(separator(cx)).child(
        switch(
            "transcribe-cpu",
            "Utiliser le processeur (plus lent)",
            settings.cpu,
            cx,
        )
        .on_click(cx.listener({
            let service = service.clone();
            move |_, _, _, cx| {
                let cpu = !service.snapshot().settings.cpu;
                service.set_setting(transcribe::CPU_KEY, &serde_json::Value::from(cpu));
                if cpu {
                    service.probe(true);
                }
                cx.notify();
            }
        })),
    );
    card = card.child(hint(
        "Désactivé, whisper.cpp utilise Vulkan, le GPU de cet ordinateur. Activé, la cause \
         « user_forced_cpu » est écrite dans le statut : c'est un choix, pas une panne.",
        cx,
    ));

    card = card.child(separator(cx)).child(
        switch(
            "transcribe-battery",
            "Seulement sur secteur",
            settings.battery_only,
            cx,
        )
        .on_click(cx.listener({
            let service = service.clone();
            move |_, _, _, cx| {
                let only = !service.snapshot().settings.battery_only;
                service.set_setting(transcribe::BATTERY_KEY, &serde_json::Value::from(only));
                cx.notify();
            }
        })),
    );
    card = card.child(hint(
        "Un portable sur batterie arrête la transcription : elle reprend au secteur, et \
         « Reprendre » repart du dernier passage. Sur un ordinateur de bureau, ce réglage \
         n'a aucun effet.",
        cx,
    ));

    card = card.child(separator(cx)).child(card_title("Priorité", cx));
    let mut priorities = div().flex().flex_wrap().items_center().gap(rems(0.25));
    for (id, label, hint) in PRIORITIES {
        let selected = settings.priority.id() == id;
        let service = service.clone();
        priorities = priorities.child(
            toggle(format!("transcribe-priority-{id}"), label, selected, cx).on_click(cx.listener(
                move |_, _, _, cx| {
                    service.set_setting(transcribe::PRIORITY_KEY, &serde_json::Value::from(id));
                    cx.notify();
                },
            )),
        );
        let _ = hint;
    }
    card = card.child(priorities);
    for (id, label, explanation) in PRIORITIES {
        if settings.priority.id() == id {
            card = card.child(
                div()
                    .flex()
                    .items_start()
                    .gap(rems(0.375))
                    .text_size(rems(0.75))
                    .child(
                        div()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(label),
                    )
                    .child(div().text_color(theme::secondary(cx)).child("—"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(theme::secondary(cx))
                            .child(explanation),
                    ),
            );
        }
    }
    card.child(hint(
        "Une pause dure au moins trois secondes : un moteur qui redémarre à chaque saccade \
         recompile ses nuances et perd plus de temps qu'il n'en gagne. Le superviseur fait \
         aussi tourner le travail à nice 19, en tâche de fond.",
        cx,
    ))
    .into_any_element()
}

// --- the job -----------------------------------------------------------------

fn job_card(
    snapshot: &Snapshot,
    service: &std::sync::Arc<transcribe::Service>,
    cx: &mut Context<Screen>,
) -> Option<AnyElement> {
    let job = snapshot.job.as_ref()?;
    let title = job
        .title
        .clone()
        .unwrap_or_else(|| format!("Épisode {}", short_id(&job.episode)));
    let (phase_label, fraction, detail) = match &job.phase {
        Phase::Audio { done, total } => {
            let fraction = total
                .filter(|total| *total > 0)
                .map(|total| *done as f32 / total as f32)
                .unwrap_or(0.0);
            (
                "Téléchargement de l'audio",
                fraction,
                match total {
                    Some(total) if *total > 0 => format!(
                        "{} sur {} (Opus 128, pour la transcription)",
                        bytes_label(*done),
                        bytes_label(*total)
                    ),
                    _ => format!("{} reçus (Opus 128)", bytes_label(*done)),
                },
            )
        }
        Phase::Transcribing => (
            "Transcription sur cet ordinateur",
            job.percent()
                .map(|percent| percent as f32 / 100.0)
                .unwrap_or(0.0),
            job.progress_label(),
        ),
        Phase::Finishing => (
            "Écriture de la transcription",
            1.0,
            "Le texte et les sous-titres sont écrits sur le disque.".to_owned(),
        ),
    };
    let mut card = card("transcribe-job", cx)
        .child(card_title(title, cx))
        .child(value_line(phase_label, detail, cx))
        .child(progress_bar(fraction, cx));

    if let Some(backend) = job.backend {
        let device = job.device.clone().unwrap_or_default();
        let label = match backend {
            Backend::Vulkan if device.is_empty() => "Vulkan".to_owned(),
            Backend::Vulkan => format!("Vulkan · {device}"),
            Backend::Cpu => "Processeur".to_owned(),
        };
        card = card.child(value_line("Processeur utilisé", label, cx));
    }
    if let Some(paused) = job.paused {
        card = card.child(
            div()
                .flex()
                .items_start()
                .gap(rems(0.5))
                .text_size(rems(0.75))
                .text_color(theme::warning(cx))
                .child(icon(IconName::Pause).size(rems(1.)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(0.125))
                        .child(SharedString::from(paused.label_fr()))
                        .child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::secondary(cx))
                                .child(paused.explain_fr()),
                        ),
                ),
        );
    }
    let service = service.clone();
    card = card.child(
        div().flex().justify_start().child(
            button(
                "transcribe-stop-job",
                "Arrêter",
                ButtonVariant::Secondary,
                cx,
            )
            .on_click(cx.listener(move |_, _, _, _| service.stop_local())),
        ),
    );
    Some(
        card.child(hint(
            "Ce qui est déjà transcrit est gardé : « Reprendre » repart du dernier passage, même \
             après un redémarrage de l'application.",
            cx,
        ))
        .into_any_element(),
    )
}

fn failures_card(
    snapshot: &Snapshot,
    service: &std::sync::Arc<transcribe::Service>,
    cx: &mut Context<Screen>,
) -> AnyElement {
    let mut card = card("transcribe-failures", cx).child(card_title("Échecs", cx));
    for failure in &snapshot.failures {
        let mut row = div()
            .flex()
            .items_start()
            .gap(rems(0.5))
            .w_full()
            .min_w_0()
            .py(rems(0.25))
            .child(
                icon(IconName::TriangleAlert)
                    .size(rems(1.))
                    .text_color(theme::danger(cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(rems(0.125))
                    .child(
                        div()
                            .text_size(rems(0.75))
                            .text_color(theme::text(cx))
                            .child(SharedString::from(failure.message.clone())),
                    )
                    .child(
                        div()
                            .text_size(rems(0.625))
                            .font_family(theme::mono(cx))
                            .text_color(theme::secondary(cx))
                            .child(SharedString::from(format!("Code : {}", failure.code))),
                    ),
            );
        if !failure.episode.is_empty() {
            let service = service.clone();
            let episode = failure.episode.clone();
            row = row.child(
                button(
                    "transcribe-retry-job",
                    "Réessayer",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(move |_, _, _, _| service.start_local(&episode, None, 0))),
            );
        }
        card = card.child(row);
    }
    card.into_any_element()
}

// --- the worker --------------------------------------------------------------

fn worker_card(
    snapshot: &Snapshot,
    service: &std::sync::Arc<transcribe::Service>,
    cx: &mut Context<Screen>,
) -> AnyElement {
    let worker = &snapshot.worker;
    let mut card = card("transcribe-worker", cx)
        .child(card_title("File du serveur", cx))
        .child(hint(
            "Un administrateur peut enrôler cet ordinateur comme appareil de transcription : \
             le serveur lui confie ses épisodes, et le texte arrive dans la bibliothèque de \
             tout le monde. Le jeton d'appareil est écrit dans votre dossier de configuration, \
             lisible par vous seul, et n'est jamais affiché ni journalisé.",
            cx,
        ));
    card = card.child(value_line(
        "État",
        if worker.enrolled {
            "Cet ordinateur est enrôlé".to_owned()
        } else {
            "Pas enrôlé".to_owned()
        },
        cx,
    ));
    if worker.enrolled && !worker.name.is_empty() {
        card = card.child(value_line("Nom sur le serveur", worker.name.clone(), cx));
    }
    if worker.enrolled {
        card = card.child(value_line(
            "Actif",
            if worker.enabled {
                "oui".to_owned()
            } else {
                "désactivé sur le serveur".to_owned()
            },
            cx,
        ));
        if let Some(seen) = &worker.last_seen {
            card = card.child(value_line("Vu la dernière fois", seen.clone(), cx));
        }
        if let Some(lease) = &worker.lease {
            card = card.child(value_line("Épisode en cours", short_id(lease), cx));
        }
        card = card.child(
            switch(
                "transcribe-run-worker",
                "Faire tourner la file du serveur ici",
                worker.running,
                cx,
            )
            .on_click(cx.listener({
                let service = service.clone();
                move |_, _, _, cx| {
                    if service.snapshot().worker.running {
                        service.stop_child();
                    } else {
                        service.start_worker();
                    }
                    cx.notify();
                }
            })),
        );
        card = card.child(hint(
            "Le même moteur sert les deux files : un épisode local en cours passe avant la \
             file du serveur, et la priorité de lecture s'applique aux deux.",
            cx,
        ));
    }

    let enrolling = worker.busy;
    if worker.enrolled {
        let service = service.clone();
        card = card.child(
            div().flex().justify_start().child(
                button(
                    "transcribe-revoke",
                    if enrolling {
                        "Révocation…"
                    } else {
                        "Révoquer cet ordinateur"
                    },
                    ButtonVariant::Danger,
                    cx,
                )
                .on_click(cx.listener(move |_, _, _, _| service.revoke())),
            ),
        );
    } else {
        let name = transcribe::hostname();
        let service = service.clone();
        card = card
            .child(value_line("Nom proposé", name.clone(), cx))
            .child(
                div().flex().justify_start().child(
                    button(
                        "transcribe-enrol",
                        if enrolling {
                            "Enrôlement…"
                        } else {
                            "Enrôler cet ordinateur"
                        },
                        ButtonVariant::Primary,
                        cx,
                    )
                    .on_click(cx.listener(move |_, _, _, _| service.enrol(&name))),
                ),
            );
    }
    card.into_any_element()
}

/// `11111111-2222-…` → `11111111…` — an id is a reference, not a title.
fn short_id(value: &str) -> String {
    let take: String = value.chars().take(8).collect();
    if take.len() < value.len() {
        format!("{take}…")
    } else {
        take
    }
}

#[cfg(test)]
mod tests {
    use super::{PRIORITIES, short_id};
    use crate::widgets::transcript_panel::{LANGUAGES, language_label};
    use crate::wiring::transcribe::{Priority, bytes_label};
    use nz_transcribe::models;

    /// Every priority the enum names has a chip, in the same order, and the first
    /// is the default: a chip nobody can select, or a stored id with no chip, is
    /// a setting that silently does nothing.
    #[test]
    fn every_priority_has_one_chip_and_the_default_is_the_first() {
        assert_eq!(PRIORITIES.len(), Priority::ALL.len());
        for (index, priority) in Priority::ALL.into_iter().enumerate() {
            assert_eq!(PRIORITIES[index].0, priority.id());
            assert_eq!(PRIORITIES[index].1, priority.label());
            assert!(!PRIORITIES[index].2.is_empty());
        }
        assert_eq!(Priority::default(), Priority::Lowest);
        assert_eq!(Priority::parse("lowest"), Priority::Lowest);
        assert_eq!(Priority::parse("inconnu"), Priority::Lowest);
    }

    /// The language chips are the server's list and every code has a French
    /// name, so a transcript annotated `pt` is never shown as `PT`.
    #[test]
    fn every_language_has_a_french_name() {
        assert!(LANGUAGES.contains(&"auto"));
        assert!(LANGUAGES.contains(&"fr"));
        for code in LANGUAGES {
            let label = language_label(code);
            assert!(!label.is_empty(), "{code}");
            assert_ne!(label, code.to_ascii_uppercase(), "{code}");
        }
    }

    /// A model's size reads in French with a comma, and an id in the job card is
    /// shortened instead of printing a UUID.
    #[test]
    fn sizes_and_ids_read_in_french() {
        assert_eq!(bytes_label(1024), "1,0 ko");
        assert_eq!(bytes_label(874_188_075), "833,7 Mo");
        assert_eq!(
            short_id("11111111-2222-3333-4444-555555555555"),
            "11111111…"
        );
        assert_eq!(short_id("court"), "court");
    }

    /// The pinned models the screen lists are the crate's six, so a size or a
    /// name can never be typed a second time in the UI.
    #[test]
    fn the_screen_lists_the_pinned_files() {
        assert_eq!(models::FILES.len(), 6);
        assert!(
            models::FILES
                .iter()
                .any(|file| file.name == models::TURBO_Q8)
        );
        assert!(models::FILES.iter().any(|file| file.name == models::VAD));
    }
}
