//! The two voices, mixed in the output callback — gaplessly.
//!
//! There is no timer between tracks and no "stop, then start": the callback
//! pops the current voice's ring, and the moment that ring is empty **and its
//! decoder has ended**, it takes the preloaded next voice and keeps writing in
//! the same buffer. That is the whole gapless mechanism: one continuous stream
//! of samples, no gap and no click to introduce one
//! (`docs/agents/desktop.md`, D5 *Design*).
//!
//! The callback never allocates, locks or waits. Everything it needs is either
//! preallocated (the rings) or an atomic (`MixerShared`); commands arrive
//! through a small `rtrb` ring it drains at the top of every buffer.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

use crate::clock::{AtomicF32, Clock};
use crate::events::MixerNote;
use crate::voice::VoiceConsumer;

/// The atomics the callback reads and the app writes. No lock anywhere near the
/// output thread: volume, mute and pause are single values, the position and
/// the underrun count are counters.
#[derive(Debug)]
pub struct MixerShared {
    /// What the person hears, in seconds of the current track.
    pub clock: Clock,
    underruns: AtomicU64,
    volume: AtomicF32,
    muted: AtomicBool,
    paused: AtomicBool,
    /// The generation of the voice the callback is playing, `0` when the queue
    /// ended. Written by the callback at an install or a gapless switch, so the
    /// engine can name the track that is actually heard — after a handover the
    /// old "current" is no longer it.
    current_generation: AtomicU64,
}

impl MixerShared {
    pub fn new(sample_rate: u32, volume: f32) -> Self {
        Self {
            clock: Clock::new(sample_rate),
            underruns: AtomicU64::new(0),
            volume: AtomicF32::new(volume.clamp(0.0, 1.0)),
            muted: AtomicBool::new(false),
            paused: AtomicBool::new(true),
            current_generation: AtomicU64::new(0),
        }
    }

    /// Output callbacks that ran out of samples since the engine started. This
    /// counts only playing audio: the silence before a track's first sample is
    /// buffering, and a paused mixer is not starving.
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Acquire)
    }

    pub(crate) fn note_underrun(&self) {
        self.underruns.fetch_add(1, Ordering::AcqRel);
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume.store(volume.clamp(0.0, 1.0));
    }

    pub fn volume(&self) -> f32 {
        self.volume.load()
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Release);
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Acquire)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
    }

    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    /// The voice the mixer is playing right now (`0` when silent).
    pub fn current_generation(&self) -> u64 {
        self.current_generation.load(Ordering::Acquire)
    }

    fn note_current(&self, generation: u64) {
        self.current_generation.store(generation, Ordering::Release);
    }
}

/// What the engine (or a test) tells the mixer. Every variant moves a voice in
/// or out; none of them allocates.
pub(crate) enum MixerCmd {
    /// A new current voice, or `None` to go silent.
    InstallCurrent(Option<VoiceConsumer>),
    /// The preloaded next voice (gapless handover), or `None` to drop it.
    InstallNext(Option<VoiceConsumer>),
    /// Stop everything; the engine is going away.
    Shutdown,
}

/// The mixer, owned by the output callback.
pub(crate) struct Mixer {
    shared: Arc<MixerShared>,
    notes: Producer<MixerNote>,
    cmds: Consumer<MixerCmd>,
    current: Option<VoiceConsumer>,
    next: Option<VoiceConsumer>,
    /// Has the current voice produced at least one sample? Before that, an empty
    /// ring is buffering, not an underrun.
    started: bool,
    /// The voice whose `Ended` note was already sent.
    ended_gen: Option<u64>,
}

impl Mixer {
    /// `notes` is the callback's side of the event bus; `cmds` comes back to the
    /// engine as the producer half.
    pub(crate) fn new(
        shared: Arc<MixerShared>,
        notes: Producer<MixerNote>,
    ) -> (Self, Producer<MixerCmd>) {
        // 8 slots: at most a current, a next and a clear between two buffers.
        let (cmd_tx, cmd_rx) = RingBuffer::new(8);
        (
            Self {
                shared,
                notes,
                cmds: cmd_rx,
                current: None,
                next: None,
                started: false,
                ended_gen: None,
            },
            cmd_tx,
        )
    }

    /// Apply everything the engine queued. Called at the top of every buffer,
    /// before a single sample is read.
    fn drain(&mut self) {
        while let Ok(cmd) = self.cmds.pop() {
            match cmd {
                MixerCmd::InstallCurrent(Some(voice)) => {
                    self.install_current(voice);
                }
                MixerCmd::InstallCurrent(None) | MixerCmd::Shutdown => {
                    self.drop_all();
                }
                MixerCmd::InstallNext(Some(voice)) => {
                    if let Some(old) = self.next.replace(voice) {
                        old.stop();
                    }
                }
                MixerCmd::InstallNext(None) => {
                    if let Some(old) = self.next.take() {
                        old.stop();
                    }
                }
            }
        }
    }

    fn install_current(&mut self, voice: VoiceConsumer) {
        if let Some(old) = self.current.take() {
            old.stop();
        }
        self.shared.clock.start_at(voice.start_s);
        self.shared.note_current(voice.generation);
        self.started = false;
        self.ended_gen = None;
        let _ = self.notes.push(MixerNote::Started {
            generation: voice.generation,
        });
        self.current = Some(voice);
    }

    fn drop_all(&mut self) {
        self.shared.note_current(0);
        if let Some(old) = self.current.take() {
            old.stop();
        }
        if let Some(old) = self.next.take() {
            old.stop();
        }
        self.started = false;
        self.ended_gen = None;
    }

    /// Fill `out` (interleaved stereo f32) from the current voice, handing over
    /// to the next one inside this call if the current one ends here.
    pub(crate) fn render(&mut self, out: &mut [f32]) {
        self.drain();
        out.fill(0.0);
        if self.shared.paused() {
            return;
        }
        let gain = if self.shared.muted() {
            0.0
        } else {
            self.shared.volume()
        };
        let frames = out.len() / 2;
        let mut frame = 0;
        let mut starved = false;
        while frame < frames {
            let Some(current) = self.current.as_mut() else {
                break;
            };
            if let Some((left, right)) = current.frame() {
                out[frame * 2] = left * gain;
                out[frame * 2 + 1] = right * gain;
                frame += 1;
                self.started = true;
                continue;
            }
            // The ring is empty. Two honest cases, one failure.
            if current.is_ended() {
                let ended_gen = current.generation;
                if let Some(next) = self.next.take() {
                    self.install_current(next);
                    // The sound is already flowing: from here an empty next
                    // voice is a real gap, not the buffering before a track.
                    self.started = true;
                    continue;
                }
                if self.ended_gen != Some(ended_gen) {
                    self.ended_gen = Some(ended_gen);
                    let _ = self.notes.push(MixerNote::Ended {
                        generation: ended_gen,
                    });
                }
                self.current = None;
                break;
            }
            if self.started {
                // Playing, not ended, nothing to play: the decoder did not keep
                // up. Count one per buffer, not one per missing sample.
                self.shared.note_underrun();
                let _ = self.notes.push(MixerNote::Underrun);
            }
            starved = true;
            break;
        }
        if frame > 0 {
            self.shared.clock.advance(frame as u64);
        }
        let _ = starved;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::voice::voice_ring;

    #[test]
    fn silence_when_paused_and_no_underrun_before_the_first_sample() {
        let shared = Arc::new(MixerShared::new(48_000, 1.0));
        let (note_tx, mut note_rx) = RingBuffer::new(64);
        let (mut mixer, mut cmds) = Mixer::new(Arc::clone(&shared), note_tx);
        let (mut producer, consumer, _status) = voice_ring(48_000, 0.5, 7, 0.0);
        cmds.push(MixerCmd::InstallCurrent(Some(consumer))).unwrap();
        let mut out = vec![0.0f32; 480 * 2];
        // Paused: silence, and the install note is still delivered.
        mixer.render(&mut out);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(note_rx.pop().unwrap(), MixerNote::Started { generation: 7 });

        // Playing but nothing decoded yet: buffering, never an underrun.
        shared.set_paused(false);
        mixer.render(&mut out);
        assert_eq!(shared.underruns(), 0);

        // Once a sample played, an empty ring is an underrun, counted once.
        producer.push(&[0.5, -0.5, 0.5, -0.5]);
        mixer.render(&mut out);
        assert_eq!(out[0], 0.5);
        assert_eq!(out[1], -0.5);
        assert_eq!(out[2], 0.5, "the second pushed frame is audio too");
        assert_eq!(out[4], 0.0, "then the ring is empty: silence");
        assert_eq!(shared.underruns(), 1);
        mixer.render(&mut out);
        assert_eq!(shared.underruns(), 2, "one per buffer, not one per sample");
        assert_eq!(note_rx.pop().unwrap(), MixerNote::Underrun);
    }

    #[test]
    fn the_next_voice_starts_in_the_same_buffer_with_no_gap() {
        let shared = Arc::new(MixerShared::new(48_000, 1.0));
        shared.set_paused(false);
        let (note_tx, mut note_rx) = RingBuffer::new(64);
        let (mut mixer, mut cmds) = Mixer::new(Arc::clone(&shared), note_tx);

        let (mut a, a_consumer, a_status) = voice_ring(48_000, 0.5, 1, 0.0);
        let (mut b, b_consumer, _b_status) = voice_ring(48_000, 0.5, 2, 0.0);
        a.push(&[0.1, 0.1, 0.2, 0.2]);
        b.push(&[0.3, 0.3, 0.4, 0.4]);
        a_status.finish();
        cmds.push(MixerCmd::InstallCurrent(Some(a_consumer)))
            .unwrap();
        cmds.push(MixerCmd::InstallNext(Some(b_consumer))).unwrap();

        let mut out = vec![0.0f32; 4 * 2];
        mixer.render(&mut out);
        assert_eq!(
            out,
            vec![0.1, 0.1, 0.2, 0.2, 0.3, 0.3, 0.4, 0.4],
            "the seam is the next voice's first sample, in the same callback"
        );
        assert_eq!(shared.underruns(), 0);
        let notes: Vec<_> = std::iter::from_fn(|| note_rx.pop().ok()).collect();
        assert_eq!(
            notes,
            vec![
                MixerNote::Started { generation: 1 },
                MixerNote::Started { generation: 2 }
            ]
        );
        assert!((shared.clock.position() - 4.0 / 48_000.0).abs() < 1e-9);
    }

    #[test]
    fn the_end_of_the_queue_is_announced_once() {
        let shared = Arc::new(MixerShared::new(48_000, 1.0));
        shared.set_paused(false);
        let (note_tx, mut note_rx) = RingBuffer::new(64);
        let (mut mixer, mut cmds) = Mixer::new(Arc::clone(&shared), note_tx);
        let (mut a, a_consumer, a_status) = voice_ring(48_000, 0.5, 9, 0.0);
        a.push(&[0.1, 0.1]);
        a_status.finish();
        cmds.push(MixerCmd::InstallCurrent(Some(a_consumer)))
            .unwrap();

        let mut out = vec![0.0f32; 4 * 2];
        mixer.render(&mut out);
        assert_eq!(&out[..2], &[0.1, 0.1]);
        assert_eq!(&out[2..], &[0.0; 6]);
        mixer.render(&mut out);
        let notes: Vec<_> = std::iter::from_fn(|| note_rx.pop().ok()).collect();
        assert_eq!(
            notes,
            vec![
                MixerNote::Started { generation: 9 },
                MixerNote::Ended { generation: 9 }
            ],
            "Ended is sent once, not on every silent buffer"
        );
        assert_eq!(shared.underruns(), 0, "the natural end is not an underrun");
    }

    #[test]
    fn volume_and_mute_apply_without_touching_the_bytes_in_the_ring() {
        let shared = Arc::new(MixerShared::new(48_000, 0.5));
        shared.set_paused(false);
        let (note_tx, _note_rx) = RingBuffer::new(64);
        let (mut mixer, mut cmds) = Mixer::new(Arc::clone(&shared), note_tx);
        let (mut a, a_consumer, _) = voice_ring(48_000, 0.5, 1, 0.0);
        a.push(&[1.0, -1.0, 1.0, -1.0]);
        cmds.push(MixerCmd::InstallCurrent(Some(a_consumer)))
            .unwrap();
        let mut out = vec![0.0f32; 2 * 2];
        mixer.render(&mut out);
        assert_eq!(out, vec![0.5, -0.5, 0.5, -0.5]);
        shared.set_muted(true);
        a.push(&[1.0, -1.0]);
        mixer.render(&mut out);
        assert_eq!(&out[..2], &[0.0, 0.0]);
        assert_eq!(
            shared.volume(),
            0.5,
            "mute does not change the stored volume"
        );
    }

    #[test]
    fn installing_a_current_voice_stops_the_previous_decoder() {
        let shared = Arc::new(MixerShared::new(48_000, 1.0));
        let (note_tx, _note_rx) = RingBuffer::new(64);
        let (mut mixer, mut cmds) = Mixer::new(shared, note_tx);
        let (mut a, a_consumer, a_status) = voice_ring(48_000, 0.5, 1, 0.0);
        let (_b, b_consumer, _) = voice_ring(48_000, 0.5, 2, 0.0);
        a.push(&[0.1, 0.1]);
        cmds.push(MixerCmd::InstallCurrent(Some(a_consumer)))
            .unwrap();
        cmds.push(MixerCmd::InstallCurrent(Some(b_consumer)))
            .unwrap();
        let mut out = vec![0.0f32; 2 * 2];
        mixer.render(&mut out);
        assert!(
            a_status.is_stopped(),
            "the replaced decode thread must exit"
        );
        assert!(a.push(&[0.1, 0.1]) > 0, "the ring itself is not closed");
    }
}
