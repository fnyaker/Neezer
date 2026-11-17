//! One episode's panel (lot D11): its chapters, its transcript, its description,
//! its facts, and the two gestures that belong to the episode itself — seek,
//! and drop a marker here.
//!
//! `webapp/src/components/PodcastEpisodePanel.svelte` is the reference. Two of
//! its decisions are kept:
//!
//! - **Everything is STACKED, nothing hides behind a tab.** The SPA uses tabs
//!   (Chapitres / Transcription) because a phone has no width; a desktop window
//!   has, and hiding the chapters of the episode being played behind a control is
//!   how a panel becomes a menu. The order is a listener's: chapters (where am
//!   I), the words, the description, the facts.
//! - **The panel owns the episode's identity, not the transport.** Play, next and
//!   queue stay the list's gestures and the transport's keys; the panel seeks
//!   inside the episode it shows and asks for nothing else.
//!
//! [`entry`] is the ONLY conversion from an episode to a queue row. A podcast row
//! must carry `podcast: true` (the engine's speed/trim guard) and its
//! `channel_id` (the show's speed key): losing either turns every episode into a
//! song at 1×. `state::entry_from_track` cannot do it — a `Track` has no channel.

use gpui_kit::base::input::InputState;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Div, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled, Window, div,
    rems,
};
use gpui_omarchy::{IconName, icon};
use nz_model::{Episode, EpisodeTranscriptInfo, Marker, PodcastPosition};

use crate::state::AppState;
use crate::theme;
use crate::widgets::chapters::{self, Chapters};
use crate::widgets::markers::{self, Markers};
use crate::widgets::track_list::duration_label;
use crate::widgets::transcript_panel::{self as transcript, Ask as TranscriptAsk};

/// What the panel asks the screen for. Network and transport stay the screen's.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// Play, or seek, the episode shown, from `seconds`.
    Seek(f64),
    /// Ask the server for this episode's chapters.
    LoadChapters,
    /// A `Réessayer` of a failed chapter read.
    RetryChapters,
    /// Put a marker at `position_ms` (milliseconds: a marker's unit).
    AddMarker(u32),
    DeleteMarker(Marker),
    ToggleDescription,
    ToggleAllChapters,
    /// The transcript panel's own verbs, forwarded unchanged.
    Transcript(TranscriptAsk),
    /// Fold the panel away (Échap does the same; a mouse needs a control).
    Close,
}

/// Everything the panel needs to know about the world, in one struct (a dozen
/// positional arguments is how a call site drifts).
pub struct View<'a> {
    pub chapters: &'a Chapters,
    pub markers: &'a Markers,
    /// Where this episode stands for this person, if anywhere.
    pub progress: Option<PodcastPosition>,
    pub is_admin: bool,
    /// This episode is the one the player has loaded.
    pub is_current: bool,
    pub playing: bool,
    /// Seconds into the episode, when it is the current one.
    pub playhead_s: f64,
}

/// One episode, open in the panel.
pub struct EpisodePanel {
    pub episode: Episode,
    pub transcript: transcript::Panel,
    pub description_open: bool,
    /// "Voir les N chapitres": the inline list shows `chapters::MAX_INLINE`
    /// until this is set.
    pub chapters_expanded: bool,
}

impl EpisodePanel {
    pub fn new(episode: Episode) -> Self {
        let transcript = transcript::Panel::new(
            episode.id.as_str(),
            Some(episode.archived),
            episode.duration,
        );
        Self {
            episode,
            transcript,
            description_open: false,
            chapters_expanded: false,
        }
    }

    /// The queue key of this episode: its row's own uuid.
    pub fn key(&self) -> &str {
        self.episode.id.as_str()
    }
}

/// The queue key of an episode: its own row uuid (`trackkey.js#trackKey`, where
/// an episode always has a row).
pub fn key(episode: &Episode) -> String {
    episode.id.as_str().to_owned()
}

/// An episode as a queue row (`commands.js#slimTrack` for a podcast).
pub fn entry(episode: &Episode) -> nz_player::Entry {
    let show_title = episode
        .album
        .as_ref()
        .map(|album| album.title.clone())
        .or_else(|| episode.artist.as_ref().map(|artist| artist.name.clone()))
        .unwrap_or_default();
    nz_player::Entry {
        id: episode.id.as_str().to_owned(),
        deezer_id: None,
        title: episode.title.clone(),
        duration: episode.duration,
        explicit: Some(episode.explicit),
        local: None,
        podcast: Some(true),
        gain: None,
        unavailable: Some(episode.unavailable),
        archived: Some(episode.archived),
        published: episode
            .published
            .filter(|seconds| *seconds > 0)
            .map(|seconds| seconds.to_string()),
        channel_id: Some(episode.channel_id.as_str().to_owned()),
        cover: episode
            .cover
            .clone()
            .or_else(|| episode.album.as_ref().and_then(|album| album.cover.clone())),
        display_artist: Some(show_title.clone()).filter(|title| !title.is_empty()),
        artist: episode
            .artist
            .as_ref()
            .map(|artist| nz_player::EntryArtist {
                id: None,
                deezer_id: (!artist.deezer_id.is_empty())
                    .then(|| artist.deezer_id.as_str().to_owned()),
                name: artist.name.clone(),
                role: None,
            }),
        artists: Vec::new(),
        album: episode.album.as_ref().map(|album| nz_player::EntryAlbum {
            id: None,
            deezer_id: (!album.deezer_id.is_empty()).then(|| album.deezer_id.clone()),
            title: album.title.clone(),
            cover: album.cover.clone(),
        }),
    }
}

/// What a row (and the panel) says about a half-heard episode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resume {
    pub done: bool,
    /// 0..1 of the episode heard.
    pub pct: f32,
    /// Seconds left.
    pub left: u32,
}

/// The SPA's `resumeOf`: nothing before the first second, « Terminé » when the
/// server said so, a bar in between.
pub fn resume_of(position: Option<&PodcastPosition>, duration: u32) -> Option<Resume> {
    let position = position?;
    if position.finished {
        return Some(Resume {
            done: true,
            pct: 1.0,
            left: 0,
        });
    }
    let at = position.position;
    let total = if position.duration > 0 {
        position.duration
    } else {
        duration
    };
    if at == 0 || total == 0 {
        return None;
    }
    Some(Resume {
        done: false,
        pct: (at as f32 / total as f32).clamp(0.0, 1.0),
        left: total.saturating_sub(at),
    })
}

/// « Reprendre · il reste 12 min », or « Terminé ».
pub fn resume_label(resume: Resume) -> String {
    if resume.done {
        return "Terminé".to_owned();
    }
    let left = resume.left;
    if left >= 3_600 {
        format!(
            "Reprendre · il reste {} h {:02}",
            left / 3_600,
            (left % 3_600) / 60
        )
    } else if left >= 60 {
        format!("Reprendre · il reste {} min", left.div_ceil(60))
    } else {
        format!("Reprendre · il reste {left} s")
    }
}

/// « 12 nov. 2026 » — the SPA's `fmtDate`, without a date crate (a podcast date
/// is a calendar date, not a time-zone question).
pub fn date_label(unix: i64) -> String {
    if unix <= 0 {
        return String::new();
    }
    const MONTHS: [&str; 12] = [
        "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
        "déc.",
    ];
    let days = unix.div_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let month = MONTHS
        .get((month as usize).saturating_sub(1))
        .copied()
        .unwrap_or("");
    format!("{day} {month} {year}")
}

/// Howard Hinnant's `civil_from_days`: days since the epoch → (year, month, day).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// What a show row says about an episode's transcript.
pub fn transcript_state_label(info: &EpisodeTranscriptInfo) -> String {
    match info.status.as_str() {
        "ready" => "Transcription".to_owned(),
        "queued" => "En attente".to_owned(),
        "running" | "leased" => format!("{} %", (info.progress * 100.0).round() as u32),
        "stopped" => "Arrêtée".to_owned(),
        "error" => "Échec".to_owned(),
        "local" => "Locale".to_owned(),
        _ => "Transcrire".to_owned(),
    }
}

/// The panel's whole surface.
#[allow(clippy::too_many_arguments)]
pub fn view<V: Render>(
    panel: &mut EpisodePanel,
    entity: Entity<V>,
    state: &Entity<AppState>,
    world: View<'_>,
    field: Option<&Entity<InputState>>,
    window: &mut Window,
    cx: &mut Context<V>,
    on: impl Fn(&mut V, Act, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    let episode = panel.episode.clone();
    let key = episode.id.as_str().to_owned();
    let resume = resume_of(world.progress.as_ref(), episode.duration);
    let head = div()
        .id("episode-panel-head")
        .flex()
        .flex_col()
        .gap(rems(0.1875))
        .child(
            div()
                .flex()
                .items_start()
                .gap(rems(0.375))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(rems(0.875))
                        .font_weight(FontWeight::BOLD)
                        .child(episode.title.clone()),
                )
                .child({
                    // A mouse has no Échap: without this the panel could only be
                    // replaced, never closed, in the wide layout (1280 and 2560).
                    let done = on.clone();
                    div()
                        .id("episode-panel-close")
                        .debug_selector(|| "episode-panel-close".to_owned())
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .w(rems(1.5))
                        .h(rems(1.5))
                        .rounded(rems(0.25))
                        .cursor_pointer()
                        .hover(|style| style.bg(theme::chrome(cx)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            done(this, Act::Close, cx)
                        }))
                        .child(
                            icon(IconName::Close)
                                .size(rems(0.875))
                                .text_color(theme::secondary(cx)),
                        )
                }),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(rems(0.375))
                .text_size(rems(0.6875))
                .text_color(theme::secondary(cx))
                .child(SharedString::from({
                    let mut parts = vec![date_label(episode.published.unwrap_or(0))];
                    if episode.duration > 0 {
                        parts.push(duration_label(episode.duration));
                    }
                    parts.retain(|part| !part.is_empty());
                    parts.join(" · ")
                }))
                .when(!episode.archived, |row| {
                    row.child(div().text_color(theme::warning(cx)).child("non archivé"))
                })
                .when_some(resume, |row, resume| {
                    row.child(if resume.done {
                        div().text_color(theme::success(cx)).child("Terminé")
                    } else {
                        div().child(resume_label(resume))
                    })
                }),
        );

    let mut body = div()
        .id("episode-panel-body")
        .flex()
        .flex_col()
        .gap(rems(0.75))
        .w_full()
        .min_w_0()
        .min_h_0()
        // The scroller must BE the flex item: a block wrapper gave it no
        // definite height, so `overflow_y_scroll` had nothing to scroll and the
        // panel ran under the player bar (1280×800, seen on the bench).
        .flex_1()
        .overflow_y_scroll();

    // -- chapters ---------------------------------------------------------
    let chapter_entry = world.chapters.get(&key);
    let mut section = div().flex().items_center().gap(rems(0.25)).w_full();
    section = section.child(section_head(IconName::List, "Chapitres", cx));
    if let Some(entry) = chapter_entry
        && !entry.chapters.is_empty()
    {
        section = section.child(
            div()
                .text_size(rems(0.6875))
                .font_family(theme::mono(cx))
                .text_color(theme::secondary(cx))
                .child(chapters::count_label(entry.chapters.len() as u32)),
        );
    }
    body = body.child(section);
    match chapter_entry {
        Some(entry) if !entry.chapters.is_empty() => {
            let current = if world.is_current {
                chapters::at(&entry.chapters, world.playhead_s)
            } else {
                None
            };
            let limit = if panel.chapters_expanded {
                entry.chapters.len()
            } else {
                chapters::MAX_INLINE.min(entry.chapters.len())
            };
            let seek = on.clone();
            body = body.child(chapters::list(
                &entry.chapters,
                current,
                limit,
                cx,
                move |this, start, cx| seek(this, Act::Seek(start), cx),
            ));
            let hidden = entry.chapters.len().saturating_sub(limit);
            if hidden > 0 {
                body = body.child(link(
                    "episode-chapters-all",
                    &format!("Voir les {} chapitres", entry.chapters.len()),
                    cx,
                    on.clone(),
                    Act::ToggleAllChapters,
                ));
            } else if panel.chapters_expanded && entry.chapters.len() > chapters::MAX_INLINE {
                body = body.child(link(
                    "episode-chapters-fewer",
                    "Réduire",
                    cx,
                    on.clone(),
                    Act::ToggleAllChapters,
                ));
            }
        }
        Some(entry) if entry.archived == Some(false) => {
            body = body.child(chapters::note(
                "Les chapitres seront lus à la fin de l'archivage.",
                cx,
            ));
        }
        Some(entry) if entry.status == chapters::Status::Failed => {
            let cause = entry
                .error
                .as_ref()
                .map(crate::widgets::card::cause)
                .unwrap_or_else(|| "Chapitres indisponibles.".into());
            let retry = on.clone();
            body = body.child(chapters::retry_row(cause, cx, move |this, cx| {
                retry(this, Act::RetryChapters, cx)
            }));
        }
        Some(entry) if entry.status == chapters::Status::Loading => {
            body = body.child(chapters::loading_rows(cx));
        }
        Some(_) => {
            body = body.child(chapters::note("Aucun chapitre pour cet épisode.", cx));
        }
        None => {
            body = body.child(chapters::loading_rows(cx));
            body = body.child(link(
                "episode-chapters-load",
                "Charger les chapitres",
                cx,
                on.clone(),
                Act::LoadChapters,
            ));
        }
    }

    // -- the words ---------------------------------------------------------
    body = body.child(section_head(IconName::ScrollText, "Transcription", cx));
    let forward = on.clone();
    body = body.child(transcript::view(
        &mut panel.transcript,
        entity,
        state,
        field,
        world.is_admin,
        window,
        cx,
        move |this, ask, cx| forward(this, Act::Transcript(ask), cx),
    ));

    // -- the description ----------------------------------------------------
    if let Some(description) = episode
        .description
        .clone()
        .filter(|text| !text.trim().is_empty())
    {
        body = body.child(section_head(IconName::Info, "Description", cx));
        body = body.child(
            div()
                .id("episode-description")
                .w_full()
                .min_w_0()
                .text_size(rems(0.75))
                .line_height(rems(1.125))
                .text_color(theme::secondary(cx))
                .when(!panel.description_open, |text| text.max_h(rems(6.75)))
                .overflow_hidden()
                .child(description),
        );
        body = body.child(link(
            "episode-description-more",
            if panel.description_open {
                "Réduire"
            } else {
                "Lire la suite"
            },
            cx,
            on.clone(),
            Act::ToggleDescription,
        ));
    }

    // -- the facts -----------------------------------------------------------
    body = body.child(section_head(IconName::Hash, "Informations", cx));
    let show_title = episode
        .album
        .as_ref()
        .map(|album| album.title.clone())
        .or_else(|| episode.artist.as_ref().map(|artist| artist.name.clone()))
        .unwrap_or_default();
    let mut facts = div().flex().flex_col().gap(rems(0.1875)).w_full();
    for (name, value) in [
        ("Podcast", show_title),
        ("Publié le", date_label(episode.published.unwrap_or(0))),
        ("Durée", duration_label(episode.duration)),
        (
            "Archivé",
            if episode.archived { "Oui" } else { "Non" }.to_owned(),
        ),
    ] {
        if value.is_empty() {
            continue;
        }
        facts = facts.child(
            div()
                .flex()
                .items_baseline()
                .gap(rems(0.5))
                .child(
                    div()
                        .w(rems(5.5))
                        .flex_shrink_0()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .child(name),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(rems(0.75))
                        .text_color(theme::text(cx))
                        .child(value),
                ),
        );
    }
    body = body.child(facts);

    // -- this person's markers ----------------------------------------------
    if !world.markers.for_episode(&key).is_empty() || world.is_current {
        body = body.child(section_head(IconName::Bookmark, "Marqueurs", cx));
        let jump = on.clone();
        let delete = on.clone();
        if let Some(chips) = markers::chips(
            &key,
            world.markers.for_episode(&key),
            usize::MAX,
            cx,
            move |this, position, cx| jump(this, Act::Seek(f64::from(position) / 1_000.0), cx),
            move |this, marker, cx| delete(this, Act::DeleteMarker(marker), cx),
        ) {
            body = body.child(chips);
        } else {
            body = body.child(
                div()
                    .text_size(rems(0.6875))
                    .text_color(theme::secondary(cx))
                    .child("Aucun marqueur dans cet épisode."),
            );
        }
        if world.is_current {
            let add = on.clone();
            let position = markers::build_position(world.playhead_s);
            body = body.child(markers::add_button(
                "episode-marker-add",
                world.playhead_s,
                true,
                cx,
                move |this, cx| add(this, Act::AddMarker(position), cx),
            ));
        }
    }

    div()
        .id("episode-panel")
        .flex()
        .flex_col()
        .gap(rems(0.5))
        .w_full()
        .h_full()
        .min_w_0()
        .min_h_0()
        .child(head)
        .child(body)
        .into_any_element()
}

fn section_head(glyph: IconName, title: &'static str, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap(rems(0.25))
        .w_full()
        .child(
            icon(glyph)
                .size(rems(0.8125))
                .text_color(theme::secondary(cx)),
        )
        .child(
            div()
                .text_size(rems(0.75))
                .font_weight(FontWeight::BOLD)
                .text_color(theme::text(cx))
                .child(title),
        )
}

fn link<V: Render>(
    id: &'static str,
    label: &str,
    cx: &mut Context<V>,
    on: impl Fn(&mut V, Act, &mut Context<V>) + Clone + 'static,
    action: Act,
) -> AnyElement {
    let label = label.to_owned();
    div()
        .id(id)
        .flex()
        .items_center()
        .self_start()
        .h(rems(1.5))
        .px(rems(0.25))
        .text_size(rems(0.6875))
        .text_color(theme::accent(cx))
        .cursor_pointer()
        .hover(|style| style.text_color(theme::text(cx)))
        .on_click(cx.listener(move |this, _, _, cx| on(this, action.clone(), cx)))
        .child(SharedString::from(label))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_model::{EpisodeAlbum, ids::ChannelId, ids::EpisodeId};

    fn episode() -> Episode {
        Episode {
            id: EpisodeId::new("5ef08406-6ef3-4adf-835b-c9e9927ba70e"),
            deezer_id: Some("5ef08406-6ef3-4adf-835b-c9e9927ba70e".into()),
            title: "Épisode 1".into(),
            duration: 3_600,
            channel_id: ChannelId::new("c1"),
            published: Some(1_794_748_649),
            archived: true,
            podcast: true,
            album: Some(EpisodeAlbum {
                deezer_id: "c1".into(),
                title: "Les Histoires".into(),
                cover: Some("/api/cover/c1".into()),
            }),
            ..Episode::default()
        }
    }

    /// The queue row of an episode carries what the engine and the speed map
    /// need: `podcast` and the show's id. `entry_from_track` sets neither.
    #[test]
    fn an_episode_becomes_a_queue_row_the_engine_treats_as_a_podcast() {
        let row = entry(&episode());
        assert_eq!(row.id, "5ef08406-6ef3-4adf-835b-c9e9927ba70e");
        assert_eq!(row.deezer_id, None, "the row uuid is the stream id");
        assert_eq!(row.podcast, Some(true));
        assert_eq!(row.channel_id.as_deref(), Some("c1"));
        assert_eq!(
            row.album.as_ref().map(|album| album.title.as_str()),
            Some("Les Histoires")
        );
        assert_eq!(row.display_artist.as_deref(), Some("Les Histoires"));
        assert_eq!(row.published.as_deref(), Some("1794748649"));
        assert!(row.is_podcast());
        assert_eq!(
            crate::widgets::speed::speed_key(&row),
            Some("c1".to_owned()),
            "the show's rate is found again on the next episode"
        );
    }

    /// The SPA's `fmtDate` and its resume states.
    #[test]
    fn a_date_and_a_resume_read_in_french() {
        assert_eq!(date_label(0), "");
        assert_eq!(date_label(1_794_748_649), "15 nov. 2026");
        assert_eq!(date_label(1_700_000_000), "14 nov. 2023");
        assert_eq!(date_label(1_709_164_800), "29 févr. 2024");

        let none = PodcastPosition::default();
        assert_eq!(resume_of(Some(&none), 3_600), None, "0 s is not a resume");
        let started = PodcastPosition {
            position: 900,
            duration: 3_600,
            ..PodcastPosition::default()
        };
        let resume = resume_of(Some(&started), 3_600).expect("a resume");
        assert!(!resume.done);
        assert!((resume.pct - 0.25).abs() < f32::EPSILON);
        assert_eq!(resume_label(resume), "Reprendre · il reste 45 min");
        assert_eq!(
            resume_label(Resume {
                done: true,
                pct: 1.0,
                left: 0
            }),
            "Terminé"
        );
        let finished = PodcastPosition {
            position: 3_600,
            duration: 3_600,
            finished: true,
            ..PodcastPosition::default()
        };
        assert!(resume_of(Some(&finished), 3_600).is_some_and(|resume| resume.done));
    }

    /// The civil date conversion, at the edges a podcast date hits.
    #[test]
    fn the_calendar_is_right_at_leap_days_and_year_ends() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(59), (1970, 3, 1));
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(18_993), (2022, 1, 1));
    }

    #[test]
    fn a_transcripts_state_reads_in_french() {
        let mut info = EpisodeTranscriptInfo::default();
        assert_eq!(transcript_state_label(&info), "Transcrire");
        info.status = "queued".into();
        assert_eq!(transcript_state_label(&info), "En attente");
        info.status = "running".into();
        info.progress = 0.42;
        assert_eq!(transcript_state_label(&info), "42 %");
        info.status = "ready".into();
        assert_eq!(transcript_state_label(&info), "Transcription");
        info.status = "error".into();
        assert_eq!(transcript_state_label(&info), "Échec");
    }
}
