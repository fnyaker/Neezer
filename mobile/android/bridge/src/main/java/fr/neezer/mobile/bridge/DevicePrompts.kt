package fr.neezer.mobile.bridge

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build

/**
 * Les demandes d'autorisation qui appartiennent au CONTEXTE, pas au démarrage
 * (la règle déjà suivie par le réseau local, `LanScreen.kt:582`).
 *
 * `POST_NOTIFICATIONS` est déclarée depuis MB0 et n'était jamais demandée : la
 * notification du mode voiture et la progression des téléchargements restaient
 * muettes jusqu'à ce que la personne aille les activer dans les réglages Android
 * (audit §1). Android 13+ exempte la session média, donc la lecture n'y perd
 * rien : ce sont les avis de TÉLÉCHARGEMENT et de VOITURE qui en dépendent.
 *
 * Une seule mémoire locale pour les deux contextes : la personne a répondu une
 * fois, on ne redemande pas. `SharedPreferences` et non le `kv` du noyau : c'est
 * une trace d'interface, pas une préférence migrée du SPA (même choix que
 * `playback/BatteryExemption.kt`). Aucune dépendance ajoutée : `minSdk` est 26,
 * donc `Context.checkSelfPermission` suffit là où `ContextCompat` serait requis.
 */
object DevicePrompts {
    private const val PREFS = "device-prompts"
    private const val ASKED_NOTIFICATIONS = "notifications.asked"

    /** La phrase du refus, dans les mots de la personne (rule 3). */
    const val NOTIFICATIONS_LOST =
        "Sans notifications, la progression ne s'affichera pas hors de l'application. " +
            "Le téléchargement continue."

    /** `POST_NOTIFICATIONS` manque-t-elle ET ne l'a-t-on jamais demandée ? */
    fun shouldAskNotifications(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < 33) return false
        val granted = context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        if (granted) return false
        return !prefs(context).getBoolean(ASKED_NOTIFICATIONS, false)
    }

    /** La réponse (accordée ou non) est retenue : on ne redemande pas. */
    fun noteNotificationsAnswered(context: Context) {
        prefs(context).edit().putBoolean(ASKED_NOTIFICATIONS, true).apply()
    }

    private fun prefs(context: Context) =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
}
