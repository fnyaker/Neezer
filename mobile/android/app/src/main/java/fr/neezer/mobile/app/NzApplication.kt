package fr.neezer.mobile.app

import android.app.Application
import fr.neezer.mobile.bridge.NzBridge
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * The app's process, and the ONE place the Rust core is built
 * (docs/agents/mobile.md: ":bridge at `Application.onCreate` (cheap: opens
 * SQLite, starts the runtime; no network)").
 *
 * Android kills the process whenever it likes, so nothing correct may depend on
 * a clean shutdown: state lives in SQLite (WAL) after every change, and
 * `NzCore::shutdown` is for the tests only. MB2 fills what [NzBridge.install]
 * does; the network callback and the foreground lifecycle land there.
 */
class NzApplication : Application() {
    /**
     * The process' scope for the core calls that outlive every screen — today the
     * one session restore below. A `SupervisorJob`, so one failed call never
     * cancels the others, and it lives as long as the process (Android kills it;
     * nothing correct may depend on a clean stop).
     */
    private val processScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    override fun onCreate() {
        super.onCreate()
        // Avant le noyau : si c'est lui qui tombe, la trace existe déjà.
        CrashLog.install(this)
        NzBridge.install(this)
        // A launch with a cookie — the person is already signed in — restores the
        // session INTO THE CORE (`restore()` is `/api/me`). Without it `Core::me()`
        // stays `None` for the whole process, and every screen that reads
        // `/api/me`'s flags must guess: MB17's administrator gate had no other way
        // to know it, and its two pages were unreachable on a phone (measured on
        // the emulator).
        //
        // Off a network it fails, and that is not a crash: the app keeps working
        // from the device (hors ligne), the cause is logged without a cookie or a
        // token, and the next Réglages visit asks again — which is what makes the
        // administrator categories reappear once the link is back.
        processScope.launch { restoreSession() }
    }

    private suspend fun restoreSession() {
        val core = NzBridge.core ?: return
        runCatching { core.restore() }
            .onFailure { cause ->
                Log.w(TAG, "session restore: ${cause::class.java.simpleName} ${cause.message ?: ""}")
            }
    }

    private companion object {
        const val TAG = "NeezerCore"
    }
}
