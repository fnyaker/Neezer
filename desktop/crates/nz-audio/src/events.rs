//! What the engine tells the rest of the app, and how it survives the
//! real-time thread.
//!
//! The output callback runs under PipeWire's real-time scheduling: it may not
//! allocate, lock or wait (`docs/agents/desktop.md`, D5). So it never sends on
//! the channels below. It writes fixed-size [`MixerNote`]s into an `rtrb` ring
//! ([`EventBus`]); one ordinary thread drains that ring and forwards the
//! translated [`EngineEvent`]s to every subscriber. A loader thread, which is
//! not real-time, emits its own events directly.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rtrb::{Consumer, Producer, RingBuffer};

use crate::mixer::MixerShared;
use crate::source::Quality;

/// A note from the output callback to the event thread. `Copy`, no `String`,
/// so moving one through the ring can never allocate.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum MixerNote {
    /// The mixer started reading voice `generation` (the first or the gapless next).
    Started { generation: u64 },
    /// Voice `generation` drained and nothing followed it.
    Ended { generation: u64 },
    /// The callback found the current voice empty while it was still playing.
    Underrun,
}

/// Something the app has to know about, in the order it happened.
#[derive(Clone, Debug, PartialEq)]
pub enum EngineEvent {
    /// A track's first sample is about to leave the mixer. `position_s` is where
    /// in the track it starts (a `?start=` slice or a seek is not 0).
    Started {
        id: String,
        position_s: f64,
        duration_s: Option<f64>,
        /// The voice's own generation (the mixer's ticket). The player ignores a
        /// start/end older than the last `Engine::stop_current`, so a stopped
        /// voice that was already in flight cannot advance the queue.
        generation: u64,
    },
    /// The last queued voice ended: the queue is now silent.
    Ended { id: String, generation: u64 },
    /// One or more output callbacks ran out of decoded samples. `count` is the
    /// running total since the engine started, so a listener never has to add.
    Underrun { count: u64 },
    /// The server cannot answer this seek yet (nothing on disk, or the file is
    /// still being written). The engine keeps asking `/stream/<id>/ready` and
    /// lands the seek as soon as the answer changes, exactly as
    /// `webapp/src/lib/audio/seekplan.js` does in the browser.
    SeekDeferred { target_s: f64 },
    /// A seek landed; `position_s` is what the bar must show.
    Seeked { position_s: f64 },
    /// A voice could not be opened or decoded. `id` is `None` for a failure with
    /// no track attached (the output device, for instance).
    Error { id: Option<String>, message: String },
    /// The output was (re)started on this device, at this rate.
    OutputChanged { device: String, sample_rate: u32 },
    /// A network source reached the end of its body and its spool file is
    /// complete on disk. The app may publish it into the play cache (`nz-cache`)
    /// and then delete the spool; the engine reuses it until then.
    ///
    /// `quality` is the quality the source REALLY delivered
    /// ([`ByteStream::quality`], or the requested rung when the body does not
    /// say): a peer that only holds the FLAC answers an Opus 320 request with
    /// FLAC, and the file must be cached under FLAC
    /// (`docs/agents/lan-nodes.md`, *Shared-crate changes*).
    StreamComplete {
        id: String,
        quality: Quality,
        path: PathBuf,
        bytes: u64,
    },
}

/// The subscribers of [`EngineEvent`] plus the callback's note ring.
pub(crate) struct EventBus {
    subscribers: Mutex<Vec<Sender<EngineEvent>>>,
    note_producer: Mutex<Option<Producer<MixerNote>>>,
    note_consumer: Mutex<Option<Consumer<MixerNote>>>,
}

impl EventBus {
    pub(crate) fn new() -> Arc<Self> {
        // 64 notes: a burst of underruns or a fast track change never blocks the
        // callback (a full ring drops the note, and the count is an atomic
        // anyway). 64 `MixerNote`s are 16 bytes each.
        let (producer, consumer) = RingBuffer::new(64);
        Arc::new(Self {
            subscribers: Mutex::new(Vec::new()),
            note_producer: Mutex::new(Some(producer)),
            note_consumer: Mutex::new(Some(consumer)),
        })
    }

    /// The output callback's producer, taken exactly once when the stream is
    /// built. Never touched from the callback itself, only handed to it.
    pub(crate) fn take_note_producer(&self) -> Producer<MixerNote> {
        self.note_producer
            .lock()
            .expect("event bus poisoned")
            .take()
            .expect("the note producer is taken once")
    }

    /// A new listener. Several may coexist (the app's player and a test).
    pub fn subscribe(&self) -> Receiver<EngineEvent> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.subscribers
            .lock()
            .expect("event bus poisoned")
            .push(tx);
        rx
    }

    pub(crate) fn emit(&self, event: EngineEvent) {
        let mut subs = self.subscribers.lock().expect("event bus poisoned");
        // A receiver that went away is not an error: drop the sender.
        subs.retain(|tx| tx.send(event.clone()).is_ok());
    }

    /// Drains the callback's notes on an ordinary thread, translating each
    /// `generation` into the track it names. A 10 ms poll: the event is a UI
    /// notification (the mixer's handover happened on the audio thread), and
    /// 2 ms was 500 wakeups a second measured at 0.8 % of a core behind a locked
    /// screen (MQ2-follow-up, docs/agents/mobile.md).
    /// only thread that locks here.
    pub(crate) fn spawn_bridge(
        self: &Arc<Self>,
        stop: Arc<AtomicBool>,
        info: Arc<VoiceInfo>,
        shared: Arc<MixerShared>,
    ) -> JoinHandle<()> {
        let mut consumer = self
            .note_consumer
            .lock()
            .expect("event bus poisoned")
            .take()
            .expect("the note consumer is taken once");
        let bus = Arc::clone(self);
        thread::Builder::new()
            .name("nz-audio-events".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let mut last_underruns = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    let mut got = false;
                    while let Ok(note) = consumer.pop() {
                        got = true;
                        match note {
                            MixerNote::Started { generation } => {
                                if let Some(v) = info.get(generation) {
                                    bus.emit(EngineEvent::Started {
                                        id: v.id,
                                        position_s: v.start_s,
                                        duration_s: v.duration_s,
                                        generation,
                                    });
                                }
                            }
                            MixerNote::Ended { generation } => {
                                if let Some(v) = info.get(generation) {
                                    bus.emit(EngineEvent::Ended {
                                        id: v.id,
                                        generation,
                                    });
                                }
                            }
                            MixerNote::Underrun => {
                                let count = shared.underruns();
                                if count != last_underruns {
                                    last_underruns = count;
                                    bus.emit(EngineEvent::Underrun { count });
                                }
                            }
                        }
                    }
                    if !got {
                        thread::sleep(Duration::from_millis(10));
                    }
                }
            })
            .expect("spawn the audio event thread")
    }
}

/// The `generation -> track` table the event thread reads. Inserted by loader threads,
/// read by the event thread; bounded so a long session cannot grow it forever.
#[derive(Default)]
pub(crate) struct VoiceInfo {
    entries: Mutex<Vec<(u64, VoiceInfoEntry)>>,
}

#[derive(Clone, Debug)]
pub(crate) struct VoiceInfoEntry {
    pub id: String,
    pub quality: Quality,
    pub start_s: f64,
    pub duration_s: Option<f64>,
}

impl VoiceInfo {
    pub(crate) fn insert(&self, generation: u64, entry: VoiceInfoEntry) {
        let mut entries = self.entries.lock().expect("voice info poisoned");
        entries.push((generation, entry));
        // 256 entries is several hours of tracks; the oldest half is dropped
        // when it is exceeded, and a dropped entry only costs a missing event
        // for a voice that finished long ago.
        if entries.len() > 256 {
            let drop_to = entries.len() - 128;
            entries.drain(..drop_to);
        }
    }

    pub(crate) fn get(&self, generation: u64) -> Option<VoiceInfoEntry> {
        self.entries
            .lock()
            .expect("voice info poisoned")
            .iter()
            .rev()
            .find(|(g, _)| *g == generation)
            .map(|(_, e)| e.clone())
    }
}
