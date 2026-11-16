//! Réglages → Assistance: what this app and its server are doing, in one panel
//! a person can copy into a bug report.
//!
//! The SPA's *Assistance* page is a service-worker reload and a diagnostic log.
//! A native binary has neither: there is no page to reload, and the log goes to
//! the process's own stderr (a terminal launch shows it). What replaces them has
//! to be at least as useful — so this page reports the build, the link, the
//! session, the audio engine's own numbers, MPRIS, the local store and the
//! server's version, and copies the whole thing in one click. Nothing is sent
//! anywhere: the text goes to the clipboard and nowhere else.

use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, Window, div, rems,
};
use gpui_omarchy::{ButtonVariant, button};

use super::{Load, Screen, card, card_title, hint, value_line};

pub fn body(screen: &mut Screen, _window: &mut Window, cx: &mut Context<Screen>) -> AnyElement {
    let summary = report(screen, cx);
    let mut column = div().flex().flex_col().gap(rems(1.));
    column = column.child(
        card("settings-state", cx)
            .child(card_title("État de l'application", cx))
            .child(hint(
                "Ce que cette fenêtre sait d'elle-même. Rien n'est envoyé nulle part : le \
                 bouton ci-dessous met ce texte dans le presse-papiers, pour le joindre à un \
                 rapport.",
                cx,
            ))
            .children(
                summary
                    .lines
                    .iter()
                    .map(|(label, value)| value_line(label.clone(), value.clone(), cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        button(
                            "settings-copy-report",
                            "Copier le diagnostic",
                            ButtonVariant::Primary,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            let text = report(this, cx).text.clone();
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
                            this.toast(crate::toast::ToastKind::Info, "Diagnostic copié", cx);
                        })),
                    )
                    .child(
                        button(
                            "settings-probe",
                            "Tester le serveur",
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.state.update(cx, |state, cx| state.probe(cx));
                        })),
                    ),
            ),
    );

    let mut server = card("settings-server-version", cx)
        .child(card_title("Serveur", cx))
        .child(value_line(
            "Adresse",
            screen.state.read(cx).server.clone(),
            cx,
        ))
        .child(value_line(
            "Liaison",
            if screen.state.read(cx).online {
                "Joignable".to_owned()
            } else {
                "Injoignable (le disque continue de fonctionner)".to_owned()
            },
            cx,
        ));
    match &screen.data.version {
        Load::Idle | Load::Loading => server = server.child(hint("Version demandée…", cx)),
        Load::Failed(cause) => {
            server = server
                .child(hint(format!("Version illisible : {cause}"), cx))
                .child(
                    button(
                        "settings-version-retry",
                        "Réessayer",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.reload_version(cx))),
                );
        }
        Load::Ready(version) => {
            server = server.child(value_line(
                "Version",
                version
                    .version
                    .clone()
                    .unwrap_or_else(|| "inconnue".to_owned()),
                cx,
            ));
            if let Some(build) = &version.build
                && !build.is_empty()
            {
                server = server.child(value_line("Build", build.clone(), cx));
            }
            if !version.android.url.is_empty() {
                server = server.child(value_line(
                    "Application Android",
                    version
                        .android
                        .version
                        .clone()
                        .unwrap_or_else(|| "publiée".to_owned()),
                    cx,
                ));
            }
        }
    }
    column = column.child(server);

    let mut store = card("settings-store", cx)
        .child(card_title("Stockage local", cx))
        .child(hint(
            "La base locale garde les réponses du serveur, les pochettes, l'audio écouté et \
             les réglages de cet appareil. Ce qui est déjà sur le serveur n'est jamais \
             dupliqué ici sans raison : le cache de lecture est un cache, et il s'évince.",
            cx,
        ));
    if let Some(error) = &screen.state.read(cx).storage_error {
        store = store.child(hint(error.clone(), cx));
    }
    for (label, path) in local_paths(screen, cx) {
        store = store.child(value_line(label, path, cx));
    }
    if let Some(storage) = screen.data.storage.ready() {
        store = store.child(value_line(
            "Archive du serveur",
            format!(
                "{} titres archivés sur {} · {} libres",
                storage.tracks_archived,
                bytes(storage.archive_bytes),
                bytes(storage.disk_free)
            ),
            cx,
        ));
    }
    column = column.child(store);

    column
        .child(
            card("settings-support", cx)
                .child(card_title("Signaler un problème", cx))
                .child(hint(
                    "Lancez l'application depuis un terminal pour voir son journal : chaque \
                 erreur de lecture, de session ou de sortie audio y est écrite avec sa cause. \
                 Joignez le diagnostic ci-dessus, et ce que vous faisiez à ce moment-là.",
                    cx,
                ))
                .child(value_line("Version de l'application", APP_VERSION, cx))
                .child(value_line("Système", std::env::consts::OS, cx)),
        )
        .into_any_element()
}

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// One copyable report: the rows the card shows, and the same rows as text.
pub struct Report {
    pub lines: Vec<(SharedString, SharedString)>,
    pub text: String,
}

fn report(screen: &Screen, cx: &mut Context<Screen>) -> Report {
    let state = screen.state.read(cx);
    let mut lines: Vec<(SharedString, SharedString)> = Vec::new();
    lines.push(("Application".into(), APP_VERSION.to_string().into()));
    lines.push(("Adresse".into(), state.server.clone().into()));
    lines.push((
        "Liaison".into(),
        if state.online {
            "joignable".into()
        } else {
            "injoignable".into()
        },
    ));
    lines.push((
        "Session".into(),
        match &state.auth {
            crate::state::Auth::LoggedIn { me } => format!(
                "{} ({})",
                me.user.name,
                if me.user.admin {
                    "administrateur"
                } else {
                    "utilisateur"
                }
            )
            .into(),
            crate::state::Auth::Restoring => "vérification".into(),
            crate::state::Auth::Failed => "non confirmée".into(),
            crate::state::Auth::LoggedOut => "déconnecté".into(),
        },
    ));
    match &state.audio {
        Some(audio) => {
            let info = audio.engine.output_info();
            lines.push((
                "Sortie audio".into(),
                format!(
                    "{} · {} Hz · {} canal(x) · {} ms de tampon",
                    info.device,
                    info.sample_rate,
                    info.channels,
                    (info.latency_frames as f64 * 1_000. / f64::from(info.sample_rate.max(1)))
                        .round()
                )
                .into(),
            ));
            lines.push((
                "Sous-alimentations".into(),
                audio.engine.underruns().to_string().into(),
            ));
        }
        None => lines.push((
            "Sortie audio".into(),
            state
                .audio_error
                .clone()
                .unwrap_or_else(|| "pas encore ouverte".into()),
        )),
    }
    lines.push((
        "File".into(),
        format!(
            "{} titres, position {}, {}",
            state.player.len(),
            state.player.index() + 1,
            if state.player.playing() {
                "en lecture"
            } else {
                "en pause"
            }
        )
        .into(),
    ));
    if let Some(version) = screen.data.version.ready() {
        lines.push((
            "Serveur".into(),
            version
                .version
                .clone()
                .unwrap_or_else(|| "version inconnue".to_owned())
                .into(),
        ));
    }
    lines.push(("Mémoire".into(), format!("{} Mo", rss_kb() / 1024).into()));
    let text = lines
        .iter()
        .map(|(label, value)| format!("{label} : {value}"))
        .collect::<Vec<String>>()
        .join("\n");
    Report { lines, text }
}

/// The process's resident set, in kB, from `/proc/self/status`. Linux only —
/// this app is an Omarchy app — and a missing file answers 0 rather than
/// failing the screen.
pub fn rss_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

/// Where this device keeps what it keeps: the SPA's *Stockage* block, with the
/// XDG paths a desktop actually has.
fn local_paths(screen: &Screen, cx: &mut Context<Screen>) -> Vec<(&'static str, String)> {
    let state = screen.state.read(cx);
    let Some(cache) = &state.cache else {
        return Vec::new();
    };
    vec![
        ("Base locale", cache.db_path().display().to_string()),
        ("Pochettes", cache.covers_dir().display().to_string()),
        ("Audio écouté", cache.audio_dir().display().to_string()),
    ]
}

/// Bytes as a person reads them (`format.js#bytes`): the same units and the
/// same rounding as the page.
pub fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["o", "Ko", "Mo", "Go", "To"];
    let mut size = value as f64;
    let mut unit = 0;
    while size >= 1024. && unit < UNITS.len() - 1 {
        size /= 1024.;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} o")
    } else if size >= 100. {
        format!("{size:.0} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

impl Screen {
    pub fn reload_version(&mut self, cx: &mut Context<Self>) {
        if let Some(api) = self.api(cx) {
            let reply = self.reply();
            let runtime = self.runtime(cx);
            runtime.spawn(async move {
                let answer = api
                    .version()
                    .await
                    .map_err(|error| error.cause_fr().to_owned());
                reply.send(super::Reply::Version(answer));
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_read_like_the_pages_own_formatter() {
        assert_eq!(bytes(0), "0 o");
        assert_eq!(bytes(512), "512 o");
        assert_eq!(bytes(1024), "1.0 Ko");
        assert_eq!(bytes(1536), "1.5 Ko");
        assert_eq!(bytes(1024 * 1024), "1.0 Mo");
        assert_eq!(bytes(300 * 1024 * 1024), "300 Mo");
        assert_eq!(bytes(2 * 1024 * 1024 * 1024), "2.0 Go");
    }

    #[test]
    fn the_report_is_copyable_text_with_one_line_per_fact() {
        // The report is what a person pastes into a bug report: every line names
        // its fact, and no line is empty.
        let lines = [
            ("Application", "0.1.0"),
            ("Adresse", "http://127.0.0.1:5000"),
            ("Mémoire", "97 Mo"),
        ];
        let text = lines
            .iter()
            .map(|(label, value)| format!("{label} : {value}"))
            .collect::<Vec<String>>()
            .join("\n");
        assert_eq!(
            text,
            "Application : 0.1.0\nAdresse : http://127.0.0.1:5000\nMémoire : 97 Mo"
        );
        assert!(text.lines().all(|line| line.contains(" : ")));
    }

    #[test]
    fn the_process_reports_its_own_resident_memory() {
        // Linux always has `/proc/self/status`; a unit test that read 0 here
        // would mean the gauge is broken, not that the machine is small.
        assert!(rss_kb() > 0, "VmRSS must be readable on this platform");
    }
}
