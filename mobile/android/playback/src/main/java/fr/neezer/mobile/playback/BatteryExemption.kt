// :playback — the battery-exemption offer (docs/agents/mobile.md, "MB3c": "the
// battery-exemption offer once (only when a playback was killed by the system —
// a kv flag set at play, cleared at clean pause)"; android.md's
// `offerBatteryExemptionOnce`).
package fr.neezer.mobile.playback

/**
 * The few booleans the offer needs, behind an interface so the whole rule runs
 * on the JVM. [SharedPreferencesFlags] is the device's answer.
 */
interface FlagStore {
    fun getBoolean(key: String): Boolean

    /** Writes synchronously: the process may be killed right after the call. */
    fun putBoolean(key: String, value: Boolean)
}

/**
 * "Android killed us while we were playing" — the ONLY moment the app asks for
 * an exemption from battery optimisations.
 *
 * Why a flag and not a timer or a crash count: the system kills a background
 * process with no callback, so the evidence left behind is what the app wrote
 * before it started playing. A clean pause clears it, because a person who
 * stopped the music and then closed the app was not killed by anything.
 *
 * Why once ever: the exemption is a system dialog. Asking again after "Non" is
 * the nagging the old app's screen carried for a while; `ASKED` is what makes
 * "once" true, and it survives a reinstall of the same data directory.
 */
class BatteryExemption(private val store: FlagStore) {

    /**
     * The app started playing: if the process dies before the next clean pause,
     * that is a kill. Written with a commit (not an apply) exactly because the
     * death can follow within milliseconds.
     */
    fun notePlaying() {
        if (!store.getBoolean(PLAYING)) store.putBoolean(PLAYING, true)
    }

    /** A pause the person asked for (or the end of the queue): nothing was killed. */
    fun noteCleanPause() {
        if (store.getBoolean(PLAYING)) store.putBoolean(PLAYING, false)
    }

    /**
     * At service start: did the last run die while playing?
     *
     * It ANSWERS the question and clears the evidence, so a second service start
     * in the same process does not "discover" the same kill twice.
     */
    fun processStarted(): Boolean {
        if (!store.getBoolean(PLAYING)) return false
        store.putBoolean(PLAYING, false)
        store.putBoolean(KILLED, true)
        return true
    }

    /** The dialog belongs on screen: a kill happened and the person was never asked. */
    fun shouldOffer(): Boolean = store.getBoolean(KILLED) && !store.getBoolean(ASKED)

    /** The dialog was shown: never again, whatever the person answered. */
    fun markOffered() {
        store.putBoolean(ASKED, true)
        store.putBoolean(KILLED, false)
    }

    companion object {
        /** Written at play, cleared at a clean pause. */
        const val PLAYING = "battery.playing"

        /** The last run died while playing. */
        const val KILLED = "battery.killed"

        /** The dialog was already shown once, ever. */
        const val ASKED = "battery.asked"
    }
}
