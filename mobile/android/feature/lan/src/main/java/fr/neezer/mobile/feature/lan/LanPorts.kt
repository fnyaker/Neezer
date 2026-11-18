package fr.neezer.mobile.feature.lan

import fr.neezer.mobile.bridge.NzBridge
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import uniffi.nz_ffi.LanApi
import uniffi.nz_ffi.LanDeviceFfi
import uniffi.nz_ffi.LanFriendsFfi
import uniffi.nz_ffi.LanListener
import uniffi.nz_ffi.LanSettingsFfi
import uniffi.nz_ffi.LanSnapshotFfi
import uniffi.nz_ffi.PeerSetFfi

/**
 * The one door to the LAN node (lot LN14).
 *
 * `nz-core`'s `LanApi` is the process's ONE node (LN13): every object built by
 * `NzCore::lan()` talks to the same slot, so this module never holds a second
 * node and a listener registered anywhere hears every change.
 *
 * An interface, not `LanApi` itself, for the reason every screen lot gives: the
 * whole screen state machine (optimistic switches, rollback, the causes, the
 * 「 aucun appareil 」 hint) is then held by a JVM test against a fake — no JNA,
 * no `.so`, no device. The device run is what proves the NSD and the lock.
 */
interface LanPort {
    /** The whole node state, cloned from memory (synchronous, one frame). */
    fun snapshot(): LanSnapshotFfi

    /** The settings the person edits; stored ones answer before any node exists. */
    fun settings(): LanSettingsFfi

    fun setSettings(settings: LanSettingsFfi)

    /** Called on CHANGE only (`changed(version)`), on a Rust thread. */
    fun subscribe(listener: LanListener)

    /** Starts the node for this session (suspends on the server probe). */
    suspend fun start()

    fun stop()

    // -- the platform's discovery feed (NsdManager) --------------------------
    fun found(host: String, port: Int)

    fun lost(host: String, port: Int)

    fun networkChanged()

    /** Rule 1's battery flags: foreground, playing, downloads running. */
    fun setActive(foreground: Boolean, playing: Boolean, downloading: Boolean)

    /** The port really bound, which is what Kotlin registers with NSD. */
    fun advertisedPort(): Int

    // -- « Sur vos appareils » ----------------------------------------------
    fun peerSets(): List<PeerSetFfi>

    fun downloadSets(keys: List<String>)

    // -- the account's trust chain (server, `User` lane) ---------------------
    suspend fun devices(): List<LanDeviceFfi>

    suspend fun revoke(id: String)

    suspend fun rename(id: String, name: String)

    suspend fun friends(): LanFriendsFfi

    suspend fun addFriend(name: String)

    suspend fun setFriendScope(userId: String, all: Boolean, playlists: List<String>)

    suspend fun removeFriend(userId: String)

    /** The counted bytes of the node's state (worker rule 4: never an RSS). */
    fun memoryBytes(): Long
}

/** `LanPort` over the Rust node. Every call answers `null` until a core exists. */
internal class FfiLanPort : LanPort {
    private fun api(): LanApi = NzBridge.core?.lan() ?: throw LanUnavailable()

    override fun snapshot(): LanSnapshotFfi = api().snapshot()

    override fun settings(): LanSettingsFfi = api().settings()

    override fun setSettings(settings: LanSettingsFfi) = api().setSettings(settings)

    override fun subscribe(listener: LanListener) = api().subscribe(listener)

    override suspend fun start() = api().start()

    override fun stop() = api().stop()

    override fun found(host: String, port: Int) {
        // The FFI takes a `u16`: a port from the wire is checked HERE, before it
        // becomes one (worker rule 10: nothing from the network is trusted).
        if (!validDiscoveredPort(port)) return
        api().found(host, port.toUShort())
    }

    override fun lost(host: String, port: Int) {
        if (!validDiscoveredPort(port)) return
        api().lost(host, port.toUShort())
    }

    override fun networkChanged() = api().networkChanged()

    override fun setActive(foreground: Boolean, playing: Boolean, downloading: Boolean) =
        api().setActive(foreground, playing, downloading)

    override fun advertisedPort(): Int = api().advertisedPort().toInt()

    override fun peerSets(): List<PeerSetFfi> = api().peerSets()

    override fun downloadSets(keys: List<String>) = api().downloadSets(keys)

    override suspend fun devices(): List<LanDeviceFfi> = api().devices()

    override suspend fun revoke(id: String) = api().revoke(id)

    override suspend fun rename(id: String, name: String) = api().rename(id, name)

    override suspend fun friends(): LanFriendsFfi = api().friends()

    override suspend fun addFriend(name: String) = api().addFriend(name)

    override suspend fun setFriendScope(userId: String, all: Boolean, playlists: List<String>) =
        api().setFriendScope(userId, all, playlists)

    override suspend fun removeFriend(userId: String) = api().removeFriend(userId)

    override fun memoryBytes(): Long = api().memoryBytes().toLong()
}

/** The native core was never built: the sentence the screen shows (rule 3). */
class LanUnavailable : IllegalStateException(
    "L'échange entre appareils n'est pas disponible : le noyau n'a pas démarré",
)

/**
 * The node's change counter, as the screens read it.
 *
 * Rust pushes on CHANGE only, never on a timer (`docs/agents/mobile.md`, "The FFI
 * boundary"): this object hands the version to a `StateFlow` and returns at once,
 * because the callback runs on a Rust thread and suspending there would block it.
 * A screen collects the version and re-reads `snapshot()`, which is a memory
 * clone (measured 2.3 µs, LN13).
 */
object LanFeed {
    private val _version = MutableStateFlow(0UL)
    val version: StateFlow<ULong> = _version.asStateFlow()

    internal val listener: LanListener = object : LanListener {
        override fun changed(version: ULong) {
            _version.value = version
        }
    }
}
