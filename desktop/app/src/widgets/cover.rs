//! Sleeves: fetch one, keep it on the device, paint it — or its placeholder.
//!
//! A cover is a file `nz-cache` holds (`covers.put` sniffs its format and writes
//! `<FNV-1a-64(url)>.<ext>`), and this widget is the only place that reads one.
//! The rules it follows are the web app's `lib/coverqueue.js` and rule 1:
//!
//! - **Visible first, bounded.** A screen asks for the sleeves of the rows it is
//!   ACTUALLY showing (`Covers::want` from the list's visible-range callback),
//!   at most [`MAX_IN_FLIGHT`] at a time; a hidden row costs nothing.
//! - **A screen never waits on a sleeve.** The placeholder paints in the same
//!   frame, the image replaces it when it lands, and the *data* request is what
//!   the screen waits for.
//! - **What was fetched once is kept**: the second visit finds the file and
//!   makes **0 requests** (measured in the D8a tests).
//!
//! The bytes come from `Api::bytes` on the `User` lane — a sleeve for what is on
//! screen is part of the action just taken, not background work — and the cache
//! write happens on the network thread, never on GPUI's.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gpui_kit::{
    App, IntoElement, ObjectFit, ParentElement, Pixels, SharedString, Styled, StyledImage as _,
    img, px,
};
use gpui_omarchy::{IconName, icon};
use nz_api::Lane;

use crate::state::AppState;
use crate::theme;

/// How many sleeves one screen asks for at once. Six is `Lane::User`'s own slot
/// count, so two more are queued and start as the first ones land: enough for a
/// list's first screenful without ever being a fan-out (rule 3).
pub const MAX_IN_FLIGHT: usize = 8;

/// The size class a screen draws its sleeves at.
///
/// The FILE the app paints is a copy at this size (`nz-cache`'s
/// `Covers::variant`), because GPUI uploads a painted image into an atlas tile
/// at its NATURAL size and never drops that tile on its own: a 1 000 px sleeve
/// in a 36 px row is 4 MB of decoded heap and 4 MB of atlas for 1 296 useful
/// pixels. Measured on the 4 000-row favourites list at 2560×1600 (see
/// docs/agents/desktop.md, *Memory*).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Class {
    /// A row's thumbnail (2.25 rem ≈ 36 px), the hot path of a long list.
    Row,
    /// A page's cards and shelves (≤ 220 px).
    #[default]
    Card,
    /// A page's own sleeve (the artist/album header), ≤ 340 px.
    Page,
}

impl Class {
    /// The longest side of the file this class paints, in physical pixels:
    /// about 3.5× the logical box, so a HiDPI panel still has real pixels to
    /// sample (a 36 px row reads a 128 px file).
    pub fn px(self) -> u32 {
        match self {
            Class::Row => 128,
            Class::Card => 384,
            Class::Page => 512,
        }
    }
}

/// What one screen's sleeves are, and which of them are in flight.
///
/// Held by the screen (not global) so a closed screen makes no more requests,
/// and so two screens never fight over the same in-flight map.
#[derive(Default)]
pub struct Covers {
    /// Relative path → the file the device has, at this screen's [`Class`].
    files: HashMap<String, PathBuf>,
    /// The size this screen draws its sleeves at.
    class: Class,
    /// Asked and refused: no second request this visit (a dead sleeve is not a
    /// retry storm), and the row keeps its placeholder.
    refused: HashSet<String>,
    inflight: HashSet<String>,
}

impl Covers {
    /// A screen whose sleeves are cards or a page header.
    pub fn new() -> Self {
        Self::default()
    }

    /// A screen that draws its sleeves in LIST ROWS: the file it paints is the
    /// small one (`Class::Row`), which is what keeps a 4 000-row list's decoded
    /// images and atlas tiles at a row's size instead of a sleeve's.
    pub fn rows() -> Self {
        Self {
            class: Class::Row,
            ..Self::default()
        }
    }

    /// The size class this screen paints at.
    pub fn class(&self) -> Class {
        self.class
    }

    /// The file the device already holds for `url`, if this screen knows it.
    ///
    /// A pure map read: no SQLite, no network, no allocation on the paint path.
    pub fn file(&self, url: &str) -> Option<&PathBuf> {
        self.files.get(url)
    }

    /// How many sleeves this screen is still waiting for.
    pub fn in_flight(&self) -> usize {
        self.inflight.len()
    }

    /// Ask for `url` unless it is here, in flight, or was refused. `done` runs on
    /// the screen's own event channel when the answer lands (file or refusal).
    ///
    /// Nothing is asked when the screen is offline: the placeholder is the right
    /// answer, and a doomed request per row is what rule 3 forbids.
    pub fn want<F>(&mut self, state: &AppState, url: &str, done: F)
    where
        F: FnOnce(Option<PathBuf>) + Send + 'static,
    {
        if url.is_empty()
            || self.files.contains_key(url)
            || self.refused.contains(url)
            || self.inflight.contains(url)
            || self.inflight.len() >= MAX_IN_FLIGHT
            || !state.online
        {
            return;
        }
        let (Some(api), Some(cache)) = (state.api.clone(), state.cache.clone()) else {
            return;
        };
        self.inflight.insert(url.to_owned());
        let path = format!("/cover/{}", url.trim_start_matches("/api/cover/"));
        let key = url.to_owned();
        let px = self.class.px();
        state.runtime.spawn(async move {
            // The file may already be there from an earlier visit (`nz-cache`
            // keeps sleeves): one SQLite read on the network thread, no request.
            let cached = cache.covers().file(&key);
            let answer = match cached {
                Some(file) => Some(file),
                None => match api.bytes(Lane::User, &path).await {
                    Ok(bytes) => match cache.covers().put(&key, &bytes) {
                        Ok(file) => Some(file),
                        Err(error) => {
                            tracing::debug!(%error, "la vignette n'a pas pu être écrite");
                            None
                        }
                    },
                    Err(error) => {
                        // A failure to reach the provider is never a verdict:
                        // the row keeps its placeholder for this visit only.
                        tracing::debug!(%error, url = %key, "vignette indisponible");
                        None
                    }
                },
            };
            // Decode and resize ONCE, at the size this screen draws (`Class`):
            // a row reads a 128 px file instead of the archived 1 000 px one.
            let answer = answer.and_then(|_| cache.covers().variant(&key, px));
            done(answer);
        });
    }

    /// The answer to an earlier `want`, so the next paint draws the image.
    ///
    /// `None` marks the sleeve refused: the placeholder stays and no further
    /// request is made for it this visit.
    pub fn resolved(&mut self, url: &str, file: Option<PathBuf>) {
        self.inflight.remove(url);
        match file {
            Some(file) => {
                self.files.insert(url.to_owned(), file);
            }
            None => {
                self.refused.insert(url.to_owned());
            }
        }
    }

    /// Forget what was asked, so a `Réessayer` asks again — a screen that came
    /// back online must not keep the placeholders a dead link left.
    pub fn retry_failed(&mut self) {
        self.refused.clear();
    }
}

/// The file the device holds for `url`, as an owned path.
///
/// A screen holds its sleeves behind an `Rc<RefCell<..>>` (the fetch task and
/// the paint share it), so a bare `borrow()` in the middle of a `div()` chain
/// would hand out a reference into a temporary: one function, one clone, and the
/// call sites stay one line.
pub fn file_in(covers: &std::rc::Rc<std::cell::RefCell<Covers>>, url: &str) -> Option<PathBuf> {
    covers.borrow().file(url).cloned()
}

/// Which glyph a placeholder shows, and whether the tile is round.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Album,
    Artist,
    Playlist,
    Podcast,
}

impl Kind {
    fn icon(self) -> IconName {
        match self {
            Kind::Album => IconName::Disc3,
            Kind::Artist => IconName::Music,
            Kind::Playlist => IconName::ListMusic,
            Kind::Podcast => IconName::Mic,
        }
    }
}

/// One sleeve at `size` square pixels.
///
/// `file` is what `Covers::file` answered; `label` only names the placeholder
/// (an artist's initial, an album's title's first letter). The image is drawn
/// `Cover`-fitted and never reloaded on repaint: GPUI's asset cache keys it by
/// path.
pub fn view(
    file: Option<&PathBuf>,
    url: Option<&str>,
    label: &str,
    kind: Kind,
    size: Pixels,
    cx: &App,
) -> impl IntoElement + use<> {
    let round = kind == Kind::Artist;
    let mut tile = gpui_kit::div()
        .flex_shrink_0()
        .w(size)
        .h(size)
        .overflow_hidden()
        .bg(theme::chrome(cx))
        .flex()
        .items_center()
        .justify_center();
    tile = if round {
        tile.rounded_full()
    } else {
        tile.rounded_none()
    };
    match file {
        Some(file) => {
            // The radius goes on the image too: a rounded parent clips its
            // background but not necessarily a child's own drawing (measured on
            // an artist tile: the sleeve stayed a square).
            let mut picture = img(file.clone())
                .object_fit(ObjectFit::Cover)
                .w(size)
                .h(size);
            if round {
                picture = picture.rounded_full();
            }
            tile.child(picture)
        }
        None => {
            let initial = label
                .trim()
                .chars()
                .next()
                .map(|first| first.to_uppercase().collect::<String>())
                .unwrap_or_default();
            let glyph = if url.is_some() && !initial.is_empty() && round {
                gpui_kit::div()
                    .text_size(glyph_size(size))
                    .text_color(theme::secondary(cx))
                    .child(SharedString::from(initial))
            } else {
                gpui_kit::div().child(
                    icon(kind.icon())
                        .size(glyph_size(size))
                        .text_color(theme::secondary(cx).opacity(0.65)),
                )
            };
            tile.child(glyph)
        }
    }
}

/// A glyph or an initial takes about a third of the tile, clamped so a 20 px
/// row thumbnail and a 200 px page sleeve both read.
fn glyph_size(size: Pixels) -> Pixels {
    px((f32::from(size) * 0.34).clamp(10., 64.))
}
