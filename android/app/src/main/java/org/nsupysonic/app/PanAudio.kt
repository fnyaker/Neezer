package org.nsupysonic.app

import androidx.annotation.OptIn
import androidx.media3.common.C
import androidx.media3.common.audio.AudioProcessor
import androidx.media3.common.audio.BaseAudioProcessor
import androidx.media3.common.util.UnstableApi
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.roundToInt
import kotlin.math.sin

/** Mono and left/right balance for the app's own player: lib/audio/pan.js, in the sink. */
class PcmPan {
    @Volatile private var wantMono = false
    @Volatile private var wantBalance = 0f

    private var rate = 0f
    private var pan = 0f
    private var wet = 0f
    private var a = 1f
    private var b = 0f
    private var c = 0f
    private var d = 1f
    private var da = 0f
    private var db = 0f
    private var dc = 0f
    private var dd = 0f
    private var ta = 1f
    private var tb = 0f
    private var tc = 0f
    private var td = 1f
    private var left = 0

    fun set(mono: Boolean, balance: Float) {
        wantMono = mono
        wantBalance = if (balance.isFinite()) balance.coerceIn(-1f, 1f) else 0f
    }

    val engaged: Boolean
        get() = wantMono || wantBalance != 0f

    val active: Boolean
        get() = engaged || pan != 0f || wet != 0f

    fun configure(sampleRate: Int) {
        rate = sampleRate.toFloat().coerceAtLeast(1f)
        left = 0
    }

    fun reset() {
        pan = 0f
        wet = 0f
        a = 1f
        b = 0f
        c = 0f
        d = 1f
        da = 0f
        db = 0f
        dc = 0f
        dd = 0f
        left = 0
    }

    private fun coefficientAt(p: Float, w: Float) {
        val half = PI.toFloat() / 2f
        val xm = ((p + 1f) * 0.5f) * half
        val glM = cos(xm)
        val grM = sin(xm)
        var sa: Float
        var sb: Float
        var sc: Float
        var sd: Float
        if (p <= 0f) {
            val x = (p + 1f) * half
            sa = 1f
            sb = cos(x)
            sc = 0f
            sd = sin(x)
        } else {
            val x = p * half
            sa = cos(x)
            sb = 0f
            sc = sin(x)
            sd = 1f
        }
        ta = sa * (1f - w) + 0.5f * glM * w
        tb = sb * (1f - w) + 0.5f * glM * w
        tc = sc * (1f - w) + 0.5f * grM * w
        td = sd * (1f - w) + 0.5f * grM * w
    }

    private fun advance() {
        val panTo = wantBalance
        val panStep = BLOCK / (0.012f * rate)
        pan = when {
            pan < panTo -> (pan + panStep).coerceAtMost(panTo)
            pan > panTo -> (pan - panStep).coerceAtLeast(panTo)
            else -> panTo
        }
        val wetTo = if (wantMono) 1f else 0f
        val wetStep = BLOCK / (0.048f * rate)
        wet = when {
            wet < wetTo -> (wet + wetStep).coerceAtMost(wetTo)
            wet > wetTo -> (wet - wetStep).coerceAtLeast(wetTo)
            else -> wetTo
        }
        coefficientAt(pan, wet)
        da = (ta - a) / BLOCK
        db = (tb - b) / BLOCK
        dc = (tc - c) / BLOCK
        dd = (td - d) / BLOCK
        left = BLOCK
    }

    fun process(buffer: ByteBuffer, frames: Int) {
        if (rate <= 0f) return
        val src = buffer.order(ByteOrder.nativeOrder())
        val at = src.position()
        for (i in 0 until frames) {
            if (left == 0) advance()
            a += da
            b += db
            c += dc
            d += dd
            left--
            val o = at + i * 4
            val l = src.getShort(o).toFloat()
            val r = src.getShort(o + 2).toFloat()
            var yl = a * l + b * r
            var yr = c * l + d * r
            if (yl > 32767f) yl = 32767f else if (yl < -32768f) yl = -32768f
            if (yr > 32767f) yr = 32767f else if (yr < -32768f) yr = -32768f
            src.putShort(o, yl.roundToInt().toShort())
            src.putShort(o + 2, yr.roundToInt().toShort())
        }
    }

    companion object {
        private const val BLOCK = 32
    }
}

@OptIn(UnstableApi::class)
fun panSuits(encoding: Int, channels: Int): Boolean =
    encoding == C.ENCODING_PCM_16BIT && channels == 2

@OptIn(UnstableApi::class)
class PanProcessor(
    private val mono: () -> Boolean,
    private val balance: () -> Float,
) : BaseAudioProcessor() {
    val pcm = PcmPan()

    override fun onConfigure(inputAudioFormat: AudioProcessor.AudioFormat): AudioProcessor.AudioFormat {
        if (!panSuits(inputAudioFormat.encoding, inputAudioFormat.channelCount)) {
            return AudioProcessor.AudioFormat.NOT_SET
        }
        pcm.configure(inputAudioFormat.sampleRate)
        return inputAudioFormat
    }

    override fun queueInput(inputBuffer: ByteBuffer) {
        val size = inputBuffer.remaining()
        if (size == 0) return
        pcm.set(mono(), balance())
        val out = replaceOutputBuffer(size)
        out.put(inputBuffer)
        out.flip()
        if (!pcm.active) return
        pcm.process(out, size / 4)
    }

    override fun onFlush() = pcm.reset()

    override fun onReset() = pcm.reset()
}
