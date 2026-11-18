// :playback — the audio-focus request the service holds (docs/agents/mobile.md,
// "MB3c": one `AudioFocusRequest`, `AUDIOFOCUS_GAIN`; loss → pause; transient →
// pause then resume on gain; can-duck → `set_ducked(true)`).
package fr.neezer.mobile.playback

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import android.util.Log

/**
 * ONE focus request for the whole app (android.md's "one audio focus request for
 * every player": the old app's crossfade played two), and the decision table
 * lives in [FocusPolicy] so it is a JVM test.
 *
 * The platform's own [AudioFocusRequest] and not `AudioFocusRequestCompat`: the
 * app's `minSdk` is 26 and the compat shim exists for 16+, so it would add a
 * dependency to say exactly the same thing.
 */
class FocusController(
    context: Context,
    private val wasPlaying: () -> Boolean,
    private val apply: (FocusAction) -> Unit,
) {
    private val audio = context.getSystemService(AudioManager::class.java)
    private val handler = Handler(Looper.getMainLooper())

    private var request: AudioFocusRequest? = null
    private var ducked = false
    private var resumeOnGain = false

    /** `true` while OUR pause is the focus's doing: the request must survive it. */
    var pausedForFocus: Boolean = false
        private set

    val held: Boolean get() = request != null

    private val listener = AudioManager.OnAudioFocusChangeListener { change ->
        val event =
            when (change) {
                AudioManager.AUDIOFOCUS_GAIN -> FocusEvent.GAIN
                AudioManager.AUDIOFOCUS_LOSS -> FocusEvent.LOSS
                AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> FocusEvent.LOSS_TRANSIENT
                AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> FocusEvent.LOSS_TRANSIENT_CAN_DUCK
                else -> null
            } ?: return@OnAudioFocusChangeListener
        val action =
            FocusPolicy.on(event, wasPlaying(), ducked, resumeOnGain)
        ducked = action.ducked ?: ducked
        resumeOnGain = action.resumeOnGain
        when (event) {
            FocusEvent.LOSS -> {
                pausedForFocus = false
                abandon()
            }
            FocusEvent.LOSS_TRANSIENT -> pausedForFocus = action.play == false
            FocusEvent.GAIN -> pausedForFocus = false
            FocusEvent.LOSS_TRANSIENT_CAN_DUCK -> Unit
        }
        apply(action)
    }

    /** Ask once. `false` when the platform refused — the caller plays anyway. */
    fun request(): Boolean {
        if (request != null) return true
        val attributes =
            AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                .build()
        val built =
            AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                .setAudioAttributes(attributes)
                // We do the ducking in the engine (`set_ducked`), where the ramp
                // is already measured: the platform's own duck would fight it.
                .setWillPauseWhenDucked(false)
                .setOnAudioFocusChangeListener(listener, handler)
                .build()
        val answer = audio?.requestAudioFocus(built)
        if (answer == AudioManager.AUDIOFOCUS_REQUEST_GRANTED) {
            request = built
            return true
        }
        Log.i(TAG, "audio focus refused ($answer): playing without it")
        return false
    }

    /** Give it back, and forget the ducking we applied on the way. */
    fun abandon() {
        val current = request ?: return
        request = null
        if (ducked) {
            ducked = false
            apply(FocusAction(play = null, ducked = false, resumeOnGain = false))
        }
        runCatching { audio?.abandonAudioFocusRequest(current) }
    }

    /** The duck we applied must not outlive a new request. */
    fun reset() {
        ducked = false
        resumeOnGain = false
        pausedForFocus = false
    }

    private companion object {
        const val TAG = "NzFocus"
    }
}
