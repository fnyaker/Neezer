package fr.neezer.mobile.feature.party

import fr.neezer.mobile.bridge.NzBridge
import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.PartyApi
import uniffi.nz_ffi.PartyListener
import uniffi.nz_ffi.PartyPrefs
import uniffi.nz_ffi.PartyStateData

/**
 * What the two party screens ask of the core: this lot's `nz-ffi` methods, one to
 * one (lot MB20).
 *
 * An interface, not the generated `PartyApi`, so the screens' logic is plain
 * Kotlin that [PartyModelTest] and [PartyGuestModelTest] run on the JVM with a
 * fake — no JNA, no `.so`, no device. [NzPartyService] is the one real
 * implementation.
 */
interface PartyService {
    /** Whether this server has the party routes at all (rule 7). `null` = a doubt. */
    suspend fun supported(): Boolean?

    /** The landing card's remembered answer, disk only. */
    suspend fun cachedPeek(id: String): PartyStateData?

    /** `GET /api/party/<id>`: the card. `null` when the link never was one. */
    suspend fun peek(id: String): PartyStateData?

    /** `POST /api/party`: start (or re-open) the party this person hosts. */
    suspend fun host(): String

    /** Re-open the party this device hosted before, if any (one request). */
    suspend fun resume(): String?

    suspend fun join(id: String, name: String): PartyStateData

    /** Leave (a guest) or end it (a host). */
    suspend fun leave()

    /** The party right now, playhead included. */
    fun state(): PartyStateData

    /** The playhead, seconds: a HOT read, for the bar. */
    fun positionNow(): Double

    fun prefs(): PartyPrefs

    fun setName(name: String)

    fun setVolume(volume: Double)

    /** The device's own output delay, milliseconds (positive plays earlier). */
    fun setTrim(ms: Double)

    /** One call per CHANGE, never per frame. */
    fun subscribe(onChange: (PartyStateData) -> Unit)
}

/**
 * The one real [PartyService], over the generated object.
 *
 * It is built over the process's `PartyApi` (stateless: the service behind it is
 * the process's, which is what "one party per device" means), and [attach] binds
 * it to the core and the player the first time a party screen or the player's
 * door asks for it — MB15's `offlineAttach` seam, for the same reason: MB0
 * declared `NzCore::party()` before either object could carry a `Core`.
 */
class NzPartyService(private val api: PartyApi) : PartyService {
    override suspend fun supported(): Boolean? = api.supported()

    override suspend fun cachedPeek(id: String): PartyStateData? = api.cachedPeek(id)

    override suspend fun peek(id: String): PartyStateData? = api.peek(id)

    override suspend fun host(): String = api.host()

    override suspend fun resume(): String? = api.resume()

    override suspend fun join(id: String, name: String): PartyStateData = api.join(id, name)

    override suspend fun leave() {
        api.leave()
    }

    override fun state(): PartyStateData = api.state()

    override fun positionNow(): Double = api.positionS()

    override fun prefs(): PartyPrefs = api.prefs()

    override fun setName(name: String) = api.setName(name)

    override fun setVolume(volume: Double) = api.setVolume(volume)

    override fun setTrim(ms: Double) = api.setTrim(ms)

    override fun subscribe(onChange: (PartyStateData) -> Unit) {
        api.subscribe(
            object : PartyListener {
                override fun onChanged(state: PartyStateData) {
                    onChange(state)
                }
            },
        )
    }
}

/**
 * The process's party, bound once.
 *
 * `attach` needs the player: the guest's audio goes through the PLAYER's engine
 * (`nz_audio::PartyChannel`, SC2's output clock) and the host WATCHES that same
 * player — a second player is what the note forbids. So this happens lazily, on
 * the first party interaction, when `core.player()` is built for sure (the door
 * lives on the player's own surface).
 */
object PartyCore {
    @Volatile
    private var service: NzPartyService? = null

    /** The bound service, or `null` when the native core is missing or refused. */
    fun service(): NzPartyService? {
        service?.let { return it }
        return synchronized(this) {
            service ?: bind()?.also { service = it }
        }
    }

    private fun bind(): NzPartyService? {
        val core = NzBridge.core ?: return null
        val api = core.party()
        return try {
            api.attach(core, core.player())
            NzPartyService(api)
        } catch (_: NzException) {
            null
        }
    }

    /** Forgets the binding (a test, or a session that ended). */
    fun reset() {
        synchronized(this) { service = null }
    }
}
