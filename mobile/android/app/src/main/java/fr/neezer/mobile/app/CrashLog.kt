package fr.neezer.mobile.app

import android.app.Application
import android.content.Context
import fr.neezer.mobile.bridge.CrashRecord

/**
 * Le gestionnaire de plantage non rattrapé.
 *
 * Il ÉCRIT puis CHAÎNE au gestionnaire par défaut : Android tue le processus et
 * montre son dialogue comme avant — on ajoute une trace, on ne change pas le
 * comportement du système et on n'avale jamais l'exception.
 *
 * Le champ est un [Application], pas un `Context` : un singleton qui garde un
 * `Context` est exactement ce que lint refuse (`StaticFieldLeak`) — et c'est ce
 * que `NzBridge` fait déjà.
 */
object CrashLog {
    private var installed = false
    private var app: Application? = null

    fun install(context: Context) {
        if (installed) return
        installed = true
        app = context.applicationContext as? Application
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, error ->
            app?.let { ctx ->
                CrashRecord.write(
                    context = ctx,
                    thread = thread.name,
                    stack = error.stackTraceToString(),
                )
            }
            previous?.uncaughtException(thread, error)
        }
    }
}
