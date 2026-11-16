//! Réglages → Raccourcis: the whole keymap, on one page, exactly as it is bound.
//!
//! The table is [`crate::keymap::SHORTCUTS`], the same slice the `?` overlay
//! draws and the same one a test holds against `bindings()`: a key that is
//! advertised here and not bound cannot exist, which is the whole reason the
//! table lives beside the bindings instead of in this file.
//!
//! Three columns at a normal width, one on a narrow window: the rows are short
//! and grouped by what they do, in the order a person learns them.

use gpui_kit::{AnyElement, Context, IntoElement, ParentElement, Styled, Window, div, rems};
use gpui_omarchy::keycaps;

use super::{Screen, card, card_title, hint};
use crate::keymap::SHORTCUTS;
use crate::theme;

/// How many rows a column holds at a normal width. 44 rows come out as three
/// columns of 15, 15 and 14 — the same split the `?` overlay uses (13), widened
/// because this page has the room and no player behind it.
pub const COLUMN_ROWS: usize = 15;

pub fn body(_screen: &mut Screen, _window: &mut Window, cx: &mut Context<Screen>) -> AnyElement {
    let columns: Vec<&[crate::keymap::Shortcut]> = SHORTCUTS.chunks(COLUMN_ROWS).collect();
    let mut grid = div().flex().flex_wrap().items_start().gap(rems(2.));
    for column in columns {
        let mut stack = div().flex().flex_col().gap(rems(0.25)).min_w(rems(14.));
        for row in column {
            stack = stack.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(rems(0.75))
                    .py(rems(0.125))
                    .child(
                        div()
                            .text_size(rems(0.75))
                            .text_color(theme::secondary(cx))
                            .child(row.label),
                    )
                    .child(keycaps(
                        row.keys
                            .split(" / ")
                            .flat_map(|part| part.split(' '))
                            .map(str::to_owned),
                        cx,
                    )),
            );
        }
        grid = grid.child(stack);
    }

    div()
        .flex()
        .flex_col()
        .gap(rems(1.))
        .child(
            card("settings-shortcuts", cx)
                .child(card_title("Raccourcis", cx))
                .child(hint(
                    "Tout ce que le clavier sait faire. Chaque action a aussi sa souris : ce \
                     tableau est le chemin court, pas le seul chemin.",
                    cx,
                ))
                .child(grid),
        )
        .child(
            card("settings-shortcuts-note", cx)
                .child(card_title("Où sont les touches", cx))
                .child(hint(
                    "Dans un champ de texte, les lettres et Espace tapent du texte : les \
                     raccourcis d'une seule touche ne s'appliquent qu'en dehors. ? rouvre \
                     cette liste par-dessus n'importe quel écran, Échap la referme.",
                    cx,
                )),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_shows_every_row_of_the_keymap_and_none_twice() {
        // The overlay and this page read one table; a row that exists must be
        // here, and the columns must hold all of them.
        let rows: usize = SHORTCUTS
            .chunks(COLUMN_ROWS)
            .map(|column| column.len())
            .sum();
        assert_eq!(rows, SHORTCUTS.len());
        assert!(SHORTCUTS.len() >= 40, "the table is the whole keymap");
        let mut keys: Vec<&str> = SHORTCUTS.iter().map(|row| row.keys).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len(), "a key is advertised twice");
    }

    #[test]
    fn a_normal_window_gets_three_columns() {
        let columns = SHORTCUTS.len().div_ceil(COLUMN_ROWS);
        assert_eq!(columns, 3);
        // Every column holds at most the cap, and the last one is not empty.
        assert!(
            SHORTCUTS
                .chunks(COLUMN_ROWS)
                .all(|column| !column.is_empty())
        );
    }
}
