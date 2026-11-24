package fr.neezer.mobile.bridge

import android.app.Application
import android.os.Build
import android.util.Log
import java.io.File
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import uniffi.nz_ffi.CoreConfig
import uniffi.nz_ffi.Me
import uniffi.nz_ffi.NzCore
import uniffi.nz_ffi.SessionListener
import uniffi.nz_ffi.nzVersion

/**
 * The one holder of the Rust core, built once by [install] at
 * `Application.onCreate` (docs/agents/mobile.md, "The FFI boundary").
 *
 * Cheap by design: the core opens SQLite and its own migrations; nothing touches
 * the network. Every screen reaches the core through [core]; a `null` core means
 * the native library could not be loaded at all, and Aide says so instead of the
 * app crashing.
 *
 * [install] does four things, in this order, because the order is the contract:
 *
 * 1. **the platform TLS verifier** ([Platform.install]) — without it every
 *    `https://` request fails, and the login is the first one;
 * 2. **the core** over `filesDir/neezer` and `cacheDir/neezer`;
 * 3. **the network callback** (`ConnectivityManager`), so `set_network(None)`
 *    makes the next request answer "Hors ligne" in zero sockets;
 * 4. **the foreground observer** (`ProcessLifecycleOwner`), so prefetching only
 *    happens while somebody is looking (rule 1).
 *
 * Coil is installed separately ([NeezerImages]) because a cover is a screen's
 * business, not the core's.
 */
object NzBridge {
    private const val TAG = "NeezerCore"

    /** A session event, already off the Rust thread and safe to collect. */
    sealed interface Session {
        /** The session ended; [reason] is the French sentence to show. */
        data class Ended(val reason: String) : Session

        /** The signed-in person changed: `null` means signed out. */
        data class MeChanged(val me: Me?) : Session
    }

    private var application: Application? = null

    /** The core, or `null` when the native library could not be built. */
    var core: NzCore? = null
        private set

    /** `nz_version()`'s answer — the Rust crate's version, shown in Aide. */
    var version: String? = null
        private set

    /** The throwable that kept [core] from being built, for the Aide screen. */
    var failure: String? = null
        private set

    private val sessionEvents = MutableSharedFlow<Session>(
        replay = 0,
        extraBufferCapacity = 8,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )

    /**
     * What the shell collects. Every callback hands its event here and RETURNS
     * at once: the listener runs on a Rust worker, and suspending there would
     * block the network thread that fired it.
     */
    val sessions: SharedFlow<Session> = sessionEvents

    /**
     * Builds the object graph. Called exactly once, from `Application.onCreate`.
     *
     * The version is read FIRST and on its own: it is the call that proves the
     * `.so` loaded, and a failure there must not be confused with a failure to
     * open the caches.
     */
    fun install(app: Application) {
        application = app

        // 1. The trust store, BEFORE any request can exist.
        Platform.install(app)

        version = runCatching { nzVersion() }
            .onFailure { Log.w(TAG, "nz_version() failed: ${it.message}", it) }
            .getOrNull()

        val dataDir = File(app.filesDir, "neezer").apply { mkdirs() }
        val cacheDir = File(app.cacheDir, "neezer").apply { mkdirs() }
        val built = runCatching {
            NzCore(
                CoreConfig(
                    dataDir = dataDir.absolutePath,
                    cacheDir = cacheDir.absolutePath,
                    appVersion = BuildConfigFields.versionName(app),
                    versionCode = BuildConfigFields.versionCode(app).toULong(),
                    sdkInt = Build.VERSION.SDK_INT.toUInt(),
                    deviceName = "${Build.MANUFACTURER} ${Build.MODEL}".trim(),
                )
            )
        }.onFailure {
            failure = it.message ?: it::class.java.simpleName
            Log.w(TAG, "nz-core could not be built: $failure", it)
        }.getOrNull()
        core = built
        if (built == null) return

        // 3. The link: the core must learn about a dead radio AT ONCE, not after
        // a timeout (`docs/agents/mobile.md`, *Network errors*).
        built.subscribeSession(object : SessionListener {
            override fun onSessionEnded(reason: String) {
                sessionEvents.tryEmit(Session.Ended(reason))
            }

            override fun onMeChanged(me: Me?) {
                sessionEvents.tryEmit(Session.MeChanged(me))
            }
        })
        NetworkWatcher.start(app, built)

        // The cover loader: Rust's files in, the platform decoder out.
        NeezerImages.install(app, built.screens())

        // 4. Somebody looking at the app: prefetch may run.
        ForegroundWatcher.start(app, built)
    }

    /** `true` when the app may still reach the core (Aide shows why not). */
    val ready: Boolean
        get() = core != null

    /** The directories the core opened, for the diagnostic screen. */
    fun dataDir(): File? = application?.let { File(it.filesDir, "neezer") }

    /** Stops the observer and the core: tests and the process-death path. */
    fun shutdown() {
        NetworkWatcher.stop()
        ForegroundWatcher.stop()
        // The LAN node first (LN15): `LanApi::stop` joins `nz-lan` and its hasher and
        // frees the UDP port, which `NzCore::shutdown` alone left to the process
        // exit (the hole LN14 reported). It is the FFI's own door and not
        // `LanHost.shutdown()` because this module cannot depend on `:feature:lan`
        // (`:feature:lan` depends on `:bridge`): NSD and the activity observers are
        // the feature module's, and a process that is stopping does not need them
        // stopped first.
        core?.lan()?.stop()
        core?.shutdown()
        core = null
    }
}

/**
 * The two version numbers the core needs, read without a generated `BuildConfig`
 * in this module (`:app` owns them; `:bridge` must not depend on `:app`).
 */
private object BuildConfigFields {
    fun versionName(app: Application): String = app.packageManager
        .getPackageInfo(app.packageName, 0)
        .versionName
        ?: "0"

    fun versionCode(app: Application): Long {
        val info = app.packageManager.getPackageInfo(app.packageName, 0)
        @Suppress("DEPRECATION")
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            info.longVersionCode
        } else {
            info.versionCode.toLong()
        }
    }
}
