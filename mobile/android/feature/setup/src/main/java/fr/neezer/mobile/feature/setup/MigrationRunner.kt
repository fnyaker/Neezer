package fr.neezer.mobile.feature.setup

import android.content.Context
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzErrors
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.nz_ffi.MigrationFailureRecord
import uniffi.nz_ffi.MigrationInputRecord
import uniffi.nz_ffi.MigrationReportRecord

/**
 * Runs the one-shot migration from the old app, at most once per process, in the
 * background (lot MB4; rule 2: long work runs in the background and SAYS SO).
 *
 * It is started by [SetupMigrationProvider] at process start — a provider is the
 * only place that runs without a screen, which is exactly what an update owes:
 * the person taps the icon and finds their server, their session, their
 * downloads and their settings already there.
 *
 * The order is the note's, and each step is what makes the next one possible:
 *
 * 1. the old preferences (the server), read by the platform;
 * 2. the origin normalised by `nz-core` (`serverUrl`) — never by a second URL
 *    builder in Kotlin, which is how `…/musique:5722` happened once;
 * 3. the session cookie, for THAT origin only;
 * 4. the hidden WebView, for the SPA's `localStorage` and IndexedDB;
 * 5. `Screens.runMigration`, which moves the files and writes the settings.
 *
 * [state] is what the screens paint: running, then the notice (what was taken
 * over, and the CAUSE of everything that was not) or the cause that stopped it.
 */
object MigrationRunner {
    /** What the screens show. */
    data class State(
        /** The migration is running right now. */
        val running: Boolean = false,
        /** It finished; [notice] says what it did. */
        val done: Boolean = false,
        /** The French sentence: what was taken over, or why nothing was. */
        val notice: String? = null,
        /** Each step that could not be taken, with its cause. */
        val failures: List<MigrationFailureRecord> = emptyList(),
        /** The migration could not run at all (the core is missing, say). */
        val cause: String? = null,
    )

    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private val started = AtomicBoolean(false)
    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    /**
     * Starts it if it is owed, once: a second call (a screen opening while the
     * provider's run is in flight) is the same run.
     */
    fun startIfNeeded(context: Context) {
        val app = context.applicationContext
        if (!started.compareAndSet(false, true)) return
        scope.launch {
            val screens = NzBridge.core?.screens()
            if (screens == null) {
                started.set(false)
                _state.value = State(cause = "Le noyau de l'application n'est pas disponible")
                return@launch
            }
            val status = runCatching { screens.migrationStatus() }
                .getOrElse { error ->
                    started.set(false)
                    _state.value = State(cause = NzErrors.text(error))
                    return@launch
                }
            if (!status.needed) {
                _state.value = State(
                    done = true,
                    notice = status.report?.notice,
                    failures = status.report?.failures.orEmpty(),
                )
                return@launch
            }
            _state.value = State(running = true)
            val report = runCatching { migrate(app, screens) }
            report
                .onSuccess { done ->
                    _state.value = State(
                        done = true,
                        notice = done.notice,
                        failures = done.failures,
                    )
                }
                .onFailure { error ->
                    started.set(false)
                    _state.value = State(cause = NzErrors.text(error))
                }
        }
    }

    /** The five steps, in the order the note names them. */
    private suspend fun migrate(
        context: Context,
        screens: uniffi.nz_ffi.Screens,
    ): MigrationReportRecord {
        val prefs = OldAppData.prefs(context)
        // The origin is `nz-core`'s own normalisation: the same function the
        // setup screen uses, so a reverse-proxied host keeps its path and the
        // port never lands after it.
        val origin = prefs?.let { screens.serverUrl(it.host, it.port) }
        val cookie = origin?.let { OldAppData.cookie(it) }
        val found = origin?.let { MigrationWeb.readOrNull(context, it) }
        val downloads = OldAppData.downloads(context, found?.owners.orEmpty())
        val input = MigrationInputRecord(
            prefs = prefs ?: uniffi.nz_ffi.OldPrefsRecord(
                host = "",
                port = "",
                verifySsl = true,
                pinOrigin = null,
                pinCert = null,
            ),
            cookie = cookie,
            downloads = downloads,
            settings = found?.settings.orEmpty(),
            offlineSets = (found?.albums ?: 0).toUInt(),
        )
        return screens.runMigration(input)
    }

    /** Test hook: forget the "already started" latch (a process is once). */
    internal fun reset() {
        started.set(false)
        _state.value = State()
    }
}
