package fr.neezer.mobile.feature.offline

import android.os.StatFs
import androidx.compose.runtime.Stable
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.designsystem.NzToasts
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.nz_ffi.DownloadEntry
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.OfflineListener
import uniffi.nz_ffi.OfflineSettings
import uniffi.nz_ffi.OfflineState
import uniffi.nz_ffi.SetProgress
import uniffi.nz_ffi.StorageReport
import java.io.File

/**
 * The downloads' state machine (lot MB15), `lib/offline.js` + `lib/playcache.js`
 * + Réglages → Téléchargements PORTED.
 *
 * - **The core owns everything**; this class asks it and paints. The set rows,
 *   the causes and the counts come from the worker through [OfflineListener], so
 *   a screen never polls and never guesses (rule 2).
 * - **A write answers first, then talks**: queuing a set, removing one, retrying
 *   a failure all change the state in the same frame and roll back with a toast
 *   whose « Réessayer » runs the same call (rule 4, `webapp.md`).
 * - **A failure names its cause.** Every set the worker gave up on carries one
 *   French sentence per title (« titre — cause »), and a failed call's toast
 *   prints `nz_error_text`, never a code (rule 3).
 * - **A warm open costs nothing**: the subscription's first value paints the
 *   screen; `refresh()` reads the local store only.
 */
@Stable
class DownloadsModel(
    private val port: OfflinePort,
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate),
    private val io: CoroutineDispatcher = Dispatchers.IO,
    /** The disk's free space, in bytes: `StatFs`'s, never Rust's (see the FFI). */
    private val freeSpace: () -> Long = { 0L },
    private val toast: (String, String?, (() -> Unit)?) -> Unit = { text, label, run ->
        NzToasts.show(text, label, run)
    },
    private val causeText: (Throwable) -> String = { NzErrors.text(it) },
) {
    /** What one screen draws. `version` is the worker's own counter. */
    data class State(
        val loading: Boolean = true,
        /** The section's `LoadFailed` cause, when nothing could be read at all. */
        val failure: String? = null,
        /** What is shown is the device's copy (offline, or a refresh failed). */
        val stale: Boolean = false,
        val version: ULong = 0u,
        /** The worker's sentence: « Téléchargement en cours », « En pause »… */
        val worker: String = "",
        val paused: Boolean = false,
        val sets: List<SetProgress> = emptyList(),
        /** The contexts a click queued and the worker has not confirmed yet. */
        val pending: Set<String> = emptySet(),
        val settings: OfflineSettings? = null,
        val storage: StorageReport? = null,
        val note: String? = null,
        val playCacheBytes: Long = 0,
        val freeBytes: Long = 0,
        /** The per-title list, windowed through `rememberListWindow` (rule 4). */
        val handle: ListHandle? = null,
        /** Quality and bytes of the rows a page is showing. */
        val facts: Map<String, DownloadEntry> = emptyMap(),
        /** Bumped when the handle was re-opened, so a list re-windows. */
        val listRevision: Int = 0,
    ) {
        /** `true` when at least one title is on the device. */
        val hasDownloads: Boolean get() = (storage?.tracks ?: 0u) > 0u

        /** The download button's state for one context. */
        fun set(context: String): SetProgress? = sets.firstOrNull { it.context == context }

        /** A context this screen queued and the core has not published yet. */
        fun isPending(context: String): Boolean = pending.contains(context)
    }

    private val attached = MutableStateFlow(false)
    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var subscribed = false

    /** Subscribe once, then paint: the listener's first value IS the state. */
    fun start() {
        if (subscribed) return
        subscribed = true
        scope.launch {
            try {
                withContext(io) {
                    port.subscribe(object : OfflineListener {
                        override fun onProgress(state: OfflineState) {
                            // A Rust thread: hand it to the flow and return.
                            apply(state)
                        }

                        override fun onFailure(key: String, cause: String) {
                            // The CAUSE, in the person's words (rule 2), from a
                            // Rust thread: a toast is a state write, not work.
                            toast("Téléchargement impossible : $cause", null, null)
                        }
                    })
                }
                attached.value = true
                refresh()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update {
                    it.copy(loading = false, failure = causeText(e))
                }
            }
        }
    }

    /** Reads the local store: the state, the play cache's size, the disk's. */
    fun refresh() {
        scope.launch {
            try {
                val state = withContext(io) { port.state() }
                apply(state)
                val cacheBytes = withContext(io) { port.playCacheBytes() }
                _state.update { it.copy(playCacheBytes = cacheBytes) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update {
                    it.copy(
                        loading = false,
                        failure = it.failure ?: causeText(e),
                        // A copy already on screen stays: it IS the device's
                        // own state, and losing it would blank the section.
                        stale = it.version != 0uL,
                    )
                }
            }
        }
    }

    private fun apply(state: OfflineState) {
        val previous = _state.value
        _state.update {
            it.copy(
                loading = false,
                failure = null,
                stale = false,
                version = state.version,
                worker = state.state,
                paused = state.paused,
                sets = state.sets,
                settings = state.settings,
                storage = state.storage,
                note = state.note,
                pending = it.pending - state.sets.map { set -> set.context }.toSet(),
                freeBytes = freeSpace(),
            )
        }
        // A list that is open follows the queue: a finished title appears
        // without the person leaving and coming back.
        if (previous.version != state.version && _state.value.handle != null) {
            openDownloads()
        }
    }

    /** Opens (or re-opens) « Titres téléchargés » as a windowed handle. */
    fun openDownloads() {
        scope.launch {
            try {
                val handle = withContext(io) { port.downloads() }
                _state.update {
                    it.copy(handle = handle, listRevision = it.listRevision + 1)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("Liste des téléchargements indisponible : ${causeText(e)}", "Réessayer") {
                    openDownloads()
                }
            }
        }
    }

    /** Quality and bytes of the keys a page is showing (never all of them). */
    suspend fun loadFacts(keys: List<String>) {
        if (keys.isEmpty()) return
        val known = _state.value.facts
        val missing = keys.filter { !known.containsKey(it) }
        if (missing.isEmpty()) return
        try {
            val facts = withContext(io) { port.facts(missing) }
            _state.update { state ->
                state.copy(
                    facts = state.facts + facts.associateBy { it.key },
                )
            }
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            // A missing size or rung costs a cell, never the list.
        }
    }

    // --- writes: optimistic, one request, rollback + « Réessayer » -----------

    /** `lib/offline.js#downloadTrack` for a whole context. */
    fun add(context: String, quality: String = "", title: String = "", artist: String = "") {
        if (_state.value.isPending(context)) return
        _state.update { it.copy(pending = it.pending + context) }
        scope.launch {
            try {
                val set = withContext(io) { port.addSet(context, quality, title, artist) }
                _state.update { state ->
                    state.copy(
                        pending = state.pending - context,
                        sets = state.sets.filterNot { it.context == context } + set,
                    )
                }
                toast("« ${set.title.ifEmpty { context }} » ajouté aux téléchargements", null, null)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(pending = it.pending - context) }
                toast("Téléchargement refusé : ${causeText(e)}", "Réessayer") {
                    add(context, quality, title, artist)
                }
            }
        }
    }

    /** Takes a set back, files and all (the settings list's bin). */
    fun removeSet(context: String) {
        val previous = _state.value.sets.filter { it.context == context }
        if (previous.isEmpty()) return
        _state.update { state -> state.copy(sets = state.sets.filterNot { it.context == context }) }
        scope.launch {
            try {
                val (kind, id) = splitContext(context)
                withContext(io) { port.removeSet(kind, id) }
                toast("« ${previous.first().title.ifEmpty { context }} » retiré des téléchargements", null, null)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { state -> state.copy(sets = state.sets + previous) }
                toast("Retrait impossible : ${causeText(e)}", "Réessayer") { removeSet(context) }
            }
        }
    }

    /** Removes ONE downloaded title. */
    fun removeDownload(key: String) {
        val previous = _state.value
        _state.update { state ->
            state.copy(facts = state.facts - key)
        }
        scope.launch {
            try {
                withContext(io) { port.removeDownload(key) }
                openDownloads()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { previous }
                toast("Retrait impossible : ${causeText(e)}", "Réessayer") { removeDownload(key) }
            }
        }
    }

    /** Gives one failed title (a stream id) or one set (`kind:id`) another go. */
    fun retry(key: String) {
        scope.launch {
            try {
                withContext(io) { port.retry(key) }
                refresh()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("Nouvel essai impossible : ${causeText(e)}", "Réessayer") { retry(key) }
            }
        }
    }

    /** Every failure at once (the section's one button). */
    fun retryAll() {
        scope.launch {
            try {
                withContext(io) { port.retryAll() }
                refresh()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("Nouvel essai impossible : ${causeText(e)}", "Réessayer") { retryAll() }
            }
        }
    }

    /** Pauses the queue; what is published stays. */
    fun pause() {
        _state.update { it.copy(paused = true) }
        scope.launch {
            try {
                withContext(io) { port.pause() }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(paused = false) }
                toast("Pause impossible : ${causeText(e)}", "Réessayer") { pause() }
            }
        }
    }

    fun resume() {
        _state.update { it.copy(paused = false) }
        scope.launch {
            try {
                withContext(io) { port.resume() }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(paused = true) }
                toast("Reprise impossible : ${causeText(e)}", "Réessayer") { resume() }
            }
        }
    }

    /** `offline.quality` — the rung a NEW download takes. */
    fun setQuality(quality: String) {
        val previous = _state.value.settings
        _state.update { state ->
            state.copy(settings = state.settings?.copy(quality = quality) ?: state.settings)
        }
        scope.launch {
            try {
                withContext(io) { port.setQuality(quality) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(settings = previous) }
                toast("Qualité non enregistrée : ${causeText(e)}", "Réessayer") { setQuality(quality) }
            }
        }
    }

    /** `offline.onlyDownloaded`. */
    fun setOfflineOnly(onlyDownloaded: Boolean) {
        val previous = _state.value.settings
        _state.update { state ->
            state.copy(
                settings = state.settings?.copy(offlineOnly = onlyDownloaded) ?: state.settings,
            )
        }
        scope.launch {
            try {
                withContext(io) { port.setOfflineOnly(onlyDownloaded) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(settings = previous) }
                toast("Réglage non enregistré : ${causeText(e)}", "Réessayer") {
                    setOfflineOnly(onlyDownloaded)
                }
            }
        }
    }

    /** `cache.limit`: lowering it trims the play cache on the spot. */
    fun setPlayCacheCap(bytes: Long) {
        val previous = _state.value.settings
        _state.update { state ->
            state.copy(settings = state.settings?.copy(playCacheCap = bytes) ?: state.settings)
        }
        scope.launch {
            try {
                withContext(io) { port.setPlayCacheCap(bytes) }
                refresh()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(settings = previous) }
                toast("Limite non enregistrée : ${causeText(e)}", "Réessayer") { setPlayCacheCap(bytes) }
            }
        }
    }

    /** Empties the play cache. A download is NOT a cache: none is touched. */
    fun clearPlayCache() {
        val previous = _state.value.playCacheBytes
        _state.update { it.copy(playCacheBytes = 0) }
        scope.launch {
            try {
                withContext(io) { port.clearPlayCache() }
                toast("Cache de lecture vidé", null, null)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(playCacheBytes = previous) }
                toast("Vidage impossible : ${causeText(e)}", "Réessayer") { clearPlayCache() }
            }
        }
    }

    companion object {
        /** `stores.js`' own rungs, in the order the picker shows them. */
        val QUALITIES: List<Pair<String, String>> = listOf(
            "FLAC" to "Sans perte — lourd",
            "OPUS_320" to "Haute qualité (recommandé)",
            "OPUS_128" to "Standard, léger",
            "OPUS_64" to "Données réduites",
        )

        /** `settings.svelte`'s five caps, in bytes and as they are read. */
        val CACHE_CAPS: List<Pair<Long, String>> = listOf(
            256L * 1024 * 1024 to "256 Mo",
            512L * 1024 * 1024 to "512 Mo",
            1024L * 1024 * 1024 to "1 Go",
            2L * 1024 * 1024 * 1024 to "2 Go",
            4L * 1024 * 1024 * 1024 to "4 Go",
        )

        /**
         * `lib/format.js#bytes`, ported verbatim: `1,5 Go`, `812 Ko`, `0 o`.
         *
         * The SPA's own rule, and it is not a taste: ONE decimal below ten units
         * (`1,4 Go`) and a rounded integer above (`812 Ko`, never `812,0 Ko`),
         * with `o/Ko/Mo/Go/To` — the units a French screen prints.
         */
        fun bytesLabel(bytes: Long): String {
            var value = bytes.coerceAtLeast(0).toDouble()
            val units = listOf("o", "Ko", "Mo", "Go", "To")
            var index = 0
            while (value >= 1024 && index < units.size - 1) {
                value /= 1024.0
                index += 1
            }
            val text = if (value < 10 && index > 0) {
                String.format(java.util.Locale.FRANCE, "%.1f", value)
            } else {
                Math.round(value).toString()
            }
            return "$text ${units[index]}"
        }

        /** The rung's label as a chip prints it (`OPUS_320` -> `Opus 320`). */
        fun qualityLabel(quality: String): String = when (quality) {
            "" -> ""
            "FLAC" -> "FLAC"
            else -> quality.replace("OPUS_", "Opus ").lowercase()
                .replaceFirstChar { it.uppercase() }
                .replace("opus ", "Opus ")
        }

        /** The disk's free space, from the platform (`StatFs`). */
        fun freeSpaceOf(dir: File): Long = runCatching {
            val stat = StatFs(dir.absolutePath)
            stat.availableBytes
        }.getOrDefault(0L)
    }
}

/**
 * The process's one downloads model: the album's button, the playlist's and
 * Réglages → Téléchargements all read the same queue, so a set queued from an
 * album shows up in the section in the same frame.
 *
 * Public so the shell can install a fake in a JVM test; a screen calls
 * [model] and nothing else.
 */
object OfflineHost {
    private var installed: DownloadsModel? = null

    /** The model, built on first use over the Rust service. */
    fun model(): DownloadsModel = installed ?: DownloadsModel(
        port = FfiOfflinePort(),
        // `StatFs` on the app's own files directory: the platform owns the
        // disk's free space (the core answers 0 there, see `StorageReport`).
        freeSpace = { NzBridge.dataDir()?.let(DownloadsModel::freeSpaceOf) ?: 0L },
    ).also { installed = it }

    /** A test's own model (never the process's). */
    fun install(model: DownloadsModel) {
        installed = model
    }

    /** Forgets the installed model (a test's cleanup, a core rebuild). */
    fun reset() {
        installed = null
        OfflineCore.forget()
    }
}
