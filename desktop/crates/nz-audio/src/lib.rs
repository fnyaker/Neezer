//! The audio engine: source, decode, mix (gapless and crossfade), DSP, output
//! and the analysis tap.
//!
//! Owned by lots D5 and D6 (`docs/agents/desktop.md`). D5 is this document's
//! *D5* lot: a track plays through PipeWire from the network or from a file,
//! gaplessly into the next, and a seek is answered by the server or explicitly
//! deferred. D6 adds the web player's sound on top (DSP chain, crossfade,
//! silence trim, podcast speed, analysis tap).
//!
//! The shape, in one paragraph: [`Engine`] is a cheap handle; every blocking
//! step (open a source, probe a container, seek) runs on a loader thread, and
//! every sample is decoded on a per-voice thread into a 0.5 s `rtrb` ring. The
//! output callback owns the `Mixer`, which pops the current voice and, when it
//! ends, the preloaded next one **in the same buffer** — the gapless seam. No
//! callback ever allocates, locks or waits.
//!
//! Two outputs, one mixer: the desktop's own device through cpal (the `cpal`
//! feature, on by default: [`Engine::new`]), and an [`ExternalSink`] a platform
//! hands to ITS real-time callback ([`Engine::new_external`], Android's AAudio
//! and `AudioTrack`). With `default-features = false` nothing that names cpal is
//! compiled at all, so the crate builds for a target that has no PipeWire and no
//! ALSA (`docs/agents/mobile.md`, lot SC2). `new_offline` is `new_external` with
//! a test's name on it: what a test renders is what a phone renders.

mod clock;
mod crossfade;
mod decode;
mod dsp;
mod engine;
mod events;
mod mixer;
mod output;
mod party;
mod source;
mod stretch;
mod tap;
mod trim;
mod voice;

pub use clock::Clock;
pub use crossfade::{FadeSettings, equal_power};
pub use dsp::{Normalization, Settings, gain_db};
pub use engine::{
    Engine, EngineConfig, EngineError, EngineHandle, OutputClock, PartyChannel, TrackMeta,
};
pub use events::EngineEvent;
pub use mixer::MixerShared;
pub use nscore::dsp::{ChainMeter, METER_FLOOR_DB};
pub use output::{
    ExternalSink, OfflineSink, OutputChoice, OutputError, OutputInfo, is_default_output_alias,
    output_devices,
};
pub use party::{EDGE_S, SLOTS as PARTY_SLOTS, Segment};
pub use source::{ByteStream, Lane, Quality, SourceError, StreamOpener, StreamReady, TrackId};
pub use stretch::{MAX_SPEED, MIN_SPEED, clamp_speed};
pub use tap::{MAX_TAP_READERS, TAP_FRAMES, TapFeed, TapReader, TapSubscription, tap_channel};
pub use trim::{EDGE_LENGTH_SLACK, Edges, MAX_EDGE_TRIM, Trim, trim_for};

#[cfg(all(test, feature = "cpal"))]
mod tests {
    use symphonia::core::codecs::registry::RegisterableAudioDecoder;

    /// `HostId::PipeWire` only exists when cpal was built with its `pipewire`
    /// feature: this assertion fails to COMPILE if the pin loses it, which is
    /// exactly the mistake CI must never let through (rule 1's output path).
    #[test]
    fn cpal_is_built_with_its_pipewire_and_alsa_hosts() {
        assert!(cpal::ALL_HOSTS.contains(&cpal::HostId::PipeWire));
        assert!(cpal::ALL_HOSTS.contains(&cpal::HostId::Alsa));
    }

    /// The Opus decoder is an adapter over a bundled libopus; asking it which
    /// codecs it serves links the C library into this test binary, so a broken
    /// bundled build fails CI here and not on the person's laptop.
    #[test]
    fn the_bundled_libopus_adapter_is_linked() {
        let codecs =
            <symphonia_adapter_libopus::OpusDecoder as RegisterableAudioDecoder>::supported_codecs(
            );
        assert!(!codecs.is_empty());
    }
}
