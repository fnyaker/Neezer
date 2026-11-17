package fr.neezer.mobile.app

import android.app.Application
import fr.neezer.mobile.bridge.NzBridge

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
    }
}
