package fr.neezer.mobile.app

import android.app.Application
import fr.neezer.mobile.bridge.NzBridge
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
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
    override fun onCreate() {
        super.onCreate()
        NzBridge.install(this)
        // A launch with a cookie — the person is already signed in — restores the
        // session INTO THE CORE (`restore()` is `/api/me`). Without it `Core::me()`
        // stays `None` for the whole process, and every screen that reads
        // `/api/me`'s flags must guess: MB17's administrator gate had no other way
        // to know it, and its two pages were unreachable on a phone (measured on
        // the emulator). One request, on the net scope, never on the first frame.
        CoroutineScope(Dispatchers.IO).launch { runCatching { NzBridge.core?.restore() } }
    }
}
