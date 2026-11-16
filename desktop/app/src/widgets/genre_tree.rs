//! The genre hierarchy: the DAG of genres, its links and its edits.
//!
//! `components/GenreTree.svelte` and `/genre/tree*` are the reference. Genres
//! form a DAG — `genre_parent` carries weighted multi-parent edges, 1 meaning an
//! outright sub-genre — so a genre may sit under several parents; the engine's
//! *family* is the nearest ancestor the classifier can name. The tree is the
//! server's (cached in its process, versioned in `Meta`), every write bumps the
//! version and tells the devices, and an edit can be undone down to the default
//! tree. This file draws it and asks for edits; it never re-implements the
//! cycle rule or the version, which live on the server.
//!
//! One deliberate desktop shape: the tree is a FLAT list of visible rows with a
//! depth, built from the edges once per change. A DAG drawn recursively would
//! have to decide what to do with a node that has three parents — list it three
//! times, or once under a path — and the flat list answers honestly: once, under
//! its first parent in name order, every other parent shown as a chip on the
//! row. A tree that hides a link is a tree that lies about the vocabulary.

use std::collections::{BTreeMap, BTreeSet};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement as _, Styled, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};
use nz_model::{GenreEdge, GenreNode, GenreTree};

use crate::theme;
use crate::widgets::genre_label::{self, Tag};

/// One row of the flat list.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub name: String,
    pub tag: Tag,
    /// 0 for a root.
    pub depth: usize,
    pub children: usize,
    pub collapsed: bool,
    /// Every parent, in name order: the first is the one the row is drawn under.
    pub parents: Vec<String>,
    /// The engine family, when the tree resolves one.
    pub family: Option<String>,
}

/// The widget's state: the server's tree plus what this screen has done to it.
#[derive(Clone, Debug, Default)]
pub struct Tree {
    pub nodes: Vec<GenreNode>,
    pub edges: Vec<GenreEdge>,
    pub roots: Vec<String>,
    pub version: String,
    /// The server answered from a virtual tree built from the defaults: nothing
    /// has been written yet, so `can_undo` is false there.
    pub default: bool,
    pub can_undo: bool,
    pub collapsed: BTreeSet<String>,
    /// The selected row's index into [`Tree::rows`], not into `nodes`.
    pub cursor: usize,
    /// The node whose parent is being chosen, when the link picker is open.
    pub linking: Option<String>,
    /// The new genre's name, in the create editor.
    pub draft: String,
    pub editor: Option<Editor>,
    pub error: Option<SharedString>,
    pub busy: bool,
}

/// Which field the small editor under the tree is editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    /// A new genre under the selected node (or a root when nothing is
    /// selected).
    Create,
    /// The selected node's name, colour and archetype.
    Rename,
}

impl Tree {
    pub fn set(&mut self, tree: &GenreTree) {
        self.nodes = tree.nodes.clone();
        self.edges = tree.edges.clone();
        self.roots = tree.roots.clone();
        self.version = tree.version.clone();
        self.default = tree.default;
        self.can_undo = tree.can_undo;
        self.error = None;
        self.busy = false;
        // A node that disappeared from the server's answer must not stay
        // collapsed for ever under a name nobody has any more.
        let names: BTreeSet<&str> = self.nodes.iter().map(|node| node.name.as_str()).collect();
        self.collapsed.retain(|name| names.contains(name.as_str()));
    }

    /// The tag of one node, with the family the tree resolved.
    pub fn tag(&self, name: &str) -> Option<Tag> {
        self.nodes
            .iter()
            .find(|node| node.name == name)
            .map(|node| {
                let mut tag = Tag::from_node(node);
                tag.family = tag.family.or_else(|| self.family_of(name));
                tag
            })
    }

    /// The nearest ancestor the engine can name, from the server's own field
    /// (`_tree_json` sends it per node); `None` when the tree has no answer.
    pub fn family_of(&self, name: &str) -> Option<String> {
        self.nodes
            .iter()
            .find(|node| node.name == name)
            .and_then(|node| node.family.clone())
    }

    /// Every parent of `name`, in name order.
    pub fn parents_of(&self, name: &str) -> Vec<String> {
        let mut parents: Vec<String> = self
            .edges
            .iter()
            .filter(|edge| edge.child == name)
            .map(|edge| edge.parent.clone())
            .collect();
        parents.sort();
        parents.dedup();
        parents
    }

    fn children_of(&self, name: &str) -> Vec<String> {
        let mut children: Vec<String> = self
            .edges
            .iter()
            .filter(|edge| edge.parent == name)
            .map(|edge| edge.child.clone())
            .collect();
        children.sort();
        children.dedup();
        children
    }

    /// The visible rows: roots in name order, each expanded node's children
    /// under it, a node drawn ONCE (under its first parent in name order).
    pub fn rows(&self) -> Vec<Row> {
        let known: BTreeSet<&str> = self.nodes.iter().map(|node| node.name.as_str()).collect();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut rows = Vec::new();
        let mut roots: Vec<String> = self
            .roots
            .iter()
            .filter(|name| known.contains(name.as_str()))
            .cloned()
            .collect();
        // A node with no edge at all is a root even if `roots` forgot it (the
        // vocabulary seed writes tags before the tree does).
        for node in &self.nodes {
            if self.parents_of(&node.name).is_empty() && !roots.contains(&node.name) {
                roots.push(node.name.clone());
            }
        }
        roots.sort();
        roots.dedup();
        for root in roots {
            self.walk(&root, 0, &mut seen, &mut rows);
        }
        // A node reachable only through an edge the server did not list as a
        // root is still part of the tree: paint it rather than lose it.
        for node in &self.nodes {
            if !seen.contains(&node.name) {
                self.walk(&node.name, 0, &mut seen, &mut rows);
            }
        }
        rows
    }

    fn walk(&self, name: &str, depth: usize, seen: &mut BTreeSet<String>, rows: &mut Vec<Row>) {
        if !seen.insert(name.to_owned()) {
            return;
        }
        let children = self.children_of(name);
        let collapsed = self.collapsed.contains(name);
        let tag = self.tag(name).unwrap_or_else(|| Tag::new(0, name));
        rows.push(Row {
            name: name.to_owned(),
            tag,
            depth,
            children: children.len(),
            collapsed,
            parents: self.parents_of(name),
            family: self.family_of(name),
        });
        if collapsed {
            // A collapsed subtree is HIDDEN, not absent: its nodes must not be
            // re-painted later as orphans by the "unreachable" sweep below.
            for child in children {
                self.mark_seen(&child, seen);
            }
            return;
        }
        for child in children {
            self.walk(&child, depth + 1, seen, rows);
        }
    }

    fn mark_seen(&self, name: &str, seen: &mut BTreeSet<String>) {
        if !seen.insert(name.to_owned()) {
            return;
        }
        for child in self.children_of(name) {
            self.mark_seen(&child, seen);
        }
    }

    pub fn selected(&self) -> Option<Row> {
        self.rows().into_iter().nth(self.cursor)
    }

    pub fn select(&mut self, cursor: usize) {
        let len = self.rows().len();
        self.cursor = cursor.min(len.saturating_sub(1));
    }

    pub fn move_by(&mut self, delta: i64) {
        let len = self.rows().len() as i64;
        if len == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = (self.cursor as i64 + delta).clamp(0, len - 1) as usize;
    }

    /// Open/close one node; a collapsed subtree disappears from the list, which
    /// is also how a 200-genre vocabulary stays readable.
    pub fn toggle(&mut self, name: &str) {
        if !self.collapsed.remove(name) {
            self.collapsed.insert(name.to_owned());
        }
    }

    /// Toggle the row under the cursor (the keyboard's `Entrée` on the tree).
    pub fn toggle_selected(&mut self) {
        if let Some(row) = self.selected()
            && row.children > 0
        {
            self.toggle(&row.name);
        }
    }

    /// What a create/rename request would send — the editor's own values.
    pub fn draft_name(&self) -> String {
        self.draft.trim().to_owned()
    }

    pub fn counts(&self) -> (usize, usize) {
        (self.nodes.len(), self.edges.len())
    }

    /// The nodes a link may point at, matching what was typed: folded, name
    /// first then anywhere. The selected node itself is excluded (the server
    /// refuses a cycle; the sheet does not offer one).
    pub fn link_candidates(&self, child: &str, query: &str) -> Vec<&GenreNode> {
        let mut out: Vec<&GenreNode> = self
            .nodes
            .iter()
            .filter(|node| node.name != child)
            .filter(|node| {
                let q = genre_label::fold(query);
                q.is_empty() || genre_label::fold(&node.name).contains(&q)
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }
}

/// What the tree asks the screen to do. Every one is ONE request; the screen
/// owns the HTTP and the optimistic state.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Select(usize),
    Toggle(String),
    CreateRoot,
    CreateChild,
    Rename,
    /// The editor's OK: create or rename, depending on which editor is open.
    SubmitEditor,
    Delete,
    Cut(String),
    LinkOpen,
    LinkClose,
    Link(String),
    Undo,
    Reset,
    CancelEditor,
}

/// The tree, drawn: a toolbar, the visible rows, the small editor under it.
pub fn view<V: Render>(
    tree: &Tree,
    draft_field: Option<AnyElement>,
    cx: &mut Context<V>,
    on: impl Fn(&mut V, Action, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    let (nodes, edges) = tree.counts();
    let selected = tree.selected();
    let mut column = div()
        .id("genre-tree")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .gap(rems(0.75))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .flex_wrap()
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
                                .child("Arbre des genres"),
                        )
                        .child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::secondary(cx))
                                .child(format!(
                                    "{nodes} genres · {edges} liens · version {}",
                                    if tree.version.is_empty() {
                                        "—"
                                    } else {
                                        tree.version.as_str()
                                    }
                                )),
                        ),
                )
                .child(
                    button(
                        "genre-tree-new",
                        "Nouveau genre",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .on_click({
                        let on = on.clone();
                        cx.listener(move |this, _, _, cx| on(this, Action::CreateRoot, cx))
                    }),
                )
                .child(
                    button("genre-tree-undo", "Annuler", ButtonVariant::Outline, cx)
                        .disabled(!tree.can_undo || tree.busy)
                        .on_click({
                            let on = on.clone();
                            cx.listener(move |this, _, _, cx| on(this, Action::Undo, cx))
                        }),
                )
                .child(
                    button(
                        "genre-tree-reset",
                        "Arbre d'origine",
                        ButtonVariant::Outline,
                        cx,
                    )
                    .disabled(tree.busy)
                    .on_click({
                        let on = on.clone();
                        cx.listener(move |this, _, _, cx| on(this, Action::Reset, cx))
                    }),
                ),
        );
    if let Some(error) = &tree.error {
        column = column.child(
            div()
                .text_size(rems(0.75))
                .text_color(theme::danger(cx))
                .child(error.clone()),
        );
    }
    let mut list = div()
        .id("genre-tree-rows")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .border_1()
        .border_color(theme::border(cx))
        .bg(theme::chrome(cx));
    let rows = tree.rows();
    if rows.is_empty() {
        list = list.child(
            div()
                .p(rems(0.75))
                .text_size(rems(0.75))
                .text_color(theme::secondary(cx))
                .child("Aucun genre : créez-en un, ou laissez le serveur semer le vocabulaire du moteur."),
        );
    }
    for (index, row) in rows.iter().enumerate() {
        let selected = index == tree.cursor;
        let has_children = row.children > 0;
        let mut line = div()
            .id(("genre-tree-row", index))
            .flex()
            .items_center()
            .gap(rems(0.375))
            .py(rems(0.25))
            .pr(rems(0.5))
            .pl(rems(0.375 + 0.75 * row.depth as f32))
            .border_b_1()
            .border_color(theme::border(cx))
            .text_size(rems(0.8125))
            .cursor_pointer()
            .when(selected, |line| line.bg(theme::selected(cx)))
            .hover(|style| style.bg(theme::selected(cx).opacity(0.6)))
            .on_click({
                let on = on.clone();
                cx.listener(move |this, _, _, cx| on(this, Action::Select(index), cx))
            })
            .child(
                div()
                    .id(("genre-tree-toggle", index))
                    .flex_shrink_0()
                    .w(rems(0.75))
                    .h(rems(0.75))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .when(has_children, |toggle| {
                        toggle.on_click({
                            let name = row.name.clone();
                            let on = on.clone();
                            cx.listener(move |this, _, _, cx| {
                                on(this, Action::Toggle(name.clone()), cx)
                            })
                        })
                    })
                    .child(if has_children {
                        icon(if row.collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .into_any_element()
                    } else {
                        div().into_any_element()
                    }),
            )
            .child(genre_label::dot(&row.tag, px(8.), cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(SharedString::from(row.name.clone())),
            )
            .when(row.family.is_some(), |line| {
                let family = row.family.clone().unwrap_or_default();
                line.child(
                    div()
                        .flex_shrink_0()
                        .text_size(rems(0.625))
                        .text_color(theme::secondary(cx))
                        .child(format!("famille : {family}")),
                )
            })
            .when(row.children > 0, |line| {
                line.child(
                    div()
                        .flex_shrink_0()
                        .text_size(rems(0.625))
                        .font_family(theme::mono(cx))
                        .text_color(theme::secondary(cx))
                        .child(format!("{}", row.children)),
                )
            });
        // A second parent is a chip on the row: the tree must not pretend the
        // node has one path when the DAG gives it three.
        for parent in row.parents.iter().skip(1) {
            line = line.child(
                div()
                    .flex_shrink_0()
                    .text_size(rems(0.625))
                    .text_color(theme::secondary(cx))
                    .child(format!("sous {parent}")),
            );
        }
        list = list.child(line);
    }
    column = column.child(list);

    if let Some(row) = selected {
        let mut actions = div()
            .flex()
            .items_center()
            .gap(rems(0.375))
            .flex_wrap()
            .child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(format!("Sélection : {}", row.name)),
            )
            .child(div().flex_1())
            .child(
                button("genre-tree-child", "Sous-genre", ButtonVariant::Outline, cx).on_click({
                    let on = on.clone();
                    cx.listener(move |this, _, _, cx| on(this, Action::CreateChild, cx))
                }),
            )
            .child(
                button("genre-tree-rename", "Renommer", ButtonVariant::Outline, cx).on_click({
                    let on = on.clone();
                    cx.listener(move |this, _, _, cx| on(this, Action::Rename, cx))
                }),
            )
            .child(
                button("genre-tree-link", "Relier…", ButtonVariant::Outline, cx).on_click({
                    let on = on.clone();
                    cx.listener(move |this, _, _, cx| on(this, Action::LinkOpen, cx))
                }),
            )
            .child(
                button("genre-tree-delete", "Supprimer", ButtonVariant::Danger, cx)
                    .disabled(row.children > 0)
                    .on_click({
                        let on = on.clone();
                        cx.listener(move |this, _, _, cx| on(this, Action::Delete, cx))
                    }),
            );
        // A parent can be cut, so a wrong link costs one click rather than a
        // reset of the whole tree.
        for parent in row.parents.iter().skip(1).chain(row.parents.iter().take(1)) {
            actions = actions.child(
                button(
                    format!("genre-tree-cut-{parent}"),
                    format!("Retirer le lien vers {parent}"),
                    ButtonVariant::Outline,
                    cx,
                )
                .on_click({
                    let parent = parent.clone();
                    let on = on.clone();
                    cx.listener(move |this, _, _, cx| on(this, Action::Cut(parent.clone()), cx))
                }),
            );
        }
        column = column.child(actions);
        if row.children > 0 {
            column = column.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(
                        "Ce genre a des sous-genres : supprimez-les d'abord, ou retirez leur lien.",
                    ),
            );
        }
    }

    if let Some(editor) = tree.editor {
        let (title, hint, verb) = match editor {
            Editor::Create => (
                "Nouveau genre",
                "Le nom tel que vous l'écrivez : il devient une étiquette de la liste.",
                "Créer",
            ),
            Editor::Rename => (
                "Renommer le genre",
                "Le nouveau nom remplace l'ancien partout ; les titres étiquetés le suivent.",
                "Renommer",
            ),
        };
        column = column.child(
            div()
                .id("genre-tree-editor")
                .flex()
                .flex_col()
                .gap(rems(0.375))
                .p(rems(0.75))
                .border_1()
                .border_color(theme::border(cx))
                .bg(theme::chrome(cx))
                .child(
                    div()
                        .text_size(rems(0.8125))
                        .font_weight(FontWeight::BOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .child(hint),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        // The name is the page's one text field, focused when
                        // the editor opens (the screen owns the input and its
                        // draft, so a keystroke never rebuilds the tree).
                        .child(
                            div()
                                .flex_1()
                                .min_h(rems(1.75))
                                .flex()
                                .items_center()
                                .child(draft_field.unwrap_or_else(|| {
                                    div()
                                        .text_size(rems(0.75))
                                        .text_color(theme::secondary(cx))
                                        .child("Nom du genre")
                                        .into_any_element()
                                })),
                        )
                        .child(
                            button("genre-tree-editor-ok", verb, ButtonVariant::Primary, cx)
                                .disabled(tree.draft_name().is_empty() || tree.busy)
                                .on_click({
                                    let on = on.clone();
                                    cx.listener(move |this, _, _, cx| {
                                        on(this, Action::SubmitEditor, cx)
                                    })
                                }),
                        )
                        .child(
                            button(
                                "genre-tree-editor-cancel",
                                "Annuler",
                                ButtonVariant::Outline,
                                cx,
                            )
                            .on_click({
                                let on = on.clone();
                                cx.listener(move |this, _, _, cx| {
                                    on(this, Action::CancelEditor, cx)
                                })
                            }),
                        ),
                ),
        );
    }

    if let Some(child) = &tree.linking {
        let mut picker = div()
            .id("genre-tree-links")
            .flex()
            .flex_wrap()
            .gap(rems(0.375))
            .max_h(rems(9.))
            .overflow_y_scroll()
            .p(rems(0.75))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .w_full()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(format!("Relier « {child} » sous…")),
            );
        // The DAG lets a genre have several parents; the picker offers every
        // node, the selected one's existing parents included (the server's
        // cycle rule is the only refusal that matters).
        let child = child.clone();
        for node in tree.nodes.iter().filter(|node| node.name != child) {
            let name = node.name.clone();
            let tag = Tag::from_node(node);
            picker = picker.child(
                genre_label::chip(
                    format!("genre-tree-link-{}", node.id.max(0)),
                    &tag,
                    None,
                    false,
                    cx,
                )
                .on_click({
                    let on = on.clone();
                    cx.listener(move |this, _, _, cx| on(this, Action::Link(name.clone()), cx))
                }),
            );
        }
        picker = picker.child(
            button(
                "genre-tree-link-cancel",
                "Fermer",
                ButtonVariant::Outline,
                cx,
            )
            .on_click({
                let on = on.clone();
                cx.listener(move |this, _, _, cx| on(this, Action::LinkClose, cx))
            }),
        );
        column = column.child(picker);
    }

    column.into_any_element()
}

/// A small vocabulary helper the tagging tab's "create it here" line uses: the
/// request body `POST /genre/tags` takes (`name`, `color`, `archetype`).
pub fn create_body(name: &str) -> serde_json::Value {
    serde_json::json!({"name": name, "color": null, "archetype": null})
}

/// The parents' counts, for the preview's own words: `{node: children}`.
pub fn children_counts(tree: &GenreTree) -> BTreeMap<String, u32> {
    let mut out: BTreeMap<String, u32> = BTreeMap::new();
    for edge in &tree.edges {
        *out.entry(edge.parent.clone()).or_insert(0) += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Tree {
        let mut tree = Tree::default();
        let body = serde_json::json!({
            "version": "v3",
            "default": false,
            "can_undo": true,
            "nodes": [
                {"id": 1, "name": "Hardcore", "color": "#ff0000", "archetype": "hard", "family": "hardcore"},
                {"id": 2, "name": "Frenchcore", "archetype": "hard", "family": "hardcore"},
                {"id": 3, "name": "Hardtek"},
                {"id": 4, "name": "Acid"}
            ],
            "edges": [
                {"child": "Frenchcore", "parent": "Hardcore", "weight": 1.0},
                {"child": "Hardtek", "parent": "Hardcore", "weight": 0.5}
            ],
            "roots": ["Hardcore", "Acid"]
        });
        let tree_reply: GenreTree = serde_json::from_value(body).expect("tree");
        tree.set(&tree_reply);
        tree
    }

    #[test]
    fn the_flat_list_follows_the_edges_and_the_roots() {
        let tree = tree();
        let rows = tree.rows();
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["Acid", "Hardcore", "Frenchcore", "Hardtek"]);
        assert_eq!(rows[2].depth, 1);
        assert_eq!(rows[2].parents, ["Hardcore"]);
        assert_eq!(rows[2].family.as_deref(), Some("hardcore"));
    }

    #[test]
    fn collapsing_a_parent_hides_its_subtree_and_keeps_the_selection_sane() {
        let mut tree = tree();
        tree.select(1);
        assert_eq!(
            tree.selected().map(|row| row.name.clone()),
            Some("Hardcore".to_owned())
        );
        tree.toggle("Hardcore");
        let names: Vec<String> = tree.rows().into_iter().map(|row| row.name).collect();
        assert_eq!(names, ["Acid", "Hardcore"]);
        tree.toggle("Hardcore");
        assert_eq!(tree.rows().len(), 4);
    }

    #[test]
    fn a_node_with_two_parents_is_drawn_once_and_names_the_other() {
        let mut tree = tree();
        tree.edges.push(GenreEdge {
            child: "Frenchcore".to_owned(),
            parent: "Acid".to_owned(),
            weight: 0.5,
        });
        let rows = tree.rows();
        let french = rows
            .iter()
            .find(|row| row.name == "Frenchcore")
            .expect("Frenchcore");
        assert_eq!(french.parents, ["Acid", "Hardcore"]);
        assert_eq!(
            rows.iter().filter(|row| row.name == "Frenchcore").count(),
            1,
            "the DAG draws it once"
        );
    }

    #[test]
    fn a_link_candidate_list_never_offers_the_node_itself() {
        let tree = tree();
        let names: Vec<&str> = tree
            .link_candidates("Frenchcore", "")
            .into_iter()
            .map(|node| node.name.as_str())
            .collect();
        assert!(!names.contains(&"Frenchcore"));
        assert_eq!(names, ["Acid", "Hardcore", "Hardtek"]);
        let filtered: Vec<&str> = tree
            .link_candidates("Frenchcore", "hard")
            .into_iter()
            .map(|node| node.name.as_str())
            .collect();
        assert_eq!(filtered, ["Hardcore", "Hardtek"]);
    }

    #[test]
    fn a_create_body_is_the_route_the_server_takes() {
        assert_eq!(create_body("Hardtekk")["name"], "Hardtekk");
        assert!(create_body("Hardtekk")["color"].is_null());
    }
}
