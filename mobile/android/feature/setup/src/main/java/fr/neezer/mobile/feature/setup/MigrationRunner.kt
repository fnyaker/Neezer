package fr.neezer.mobile.feature.setup

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
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
 * Runs the one-shot check of the old app's presence, at most once per process,
 * in the background (lot MB4, revised for the coexistence release; rule 2: long
 * work runs in the background and SAYS SO).
 *
 * It is started by [SetupMigrationProvider] at process start, so the first
 * screen can say what became of the old app before the person types anything.
 *
 * **What it can and cannot do now.** With its own `applicationId` this app is a
 * different package, and Android forbids one package from reading another's
 * private preferences, files, cookies or WebView storage. The automatic
 * takeover MB4 shipped — server, session, downloads, settings — was only
 * possible because that build installed OVER the old app. It is therefore gone,
 * not silently broken: [OldApp] answers whether the old app is installed, and
 * `nz-core` turns that into a sentence the person reads ("Android ne permet pas
 * …, reconnectez-vous ici, rien n'a été touché"). The old app keeps every byte;
 * uninstalling either one leaves the other alone.
 *
 * `runMigration` itself stays — it is the tested machinery for a future
 * explicit export — but it is fed an EMPTY input here: no private path is read,
 * no cookie is lifted, no hidden WebView is opened.
 *
 * [state] is what the screens paint: running, then the notice (what could not be
 * taken over, and why) or the cause that stopped it.
 */
object MigrationRunner {
    /** The journal's tag: Aide reads `neezer`'s lines, and this one says what the
     * one-shot migration did — a takeover that says nothing is a bug (rule 2). */
    private const val TAG = "NzMigration"

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
        val application = context.applicationContext
        if (!started.compareAndSet(false, true)) return
        // **MEASURED on API 35**: a ContentProvider's `onCreate` runs BEFORE
        // `Application.onCreate` (the framework installs the providers first), so
        // the core did not exist yet and the first version of this class did
        // nothing at all — the log said `core=false` then the core started.
        // Posting to the main looper puts the work after the message that is
        // running the whole boot, which is where the core is built; no sleep, no
        // latch, no polling.
        Handler(Looper.getMainLooper()).post { begin(application) }
    }

    /** Starts the migration once, now that the process is fully up. */
    private fun begin(app: Context) {
        Log.i(TAG, "startIfNeeded: core=${NzBridge.core != null}")
        scope.launch {
            val screens = NzBridge.core?.screens()
            if (screens == null) {
                started.set(false)
                Log.w(TAG, "no core: nothing can be taken over")
                _state.value = State(cause = "Le noyau de l'application n'est pas disponible")
                return@launch
            }
            val status = runCatching { screens.migrationStatus() }
                .getOrElse { error ->
                    started.set(false)
                    Log.w(TAG, "status failed: ${error.message}", error)
                    _state.value = State(cause = NzErrors.text(error))
                    return@launch
                }
            Log.i(
                TAG,
                "status: needed=${status.needed} done=${status.done} " +
                    "oldApp=${status.oldAppFound} oldAppInstalled=${OldApp.installed(app)}",
            )
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
                    Log.i(
                        TAG,
                        "done: tracks=${done.tracks} settings=${done.settings} " +
                            "session=${done.session} server=${done.server} " +
                            "failures=${done.failures.size}",
                    )
                    _state.value = State(
                        done = true,
                        notice = done.notice,
                        failures = done.failures,
                    )
                }
                .onFailure { error ->
                    started.set(false)
                    Log.w(TAG, "migration failed: ${error.message}", error)
                    _state.value = State(cause = NzErrors.text(error))
                }
        }
    }

    /**
     * The one step that is still possible: say whether the old app is installed.
     *
     * Every private source the first version read is another package's now, so
     * the input is deliberately empty. `nz-core` owns the sentence and the
     * report; this function owns the one Android fact it needs.
     */
    private suspend fun migrate(
        context: Context,
        screens: uniffi.nz_ffi.Screens,
    ): MigrationReportRecord {
        val input = MigrationInputRecord(
            prefs = uniffi.nz_ffi.OldPrefsRecord(
                host = "",
                port = "",
                verifySsl = true,
                pinOrigin = null,
                pinCert = null,
            ),
            cookie = null,
            downloads = emptyList(),
            settings = emptyList(),
            offlineSets = 0u,
            settingsCause = null,
            oldAppInstalled = OldApp.installed(context),
        )
        return screens.runMigration(input)
    }

    /** Test hook: forget the "already started" latch (a process is once). */
    internal fun reset() {
        started.set(false)
        _state.value = State()
    }
}
