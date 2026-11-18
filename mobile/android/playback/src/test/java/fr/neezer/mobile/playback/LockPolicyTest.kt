// :playback — which locks a state needs (docs/agents/mobile.md, "MB3c" and
// "Rule 1 on the phone"). JVM: a lock held one state too long is a battery bug.
package fr.neezer.mobile.playback

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.PlayState

class LockPolicyTest {

    @Test
    fun the_cpu_stays_awake_only_while_sound_is_coming() {
        assertTrue(LockPolicy.wakeLock(PlayState.PLAYING))
        assertTrue("a refill is still playing", LockPolicy.wakeLock(PlayState.BUFFERING))
        assertFalse(LockPolicy.wakeLock(PlayState.PAUSED))
        assertFalse(LockPolicy.wakeLock(PlayState.STOPPED))
        assertFalse(LockPolicy.wakeLock(PlayState.ENDED))
    }

    @Test
    fun the_radio_is_held_only_while_a_network_source_is_read() {
        assertTrue(LockPolicy.wifiLock(streaming = true))
        assertFalse("a downloaded track needs no radio", LockPolicy.wifiLock(streaming = false))
    }
}
