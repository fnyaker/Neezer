package fr.neezer.mobile.bridge

import android.content.Context
import android.util.Log

/**
 * The one line of Android that must run BEFORE anything else (docs/agents/
 * mobile.md, *Pitfalls*): `rustls-platform-verifier`'s Android side needs this
 * process' `Context` before the first handshake, or every TLS request fails —
 * including the login.
 *
 * The native side is `Java_fr_neezer_mobile_bridge_Platform_init` in `nz-ffi`,
 * which calls `rustls_platform_verifier::android::init_with_env`. It is JNI and
 * not UniFFI because the argument is a `Context`, which no UniFFI record can
 * carry.
 */
object Platform {
    private const val TAG = "NeezerPlatform"

    /**
     * Whether the platform verifier accepted the context. `false` means every
     * `https://` request will fail: the diagnostic screen says so instead of
     * letting the login look like a bad password.
     */
    @Volatile
    var ready: Boolean = false
        private set

    /** The cause when [ready] is false, for Aide. */
    @Volatile
    var failure: String? = null
        private set

    private external fun init(context: Context)

    init {
        System.loadLibrary("nz_ffi")
    }

    /**
     * Installs the platform trust store. Called FIRST from [NzBridge.install],
     * before the core is built, and once: a second call is a no-op.
     */
    fun install(app: Context) {
        if (ready) return
        ready = runCatching { init(app.applicationContext) }
            .onFailure {
                failure = it.message ?: it::class.java.simpleName
                Log.e(TAG, "the platform TLS verifier could not be initialised", it)
            }
            .isSuccess
    }
}
