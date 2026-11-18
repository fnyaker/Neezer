//! « Sur vos appareils » : the download sets the peers on this network expose
//! (lot LN12, `docs/agents/lan-nodes.md` → *Downloads and playlists*).
//!
//! A second device that already holds a playlist is the fastest way to get it on
//! this one — and the only way at all when the server is out of reach on a trip.
//! The card lists what each connected peer exposes, with its track count, its
//! size on that peer and whether it is **complete** there, lets the person
//! multi-select, and hands the selection to `nz-lan`'s own `download_peer_sets`:
//! one `nz-offline` job per set, so the downloads screen shows the progress and
//! each failure's cause like any other job (rule 2).
//!
//! Three rules shape it.
//!
//! - **It reads memory, never the network.** `Lan::sets()` is the replica the
//!   node's control loop already replicates; the card re-reads it only when the
//!   node's version moved, and drops the track ids before it keeps a row (a
//!   4 000-track set would otherwise sit in the UI's state for nothing).
//! - **Nothing is taken from a set the peer no longer has whole.** An incomplete
//!   set is listed with « incomplet » and no checkbox: a half-downloaded
//!   playlist is worse than one offered again, and `nz-lan` re-checks anyway.
//! - **A click lands on this screen only** (D18's rule): the card lives in the
//!   downloads page and acts for itself; nothing else in the app selects a set.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement,
    Render, Styled, Window, div, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};
use nz_lan::service::PeerSet;
use nz_lan::sets::PeerSetKey;
use nz_lan::types::Relation;
use std::collections::HashSet;

use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;
use crate::wiring::lan;
use crate::wiring::offline::{Offline, WorkerState, bytes_label};

/// One row: what a peer exposes, with the track ids dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// `"<peer>:<kind>:<ident>"` — the token the node parses back into a
    /// [`PeerSetKey`]; never a path (nothing from the wire becomes one).
    pub key: String,
    pub peer: [u8; 32],
    pub peer_name: String,
    pub relation: Relation,
    pub kind: String,
    pub ident: String,
    pub title: String,
    pub tracks: u32,
    pub local: u32,
    pub complete: bool,
    pub bytes: u64,
}

impl Row {
    /// The public shape of a peer set, without its track list.
    pub fn summary(set: &PeerSet) -> Row {
        Row {
            key: key_of(set.peer, &set.set.kind, &set.set.ident),
            peer: set.peer,
            peer_name: set.peer_name.clone(),
            relation: set.relation.clone(),
            kind: set.set.kind.clone(),
            ident: set.set.ident.clone(),
            title: set.set.title.clone(),
            tracks: set.set.tracks.len().min(u32::MAX as usize) as u32,
            local: set.set.local,
            complete: set.set.complete,
            bytes: set.bytes,
        }
    }

    /// What the row says under its title: the peer, the count, the size.
    pub fn detail(&self) -> String {
        let relation = lan::relation_label(&self.relation);
        let state = if self.complete {
            "complet".to_owned()
        } else {
            format!("incomplet ({}/{})", self.local, self.tracks)
        };
        let size = if self.bytes == 0 {
            String::new()
        } else {
            format!(" · {}", bytes_label(self.bytes))
        };
        format!(
            "{} · {} · {} titre{}{size} · {state}",
            self.peer_name,
            relation,
            self.tracks,
            if self.tracks > 1 { "s" } else { "" }
        )
    }

    /// The token `nz-lan` parses back: `peer` as hex, then the two strings.
    pub fn to_key(&self) -> Option<PeerSetKey> {
        Some(PeerSetKey {
            peer: self.peer,
            kind: self.kind.clone(),
            ident: self.ident.clone(),
        })
    }
}

/// `"<peer hex>:<kind>:<ident>"`.
pub fn key_of(peer: [u8; 32], kind: &str, ident: &str) -> String {
    let hex: String = peer.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{hex}:{kind}:{ident}")
}

/// The card, as the downloads screen mounts it.
pub struct Card {
    state: Entity<AppState>,
    /// The node's version this view last read.
    painted: u64,
    rows: Vec<Row>,
    selected: HashSet<String>,
    /// The last take: how many jobs, or its cause.
    note: Option<String>,
    error: Option<String>,
}

impl Card {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let painted = lan::version();
        let rows = lan::peer_sets().iter().map(Row::summary).collect();
        // A new node (a login) drops a selection that belonged to the old one.
        cx.observe(&state, |this, _, cx| {
            if lan::version() != this.painted {
                this.repaint();
                cx.notify();
            }
        })
        .detach();
        Self {
            state,
            painted,
            rows,
            selected: HashSet::new(),
            note: None,
            error: None,
        }
    }

    fn repaint(&mut self) {
        self.painted = lan::version();
        self.rows = lan::peer_sets().iter().map(Row::summary).collect();
        // A set that left the network is no longer selected.
        let live: HashSet<String> = self.rows.iter().map(|row| row.key.clone()).collect();
        self.selected.retain(|key| live.contains(key));
    }

    /// Take every selected set: one `nz-offline` job each, with the peer's own
    /// metadata and sleeves (`nz-lan`'s `download_peer_sets`).
    fn take(&mut self, cx: &mut Context<Self>) {
        let keys: Vec<PeerSetKey> = self
            .rows
            .iter()
            .filter(|row| self.selected.contains(&row.key))
            .filter_map(Row::to_key)
            .collect();
        if keys.is_empty() {
            return;
        }
        match lan::download_sets(&keys) {
            Ok(count) => {
                self.error = None;
                self.note = Some(format!(
                    "{count} ensemble{} ajouté{} aux téléchargements.",
                    if count > 1 { "s" } else { "" },
                    if count > 1 { "s" } else { "" }
                ));
                // `nz-offline`'s worker stops entirely while the app knows the
                // SERVER is unreachable (LN11 reported it; the fix belongs to
                // `nz-offline`, which this lot does not own). A peer does not
                // need the server, so the queue would sit still — and a button
                // that seems to do nothing is a bug (rule 2): the card says so.
                if Offline::get()
                    .is_some_and(|service| service.snapshot().state == WorkerState::Offline)
                {
                    self.note = Some(
                        "Ajouté, mais en attente : le serveur est injoignable et le \
                         téléchargement ne démarre pas hors ligne."
                            .to_owned(),
                    );
                }
                self.selected.clear();
                self.state.update(cx, |state, cx| {
                    state.toasts.push(
                        ToastKind::Info,
                        format!(
                            "{count} ensemble{} en téléchargement depuis vos appareils.",
                            if count > 1 { "s" } else { "" }
                        ),
                    );
                    cx.notify();
                });
            }
            Err(error) => {
                self.note = None;
                // The cause, in French: « cet appareil ne répond plus », « cet
                // ensemble n'est plus complet sur cet appareil »…
                self.error = Some(error.cause_fr());
            }
        }
        cx.notify();
    }
}

impl Render for Card {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if lan::current().is_none() {
            // Not a node (an older server, or the feature switched off): the
            // card is not drawn at all (`Réglages` is where the person turns it
            // on).
            return div().into_any_element();
        }
        if lan::version() != self.painted {
            self.repaint();
        }
        let mut body = div()
            .id("peer-sets")
            .debug_selector(|| "peer-sets".to_owned())
            .flex()
            .flex_col()
            .w_full()
            .gap(rems(0.75))
            .p(rems(1.25))
            .border_1()
            .border_color(theme::border(cx))
            .bg(theme::chrome(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        icon(IconName::Laptop)
                            .size(rems(1.))
                            .text_color(theme::accent(cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(rems(0.875))
                            .font_weight(FontWeight::BOLD)
                            .child("Sur vos appareils"),
                    )
                    .child(
                        button("peer-sets-take", "Télécharger", ButtonVariant::Primary, cx)
                            .debug_selector(|| "peer-sets-take".to_owned())
                            .when(self.selected.is_empty(), |control| control.opacity(0.4))
                            .on_click(cx.listener(|this, _, _, cx| this.take(cx))),
                    ),
            );
        if self.rows.is_empty() {
            body = body.child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(
                        "Aucun appareil ne propose de liste pour l'instant. Un appareil \
                         apparaît ici dès qu'il a téléchargé une playlist, un album ou une \
                         émission.",
                    ),
            );
        } else {
            for row in &self.rows {
                body = body.child(self.row(row, cx));
            }
        }
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .flex()
                    .items_start()
                    .gap(rems(0.5))
                    .text_size(rems(0.75))
                    .text_color(theme::danger(cx))
                    .child(icon(IconName::CircleAlert).size(rems(0.875)))
                    .child(div().flex_1().child(error.clone())),
            );
        }
        if let Some(note) = &self.note {
            body = body.child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(note.clone()),
            );
        }
        body.into_any_element()
    }
}

impl Card {
    fn row(&self, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.selected.contains(&row.key);
        let key = row.key.clone();
        let complete = row.complete;
        div()
            .id(format!("peer-set-{}", row.key))
            .debug_selector({
                let key = row.key.clone();
                move || format!("peer-set-{key}")
            })
            .flex()
            .items_center()
            .gap(rems(0.625))
            .py(rems(0.25))
            .child(
                button(
                    format!("peer-set-toggle-{}", row.key),
                    if selected { "Choisi" } else { "Choisir" },
                    if selected {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Outline
                    },
                    cx,
                )
                .disabled(!complete)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !complete {
                        return;
                    }
                    if !this.selected.remove(&key) {
                        this.selected.insert(key.clone());
                    }
                    this.note = None;
                    this.error = None;
                    cx.notify();
                })),
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
                            .truncate()
                            .text_size(rems(0.8125))
                            .child(row.title.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(rems(0.6875))
                            .text_color(if row.complete {
                                theme::secondary(cx)
                            } else {
                                theme::warning(cx)
                            })
                            .child(row.detail()),
                    ),
            )
            .child(
                div()
                    .text_size(rems(0.625))
                    .text_color(theme::secondary(cx))
                    .child(kind_label(&row.kind)),
            )
            .into_any_element()
    }
}

/// The word a set's kind carries, so a row says what it is beyond its title.
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "playlist" => "playlist",
        "album" => "album",
        "show" => "émission",
        _ => "ensemble",
    }
}

/// The sets a peer exposes, without the ids, for a test that wants the summary.
pub fn summaries(sets: &[PeerSet]) -> Vec<Row> {
    sets.iter().map(Row::summary).collect()
}

/// The selected rows as `nz-lan` takes them (the card's own test and the take).
pub fn selected_keys(rows: &[Row], selected: &HashSet<String>) -> Vec<String> {
    rows.iter()
        .filter(|row| selected.contains(&row.key))
        .map(|row| row.key.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::PathBuf;

    use gpui_kit::{AppContext as _, IntoElement, ParentElement, Render, Styled, Window};

    use super::*;
    use crate::state::{AppConfig, Auth};

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neezer-ln12-sets-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn signed_in(cx: &mut gpui::TestAppContext, dir: &PathBuf) -> Entity<AppState> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (events, _receiver) = crate::state::event_channel();
        let config = AppConfig {
            server: Some("http://127.0.0.1:1".to_owned()),
            start_audio: false,
            start_mpris: false,
            ..AppConfig::for_test(dir)
        };
        let state = {
            let handle = runtime.handle().clone();
            cx.update(|cx| cx.new(move |_| AppState::new(config, handle, events, None)))
        };
        cx.update(|cx| {
            state.update(cx, |state, _| {
                state.auth = Auth::LoggedIn {
                    me: Box::new(nz_model::Me {
                        user: nz_model::User {
                            id: nz_model::UserId::new("u1"),
                            name: "bench".to_owned(),
                            admin: false,
                            ..nz_model::User::default()
                        },
                        remote: None,
                    }),
                };
            });
        });
        std::mem::forget(runtime);
        state
    }

    struct Host(Entity<Card>);

    impl Render for Host {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.0.clone())
        }
    }

    /// **Without a node the card draws nothing at all** (LN12's acceptance: an
    /// older server shows no badge and no section). A process that never started
    /// a `Lan` has `lan::current() == None`, and the card's root is absent.
    #[gpui::test]
    async fn without_a_node_the_card_is_not_drawn(cx: &mut gpui::TestAppContext) {
        cx.update(crate::theme::init);
        let dir = temp_dir();
        let state = signed_in(cx, &dir);
        let (host, cx) =
            cx.add_window_view(move |_window, cx| Host(cx.new(|cx| Card::new(state.clone(), cx))));
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        assert!(
            cx.debug_bounds("peer-sets").is_none(),
            "the « Sur vos appareils » card is not painted without a node"
        );
        assert!(!available_in_this_process());
        drop(host);
        std::fs::remove_dir_all(dir).ok();
    }

    fn available_in_this_process() -> bool {
        crate::wiring::lan::current().is_some()
    }

    /// What a row prints, what it hands to `nz-lan`, and the key's own spelling:
    /// the token is `"<peer hex>:<kind>:<ident>"`, and nothing from the wire is a
    /// path (it is never used as one).
    #[test]
    fn a_row_says_what_it_is_and_its_key_is_not_a_path() {
        let set = PeerSet {
            peer: [0xab; 32],
            peer_name: "Téléphone".to_owned(),
            relation: Relation::Friend {
                name: "Alice".to_owned(),
            },
            set: nz_lan::proto::SetInfo {
                kind: "playlist".to_owned(),
                ident: "../../etc/passwd".to_owned(),
                title: "Route".to_owned(),
                cover: None,
                tracks: vec!["1".to_owned(), "2".to_owned()],
                local: 1,
                complete: false,
            },
            bytes: 0,
        };
        let row = Row::summary(&set);
        assert_eq!(row.tracks, 2);
        assert!(!row.complete);
        assert!(row.detail().contains("incomplet (1/2)"), "{}", row.detail());
        assert!(row.detail().contains("Alice"));
        assert_eq!(
            row.key,
            format!("{}:playlist:../../etc/passwd", "ab".repeat(32))
        );
        // It goes back as the triple `nz-lan` parses, and never as a file name.
        let key = row.to_key().expect("a key");
        assert_eq!(key.peer, [0xab; 32]);
        assert_eq!(key.kind, "playlist");
        assert_eq!(key.ident, "../../etc/passwd");

        // The selection is the rows' keys, in the order they are drawn.
        let mut selected = HashSet::new();
        selected.insert(row.key.clone());
        assert_eq!(
            selected_keys(std::slice::from_ref(&row), &selected),
            vec![row.key.clone()]
        );
        // A set's kind, in the person's words.
        assert_eq!(kind_label("playlist"), "playlist");
        assert_eq!(kind_label("show"), "émission");
        assert_eq!(kind_label("album"), "album");
    }
}
