package fr.neezer.mobile.feature.offline

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import uniffi.nz_ffi.DownloadEntry
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.OfflineListener
import uniffi.nz_ffi.OfflineSettings
import uniffi.nz_ffi.OfflineState
import uniffi.nz_ffi.SetProgress
import uniffi.nz_ffi.StorageReport

/**
 * The lot's port, held by the JVM tests: a real `DownloadsModel` over an
 * in-memory queue, with one hook per call to make it fail the way the core can.
 *
 * It is a fake of the CONTRACT, never of the rules: the optimistic row, the
 * rollback, the causes and the counts are the model's, so a green test here is a
 * statement about the code a phone runs.
 */
class FakeOfflinePort : OfflinePort {
    val stateFlow = MutableStateFlow(emptyState())
    var listener: OfflineListener? = null
        private set

    /** Calls the model made, in order — the "0 requests" assertions read it. */
    val calls = mutableListOf<String>()

    var failAdd: Throwable? = null
    var failRemove: Throwable? = null
    var failRetry: Throwable? = null
    var failQuality: Throwable? = null
    var failCap: Throwable? = null
    var failClear: Throwable? = null

    var added: SetProgress? = null
    var playCache = 0L

    /** Holds `addSet` open, so a test can observe the optimistic row. */
    var addGate: kotlinx.coroutines.CompletableDeferred<Unit>? = null

    override suspend fun state(): OfflineState {
        calls += "state"
        return stateFlow.value
    }

    override suspend fun addSet(
        context: String,
        quality: String,
        title: String,
        artist: String,
    ): SetProgress {
        calls += "add:$context:$quality"
        addGate?.await()
        failAdd?.let { throw it }
        val (kind, id) = splitContext(context)
        val set = added ?: SetProgress(
            context = context,
            title = title.ifEmpty { "Nuit" },
            kind = kind,
            id = id,
            done = 0u,
            total = 200u,
            failed = 0u,
            running = true,
            paused = false,
            failures = emptyList(),
        )
        return set
    }

    override suspend fun removeSet(kind: String, id: String) {
        calls += "removeSet:$kind:$id"
        failRemove?.let { throw it }
    }

    override suspend fun removeDownload(key: String) {
        calls += "removeDownload:$key"
        failRemove?.let { throw it }
    }

    override suspend fun retry(key: String) {
        calls += "retry:$key"
        failRetry?.let { throw it }
    }

    override suspend fun retryAll() {
        calls += "retryAll"
        failRetry?.let { throw it }
    }

    override suspend fun pause() {
        calls += "pause"
    }

    override suspend fun resume() {
        calls += "resume"
    }

    override suspend fun setQuality(quality: String) {
        calls += "setQuality:$quality"
        failQuality?.let { throw it }
    }

    override suspend fun setOfflineOnly(onlyDownloaded: Boolean) {
        calls += "setOfflineOnly:$onlyDownloaded"
    }

    override suspend fun setPlayCacheCap(bytes: Long) {
        calls += "setPlayCacheCap:$bytes"
        failCap?.let { throw it }
    }

    override suspend fun clearPlayCache() {
        calls += "clearPlayCache"
        failClear?.let { throw it }
        playCache = 0
    }

    override suspend fun downloads(): ListHandle = throw UnsupportedOperationException("a real handle needs the core")

    override suspend fun facts(keys: List<String>): List<DownloadEntry> {
        calls += "facts:${keys.size}"
        return keys.map { key -> DownloadEntry(key, "", "", null, 0u, false, "OPUS_320", 4096uL, 0L) }
    }

    override suspend fun playCacheBytes(): Long = playCache

    override suspend fun subscribe(listener: OfflineListener) {
        calls += "subscribe"
        this.listener = listener
        listener.onProgress(stateFlow.value)
    }

    /** What the Rust worker does: publish a new state. */
    fun publish(state: OfflineState) {
        stateFlow.value = state
        listener?.onProgress(state)
    }
}

/** A state with one published set and one failure, as the worker reports it. */
fun offlineState(
    version: ULong = 3uL,
    done: UInt = 194u,
    failed: UInt = 6u,
    running: Boolean = false,
    onlyDownloaded: Boolean = true,
): OfflineState = OfflineState(
    version = version,
    state = if (running) "Téléchargement en cours" else "À jour",
    paused = false,
    sets = listOf(
        SetProgress(
            context = "playlist:42",
            title = "Nuit",
            kind = "playlist",
            id = "42",
            done = done,
            total = 200u,
            failed = failed,
            running = running,
            paused = false,
            failures = if (failed > 0u) listOf("Titre 7 — le serveur prépare ce titre") else emptyList(),
        ),
    ),
    settings = OfflineSettings("OPUS_320", onlyDownloaded, 1024L * 1024 * 1024),
    storage = StorageReport(
        downloadsBytes = 64uL * 1024uL * 1024uL,
        cacheBytes = 12uL * 1024uL * 1024uL,
        coversBytes = 3uL * 1024uL * 1024uL,
        tracks = done,
        freeBytes = 0uL,
    ),
    note = null,
)

fun emptyState(): OfflineState = OfflineState(
    version = 0uL,
    state = "À jour",
    paused = false,
    sets = emptyList(),
    settings = OfflineSettings("OPUS_320", true, 1024L * 1024 * 1024),
    storage = StorageReport(0uL, 0uL, 0uL, 0u, 0uL),
    note = null,
)
