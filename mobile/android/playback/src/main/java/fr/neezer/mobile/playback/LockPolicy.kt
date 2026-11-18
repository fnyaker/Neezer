// :playback — the wake and Wi-Fi locks (docs/agents/mobile.md, "MB3c": a
// `PARTIAL_WAKE_LOCK` while Playing/Buffering, a `WifiLock`
// (`WIFI_MODE_FULL_HIGH_PERF`) only while `snapshot.streaming`; "Rule 1 on the
// phone": "the Wi-Fi lock is held only while a network source is being read").
package fr.neezer.mobile.playback

import uniffi.nz_ffi.PlayState

/**
 * Which locks a given state needs — pure, so the rule is asserted on the JVM
 * and the service only applies it.
 *
 * Why not "the wake lock while the service is up": a paused player that holds a
 * PARTIAL_WAKE_LOCK keeps the phone awake for ever (the old app's dead
 * notification in a pocket). Why not "the Wi-Fi lock while playing": a track
 * read from the disk or the play cache needs no radio, and holding a high
 * performance Wi-Fi lock then is exactly the battery cost rule 1 forbids.
 */
object LockPolicy {
    /** The CPU must not sleep while the engine is rendering or refilling. */
    fun wakeLock(state: PlayState): Boolean =
        state == PlayState.PLAYING || state == PlayState.BUFFERING

    /** A network source is being read: the radio must not go into a deep sleep. */
    fun wifiLock(streaming: Boolean): Boolean = streaming
}
