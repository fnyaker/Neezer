// :playback — the API-26 output (lot MB3a, docs/agents/mobile.md "MB3a" and
// "Threads"; the buffer lesson is android.md bridge 12).
package fr.neezer.mobile.playback

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import android.os.Handler
import android.os.Looper
import android.os.Process
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * The API-26 output: **one** thread pulls PCM from the Rust engine and hands it to
 * an [AudioTrack] in [AudioTrack.MODE_STREAM].
 *
 * Why Android 8.0 gets this and not AAudio: AAudio exists from API 26 but can
 * crash after `close` on 8.0 (Oboe #40), so the engine's AAudio path starts at 27
 * (`nz_droid_audio::output_for`) and 8.0 pulls here instead — same mixer, same
 * clock, only the device side differs.
 *
 * The rules this file exists to keep, each paid for by the old app's "the sound
 * cuts for a few milliseconds" (android.md, bridge 12):
 *
 * - **One thread, real-time priority**, `MODE_STREAM`, `ENCODING_PCM_FLOAT` — the
 *   engine renders stereo f32 and nothing else has to convert it;
 * - **a buffer of at least 500 ms** ([bufferBytes] takes the larger of 500 ms and
 *   the platform's own minimum): a music player wants few wake-ups, and 250 ms of
 *   margin is what a normal-priority thread needs to survive a GC or a decoder
 *   burst;
 * - **nothing allocates while playing**: the direct [ByteBuffer] is allocated
 *   once, before the loop, and reused for every block;
 * - **the blocking write is the clock**: half a buffer per `nativeRender`, so the
 *   loop wakes about four times a second and the Rust callback is never called
 *   faster than the device consumes;
 * - **the device's own numbers reach the engine**: [AudioTrack.getUnderrunCount]
 *   (a TOTAL per track, accumulated here across the tracks a route change
 *   re-creates) and the buffer the track really has, which is what the position
 *   subtracts;
 * - **a route change is handled OUT of the audio loop**: the
 *   [AudioDeviceCallback] only raises a flag, the loop releases its track and opens
 *   a new one, and the engine's mixer — and therefore the song — keeps playing.
 *
 * The `handle` is a `Box<PullSession>` from Rust; [close] gives it back exactly
 * once. The natives are instance methods (`nativeRender(handle, …)`), which is why
 * their JNI names carry no `$Companion`.
 */
class AudioTrackPull(
    private val handle: Long,
    private val sampleRate: Int = DEFAULT_SAMPLE_RATE,
    private val bufferMs: Int = DEFAULT_BUFFER_MS,
    private val onFailure: (String) -> Unit = {},
) : AutoCloseable {
    @Volatile
    private var running = false

    @Volatile
    private var recreate = false

    /** The live track, for [close] to unblock a write on a wedged device. */
    @Volatile
    private var track: AudioTrack? = null

    private var thread: Thread? = null
    private var closed = false
    private var audioManager: AudioManager? = null
    private var devices: AudioDeviceCallback? = null

    /**
     * Frames in one `nativeRender`: **an eighth of the buffer** (62.5 ms for the
     * 500 ms default).
     *
     * Half the buffer looked cheaper — four wake-ups a second — and measured
     * **3 mixer starvations in 30 s** on the emulator: the engine's decoded ring is
     * half a second, so asking for a quarter second in one call leaves only a
     * quarter second for the decoder to be descheduled. An eighth leaves 437 ms,
     * and 16 wake-ups a second of a 3 000-frame copy is nothing (the device buffer
     * the person hears is still 500 ms).
     */
    private val framesPerWrite: Int =
        (sampleRate.toLong() * bufferMs / 8_000L).toInt().coerceAtLeast(1)

    /**
     * Start pulling.
     *
     * [context] is only used to hear about audio devices going away (a headset
     * unplugged, a Bluetooth sink connected): the application context is kept,
     * never the activity. `null` starts the thread without that watcher — what a
     * headless diagnostic (or a bench) does, since it has no `Context`.
     */
    fun start(context: Context?) {
        if (running || closed) return
        running = true
        if (context != null) registerDeviceCallback(context.applicationContext)
        thread = Thread({ loop() }, "nz-audiotrack").apply {
            isDaemon = true
            start()
        }
    }

    /**
     * Ask the loop to release its `AudioTrack` and open a new one on the device
     * that is there now. The thread picks it up at its next block (62.5 ms for the
     * 500 ms default), so the caller — a device callback, a diagnostic, an output
     * switch — never touches the audio thread.
     *
     * The engine's mixer is not touched: the song continues, only the device side
     * starts over (with one block queued before `play()`, so the switch is not a
     * click).
     */
    fun reopen() {
        recreate = true
    }

    /** Stop, join the thread, and release the Rust session. Safe to call twice. */
    override fun close() {
        if (closed) return
        closed = true
        running = false
        unregisterDeviceCallback()
        val worker = thread
        thread = null
        if (worker != null) {
            worker.join(JOIN_MS)
            if (worker.isAlive) {
                // A blocking write lasts at most one buffer; if it has not returned,
                // the device is wedged — pause it and give the loop a second chance.
                runCatching { track?.pause() }
                worker.join(JOIN_MS)
            }
            if (worker.isAlive) {
                // Never free a handle a live thread may still render into: leaking
                // one session is better than a use-after-free.
                return
            }
        }
        if (handle != 0L) nativeClose(handle)
    }

    private fun loop() {
        Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO)
        val frames = framesPerWrite
        val bytes = frames * BYTES_PER_FRAME
        // Allocated ONCE: the audio thread must not allocate (the Rust side counts
        // its own allocations too, tests/callback.rs).
        val buffer = ByteBuffer.allocateDirect(bytes).order(ByteOrder.nativeOrder())
        var underrunsBase = 0

        while (running) {
            val created = try {
                open()
            } catch (refused: IllegalStateException) {
                fail(refused.message ?: "AudioTrack refusé")
                return
            }
            track = created
            var primed = false
            var underrunsSeen = 0
            try {
                while (running) {
                    if (recreate) {
                        recreate = false
                        break
                    }
                    buffer.clear()
                    val rendered = nativeRender(handle, buffer, frames)
                    if (rendered != frames) {
                        fail("le moteur a refusé un bloc ($rendered trames)")
                        return
                    }
                    val written = created.write(buffer, bytes, AudioTrack.WRITE_BLOCKING)
                    if (written < 0) {
                        if (written == AudioTrack.ERROR_DEAD_OBJECT) break // the device went away
                        fail("écriture AudioTrack refusée ($written)")
                        return
                    }
                    if (!primed) {
                        // Play with one block ALREADY queued: a track that starts
                        // empty counts an underrun the instant the HAL asks for
                        // data, and that is the click the person hears at every
                        // route change (measured: 1 per track, 0 with this).
                        created.play()
                        primed = true
                        // What the track really holds is what is written but not
                        // yet heard: the engine's clock subtracts it.
                        nativeSetLatency(handle, created.bufferSizeInFrames.toLong())
                    }
                    val underruns = created.underrunCount
                    if (underruns > underrunsSeen) {
                        underrunsSeen = underruns
                        nativeUnderruns(handle, (underrunsBase + underruns).toLong())
                    }
                }
            } finally {
                underrunsBase += underrunsSeen
                track = null
                runCatching { created.pause() }
                runCatching { created.flush() }
                created.release()
            }
        }
    }

    /** A fresh `AudioTrack` at the engine's rate, already playing. */
    private fun open(): AudioTrack {
        val attributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
            .build()
        val format = AudioFormat.Builder()
            .setEncoding(AudioFormat.ENCODING_PCM_FLOAT)
            .setSampleRate(sampleRate)
            .setChannelMask(AudioFormat.CHANNEL_OUT_STEREO)
            .build()
        val created = AudioTrack.Builder()
            .setAudioAttributes(attributes)
            .setAudioFormat(format)
            .setTransferMode(AudioTrack.MODE_STREAM)
            .setBufferSizeInBytes(bufferBytes(sampleRate, bufferMs))
            .build()
        if (created.state != AudioTrack.STATE_INITIALIZED) {
            created.release()
            throw IllegalStateException("AudioTrack non initialisé (état ${created.state})")
        }
        // NOT played here: the loop writes one block first (see `primed`).
        return created
    }

    private fun fail(cause: String) {
        running = false
        onFailure(cause)
    }

    private fun registerDeviceCallback(context: Context) {
        if (devices != null) return
        val manager = context.getSystemService(AudioManager::class.java) ?: return
        val callback = object : AudioDeviceCallback() {
            override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>) {
                // A flag, never work: this runs on the main thread.
                if (addedDevices.isNotEmpty()) reopen()
            }

            override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>) {
                if (removedDevices.isNotEmpty()) reopen()
            }
        }
        manager.registerAudioDeviceCallback(callback, Handler(Looper.getMainLooper()))
        audioManager = manager
        devices = callback
    }

    private fun unregisterDeviceCallback() {
        val manager = audioManager
        val callback = devices
        audioManager = null
        devices = null
        if (manager != null && callback != null) manager.unregisterAudioDeviceCallback(callback)
    }

    private external fun nativeRender(handle: Long, buffer: ByteBuffer, frames: Int): Int

    private external fun nativeSetLatency(handle: Long, frames: Long)

    private external fun nativeUnderruns(handle: Long, total: Long)

    private external fun nativeClose(handle: Long)

    companion object {
        /** What the voices are decoded at; the engine's `EngineConfig::sample_rate`. */
        const val DEFAULT_SAMPLE_RATE = 48_000

        /** The old app's measured NativeSink lesson: never below half a second. */
        const val DEFAULT_BUFFER_MS = 500

        /** Interleaved stereo f32. */
        const val BYTES_PER_FRAME = 8

        private const val JOIN_MS = 1_000L

        init {
            // The app's one native library (MB0): the engine, the bridge and these
            // four entry points. Loading it here keeps the class usable from a
            // diagnostic too.
            System.loadLibrary("nz_ffi")
        }

        /**
         * The bytes to ask `AudioTrack` for: **500 ms or the platform's own
         * minimum, whichever is larger**.
         *
         * Below the minimum the track refuses the buffer, and below 500 ms a
         * normal-priority thread starves on the first GC (bridge 12's "cuts every
         * few seconds"). Asking for MORE than 500 ms costs nothing but the audio
         * already written when a pause or a seek flushes the track, which is silent
         * — one buffer is enough here because the write blocks.
         */
        @JvmStatic
        fun bufferBytes(sampleRate: Int, bufferMs: Int): Int {
            val wanted = (sampleRate.toLong() * bufferMs / 1_000L * BYTES_PER_FRAME).toInt()
            val minimum = AudioTrack.getMinBufferSize(
                sampleRate,
                AudioFormat.CHANNEL_OUT_STEREO,
                AudioFormat.ENCODING_PCM_FLOAT,
            )
            return if (minimum > 0) maxOf(wanted, minimum) else wanted
        }
    }
}
