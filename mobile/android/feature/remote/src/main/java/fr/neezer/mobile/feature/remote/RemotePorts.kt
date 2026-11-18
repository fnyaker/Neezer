package fr.neezer.mobile.feature.remote

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.designsystem.NzRowSource
import fr.neezer.mobile.designsystem.asRowSource
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.RemoteApi
import uniffi.nz_ffi.RemoteCommand
import uniffi.nz_ffi.RemoteController
import uniffi.nz_ffi.RemoteDuration
import uniffi.nz_ffi.RemoteGrant
import uniffi.nz_ffi.RemoteLevel
import uniffi.nz_ffi.RemoteLink
import uniffi.nz_ffi.RemoteListener
import uniffi.nz_ffi.RemoteLocalSettings
import uniffi.nz_ffi.RemoteNow
import uniffi.nz_ffi.RemotePhase
import uniffi.nz_ffi.RemoteStatus
import uniffi.nz_ffi.TrackRow
import uniffi.nz_ffi.remoteAttach

/**
 * The one door to the remote-control core (lot MB19).
 *
 * `NzCore::remote()` answers an object MB0 declared WITHOUT state, so the
 * process's service is bound once, from an async call: `remoteAttach(core)` also
 * hands the host side the phone's ONE player (`MB3c`'s rule) and resumes what the
 * session already is — a loan starts driving, and a device that was lent before
 * asks the server once whether a link still points at it. Everything in this
 * module goes through [api], so no call can reach the FFI before that bind: a
 * screen that read `status()` first would show « la télécommande n'est pas reliée
 * au noyau » instead of its state.
 *
 * It is a singleton like `NzBridge`: the sheet, the chip and the claim screen
 * speak to the same service, and a second `attach` could not start a second
 * thread over one player.
 */
internal object RemoteCore {
    private val mutex = Mutex()
    private val attached = java.util.concurrent.atomic.AtomicBoolean(false)

    suspend fun api(): RemoteApi? = mutex.withLock {
        val core = NzBridge.core ?: return null
        if (!attached.get()) {
            try {
                remoteAttach(core)
                attached.set(true)
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // The bind is retried by the next call; the screen shows the
                // cause the FFI answers meanwhile.
                return null
            }
        }
        core.remote()
    }

    /** Re-bind after a core rebuild (a test's own bridge, a process restart). */
    fun forget() {
        attached.set(false)
    }
}

/**
 * What the remote screens ask of the core, as an interface.
 *
 * Not the generated `RemoteApi` directly, so every rule of the screen is plain
 * Kotlin over a fake in `RemoteModelTest` — no JNA, no `.so`, no device.
 * [NzRemoteService] is the one implementation, a one-to-one delegation to
 * `nz-ffi/src/remote.rs`.
 */
interface RemoteService {
    fun status(): RemoteStatus

    fun now(): RemoteNow

    /** The hot read: the owner's published line, per visible frame. */
    fun position(): Double

    fun queue(): ListHandle

    fun cachedLinks(): List<RemoteLink>

    suspend fun links(): List<RemoteLink>

    suspend fun createLink(level: String, duration: String, label: String?): RemoteLink

    suspend fun revokeLink(id: String)

    suspend fun revokeAll()

    suspend fun claim(token: String): RemoteGrant

    suspend fun startLoan()

    suspend fun leave(): String?

    suspend fun send(command: RemoteCommand)

    suspend fun startHost()

    suspend fun resumeHost(): Boolean

    fun stopHost(): Boolean

    fun hostedBefore(): Boolean

    fun levels(): List<RemoteLevel>

    fun durations(): List<RemoteDuration>

    fun localSettings(): RemoteLocalSettings

    fun subscribe(listener: RemoteListener)
}

/**
 * The real service: `nz-ffi/src/remote.rs`, one call each.
 *
 * Every call answers [Unavailable] when the core could not be bound; the model
 * turns that into a French cause with « Réessayer » (rule 3), never a blank
 * button.
 */
object NzRemoteService : RemoteService {
    private class Unavailable : IllegalStateException("la télécommande n'est pas reliée au noyau")

    private suspend fun api(): RemoteApi = RemoteCore.api() ?: throw Unavailable()

    override fun status(): RemoteStatus = ((NzBridge.core?.remote()) ?: throw Unavailable()).status()

    override fun now(): RemoteNow = ((NzBridge.core?.remote()) ?: throw Unavailable()).now()

    override fun position(): Double =
        ((NzBridge.core?.remote()) ?: throw Unavailable()).position()

    override fun queue(): ListHandle = ((NzBridge.core?.remote()) ?: throw Unavailable()).queue()

    override fun cachedLinks(): List<RemoteLink> =
        ((NzBridge.core?.remote()) ?: throw Unavailable()).cachedLinks()

    override suspend fun links(): List<RemoteLink> = api().links()

    override suspend fun createLink(
        level: String,
        duration: String,
        label: String?,
    ): RemoteLink = api().createLink(level, duration, label)

    override suspend fun revokeLink(id: String) {
        api().revokeLink(id)
    }

    override suspend fun revokeAll() {
        api().revokeAll()
    }

    override suspend fun claim(token: String): RemoteGrant = api().claim(token)

    override suspend fun startLoan() {
        api().startLoan()
    }

    override suspend fun leave(): String? = api().leave()

    override suspend fun send(command: RemoteCommand) {
        api().send(command)
    }

    override suspend fun startHost() {
        api().startHost()
    }

    override suspend fun resumeHost(): Boolean = api().resumeHost()

    override fun stopHost(): Boolean {
        (NzBridge.core?.remote() ?: throw Unavailable()).stopHost()
        return true
    }

    override fun hostedBefore(): Boolean =
        ((NzBridge.core?.remote()) ?: throw Unavailable()).hostedBefore()

    override fun levels(): List<RemoteLevel> =
        ((NzBridge.core?.remote()) ?: throw Unavailable()).levels()

    override fun durations(): List<RemoteDuration> =
        ((NzBridge.core?.remote()) ?: throw Unavailable()).durations()

    override fun localSettings(): RemoteLocalSettings =
        ((NzBridge.core?.remote()) ?: throw Unavailable()).localSettings()

    override fun subscribe(listener: RemoteListener) {
        (NzBridge.core?.remote() ?: throw Unavailable()).subscribe(listener)
    }
}

// --- the rows the screen draws ---------------------------------------------

/**
 * One link as the sheet lists it (the FFI record, minus nothing): the URL is kept
 * because sharing it is the sheet's whole point.
 */
data class RemoteLinkRow(
    val id: String,
    val url: String,
    val level: String,
    val levelLabel: String,
    val label: String,
    val title: String,
    val expiry: String,
    val controllers: Int,
) {
    /** `connecté`, `2 connectés` — the live line the SPA prints. */
    val live: Boolean get() = controllers > 0
}

fun RemoteLink.toRow() = RemoteLinkRow(
    id = id,
    url = url,
    level = level,
    levelLabel = levelLabel,
    label = label,
    title = title,
    expiry = expiry,
    controllers = controllers.toInt(),
)

/** One controller driving this device, as the chip names it. */
data class RemoteControllerRow(
    val link: String,
    val level: String,
    val levelLabel: String,
    val label: String,
)

fun RemoteController.toRow() = RemoteControllerRow(
    link = link,
    level = level,
    levelLabel = levelLabel,
    label = label,
)

/** The grant a loan runs under, as the banner and the cards read it. */
data class RemoteGrantFi(
    val level: String,
    val levelLabel: String,
    val owner: String,
    val device: String,
    val label: String,
    val canBrowse: Boolean,
    val canKeep: Boolean,
    val canAdmin: Boolean,
)

fun RemoteGrant.toFi() = RemoteGrantFi(
    level = level,
    levelLabel = levelLabel,
    owner = owner,
    device = device,
    label = label,
    canBrowse = canBrowse,
    canKeep = canKeep,
    canAdmin = canAdmin,
)

/**
 * The chip and the banner, composed. [role] is what decides which one is drawn:
 * a phone can be lent to somebody, or driving somebody, and the two are one
 * record because they come from one service.
 */
data class RemoteStatusFi(
    val controlling: Boolean,
    val driven: Boolean,
    val hostName: String,
    val who: String,
    val controllers: List<RemoteControllerRow>,
    val hostLevel: String?,
    val hostError: String?,
    val applied: Int,
    val phase: RemotePhase,
    val online: Boolean,
    val grant: RemoteGrantFi?,
    val line: String,
    val error: String?,
    val restored: String?,
) {
    /** `« Contrôlé par Léa »`, `« Contrôlé par 2 appareils »`. */
    val live: Boolean get() = driven
}

fun RemoteStatus.toFi() = RemoteStatusFi(
    controlling = role == "controller",
    driven = role == "controlled",
    hostName = if (role == "controller") grant?.owner.orEmpty() else who,
    who = who,
    controllers = controllers.map { it.toRow() },
    hostLevel = hostLevel,
    hostError = hostError,
    applied = applied.toInt(),
    phase = phase,
    online = online,
    grant = grant?.toFi(),
    line = line,
    error = error,
    restored = restored,
)

/** The owner's player, as the "Contrôle distant" screen draws it. */
data class RemoteNowFi(
    val track: TrackRow?,
    val playing: Boolean,
    val index: Int,
    val len: Int,
    val positionS: Double,
    val durationS: Double,
    val volume: Double,
    val muted: Boolean,
    val shuffle: Boolean,
    val repeat: String,
    val status: String,
    val online: Boolean,
    val queueVersion: Long,
)

fun RemoteNow.toFi() = RemoteNowFi(
    track = track,
    playing = playing,
    index = index.toInt(),
    len = len.toInt(),
    positionS = positionS,
    durationS = durationS,
    volume = volume,
    muted = muted,
    shuffle = shuffle,
    repeat = repeat,
    status = status,
    online = online,
    queueVersion = queueVersion.toLong(),
)

/**
 * The owner's queue, as the screen windows it: the design system's own source
 * behind `ListHandle`, exactly as the other list screens do it (never a local
 * adapter — MB6's `StackOverflowError`).
 *
 * The source is what the screen needs (`ListWindow` reads it), and the handle is
 * kept because the FFI object must outlive the window it feeds. A JVM test hands
 * a plain source: a screenshot of a list of skeletons would not be a screenshot of
 * this screen.
 */
class RemoteRows(val source: NzRowSource, val handle: ListHandle? = null) {
    companion object {
        fun of(handle: ListHandle) = RemoteRows(handle.asRowSource(), handle)
    }
}
