// :playback — the battery-exemption flag machine (docs/agents/mobile.md, "MB3c":
// "the battery-exemption offer once (only when a playback was killed by the
// system — a kv flag set at play, cleared at clean pause)"). JVM: the flag is the
// only evidence a system kill leaves.
package fr.neezer.mobile.playback

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class BatteryExemptionTest {

    /** Counts its writes: the flag must be written on TRANSITIONS, not per state. */
    private class FakeFlags : FlagStore {
        val values = mutableMapOf<String, Boolean>()
        var writes = 0

        override fun getBoolean(key: String): Boolean = values[key] ?: false

        override fun putBoolean(key: String, value: Boolean) {
            writes++
            values[key] = value
        }
    }

    @Test
    fun a_clean_run_never_offers_anything() {
        val flags = FakeFlags()
        val exemption = BatteryExemption(flags)
        exemption.notePlaying()
        exemption.noteCleanPause()
        // A service restart in the same process finds nothing to report.
        assertFalse(exemption.processStarted())
        assertFalse(exemption.shouldOffer())
    }

    @Test
    fun a_playback_the_system_killed_is_detected_once() {
        val flags = FakeFlags()
        val exemption = BatteryExemption(flags)
        exemption.notePlaying()

        // The process died here: the next start reads the flag it wrote.
        assertTrue(exemption.processStarted())
        assertTrue(exemption.shouldOffer())
        // The evidence is consumed: a second start in the same process does not
        // "discover" the same kill again.
        assertFalse(exemption.processStarted())
        assertTrue("the offer is still pending", exemption.shouldOffer())
    }

    @Test
    fun the_dialog_is_offered_once_ever() {
        val flags = FakeFlags()
        val exemption = BatteryExemption(flags)
        exemption.notePlaying()
        exemption.processStarted()
        assertTrue(exemption.shouldOffer())

        exemption.markOffered()
        assertFalse(exemption.shouldOffer())
        // Even a LATER kill does not bring the dialog back: the person answered.
        exemption.notePlaying()
        exemption.processStarted()
        assertFalse(exemption.shouldOffer())
    }

    /** A play/pause cycle writes twice; a burst of playing states writes once. */
    @Test
    fun the_flag_is_written_on_transitions_only() {
        val flags = FakeFlags()
        val exemption = BatteryExemption(flags)
        repeat(50) { exemption.notePlaying() }
        assertEquals(1, flags.writes)
        repeat(50) { exemption.noteCleanPause() }
        assertEquals(2, flags.writes)
    }

    /** A pause the person asked for is not a kill, and clears the evidence. */
    @Test
    fun a_pause_before_the_process_died_is_not_a_kill() {
        val flags = FakeFlags()
        val exemption = BatteryExemption(flags)
        exemption.notePlaying()
        exemption.noteCleanPause()
        assertFalse(exemption.processStarted())
        assertFalse(exemption.shouldOffer())
    }
}
