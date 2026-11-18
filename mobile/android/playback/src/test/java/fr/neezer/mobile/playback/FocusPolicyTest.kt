// :playback — the audio-focus decision table (docs/agents/mobile.md, "MB3c").
// JVM: each row is a moment the person hears or notices.
package fr.neezer.mobile.playback

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class FocusPolicyTest {

    @Test
    fun a_transient_loss_pauses_and_a_later_gain_resumes() {
        val lost = FocusPolicy.on(FocusEvent.LOSS_TRANSIENT, wasPlaying = true, ducked = false, resumeOnGain = false)
        assertEquals(false, lost.play)
        assertTrue("a call gives the music back", lost.resumeOnGain)
        assertNull(lost.ducked)

        val back = FocusPolicy.on(FocusEvent.GAIN, wasPlaying = false, ducked = false, resumeOnGain = true)
        assertEquals(true, back.play)
        assertFalse("resumed once", back.resumeOnGain)
    }

    /** A person who paused before the call does not want the music back after it. */
    @Test
    fun a_transient_loss_does_not_resume_what_was_paused() {
        val lost = FocusPolicy.on(FocusEvent.LOSS_TRANSIENT, wasPlaying = false, ducked = false, resumeOnGain = false)
        assertNull("the transport is not touched", lost.play)
        assertFalse(lost.resumeOnGain)

        val back = FocusPolicy.on(FocusEvent.GAIN, wasPlaying = false, ducked = false, resumeOnGain = false)
        assertNull(back.play)
    }

    /** A permanent loss is the other app's turn: we stop and stay stopped. */
    @Test
    fun a_permanent_loss_is_never_resumed_by_a_later_gain() {
        val lost = FocusPolicy.on(FocusEvent.LOSS, wasPlaying = true, ducked = false, resumeOnGain = false)
        assertEquals(false, lost.play)
        assertFalse(lost.resumeOnGain)

        val back = FocusPolicy.on(FocusEvent.GAIN, wasPlaying = false, ducked = false, resumeOnGain = false)
        assertNull("nothing to resume", back.play)
    }

    /** A prompt that tolerates us ducks the ENGINE and keeps playing. */
    @Test
    fun a_duck_lowers_the_sound_without_touching_the_transport() {
        val ducked = FocusPolicy.on(FocusEvent.LOSS_TRANSIENT_CAN_DUCK, wasPlaying = true, ducked = false, resumeOnGain = false)
        assertNull(ducked.play)
        assertEquals(true, ducked.ducked)

        val back = FocusPolicy.on(FocusEvent.GAIN, wasPlaying = true, ducked = true, resumeOnGain = false)
        assertEquals("a duck is undone, and the transport stays playing", false, back.ducked)
        assertNull("the duck was applied by us, not by the focus", back.play)
    }

    /** The headset going away pauses AND gives the focus back. */
    @Test
    fun becoming_noisy_pauses_and_abandons_the_focus() {
        val noisy = FocusPolicy.onBecomingNoisy(wasPlaying = true)
        assertEquals(false, noisy.play)
        assertTrue(noisy.abandon)
    }
}
