package fr.neezer.mobile.bridge

import android.app.Application
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import uniffi.nz_ffi.NzCore

/**
 * `ProcessLifecycleOwner` → `NzCore.set_foreground`.
 *
 * Rule 1 puts "what comes right after the action just taken" (the next track,
 * the next screen) above everything else, but only while it can be SEEN: a
 * backgrounded app must not prefetch, and `nz-api`'s `Prefetch` lane is what the
 * core gates on this flag. `ProcessLifecycleOwner`'s callbacks fire on the main
 * thread and the FFI call is a plain atomic store there — nothing waits.
 */
internal object ForegroundWatcher {
    private var observer: DefaultLifecycleObserver? = null

    fun start(app: Application, core: NzCore) {
        if (observer != null) return
        val watcher = object : DefaultLifecycleObserver {
            override fun onStart(owner: LifecycleOwner) {
                core.setForeground(true)
            }

            override fun onStop(owner: LifecycleOwner) {
                core.setForeground(false)
            }
        }
        observer = watcher
        ProcessLifecycleOwner.get().lifecycle.addObserver(watcher)
    }

    fun stop() {
        val watcher = observer ?: return
        runCatching { ProcessLifecycleOwner.get().lifecycle.removeObserver(watcher) }
        observer = null
    }

    val started: Boolean
        get() = observer != null
}
