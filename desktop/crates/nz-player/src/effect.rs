//! What the state machine asks the app to do.
//!
//! One effect, one owner, in this order:
//!
//! | effect | owner | what it does |
//! |---|---|---|
//! | [`Effect::ReportListen`] | `nz-api` | `POST /api/listen` with the seconds actually played |
//! | [`Effect::Load`] | `nz-audio` | open `entry` at `start_s`, play if `autoplay` |
//! | [`Effect::PreloadNext`] | `nz-audio` | fill the second voice (or drop it) |
//! | [`Effect::NeedMore`] | flow/radio wiring | fetch the next title(s) and `auto_extend` them |
//! | [`Effect::Persist`] | `nz-cache` | write the queue snapshot, off the main thread |
//! | [`Effect::Changed`] | the shell | redraw what shows the transport |
//!
//! A `Load` is also what a `Seek` command produces (`start_s` is where to open):
//! the app hands it to the engine's own seek/load path, so a network stream is
//! reopened with `?start=` or a byte range rather than restarted from zero.

use crate::command::Context;
use crate::state::Entry;

/// One thing the transport did, for the app to carry out.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Open this row and start it (or hold it paused when `autoplay` is false,
    /// which is how a restored session waits for a click).
    Load {
        entry: Entry,
        autoplay: bool,
        start_s: f64,
    },
    /// What is lined up after the playing row, if anything. `None` clears the
    /// engine's second voice: the queue is empty or the playing row is last.
    PreloadNext(Option<Entry>),
    /// The row that just left the transport, and how long it was actually
    /// played. `next_id` is the row taking over, `None` at the end of a queue.
    ReportListen {
        id: String,
        listened_s: u32,
        next_id: Option<String>,
        context: Option<Context>,
        shuffle: bool,
    },
    /// A Flow or a radio queue is at its end: ask for more, then
    /// [`crate::PlayerState::auto_extend`] them.
    NeedMore { context: Context },
    /// The queue, its position or a transport setting changed: write the
    /// snapshot.
    Persist,
    /// Something the transport shows changed: redraw.
    Changed,
}

impl Effect {
    /// The row a `Load` opens, for a caller that only wants the id.
    pub fn loaded(&self) -> Option<&Entry> {
        match self {
            Self::Load { entry, .. } => Some(entry),
            _ => None,
        }
    }
}
