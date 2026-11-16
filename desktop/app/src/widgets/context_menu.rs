//! The row/entity menu: what a right click, the `⋯` button or `Menu`/
//! `Maj+F10` opens (`lib/trackmenu.js`, `ContextMenu.svelte`).
//!
//! It is drawn by hand rather than through `gpui_omarchy::menu` because a row
//! menu has to open from an ACTION as well as from a trigger element: the
//! framework's menu is anchored to a trigger it owns, and the keymap's
//! `RowContextMenu` has no trigger. What it borrows is the language — the same
//! surface, border, row height, hover fill and keycaps — so it does not read as
//! a second menu.
//!
//! The items are the ones D8a can actually carry out. Adding to a playlist and
//! replacing a track open sheets D8b owns (`widgets/playlist_picker.rs`,
//! `widgets/sheets.rs`), so they are NOT listed here: a menu entry that does
//! nothing is a bug (rule 2).

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, Pixels,
    Render, StatefulInteractiveElement as _, Styled, div, px, rems,
};
use gpui_omarchy::{IconName, icon, keycaps};

use crate::theme;
use crate::widgets::cover::Kind;

/// One entry of the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Play,
    PlayNext,
    Queue,
    Like,
    Dislike,
    Radio,
    Credits,
    GoAlbum,
    GoArtist,
    /// A card's own action: open the album / artist / playlist it names.
    Open,
}

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Item::Play => "Lire",
            Item::PlayNext => "Lire ensuite",
            Item::Queue => "Ajouter à la file",
            Item::Like => "Aimer",
            Item::Dislike => "Écarter",
            Item::Radio => "Radio du titre",
            Item::Credits => "Crédits du titre",
            Item::GoAlbum => "Aller à l'album",
            Item::GoArtist => "Aller à l'artiste",
            Item::Open => "Ouvrir",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Item::Play => IconName::Play,
            Item::PlayNext => IconName::SkipForward,
            Item::Queue => IconName::ListMusic,
            Item::Like => IconName::Star,
            Item::Dislike => IconName::Close,
            Item::Radio => IconName::Sparkles,
            Item::Credits => IconName::Info,
            Item::GoAlbum => IconName::Disc3,
            Item::GoArtist => IconName::Music,
            Item::Open => IconName::ChevronRight,
        }
    }

    /// The key that does the same thing without the mouse (`keymap.rs`), so a
    /// person learns the keyboard from the menu.
    fn shortcut(self) -> Option<&'static str> {
        match self {
            Item::Play => Some("Entrée"),
            Item::PlayNext => Some("Maj+Entrée"),
            Item::Queue => Some("a"),
            Item::Like => Some("l"),
            Item::Dislike => Some("d"),
            _ => None,
        }
    }
}

/// The items a track row offers, in the web app's order
/// (`lib/trackmenu.js#buildTrackMenu`), minus the sheets D8b owns.
pub fn track_items(has_album: bool, has_artist: bool) -> Vec<Item> {
    let mut items = vec![Item::Play, Item::PlayNext, Item::Queue];
    items.push(Item::Radio);
    items.push(Item::Like);
    items.push(Item::Dislike);
    items.push(Item::Credits);
    if has_album {
        items.push(Item::GoAlbum);
    }
    if has_artist {
        items.push(Item::GoArtist);
    }
    items
}

/// A card's menu: the two things a sleeve can be opened by, and its radio when
/// the catalogue can build one.
pub fn card_items(kind: Kind, catalogued: bool) -> Vec<Item> {
    let mut items = vec![Item::Open];
    if catalogued {
        items.push(Item::Radio);
    }
    if kind == Kind::Album {
        items.push(Item::GoArtist);
    }
    items
}

/// A menu that is open, where it is, and which row the keyboard is on.
#[derive(Clone, Debug, Default)]
pub struct Menu {
    open: bool,
    /// Where the panel's top edge sits, in the list's own coordinates.
    top: Pixels,
    items: Vec<Item>,
    cursor: usize,
}

impl Menu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open_at(&mut self, top: Pixels, items: Vec<Item>) {
        if items.is_empty() {
            return;
        }
        self.open = true;
        self.top = top;
        self.items = items;
        self.cursor = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
        self.cursor = 0;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The row the keyboard is on: what `Entrée` picks.
    pub fn cursor_item(&self) -> Option<Item> {
        self.open
            .then(|| self.items.get(self.cursor).copied())
            .flatten()
    }

    /// Move the cursor, clamped: a menu is a short list, so wrapping would make
    /// the last row's neighbour the first one by surprise.
    pub fn move_cursor(&mut self, delta: i64) {
        if !self.open || self.items.is_empty() {
            return;
        }
        let last = self.items.len() - 1;
        self.cursor = if delta < 0 {
            self.cursor.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            (self.cursor + delta as usize).min(last)
        };
    }

    /// A pointer's row.
    pub fn set_cursor(&mut self, index: usize) {
        if self.open && index < self.items.len() {
            self.cursor = index;
        }
    }

    /// Draw the panel. `on_pick` runs the item the person chose; the screen
    /// closes the menu first, so a pick that opens a screen cannot leave a panel
    /// floating over it.
    pub fn view<V: Render>(
        &self,
        cx: &mut Context<V>,
        on_pick: impl Fn(&mut V, Item, &mut Context<V>) + Clone + 'static,
        on_close: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
    ) -> AnyElement {
        if !self.open {
            return div().into_any_element();
        }
        let row_height = 2.;
        let mut panel = div()
            .absolute()
            .id("row-menu")
            .left(px(12.))
            .top(self.top)
            .w(rems(15.))
            .py(rems(0.25))
            .flex()
            .flex_col()
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .text_color(theme::text(cx))
            .font_family(theme::font(cx))
            .text_size(rems(0.75));

        for (index, item) in self.items.iter().copied().enumerate() {
            let selected = index == self.cursor;
            panel = panel.child(
                div()
                    .id(("row-menu-item", index))
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .h(rems(row_height))
                    .px(rems(0.5))
                    .bg(if selected {
                        theme::selected(cx)
                    } else {
                        theme::chrome(cx)
                    })
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::selected(cx)))
                    .on_click({
                        let on_pick = on_pick.clone();
                        cx.listener(move |this, _, _window, cx| {
                            cx.stop_propagation();
                            on_pick(this, item, cx);
                        })
                    })
                    .child(
                        icon(item.icon())
                            .size(rems(0.875))
                            .text_color(theme::secondary(cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::NORMAL)
                            .child(item.label()),
                    )
                    .when_some(item.shortcut(), |row, shortcut| {
                        row.child(keycaps(shortcut.split('+').map(str::to_owned), cx))
                    }),
            );
        }
        // The backdrop catches a click outside the panel and is BELOW it, so a
        // press on an item still reaches the item.
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .id("row-menu-backdrop")
                    .absolute()
                    .inset_0()
                    .on_click({
                        let on_close = on_close.clone();
                        cx.listener(move |this, _, _window, cx| {
                            cx.stop_propagation();
                            on_close(this, cx);
                        })
                    }),
            )
            .child(panel)
            .into_any_element()
    }
}

/// Keep the panel inside the viewport: a menu opened on the last visible row
/// must open upwards, not off-screen.
pub fn clamped_top(top: Pixels, list_height: Pixels, item_count: usize) -> Pixels {
    let panel = px(item_count as f32 * 32. + 12.);
    let max = (list_height - panel).max(px(0.));
    top.clamp(px(0.), max)
}
