//! The audio engine: source, decode, mix (gapless and crossfade), DSP, output
//! and the analysis tap.
//!
//! Owned by lots D5 and D6 (`docs/agents/desktop.md`). This is D0's skeleton;
//! its tests are the ones that must stay true for the whole crate's life: the
//! output backend is cpal with its PipeWire host, Opus comes from the bundled
//! libopus (symphonia has no native decoder), and the decode→callback handoff
//! is an `rtrb` ring that never allocates in the callback.

#[cfg(test)]
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

    /// 0.5 s of stereo at 48 kHz per voice, the ring D5 hands to the output
    /// callback: writing and reading it must not allocate on the hot path.
    #[test]
    fn an_half_second_voice_ring_round_trips() {
        let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(48_000 * 2 / 2);
        producer.push(0.25).expect("space");
        producer.push(-0.25).expect("space");
        assert_eq!(consumer.pop().expect("sample"), 0.25);
        assert_eq!(consumer.pop().expect("sample"), -0.25);
    }
}
