//! The complete keymap, in one file, bound once.
//!
//! Keyboard and mouse are the same application: every mouse affordance
//! dispatches the action that this table binds, so there is one handler to
//! test and no second path to drift. The context is `Neezer && !Input`: the
//! `!Input` half is what lets a person type a space, an `a`, an `s` or a `q`
//! into any text field without toggling the transport — GPUI evaluates the
//! predicate against the whole focus path, and gpui-base's editor puts `Input`
//! on it (`input/base/state.rs`, `CONTEXT`).
//!
//! After D7 a later lot binds nothing: the actions it implements are already
//! here, dispatched to no one until its screen handles them (the plan's rule
//! that no lot edits `keymap.rs` after D7).

use gpui_kit::{App, KeyBinding};
use gpui_omarchy::ActiveTheme;

use crate::actions::*;
use crate::app::GoGenres;

/// The context every screen binding lives in.
pub const CONTEXT: &str = "Neezer && !Input";

/// One row of the `?` overlay and of Réglages → Raccourcis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    /// The keystrokes, as the overlay prints them (`Ctrl+→`, `g h`).
    pub keys: &'static str,
    /// What it does, in French.
    pub label: &'static str,
}

/// The table the overlay and the settings screen render. Kept next to the
/// bindings so the two cannot disagree about what exists.
pub const SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        keys: "Espace",
        label: "Lecture / pause",
    },
    Shortcut {
        keys: "Ctrl+→",
        label: "Titre suivant",
    },
    Shortcut {
        keys: "Ctrl+←",
        label: "Titre précédent",
    },
    Shortcut {
        keys: "Maj+→",
        label: "Avancer de 10 s",
    },
    Shortcut {
        keys: "Maj+←",
        label: "Reculer de 10 s",
    },
    Shortcut {
        keys: "Ctrl+↑",
        label: "Monter le volume",
    },
    Shortcut {
        keys: "Ctrl+↓",
        label: "Baisser le volume",
    },
    Shortcut {
        keys: "m",
        label: "Couper le son",
    },
    Shortcut {
        keys: "s",
        label: "Lecture aléatoire",
    },
    Shortcut {
        keys: "r",
        label: "Répéter : non, tout, un",
    },
    Shortcut {
        keys: "/",
        label: "Rechercher",
    },
    Shortcut {
        keys: "Ctrl+K",
        label: "Rechercher",
    },
    Shortcut {
        keys: "g h",
        label: "Accueil",
    },
    Shortcut {
        keys: "g s",
        label: "Recherche",
    },
    Shortcut {
        keys: "g l",
        label: "Bibliothèque",
    },
    Shortcut {
        keys: "g p",
        label: "Podcasts",
    },
    Shortcut {
        keys: "g r",
        label: "Nouveautés",
    },
    Shortcut {
        keys: "g n",
        label: "Lecture en cours",
    },
    Shortcut {
        keys: "g e",
        label: "Studio de genres",
    },
    Shortcut {
        keys: "g q",
        label: "File d'attente",
    },
    Shortcut {
        keys: "g ,",
        label: "Réglages",
    },
    Shortcut {
        keys: "Alt+←",
        label: "Retour",
    },
    Shortcut {
        keys: "Alt+→",
        label: "Avancer",
    },
    Shortcut {
        keys: "Échap",
        label: "Fermer la feuille ou revenir",
    },
    Shortcut {
        keys: "j / k",
        label: "Ligne suivante / précédente",
    },
    Shortcut {
        keys: "↓ / ↑",
        label: "Ligne suivante / précédente",
    },
    Shortcut {
        keys: "Début / Fin",
        label: "Première / dernière ligne",
    },
    Shortcut {
        keys: "Page préc. / Page suiv.",
        label: "Une page de lignes",
    },
    Shortcut {
        keys: "Entrée",
        label: "Lire à partir d'ici",
    },
    Shortcut {
        keys: "Maj+Entrée",
        label: "Lire ensuite",
    },
    Shortcut {
        keys: "a",
        label: "Ajouter à la file",
    },
    Shortcut {
        keys: "Menu",
        label: "Menu contextuel de la ligne",
    },
    Shortcut {
        keys: "Maj+F10",
        label: "Menu contextuel de la ligne",
    },
    Shortcut {
        keys: "l",
        label: "Aimer le titre",
    },
    Shortcut {
        keys: "d",
        label: "Écarter le titre",
    },
    Shortcut {
        keys: "t",
        label: "Télécharger le titre ou la collection",
    },
    Shortcut {
        keys: "Maj+T",
        label: "Retirer, annuler ou réessayer le téléchargement",
    },
    Shortcut {
        keys: "q",
        label: "Panneau de la file",
    },
    Shortcut {
        keys: "f",
        label: "Lecteur plein écran",
    },
    Shortcut {
        keys: "v",
        label: "Visualiseur",
    },
    Shortcut {
        keys: "Ctrl+= / Ctrl+-",
        label: "Agrandir / réduire l'interface",
    },
    Shortcut {
        keys: "Ctrl+0",
        label: "Taille normale",
    },
    Shortcut {
        keys: "?",
        label: "Cette liste",
    },
    Shortcut {
        keys: "Ctrl+Q",
        label: "Quitter",
    },
];

/// Every binding the app installs. One vector so a test can count them and a
/// key cannot be added to only one of the two lists.
pub fn bindings() -> Vec<KeyBinding> {
    let c = Some(CONTEXT);
    vec![
        // Application-wide, even over a text field: quitting, zooming and the
        // history are not text editing.
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("ctrl-=", ZoomIn, None),
        KeyBinding::new("ctrl-shift-=", ZoomIn, None),
        KeyBinding::new("ctrl--", ZoomOut, None),
        KeyBinding::new("ctrl-0", ZoomReset, None),
        KeyBinding::new("alt-left", Back, c),
        KeyBinding::new("alt-right", Forward, c),
        KeyBinding::new("escape", CloseTop, c),
        KeyBinding::new("shift-/", ShowShortcuts, c),
        // Transport.
        KeyBinding::new("space", PlayPause, c),
        KeyBinding::new("ctrl-right", NextTrack, c),
        KeyBinding::new("ctrl-left", PreviousTrack, c),
        KeyBinding::new("shift-right", SeekForward, c),
        KeyBinding::new("shift-left", SeekBackward, c),
        KeyBinding::new("ctrl-up", VolumeUp, c),
        KeyBinding::new("ctrl-down", VolumeDown, c),
        KeyBinding::new("m", Mute, c),
        KeyBinding::new("s", ToggleShuffle, c),
        KeyBinding::new("r", CycleRepeat, c),
        KeyBinding::new("q", ToggleQueue, c),
        KeyBinding::new("f", ToggleFullscreen, c),
        KeyBinding::new("v", ToggleViz, c),
        // Search and navigation.
        KeyBinding::new("/", GoSearch, c),
        KeyBinding::new("ctrl-k", GoSearch, c),
        KeyBinding::new("g h", GoHome, c),
        KeyBinding::new("g s", GoSearch, c),
        KeyBinding::new("g l", GoLibrary, c),
        KeyBinding::new("g p", GoPodcasts, c),
        KeyBinding::new("g r", GoReleases, c),
        KeyBinding::new("g n", GoNowPlaying, c),
        KeyBinding::new("g q", GoQueue, c),
        KeyBinding::new("g e", GoGenres, c),
        KeyBinding::new("g ,", GoSettings, c),
        // Lists and rows. A screen that does not show a list yet simply has
        // nothing to handle them.
        KeyBinding::new("j", ListNext, c),
        KeyBinding::new("k", ListPrevious, c),
        KeyBinding::new("down", ListNext, c),
        KeyBinding::new("up", ListPrevious, c),
        KeyBinding::new("home", ListFirst, c),
        KeyBinding::new("end", ListLast, c),
        KeyBinding::new("pageup", ListPageUp, c),
        KeyBinding::new("pagedown", ListPageDown, c),
        KeyBinding::new("enter", PlayFromHere, c),
        KeyBinding::new("shift-enter", PlayNextHere, c),
        KeyBinding::new("a", AddHereToQueue, c),
        KeyBinding::new("menu", RowContextMenu, c),
        KeyBinding::new("shift-f10", RowContextMenu, c),
        KeyBinding::new("l", LikeHere, c),
        KeyBinding::new("d", DislikeHere, c),
        // Offline downloads (D18): `t` asks for the focused row (or the page's
        // collection when it has no row), `Maj+T` takes it back, cancels it or
        // asks again — the same two verbs the row's menu and the page's button
        // dispatch.
        KeyBinding::new("t", DownloadSelection, c),
        KeyBinding::new("shift-t", RemoveDownload, c),
        // Login: Tab is gpui-omarchy's focus_scope; Enter submits from any
        // field (the login screen handles it in its own `Input` context too).
        KeyBinding::new("enter", SubmitLogin, c),
    ]
}

/// Install the keymap once, at startup.
pub fn install(cx: &mut App) {
    // A `?` printed by every theme is the one key whose name depends on the
    // layout; GPUI spells it `shift-/` on a US layout, which is the one the
    // Omarchy keyboard uses. The overlay itself is the discovery path.
    cx.bind_keys(bindings());
    let _ = cx.omarchy();
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn every_shortcut_has_a_binding() {
        // The overlay is generated from the table, so a row without a binding
        // would advertise something that does nothing. The keys are compared by
        // their canonical spelling where they are one stroke; grouped rows
        // ("j / k", "Ctrl+= / Ctrl+-") name two, both bound.
        let bound: Vec<String> = bindings()
            .iter()
            .map(|binding| {
                binding
                    .keystrokes()
                    .iter()
                    .map(|stroke| format!("{stroke}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        let mut seen = HashSet::new();
        for row in SHORTCUTS {
            if row.keys.contains(" / ") {
                for part in row.keys.split(" / ") {
                    assert!(
                        bound.iter().any(|key| key_matches(key, part)),
                        "« {} » is advertised but not bound",
                        part
                    );
                }
            } else {
                assert!(
                    bound
                        .iter()
                        .any(|key| key_matches(key, row.keys) || grouped(key, row.keys)),
                    "« {} » is advertised but not bound",
                    row.keys
                );
            }
            assert!(seen.insert(row.keys), "duplicate row {}", row.keys);
        }
    }

    fn grouped(bound: &str, advertised: &str) -> bool {
        // "g h" is bound as two strokes; `Keystroke` prints "g" and "h"
        // separately, and the test joins them with a space already.
        bound == advertised.to_lowercase()
    }

    fn key_matches(bound: &str, advertised: &str) -> bool {
        let normalise = |text: &str| {
            text.to_lowercase()
                .replace("ctrl+", "ctrl-")
                .replace("alt+", "alt-")
                .replace("maj+", "shift-")
        };
        let special = |text: &str| match text {
            "espace" => "space".to_owned(),
            "entrée" => "enter".to_owned(),
            "maj+entrée" => "shift-enter".to_owned(),
            "échap" => "escape".to_owned(),
            "début" => "home".to_owned(),
            "fin" => "end".to_owned(),
            "page préc." => "pageup".to_owned(),
            "page suiv." => "pagedown".to_owned(),
            "menu" => "menu".to_owned(),
            "maj+f10" => "shift-f10".to_owned(),
            "ctrl+→" => "ctrl-right".to_owned(),
            "ctrl+←" => "ctrl-left".to_owned(),
            "maj+→" => "shift-right".to_owned(),
            "maj+←" => "shift-left".to_owned(),
            "ctrl+↑" => "ctrl-up".to_owned(),
            "ctrl+↓" => "ctrl-down".to_owned(),
            "↓" => "down".to_owned(),
            "↑" => "up".to_owned(),
            "alt+←" => "alt-left".to_owned(),
            "alt+→" => "alt-right".to_owned(),
            "?" => "shift-/".to_owned(),
            other => other.to_owned(),
        };
        normalise(bound) == normalise(&special(&advertised.to_lowercase()))
    }

    #[test]
    fn the_table_covers_the_plan() {
        // The rows of *Keyboard AND mouse* in `docs/agents/desktop.md`; a
        // missing one is a key a person has to reach for the mouse to find.
        let keys: HashSet<&str> = SHORTCUTS.iter().map(|row| row.keys).collect();
        for required in [
            "Espace",
            "Ctrl+→",
            "Ctrl+←",
            "Maj+→",
            "Maj+←",
            "Ctrl+↑",
            "Ctrl+↓",
            "m",
            "s",
            "r",
            "/",
            "Ctrl+K",
            "g h",
            "g s",
            "g l",
            "g p",
            "g r",
            "g n",
            "g q",
            "g ,",
            "Alt+←",
            "Alt+→",
            "Échap",
            "j / k",
            "Entrée",
            "Maj+Entrée",
            "a",
            "Menu",
            "l",
            "d",
            "t",
            "Maj+T",
            "q",
            "f",
            "v",
            "Ctrl+= / Ctrl+-",
            "Ctrl+0",
            "Ctrl+Q",
        ] {
            assert!(keys.contains(required), "{required} is missing");
        }
    }

    #[test]
    fn the_transport_is_not_bound_over_a_text_field() {
        // `Neezer && !Input` is the contract that makes Space play instead of
        // typing a space; GPUI evaluates the predicate against the whole focus
        // path, and an editor puts `Input` on it.
        assert!(CONTEXT.contains("!Input"));
        for binding in bindings() {
            let keys: String = binding
                .keystrokes()
                .iter()
                .map(|stroke| format!("{stroke} "))
                .collect();
            if keys.contains("space") || keys.starts_with("s ") || keys.starts_with("a ") {
                assert!(
                    binding.predicate().is_some(),
                    "{keys} is bound with no context and would fire in a text field"
                );
            }
        }
    }
}
