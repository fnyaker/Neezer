//! Réglages → Appareils à proximité (lot LN12).
//!
//! The desktop's half of `docs/agents/lan-nodes.md` (*Screens and Réglages*): the
//! two switches, « Qualités tolérées », the devices seen now, this account's
//! enrolled devices, the trusted friends and what each of them may take, the UDP
//! port with the one `ufw` line, and the key's state — in French, with the app's
//! glyphs and tokens.
//!
//! Four rules shape this page.
//!
//! - **Nothing here polls the network or SQLite.** The node's state is
//!   [`crate::wiring::lan::snapshot`] (one short lock over counted replicas), and
//!   the three server-owned lists are asked ONCE per open, on `nz-net`, through
//!   this screen's own channel — a worker never touches GPUI state (D7's rule).
//! - **A control acts at once** (rule 4): a switch writes the node's settings in
//!   the click's own frame (the driver persists them a moment later), and the
//!   checkbox that would empty « Qualités tolérées » is refused in the same frame
//!   with its reason — the note says at least one.
//! - **A failure is a cause, never a count** (rule 2): an enrolment that failed
//!   prints its French sentence and offers « Réessayer », and the firewall line
//!   names the port really bound (and says when it is not 45910).
//! - **The page fits a 660 px column** (D14a's laptop-with-the-panel case): one
//!   centred column of cards at most 56 rem, and every long list is paged.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::base::CheckboxState;
use gpui_kit::base::input::InputState;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClipboardItem, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement, Render, ScrollHandle, StatefulInteractiveElement as _, Styled,
    Subscription, Window, div, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, checkbox, icon, icon_button, input, switch};
use nz_api::Api;
use nz_model::{LanDevice, LanFriend, LanFriends, LanScope, PlaylistList, UserId};

use crate::nav::SettingsSection;
use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;
use crate::wiring::events::{self, Sender};
use crate::wiring::lan::{self, LanSettings, LanSnapshot, LanState, PeerView, TransferView};

use super::{Load, card, card_title, eid, failed, hint, loading, value_line};

/// How often the page re-reads the node's own snapshot. Peers connect, a track
/// is exchanged and a transfer ends at their own pace; the node publishes a
/// version, so this is a compare and (only when it moved) one clone.
const POLL: Duration = Duration::from_millis(500);

/// The note's « Aucun appareil trouvé » sentence appears after ten seconds with
/// no peer while another enrolled device exists (rule 3: the person is told why,
/// and the hotel's isolated Wi-Fi is the common cause).
const NOTHING_FOUND_AFTER: Duration = Duration::from_secs(10);

/// Rows per page: a page of controls that are not painted costs nothing, and a
/// Réglages page is paged rather than windowed (D13's rule).
const PAGE: usize = 8;

/// The « Qualités tolérées » checkboxes, with the word the person reads.
const QUALITIES: [(nz_api::Quality, &str); 4] = [
    (nz_api::Quality::Flac, "FLAC"),
    (nz_api::Quality::Opus320, "Opus 320"),
    (nz_api::Quality::Opus128, "Opus 128"),
    (nz_api::Quality::Opus64, "Opus 64"),
];

pub struct Screen {
    state: Entity<AppState>,
    /// The device's settings — the node's when it runs, the store's otherwise.
    settings: LanSettings,
    /// The node's published state, re-read only when its version moved.
    lan_version: u64,
    snapshot: LanSnapshot,
    /// The three lists the server owns, asked once per open (`User` lane).
    devices: Load<Vec<LanDevice>>,
    friends: Load<LanFriends>,
    playlists: Load<PlaylistList>,
    /// The page of each paged list.
    device_page: usize,
    friend_page: usize,
    /// A destructive control armed by its first press (« Révoquer », « Retirer »).
    armed: Option<String>,
    /// The last mutation's cause, in French: never a silent failure (rule 2).
    error: Option<String>,
    /// The last thing that worked, for the line under a card.
    note: Option<String>,
    /// The firewall line was copied to the clipboard.
    copied: bool,
    /// When this page was opened, for the « rien trouvé » sentence.
    opened: Instant,
    /// The sentence's own latch, so the tick repaints once and not every 500 ms.
    waited: bool,
    /// Per-row drafts, created on first paint (a GPUI `InputState` wants a
    /// window) and kept per window so a draft survives a repaint.
    friend_draft: Option<Entity<InputState>>,
    port_draft: Option<Entity<InputState>>,
    rename: Option<(String, Entity<InputState>)>,
    reply: Sender<Reply>,
    scroll: ScrollHandle,
    _state: Subscription,
}

/// What this screen's background requests answer.
enum Reply {
    Devices(Result<Vec<LanDevice>, String>),
    Friends(Result<LanFriends, String>),
    Playlists(Result<PlaylistList, String>),
    /// A mutation landed (or failed, with its cause): the lists are re-read.
    Landed(Result<(), String>),
}

impl Screen {
    /// The section's own view, built the way every Réglages section is
    /// (`settings/mod.rs`): `Screen::new(section, state, cx)`.
    pub fn new(section: SettingsSection, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        debug_assert_eq!(
            section,
            SettingsSection::Lan,
            "lan::Screen est la section Appareils à proximité"
        );
        let cache = state.read(cx).cache.clone();
        let settings = lan::settings(cache.as_ref());
        // No node yet (or no node at all) is « pas encore enregistré », never
        // « désactivé »: the second sentence would be a wrong verdict about a
        // feature the person did not switch off.
        let snapshot = lan::snapshot().unwrap_or_else(|| LanSnapshot {
            state: LanState::NotEnrolled,
            ..LanSnapshot::default()
        });
        let lan_version = lan::version();
        let (reply, receiver) = events::channel::<Reply>();

        let _state = cx.observe(&state, |this, _, cx| {
            // A login, a logout or a node that started change what this page can
            // show; the snapshot below catches up on its own version.
            this.reload(cx);
            cx.notify();
        });

        // The node's own changes: one timer and a version compare (D13's shape,
        // for the same reason — `nz-lan` publishes a version and nothing else).
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let alive = this.update(cx, |this, cx| {
                    let version = lan::version();
                    if version != this.lan_version {
                        this.lan_version = version;
                        this.snapshot = lan::snapshot().unwrap_or_else(|| LanSnapshot {
                            state: LanState::NotEnrolled,
                            ..LanSnapshot::default()
                        });
                        let cache = this.state.read(cx).cache.clone();
                        this.settings = lan::settings(cache.as_ref());
                        cx.notify();
                    } else if !this.waited && this.opened.elapsed() >= NOTHING_FOUND_AFTER {
                        // The ten-second sentence: one repaint, then never again
                        // for this visit.
                        this.waited = true;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();

        // One foreground drain for this screen's answers.
        cx.spawn(async move |this, cx| {
            while let Some(reply) = receiver.next().await {
                if this.update(cx, |this, cx| this.apply(reply, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            state,
            settings,
            lan_version,
            snapshot,
            devices: Load::Idle,
            friends: Load::Idle,
            playlists: Load::Idle,
            device_page: 0,
            friend_page: 0,
            armed: None,
            error: None,
            note: None,
            copied: false,
            opened: Instant::now(),
            waited: false,
            friend_draft: None,
            port_draft: None,
            rename: None,
            reply,
            scroll: ScrollHandle::new(),
            _state,
        }
    }

    /// A page drawn from a state a test hands it (the layout walk and the
    /// screenshots): the node itself is not needed to lay a card out.
    #[cfg(test)]
    pub fn with_snapshot(
        state: Entity<AppState>,
        snapshot: LanSnapshot,
        devices: Vec<LanDevice>,
        friends: LanFriends,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut screen = Self::new(SettingsSection::Lan, state, cx);
        screen.lan_version = lan::version();
        screen.snapshot = snapshot;
        screen.devices = Load::Ready(devices);
        screen.friends = Load::Ready(friends);
        screen.waited = true;
        screen
    }

    // -- the server's own lists ---------------------------------------------

    /// Ask the three lists once, when this page can (`User` lane: it is what the
    /// person opened).
    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        if self.devices.is_idle() {
            self.devices = Load::Loading;
            let reply = self.reply.clone();
            let api = Arc::clone(&api);
            runtime.spawn(async move {
                reply.send(Reply::Devices(
                    api.lan_devices().await.map_err(|error| error.cause_fr()),
                ));
            });
        }
        if self.friends.is_idle() {
            self.friends = Load::Loading;
            let reply = self.reply.clone();
            let api = Arc::clone(&api);
            runtime.spawn(async move {
                reply.send(Reply::Friends(
                    api.lan_friends().await.map_err(|error| error.cause_fr()),
                ));
            });
        }
        if self.playlists.is_idle() {
            self.playlists = Load::Loading;
            let reply = self.reply.clone();
            runtime.spawn(async move {
                reply.send(Reply::Playlists(
                    api.my_playlists().await.map_err(|error| error.cause_fr()),
                ));
            });
        }
    }

    fn api(&self, cx: &App) -> Option<Arc<Api>> {
        self.state.read(cx).api.clone()
    }

    fn apply(&mut self, reply: Reply, cx: &mut Context<Self>) {
        match reply {
            Reply::Devices(result) => self.devices = into_load(result),
            Reply::Friends(result) => self.friends = into_load(result),
            Reply::Playlists(result) => self.playlists = into_load(result),
            Reply::Landed(result) => match result {
                Ok(()) => {
                    self.error = None;
                    // The lists are the server's: read them again (one round trip
                    // each, and only after a mutation).
                    self.devices = Load::Idle;
                    self.friends = Load::Idle;
                    self.reload(cx);
                }
                Err(cause) => self.error = Some(cause),
            },
        }
        cx.notify();
    }

    /// Run one mutation on `nz-net` and land its outcome on this screen.
    fn mutate(
        &self,
        cx: &Context<Self>,
        answer: impl Future<Output = Result<(), String>> + Send + 'static,
    ) {
        let reply = self.reply.clone();
        self.state.read(cx).runtime.spawn(async move {
            reply.send(Reply::Landed(answer.await));
        });
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        self.devices = Load::Idle;
        self.friends = Load::Idle;
        self.playlists = Load::Idle;
        self.error = None;
        self.reload(cx);
        cx.notify();
    }

    // -- the settings ------------------------------------------------------

    /// Write the settings the person just changed: `nz-cache` keeps them for the
    /// next launch, and a running node applies them in its own driver (one
    /// function, `wiring::lan::save_settings`, so the two cannot disagree).
    fn save(&mut self, settings: LanSettings, cx: &mut Context<Self>) {
        self.settings = settings.clone();
        let cache = self.state.read(cx).cache.clone();
        lan::save_settings(cache.as_ref(), settings);
        cx.notify();
    }

    fn toggle_quality(&mut self, quality: nz_api::Quality, wanted: bool, cx: &mut Context<Self>) {
        if !wanted && self.settings.tolerated.0.len() == 1 {
            // The note's « at least one »: an empty set means "never take
            // anything from a peer", which is not what this switch is about.
            self.error = Some(
                "Gardez au moins une qualité : sans cela, aucun appareil proche ne serait jamais \
                 utilisé."
                    .to_owned(),
            );
            cx.notify();
            return;
        }
        let tolerated = lan::toggle_tolerated(&self.settings.tolerated, quality, wanted);
        self.error = None;
        self.save(
            LanSettings {
                tolerated,
                ..self.settings.clone()
            },
            cx,
        );
    }

    // -- the server mutations ---------------------------------------------

    fn revoke(&mut self, id: String, cx: &mut Context<Self>) {
        if self.armed.as_deref() != Some(id.as_str()) {
            self.armed = Some(id);
            cx.notify();
            return;
        }
        self.armed = None;
        let Some(api) = self.api(cx) else {
            return;
        };
        self.mutate(cx, async move {
            api.lan_revoke(&id).await.map_err(|error| error.cause_fr())
        });
    }

    fn rename_device(&mut self, id: String, name: String, cx: &mut Context<Self>) {
        let name = name.trim().to_owned();
        if name.is_empty() || name.chars().count() > 64 {
            self.error = Some("Un nom d'appareil fait 1 à 64 caractères.".to_owned());
            cx.notify();
            return;
        }
        self.rename = None;
        let Some(api) = self.api(cx) else {
            return;
        };
        self.mutate(cx, async move {
            api.lan_rename(&id, &name)
                .await
                .map(|_| ())
                .map_err(|error| error.cause_fr())
        });
    }

    fn add_friend(&mut self, name: String, cx: &mut Context<Self>) {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return;
        }
        let Some(api) = self.api(cx) else {
            return;
        };
        self.mutate(cx, async move {
            api.lan_add_friend(&name)
                .await
                .map(|_| ())
                .map_err(|error| error.cause_fr())
        });
    }

    fn remove_friend(&mut self, user_id: UserId, cx: &mut Context<Self>) {
        let key = format!("friend:{user_id}");
        if self.armed.as_deref() != Some(key.as_str()) {
            self.armed = Some(key);
            cx.notify();
            return;
        }
        self.armed = None;
        let Some(api) = self.api(cx) else {
            return;
        };
        self.mutate(cx, async move {
            api.lan_remove_friend(&user_id)
                .await
                .map_err(|error| error.cause_fr())
        });
    }

    /// What a friend may take: everything, or the playlists the person ticked.
    /// The server re-checks that the caller may read each one (`playlist_access`).
    fn set_scope(
        &mut self,
        friend: &LanFriend,
        all: bool,
        playlists: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(api) = self.api(cx) else {
            return;
        };
        let owner = friend.user_id.clone();
        let scope = LanScope { all, playlists };
        self.mutate(cx, async move {
            api.lan_set_friend(&owner, &scope)
                .await
                .map(|_| ())
                .map_err(|error| error.cause_fr())
        });
    }

    /// The port: parsed, bounded, then applied like any other setting.
    fn apply_port(&mut self, raw: String, window: &mut Window, cx: &mut Context<Self>) {
        match raw.trim().parse::<u16>() {
            Ok(port) if port == 0 || port >= 1_024 => {
                self.error = None;
                self.note = Some(if port == 0 {
                    "Port : le système en choisit un.".to_owned()
                } else {
                    format!("Port {port} : le nœud se réannonce dans un instant.")
                });
                self.save(
                    LanSettings {
                        port,
                        ..self.settings.clone()
                    },
                    cx,
                );
                // The field shows what was kept, not what was typed.
                if let Some(field) = self.port_draft.clone() {
                    let kept = self.settings.port.to_string();
                    field.update(cx, |input, cx| input.set_value(kept, window, cx));
                }
            }
            Ok(_) => {
                self.error = Some(
                    "Choisissez un port entre 1024 et 65535, ou 0 pour laisser le système choisir."
                        .to_owned(),
                );
                cx.notify();
            }
            Err(_) => {
                self.error = Some("Ce n'est pas un numéro de port.".to_owned());
                cx.notify();
            }
        }
    }

    // -- painting ----------------------------------------------------------

    /// The sentence an empty device list carries, or `None` while there is
    /// nothing to say (rule 3: no endless spinner, and no premature verdict).
    fn nothing_found(&self) -> String {
        if self.snapshot.state == LanState::Off {
            return "L'échange est désactivé sur cet appareil : activez-le pour voir vos autres \
                    appareils."
                .to_owned();
        }
        let enrolled = self
            .devices
            .ready()
            .map(|devices| {
                devices
                    .iter()
                    .filter(|device| device.revoked.is_none())
                    .count()
            })
            .unwrap_or(0);
        if enrolled < 2 {
            return "Aucun autre appareil pour l'instant : connectez-en un second avec le même \
                    compte."
                .to_owned();
        }
        if !self.waited {
            return "Recherche des appareils sur ce réseau…".to_owned();
        }
        "Aucun appareil trouvé. Sur un Wi-Fi d'hôtel, les appareils sont souvent isolés : \
         partagez la connexion du téléphone."
            .to_owned()
    }

    fn state_label(&self) -> (&'static str, IconName) {
        match self.snapshot.state {
            LanState::Off => ("Désactivé sur cet appareil", IconName::CirclePause),
            LanState::Unsupported => (
                "Ce serveur ne connaît pas les appareils proches",
                IconName::CircleAlert,
            ),
            LanState::NotEnrolled => (
                "Pas encore enregistré auprès du serveur",
                IconName::CloudOff,
            ),
            LanState::Searching => ("Enregistré — recherche des appareils", IconName::Radar),
            LanState::Connected(0) => ("Enregistré — aucun appareil connecté", IconName::Radar),
            LanState::Connected(1) => ("1 appareil connecté", IconName::CircleCheck),
            LanState::Connected(_) => ("Appareils connectés", IconName::CircleCheck),
        }
    }

    // -- the cards ---------------------------------------------------------

    fn this_device_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let (label, glyph) = self.state_label();
        let snapshot = &self.snapshot;
        let mut body = card(eid("lan-this"), cx)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.625))
                    .child(
                        icon(glyph)
                            .size(rems(1.125))
                            .text_color(match snapshot.state {
                                LanState::Connected(count) if count > 0 => theme::accent(cx),
                                LanState::Off | LanState::Unsupported => theme::secondary(cx),
                                _ => theme::secondary(cx),
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(rems(0.875))
                            .font_weight(FontWeight::BOLD)
                            .child(label.to_owned()),
                    )
                    .when(
                        matches!(snapshot.state, LanState::Connected(count) if count > 0),
                        |row| {
                            row.child(
                                div()
                                    .text_size(rems(0.75))
                                    .text_color(theme::secondary(cx))
                                    .child(format!("{} appareil(s)", snapshot.peers.len())),
                            )
                        },
                    ),
            )
            .child(value_line("Cet appareil", lan::device_name(), cx))
            .child(value_line("Port d'écoute", port_label(snapshot), cx));
        if snapshot.port_fallback {
            body = body.child(hint(
                format!(
                    "Le port 45910 était pris : le nœud écoute sur {}. La ligne de pare-feu \
                     ci-dessous suit ce port.",
                    snapshot.port
                ),
                cx,
            ));
        }
        // The one `ufw` line, beside the port it opens: it is what the person runs
        // once on this machine (a Linux build ships a drop-by-default input
        // policy), and it must be readable without scrolling.
        let firewall = lan::firewall_line(snapshot);
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(theme::mono(cx))
                        .text_size(rems(0.75))
                        .debug_selector(|| "lan-ufw".to_owned())
                        .child(firewall.clone()),
                )
                .child(
                    icon_button(
                        "lan-copy-ufw",
                        IconName::Copy,
                        "Copier la ligne de pare-feu",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .debug_selector(|| "lan-copy-ufw".to_owned())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(firewall.clone()));
                        this.copied = true;
                        this.state.update(cx, |state, cx| {
                            state
                                .toasts
                                .push(ToastKind::Info, "Ligne de pare-feu copiée.".to_owned());
                            cx.notify();
                        });
                        cx.notify();
                    })),
                ),
        );
        // The key: its validity, or the cause of a renewal that failed (rule 2).
        let key = lan::key_line(snapshot);
        let failed = snapshot.key_error.is_some() || snapshot.key_until.is_none();
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    icon(if failed {
                        IconName::CircleAlert
                    } else {
                        IconName::BadgeCheck
                    })
                    .size(rems(1.))
                    .text_color(if failed {
                        theme::warning(cx)
                    } else {
                        theme::secondary(cx)
                    }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(rems(0.75))
                        .text_color(if failed {
                            theme::warning(cx)
                        } else {
                            theme::secondary(cx)
                        })
                        .child(key),
                )
                .child(
                    button("lan-renew", "Renouveler", ButtonVariant::Secondary, cx)
                        .debug_selector(|| "lan-renew".to_owned())
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(lan) = lan::current() {
                                lan.enroll_now();
                                this.note = Some(
                                    "Nouvelle clé demandée : le serveur répond dans un instant."
                                        .to_owned(),
                                );
                                this.error = None;
                            } else {
                                this.error =
                                    Some("Le nœud n'est pas démarré sur cet appareil.".to_owned());
                            }
                            cx.notify();
                        })),
                ),
        );
        let wire = lan::current().map(|lan| lan.wire());
        if let Some(wire) = wire {
            body = body.child(value_line(
                "Échangé sur ce réseau",
                format!(
                    "{} paquet(s) envoyés, {} reçus",
                    wire.packets_out, wire.packets_in
                ),
                cx,
            ));
        }
        body.into_any_element()
    }

    fn exchange_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let enabled = self.settings.enabled;
        let serve = self.settings.serve;
        card(eid("lan-exchange"), cx)
            .child(card_title("Échange", cx))
            .child(
                div()
                    .id("lan-enabled-row")
                    .debug_selector(|| "lan-enabled".to_owned())
                    .child(
                        switch(
                            "lan-enabled",
                            "Échanger avec mes appareils à proximité",
                            enabled,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                        let enabled = !this.settings.enabled;
                        this.error = None;
                        this.note = Some(if enabled {
                            "Recherche des appareils sur ce réseau.".to_owned()
                        } else {
                            "Échange arrêté sur cet appareil : plus aucune annonce ni connexion."
                                .to_owned()
                        });
                            this.save(
                                LanSettings {
                                    enabled,
                                    ..this.settings.clone()
                                },
                                cx,
                            );
                        })),
                    ),
            )
            .child(
                div()
                    .id("lan-serve-row")
                    .debug_selector(|| "lan-serve".to_owned())
                    .child(
                        switch("lan-serve", "Partager mes sons avec mes appareils", serve, cx)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let serve = !this.settings.serve;
                                this.error = None;
                                this.save(
                                    LanSettings {
                                        serve,
                                        ..this.settings.clone()
                                    },
                                    cx,
                                );
                            })),
                    ),
            )
            .child(hint(
                "Rien n'est partagé avec un inconnu : seuls vos appareils et les amis de \
                 confiance ci-dessous peuvent se connecter.",
                cx,
            ))
            .into_any_element()
    }

    fn qualities_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let tolerated = self.settings.tolerated.clone();
        let mut body = card(eid("lan-qualities"), cx).child(card_title("Qualités tolérées", cx));
        let mut row = div().flex().flex_wrap().items_center().gap(rems(1.));
        for (quality, label) in QUALITIES {
            let checked = tolerated.contains(quality);
            let slug = quality_slug(quality);
            row = row.child(
                div()
                    .id(format!("lan-quality-{slug}-row"))
                    .debug_selector(move || format!("lan-quality-{slug}"))
                    .child(
                        checkbox(
                            eid(format!("lan-quality-{slug}")),
                            label,
                            if checked {
                                CheckboxState::Checked
                            } else {
                                CheckboxState::Unchecked
                            },
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_quality(quality, !checked, cx);
                        })),
                    ),
            );
        }
        body = body.child(row).child(hint(
            "Utilisées quand un appareil proche a le titre et que le serveur est joignable. Sans \
             serveur, la qualité de l'appareil est toujours acceptée.",
            cx,
        ));
        body.into_any_element()
    }

    fn devices_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = &self.snapshot;
        let mut body = card(eid("lan-devices"), cx).child(card_title("Appareils connectés", cx));
        if snapshot.peers.is_empty() {
            body = body.child(hint(self.nothing_found(), cx));
        } else {
            for view in &snapshot.peers {
                body = body.child(peer_row(view, cx));
            }
        }
        if !snapshot.transfers.is_empty() {
            body = body.child(
                div()
                    .text_size(rems(0.6875))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::secondary(cx))
                    .child("En cours d'échange"),
            );
            for transfer in snapshot.transfers.iter().take(PAGE) {
                body = body.child(transfer_row(transfer, cx));
            }
        }
        if !snapshot.failures.is_empty() {
            for (what, why) in snapshot.failures.iter().take(4) {
                body = body.child(failed(
                    eid(format!("lan-failure-{what}")),
                    format!("{what} — {why}"),
                    cx,
                ));
            }
        }
        body.into_any_element()
    }

    fn feedback_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut body = card(eid("lan-feedback"), cx);
        if let Some(error) = &self.error {
            body = body.child(failed(eid("lan-error"), error.clone(), cx));
        }
        if let Some(note) = &self.note {
            body = body.child(hint(note.clone(), cx));
        }
        body.into_any_element()
    }

    fn mine_card(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut body = card(eid("lan-mine"), cx).child(card_title("Mes appareils", cx));
        match &self.devices {
            Load::Idle | Load::Loading => body = body.child(loading(cx)),
            Load::Failed(cause) => {
                body = body
                    .child(failed(eid("lan-devices-failed"), cause.clone(), cx))
                    .child(
                        button("lan-devices-retry", "Réessayer", ButtonVariant::Outline, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
                    )
            }
            Load::Ready(devices) => {
                if devices.is_empty() {
                    body = body.child(hint("Aucun appareil enregistré pour l'instant.", cx));
                }
                let pages = devices.len().div_ceil(PAGE).max(1);
                let page = self.device_page.min(pages - 1);
                for device in devices.iter().skip(page * PAGE).take(PAGE) {
                    body = body.child(self.device_row(device, window, cx));
                }
                if pages > 1 {
                    body = body.child(pager("lan-devices", page, pages, cx, |this, page, _| {
                        this.device_page = page
                    }));
                }
            }
        }
        body.into_any_element()
    }

    fn device_row(
        &self,
        device: &LanDevice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = device.id.clone();
        let armed = self.armed.as_deref() == Some(id.as_str());
        let renaming = self
            .rename
            .as_ref()
            .is_some_and(|(renaming, _)| renaming == &id);
        let mut row = div()
            .flex()
            .flex_col()
            .gap(rems(0.375))
            .py(rems(0.375))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(icon(platform_glyph(&device.platform)).size(rems(1.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(rems(0.8125))
                            .child(device.name.clone()),
                    )
                    .when(device.current, |line| {
                        line.child(
                            div()
                                .px(rems(0.375))
                                .py(rems(0.0625))
                                .border_1()
                                .border_color(theme::border(cx))
                                .text_size(rems(0.625))
                                .text_color(theme::secondary(cx))
                                .child("cet appareil"),
                        )
                    })
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .text_color(theme::secondary(cx))
                            .child(last_seen(device)),
                    ),
            );
        if renaming {
            let field = self
                .rename
                .as_ref()
                .map(|(_, field)| field.clone())
                .expect("the field");
            row = row.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(div().flex_1().min_w_0().child(input(
                        eid(format!("lan-rename-{id}")),
                        &field,
                        window,
                        cx,
                    )))
                    .child(
                        button("lan-rename-save", "Enregistrer", ButtonVariant::Primary, cx)
                            .on_click(cx.listener(|this, _, window, cx| {
                                let Some((id, field)) = this.rename.clone() else {
                                    return;
                                };
                                let name = field.read(cx).value().to_string();
                                this.rename_device(id, name, cx);
                                let _ = window;
                            })),
                    )
                    .child(
                        button("lan-rename-cancel", "Annuler", ButtonVariant::Secondary, cx)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.rename = None;
                                cx.notify();
                            })),
                    ),
            );
        } else {
            row = row.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(rems(0.375))
                    .child(
                        button("lan-rename", "Renommer", ButtonVariant::Secondary, cx).on_click(
                            cx.listener({
                                let id = id.clone();
                                let name = device.name.clone();
                                move |this, _, window, cx| {
                                    let field = window.use_keyed_state(
                                        eid(format!("lan-rename-field-{id}")),
                                        cx,
                                        |window, cx| {
                                            InputState::new(window, cx)
                                                .placeholder("Nom de l'appareil")
                                        },
                                    );
                                    let name = name.clone();
                                    field.update(cx, |input, cx| {
                                        input.set_value(name.clone(), window, cx)
                                    });
                                    this.rename = Some((id.clone(), field));
                                    cx.notify();
                                }
                            }),
                        ),
                    )
                    .child(
                        button(
                            "lan-revoke",
                            if armed { "Confirmer" } else { "Révoquer" },
                            if armed {
                                ButtonVariant::Danger
                            } else {
                                ButtonVariant::Secondary
                            },
                            cx,
                        )
                        .debug_selector(|| "lan-revoke".to_owned())
                        .on_click(cx.listener({
                            let id = id.clone();
                            move |this, _, _, cx| this.revoke(id.clone(), cx)
                        })),
                    )
                    .when(armed, |line| {
                        line.child(hint(
                            "Révoquer déconnecte cet appareil du réseau local ; il devra se \
                             réenregistrer pour revenir.",
                            cx,
                        ))
                    }),
            );
        }
        row.into_any_element()
    }

    fn friends_card(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut body = card(eid("lan-friends"), cx).child(card_title("Amis de confiance", cx));
        match &self.friends {
            Load::Idle | Load::Loading => body = body.child(loading(cx)),
            Load::Failed(cause) => {
                body = body
                    .child(failed(eid("lan-friends-failed"), cause.clone(), cx))
                    .child(
                        button("lan-friends-retry", "Réessayer", ButtonVariant::Outline, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
                    )
            }
            Load::Ready(friends) => {
                if friends.shares_to.is_empty() && friends.trusted_by.is_empty() {
                    body = body.child(hint(
                        "Personne pour l'instant : ajoutez le compte d'une personne de \
                         confiance, puis choisissez ce qu'elle peut prendre.",
                        cx,
                    ));
                }
                let pages = friends.shares_to.len().div_ceil(PAGE).max(1);
                let page = self.friend_page.min(pages - 1);
                for friend in friends.shares_to.iter().skip(page * PAGE).take(PAGE) {
                    body = body.child(self.friend_row(friend, cx));
                }
                if pages > 1 {
                    body = body.child(pager("lan-friends", page, pages, cx, |this, page, _| {
                        this.friend_page = page
                    }));
                }
                if !friends.trusted_by.is_empty() {
                    body = body.child(hint("Partagent avec vous", cx));
                    for friend in &friends.trusted_by {
                        body = body.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(rems(0.5))
                                .child(icon(IconName::User).size(rems(1.)))
                                .child(div().text_size(rems(0.8125)).child(friend.name.clone())),
                        );
                    }
                }
                // Add by account name: a text field and one button.
                let field = self.friend_draft.clone();
                if let Some(field) = field {
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.5))
                            .child(div().flex_1().min_w_0().child(input(
                                "lan-friend-name",
                                &field,
                                window,
                                cx,
                            )))
                            .child(
                                button("lan-add-friend", "Ajouter", ButtonVariant::Primary, cx)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let Some(field) = this.friend_draft.clone() else {
                                            return;
                                        };
                                        let name = field.read(cx).value().to_string();
                                        this.add_friend(name, cx);
                                    })),
                            ),
                    );
                }
            }
        }
        body.into_any_element()
    }

    fn friend_row(&self, friend: &LanFriend, cx: &mut Context<Self>) -> AnyElement {
        let mut row = card(eid(format!("lan-friend-{}", friend.user_id)), cx)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(icon(IconName::User).size(rems(1.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(rems(0.8125))
                            .child(friend.name.clone()),
                    ),
            )
            .child(
                switch(
                    eid(format!("lan-friend-all-{}", friend.user_id)),
                    "Tout partager",
                    friend.all,
                    cx,
                )
                .on_click(cx.listener({
                    let friend = friend.clone();
                    move |this, _, _, cx| {
                        let all = !friend.all;
                        let playlists = if all {
                            Vec::new()
                        } else {
                            friend
                                .playlists
                                .iter()
                                .map(|playlist| playlist.id.clone())
                                .collect()
                        };
                        this.set_scope(&friend, all, playlists, cx);
                    }
                })),
            );
        if !friend.all {
            if let Some(list) = self.playlists.ready() {
                let chosen: Vec<String> = friend
                    .playlists
                    .iter()
                    .map(|playlist| playlist.id.clone())
                    .collect();
                let mut picker = div().flex().flex_col().gap(rems(0.25));
                for playlist in list.playlists.iter().take(40) {
                    let id = playlist.id.as_str().to_owned();
                    let checked = chosen.contains(&id);
                    picker = picker.child(
                        checkbox(
                            eid(format!("lan-share-{}-{id}", friend.user_id)),
                            playlist.title.clone(),
                            if checked {
                                CheckboxState::Checked
                            } else {
                                CheckboxState::Unchecked
                            },
                            cx,
                        )
                        .on_click(cx.listener({
                            let friend = friend.clone();
                            let id = playlist.id.as_str().to_owned();
                            let now = chosen.clone();
                            move |this, _, _, cx| {
                                let mut next = now.clone();
                                if next.contains(&id) {
                                    next.retain(|kept| kept != &id);
                                } else {
                                    next.push(id.clone());
                                }
                                this.set_scope(&friend, false, next, cx);
                            }
                        })),
                    );
                }
                row = row.child(picker);
            } else {
                row = row.child(loading(cx));
            }
        }
        let armed = self
            .armed
            .as_deref()
            .is_some_and(|key| key == format!("friend:{}", friend.user_id));
        row.child(
            button(
                "lan-remove-friend",
                if armed { "Confirmer" } else { "Retirer" },
                if armed {
                    ButtonVariant::Danger
                } else {
                    ButtonVariant::Secondary
                },
                cx,
            )
            .on_click(cx.listener({
                let friend = friend.clone();
                move |this, _, _, cx| this.remove_friend(friend.user_id.clone(), cx)
            })),
        )
        .into_any_element()
    }

    fn advanced_card(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        port: &Entity<InputState>,
    ) -> AnyElement {
        let snapshot = &self.snapshot;
        card(eid("lan-advanced"), cx)
            .child(card_title("Avancé", cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_end()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(rems(0.25))
                            .w(rems(9.))
                            .child(hint("Port UDP", cx))
                            .child(input("lan-port", port, window, cx)),
                    )
                    .child(
                        button("lan-port-apply", "Appliquer", ButtonVariant::Outline, cx).on_click(
                            cx.listener(|this, _, window, cx| {
                                let Some(field) = this.port_draft.clone() else {
                                    return;
                                };
                                let raw = field.read(cx).value().to_string();
                                this.apply_port(raw, window, cx);
                            }),
                        ),
                    ),
            )
            .child(hint(
                "Sur cet ordinateur, ufw refuse les connexions entrantes : la ligne de la \
                 première carte ouvre le port d'écoute, à exécuter une fois dans un terminal.",
                cx,
            ))
            .child(value_line("Clé LAN", lan::key_line(snapshot), cx))
            .into_any_element()
    }
}

/// The token a quality's checkbox is found by, in the tests and in `debug_bounds`.
pub fn quality_slug(quality: nz_api::Quality) -> &'static str {
    match quality {
        nz_api::Quality::Flac => "flac",
        nz_api::Quality::Opus320 => "opus320",
        nz_api::Quality::Opus128 => "opus128",
        nz_api::Quality::Opus64 => "opus64",
        nz_api::Quality::Opus256 => "opus256",
        nz_api::Quality::Opus192 => "opus192",
    }
}

fn into_load<T>(result: Result<T, String>) -> Load<T> {
    match result {
        Ok(value) => Load::Ready(value),
        Err(cause) => Load::Failed(cause.into()),
    }
}

/// The port as the state card prints it.
fn port_label(snapshot: &LanSnapshot) -> String {
    if snapshot.port == 0 {
        "choisi par le système".to_owned()
    } else {
        format!("{}/udp", snapshot.port)
    }
}

/// The glyph of a peer's platform (`Hello`'s `platform`).
fn platform_glyph(platform: &str) -> IconName {
    match platform {
        "android" => IconName::Smartphone,
        "ios" => IconName::Smartphone,
        _ => IconName::Monitor,
    }
}

/// « Bureau · vu le 18 nov. 2026 », or the platform alone when the server never
/// saw the device. The server sends an ISO instant (`2026-11-18T18:40:00Z`); a
/// person reads a day, so the day is what this prints.
fn last_seen(device: &LanDevice) -> String {
    const MONTHS: [&str; 12] = [
        "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
        "déc.",
    ];
    let platform = match device.platform.as_str() {
        "android" => "Android",
        "ios" => "iPhone",
        _ => "Bureau",
    };
    let Some(seen) = device.last_seen.as_deref().filter(|seen| !seen.is_empty()) else {
        return platform.to_owned();
    };
    // `YYYY-MM-DD…`, and nothing else: a shape this refuses is shown as the server
    // sent it, never sliced (an ISO instant is not a day a person reads).
    let date = seen
        .get(0..4)
        .zip(seen.get(5..7))
        .zip(seen.get(8..10))
        .filter(|_| seen.as_bytes().get(4) == Some(&b'-') && seen.as_bytes().get(7) == Some(&b'-'))
        .and_then(|((year, month), day)| {
            let month = month
                .parse::<usize>()
                .ok()
                .filter(|month| (1..=12).contains(month))?;
            year.parse::<u16>().ok()?;
            day.parse::<u8>().ok()?;
            Some((
                year.to_owned(),
                month,
                day.trim_start_matches('0').to_owned(),
            ))
        });
    match date {
        Some((year, month, day)) => {
            format!("{platform} · vu le {day} {} {year}", MONTHS[month - 1])
        }
        None => format!("{platform} · vu {seen}"),
    }
}

fn peer_row(view: &PeerView, cx: &mut App) -> AnyElement {
    let relation = lan::relation_label(&view.relation);
    div()
        .flex()
        .items_center()
        .gap(rems(0.5))
        .py(rems(0.25))
        .child(icon(platform_glyph(&view.platform)).size(rems(1.)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(rems(0.8125))
                .child(view.name.clone()),
        )
        .child(
            div()
                .text_size(rems(0.6875))
                .text_color(theme::secondary(cx))
                .child(format!(
                    "{} · connecté · {} ms · {} titres",
                    relation, view.rtt_ms, view.tracks
                )),
        )
        .into_any_element()
}

fn transfer_row(view: &TransferView, cx: &mut App) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(rems(0.5))
        .child(icon(IconName::ArrowLeftRight).size(rems(0.875)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(rems(0.75))
                .child(lan::transfer_line(view)),
        )
        .child(
            div()
                .text_size(rems(0.6875))
                .text_color(theme::secondary(cx))
                .child(format!(
                    "{:.0} %",
                    if view.total == 0 {
                        0.0
                    } else {
                        view.bytes as f64 * 100.0 / view.total as f64
                    }
                )),
        )
        .into_any_element()
}

/// A two-button pager for a paged list.
fn pager(
    id: &'static str,
    page: usize,
    pages: usize,
    cx: &mut Context<Screen>,
    go: fn(&mut Screen, usize, &mut Context<Screen>),
) -> impl IntoElement + use<> {
    div()
        .flex()
        .items_center()
        .gap(rems(0.5))
        .child(
            button(
                format!("{id}-prev"),
                "Précédent",
                ButtonVariant::Secondary,
                cx,
            )
            .when(page == 0, |control| control.opacity(0.4))
            .on_click(cx.listener(move |this, _, _, cx| {
                if page > 0 {
                    go(this, page - 1, cx);
                    cx.notify();
                }
            })),
        )
        .child(hint(format!("{} / {pages}", page + 1), cx))
        .child(
            button(
                format!("{id}-next"),
                "Suivant",
                ButtonVariant::Secondary,
                cx,
            )
            .when(page + 1 >= pages, |control| control.opacity(0.4))
            .on_click(cx.listener(move |this, _, _, cx| {
                if page + 1 < pages {
                    go(this, page + 1, cx);
                    cx.notify();
                }
            })),
        )
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.reload(cx);
        // The drafts: created on first paint, kept per window (a
        // `use_keyed_state` may only run during layout/paint, D14a's rule).
        let friend_draft = window.use_keyed_state(eid("lan-friend"), cx, |window, cx| {
            InputState::new(window, cx).placeholder("Nom du compte à ajouter…")
        });
        self.friend_draft = Some(friend_draft.clone());
        let port_field = window.use_keyed_state(eid("lan-port"), cx, |window, cx| {
            InputState::new(window, cx).placeholder("45910")
        });
        if self.port_draft.is_none() {
            let port = self.settings.port.to_string();
            port_field.update(cx, |field, cx| field.set_value(port, window, cx));
        }
        self.port_draft = Some(port_field.clone());

        let mut column = div()
            .id("lan-column")
            .debug_selector(|| "lan-column".to_owned())
            .flex()
            .flex_col()
            .w_full()
            .max_w(rems(56.))
            .gap(rems(1.))
            .px(rems(1.25))
            .py(rems(1.125))
            .child(self.this_device_card(cx))
            .child(self.exchange_card(cx))
            .child(self.qualities_card(cx))
            .child(self.devices_card(cx));

        if self.error.is_some() || self.note.is_some() {
            column = column.child(self.feedback_card(cx));
        }
        column = column
            .child(self.mine_card(window, cx))
            .child(self.friends_card(window, cx))
            .child(self.advanced_card(window, cx, &port_field));

        div()
            .id("settings-lan")
            .debug_selector(|| "settings-lan".to_owned())
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .font_family(theme::font(cx))
            .text_color(theme::text(cx))
            .bg(theme::shell(cx))
            .child(div().flex().justify_center().w_full().child(column))
    }
}

#[cfg(test)]
mod ui {
    //! The section's own geometry and controls: the three windows the lot names,
    //! and a click that ACTS (a switch and a quality checkbox write the device's
    //! settings in the same frame).
    //!
    //! The state is built like D14a's harness: `auth` is set directly, so no
    //! request is needed, and the token runtime is never driven — a spawned
    //! request stays queued, which is what a geometry test wants. The snapshot the
    //! page draws is `with_snapshot`'s: two connected devices, a transfer, two
    //! enrolled devices, two friends, and a key with a date.

    use std::path::PathBuf;

    use gpui_kit::{
        AppContext as _, Context, IntoElement, ParentElement, Render, Styled, Window, point, px,
        size,
    };

    use super::*;
    use crate::nav::{Route, SettingsSection};
    use crate::state::{AppConfig, Auth, Feature, Features};
    use crate::wiring::lan::{
        Direction, LanSnapshot, LanState, PeerSet, PeerView, Relation, TransferView,
    };

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neezer-ln12-ui-{name}-{}-{}",
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

    /// The busiest realistic page: two devices connected (one of mine, one of a
    /// friend), a track being served, two enrolled devices, two trusted friends.
    fn snapshot() -> LanSnapshot {
        LanSnapshot {
            state: LanState::Connected(2),
            key_until: Some(1_801_353_600),
            key_error: None,
            peers: vec![
                PeerView {
                    device_pub: [7; 32],
                    name: "PC du salon".to_owned(),
                    relation: Relation::Own,
                    platform: "desktop".to_owned(),
                    rtt_ms: 3,
                    tracks: 8_000,
                    sets: 12,
                },
                PeerView {
                    device_pub: [9; 32],
                    name: "Alice".to_owned(),
                    relation: Relation::Friend {
                        name: "Alice".to_owned(),
                    },
                    platform: "android".to_owned(),
                    rtt_ms: 11,
                    tracks: 412,
                    sets: 2,
                },
            ],
            transfers: vec![TransferView {
                track: "3135556".to_owned(),
                title: "Balancing the Dark".to_owned(),
                peer: "PC du salon".to_owned(),
                direction: Direction::Recv,
                bytes: 1_200_000,
                total: 4_800_000,
                urgency: nz_lan::Urgency::Play,
            }],
            failures: Vec::new(),
            port: 45_910,
            port_fallback: false,
        }
    }

    fn devices() -> Vec<LanDevice> {
        vec![
            LanDevice {
                id: "d1".to_owned(),
                name: "PC du salon".to_owned(),
                platform: "desktop".to_owned(),
                created: None,
                last_seen: Some("il y a 2 min".to_owned()),
                revoked: None,
                current: true,
            },
            LanDevice {
                id: "d2".to_owned(),
                name: "Téléphone".to_owned(),
                platform: "android".to_owned(),
                created: None,
                last_seen: Some("hier".to_owned()),
                revoked: None,
                current: false,
            },
        ]
    }

    fn friends() -> LanFriends {
        use nz_model::{LanFriend, LanPlaylistRef};
        LanFriends {
            shares_to: vec![LanFriend {
                user_id: nz_model::UserId::new("u2"),
                name: "Alice".to_owned(),
                all: false,
                playlists: vec![LanPlaylistRef {
                    id: "p1".to_owned(),
                    title: "Route".to_owned(),
                }],
            }],
            trusted_by: vec![LanFriend {
                user_id: nz_model::UserId::new("u3"),
                name: "Benoît".to_owned(),
                all: true,
                playlists: Vec::new(),
            }],
        }
    }

    struct Host {
        screen: Entity<Screen>,
        focus: gpui_kit::FocusHandle,
        anchored: bool,
    }

    impl Render for Host {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            if !self.anchored {
                self.anchored = true;
                window.focus(&self.focus, cx);
            }
            gpui_omarchy::focus_scope("host")
                .size_full()
                .track_focus(&self.focus)
                .child(self.screen.clone())
        }
    }

    fn centre(bounds: gpui_kit::Bounds<gpui_kit::Pixels>) -> gpui_kit::Point<gpui_kit::Pixels> {
        point(
            bounds.origin.x + bounds.size.width / 2.,
            bounds.origin.y + bounds.size.height / 2.,
        )
    }

    fn click(cx: &mut gpui::VisualTestContext, at: gpui_kit::Point<gpui_kit::Pixels>) {
        cx.simulate_click(at, gpui_kit::Modifiers::default());
        cx.run_until_parked();
    }

    fn draw(cx: &mut gpui::VisualTestContext) {
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
    }

    /// **The page fits the three windows**, and every card the note names is
    /// painted: the exchange switches, the four tolerated qualities, the device
    /// lists, the firewall line, the key.
    #[gpui::test]
    async fn the_section_fits_the_three_windows(cx: &mut gpui::TestAppContext) {
        cx.update(crate::theme::init);
        let dir = temp_dir("layout");
        let state = signed_in(cx, &dir);
        let host_state = state.clone();
        let (host, cx) = cx.add_window_view(move |_window, cx| Host {
            screen: cx.new(|cx| {
                Screen::with_snapshot(host_state.clone(), snapshot(), devices(), friends(), cx)
            }),
            focus: cx.focus_handle(),
            anchored: false,
        });
        let handle = cx.windows()[0];
        for (width, height) in [(2560., 1600.), (1280., 800.), (660., 800.)] {
            cx.simulate_window_resize(handle, size(px(width), px(height)));
            cx.run_until_parked();
            draw(cx);
            let viewport = cx.update(|window, _| window.viewport_size());
            assert!(
                (f32::from(viewport.width) - width).abs() < 1.,
                "the window is {width} wide"
            );
            let root = cx
                .debug_bounds("settings-lan")
                .unwrap_or_else(|| panic!("no LAN screen at {width}"));
            assert!(
                (f32::from(root.size.width) - width).abs() < 1.,
                "the screen paints {} wide in a {width} px window",
                root.size.width
            );
            let column = cx
                .debug_bounds("lan-column")
                .unwrap_or_else(|| panic!("no column at {width}"));
            assert!(
                f32::from(column.size.width) <= width,
                "the column overflows sideways at {width}: {} px",
                column.size.width
            );
            // The cards of the note's list, all really painted, at every size.
            for selector in [
                "lan-enabled",
                "lan-serve",
                "lan-quality-flac",
                "lan-quality-opus320",
                "lan-quality-opus128",
                "lan-quality-opus64",
                "lan-copy-ufw",
                "lan-renew",
            ] {
                let bounds = cx
                    .debug_bounds(selector)
                    .unwrap_or_else(|| panic!("{selector} is not painted at {width}"));
                assert!(
                    f32::from(bounds.size.width) > 0. && f32::from(bounds.size.height) > 0.,
                    "{selector} has no box at {width}"
                );
            }
        }
        drop(host);
        std::fs::remove_dir_all(dir).ok();
    }

    /// **A click acts, in the same frame** (rule 4): the two switches and a
    /// quality checkbox write the device's settings — `lan.enabled`, `lan.serve`
    /// and `lan.tolerated` — and the switch's own label follows at once.
    #[gpui::test]
    async fn a_click_writes_the_device_settings_and_forgets_nothing(cx: &mut gpui::TestAppContext) {
        cx.update(crate::theme::init);
        let dir = temp_dir("clicks");
        let state = signed_in(cx, &dir);
        let host_state = state.clone();
        let (_host, cx) = cx.add_window_view(move |_window, cx| Host {
            screen: cx.new(|cx| {
                Screen::with_snapshot(host_state.clone(), snapshot(), devices(), friends(), cx)
            }),
            focus: cx.focus_handle(),
            anchored: false,
        });
        let handle = cx.windows()[0];
        cx.simulate_window_resize(handle, size(px(1280.), px(800.)));
        cx.run_until_parked();
        draw(cx);

        let cache = state.read_with(cx, |state, _| state.cache.clone().expect("a store"));
        let read = |cache: &nz_cache::Cache| -> (bool, bool, Vec<String>) {
            (
                cache.kv().get::<bool>("lan.enabled").unwrap_or(true),
                cache.kv().get::<bool>("lan.serve").unwrap_or(true),
                cache
                    .kv()
                    .get::<Vec<String>>("lan.tolerated")
                    .unwrap_or_default(),
            )
        };
        // Nothing written yet: a fresh device is the defaults.
        assert_eq!(read(&cache), (true, true, Vec::new()));

        let serve = cx.debug_bounds("lan-serve").expect("the share switch");
        click(cx, centre(serve));
        assert!(
            !read(&cache).1,
            "« Partager mes sons » is off on the device after one click"
        );

        // Opus 64 is the one rung the default set does not tolerate: one click
        // adds it (and the setting is the SPA's own spelling).
        let opus64 = cx
            .debug_bounds("lan-quality-opus64")
            .expect("the Opus 64 box");
        click(cx, centre(opus64));
        assert!(
            read(&cache).2.contains(&"OPUS_64".to_owned()),
            "Opus 64 is tolerated: {:?}",
            read(&cache).2
        );
        // Turning every rung off is refused in the same frame: at least one stays.
        for selector in [
            "lan-quality-flac",
            "lan-quality-opus320",
            "lan-quality-opus128",
        ] {
            let card = cx.debug_bounds(selector).expect("a quality box");
            click(cx, centre(card));
        }
        let left = read(&cache).2;
        assert_eq!(left, vec!["OPUS_64".to_owned()], "one rung stays: {left:?}");
        // The switch above still reads what was written (the page follows the
        // device, not a stale copy).
        assert!(!read(&cache).1);
        std::fs::remove_dir_all(dir).ok();
    }

    /// A server date is read as a day (`2026-11-18T18:40:00Z` → « 18 nov. 2026 »),
    /// and a shape this refuses is not printed raw.
    #[test]
    fn a_device_row_prints_a_day_and_a_platform() {
        let device = |platform: &str, seen: Option<&str>| LanDevice {
            id: "d".to_owned(),
            name: "PC".to_owned(),
            platform: platform.to_owned(),
            created: None,
            last_seen: seen.map(str::to_owned),
            revoked: None,
            current: false,
        };
        assert_eq!(
            last_seen(&device("desktop", Some("2026-11-18T18:40:00Z"))),
            "Bureau · vu le 18 nov. 2026"
        );
        assert_eq!(
            last_seen(&device("android", Some("2026-01-05T07:02:00Z"))),
            "Android · vu le 5 janv. 2026"
        );
        assert_eq!(last_seen(&device("desktop", None)), "Bureau");
        assert_eq!(
            last_seen(&device("desktop", Some("hier"))),
            "Bureau · vu hier",
            "an unexpected shape is not dropped, and not printed as an instant"
        );
    }

    /// The section is in the strip with the person's own word, and its title is
    /// the note's: « Appareils à proximité ».
    #[test]
    fn the_section_is_named_like_the_note_says() {
        assert_eq!(SettingsSection::Lan.title(), "Appareils à proximité");
        assert_eq!(SettingsSection::Lan.slug(), "appareils-a-proximite");
        assert_eq!(
            SettingsSection::from_slug("appareils-a-proximite"),
            Some(SettingsSection::Lan)
        );
        assert_eq!(
            super::super::section_hint(SettingsSection::Lan),
            "Vos appareils sur le même réseau, et ce que vous partagez"
        );
        // The peer-set card's summary drops the track ids (a 4 000-track set must
        // not sit in the UI's state) and keeps what a row prints.
        let set = PeerSet {
            peer: [7; 32],
            peer_name: "PC du salon".to_owned(),
            relation: Relation::Own,
            set: nz_lan::proto::SetInfo {
                kind: "playlist".to_owned(),
                ident: "p1".to_owned(),
                title: "Route".to_owned(),
                cover: None,
                tracks: (0..4_000).map(|index| format!("t{index}")).collect(),
                local: 4_000,
                complete: true,
            },
            bytes: 640_000_000,
        };
        let row = crate::screens::peer_sets::Row::summary(&set);
        assert_eq!(row.tracks, 4_000);
        assert!(row.complete);
        assert_eq!(row.key, format!("{}:playlist:p1", "07".repeat(32)));
        assert!(row.detail().contains("complet"));
        assert!(row.detail().contains("610,4 Mo"), "{}", row.detail());
        // A server without the feature: the route says which one is missing.
        assert_eq!(
            Features::from_missing([Feature::Lan]).absent(&Route::Settings {
                section: SettingsSection::Lan
            }),
            Some("L'échange entre appareils proches")
        );
    }
}
