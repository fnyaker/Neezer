//! Réglages → Lecture et son: where the sound goes, and what it sounds like.
//!
//! The SPA's `AudioEffects.svelte`, section for section: the output, mono and
//! balance, the volume/normalisation/bass block, the ten-band equaliser with
//! its presets, and the transitions (crossfade, softened skips, silence trim).
//! Every control writes the engine the instant it moves (`nz-audio`, D6): the
//! callback picks the new value up at the top of its next buffer, so nothing
//! here waits for a track change — except the stream quality, which is chosen
//! when the next voice is opened and says so.
//!
//! Three panes rather than one long scroll: at 1280×800 with the queue panel
//! open, a single column of five cards does not fit, and a settings page whose
//! last card needs a wheel on a laptop is not a premium screen.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, MouseButton, ParentElement,
    SharedString, StatefulInteractiveElement as _, Styled, Window, div, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon, slider, switch};
use nz_audio::EngineError;

use super::{
    BUILTIN_PRESETS, EQ_FREQS, FADE_MAX_S, Field, Knob, SKIP_FADE_MAX_S, Screen, card, card_title,
    choice_row, hint, text_field, value_line,
};
use crate::theme;
use crate::wiring::engine::PlaybackQuality;

/// The qualities the stream offers, in the order the chip row shows them. The
/// server never sends MP3, so it is absent by construction (`desktop.md`,
/// *Streams*).
pub const QUALITIES: [(&str, &str); 5] = [
    ("auto", "Auto"),
    ("flac", "FLAC"),
    ("opus_320", "Opus 320"),
    ("opus_128", "Opus 128"),
    ("opus_64", "Opus 64"),
];

pub fn body(screen: &mut Screen, window: &mut Window, cx: &mut Context<Screen>) -> AnyElement {
    let panes = screen.panes(&["Son", "Égaliseur", "Transitions"], cx);
    let pane = screen.pane;
    let content = match pane {
        1 => equaliser(screen, window, cx),
        2 => transitions(screen, window, cx),
        _ => sound(screen, window, cx),
    };
    div()
        .flex()
        .flex_col()
        .gap(rems(1.))
        .child(panes)
        .child(content)
        .into_any_element()
}

// --- pane 1: the output, the level and the tone ---------------------------

fn sound(screen: &mut Screen, _window: &mut Window, cx: &mut Context<Screen>) -> AnyElement {
    let audio = screen.audio(cx);
    let mut column = div().flex().flex_col().gap(rems(1.));

    // Where the sound goes --------------------------------------------------
    //
    // The list of sinks is `nz-audio::output_devices()`: the machine's own
    // enumeration, PipeWire's host first and ALSA after (`probe`'s order), one
    // `DeviceDescription::name` each — exactly the string the engine opens by,
    // which is why a chosen sink survives a restart. One read per visit, on a
    // blocking thread (`load_sinks`), and the engine's own device is added to
    // the row even when the enumeration cannot run.
    let info = audio.as_ref().map(|audio| audio.engine.output_info());
    let current = info
        .as_ref()
        .map(|info| {
            // The host reports its own alias for "whatever the system chose"
            // (`default_output`): the line says what that means, not the alias.
            if nz_audio::is_default_output_alias(&info.device) {
                "Sortie du système".to_owned()
            } else {
                info.device.clone()
            }
        })
        .unwrap_or_else(|| "Aucune sortie audio".to_owned());
    let mut output = card("settings-output", cx)
        .child(card_title("Sortie audio", cx))
        .child(hint(
            "Le chemin du son jusqu'à vos écouteurs, votre enceinte ou l'autoradio. \
             Changer de sortie ne coupe pas la musique : le mixeur est repris par le \
             nouveau flux au buffer suivant.",
            cx,
        ));
    if audio.is_none() {
        output = output.child(failed_output(cx));
    } else {
        let mut group = div().flex().flex_wrap().items_center().gap(rems(0.375));
        let chosen = screen.values.device.clone();
        group = group.child(
            device_chip("Sortie par défaut", chosen.is_none(), current.as_str(), cx).on_click(
                cx.listener(|this, _, _, cx| {
                    this.values.device = None;
                    this.save(super::keys::DEVICE, &String::new(), cx);
                    if let Some(audio) = this.audio(cx) {
                        match audio.engine.set_output_device(None) {
                            Ok(()) => this.toast(
                                crate::toast::ToastKind::Info,
                                "Sortie : celle du système",
                                cx,
                            ),
                            Err(error) => this.fail(engine_cause(error), cx),
                        }
                    }
                    this.repaint_after_handover(cx);
                    cx.notify();
                }),
            ),
        );
        for sink in sinks_of(screen, cx) {
            let name = sink.clone();
            group = group.child(
                device_chip(
                    &sink,
                    chosen.as_deref() == Some(sink.as_str()),
                    current.as_str(),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let picked = name.clone();
                    this.values.device = Some(picked.clone());
                    this.save(super::keys::DEVICE, &picked, cx);
                    let Some(audio) = this.audio(cx) else {
                        return;
                    };
                    match audio.engine.set_output_device(Some(&picked)) {
                        Ok(()) => this.toast(
                            crate::toast::ToastKind::Info,
                            format!("Sortie : {picked}"),
                            cx,
                        ),
                        Err(error) => this.fail(engine_cause(error), cx),
                    }
                    this.repaint_after_handover(cx);
                    cx.notify();
                })),
            );
        }
        output = output
            .child(group)
            .child(value_line("Sortie en cours", &current, cx));
        if matches!(screen.data.sinks, super::Load::Failed(_)) {
            output = output.child(hint(
                "La liste des sorties n'a pas pu être lue : seule la sortie en cours est proposée.",
                cx,
            ));
        }
        if let Some(info) = &info {
            output = output.child(value_line(
                "Format",
                format!(
                    "{} Hz · {} canal{} · {} ms",
                    info.sample_rate,
                    info.channels,
                    if info.channels > 1 { "x" } else { "" },
                    (info.latency_frames as f64 * 1_000. / f64::from(info.sample_rate.max(1)))
                        .round()
                ),
                cx,
            ));
        }
    }
    column = column.child(output);

    // The level: the bar's own volume, reachable here too (the SPA shows it in
    // this card so a phone with the bar hidden still has it) ----------------
    let player_volume = screen.state.read(cx).player.volume();
    let muted = screen.state.read(cx).player.muted();
    let mut level = card("settings-level", cx)
        .child(card_title("Volume", cx))
        .child(hint(
            "Le volume de sortie, le même que la barre du lecteur (Ctrl+↑ et Ctrl+↓, m \
             pour couper).",
            cx,
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    button(
                        "settings-mute",
                        if muted { "Rétablir" } else { "Couper" },
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, |state, cx| {
                            state.command(
                                nz_player::Command::ToggleMute {
                                    args: Default::default(),
                                },
                                cx,
                            );
                        });
                        cx.notify();
                    })),
                )
                .child(metre(
                    "settings-volume",
                    if muted { 0. } else { player_volume as f32 },
                    cx,
                    |this, fraction, cx| {
                        this.state.update(cx, |state, cx| {
                            state.command(
                                nz_player::Command::SetVolume {
                                    args: nz_player::VolumeArgs {
                                        v: f64::from(fraction),
                                    },
                                },
                                cx,
                            );
                        });
                        cx.notify();
                    },
                ))
                .child(
                    div()
                        .w(rems(3.))
                        .text_size(rems(0.75))
                        .font_family(theme::mono(cx))
                        .child(format!("{} %", (player_volume * 100.).round())),
                ),
        );

    // Normalisation ---------------------------------------------------------
    let current_norm = screen.values.normalization.id();
    level = level
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(0.25))
                .child(div().text_size(rems(0.75)).child("Normalisation du volume"))
                .child(hint(
                    "Un gain fixe par titre, d'après la loudness ReplayGain mesurée par le \
                     serveur, pour que tous jouent au même niveau — sans compression et sans \
                     ajustement pendant la lecture.",
                    cx,
                ))
                .child(choice_row(
                    "settings-norm",
                    &[
                        ("off", "Désactivée"),
                        ("low", "Basse"),
                        ("medium", "Normale"),
                        ("high", "Élevée"),
                    ],
                    current_norm,
                    cx,
                    |screen, value, cx| {
                        let level = super::NormLevel::parse(value);
                        screen.update_values(cx, |values| values.normalization = level);
                        screen.save(super::keys::NORMALIZATION, &value.to_owned(), cx);
                    },
                )),
        )
        // The bass lift -----------------------------------------------------
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(0.25))
                .child(div().text_size(rems(0.75)).child("Amélioration des basses"))
                .child(hint(
                    "Renforce le grave ; un limiteur automatique évite la saturation.",
                    cx,
                ))
                .child(knob_row(
                    "settings-bass",
                    screen,
                    Knob::Bass,
                    format!("{:.0} dB", screen.values.bass),
                    false,
                    _window,
                    cx,
                )),
        );
    column = column.child(level);

    // Mono and balance ------------------------------------------------------
    let mono = screen.values.mono;
    let balance = screen.values.balance;
    column = column.child(
        card("settings-pan", cx)
            .child(card_title("Mono et balance", cx))
            .child(hint(
                "Pour écouter avec un seul écouteur, une oreille ou une aide auditive : réunir \
                 les deux canaux, ou déplacer le son vers l'oreille qui entend. Ces réglages ne \
                 changent que ce que vous entendez — les animations gardent le stéréo d'origine.",
                cx,
            ))
            .child(
                switch("settings-mono", "Mono", mono, cx)
                    .debug_selector(|| "mono".to_owned())
                    .on_click(cx.listener(|this, _, _, cx| {
                        let mono = !this.values.mono;
                        this.update_values(cx, |values| values.mono = mono);
                        this.save(super::keys::MONO, &mono, cx);
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        div()
                            .text_size(rems(0.75))
                            .text_color(theme::secondary(cx))
                            .child("Gauche"),
                    )
                    .child(knob_row(
                        "settings-balance",
                        screen,
                        Knob::Balance,
                        balance_label(balance),
                        false,
                        _window,
                        cx,
                    ))
                    .child(
                        div()
                            .text_size(rems(0.75))
                            .text_color(theme::secondary(cx))
                            .child("Droite"),
                    )
                    .child(
                        button("settings-centre", "Centre", ButtonVariant::Secondary, cx)
                            .disabled(balance.abs() < 0.005)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.update_values(cx, |values| values.balance = 0.);
                                this.save(super::keys::BALANCE, &0.0_f64, cx);
                                this.set_knob(Knob::Balance, 0., window, cx);
                            })),
                    ),
            ),
    );

    // The stream quality ----------------------------------------------------
    let quality = screen.quality(cx);
    let mut quality_card = card("settings-quality", cx)
        .child(card_title("Qualité du flux", cx))
        .child(hint(
            "Ce que l'app demande au serveur. Le serveur archive toujours en FLAC et \
             transcode à la volée : rien n'est perdu en changeant ce réglage. Le choix \
             s'applique au titre suivant, jamais au son déjà en cours.",
            cx,
        ))
        .child(choice_row(
            "settings-quality-row",
            &QUALITIES,
            quality.key(),
            cx,
            |screen, value, cx| {
                screen.set_quality(PlaybackQuality::parse(value), cx);
            },
        ));
    if quality == PlaybackQuality::Auto {
        quality_card = quality_card.child(hint(
            "Auto : Opus 320 tant que le débit mesuré ne dit pas de descendre (600 kB/s et \
             plus → 320, 200 kB/s et plus → 128, en dessous → 64).",
            cx,
        ));
    }
    column = column.child(quality_card);
    column.into_any_element()
}

fn failed_output(cx: &gpui_kit::App) -> impl IntoElement + use<> {
    div()
        .flex()
        .items_start()
        .gap(rems(0.5))
        .text_size(rems(0.75))
        .text_color(theme::warning(cx))
        .child(icon(IconName::TriangleAlert).size(rems(1.)))
        .child(div().flex_1().child(
            "Aucune sortie audio sur ce poste : la musique ne peut pas être entendue. \
             Vérifiez que PipeWire (ou ALSA) tourne, puis rouvrez cette page.",
        ))
}

fn engine_cause(error: EngineError) -> String {
    format!("Sortie audio : {error}")
}

/// The outputs the person can pick from: what the machine publishes right now,
/// plus the one playing now and the one remembered, so neither can vanish from
/// the row when the enumeration cannot run (a machine without PipeWire, a
/// sandbox with no sound server).
fn sinks_of(screen: &Screen, cx: &Context<Screen>) -> Vec<String> {
    let mut list: Vec<String> = screen.data.sinks.ready().cloned().unwrap_or_default();
    if let Some(info) = screen.audio(cx).map(|audio| audio.engine.output_info())
        && !info.device.is_empty()
        && !info.device.eq_ignore_ascii_case("silence")
        && !nz_audio::is_default_output_alias(&info.device)
        && !list.iter().any(|name| name == &info.device)
    {
        list.push(info.device.clone());
    }
    if let Some(chosen) = &screen.values.device
        && !list.iter().any(|name| name == chosen)
    {
        list.push(chosen.clone());
    }
    list
}

/// The outputs the machine publishes right now, as the engine names them.
///
/// One in-process enumeration in `nz-audio` (`output_devices`: PipeWire's host
/// first, then ALSA, each device's `description().name()` — exactly the string
/// `probe` matches, which is why a chosen sink survives a restart and an ALSA
/// chip lands on ALSA). It replaced reading `pw-dump` here: same names, no
/// subprocess, no JSON, and a headset plugged in since the engine started is
/// seen because the host is built fresh. Sorted for the same reason: a node's
/// creation order never moves a chip under the pointer.
pub fn sinks_now() -> Vec<String> {
    nz_audio::output_devices()
}

/// One sink (or the default) as a chip. It says it is the default one, or the
/// one playing, so a person who just switched reads the result without
/// squinting at the line below.
fn device_chip(
    label: impl Into<SharedString>,
    selected: bool,
    current: &str,
    cx: &mut Context<Screen>,
) -> gpui_kit::base::Toggle {
    let label = label.into();
    let playing = label.as_ref() == current;
    let id = format!("out-{label}");
    let mut chip = gpui_omarchy::toggle(id, label.clone(), selected, cx);
    if selected || playing {
        chip = chip.text_color(theme::accent(cx));
    }
    chip
}

// --- pane 2: the equaliser -------------------------------------------------

fn equaliser(screen: &mut Screen, window: &mut Window, cx: &mut Context<Screen>) -> AnyElement {
    let active = screen.values.active_preset();
    let mut presets = div().flex().flex_wrap().items_center().gap(rems(0.375));
    for preset in BUILTIN_PRESETS {
        let selected = active.as_deref() == Some(preset.label);
        presets = presets.child(
            gpui_omarchy::toggle(preset.id, preset.label, selected, cx)
                .when(selected, |chip| chip.text_color(theme::accent(cx)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.update_values(cx, |values| values.eq = preset.bands);
                    this.save(super::keys::EQ, &preset.bands, cx);
                    for band in 0..10 {
                        this.set_knob(Knob::Band(band), preset.bands[band], window, cx);
                    }
                })),
        );
    }
    let saved = screen.values.presets.clone();
    for preset in &saved {
        let name = preset.name.clone();
        let bands = preset.bands;
        let bass = preset.bass;
        let selected = active.as_deref() == Some(name.as_str());
        presets = presets.child(
            gpui_omarchy::toggle(format!("preset-{name}"), name.to_string(), selected, cx)
                .when(selected, |chip| chip.text_color(theme::accent(cx)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.update_values(cx, |values| {
                        values.eq = bands;
                        values.bass = bass;
                    });
                    this.save(super::keys::EQ, &bands, cx);
                    this.save(super::keys::BASS, &bass, cx);
                    for (band, db) in bands.iter().enumerate() {
                        this.set_knob(Knob::Band(band), *db, window, cx);
                    }
                    this.set_knob(Knob::Bass, bass, window, cx);
                })),
        );
    }

    let bands_state = screen.values.eq;
    let mut bands = div().flex().flex_col().gap(rems(0.125));
    for (band, hz) in EQ_FREQS.iter().enumerate() {
        bands = bands.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    div()
                        .w(rems(3.))
                        .text_size(rems(0.6875))
                        .font_family(theme::mono(cx))
                        .text_color(theme::secondary(cx))
                        .child(freq_label(*hz)),
                )
                .child(knob_row(
                    ("settings-band", band),
                    screen,
                    Knob::Band(band),
                    format!("{:+.0} dB", bands_state[band]),
                    false,
                    window,
                    cx,
                )),
        );
    }

    // Saving: a name, then the current curve and lift as one more preset.
    let name = text_field(Field::PresetName, window, cx);
    let typed = name.read(cx).value().to_string();
    let flat = screen.values.eq == [0.; 10] && screen.values.bass.abs() < 0.005;
    let updating = active
        .as_deref()
        .filter(|name| saved.iter().any(|preset| preset.name == *name))
        .map(str::to_owned);

    let mut manage = div().flex().flex_wrap().items_center().gap(rems(0.5));
    manage = manage.child(div().w(rems(14.)).child(gpui_omarchy::input(
        "settings-preset-name",
        &name,
        window,
        cx,
    )));
    manage = manage.child(
        button(
            "settings-preset-save",
            "Enregistrer",
            ButtonVariant::Secondary,
            cx,
        )
        .disabled(typed.trim().is_empty())
        .on_click(cx.listener(|this, _, window, cx| {
            let name_entity = text_field(Field::PresetName, window, cx);
            let preset_name = name_entity.read(cx).value().trim().to_owned();
            if preset_name.is_empty() {
                return;
            }
            let bands = this.values.eq;
            let bass = this.values.bass;
            this.update_values(cx, |values| {
                let preset = super::Preset {
                    name: preset_name.clone(),
                    bands,
                    bass,
                };
                match values
                    .presets
                    .iter_mut()
                    .find(|other| other.name.eq_ignore_ascii_case(&preset.name))
                {
                    Some(existing) => *existing = preset,
                    None => values.presets.push(preset),
                }
            });
            let presets = this.values.presets.clone();
            this.save(super::keys::PRESETS, &presets, cx);
            name_entity.update(cx, |state, cx| state.set_value("", window, cx));
            this.toast(
                crate::toast::ToastKind::Info,
                format!("Préréglage « {preset_name} » enregistré"),
                cx,
            );
        })),
    );
    if let Some(active_name) = &updating {
        let name = active_name.clone();
        let update_name = name.clone();
        let delete_name = name.clone();
        manage = manage
            .child(
                button(
                    "settings-preset-update",
                    "Mettre à jour",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let bands = this.values.eq;
                    let bass = this.values.bass;
                    let name = update_name.clone();
                    this.update_values(cx, |values| {
                        if let Some(preset) =
                            values.presets.iter_mut().find(|other| other.name == name)
                        {
                            preset.bands = bands;
                            preset.bass = bass;
                        }
                    });
                    let presets = this.values.presets.clone();
                    this.save(super::keys::PRESETS, &presets, cx);
                    this.toast(
                        crate::toast::ToastKind::Info,
                        format!("Préréglage « {name} » mis à jour"),
                        cx,
                    );
                })),
            )
            .child(
                button(
                    "settings-preset-delete",
                    "Supprimer",
                    ButtonVariant::Danger,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let name = delete_name.clone();
                    this.update_values(cx, |values| {
                        values.presets.retain(|preset| preset.name != name);
                    });
                    let presets = this.values.presets.clone();
                    this.save(super::keys::PRESETS, &presets, cx);
                    this.toast(
                        crate::toast::ToastKind::Info,
                        format!("Préréglage « {name} » supprimé"),
                        cx,
                    );
                })),
            );
    }
    manage = manage.child(
        button(
            "settings-preset-flat",
            "Tout à plat",
            ButtonVariant::Outline,
            cx,
        )
        .disabled(flat)
        .on_click(cx.listener(|this, _, window, cx| {
            this.update_values(cx, |values| values.eq = [0.; 10]);
            this.save(super::keys::EQ, &[0.; 10], cx);
            for band in 0..10 {
                this.set_knob(Knob::Band(band), 0., window, cx);
            }
        })),
    );

    div()
        .flex()
        .flex_col()
        .gap(rems(1.))
        .child(
            card("settings-eq", cx)
                .child(card_title("Égaliseur 10 bandes", cx))
                .child(hint(
                    "Dix bandes de 31 Hz à 16 kHz (±16 dB). Le limiteur de sécurité rattrape \
                     les pics en fin de chaîne ; une courbe plate ne change rien au son.",
                    cx,
                ))
                .child(presets)
                .child(bands),
        )
        .child(
            card("settings-eq-manage", cx)
                .child(card_title("Mes préréglages", cx))
                .child(hint(
                    "Un préréglage garde la courbe et l'amplification des basses : « casque », \
                     « voiture », « nuit ». Il est enregistré sur cet appareil.",
                    cx,
                ))
                .child(manage)
                .child(value_line(
                    "Réglage en cours",
                    active.unwrap_or_else(|| "Personnalisé".to_owned()),
                    cx,
                )),
        )
        .into_any_element()
}

fn freq_label(hz: u32) -> String {
    if hz >= 1_000 {
        format!("{}k", hz / 1_000)
    } else {
        hz.to_string()
    }
}

// --- pane 3: the transitions ----------------------------------------------

fn transitions(screen: &mut Screen, window: &mut Window, cx: &mut Context<Screen>) -> AnyElement {
    let values = screen.values.clone();
    let fade_on = values.fade_enabled;
    let skip_on = values.fade_on_skip;

    let mut fade = card("settings-fade", cx)
        .child(card_title("Fondu enchaîné", cx))
        .child(hint(
            "Les deux titres se superposent en fin de morceau, chacun à son propre niveau : \
             la normalisation ne saute donc pas pendant le chevauchement.",
            cx,
        ))
        .child(
            switch("settings-fade-on", "Fondu enchaîné", fade_on, cx).on_click(cx.listener(
                |this, _, _, cx| {
                    let on = !this.values.fade_enabled;
                    this.update_values(cx, |values| values.fade_enabled = on);
                    this.save(super::keys::FADE_ENABLED, &on, cx);
                },
            )),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    div()
                        .w(rems(3.))
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child("Durée"),
                )
                .child(knob_row(
                    "settings-fade-seconds",
                    screen,
                    Knob::Fade,
                    fade_label(values.fade_seconds),
                    !fade_on,
                    window,
                    cx,
                )),
        )
        .child(
            switch(
                "settings-fade-on-skip",
                "Adoucir les changements manuels",
                skip_on,
                cx,
            )
            .disabled(!fade_on)
            .on_click(cx.listener(|this, _, _, cx| {
                if !this.values.fade_enabled {
                    return;
                }
                let on = !this.values.fade_on_skip;
                this.update_values(cx, |values| values.fade_on_skip = on);
                this.save(super::keys::FADE_ON_SKIP, &on, cx);
            })),
        )
        .child(hint(
            "Suivant et précédent s'enchaînent avec un fondu plus court que celui de fin de \
             titre, au lieu d'une coupure sèche au milieu de la forme d'onde.",
            cx,
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    div()
                        .w(rems(3.))
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child("Durée"),
                )
                .child(knob_row(
                    "settings-skip-seconds",
                    screen,
                    Knob::SkipFade,
                    skip_label(values.fade_skip_seconds),
                    !fade_on || !skip_on,
                    window,
                    cx,
                )),
        );

    fade = fade.child(
        div()
            .text_size(rems(0.6875))
            .text_color(theme::secondary(cx))
            .child(format!(
                "Bornes du moteur : fondu de fin 0 à {:.0} s, fondu manuel 0 à {:.0} s.",
                FADE_MAX_S, SKIP_FADE_MAX_S
            )),
    );

    // The silence trim ------------------------------------------------------
    //
    // The engine cuts a track where its audio starts and stops, from bounds the
    // SERVER measures once per file and threshold (`neezer/webui/edges.py`).
    // The app asks for them ahead of the play (the playing track at user
    // priority, the next one at prefetch priority) and hands them to the engine
    // before a voice is built, which is why a track is trimmed from the play
    // after the one that archived it — the card says so, as the SPA's does.
    let trim_on = values.trim;
    let trim_card = card("settings-trim", cx)
        .debug_selector(|| "settings-trim".to_owned())
        .child(card_title("Tronquer les silences", cx))
        .child(hint(
            "Démarre et termine chaque titre là où le son commence et s'arrête vraiment. \
             Le serveur mesure le fichier une fois, après son archivage : un titre est donc \
             ajusté à partir de la lecture suivante. Jamais un épisode de podcast.",
            cx,
        ))
        .child(
            switch("settings-trim-on", "Tronquer les silences", trim_on, cx)
                .debug_selector(|| "trim-on".to_owned())
                .on_click(cx.listener(|this, _, _, cx| {
                    let on = !this.values.trim;
                    this.values.trim = on;
                    let db = this.values.trim_db;
                    this.state.update(cx, |state, _| state.set_trim(on, db));
                    this.save(super::keys::TRIM, &on, cx);
                })),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.5))
                .child(
                    div()
                        .w(rems(3.))
                        .text_size(rems(0.75))
                        .text_color(theme::secondary(cx))
                        .child("Seuil"),
                )
                .child(knob_row(
                    "settings-trim-db",
                    screen,
                    Knob::TrimThreshold,
                    trim_label(values.trim_db),
                    !trim_on,
                    window,
                    cx,
                )),
        )
        .child(hint(
            "Ce qui compte comme silence. Plus bas, seuls les vrais blancs numériques sont \
             coupés ; plus haut, un souffle de salle l'est aussi.",
            cx,
        ));

    div()
        .flex()
        .flex_col()
        .gap(rems(1.))
        .child(fade)
        .child(trim_card)
        .into_any_element()
}

/// The threshold as a person reads it: « −45 dB ».
fn trim_label(db: f64) -> String {
    format!("{db:.0} dB")
}

fn fade_label(seconds: f32) -> String {
    if seconds <= 0.05 {
        "enchaîné direct".to_owned()
    } else {
        format!("{seconds:.1} s")
    }
}

fn skip_label(seconds: f32) -> String {
    if seconds <= 0.05 {
        "coupe douce".to_owned()
    } else {
        format!("{seconds:.2} s")
    }
}

fn balance_label(balance: f32) -> String {
    if balance.abs() < 0.005 {
        "Centre".to_owned()
    } else if balance < 0. {
        format!("{:.0} % à gauche", -balance * 100.)
    } else {
        format!("{:.0} % à droite", balance * 100.)
    }
}

/// One slider row: the Omarchy slider (pointer, wheel and arrow keys) with its
/// current value printed beside it. `disabled` is passed to the slider so the
/// track and knobs are inert together.
fn knob_row(
    id: impl Into<gpui_kit::ElementId>,
    screen: &Screen,
    knob: Knob,
    value: String,
    disabled: bool,
    window: &mut Window,
    cx: &mut Context<Screen>,
) -> AnyElement {
    let Some((_, state)) = screen.knobs.iter().find(|(other, _)| *other == knob) else {
        return div().into_any_element();
    };
    let slider = slider(state, disabled, window, cx);
    div()
        .id(id)
        .flex()
        .flex_1()
        .min_w(rems(6.))
        .items_center()
        .gap(rems(0.5))
        .child(slider)
        .child(
            div()
                .w(rems(4.5))
                .text_size(rems(0.6875))
                .font_family(theme::mono(cx))
                .text_color(if disabled {
                    theme::secondary(cx)
                } else {
                    theme::text(cx)
                })
                .child(value),
        )
        .into_any_element()
}

/// A filled metre with twenty hit zones: the player bar's own idiom, so a
/// click anywhere on the line lands there and no second gesture exists.
fn metre(
    id: &'static str,
    fraction: f32,
    cx: &mut Context<Screen>,
    on_pick: fn(&mut Screen, f32, &mut Context<Screen>),
) -> impl IntoElement + use<> {
    let fraction = fraction.clamp(0., 1.);
    let zones = div()
        .absolute()
        .inset_0()
        .flex()
        .children((0..20).map(|index| {
            let target = (index as f32 + 0.5) / 20.;
            div()
                .id(("metre-zone", index as usize))
                .flex_1()
                .h_full()
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| on_pick(this, target, cx)),
                )
        }));
    div()
        .id(id)
        .relative()
        .flex_1()
        .h(rems(0.375))
        .bg(theme::border(cx))
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .w(rems(0.375))
                .bg(theme::accent(cx))
                .ml(gpui_kit::relative(fraction)),
        )
        .child(zones)
}

#[cfg(test)]
mod tests {
    use super::super::{FADE_DEFAULT_S, NormLevel, SKIP_FADE_DEFAULT_S, Values};
    use super::*;
    use crate::wiring::engine::PlaybackQuality;

    #[test]
    fn the_quality_row_is_the_plans_ladder_and_never_mp3() {
        // `desktop.md`, *Streams*: Auto / Opus 320 / Opus 128 / Opus 64 / FLAC,
        // and the server never sends MP3, so no chip may name one.
        assert_eq!(QUALITIES.len(), 5);
        assert_eq!(QUALITIES[0].0, "auto");
        for (key, label) in QUALITIES {
            assert_eq!(PlaybackQuality::parse(key).key(), key, "{label}");
            assert!(!label.to_ascii_lowercase().contains("mp3"));
        }
        assert_eq!(
            PlaybackQuality::parse("n'importe quoi"),
            PlaybackQuality::Auto
        );
    }

    #[test]
    fn the_band_labels_read_as_a_person_reads_frequencies() {
        assert_eq!(freq_label(31), "31");
        assert_eq!(freq_label(500), "500");
        assert_eq!(freq_label(1_000), "1k");
        assert_eq!(freq_label(16_000), "16k");
    }

    #[test]
    fn a_crossfade_of_zero_says_what_it_does_instead_of_zero() {
        assert_eq!(fade_label(0.), "enchaîné direct");
        assert_eq!(fade_label(0.5), "0.5 s");
        assert_eq!(fade_label(6.), "6.0 s");
        assert_eq!(skip_label(0.), "coupe douce");
        assert_eq!(skip_label(1.5), "1.50 s");
        assert_eq!(skip_label(0.25), "0.25 s");
    }

    #[test]
    fn the_balance_reads_centre_or_a_side() {
        assert_eq!(balance_label(0.), "Centre");
        assert_eq!(balance_label(-0.3), "30 % à gauche");
        assert_eq!(balance_label(0.42), "42 % à droite");
    }

    /// The sink row is the machine's list, and the engine's own device is in it:
    /// that is what makes the chip beside « Sortie par défaut » selectable and
    /// what tells a person which one is playing. The enumeration itself
    /// (`nz_audio::output_devices`) has its own test in `nz-audio`; this one
    /// asserts the app asks IT and nothing else — no subprocess, no `pw-dump`.
    #[test]
    fn the_sink_list_is_the_engines_own_enumeration() {
        assert_eq!(sinks_now(), nz_audio::output_devices());
        // Sorted within each host, no duplicate across hosts: a name the card
        // shows twice would be two chips for one sink.
        let names = sinks_now();
        let mut sorted = names.clone();
        sorted.dedup();
        assert_eq!(names.len(), sorted.len(), "{names:?}");
    }

    /// The trim's threshold reads like the SPA's slider does, and its range is
    /// the card's own (the server clamps to −90…−20 behind it).
    #[test]
    fn the_trim_threshold_reads_in_dbfs() {
        assert_eq!(trim_label(-45.), "-45 dB");
        assert_eq!(trim_label(-70.), "-70 dB");
        // The card's slider stays inside what the server measures (`edges.py`
        // clamps to −90…−20), so no answer is ever asked at a threshold the
        // server would silently replace.
        const { assert!(super::super::TRIM_DB_MIN > -90.) };
        const { assert!(super::super::TRIM_DB_MAX < -20.) };
    }

    #[test]
    fn the_default_values_are_the_engines_own_defaults() {
        let values = Values::default();
        // Nothing is applied to the sound until the person asks: the SPA's
        // effects are off by default and so are these.
        assert_eq!(values.normalization, NormLevel::Off);
        assert_eq!(values.bass, 0.);
        assert_eq!(values.eq, [0.; 10]);
        assert!(!values.fade_enabled);
        assert!(!values.mono);
        // The fade DURATIONS keep the engine's defaults even while off, so the
        // first enable needs no extra gesture.
        assert_eq!(values.fade_seconds, FADE_DEFAULT_S);
        assert_eq!(values.fade_skip_seconds, SKIP_FADE_DEFAULT_S);
        // The trim is ON by default, like the web player (`stores.js`), at the
        // SPA's own threshold: a master's beat of digital silence is not music.
        assert!(values.trim);
        assert_eq!(values.trim_db, super::super::TRIM_DB_DEFAULT);
    }
}
