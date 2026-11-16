//! The playlist folder tree: what a folder holds, what a keyboard does in it,
//! and which `PUT …/order` a move means.
//!
//! The model is pure on purpose. The library screen owns the network and the
//! drawing; this widget owns the one part a test can pin without a renderer:
//! the order `lib/playlistfolders.js` produces (`groupPlaylists`, `moveItem`,
//! `orderFolders`, `orderContainer`) and the two moves a person makes — a
//! folder among folders, a playlist inside its container — mapped onto the
//! endpoint each one calls. A tree that moves on screen but writes the wrong
//! order is a playlist that jumps back on the next open, so every rule here is
//! also an assertion.

use std::collections::HashSet;

use nz_model::{Playlist, PlaylistFolder};

/// What a row of the tree is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// A folder and how many playlists it holds.
    Folder { id: String, count: usize },
    /// A playlist, with the folder it sits in (`None` at the root).
    Playlist { id: String, folder: Option<String> },
}

/// One visible row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// Stable key: `folder:<id>` or `playlist:<id>`. The collapse state and the
    /// selection use it, so a reload keeps both.
    pub key: String,
    pub kind: NodeKind,
    pub title: String,
    /// `0` for a folder, `1` for a playlist: the indent of the row.
    pub depth: usize,
}

/// The grouping `groupPlaylists` produces: folders in their own order, then
/// the playlists that sit in none (a playlist whose `folder_id` no longer
/// exists belongs here too, never lost).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grouping {
    pub folders: Vec<(String, String, usize)>,
    pub unfiled: Vec<String>,
    /// A playlist that came from Deezer and belongs in the "Depuis Deezer"
    /// section, not among the local ones.
    pub imported: Vec<String>,
}

/// Group `playlists` under `folders`, the way the web app does.
pub fn group(playlists: &[Playlist], folders: &[PlaylistFolder]) -> Grouping {
    let mut grouping = Grouping {
        folders: folders
            .iter()
            .map(|folder| (folder.id.clone(), folder.title.clone(), 0))
            .collect(),
        ..Grouping::default()
    };
    let known: HashSet<&str> = folders.iter().map(|folder| folder.id.as_str()).collect();
    for playlist in playlists {
        let folder = playlist
            .folder_id
            .as_deref()
            .filter(|id| known.contains(id));
        match folder {
            Some(id) => {
                if let Some(entry) = grouping.folders.iter_mut().find(|entry| entry.0 == id) {
                    entry.2 += 1;
                }
            }
            None if playlist.folder_id.is_none() && playlist.deezer_id.is_some() => {
                grouping.imported.push(playlist.id.as_str().to_owned());
            }
            None => grouping.unfiled.push(playlist.id.as_str().to_owned()),
        }
    }
    grouping
}

/// `moveItem`: `(list, from, to)` with both ends clamped, the item taken out and
/// put back at `to`. The web app's own arithmetic, so a move lands where the
/// person saw it.
pub fn move_item<T: Clone>(list: &[T], from: usize, to: usize) -> Vec<T> {
    let mut next = list.to_vec();
    if next.is_empty() {
        return next;
    }
    let from = from.min(next.len() - 1);
    let to = to.min(next.len() - 1);
    if from == to {
        return next;
    }
    let item = next.remove(from);
    next.insert(to, item);
    next
}

/// The order a folder move writes: `PUT /api/playlist-folders/order`.
pub fn folder_order(folders: &[PlaylistFolder]) -> Vec<String> {
    folders.iter().map(|folder| folder.id.clone()).collect()
}

/// The order a playlist move writes: `PUT /api/playlist-folders/{id|root}/order`.
pub fn container_order(playlists: &[Playlist], folder: Option<&str>) -> Vec<String> {
    playlists
        .iter()
        .filter(|playlist| playlist.folder_id.as_deref().filter(|id| !id.is_empty()) == folder)
        .map(|playlist| playlist.id.as_str().to_owned())
        .collect()
}

/// Where a move in the tree writes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The folders' own order.
    Folders,
    /// The playlists of one folder, or of the root (`None`).
    Container(Option<String>),
}

/// The tree as the screen draws it: the folders in order, each followed by its
/// playlists unless it is collapsed, then the playlists that sit in no folder.
#[derive(Debug, Default)]
pub struct FolderTree {
    pub nodes: Vec<Node>,
    pub selected: usize,
    pub collapsed: HashSet<String>,
    /// Rows per page for `PgUp`/`PgDn`: the screen sets what it measured.
    pub page: usize,
}

impl FolderTree {
    pub fn new() -> Self {
        Self {
            page: 10,
            ..Self::default()
        }
    }

    /// Rebuild the rows from the library's answer, keeping the selection on the
    /// same row when it is still there (a reload must not move the cursor).
    pub fn rebuild(&mut self, playlists: &[Playlist], folders: &[PlaylistFolder]) {
        let previous = self.nodes.get(self.selected).map(|node| node.key.clone());
        let mut nodes = Vec::new();
        for folder in folders {
            let key = format!("folder:{}", folder.id);
            nodes.push(Node {
                key: key.clone(),
                kind: NodeKind::Folder {
                    id: folder.id.clone(),
                    count: playlists
                        .iter()
                        .filter(|playlist| {
                            playlist.folder_id.as_deref() == Some(folder.id.as_str())
                        })
                        .count(),
                },
                title: folder.title.clone(),
                depth: 0,
            });
            if self.collapsed.contains(&key) {
                continue;
            }
            for playlist in playlists {
                if playlist.folder_id.as_deref() == Some(folder.id.as_str()) {
                    nodes.push(Node {
                        key: format!("playlist:{}", playlist.id),
                        kind: NodeKind::Playlist {
                            id: playlist.id.as_str().to_owned(),
                            folder: Some(folder.id.clone()),
                        },
                        title: playlist.title.clone(),
                        depth: 1,
                    });
                }
            }
        }
        let known: HashSet<&str> = folders.iter().map(|folder| folder.id.as_str()).collect();
        for playlist in playlists {
            let filed = playlist
                .folder_id
                .as_deref()
                .is_some_and(|id| known.contains(id));
            if filed {
                continue;
            }
            nodes.push(Node {
                key: format!("playlist:{}", playlist.id),
                kind: NodeKind::Playlist {
                    id: playlist.id.as_str().to_owned(),
                    folder: None,
                },
                title: playlist.title.clone(),
                depth: 0,
            });
        }
        self.nodes = nodes;
        self.selected = previous
            .and_then(|key| self.nodes.iter().position(|node| node.key == key))
            .unwrap_or_else(|| self.selected.min(self.nodes.len().saturating_sub(1)));
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn current(&self) -> Option<&Node> {
        self.nodes.get(self.selected)
    }

    /// `j`/`↓`: the next row, clamped at the end (a list does not wrap: wrapping
    /// hides where the end is).
    pub fn next(&mut self) {
        if !self.nodes.is_empty() {
            self.selected = (self.selected + 1).min(self.nodes.len() - 1);
        }
    }

    pub fn previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn first(&mut self) {
        self.selected = 0;
    }

    pub fn last(&mut self) {
        self.selected = self.nodes.len().saturating_sub(1);
    }

    pub fn page_up(&mut self) {
        self.selected = self.selected.saturating_sub(self.page.max(1));
    }

    pub fn page_down(&mut self) {
        if !self.nodes.is_empty() {
            self.selected = (self.selected + self.page.max(1)).min(self.nodes.len() - 1);
        }
    }

    /// Space / Enter on a folder row folds it; on a playlist row it is the
    /// screen's business (opening the playlist). The caller REBUILDS afterwards
    /// (`rebuild` is what turns the fold into fewer rows), which is also how the
    /// screen keeps the selection on the row it folded.
    pub fn toggle(&mut self) -> bool {
        let Some(node) = self.current() else {
            return false;
        };
        let key = node.key.clone();
        if !matches!(node.kind, NodeKind::Folder { .. }) {
            return false;
        }
        if !self.collapsed.remove(&key) {
            self.collapsed.insert(key);
        }
        true
    }

    /// `Alt+↑`/`Alt+↓` on the focused row. Returns the new order and where it
    /// must be written, or `None` when the move means nothing (the first folder
    /// up, the last playlist down, a playlist out of its own container).
    pub fn shift(
        &mut self,
        delta: isize,
        playlists: &[Playlist],
        folders: &[PlaylistFolder],
    ) -> Option<(Target, Vec<String>)> {
        let node = self.nodes.get(self.selected)?.clone();
        match node.kind {
            NodeKind::Folder { ref id, .. } => {
                let from = folders.iter().position(|folder| &folder.id == id)?;
                let to = from.checked_add_signed(delta)?;
                if to >= folders.len() {
                    return None;
                }
                let moved = move_item(folders, from, to);
                self.rebuild(playlists, &moved);
                Some((Target::Folders, folder_order(&moved)))
            }
            NodeKind::Playlist { ref id, ref folder } => {
                // The rows of the container, in the order the screen shows.
                let container: Vec<&Playlist> = match folder.as_deref() {
                    Some(folder) => playlists
                        .iter()
                        .filter(|playlist| playlist.folder_id.as_deref() == Some(folder))
                        .collect(),
                    None => playlists
                        .iter()
                        .filter(|playlist| {
                            playlist
                                .folder_id
                                .as_deref()
                                .is_none_or(|id| !folders.iter().any(|folder| folder.id == id))
                        })
                        .collect(),
                };
                let from = container
                    .iter()
                    .position(|playlist| playlist.id.as_str() == id)?;
                let to = from.checked_add_signed(delta)?;
                if to >= container.len() {
                    return None;
                }
                let moved: Vec<&Playlist> = move_item(&container, from, to);
                let ids: Vec<String> = moved
                    .iter()
                    .map(|playlist| playlist.id.as_str().to_owned())
                    .collect();
                Some((Target::Container(folder.clone()), ids))
            }
        }
    }

    /// The request a move becomes, for the screen's log line and its test.
    pub fn describe(target: &Target) -> String {
        match target {
            Target::Folders => "PUT /api/playlist-folders/order".to_owned(),
            Target::Container(None) => "PUT /api/playlist-folders/root/order".to_owned(),
            Target::Container(Some(folder)) => format!("PUT /api/playlist-folders/{folder}/order"),
        }
    }
}

#[cfg(test)]
mod tests {
    use nz_model::{Playlist, PlaylistFolder};

    use super::*;

    fn playlist(id: &str, title: &str, folder: Option<&str>, deezer: bool) -> Playlist {
        Playlist {
            id: nz_model::PlaylistId::new(id),
            title: title.to_owned(),
            folder_id: folder.map(str::to_owned),
            deezer_id: deezer.then(|| format!("dz-{id}")),
            ..Playlist::default()
        }
    }

    fn folder(id: &str, title: &str) -> PlaylistFolder {
        PlaylistFolder {
            id: id.to_owned(),
            title: title.to_owned(),
            ..PlaylistFolder::default()
        }
    }

    #[test]
    fn a_playlist_whose_folder_vanished_lands_at_the_root_not_nowhere() {
        // Two ways to be unfiled: no folder at all, or a folder the library no
        // longer lists (a folder deleted on another device while this one was
        // open). Both must stay reachable.
        let folders = vec![folder("f1", "Route")];
        let playlists = vec![
            playlist("p1", "Dans le dossier", Some("f1"), false),
            playlist("p2", "Sans dossier", None, false),
            playlist("p3", "Dossier disparu", Some("f9"), false),
            playlist("p4", "Depuis Deezer", None, true),
        ];
        let grouping = group(&playlists, &folders);
        assert_eq!(grouping.folders.len(), 1);
        assert_eq!(grouping.folders[0].2, 1, "one playlist in the folder");
        assert_eq!(grouping.unfiled, ["p2", "p3"]);
        assert_eq!(grouping.imported, ["p4"]);
    }

    #[test]
    fn the_tree_folds_a_folder_and_keeps_the_cursor_on_its_row() {
        let folders = vec![folder("f1", "Route"), folder("f2", "Soirée")];
        let playlists = vec![
            playlist("p1", "Autoroute", Some("f1"), false),
            playlist("p2", "Minuit", Some("f2"), false),
            playlist("p3", "Racine", None, false),
        ];
        let mut tree = FolderTree::new();
        tree.rebuild(&playlists, &folders);
        assert_eq!(tree.len(), 5, "2 folders + 3 playlists");
        assert_eq!(tree.nodes[1].depth, 1, "a playlist is indented");
        tree.first();
        assert!(tree.toggle(), "the cursor starts on the first folder");
        tree.rebuild(&playlists, &folders);
        assert_eq!(tree.len(), 4);
        assert_eq!(
            tree.current().map(|node| node.key.as_str()),
            Some("folder:f1")
        );
        // Folding again brings it back, and the cursor follows the same row.
        tree.rebuild(&playlists, &folders);
        assert_eq!(tree.len(), 4, "the fold survives a rebuild");
        assert_eq!(
            tree.current().map(|node| node.key.as_str()),
            Some("folder:f1")
        );
        assert!(tree.toggle());
        tree.rebuild(&playlists, &folders);
        assert_eq!(tree.len(), 5);
    }

    #[test]
    fn a_move_writes_the_order_it_shows_and_never_wraps() {
        let folders = vec![folder("f1", "A"), folder("f2", "B"), folder("f3", "C")];
        let playlists = vec![
            playlist("p1", "Un", Some("f1"), false),
            playlist("p2", "Deux", Some("f1"), false),
            playlist("p3", "Trois", Some("f1"), false),
        ];
        let mut tree = FolderTree::new();
        tree.rebuild(&playlists, &folders);

        // The first folder up is not a move.
        assert_eq!(tree.shift(-1, &playlists, &folders), None);
        // Down moves it and returns the whole new order.
        assert_eq!(folder_order(&folders), ["f1", "f2", "f3"]);
        let (target, order) = tree.shift(1, &playlists, &folders).expect("a move");
        assert_eq!(target, Target::Folders);
        assert_eq!(order, ["f2", "f1", "f3"]);
        assert_eq!(
            FolderTree::describe(&target),
            "PUT /api/playlist-folders/order"
        );

        // A playlist moves inside its own container only.
        tree.first();
        let mut tree = FolderTree::new();
        tree.rebuild(&playlists, &folders);
        tree.selected = 1; // p1, first playlist of f1
        assert_eq!(
            tree.shift(-1, &playlists, &folders),
            None,
            "top of the folder"
        );
        let (target, order) = tree.shift(1, &playlists, &folders).expect("a move");
        assert_eq!(target, Target::Container(Some("f1".to_owned())));
        assert_eq!(order, ["p2", "p1", "p3"]);
        assert_eq!(
            FolderTree::describe(&target),
            "PUT /api/playlist-folders/f1/order"
        );
        // p3 down is the end of the container.
        tree.selected = 3;
        assert_eq!(tree.shift(1, &playlists, &folders), None);
    }

    #[test]
    fn a_root_move_names_the_root_route_and_leaves_other_containers_alone() {
        let folders = vec![folder("f1", "A")];
        let playlists = vec![
            playlist("p1", "Un", None, false),
            playlist("p2", "Deux", Some("f1"), false),
            playlist("p3", "Trois", None, false),
        ];
        let mut tree = FolderTree::new();
        tree.rebuild(&playlists, &folders);
        tree.selected = tree
            .nodes
            .iter()
            .position(|node| node.key == "playlist:p1")
            .expect("p1 is listed at the root");
        let (target, order) = tree.shift(1, &playlists, &folders).expect("a move");
        assert_eq!(target, Target::Container(None));
        assert_eq!(order, ["p3", "p1"], "only the root's own rows");
        assert_eq!(
            FolderTree::describe(&target),
            "PUT /api/playlist-folders/root/order"
        );
        assert_eq!(
            container_order(&playlists, Some("f1")),
            ["p2"],
            "a folder's own order is untouched"
        );
    }

    #[test]
    fn the_keyboard_walks_the_rows_and_stops_at_both_ends() {
        let folders = vec![folder("f1", "A"), folder("f2", "B")];
        let playlists = vec![playlist("p1", "Un", None, false)];
        let mut tree = FolderTree::new();
        tree.rebuild(&playlists, &folders);
        tree.page = 2;
        tree.previous();
        assert_eq!(tree.selected, 0, "the first row is the top");
        tree.page_down();
        assert_eq!(tree.selected, 2);
        tree.last();
        assert_eq!(tree.selected, 2);
        tree.next();
        assert_eq!(tree.selected, 2, "the list does not wrap");
        tree.first();
        tree.page_up();
        assert_eq!(tree.selected, 0);
    }
}
