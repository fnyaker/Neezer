package fr.neezer.mobile.bridge

import android.app.Application
import android.os.Build
import android.util.Log
import java.io.File
import uniffi.nz_ffi.CoreConfig
import uniffi.nz_ffi.NzCore
import uniffi.nz_ffi.nzVersion

/**
 * The one holder of the Rust core, built once by [install] at
 * `Application.onCreate` (docs/agents/mobile.md, "The FFI boundary").
 *
 * Cheap by design: MB2 opens SQLite and starts the runtime here, and nothing
 * touches the network. Every screen reaches the core through [core]; a `null`
 * core means the native library could not be loaded at all, and the app says so
 * in Aide instead of crashing.
 *
 * MB0: the object graph exists and answers `nz_version()`; MB2 fills [install]
 * (cache directories, the platform TLS init, the network callback, the Coil
 * loader).
 */
object NzBridge {
    private const val TAG = "NeezerCore"

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

    /**
     * Builds the object graph. Called exactly once, from `Application.onCreate`.
     *
     * The version is read FIRST and on its own: it is the call that proves the
     * `.so` loaded, and a failure there must not be confused with a failure to
     * open the caches.
     */
    fun install(app: Application) {
        application = app
        version = runCatching { nzVersion() }
            .onFailure { Log.w(TAG, "nz_version() failed: ${it.message}", it) }
            .getOrNull()

        val dataDir = File(app.filesDir, "neezer").apply { mkdirs() }
        val cacheDir = File(app.cacheDir, "neezer").apply { mkdirs() }
        core = runCatching {
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
