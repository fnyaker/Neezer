//! The output: PipeWire first, ALSA second, and a paced silence pump when
//! neither exists.
//!
//! cpal's `Stream` is created **inside** a dedicated thread and never moves: on
//! several backends it is not `Send`, and a device change needs a new stream on
//! the same thread that dropped the old one. The [`Mixer`] lives inside the
//! callback; to rebuild the stream without losing what is playing, the callback
//! hands it back through a one-slot ring when asked (a move, no allocation, no
//! lock) and the new stream takes it over through another one-slot ring.
//!
//! A machine with no sound server at all (CI, a headless box) still runs the
//! engine: [`start`] falls back to a pump thread that calls the same `render`
//! every period. That is what makes the whole pipeline testable, and it is why
//! the D5 acceptance numbers can be measured without a speaker.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, StreamConfig, SupportedStreamConfig};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::events::{EngineEvent, EventBus, MixerNote};
use crate::mixer::{Mixer, MixerCmd, MixerShared};

/// What the app shows about the output, and what the voice rings are sized for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputInfo {
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// Frames the device holds after a callback returns: what is written but not
    /// yet heard. The clock subtracts it from the position.
    pub latency_frames: u64,
}

/// Which device to open. `Named` matches `DeviceTrait::name`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum OutputChoice {
    #[default]
    Default,
    Named(String),
}

impl OutputChoice {
    pub fn named(name: impl Into<String>) -> Self {
        OutputChoice::Named(name.into())
    }
}

#[derive(Debug)]
pub enum OutputError {
    /// No host answered: no PipeWire, no ALSA, or no output device on either.
    Unavailable(String),
    /// The device exists but refused this configuration.
    Build(String),
}

impl std::fmt::Display for OutputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputError::Unavailable(message) | OutputError::Build(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for OutputError {}

/// Everything the output callback owns. Preallocated: `scratch` holds up to
/// `SCRATCH_FRAMES` stereo frames and is reused for every device layout.
struct Callback {
    /// Arrives from the output thread once the stream is playing, and goes back
    /// there on a device change.
    mixer: Option<Mixer>,
    forward: Consumer<Mixer>,
    back: Producer<Mixer>,
    shared: Arc<MixerShared>,
    /// Interleaved stereo f32, the mixer's only output format.
    scratch: Vec<f32>,
    device_channels: usize,
    sample_rate: u32,
    released: Arc<AtomicBool>,
    /// Whether `playback - callback` timestamps are usable on this backend.
    timestamps: bool,
}

/// 8 192 stereo frames = 170 ms at 48 kHz: far more than any sane device buffer
/// (PipeWire's maximum is 2 048 frames), and allocated once, so the callback can
/// always render in whole chunks of it.
const SCRATCH_FRAMES: usize = 8_192;

impl Callback {
    fn fill<T>(&mut self, data: &mut [T])
    where
        T: Sample + FromSample<f32> + 'static,
    {
        if self.released.load(Ordering::Acquire) {
            if let Some(mixer) = self.mixer.take() {
                let _ = self.back.push(mixer);
            }
            data.fill(T::from_sample(0.0));
            return;
        }
        if self.mixer.is_none()
            && let Ok(mixer) = self.forward.pop()
        {
            self.mixer = Some(mixer);
        }
        let Some(mixer) = self.mixer.as_mut() else {
            data.fill(T::from_sample(0.0));
            return;
        };
        let frames = data.len() / self.device_channels;
        let mut done = 0;
        while done < frames {
            let chunk = (frames - done).min(SCRATCH_FRAMES);
            let scratch = &mut self.scratch[..chunk * 2];
            mixer.render(scratch);
            for frame in 0..chunk {
                let left = scratch[frame * 2];
                let right = scratch[frame * 2 + 1];
                let base = (done + frame) * self.device_channels;
                match self.device_channels {
                    1 => data[base] = T::from_sample(0.5 * (left + right)),
                    _ => {
                        data[base] = T::from_sample(left);
                        data[base + 1] = T::from_sample(right);
                        for extra in 2..self.device_channels {
                            data[base + extra] = T::from_sample(0.0);
                        }
                    }
                }
            }
            done += chunk;
        }
    }

    fn note_latency(&self, info: &cpal::OutputCallbackInfo) {
        if !self.timestamps {
            return;
        }
        let stamp = info.timestamp();
        let delay = stamp.playback.duration_since(stamp.callback);
        let frames = (delay.as_secs_f64() * f64::from(self.sample_rate)).round() as u64;
        if frames > 0 {
            self.shared.clock.set_latency_frames(frames);
        }
    }
}

/// A device for the output thread. Opened on the calling thread so a failure is
/// reported before the engine claims to be playing.
struct Setup {
    name: String,
    device: cpal::Device,
    config: SupportedStreamConfig,
    buffer_frames: u32,
}

/// PipeWire first, ALSA as the fallback (`docs/agents/desktop.md`, *Output*).
fn probe(
    choice: &OutputChoice,
    preferred_rate: u32,
    period_frames: u32,
) -> Result<Setup, OutputError> {
    let mut last = String::new();
    for host_id in [cpal::HostId::PipeWire, cpal::HostId::Alsa] {
        let host = match cpal::host_from_id(host_id) {
            Ok(host) => host,
            Err(e) => {
                last = format!("{} : {e}", host_name(host_id));
                continue;
            }
        };
        let device = match choice {
            OutputChoice::Default => host.default_output_device(),
            OutputChoice::Named(name) => host
                .output_devices()
                .ok()
                .and_then(|mut devices| {
                    devices.find(|device| {
                        device
                            .description()
                            .map(|description| description.name() == name)
                            .unwrap_or(false)
                    })
                })
                .or_else(|| host.default_output_device()),
        };
        let Some(device) = device else {
            last = format!("{} : aucun périphérique de sortie", host_name(host_id));
            continue;
        };
        let name = device
            .description()
            .map(|description| description.name().to_string())
            .unwrap_or_else(|_| "sortie par défaut".into());
        let default = match device.default_output_config() {
            Ok(config) => config,
            Err(e) => {
                last = format!("{name} : {e}");
                continue;
            }
        };
        // Keep the device's rate unless the engine asked for one it supports:
        // the decoder resamples every track to the engine's rate, so the rate
        // must be fixed before the first voice exists.
        let config = if default.sample_rate() == preferred_rate {
            default
        } else {
            match device.supported_output_configs() {
                Ok(ranges) => {
                    let mut chosen = None;
                    for range in ranges {
                        if range.channels() >= 2
                            && let Some(config) = range.try_with_sample_rate(preferred_rate)
                        {
                            chosen = Some(config);
                            break;
                        }
                    }
                    chosen.unwrap_or(default)
                }
                Err(_) => default,
            }
        };
        // The callback period is the engine's 10 ms when the device can do it
        // (PipeWire advertises 32–2048 frames), and the device's own range
        // otherwise.
        let buffer_frames = match config.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => period_frames.max(1).clamp(*min, *max),
            cpal::SupportedBufferSize::Unknown => period_frames.max(1),
        };
        return Ok(Setup {
            name,
            device,
            config,
            buffer_frames,
        });
    }
    Err(OutputError::Unavailable(if last.is_empty() {
        "aucune sortie audio".into()
    } else {
        last
    }))
}

fn host_name(host: cpal::HostId) -> &'static str {
    match host {
        cpal::HostId::PipeWire => "PipeWire",
        cpal::HostId::Alsa => "ALSA",
        // Only reachable on a platform with another compiled-in host (this app
        // targets Linux, where the two above are the whole list).
        #[allow(unreachable_patterns)]
        _ => "hôte audio",
    }
}

/// The command the output thread obeys.
enum OutCmd {
    SetDevice(OutputChoice),
    Stop,
}

/// A device output, rebuilt on demand on its own thread.
pub(crate) struct DeviceOutput {
    cmd: Sender<OutCmd>,
    info: Arc<Mutex<OutputInfo>>,
    join: Option<JoinHandle<()>>,
}

impl DeviceOutput {
    fn set_device(&self, choice: OutputChoice) {
        let _ = self.cmd.send(OutCmd::SetDevice(choice));
    }

    fn info(&self) -> OutputInfo {
        self.info.lock().expect("output info poisoned").clone()
    }
}

fn build_device_output(
    setup: Setup,
    mixer: Mixer,
    shared: Arc<MixerShared>,
    bus: Arc<EventBus>,
) -> DeviceOutput {
    let info = Arc::new(Mutex::new(info_of(&setup)));
    let stored = Arc::clone(&info);
    let (cmd_tx, cmd_rx) = channel();
    let join = thread::Builder::new()
        .name("nz-audio-output".into())
        .stack_size(256 * 1024)
        .spawn(move || run_device(setup, mixer, shared, bus, cmd_rx, stored))
        .expect("spawn the output thread");
    DeviceOutput {
        cmd: cmd_tx,
        info,
        join: Some(join),
    }
}

fn info_of(setup: &Setup) -> OutputInfo {
    OutputInfo {
        device: setup.name.clone(),
        sample_rate: setup.config.sample_rate(),
        channels: setup.config.channels(),
        latency_frames: u64::from(setup.buffer_frames),
    }
}

/// The output thread: one stream at a time, rebuilt on `SetDevice`.
fn run_device(
    mut setup: Setup,
    mut mixer: Mixer,
    shared: Arc<MixerShared>,
    bus: Arc<EventBus>,
    cmd_rx: Receiver<OutCmd>,
    info: Arc<Mutex<OutputInfo>>,
) {
    loop {
        let (released, mut forward_tx, forward_rx) = {
            let (forward_tx, forward_rx) = RingBuffer::new(1);
            (Arc::new(AtomicBool::new(false)), forward_tx, forward_rx)
        };
        let (back_tx, mut back_rx) = RingBuffer::new(1);
        let stream = build_stream(&setup, &shared, &released, forward_rx, back_tx);
        let stream = match stream {
            Ok(stream) => stream,
            Err(e) => {
                bus.emit(EngineEvent::Error {
                    id: None,
                    message: format!("{} : {e}", setup.name),
                });
                // Nothing to play through: wait for a new device or the end. The
                // mixer never left this thread, so a retry loses nothing.
                match cmd_rx.recv() {
                    Ok(OutCmd::SetDevice(choice)) => {
                        let rate = info.lock().expect("output info poisoned").sample_rate;
                        match probe(&choice, rate, setup.buffer_frames) {
                            Ok(new_setup) => {
                                setup = new_setup;
                                *info.lock().expect("output info poisoned") = info_of(&setup);
                            }
                            Err(e) => {
                                bus.emit(EngineEvent::Error {
                                    id: None,
                                    message: e.to_string(),
                                });
                            }
                        }
                        continue;
                    }
                    _ => return,
                }
            }
        };
        shared
            .clock
            .set_latency_frames(u64::from(setup.buffer_frames));
        if let Err(e) = stream.play() {
            bus.emit(EngineEvent::Error {
                id: None,
                message: format!("{} : {e}", setup.name),
            });
            return;
        }
        // Only now does the callback get the mixer: a stream that failed to
        // build can never eat it.
        if forward_tx.push(mixer).is_err() {
            bus.emit(EngineEvent::Error {
                id: None,
                message: "sortie audio indisponible".into(),
            });
            return;
        }
        bus.emit(EngineEvent::OutputChanged {
            device: setup.name.clone(),
            sample_rate: setup.config.sample_rate(),
        });
        loop {
            match cmd_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(OutCmd::Stop) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    released.store(true, Ordering::Release);
                    wait_mixer(&mut back_rx, Duration::from_millis(200));
                    drop(stream);
                    return;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Ok(OutCmd::SetDevice(choice)) => {
                    released.store(true, Ordering::Release);
                    match wait_mixer(&mut back_rx, Duration::from_millis(300)) {
                        Some(returned) => {
                            drop(stream);
                            mixer = returned;
                            let rate = info.lock().expect("output info poisoned").sample_rate;
                            match probe(&choice, rate, setup.buffer_frames) {
                                Ok(new_setup) => {
                                    setup = new_setup;
                                    *info.lock().expect("output info poisoned") = info_of(&setup);
                                }
                                Err(e) => {
                                    bus.emit(EngineEvent::Error {
                                        id: None,
                                        message: e.to_string(),
                                    });
                                    return;
                                }
                            }
                            break;
                        }
                        None => {
                            drop(stream);
                            bus.emit(EngineEvent::Error {
                                id: None,
                                message: "changement de sortie audio impossible".into(),
                            });
                            return;
                        }
                    }
                }
            }
        }
    }
}

/// Ask the callback to give the mixer back, and wait for it.
fn wait_mixer(back: &mut Consumer<Mixer>, timeout: Duration) -> Option<Mixer> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(mixer) = back.pop() {
            return Some(mixer);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

fn build_stream(
    setup: &Setup,
    shared: &Arc<MixerShared>,
    released: &Arc<AtomicBool>,
    forward: Consumer<Mixer>,
    back: Producer<Mixer>,
) -> Result<cpal::Stream, String> {
    let channels = setup.config.channels();
    let sample_rate = setup.config.sample_rate();
    let mut config: StreamConfig = setup.config.config();
    config.buffer_size = cpal::BufferSize::Fixed(setup.buffer_frames);
    let device_channels = usize::from(channels);
    let mut state = Callback {
        mixer: None,
        forward,
        back,
        shared: Arc::clone(shared),
        scratch: vec![0.0; SCRATCH_FRAMES * 2],
        device_channels,
        sample_rate,
        released: Arc::clone(released),
        timestamps: true,
    };
    let errors = Arc::clone(shared);
    let error_callback = move |error: cpal::Error| {
        if error.kind() == cpal::ErrorKind::Xrun {
            // The device itself missed its deadline (not our decoder): count it
            // in the same total, it is a glitch the person heard.
            errors.note_underrun();
        }
    };
    let built = match setup.config.sample_format() {
        SampleFormat::F32 => setup.device.build_output_stream::<f32, _, _>(
            config,
            move |data, info| {
                state.note_latency(info);
                state.fill(data);
            },
            error_callback,
            None,
        ),
        SampleFormat::I16 => setup.device.build_output_stream::<i16, _, _>(
            config,
            move |data, info| {
                state.note_latency(info);
                state.fill(data);
            },
            error_callback,
            None,
        ),
        other => {
            return Err(format!(
                "format d'échantillon non pris en charge : {other:?}"
            ));
        }
    };
    built.map_err(|e| e.to_string())
}

/// A paced pump with no device: the same `render` every period, so the engine
/// runs (and is measured) on a machine with no sound server.
pub(crate) struct NullOutput {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl NullOutput {
    fn start(mut mixer: Mixer, sample_rate: u32, period_frames: u32) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let join = thread::Builder::new()
            .name("nz-audio-null".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let period = Duration::from_secs_f64(
                    f64::from(period_frames.max(1)) / f64::from(sample_rate.max(1)),
                );
                let mut buffer = vec![0.0f32; period_frames.max(1) as usize * 2];
                let mut next = Instant::now();
                while !stopped.load(Ordering::Relaxed) {
                    mixer.render(&mut buffer);
                    next += period;
                    let now = Instant::now();
                    if next > now {
                        thread::sleep(next - now);
                    } else {
                        next = now;
                    }
                }
            })
            .expect("spawn the null output thread");
        Self {
            stop,
            join: Some(join),
        }
    }
}

impl Drop for NullOutput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Pull-driven output for tests and CI: the caller decides when a buffer is
/// rendered, so a gapless seam is a deterministic assertion, not a race with a
/// sound card.
pub struct OfflineSink {
    mixer: Mixer,
    shared: Arc<MixerShared>,
}

impl OfflineSink {
    /// Render `out` (interleaved stereo f32), exactly like a callback would.
    pub fn render(&mut self, out: &mut [f32]) {
        self.mixer.render(out);
    }

    pub fn shared(&self) -> &Arc<MixerShared> {
        &self.shared
    }
}

/// The engine's output, whatever it turned out to be.
pub(crate) enum Output {
    Device(DeviceOutput),
    Null {
        /// Kept for its `Drop`: stopping the pump is what ends its thread.
        _pump: NullOutput,
    },
    /// The mixer lives in the [`OfflineSink`] the caller holds.
    Offline,
}

impl Output {
    pub(crate) fn info(&self) -> Option<OutputInfo> {
        match self {
            Output::Device(device) => Some(device.info()),
            Output::Null { .. } | Output::Offline => None,
        }
    }

    pub(crate) fn set_device(&self, choice: OutputChoice) -> Result<(), OutputError> {
        match self {
            Output::Device(device) => {
                device.set_device(choice);
                Ok(())
            }
            Output::Null { .. } | Output::Offline => Err(OutputError::Unavailable(
                "aucune sortie audio réelle".into(),
            )),
        }
    }
}

impl Drop for DeviceOutput {
    fn drop(&mut self) {
        let _ = self.cmd.send(OutCmd::Stop);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Start the engine's output: the real device when there is one, the pump
/// otherwise. Returns the mixer's command producer with the output.
pub(crate) fn start(
    choice: &OutputChoice,
    preferred_rate: u32,
    volume: f32,
    period_frames: u32,
    bus: Arc<EventBus>,
    notes: Producer<MixerNote>,
) -> (Output, Arc<MixerShared>, Producer<MixerCmd>, OutputInfo) {
    match probe(choice, preferred_rate, period_frames) {
        Ok(setup) => {
            let rate = setup.config.sample_rate();
            let shared = Arc::new(MixerShared::new(rate, volume));
            let (mixer, cmds) = Mixer::new(Arc::clone(&shared), notes);
            let info = info_of(&setup);
            let output = build_device_output(setup, mixer, Arc::clone(&shared), bus);
            (Output::Device(output), shared, cmds, info)
        }
        Err(_) => {
            let shared = Arc::new(MixerShared::new(preferred_rate, volume));
            let (mixer, cmds) = Mixer::new(Arc::clone(&shared), notes);
            let pump = NullOutput::start(mixer, preferred_rate, period_frames);
            let info = OutputInfo {
                device: "sans sortie audio".into(),
                sample_rate: preferred_rate,
                channels: 2,
                latency_frames: u64::from(period_frames),
            };
            (Output::Null { _pump: pump }, shared, cmds, info)
        }
    }
}

/// The pull-driven variant of [`start`], for tests: the caller gets the sink and
/// drives the clock.
pub(crate) fn start_offline(
    sample_rate: u32,
    volume: f32,
    notes: Producer<MixerNote>,
) -> (Output, OfflineSink, Arc<MixerShared>, Producer<MixerCmd>) {
    let shared = Arc::new(MixerShared::new(sample_rate, volume));
    let (mixer, cmds) = Mixer::new(Arc::clone(&shared), notes);
    let sink = OfflineSink {
        mixer,
        shared: Arc::clone(&shared),
    };
    (Output::Offline, sink, shared, cmds)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The host order is the contract: PipeWire first, ALSA as the fallback
    /// (see the note's *Output* row). This machine and CI both compile both.
    #[test]
    fn pipewire_is_tried_before_alsa() {
        assert_eq!(cpal::ALL_HOSTS[0], cpal::HostId::PipeWire);
        assert!(cpal::ALL_HOSTS.contains(&cpal::HostId::Alsa));
    }

    fn callback_with_voice(device_channels: usize, samples: &[f32], generation: u64) -> Callback {
        let shared = Arc::new(MixerShared::new(48_000, 1.0));
        let (notes, _notes_rx) = RingBuffer::new(8);
        let (mixer, mut cmds) = Mixer::new(Arc::clone(&shared), notes);
        let (mut producer, consumer, _status) =
            crate::voice::voice_ring(48_000, 0.5, generation, 0.0);
        assert_eq!(producer.push(samples), samples.len());
        cmds.push(MixerCmd::InstallCurrent(Some(consumer))).unwrap();
        shared.set_paused(false);
        let (back, _back_rx) = RingBuffer::new(1);
        Callback {
            mixer: Some(mixer),
            forward: RingBuffer::new(1).1,
            back,
            shared,
            scratch: vec![0.0; SCRATCH_FRAMES * 2],
            device_channels,
            sample_rate: 48_000,
            released: Arc::new(AtomicBool::new(false)),
            timestamps: false,
        }
    }

    /// 1.0 and 0.0 average to 0.5: the mono down-mix is 0.5·(L+R), the same law
    /// the web player's `pan.js` uses. Extra channels are silent, never copies
    /// of the left one.
    #[test]
    fn a_mono_device_gets_the_down_mix_and_extra_channels_silence() {
        let mut callback = callback_with_voice(1, &[1.0, 0.0], 1);
        let mut mono = [0i16; 2];
        callback.fill(&mut mono);
        assert_eq!(mono[0], i16::from_sample(0.5f32));

        let mut callback = callback_with_voice(4, &[0.25, -0.25], 2);
        let mut quad = [0i16; 4];
        callback.fill(&mut quad);
        assert_eq!(quad[0], i16::from_sample(0.25f32));
        assert_eq!(quad[1], i16::from_sample(-0.25f32));
        assert_eq!(quad[2], 0);
        assert_eq!(quad[3], 0);
    }

    #[test]
    fn a_buffer_larger_than_the_scratch_still_renders_every_frame() {
        let frames = SCRATCH_FRAMES + 1_000;
        let mut callback = callback_with_voice(2, &[1.0, -1.0], 3);
        // One frame of audio, then the rest is (correctly) silence.
        let mut out = vec![0f32; frames * 2];
        callback.fill(&mut out);
        assert_eq!(out[0], 1.0);
        assert_eq!(out[1], -1.0);
        assert_eq!(out[2], 0.0);
    }
}
