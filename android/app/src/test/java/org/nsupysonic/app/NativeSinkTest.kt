package org.nsupysonic.app

import android.media.AudioFormat
import androidx.annotation.OptIn
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.audio.DefaultAudioSink
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The AudioTrack buffer is the margin the playback thread has between two
 * writes. Media3 1.4.1 sizes it as 4x the platform minimum, constrained to
 * 250..750 ms, which on most phones lands on the **250 ms floor** — the buffer
 * the native player used, and the reason a WebView/GC/decoder hiccup was an
 * audible cut. NativeSink asks for 500 ms..1 s instead; these are the exact
 * numbers, at 48 kHz stereo 16-bit (4 bytes per frame).
 */
@OptIn(UnstableApi::class)
class NativeSinkTest {
    private fun bytes(provider: DefaultAudioSink.AudioTrackBufferSizeProvider, min: Int): Int =
        provider.getBufferSizeInBytes(
            min,
            AudioFormat.ENCODING_PCM_16BIT,
            DefaultAudioSink.OUTPUT_MODE_PCM,
            4, // bytes per frame: 16-bit stereo
            48_000, // Hz
            0, // bitrate, unused for PCM
            1.0, // max playback speed multiplier
        )

    @Test
    fun `the default floor is a quarter of a second`() {
        // 3 840 B is ~20 ms at 48 kHz stereo: a platform minimum on the phones
        // where Media3's default ends up on its 250 ms floor (48 000 B). This is
        // what the native player used before NativeSink existed.
        assertEquals(48_000, bytes(DefaultAudioSink.AudioTrackBufferSizeProvider.DEFAULT, 3_840))
    }

    @Test
    fun `a phone minimum gets half a second now`() {
        assertEquals(96_000, bytes(NativeSink.provider(), 3_840))
    }

    @Test
    fun `a large platform minimum is capped at one second`() {
        // 120 000 B is 625 ms at 48 kHz stereo; x4 (1.25 s) is above the cap.
        assertEquals(192_000, bytes(NativeSink.provider(), 120_000))
    }
}
