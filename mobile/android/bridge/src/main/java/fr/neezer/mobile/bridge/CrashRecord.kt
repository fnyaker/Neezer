package fr.neezer.mobile.bridge

import android.content.Context
import java.io.File

/**
 * Le dernier plantage non rattrapé, dans le stockage privé de l'application
 * (audit §9 : aucun gestionnaire n'existait, un plantage Kotlin partait avec le
 * dialogue d'Android et ne laissait rien dans Aide).
 *
 * Un fichier, pas SQLite : il s'écrit alors que le processus meurt, sans noyau et
 * sans allocation inutile. Une seule entrée, la dernière — un journal de
 * plantages serait un deuxième produit.
 *
 * Le stockage vit dans `:bridge` parce que `:feature:settings` ne dépend pas de
 * `:app` (et ne doit pas) : le gestionnaire écrit, la carte d'Assistance lit.
 */
object CrashRecord {
    private const val FILE = "last-crash.txt"

    /** Écrit l'incident ; jamais d'exception qui remonterait dans le gestionnaire. */
    fun write(context: Context, thread: String, stack: String) {
        runCatching {
            File(context.filesDir, FILE).writeText(
                "time=${System.currentTimeMillis()}\nthread=$thread\n$stack\n",
            )
        }
    }

    /** Le dernier incident, ou `null`. */
    fun read(context: Context): String? = runCatching {
        File(context.filesDir, FILE).takeIf { it.isFile }?.readText()
    }.getOrNull()

    fun clear(context: Context) {
        runCatching { File(context.filesDir, FILE).delete() }
    }
}
