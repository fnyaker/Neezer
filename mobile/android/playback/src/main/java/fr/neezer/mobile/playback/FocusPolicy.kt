// :playback — audio focus (docs/agents/mobile.md, "MB3c": "one request,
// AUDIOFOCUS_GAIN; loss → pause; transient → pause, resume on gain if it was
// playing; can-duck → set_ducked(true)"; android.md's "one audio focus request
// for every player").
package fr.neezer.mobile.playback

/** What Android told us about the focus we hold. */
enum class FocusEvent {
    /** The focus is ours again. */
    GAIN,

    /** Somebody else keeps it: we stop and stay stopped. */
    LOSS,

    /** A call or a navigation prompt: we pause and hope to come back. */
    LOSS_TRANSIENT,

    /** A prompt that tolerates us at a lower volume. */
    LOSS_TRANSIENT_CAN_DUCK,
}

/**
 * What the player must do about [FocusEvent] — a value, so the rule is a JVM
 * test rather than a device run.
 */
data class FocusAction(
    /** `true` play, `false` pause, `null` do not touch the transport. */
    val play: Boolean?,
    /** `true`/`false` to change the ducking, `null` to leave it. */
    val ducked: Boolean?,
    /** Whether a later `GAIN` resumes the transport. */
    val resumeOnGain: Boolean,
    /** Whether the focus request can be given up (`ACTION_AUDIO_BECOMING_NOISY`). */
    val abandon: Boolean = false,
)

/**
 * The audio-focus rules, in one place and with no Android type in them.
 *
 * The three facts that make this a decision table instead of a callback body
 * (each is a way a media player annoys somebody):
 *
 * - a PERMANENT loss must NOT be resumed by a later gain (the other app is
 *   playing; coming back would fight it);
 * - a TRANSIENT loss resumes only if we were really playing — a person who
 *   paused before the call does not want the music back after it;
 * - a duck never touches the transport: the sound goes down through the engine
 *   (`set_ducked`), because the device volume is the person's.
 */
object FocusPolicy {
    fun on(event: FocusEvent, wasPlaying: Boolean, ducked: Boolean, resumeOnGain: Boolean): FocusAction =
        when (event) {
            FocusEvent.GAIN ->
                // Only a TRANSIENT loss paused us, so only it resumes: coming
                // back from a duck would restart a title the person paused while
                // the prompt was on screen (the test that caught this).
                FocusAction(
                    play = if (resumeOnGain) true else null,
                    ducked = if (ducked) false else null,
                    resumeOnGain = false,
                )

            FocusEvent.LOSS ->
                FocusAction(play = false, ducked = null, resumeOnGain = false)

            FocusEvent.LOSS_TRANSIENT ->
                FocusAction(play = if (wasPlaying) false else null, ducked = null, resumeOnGain = wasPlaying)

            FocusEvent.LOSS_TRANSIENT_CAN_DUCK ->
                FocusAction(play = null, ducked = true, resumeOnGain = resumeOnGain)

            // Handled by `abandon` below; kept exhaustive for the compiler.
        }

    /**
     * Becoming noisy (a headset unplugged, a Bluetooth sink gone): pause AND give
     * the focus back, because the person is probably about to play something
     * else on the speaker.
     */
    fun onBecomingNoisy(wasPlaying: Boolean): FocusAction =
        FocusAction(play = false, ducked = null, resumeOnGain = false, abandon = true)
}
