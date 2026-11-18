package fr.neezer.mobile.feature.lan

import android.app.Application
import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.net.Uri

/**
 * Joins the app's process start without editing `:app` (lot LN14).
 *
 * A feature module may not touch `NzApplication` (worker rule 3: the shell's
 * files belong to MB0/MB2), and the LAN node has to be wired before the first
 * frame anyway: a person who opens the app and plays must be discoverable
 * without first visiting « Appareils à proximité ». A library `ContentProvider`
 * declared in this module's manifest is the standard Android way to do that —
 * WorkManager, Firebase and AndroidX Startup all use it — and it costs no
 * dependency.
 *
 * Android instantiates providers BEFORE `Application.onCreate`, so this only
 * records the [Application]; [LanHost.install] reads `NzBridge.core` lazily and
 * the lifecycle observer fires later, when the core exists.
 */
class LanInitProvider : ContentProvider() {
    override fun onCreate(): Boolean {
        (context?.applicationContext as? Application)?.let { app ->
            runCatching { LanHost.install(app) }
        }
        return true
    }

    override fun query(
        uri: Uri,
        projection: Array<out String>?,
        selection: String?,
        selectionArgs: Array<out String>?,
        sortOrder: String?,
    ): Cursor? = null

    override fun getType(uri: Uri): String? = null

    override fun insert(uri: Uri, values: ContentValues?): Uri? = null

    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = 0

    override fun update(
        uri: Uri,
        values: ContentValues?,
        selection: String?,
        selectionArgs: Array<out String>?,
    ): Int = 0
}
