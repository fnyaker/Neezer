//! Écoute partagée (lot D15b) : animer une party, ou en écouter une.
//!
//! The SPA's `PartySheet.svelte` (the host) and `routes/Party.svelte` (the
//! guest) in one screen, because a desktop app has no link to open in a
//! browser: a guest pastes the id or the link it was sent, exactly like D15a's
//! controller pastes a control link. The contract is `neezer/webui/party.py`,
//! the behaviour `webapp/src/lib/party/`, the notes
//! [party-remote.md](../../../../docs/agents/party-remote.md).
//!
//! Three rules, each one the SPA's:
//!
//! - **The host keeps the hand and the player is untouched.** Starting a party
//!   publishes the line this player follows (`wiring::party_host`); it never
//!   stops, seeks or reorders anything, and a guest hears exactly what the host
//!   plays.
//! - **A guest is a listener, never a controller.** It sees the host's track,
//!   its own playhead on the shared timeline, who else is there, and two
//!   controls the room needs: the volume in this room, and this device's own
//!   output delay (`Décalage` — a Bluetooth speaker that does not report it).
//! - **Nothing long happens in a frame** (rule 2): the peek, the join, the
//!   start and the end are tasks on `nz-net`; the music and the clock keep
//!   running in `wiring::party_guest` after this screen is gone.
//!
//! Every control is a `button`/`toggle`, so Tab reaches it and Entrée presses
//! it, and the screen takes NO focus (D8b's rule: a screen that focuses itself
//! breaks the shell's focus anchor and every scoped key with it).

use std::time::Duration;

use gpui_kit::base::input::InputState;
use gpui_kit::base::slider::{SliderEvent, SliderState, SliderValue};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, Entity, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon, icon_button, input, progress, slider};
use nz_api::Lane;
use nz_model::PartyState;
use nz_player::Command;

use crate::nav::Route;
use crate::screens::settings::{card as settings_card, card_title, hint};
use crate::state::AppState;
use crate::theme;
use crate::toast::ToastKind;
use crate::widgets::cover::{self, Covers};
use crate::widgets::sheets::{error_line, face, field_label, note_line, push_toast};
use crate::wiring::party_guest::{self, Guest, GuestView, LATENCY_KEY, NAME_KEY, Phase};
use crate::wiring::party_host::{HostStatus, PartyHost};

/// The paint clock: the guest's playhead moves between two polls, and the host's
/// listener list changes on `nz-net`.
const PAINT: Duration = Duration::from_millis(250);
/// The widest a card gets: on a 2 560 px window the two columns are margins
/// rather than a two-metre line (D11/D12's rule).
const CARD: f32 = 30.0;

pub struct Screen {
    route: Route,
    state: Entity<AppState>,

    // -- the host's side ----------------------------------------------------
    host: Option<PartyHost>,
    host_busy: bool,
    confirm_end: bool,

    // -- the guest's side ---------------------------------------------------
    guest: Option<Guest>,
    /// The id or link being typed, and what the peek answered.
    link: Option<Entity<InputState>>,
    peek: Option<PartyState>,
    peek_of: String,
    peek_busy: bool,
    peek_error: Option<SharedString>,
    /// The listener's name, and the two controls the room needs.
    name: Option<Entity<InputState>>,
    volume: Option<Entity<SliderState>>,
    join_busy: bool,
    /// The sleeves of the two panes (the host's track and the guest's).
    covers: std::rc::Rc<std::cell::RefCell<Covers>>,

    // -- paint bookkeeping --------------------------------------------------
    painted_host: u64,
    painted_guest: u64,
    painted_position: f64,
    _state: Subscription,
}

impl Screen {
    /// The screen `screens::build` calls. No request and no field: a screen is
    /// built without a window (D7), so the two text fields and the slider are
    /// made on the first paint.
    pub fn new(route: Route, state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let _state = cx.observe(&state, |_, _, cx| cx.notify());
        let host = PartyHost::get();
        let guest = Guest::get().filter(|guest| {
            matches!(
                guest.view().phase,
                Phase::Joining | Phase::Live | Phase::Left | Phase::Ended
            )
        });
        let mut painted_host = host.as_ref().map(|host| host.status().version).unwrap_or(0);
        let mut painted_guest = guest
            .as_ref()
            .map(|guest| guest.view().version)
            .unwrap_or(0);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PAINT).await;
                let alive = this.update(cx, |this, cx| this.paint(cx));
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        let _ = (&mut painted_host, &mut painted_guest);
        Self {
            route,
            state,
            host,
            host_busy: false,
            confirm_end: false,
            guest,
            link: None,
            peek: None,
            peek_of: String::new(),
            peek_busy: false,
            peek_error: None,
            name: None,
            volume: None,
            join_busy: false,
            covers: std::rc::Rc::new(std::cell::RefCell::new(Covers::new())),
            painted_host,
            painted_guest,
            painted_position: 0.0,
            _state,
        }
    }

    pub fn route(&self) -> &Route {
        &self.route
    }

    /// What the paint clock watches: the host's version, the guest's, and the
    /// playhead — which moves even when nothing else does.
    fn paint(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        if self.host.is_none() {
            self.host = PartyHost::get();
        }
        if let Some(host) = &self.host {
            let version = host.status().version;
            if version != self.painted_host {
                self.painted_host = version;
                changed = true;
            }
        }
        if self.guest.is_none() {
            self.guest = Guest::get();
        }
        if let Some(guest) = &self.guest {
            let view = guest.view();
            if view.version != self.painted_guest {
                self.painted_guest = view.version;
                changed = true;
            }
            if view.phase == Phase::Live && view.playing {
                let position = guest.position_now();
                if (position - self.painted_position).abs() >= 0.2 {
                    self.painted_position = position;
                    changed = true;
                }
            }
        }
        if changed {
            cx.notify();
        }
    }

    fn ensure_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.link.is_none() {
            self.link = Some(
                window
                    .use_keyed_state("party-link", cx, |window, cx| {
                        InputState::new(window, cx)
                            .placeholder("Lien ou identifiant de la party (…/party/<id>)")
                    })
                    .clone(),
            );
        }
        if self.name.is_none() {
            let saved = self
                .state
                .read(cx)
                .cache
                .as_ref()
                .and_then(|cache| cache.kv().get::<String>(NAME_KEY))
                .unwrap_or_default();
            self.name = Some(
                window
                    .use_keyed_state("party-name", cx, |window, cx| {
                        let field = InputState::new(window, cx)
                            .placeholder("Votre prénom (visible par l'hôte)");
                        if saved.is_empty() {
                            field
                        } else {
                            field.default_value(saved)
                        }
                    })
                    .clone(),
            );
        }
        if self.volume.is_none() {
            let value = self.volume_value(cx);
            let slider = cx.new(move |_| {
                SliderState::new()
                    .min(0.)
                    .max(1.)
                    .step(0.02)
                    .default_value(value)
            });
            let guest = self.guest.clone();
            let state = self.state.clone();
            cx.subscribe(&slider, move |_this, _, event: &SliderEvent, cx| {
                let value = match event {
                    SliderEvent::Change(SliderValue::Single(value))
                    | SliderEvent::Release(SliderValue::Single(value)) => f64::from(*value),
                    _ => return,
                };
                let value = value.clamp(0.0, 1.0);
                if let Some(guest) = &guest {
                    guest.set_volume(value);
                }
                if let Some(cache) = state.read(cx).cache.clone() {
                    let _ = cache.kv().set(party_guest::VOLUME_KEY, &value);
                }
            })
            .detach();
            self.volume = Some(slider);
        }
    }

    fn volume_value(&self, cx: &App) -> f32 {
        self.state
            .read(cx)
            .cache
            .as_ref()
            .and_then(|cache| cache.kv().get::<f64>(party_guest::VOLUME_KEY))
            .map(|value| value.clamp(0.0, 1.0) as f32)
            .unwrap_or(1.0)
    }

    fn latency_value(&self, cx: &App) -> f64 {
        self.state
            .read(cx)
            .cache
            .as_ref()
            .and_then(|cache| cache.kv().get::<f64>(LATENCY_KEY))
            .unwrap_or(0.0)
    }

    // --- the host's side ---------------------------------------------------

    fn start(&mut self, cx: &mut Context<Self>) {
        if self.host_busy {
            return;
        }
        self.host_busy = true;
        cx.notify();
        let state = self.state.clone();
        cx.spawn(async move |this, cx| {
            // The start is a request: it answers on `nz-net`, and the sheet
            // reads the service's status afterwards (rule 2: a button that seems
            // to do nothing is a bug).
            state.update(cx, PartyHost::start);
            this.update(cx, |this, cx| {
                this.host_busy = false;
                this.host = PartyHost::get();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn end(&mut self, cx: &mut Context<Self>) {
        if !self.confirm_end {
            self.confirm_end = true;
            cx.notify();
            return;
        }
        self.confirm_end = false;
        let state = self.state.clone();
        cx.spawn(async move |this, cx| {
            state.update(cx, |state, cx| {
                if let Some(host) = PartyHost::get() {
                    host.end(state, cx);
                }
            });
            this.update(cx, |this, cx| {
                this.host = PartyHost::get();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The link IS the capability, so copying it is how a desktop hands one out.
    fn copy(&mut self, url: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(url));
        let state = self.state.clone();
        push_toast(
            cx,
            &state,
            ToastKind::Info,
            "Lien copié — envoyez-le aux personnes qui écoutent avec vous.",
        );
    }

    // --- the guest's side --------------------------------------------------

    /// `GET /api/party/<id>`: who invites you and what is playing, before you
    /// commit to hearing it (the SPA's landing card).
    fn peek(&mut self, cx: &mut Context<Self>) {
        if self.peek_busy {
            return;
        }
        let raw = self
            .link
            .as_ref()
            .map(|field| field.read(cx).value().to_string())
            .unwrap_or_default();
        let Some(pid) = party_id(&raw) else {
            self.peek_error =
                Some("Collez un lien d'écoute (…/party/<id>) ou son identifiant.".into());
            cx.notify();
            return;
        };
        let Some(api) = self.state.read(cx).api() else {
            self.peek_error = Some("Le serveur n'est pas joint.".into());
            cx.notify();
            return;
        };
        self.peek_busy = true;
        self.peek_error = None;
        self.peek_of = pid.clone();
        cx.notify();
        let runtime = self.state.read(cx).runtime.clone();
        let task = runtime.spawn(async move { Guest::peek(&api, &pid).await });
        cx.spawn(async move |this, cx| {
            let answer = task.await;
            this.update(cx, |this, cx| {
                this.peek_busy = false;
                match answer {
                    Ok(Ok(Some(state))) => this.peek = Some(state),
                    Ok(Ok(None)) => {
                        this.peek = None;
                        this.peek_error = Some("Cette party n'existe plus.".into());
                    }
                    Ok(Err(error)) => this.peek_error = Some(error.cause_fr().into()),
                    Err(_) => this.peek_error = Some("La party n'a pas pu être lue.".into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Join: a seat for the host's list, then the session (clock probes, poll
    /// loop, scheduler). The person's own music is paused first — two songs at
    /// once is not a feature — and that is the ONE transport path.
    fn join(&mut self, cx: &mut Context<Self>) {
        if self.join_busy {
            return;
        }
        let pid = self.peek_of.clone();
        if pid.is_empty() {
            return;
        }
        let name = self
            .name
            .as_ref()
            .map(|field| field.read(cx).value().trim().to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Invité".to_owned());
        let (Some(api), Some(_)) = (self.state.read(cx).api(), Some(())) else {
            return;
        };
        let runtime = self.state.read(cx).runtime.clone();
        let state = self.state.clone();
        let cache = state.read(cx).cache.clone();
        if let Some(cache) = &cache {
            let _ = cache.kv().set(NAME_KEY, &name);
        }
        let volume = self.volume_value(cx);
        let trim = self.latency_value(cx);
        self.join_busy = true;
        self.peek_error = None;
        cx.notify();
        let join_pid = pid.clone();
        let join_name = name.clone();
        let task = runtime.spawn(async move {
            let lid = Guest::join(&api, &join_pid, &join_name).await.ok();
            (api, lid)
        });
        cx.spawn(async move |this, cx| {
            let answer = task.await;
            this.update(cx, |this, cx| {
                this.join_busy = false;
                let Ok((api, lid)) = answer else {
                    this.peek_error = Some("La party n'a pas pu être rejointe.".into());
                    cx.notify();
                    return;
                };
                // The engine's own party channel: the guest's audio goes through
                // the same output, DSP and volume as a track (`nz-audio`).
                let (channel, rate) = {
                    let state = state.read(cx);
                    match &state.audio {
                        Some(audio) => (Some(audio.engine.party()), audio.sample_rate as u32),
                        None => (None, 48_000),
                    }
                };
                if channel.is_none() {
                    this.peek_error =
                        Some("Aucune sortie audio : l'écoute partagée jouera sans son.".into());
                }
                state.update(cx, |state, cx| {
                    // The room's sound is the party's: pause what this device
                    // was playing (queue and position kept), through the one
                    // transport path.
                    if state.player.playing() {
                        state.command(Command::pause(), cx);
                    }
                });
                let runtime = state.read(cx).runtime.clone();
                let guest = Guest::start(
                    api,
                    runtime,
                    pid.clone(),
                    name.clone(),
                    lid,
                    channel,
                    rate,
                    f64::from(volume),
                    trim,
                );
                this.guest = Some(guest);
                this.painted_guest = 0;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn leave(&mut self, cx: &mut Context<Self>) {
        if let Some(guest) = &self.guest {
            guest.leave();
        }
        self.guest = Guest::get();
        cx.notify();
    }

    /// The listener's own delay, applied a moment after the last tap so a run of
    /// taps is one re-alignment (`Party.svelte#nudge`).
    fn nudge(&mut self, delta: f64, cx: &mut Context<Self>) {
        let current = self
            .guest
            .as_ref()
            .map(|guest| guest.trim())
            .unwrap_or_else(|| self.latency_value(cx));
        let next = (current + delta).clamp(-500.0, 1000.0).round();
        if let Some(guest) = &self.guest {
            guest.set_trim(next);
        }
        if let Some(cache) = self.state.read(cx).cache.clone() {
            let _ = cache.kv().set(LATENCY_KEY, &next);
        }
        cx.notify();
    }

    fn retry_guest(&mut self, cx: &mut Context<Self>) {
        self.peek = None;
        self.peek_error = None;
        self.peek_of.clear();
        cx.notify();
    }

    // --- the sleeves -------------------------------------------------------

    /// A party's art comes through the party (`/api/party/<pid>/cover/<mid>`):
    /// a guest has no session, and the host's library stays closed.
    fn want_cover(&self, url: Option<&String>, cx: &mut Context<Self>) {
        let Some(url) = url else { return };
        if url.is_empty() || self.covers.borrow().file(url).is_some() {
            return;
        }
        let state = self.state.read(cx);
        let (Some(api), Some(cache)) = (state.api(), state.cache.clone()) else {
            return;
        };
        if !state.online {
            return;
        }
        let key = url.clone();
        let fetch = url.clone();
        let path = url.trim_start_matches("/api").to_owned();
        let runtime = state.runtime.clone();
        // The sleeve is a file: the network task only writes it and hands back
        // its path, and the map is touched here, on the screen's own thread (an
        // `Rc` does not cross a task).
        let task = runtime.spawn(async move {
            if let Some(file) = cache.covers().file(&fetch) {
                return Some(file);
            }
            match api.bytes(Lane::User, &path).await {
                Ok(bytes) => cache.covers().put(&fetch, &bytes).ok(),
                Err(_) => None,
            }
        });
        cx.spawn(async move |this, cx| {
            let answer = task.await;
            this.update(cx, |this, cx| {
                this.covers
                    .borrow_mut()
                    .resolved(&key, answer.ok().flatten());
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for Screen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_fields(window, cx);
        let host = self.host.as_ref().map(|host| host.status());
        let guest = self.guest.as_ref().map(|guest| guest.view());
        let position = self
            .guest
            .as_ref()
            .map(|guest| guest.position_now())
            .unwrap_or(0.0);
        self.want_cover(
            guest
                .as_ref()
                .and_then(|view| view.track.as_ref())
                .and_then(|track| track.cover.as_ref()),
            cx,
        );
        let tone = theme::text(cx);
        div()
            .id("party-screen")
            .size_full()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .bg(theme::shell(cx))
            .font_family(theme::font(cx))
            .text_color(tone)
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .px(rems(1.5))
                    .py(rems(1.5))
                    .gap(rems(1.25))
                    .child(self.header(cx))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_start()
                            .justify_center()
                            .gap(rems(1.25))
                            .w_full()
                            .max_w(rems(CARD * 2. + 1.5))
                            .child(self.host_card(host.as_ref(), cx))
                            .child(self.guest_card(guest.as_ref(), position, window, cx)),
                    ),
            )
    }
}

impl Screen {
    fn header(&self, cx: &App) -> Div {
        div()
            .flex()
            .items_center()
            .gap(rems(0.75))
            .w_full()
            .max_w(rems(CARD * 2. + 1.5))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(rems(2.5))
                    .border_1()
                    .border_color(theme::border(cx))
                    .bg(theme::chrome(cx))
                    .text_color(theme::accent(cx))
                    .child(icon(IconName::PartyPopper).size(rems(1.25))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.125))
                    .child(
                        div()
                            .text_size(rems(1.125))
                            .font_weight(FontWeight::BOLD)
                            .child("Écoute partagée"),
                    )
                    .child(hint(
                        "La même musique, au même instant, sur chaque appareil.",
                        cx,
                    )),
            )
    }

    fn host_card(&self, host: Option<&HostStatus>, cx: &mut Context<Self>) -> AnyElement {
        let mut body = settings_card("party-host", cx)
            .w(rems(CARD))
            .child(card_title("Animer", cx));
        if let Some(status) = host.filter(|status| status.hosting()) {
            body = body
                .child(note_line(format!("En direct · {}", status.audience()), cx))
                .child(
                    div()
                        .id("party-link")
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .p(rems(0.625))
                        .border_1()
                        .border_color(theme::border(cx))
                        .bg(theme::shell(cx))
                        .font_family(theme::mono(cx))
                        .text_size(rems(0.75))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(status.link.clone()),
                        )
                        .child(
                            icon_button(
                                "party-copy",
                                IconName::Copy,
                                "Copier le lien",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(cx.listener({
                                let link = status.link.clone();
                                move |this, _, _, cx| this.copy(link.clone(), cx)
                            })),
                        ),
                )
                .child(hint(
                    "Qui ouvre ce lien entend ce que vous jouez, calé à la milliseconde \
                     près — sans compte.",
                    cx,
                ));
            if status.lost {
                body = body.child(note_line(
                    "Le serveur a redémarré et le lien a changé — partagez le nouveau.",
                    cx,
                ));
            }
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.375))
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::secondary(cx))
                            .child("À L'ÉCOUTE"),
                    )
                    .children(if status.listeners.is_empty() {
                        vec![
                            hint(
                                "Personne pour l'instant. Dès qu'un invité ouvre le lien, il \
                             apparaît ici.",
                                cx,
                            )
                            .into_any_element(),
                        ]
                    } else {
                        status
                            .listeners
                            .iter()
                            .map(|listener| {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(rems(0.625))
                                    .py(rems(0.25))
                                    .child(face(&listener.name, 24., cx))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .text_size(rems(0.8125))
                                            .child(listener.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(rems(0.6875))
                                            .text_color(if listener.st.as_deref() == Some("sync") {
                                                theme::success(cx)
                                            } else {
                                                theme::secondary(cx)
                                            })
                                            .child(listener.status()),
                                    )
                                    .into_any_element()
                            })
                            .collect()
                    }),
            );
            body = body.child(note_line(
                match status.clock {
                    Some((spread, rtt)) => format!(
                        "Horloge de cet appareil calée à ±{:.1} ms · aller-retour {:.0} ms",
                        spread.max(0.1),
                        rtt
                    ),
                    None => "Synchronisation de l'horloge…".to_owned(),
                },
                cx,
            ));
            if let Some(error) = &status.error {
                body = body.child(error_line(&error.clone().into(), cx));
            }
            body = body.child(
                div().flex().items_center().gap(rems(0.5)).child(
                    button(
                        "party-end",
                        if self.confirm_end {
                            "Confirmer : tout le monde est déconnecté"
                        } else {
                            "Terminer la party"
                        },
                        if self.confirm_end {
                            ButtonVariant::Danger
                        } else {
                            ButtonVariant::Secondary
                        },
                        cx,
                    )
                    .child(icon(IconName::LogOut).size(rems(0.75)))
                    .on_click(cx.listener(|this, _, _, cx| this.end(cx))),
                ),
            );
        } else {
            body = body.child(self.host_intro(cx)).child(
                button(
                    "party-start",
                    if self.host_busy {
                        "Démarrage…"
                    } else {
                        "Démarrer l'écoute partagée"
                    },
                    ButtonVariant::Primary,
                    cx,
                )
                .child(icon(IconName::PartyPopper).size(rems(0.875)))
                .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
            );
            if let Some(status) = host
                && let Some(error) = &status.error
            {
                body = body.child(error_line(&error.clone().into(), cx));
            }
        }
        body.into_any_element()
    }

    fn host_intro(&self, cx: &App) -> Div {
        let mut column = div().flex().flex_col().gap(rems(0.625));
        for (glyph, text) in [
            (
                IconName::Link2,
                "Partagez un lien : ceux qui l'ouvrent entendent ce que vous jouez, calé à la \
                 milliseconde près.",
            ),
            (
                IconName::Users,
                "Dans une même pièce, chaque appareil devient une enceinte de plus.",
            ),
            (
                IconName::Music,
                "Vous gardez la main : ils suivent vos titres, vos pauses et vos enchaînements.",
            ),
        ] {
            column = column.child(
                div()
                    .flex()
                    .items_start()
                    .gap(rems(0.625))
                    .child(icon(glyph).size(rems(1.125)).text_color(theme::accent(cx)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(rems(0.8125))
                            .text_color(theme::text(cx))
                            .child(text),
                    ),
            );
        }
        column
    }

    fn guest_card(
        &self,
        view: Option<&GuestView>,
        position: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut body = settings_card("party-guest", cx)
            .w(rems(CARD))
            .child(card_title("Rejoindre", cx));
        match view.map(|view| view.phase) {
            Some(Phase::Joining) | Some(Phase::Live) => {
                let Some(view) = view else {
                    return body.into_any_element();
                };
                body = body.child(self.live_pane(view, position, window, cx));
            }
            Some(Phase::Ended) => {
                body = body.child(self.goodbye(
                    "La party est terminée",
                    "L'hôte l'a arrêtée. Merci d'avoir écouté.",
                    false,
                    cx,
                ));
            }
            Some(Phase::Left) => {
                body = body.child(self.goodbye(
                    "Vous avez quitté la party",
                    "Le son de la pièce est revenu à votre lecteur.",
                    true,
                    cx,
                ));
            }
            _ => {
                body = body.child(hint(
                    "Collez le lien qu'on vous a envoyé. Aucun compte n'est nécessaire pour \
                     écouter.",
                    cx,
                ));
                body = body.child(self.join_form(window, cx));
                if let Some(error) = &self.peek_error {
                    body = body.child(error_line(error, cx));
                }
            }
        }
        body.into_any_element()
    }

    fn goodbye(
        &self,
        title: &'static str,
        line: &'static str,
        rejoin: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut column = div()
            .flex()
            .flex_col()
            .gap(rems(0.625))
            .child(
                div()
                    .text_size(rems(0.9375))
                    .font_weight(FontWeight::BOLD)
                    .child(title),
            )
            .child(hint(line, cx));
        if rejoin {
            column = column.child(
                button(
                    "party-again",
                    "Rejoindre à nouveau",
                    ButtonVariant::Secondary,
                    cx,
                )
                .w_full()
                .child(icon(IconName::RotateCcw).size(rems(0.75)))
                .on_click(cx.listener(|this, _, _, cx| this.retry_guest(cx))),
            );
        }
        column
    }

    fn join_form(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mut column = div().flex().flex_col().gap(rems(0.5));
        if let Some(field) = &self.link {
            column = column
                .child(field_label("Lien ou identifiant", cx))
                .child(input("party-link-field", field, window, cx));
        }
        column = column.child(
            button(
                "party-peek",
                if self.peek_busy {
                    "Lecture…"
                } else {
                    "Voir la party"
                },
                ButtonVariant::Secondary,
                cx,
            )
            .on_click(cx.listener(|this, _, _, cx| this.peek(cx))),
        );
        if let Some(peek) = &self.peek {
            let listeners = peek.listeners.len();
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.25))
                    .p(rems(0.625))
                    .border_1()
                    .border_color(theme::border(cx))
                    .bg(theme::shell(cx))
                    .child(
                        div()
                            .text_size(rems(0.8125))
                            .child(format!("{} vous invite à écouter", peek.host)),
                    )
                    .child(hint(
                        match (peek.track.as_ref(), listeners) {
                            (Some(track), 0) => format!("{} — {}", track.title, track.artist),
                            (Some(track), n) => format!(
                                "{} — {} · {n} personne{} déjà là",
                                track.title,
                                track.artist,
                                if n > 1 { "s" } else { "" }
                            ),
                            (None, _) => "Rien en lecture pour l'instant".to_owned(),
                        },
                        cx,
                    )),
            );
            if let Some(field) = &self.name {
                column = column.child(field_label("Votre prénom", cx)).child(input(
                    "party-name-field",
                    field,
                    window,
                    cx,
                ));
            }
            column = column.child(
                button(
                    "party-join",
                    if self.join_busy {
                        "Connexion…"
                    } else {
                        "Rejoindre l'écoute"
                    },
                    ButtonVariant::Primary,
                    cx,
                )
                .w_full()
                .child(icon(IconName::PartyPopper).size(rems(0.875)))
                .on_click(cx.listener(|this, _, _, cx| this.join(cx))),
            );
        }
        column
    }

    fn live_pane(
        &self,
        view: &GuestView,
        position: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let track = view.track.as_ref();
        let duration = view
            .duration_s
            .max(track.map(|track| track.duration).unwrap_or(0.0));
        let percent = if duration > 0.0 {
            ((position / duration) * 100.0).clamp(0.0, 100.0) as f32
        } else {
            0.0
        };
        let (tone, line) = view.status_line();
        let file = track
            .and_then(|track| track.cover.as_ref())
            .and_then(|url| cover::file_in(&self.covers, url));
        let mut column = div().flex().flex_col().gap(rems(0.75));
        // The sleeve and the title, the SPA's stage.
        column = column.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.875))
                .child(cover::view(
                    file.as_ref(),
                    track.and_then(|track| track.cover.as_deref()),
                    track.map(|track| track.title.as_str()).unwrap_or(""),
                    if track.map(|track| track.podcast).unwrap_or(false) {
                        cover::Kind::Podcast
                    } else {
                        cover::Kind::Album
                    },
                    px(120.),
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(rems(0.25))
                        .child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme::secondary(cx))
                                .child(format!(
                                    "En direct avec {}{}",
                                    view.host,
                                    if view.listeners > 1 {
                                        format!(" · {} à l'écoute", view.listeners)
                                    } else {
                                        String::new()
                                    }
                                )),
                        )
                        .child(
                            div()
                                .text_size(rems(1.))
                                .font_weight(FontWeight::BOLD)
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(track.map(|track| track.title.clone()).unwrap_or_else(
                                    || "Rien en lecture pour l'instant".to_owned(),
                                )),
                        )
                        .child(
                            div()
                                .text_size(rems(0.8125))
                                .text_color(theme::secondary(cx))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(track.map(|track| track.artist.clone()).unwrap_or_default()),
                        ),
                ),
        );
        // The playhead: the line the room follows, not this device's own.
        column = column
            .child(progress("party-progress", percent, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .font_family(theme::mono(cx))
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child(crate::player_bar::clock(position))
                    .child(crate::player_bar::clock(duration)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.375))
                    .text_size(rems(0.8125))
                    .text_color(match tone {
                        party_guest::Tone::Ok => theme::success(cx),
                        party_guest::Tone::Warn => theme::warning(cx),
                        party_guest::Tone::Error => theme::danger(cx),
                        party_guest::Tone::Wait => theme::secondary(cx),
                    })
                    .child(line),
            );
        if let Some(next) = &view.next {
            column = column.child(hint(
                format!(
                    "Ensuite : {}{}",
                    next.title,
                    if next.artist.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", next.artist)
                    }
                ),
                cx,
            ));
        }
        // Volume and the device's own delay.
        column = column.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    icon(IconName::Volume1)
                        .size(rems(1.))
                        .text_color(theme::secondary(cx)),
                )
                .child(div().flex_1().min_w_0().child(match &self.volume {
                    Some(slider_state) => {
                        slider(slider_state, false, window, cx).into_any_element()
                    }
                    None => div().into_any_element(),
                })),
        );
        column = column.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.375))
                .child(
                    div()
                        .flex_none()
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child("Décalage"),
                )
                .child(
                    icon_button(
                        "party-latency-down",
                        IconName::Minus,
                        "Avancer de 10 ms",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.nudge(-10.0, cx))),
                )
                .child(
                    div()
                        .font_family(theme::mono(cx))
                        .text_size(rems(0.75))
                        .child(format!("{:+} ms", view.trim_ms.round() as i64)),
                )
                .child(
                    icon_button(
                        "party-latency-up",
                        IconName::Plus,
                        "Retarder de 10 ms",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.nudge(10.0, cx))),
                )
                .child(
                    icon_button(
                        "party-latency-reset",
                        IconName::RotateCcw,
                        "Remettre le décalage à zéro",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        let current = this.guest.as_ref().map(|guest| guest.trim()).unwrap_or(0.0);
                        this.nudge(-current, cx);
                    })),
                ),
        );
        column = column.child(hint(
            match view.os_latency_ms {
                Some(ms) => format!(
                    "Cet appareil sonne en retard sur les autres (enceinte Bluetooth, barre de \
                     son) ? Avancez-le. Le tampon de sortie ({ms} ms) est déjà compensé."
                ),
                None => "Cet appareil sonne en retard ? Avancez-le. En avance ? Retardez-le. \
                         Réglez à l'oreille, deux appareils côte à côte."
                    .to_owned(),
            },
            cx,
        ));
        if matches!(view.phase, Phase::Live | Phase::Joining) {
            column = column.child(
                div().flex().items_center().gap(rems(0.5)).child(
                    button("party-leave", "Quitter", ButtonVariant::Secondary, cx)
                        .child(icon(IconName::LogOut).size(rems(0.75)))
                        .on_click(cx.listener(|this, _, _, cx| this.leave(cx))),
                ),
            );
        }
        if let Some(error) = &view.error {
            column = column.child(error_line(&error.clone().into(), cx));
        }
        column.into_any_element()
    }
}

/// The party id in what a person pasted: the whole URL (`…/party/<id>`), the
/// path (`/party/<id>`), or the id alone (128 random bits, 22 characters in
/// base64url — the server's own floor for a `resume`).
pub fn party_id(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let rest = match raw.find("/party/") {
        Some(index) => &raw[index + "/party/".len()..],
        None => raw,
    };
    // One path segment, never a path: a query, a fragment or a trailing slash
    // after the id is dropped, and anything with a `.` or a `/` in it is not an
    // id at all.
    let candidate = rest.split(['?', '#', '/']).next().unwrap_or("").trim();
    let valid = candidate.len() >= 22
        && candidate.len() <= 64
        && candidate
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| candidate.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pasted_link_gives_back_the_partys_id() {
        let id = "AbCdEfGhIjKlMnOpQrStUv";
        assert_eq!(
            party_id(&format!("https://neezer.example/party/{id}")),
            Some(id.to_owned())
        );
        assert_eq!(party_id(&format!("/party/{id}")), Some(id.to_owned()));
        assert_eq!(party_id(&format!("  {id}  ")), Some(id.to_owned()));
        assert_eq!(
            party_id(&format!("https://x/party/{id}?l=abc")),
            Some(id.to_owned())
        );
        // Hostile or too short: nothing becomes a route.
        assert_eq!(party_id(""), None);
        assert_eq!(party_id("party"), None);
        assert_eq!(party_id("../etc/passwd"), None);
        assert_eq!(party_id("/party/../../etc/passwd"), None);
        // The id is the whole segment: a trailing slash or a query drops off.
        assert_eq!(
            party_id("AbCdEfGhIjKlMnOpQrStUv/"),
            Some("AbCdEfGhIjKlMnOpQrStUv".to_owned())
        );
    }
}
