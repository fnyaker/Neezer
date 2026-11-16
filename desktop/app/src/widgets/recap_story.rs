//! Récap: the year in review as a story, one card at a time.
//!
//! `lib/recap.js`, `components/RecapStory.svelte` and `components/RecapSlide.svelte`
//! are the reference, and the RULES are ported rather than re-invented: the card
//! order is the server's `cards` filtered to the ids the client knows, a card the
//! payload does not name is never invented, a year too thin to tell a story says
//! so, and the strings are the plan's §7 copy to the character.
//!
//! Two desktop decisions:
//!
//! - **The deck is read at the person's pace.** The SPA auto-advances after 7 s
//!   only on a phone-sized screen (≤ 900 px) and never under reduced motion;
//!   here the same two conditions apply to the window's width, and a key press
//!   resets the clock. On the 2560×1600 panel nothing ever moves on its own.
//! - **Only the active card and the next one are drawn** (the SPA's rule): a
//!   deck costs one card's sleeves, not nine cards'.
//!
//! Sharing is the SPA's text fallback (`shareText`), copied to the clipboard —
//! the 1080×1350 poster is a canvas artefact the desktop has no pipeline for and
//! is recorded as a gap in `docs/agents/desktop.md`, D12.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement as _, Styled, div, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, button, icon};
use nz_model::{Recap, RecapGenreRow, RecapMonth, RecapYearRow, Track};

use crate::theme;
use crate::widgets::cover::{self, Covers, Kind};

/// How many rows a top holds (`lib/recap.js#TOP`).
pub const TOP: usize = 5;
/// How many sleeves the opening card shows (`lib/recap.js#MOSAIC`).
pub const MOSAIC: usize = 4;
/// The auto-advance of a narrow window (`RecapStory.svelte#AUTO_MS`).
pub const AUTO_MS: u64 = 7_000;
/// The width under which the story behaves like the phone's, panel included:
/// the SPA's `(max-width: 900px)`.
pub const NARROW_MAX: f32 = 900.;

/// The cards a payload may name, in the fixed order of the plan
/// (`lib/recap.js#KNOWN_CARDS`).
pub const KNOWN_CARDS: [&str; 8] = [
    "year",
    "minutes",
    "artists",
    "tracks",
    "albums",
    "genres",
    "streak",
    "firstlast",
];

const MONTHS: [&str; 12] = [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre",
];

const MONTHS_SHORT: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
    "déc.",
];

// -- the numbers, formatted the way the page formats them -------------------

/// `1 234` with the French thousands separator (U+202F, a narrow no-break
/// space): `lib/recap.js#formatNumber`, character for character.
pub fn format_number(value: i64) -> String {
    let negative = value < 0;
    let digits = value.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push('\u{202f}');
        }
        out.push(digit);
    }
    if negative { format!("-{out}") } else { out }
}

/// "8 540 minutes" / "1 minute" / "0 minute" — French counts zero as singular
/// (`lib/recap.js#formatMinutes`).
pub fn format_minutes(minutes: u32) -> String {
    format!(
        "{} minute{}",
        format_number(i64::from(minutes)),
        if minutes > 1 { "s" } else { "" }
    )
}

/// A span of listened time at the precision that reads: minutes under an hour,
/// hours and minutes under a day, days and hours above (`formatSpan`).
pub fn format_span(seconds: u64) -> String {
    if seconds < 60 {
        return "0 min".to_owned();
    }
    if seconds < 3_600 {
        return format!("{} min", seconds / 60);
    }
    if seconds < 86_400 {
        let hours = seconds / 3_600;
        let minutes = (seconds % 3_600) / 60;
        return if minutes > 0 {
            format!("{hours} h {minutes:02}")
        } else {
            format!("{hours} h")
        };
    }
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    if hours > 0 {
        format!("{days} j {hours} h")
    } else {
        format!("{days} j")
    }
}

/// The server's order, filtered to the ids this screen knows, then the client's
/// own `share` card — which is never shown alone on an empty year.
pub fn card_ids(payload: &Recap) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for id in &payload.cards {
        let Some(known) = KNOWN_CARDS.iter().find(|known| **known == id.as_str()) else {
            continue;
        };
        if out.contains(known) {
            continue;
        }
        out.push(known);
    }
    if payload.enough && !out.is_empty() {
        out.push("share");
    }
    out
}

/// The big line of a card. The plan's §7 copy (`lib/recap.js#slideTitle`).
pub fn slide_title(payload: &Recap, id: &str) -> String {
    match id {
        "year" => format!("Ton année {}", payload.year),
        "minutes" => format_minutes(payload.totals.minutes),
        "artists" => "Tes 5 artistes".to_owned(),
        "tracks" => "Tes 5 titres".to_owned(),
        "albums" => "Tes 5 albums".to_owned(),
        "genres" => "Tes styles".to_owned(),
        "streak" => {
            let days = payload
                .streak
                .as_ref()
                .map(|streak| streak.days)
                .unwrap_or(0);
            format!(
                "{} jour{} d'affilée",
                format_number(i64::from(days)),
                if days > 1 { "s" } else { "" }
            )
        }
        "firstlast" => "Ton premier titre de l'année".to_owned(),
        "share" => "Partage ton année".to_owned(),
        _ => String::new(),
    }
}

/// The mono line above the title: the year (and that it is still running) on the
/// opening card, the reader's place on every other.
pub fn slide_kicker(payload: &Recap, id: &str) -> String {
    let ids = card_ids(payload);
    let Some(at) = ids.iter().position(|known| *known == id) else {
        return String::new();
    };
    if id == "year" {
        return if payload.partial {
            format!("{} · en cours", payload.year)
        } else {
            payload.year.to_string()
        };
    }
    format!("{} sur {}", at + 1, ids.len())
}

/// "1ᵉʳ janvier", "13 novembre": the article-less day a French date uses.
fn day_month(day: u32, month: u32) -> String {
    let name = MONTHS
        .get(month.saturating_sub(1) as usize)
        .copied()
        .unwrap_or("");
    format!(
        "{} {name}",
        if day == 1 {
            "1ᵉʳ".to_owned()
        } else {
            day.to_string()
        }
    )
}

fn iso_day(value: Option<&str>) -> Option<(i32, u32, u32)> {
    let value = value?;
    let mut parts = value.split('-');
    let year = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    (1..=12).contains(&month).then_some(())?;
    (1..=31).contains(&day).then_some(())?;
    Some((year, month, day))
}

/// The period a year covers: a finished year is the whole year, the running one
/// what has actually happened (`lib/recap.js#periodLabel`).
pub fn period_label(payload: &Recap) -> String {
    let label = payload.year.to_string();
    if !payload.partial {
        return format!("toute l'année {label}");
    }
    let (Some(from), Some(to)) = (
        iso_day(Some(payload.period.from_date.as_str())),
        iso_day(Some(payload.period.to.as_str())),
    ) else {
        return format!("toute l'année {label}");
    };
    format!(
        "du {} au {} {}",
        day_month(from.2, from.1),
        day_month(to.2, to.1),
        to.0
    )
}

/// One bar of the twelve-month chart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    pub month: u32,
    pub label: &'static str,
    pub listens: u32,
    /// Normalised to the busiest month, so the tallest bar is exactly 100.
    pub pct: u32,
}

/// Twelve bars, always, zero-filled and in calendar order (`monthBars`).
pub fn month_bars(payload: &Recap) -> [Bar; 12] {
    let mut listens = [0u32; 12];
    for RecapMonth {
        month,
        listens: count,
        ..
    } in &payload.months
    {
        if (1..=12).contains(month) {
            listens[*month as usize - 1] = listens[*month as usize - 1].saturating_add(*count);
        }
    }
    let max = listens.iter().copied().max().unwrap_or(0);
    std::array::from_fn(|index| Bar {
        month: index as u32 + 1,
        label: MONTHS_SHORT[index],
        listens: listens[index],
        pct: if max > 0 {
            ((u64::from(listens[index]) * 100) / u64::from(max)).min(100) as u32
        } else {
            0
        },
    })
}

/// The busiest month, named in full; ties go to the earliest (`busiestMonth`).
pub fn busiest_month(payload: &Recap) -> Option<(&'static str, u32)> {
    let bars = month_bars(payload);
    let mut best: Option<Bar> = None;
    for bar in bars {
        if bar.listens > 0 && best.is_none_or(|known| bar.listens > known.listens) {
            best = Some(bar);
        }
    }
    best.map(|bar| (MONTHS[bar.month as usize - 1], bar.listens))
}

/// The styles card's lines: nothing when the server could not vouch for the
/// measurement, so a weak year never gets an invented "éclectique".
pub fn genre_rows(payload: &Recap) -> Vec<&RecapGenreRow> {
    let genres = &payload.tops.genres;
    if !genres.available {
        return Vec::new();
    }
    genres
        .rows
        .iter()
        .filter(|row| !row.name.is_empty())
        .take(TOP)
        .collect()
}

/// Up to four sleeves for the opening card: albums first, then the top titles'
/// album art, unique URLs only (`lib/recap.js#mosaicCovers`).
pub fn mosaic_covers(payload: &Recap) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |url: Option<&String>| {
        if let Some(url) = url
            && !url.is_empty()
            && !out.contains(url)
            && out.len() < MOSAIC
        {
            out.push(url.clone());
        }
    };
    for row in &payload.tops.albums {
        push(row.cover.as_ref());
    }
    for row in &payload.tops.tracks {
        push(row.track.album.cover.as_ref());
    }
    out
}

/// Why a year has no story, in the order the person's own data gives
/// (`lib/recap.js#emptyReason`): no event at all, events but not one listen, or
/// listens the server judged too few. `None` means the deck has cards.
pub fn empty_reason(payload: &Recap) -> Option<&'static str> {
    if payload.totals.events == 0 {
        return Some("none");
    }
    if payload.totals.listens == 0 {
        return Some("skips-only");
    }
    if !payload.enough {
        return Some("thin");
    }
    None
}

/// The year selector: one chip per year that holds at least one event, newest
/// first; only the server's `now_year` may say "en cours"
/// (`lib/recap.js#yearChips`).
pub fn year_chips(state: &[RecapYearRow], now_year: i32) -> Vec<(i32, String)> {
    let mut out: Vec<(i32, String)> = Vec::new();
    for row in state {
        if row.events == 0 || out.iter().any(|(year, _)| *year == row.year) {
            continue;
        }
        let label = if row.year == now_year {
            format!("{} · en cours", row.year)
        } else {
            row.year.to_string()
        };
        out.push((row.year, label));
    }
    out.sort_by_key(|(year, _)| std::cmp::Reverse(*year));
    out
}

/// The nearest year worth offering from an empty one (`Recap.svelte#nearestYear`).
pub fn nearest_year(state: &[RecapYearRow], current: i32) -> Option<i32> {
    let rows: Vec<&RecapYearRow> = state
        .iter()
        .filter(|row| row.year != current && row.events > 0)
        .collect();
    let full: Vec<&&RecapYearRow> = rows.iter().filter(|row| row.enough).collect();
    let pool: Vec<&&RecapYearRow> = if full.is_empty() {
        rows.iter().collect()
    } else {
        full
    };
    let mut best: Option<(i32, i32)> = None;
    for row in pool {
        let distance = (row.year - current).abs();
        if best.is_none_or(|(_, known)| distance < known) {
            best = Some((row.year, distance));
        }
    }
    best.map(|(year, _)| year)
}

/// One sentence for the clipboard, the SPA's Web Share fallback
/// (`lib/recap.js#shareText`), to the character.
pub fn share_text(payload: &Recap) -> String {
    let listens = payload.totals.listens;
    let artist = payload.tops.artists.first();
    let who = artist
        .map(|artist| format!(", avec {} en tête", artist.name))
        .unwrap_or_default();
    let label = if payload.partial {
        format!("Mon année {} (en cours)", payload.year)
    } else {
        format!("Mon année {}", payload.year)
    };
    format!(
        "{label} en musique : {} écoute{}, {}{who}.",
        format_number(i64::from(listens)),
        if listens > 1 { "s" } else { "" },
        format_minutes(payload.totals.minutes)
    )
}

// -- the deck ---------------------------------------------------------------

/// The story's own state: where the reader is, and when the last card was
/// shown (the auto-advance only runs on a narrow window).
pub struct Story {
    index: usize,
    shown_at: Instant,
    /// The wire's card ids, so `index` can be clamped when a new year arrives.
    count: usize,
}

impl Default for Story {
    fn default() -> Self {
        Self::new()
    }
}

impl Story {
    pub fn new() -> Self {
        Self {
            index: 0,
            shown_at: Instant::now(),
            count: 0,
        }
    }

    pub fn index(&self) -> usize {
        self.index
    }

    /// How many cards the deck holds (the payload's own order).
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Start a year at its first card (or at the remembered card of the same
    /// year, which the screen passes in).
    pub fn reset(&mut self, index: usize, count: usize) {
        self.count = count;
        self.index = index.min(count.saturating_sub(1));
        self.shown_at = Instant::now();
    }

    /// The card shown, in the payload's own order.
    pub fn current(&self, payload: &Recap) -> Option<&'static str> {
        let ids = card_ids(payload);
        ids.get(self.index).copied()
    }

    pub fn next(&mut self) {
        if self.index + 1 < self.count {
            self.index += 1;
            self.shown_at = Instant::now();
        }
    }

    pub fn previous(&mut self) {
        if self.index > 0 {
            self.index -= 1;
            self.shown_at = Instant::now();
        }
    }

    pub fn first(&mut self) {
        self.index = 0;
        self.shown_at = Instant::now();
    }

    pub fn last(&mut self) {
        self.index = self.count.saturating_sub(1);
        self.shown_at = Instant::now();
    }

    /// One second of the screen's clock: advance on a narrow window after
    /// [`AUTO_MS`], never under reduced motion, never on the last card.
    /// Returns whether the card changed.
    pub fn tick(&mut self, narrow: bool, reduced: bool, suspended: bool) -> bool {
        if !narrow || reduced || suspended || self.count < 2 || self.index + 1 >= self.count {
            return false;
        }
        if self.shown_at.elapsed() < Duration::from_millis(AUTO_MS) {
            return false;
        }
        self.next();
        true
    }

    /// A key was pressed: the countdown starts again.
    pub fn touch(&mut self) {
        self.shown_at = Instant::now();
    }
}

/// The deck: the progress bars, the active card and the next one, the hint.
///
/// `on_step` is the tap zones (−1 / +1); the screen routes the same two steps
/// from the arrows and from `j`/`k`/`Page préc.`/`Page suiv.`.
#[allow(clippy::too_many_arguments)]
pub fn view<V: Render>(
    story: &mut Story,
    payload: &Recap,
    covers: &Rc<RefCell<Covers>>,
    cx: &mut Context<V>,
    on_step: impl Fn(&mut V, i64, &mut Context<V>) + Clone + 'static,
    on_play: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
    on_share: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    let ids = card_ids(payload);
    if ids.is_empty() {
        return div().into_any_element();
    }
    story.count = ids.len();
    if story.index >= ids.len() {
        story.index = ids.len() - 1;
    }
    let active = story.index;

    let mut bars = div().flex().w_full().gap(rems(0.25));
    for index in 0..ids.len() {
        let done = index < active;
        bars = bars.child(div().flex_1().h(px(3.)).bg(if done || index == active {
            theme::text(cx)
        } else {
            theme::border(cx)
        }));
    }

    let mut stage = div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .overflow_hidden()
        .child(card(payload, ids[active], covers, cx, on_play, on_share));

    // The two tap zones sit UNDER the card (the card's own buttons take their
    // click back, `RecapStory.svelte`): a press on the words turns the card.
    stage = stage
        .child(
            div()
                .id("recap-prev")
                .absolute()
                .left(px(0.))
                .top(px(0.))
                .bottom(px(0.))
                .w(gpui_kit::relative(0.4))
                .cursor_pointer()
                .on_click(cx.listener({
                    let on_step = on_step.clone();
                    move |this, _, _, cx| on_step(this, -1, cx)
                })),
        )
        .child(
            div()
                .id("recap-next")
                .absolute()
                .right(px(0.))
                .top(px(0.))
                .bottom(px(0.))
                .w(gpui_kit::relative(0.6))
                .cursor_pointer()
                .on_click(cx.listener({
                    let on_step = on_step.clone();
                    move |this, _, _, cx| on_step(this, 1, cx)
                })),
        );

    div()
        .id("recap-story")
        .flex()
        .flex_col()
        .size_full()
        .min_h(px(0.))
        .child(div().p(rems(0.75)).child(bars))
        .child(stage)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(rems(0.75))
                .px(rems(1.))
                .py(rems(0.5))
                .text_size(rems(0.625))
                .font_family(theme::mono(cx))
                .text_color(theme::secondary(cx))
                .child(format!("{} / {}", active + 1, ids.len()))
                .child("← →"),
        )
        .into_any_element()
}

/// One card. Its id is the payload's own order; the layout is the only thing
/// this function decides.
fn card<V: Render>(
    payload: &Recap,
    id: &str,
    covers: &Rc<RefCell<Covers>>,
    cx: &mut Context<V>,
    on_play: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
    on_share: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
) -> AnyElement {
    let title = slide_title(payload, id);
    let kicker = slide_kicker(payload, id);
    let glyph = match id {
        "minutes" => IconName::Timer,
        "artists" => IconName::User,
        "tracks" => IconName::Music,
        "albums" => IconName::Disc3,
        "genres" => IconName::Tag,
        "streak" => IconName::Activity,
        "share" => IconName::Share2,
        _ => IconName::Sparkles,
    };
    let mut face = div()
        .id(SharedString::from(format!("recap-card-{id}")))
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .gap(rems(0.75))
        .p(rems(1.25))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(rems(0.625))
                        .font_family(theme::mono(cx))
                        .text_color(theme::secondary(cx))
                        .child(kicker),
                )
                .child(icon(glyph).size(rems(1.)).text_color(theme::accent(cx))),
        )
        .child(
            div()
                .text_size(rems(1.5))
                .font_weight(FontWeight::BOLD)
                .child(title),
        );

    match id {
        "year" => {
            face = face.child(
                div()
                    .text_size(rems(0.875))
                    .text_color(theme::secondary(cx))
                    .child(format!(
                        "{} · {}",
                        count_word(payload.totals.listens, "écoute", "écoutes"),
                        format_minutes(payload.totals.minutes)
                    )),
            );
            let mosaic = mosaic_covers(payload);
            if !mosaic.is_empty() {
                let mut tiles = div().flex().gap(px(2.));
                for url in mosaic {
                    let file = cover_file(covers, &url);
                    tiles = tiles.child(cover::view(
                        file.as_ref(),
                        Some(url.as_str()),
                        "",
                        Kind::Album,
                        px(88.),
                        cx,
                    ));
                }
                face = face.child(tiles);
            }
            face = face.child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme::secondary(cx))
                    .child(period_label(payload)),
            );
        }
        "minutes" => {
            face = face.child(
                div()
                    .text_size(rems(0.875))
                    .text_color(theme::secondary(cx))
                    .child(format!(
                        "soit {} de musique",
                        format_span(payload.totals.seconds)
                    )),
            );
            let bars = month_bars(payload);
            let mut chart = div()
                .id("recap-months")
                .flex()
                .items_end()
                .gap(px(3.))
                .h(px(140.))
                .w_full();
            for bar in bars {
                chart = chart.child(
                    div()
                        .flex_1()
                        .h(px(if bar.pct > 0 {
                            1.4 * bar.pct as f32
                        } else {
                            3.
                        }))
                        .bg(if bar.pct > 0 {
                            theme::accent(cx)
                        } else {
                            theme::border(cx)
                        }),
                );
            }
            face = face.child(chart);
            let mut foot = div()
                .flex()
                .items_center()
                .justify_between()
                .text_size(rems(0.6875))
                .text_color(theme::secondary(cx))
                .child("écoutes par mois");
            if let Some((month, listens)) = busiest_month(payload) {
                foot = foot.child(div().text_color(theme::text(cx)).child(format!(
                    "Ton mois le plus fort : {month} ({})",
                    count_word(listens, "écoute", "écoutes")
                )));
            }
            face = face.child(foot);
        }
        "artists" => {
            let mut rows = div().flex().flex_col().gap(rems(0.25));
            for (index, row) in payload.tops.artists.iter().take(TOP).enumerate() {
                let file = row.cover.as_deref().and_then(|url| cover_file(covers, url));
                rows = rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .child(
                            div()
                                .w(rems(1.25))
                                .font_family(theme::mono(cx))
                                .text_size(rems(0.6875))
                                .text_color(theme::secondary(cx))
                                .child((index + 1).to_string()),
                        )
                        .child(cover::view(
                            file.as_ref(),
                            row.cover.as_deref(),
                            &row.name,
                            Kind::Artist,
                            px(28.),
                            cx,
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w(px(0.))
                                .child(div().truncate().child(row.name.clone()))
                                .child(
                                    div()
                                        .text_size(rems(0.6875))
                                        .text_color(theme::secondary(cx))
                                        .child(count_word(row.listens, "écoute", "écoutes")),
                                ),
                        ),
                );
            }
            face = face.child(rows);
        }
        "tracks" => {
            let mut rows = div().flex().flex_col().gap(rems(0.25));
            for (index, row) in payload
                .tops
                .tracks
                .iter()
                .filter(|row| !row.track.title.is_empty())
                .take(TOP)
                .enumerate()
            {
                let file = row
                    .track
                    .album
                    .cover
                    .as_deref()
                    .and_then(|url| cover_file(covers, url));
                rows = rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .child(
                            div()
                                .w(rems(1.25))
                                .font_family(theme::mono(cx))
                                .text_size(rems(0.6875))
                                .text_color(theme::secondary(cx))
                                .child((index + 1).to_string()),
                        )
                        .child(cover::view(
                            file.as_ref(),
                            row.track.album.cover.as_deref(),
                            &row.track.title,
                            Kind::Album,
                            px(26.),
                            cx,
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w(px(0.))
                                .child(div().truncate().child(row.track.title.clone()))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(rems(0.6875))
                                        .text_color(theme::secondary(cx))
                                        .child(artist_line(&row.track)),
                                ),
                        )
                        .child(
                            div()
                                .font_family(theme::mono(cx))
                                .text_size(rems(0.625))
                                .text_color(theme::secondary(cx))
                                .child(count_word(row.listens, "écoute", "écoutes")),
                        ),
                );
            }
            face = face.child(rows);
            face = face.child(
                button(
                    "recap-play-top",
                    "Lire ces 5 titres",
                    ButtonVariant::Primary,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| on_play(this, cx))),
            );
        }
        "albums" => {
            let mut rows = div().flex().flex_col().gap(rems(0.25));
            for (index, row) in payload.tops.albums.iter().take(TOP).enumerate() {
                let file = row.cover.as_deref().and_then(|url| cover_file(covers, url));
                rows = rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .child(
                            div()
                                .w(rems(1.25))
                                .font_family(theme::mono(cx))
                                .text_size(rems(0.6875))
                                .text_color(theme::secondary(cx))
                                .child((index + 1).to_string()),
                        )
                        .child(cover::view(
                            file.as_ref(),
                            row.cover.as_deref(),
                            &row.title,
                            Kind::Album,
                            px(26.),
                            cx,
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w(px(0.))
                                .child(div().truncate().child(row.title.clone()))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(rems(0.6875))
                                        .text_color(theme::secondary(cx))
                                        .child(row.artist.clone().unwrap_or_default()),
                                ),
                        )
                        .child(
                            div()
                                .font_family(theme::mono(cx))
                                .text_size(rems(0.625))
                                .text_color(theme::secondary(cx))
                                .child(count_word(row.listens, "écoute", "écoutes")),
                        ),
                );
            }
            face = face.child(rows);
        }
        "genres" => {
            let rows = genre_rows(payload);
            if rows.is_empty() {
                face = face.child(
                    div()
                        .text_size(rems(0.8125))
                        .text_color(theme::secondary(cx))
                        .child("Les styles de cette année n'ont pas été mesurés."),
                );
            } else {
                let mut lines = div().flex().flex_col().gap(rems(0.25));
                for row in rows {
                    lines = lines.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(div().child(row.name.clone()))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(rems(0.5))
                                    .child(
                                        div().w(px(80.)).h(px(4.)).bg(theme::border(cx)).child(
                                            div()
                                                .h(px(4.))
                                                .w(px(80. * row.share.clamp(0., 1.) as f32))
                                                .bg(theme::accent(cx)),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .font_family(theme::mono(cx))
                                            .text_size(rems(0.625))
                                            .text_color(theme::secondary(cx))
                                            .child(format!(
                                                "{} %",
                                                (row.share.clamp(0., 1.) * 100.).round()
                                            )),
                                    ),
                            ),
                    );
                }
                face = face.child(lines);
                face = face.child(
                    div()
                        .text_size(rems(0.625))
                        .text_color(theme::secondary(cx))
                        .child(format!(
                            "{} % des écoutes mesurées · {} styles explorés",
                            (payload.tops.genres.coverage.clamp(0., 1.) * 100.).round(),
                            payload.tops.genres.explored
                        )),
                );
            }
        }
        "streak" => {
            if let Some(streak) = &payload.streak {
                let from = iso_day(Some(streak.from_date.as_str()));
                let to = iso_day(Some(streak.to.as_str()));
                let range = match (from, to) {
                    (Some(from), Some(to)) if from.0 == to.0 && from.1 == to.1 => format!(
                        "du {} au {} {}",
                        if from.2 == 1 {
                            "1ᵉʳ".to_owned()
                        } else {
                            from.2.to_string()
                        },
                        to.2,
                        MONTHS[to.1 as usize - 1]
                    ),
                    (Some(from), Some(to)) => format!(
                        "du {} au {}",
                        day_month(from.2, from.1),
                        day_month(to.2, to.1)
                    ),
                    _ => String::new(),
                };
                face = face.child(
                    div()
                        .text_size(rems(0.875))
                        .text_color(theme::secondary(cx))
                        .child(range),
                );
            }
            if let Some(day) = &payload.biggest_day {
                face = face.child(div().text_size(rems(0.8125)).child(format!(
                    "Ta plus grosse journée : {} ({})",
                    day.date,
                    format_span(day.seconds)
                )));
            }
        }
        "firstlast" => {
            if let Some(first) = &payload.first {
                face = face.child(edge_line("Premier titre", &first.track, covers, cx));
            }
            if let Some(last) = &payload.last {
                face = face.child(edge_line("Dernier titre", &last.track, covers, cx));
            }
        }
        "share" => {
            face = face.child(
                div()
                    .text_size(rems(0.8125))
                    .text_color(theme::secondary(cx))
                    .child(share_text(payload)),
            );
            face = face.child(
                button(
                    "recap-share",
                    "Copier pour partager",
                    ButtonVariant::Primary,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| on_share(this, cx))),
            );
        }
        _ => {}
    }
    face.into_any_element()
}

fn edge_line<V: Render>(
    label: &str,
    track: &Track,
    covers: &Rc<RefCell<Covers>>,
    cx: &mut Context<V>,
) -> AnyElement {
    let file = track
        .album
        .cover
        .as_deref()
        .and_then(|url| cover_file(covers, url));
    div()
        .flex()
        .items_center()
        .gap(rems(0.5))
        .child(cover::view(
            file.as_ref(),
            track.album.cover.as_deref(),
            &track.title,
            Kind::Album,
            px(34.),
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(rems(0.5625))
                        .font_family(theme::mono(cx))
                        .text_color(theme::secondary(cx))
                        .child(label.to_owned()),
                )
                .child(div().truncate().child(track.title.clone()))
                .child(
                    div()
                        .truncate()
                        .text_size(rems(0.6875))
                        .text_color(theme::secondary(cx))
                        .child(artist_line(track)),
                ),
        )
        .into_any_element()
}

fn cover_file(covers: &Rc<RefCell<Covers>>, url: &str) -> Option<PathBuf> {
    covers.borrow().file(url).cloned()
}

/// `lib/format.js#artistLine`: the display artist when the server flattened it.
pub fn artist_line(track: &Track) -> String {
    track
        .display_artist
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| track.artist.name.clone())
}

/// "3 écoutes" / "1 écoute" / "0 écoute" — French counts zero as singular.
fn count_word(count: u32, one: &str, many: &str) -> String {
    format!(
        "{} {}",
        format_number(i64::from(count)),
        if count > 1 { many } else { one }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_model::{RecapArtistRow, RecapGenres, RecapTotals, RecapTrackRow};

    fn year(cards: &[&str]) -> Recap {
        Recap {
            year: 2026,
            enough: true,
            cards: cards.iter().map(|id| (*id).to_owned()).collect(),
            totals: RecapTotals {
                events: 120,
                listens: 8540,
                minutes: 24_310,
                seconds: 1_458_600,
                ..RecapTotals::default()
            },
            ..Recap::default()
        }
    }

    /// The numbers are the page's, to the character (a narrow no-break space is
    /// the French thousands separator).
    #[test]
    fn a_number_reads_as_the_page_writes_it() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(999), "999");
        assert_eq!(format_number(1_000), "1\u{202f}000");
        assert_eq!(format_number(8_540), "8\u{202f}540");
        assert_eq!(format_number(1_234_567), "1\u{202f}234\u{202f}567");
        assert_eq!(format_number(-4_200), "-4\u{202f}200");
        assert_eq!(format_minutes(8_540), "8\u{202f}540 minutes");
        assert_eq!(format_minutes(1), "1 minute");
        assert_eq!(format_minutes(0), "0 minute");
        assert_eq!(format_span(0), "0 min");
        assert_eq!(format_span(59), "0 min");
        assert_eq!(format_span(3_599), "59 min");
        assert_eq!(format_span(3_600), "1 h");
        assert_eq!(format_span(3_780), "1 h 03");
        assert_eq!(format_span(86_399), "23 h 59");
        assert_eq!(format_span(86_400), "1 j");
        assert_eq!(format_span(208_800 + 79_200), "3 j 8 h");
    }

    /// The card order is the server's, filtered to what the client knows, and
    /// `share` is appended only to a real story.
    #[test]
    fn the_deck_keeps_the_servers_order_and_never_invents_a_card() {
        let payload = year(&["year", "minutes", "inconnue", "year"]);
        assert_eq!(card_ids(&payload), ["year", "minutes", "share"]);

        let thin = Recap {
            enough: false,
            ..year(&["year", "minutes"])
        };
        assert_eq!(card_ids(&thin), ["year", "minutes"]);
        assert_eq!(
            empty_reason(&thin),
            Some("thin"),
            "few listens: the server says the year cannot tell a story"
        );
    }

    #[test]
    fn a_year_too_thin_says_why_it_has_no_story() {
        let mut payload = year(&["year"]);
        payload.totals.events = 0;
        assert_eq!(empty_reason(&payload), Some("none"));
        payload.totals.events = 12;
        payload.totals.listens = 0;
        assert_eq!(empty_reason(&payload), Some("skips-only"));
        payload.totals.listens = 3;
        payload.enough = false;
        assert_eq!(empty_reason(&payload), Some("thin"));
        payload.enough = true;
        assert_eq!(empty_reason(&payload), None);
    }

    #[test]
    fn a_card_is_titled_from_the_payload_only() {
        let mut payload = year(&["year", "minutes", "streak", "firstlast"]);
        assert_eq!(slide_title(&payload, "year"), "Ton année 2026");
        assert_eq!(slide_title(&payload, "minutes"), "24\u{202f}310 minutes");
        assert_eq!(slide_title(&payload, "streak"), "0 jour d'affilée");
        payload.streak = Some(nz_model::RecapStreak {
            days: 12,
            from_date: "2026-01-02".into(),
            to: "2026-01-13".into(),
        });
        assert_eq!(slide_title(&payload, "streak"), "12 jours d'affilée");
        assert_eq!(slide_kicker(&payload, "year"), "2026");
        assert_eq!(slide_kicker(&payload, "minutes"), "2 sur 5");
        payload.partial = true;
        assert_eq!(slide_kicker(&payload, "year"), "2026 · en cours");
    }

    #[test]
    fn the_month_chart_is_twelve_bars_zero_filled() {
        let mut payload = year(&["minutes"]);
        payload.months = vec![
            RecapMonth {
                month: 7,
                listens: 40,
                skips: 0,
                seconds: 0,
            },
            RecapMonth {
                month: 1,
                listens: 10,
                skips: 0,
                seconds: 0,
            },
            RecapMonth {
                month: 13,
                listens: 99,
                skips: 0,
                seconds: 0,
            },
        ];
        let bars = month_bars(&payload);
        assert_eq!(bars.len(), 12);
        assert_eq!(bars[6].pct, 100, "the busiest month is the tallest");
        assert_eq!(bars[0].pct, 25);
        assert_eq!(bars[2].pct, 0);
        assert_eq!(busiest_month(&payload), Some(("juillet", 40)));
    }

    /// The period label: a finished year is the whole year; a running one is
    /// what has actually happened.
    #[test]
    fn the_period_reads_as_a_french_date() {
        let mut payload = year(&["year"]);
        payload.period = nz_model::RecapPeriod {
            from_date: "2026-01-01".into(),
            to: "2026-11-16".into(),
        };
        assert_eq!(period_label(&payload), "toute l'année 2026");
        payload.partial = true;
        assert_eq!(period_label(&payload), "du 1ᵉʳ janvier au 16 novembre 2026");
    }

    /// The advertised sentence is the SPA's `shareText`, to the character.
    #[test]
    fn the_shared_sentence_is_the_pages_own() {
        let mut payload = year(&["year", "artists"]);
        payload.tops.artists = vec![RecapArtistRow {
            name: "Kaya Project".into(),
            listens: 120,
            ..RecapArtistRow::default()
        }];
        payload.tops.tracks = vec![RecapTrackRow::default()];
        assert_eq!(
            share_text(&payload),
            "Mon année 2026 en musique : 8\u{202f}540 écoutes, 24\u{202f}310 minutes, \
             avec Kaya Project en tête."
        );
        payload.partial = true;
        assert!(share_text(&payload).starts_with("Mon année 2026 (en cours) en musique"));
    }

    /// The styles card never invents a genre the server could not vouch for.
    #[test]
    fn a_styles_card_without_measurement_is_empty() {
        let mut payload = year(&["genres"]);
        payload.tops.genres = RecapGenres {
            available: false,
            rows: vec![RecapGenreRow {
                name: "Techno".into(),
                listens: 10,
                share: 0.5,
            }],
            ..RecapGenres::default()
        };
        assert!(genre_rows(&payload).is_empty());
        payload.tops.genres.available = true;
        assert_eq!(genre_rows(&payload).len(), 1);
    }

    #[test]
    fn the_year_chips_are_newest_first_and_only_the_server_says_en_cours() {
        let rows = vec![
            RecapYearRow {
                year: 2025,
                events: 12,
                enough: true,
                ..RecapYearRow::default()
            },
            RecapYearRow {
                year: 2026,
                events: 4,
                enough: false,
                ..RecapYearRow::default()
            },
            RecapYearRow {
                year: 2024,
                events: 0,
                ..RecapYearRow::default()
            },
        ];
        assert_eq!(
            year_chips(&rows, 2026),
            [
                (2026, "2026 · en cours".to_owned()),
                (2025, "2025".to_owned())
            ]
        );
        assert_eq!(nearest_year(&rows, 2026), Some(2025));
        assert_eq!(
            nearest_year(&rows, 2025),
            Some(2026),
            "a thin year is a fallback"
        );
    }

    /// The deck advances on a narrow window after 7 s, never on a wide one, and
    /// a key press restarts the countdown.
    #[test]
    fn the_story_advances_only_on_a_narrow_window() {
        let payload = year(&["year", "minutes", "artists"]);
        let mut story = Story::new();
        story.reset(0, card_ids(&payload).len());
        assert!(!story.tick(false, false, false), "a wide window waits");
        assert_eq!(story.index(), 0);
        // The countdown is `AUTO_MS`; the test moves the clock instead of
        // sleeping (a wall-clock wait would be the flaky thing).
        story.shown_at = Instant::now() - Duration::from_millis(AUTO_MS + 1);
        assert!(!story.tick(true, true, false), "reduced motion waits");
        assert!(!story.tick(true, false, true), "a sheet pauses it");
        assert!(story.tick(true, false, false));
        assert_eq!(story.index(), 1);
        story.touch();
        assert!(
            !story.tick(true, false, false),
            "a key restarts the countdown"
        );
        story.last();
        assert_eq!(story.index(), card_ids(&payload).len() - 1);
        assert!(!story.tick(true, false, false), "the last card stays");
        story.previous();
        assert_eq!(story.index(), card_ids(&payload).len() - 2);
        story.first();
        assert_eq!(story.index(), 0);
    }
}
