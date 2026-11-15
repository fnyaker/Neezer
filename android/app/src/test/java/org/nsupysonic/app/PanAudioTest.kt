package org.nsupysonic.app

import androidx.annotation.OptIn
import androidx.media3.common.C
import androidx.media3.common.audio.AudioProcessor
import androidx.media3.common.util.UnstableApi
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.sin

/**
 * The mono fold and the balance of the app's own player (PanAudio.kt), held to
 * the W3C algorithm lib/audio/pan.js already runs: the same laws, the same
 * level, and a ramp instead of a step.
 */
@OptIn(UnstableApi::class)
class PanAudioTest {
    private fun buffer(frames: Int, left: Short, right: Short): ByteBuffer {
        val buf = ByteBuffer.allocate(frames * 4).order(ByteOrder.nativeOrder())
        for (i in 0 until frames) {
            buf.putShort(left)
            buf.putShort(right)
        }
        buf.flip()
        return buf
    }

    private fun out(buf: ByteBuffer, frames: Int): ShortArray {
        val src = buf.order(ByteOrder.nativeOrder())
        val a = ShortArray(frames * 2)
        for (i in 0 until frames * 2) a[i] = src.getShort(i * 2)
        return a
    }

    private fun pcm(mono: Boolean, balance: Float, frames: Int, left: Short, right: Short): ShortArray {
        val p = PcmPan()
        p.configure(48_000)
        p.set(mono, balance)
        val buf = buffer(frames, left, right)
        p.process(buf, frames)
        return out(buf, frames)
    }

    @Test
    fun centreAndStereoIsTheSignalItself() {
        val a = pcm(false, 0f, 4_800, 12_345, -6_789)
        for (i in 0 until a.size step 2) {
            assertEquals(12_345, a[i].toInt())
            assertEquals(-6_789, a[i + 1].toInt())
        }
    }

    @Test
    fun monoIsTheSpecDownmixAtTheConstantPowerCentre() {
        val a = pcm(true, 0f, 4_800, 10_000, 0)
        val expected = 0.5 * 10_000 * cos(PI / 4)
        val left = a[a.size - 2].toInt()
        assertTrue("left $left ~ $expected", abs(left - expected) <= 2.0)
        assertEquals("the same signal in both ears", left, a[a.size - 1].toInt())
        // Hard left: the whole down-mix (0.5·(L+R)) on the left, nothing on the right.
        val hardLeft = pcm(true, -1f, 4_800, 10_000, 0)
        assertEquals(5_000, hardLeft[hardLeft.size - 2].toInt())
        assertEquals(0, hardLeft[hardLeft.size - 1].toInt())
    }

    @Test
    fun aStereoBalanceOfTheSameLeftUsesTheFarChannelOfTheSpec() {
        val a = pcm(false, 1f, 4_800, 10_000, 4_000)
        assertEquals("hard right: nothing on the left", 0, a[a.size - 2].toInt())
        assertEquals("and the left folded into the right", 14_000, a[a.size - 1].toInt())
        val half = pcm(false, -0.5f, 4_800, 10_000, 10_000)
        val x = 0.5 * PI / 2
        assertEquals(10_000 + 10_000 * cos(x), half[half.size - 2].toDouble(), 2.0)
        assertEquals(10_000 * sin(x), half[half.size - 1].toDouble(), 2.0)
    }

    @Test
    fun engagingMonoRampsInsteadOfStepping() {
        val p = PcmPan()
        p.configure(48_000)
        val frames = 4_800
        val dry = buffer(frames, 20_000, 0)
        p.set(false, 0f)
        p.process(dry, frames)
        assertEquals("nothing applied yet", 20_000, out(dry, frames)[(frames - 1) * 2].toInt())

        p.set(true, 0f)
        val wet = buffer(frames, 20_000, 0)
        p.process(wet, frames)
        val a = out(wet, frames)
        var previous = 20_000
        var biggest = 0
        for (i in 0 until frames) {
            biggest = maxOf(biggest, abs(a[i * 2] - previous))
            previous = a[i * 2].toInt()
        }
        assertTrue("the fold moves a few LSB a sample at most (was $biggest)", biggest < 32)
        assertTrue("and it ends on the fold ($previous)", previous in 7_060..7_080)
    }

    @Test
    fun disengagingRunsToTheCentreAndThenStops() {
        val p = PcmPan()
        p.configure(48_000)
        p.set(true, 0f)
        val frames = 4_800
        p.process(buffer(frames, 10_000, 0), frames)
        assertTrue(p.active)
        p.set(false, 0f)
        assertTrue("still ramping out", p.active)
        val back = buffer(frames, 10_000, 0)
        p.process(back, frames)
        p.set(false, 0f)
        val settled = buffer(frames, 10_000, 0)
        p.process(settled, frames)
        val a = out(settled, frames)
        assertEquals(10_000, a[(frames - 1) * 2].toInt())
        assertEquals(0, a[(frames - 1) * 2 + 1].toInt())
        assertFalse("back to a plain stereo signal", p.active)
    }

    @Test
    fun aClippingFoldIsClampedNotWrapped() {
        val a = pcm(true, -1f, 4_800, Short.MAX_VALUE, Short.MAX_VALUE)
        assertEquals(32767, a[a.size - 2].toInt())
        assertEquals(0, a[a.size - 1].toInt())
    }

    @Test
    fun only16BitStereoIsProcessed() {
        assertTrue(panSuits(C.ENCODING_PCM_16BIT, 2))
        assertFalse(panSuits(C.ENCODING_PCM_16BIT, 1))
        assertFalse(panSuits(C.ENCODING_PCM_16BIT, 6))
        assertFalse(panSuits(C.ENCODING_PCM_FLOAT, 2))
    }

    @Test
    fun anIdleProcessorIsBitForBitTransparent() {
        val pan = PanProcessor({ false }, { 0f })
        val fmt = AudioProcessor.AudioFormat(48_000, 2, C.ENCODING_PCM_16BIT)
        assertEquals(fmt, pan.configure(fmt))
        val frames = 1_024
        val samples = ShortArray(frames * 2) { (((it * 37) % 2000) - 1000).toShort() }
        val buf = ByteBuffer.allocateDirect(frames * 4).order(ByteOrder.nativeOrder())
        for (v in samples) buf.putShort(v)
        buf.flip()
        pan.queueInput(buf)
        val out = pan.getOutput() ?: throw AssertionError("no output")
        val got = ShortArray(out.remaining() / 2)
        val src = out.order(ByteOrder.nativeOrder())
        for (i in got.indices) got[i] = src.getShort(i * 2)
        assertArrayEquals("mono off and balance at the centre: the very same samples", samples, got)
        assertEquals("every byte was consumed", 0, buf.remaining())
    }

    @Test
    fun consecutiveBuffersLoseNoSampleAndAddNone() {
        val pan = PanProcessor({ true }, { 0.4f })
        pan.configure(AudioProcessor.AudioFormat(48_000, 2, C.ENCODING_PCM_16BIT))
        var fed = 0
        var drained = 0
        repeat(20) {
            val frames = 512
            val buf = ByteBuffer.allocateDirect(frames * 4).order(ByteOrder.nativeOrder())
            repeat(frames) {
                buf.putShort(1_000)
                buf.putShort(-1_000)
            }
            buf.flip()
            pan.queueInput(buf)
            fed += frames
            val out = pan.getOutput() ?: return@repeat
            drained += out.remaining() / 4
        }
        assertEquals("one frame in, one frame out, on every buffer", fed, drained)
    }
}
