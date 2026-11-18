package fr.neezer.mobile.feature.offline

import fr.neezer.mobile.bridge.NzBridge
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import uniffi.nz_ffi.DownloadEntry
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.OfflineApi
import uniffi.nz_ffi.OfflineListener
import uniffi.nz_ffi.OfflineSettings
import uniffi.nz_ffi.OfflineState
import uniffi.nz_ffi.SetProgress
import uniffi.nz_ffi.StorageReport
import uniffi.nz_ffi.offlineAttach
import java.util.concurrent.atomic.AtomicBoolean

/**
 * The one door to the downloads' Rust service (lot MB15).
 *
 * `NzCore::offline()` answers an object MB0 declared WITHOUT state, so the
 * service is bound once, from an async call: `offlineAttach(core)` starts the
 * process's worker and lets a queue left by the killed process carry on
 * (rule 2). Everything in this module goes through [api], so no call can reach
 * the FFI before that bind — a button that read `progress()` first would show
 * « les téléchargements ne sont pas reliés au noyau » instead of its state.
 *
 * It is a singleton like `NzBridge`: the shell, an album's `DownloadButton` and
 * Réglages → Téléchargements all speak to the same queue.
 */
internal object OfflineCore {
    private val mutex = Mutex()
    private val attached = AtomicBoolean(false)

    /**
     * The bound API, or `null` when the native core could not be built at all.
     *
     * **A bind that fails is NOT swallowed.** It used to be
     * (`runCatching { … }.isSuccess`), and the effect was the worst kind: the
     * attach answered `there is no reactor running` on every device — the export
     * lacked `async_runtime = "tokio"` (fixed in `nz-ffi/src/offline.rs`) — so no
     * download could ever be created, every screen said "les téléchargements ne
     * sont pas reliés au noyau", and nothing in the app surfaced the cause
     * (rules 2 and 3). The exception now reaches the caller, which shows it
     * (`DownloadsModel`'s `causeText` → a notice with « Réessayer »), and the next
     * call tries the bind again because [attached] is only set on success.
     */
    suspend fun api(): OfflineApi? = mutex.withLock {
        val core = NzBridge.core ?: return null
        if (bindOnce(attached.get()) { offlineAttach(core) }) {
            attached.set(true)
        }
        core.offline()
    }

    /** Re-bind after a core rebuild (a test's own bridge, a process restart). */
    fun forget() {
        attached.set(false)
    }
}

/**
 * Binds the downloads' service ONCE, and lets a failure OUT.
 *
 * Extracted from [OfflineCore] so a JVM test can hold the two rules without a
 * `.so`: an `attach` that throws propagates (the screen shows the cause), and the
 * binding is not marked done, so the next call tries again — a silent `false`
 * here is exactly the "button that seems to do nothing" of rule 2.
 */
internal suspend fun bindOnce(attached: Boolean, attach: suspend () -> Unit): Boolean {
    if (attached) return true
    attach()
    return true
}

/**
 * What the downloads screen and the download button ask of the core.
 *
 * An interface, not `OfflineApi` itself: the ViewModel's whole state machine —
 * optimistic rows, rollback, the causes, the "n of m" — is then held by a JVM
 * test against a fake, which is what the lot's acceptance counts (0 requests on
 * a warm open, one toast with the cause of a refusal).
 */
interface OfflinePort {
    suspend fun state(): OfflineState

    suspend fun addSet(context: String, quality: String, title: String, artist: String): SetProgress

    suspend fun removeSet(kind: String, id: String)

    suspend fun removeDownload(key: String)

    suspend fun retry(key: String)

    suspend fun retryAll()

    suspend fun pause()

    suspend fun resume()

    suspend fun setQuality(quality: String)

    suspend fun setOfflineOnly(onlyDownloaded: Boolean)

    suspend fun setPlayCacheCap(bytes: Long)

    suspend fun clearPlayCache()

    /** The per-title list as a WINDOWED handle (rule 4: 4 000 rows are paged). */
    suspend fun downloads(): ListHandle

    /** Quality and bytes of the rows on screen — one store read per key. */
    suspend fun facts(keys: List<String>): List<DownloadEntry>

    suspend fun playCacheBytes(): Long

    suspend fun subscribe(listener: OfflineListener)
}

/** `OfflinePort` over the Rust service. Every call binds the service first. */
internal class FfiOfflinePort : OfflinePort {
    private suspend fun api(): OfflineApi =
        OfflineCore.api() ?: throw OfflineUnavailable()

    override suspend fun state(): OfflineState = api().state()

    override suspend fun addSet(
        context: String,
        quality: String,
        title: String,
        artist: String,
    ): SetProgress {
        val (kind, id) = splitContext(context)
        return api().addSet(kind, id, quality, title, artist)
    }

    override suspend fun removeSet(kind: String, id: String) {
        api().removeSet(kind, id)
    }

    override suspend fun removeDownload(key: String) {
        api().removeDownload(key)
    }

    override suspend fun retry(key: String) {
        api().retry(key)
    }

    override suspend fun retryAll() {
        api().retryAll()
    }

    override suspend fun pause() {
        api().pause()
    }

    override suspend fun resume() {
        api().resume()
    }

    override suspend fun setQuality(quality: String) {
        api().setQuality(quality)
    }

    override suspend fun setOfflineOnly(onlyDownloaded: Boolean) {
        api().setOfflineOnly(onlyDownloaded)
    }

    override suspend fun setPlayCacheCap(bytes: Long) {
        api().setPlayCacheCap(bytes)
    }

    override suspend fun clearPlayCache() {
        api().clearPlayCache()
    }

    override suspend fun downloads(): ListHandle = api().downloads()

    override suspend fun facts(keys: List<String>): List<DownloadEntry> = api().downloadFacts(keys)

    override suspend fun playCacheBytes(): Long = api().playCacheBytes().toLong()

    override suspend fun subscribe(listener: OfflineListener) {
        // Cheap on the Rust side (it pushes the current state at once), but it
        // needs the bound service, which is why it is a suspend call: the bind
        // happens here, never on a first `progress()` alone.
        api().subscribe(listener)
    }
}

/**
 * The split of a context the UI carries around (`playlist:42`) into the two
 * halves the FFI speaks. A context without a colon is a caller's bug, and the
 * message says which one instead of sending `kind = ""` to the server.
 */
fun splitContext(context: String): Pair<String, String> {
    val at = context.indexOf(':')
    require(at > 0 && at < context.length - 1) { "contexte de téléchargement invalide : « $context »" }
    return context.substring(0, at) to context.substring(at + 1)
}

/** The core was never built: the sentence Aide shows, and a button's cause. */
class OfflineUnavailable : IllegalStateException(
    "Les téléchargements ne sont pas disponibles : le noyau n'a pas démarré",
)
