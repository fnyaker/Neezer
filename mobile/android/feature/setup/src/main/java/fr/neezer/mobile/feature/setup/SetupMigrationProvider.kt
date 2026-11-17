package fr.neezer.mobile.feature.setup

import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.net.Uri

/**
 * Runs the migration from the old app when the process starts (lot MB4).
 *
 * A `ContentProvider` is the only component Android starts without a screen and
 * without a notification — `androidx.startup` is built out of one — and the
 * framework calls it AFTER `Application.onCreate`, so `NzBridge` has already
 * built the core (the migration needs it). That is the whole reason this class
 * exists: on the first launch after the update, the person's server, session,
 * downloads and settings are taken over before they see anything, and the update
 * test proves it without touching the app.
 *
 * Three properties it must keep:
 *
 * - **`onCreate` returns at once.** It only starts a coroutine on `Dispatchers.IO`;
 *   startup is measured (rule 4) and nothing here may be on that path.
 * - **It owns no data.** `query`/`insert`/`update`/`delete` throw: no other app
 *   can reach it (`exported="false"`), and this app does not read it either.
 * - **It is safe to be created twice** (a second process, a test): the runner's
 *   latch and `nz-core`'s `migration_done` flag both make a rerun a no-op.
 */
class SetupMigrationProvider : ContentProvider() {
    override fun onCreate(): Boolean {
        context?.let { started ->
            MigrationRunner.startIfNeeded(started.applicationContext)
        }
        return true
    }

    override fun query(
        uri: Uri,
        projection: Array<out String>?,
        selection: String?,
        selectionArgs: Array<out String>?,
        sortOrder: String?,
    ): Cursor? = throw UnsupportedOperationException("aucune donnée ici")

    override fun getType(uri: Uri): String? = null

    override fun insert(uri: Uri, values: ContentValues?): Uri? =
        throw UnsupportedOperationException("aucune donnée ici")

    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int =
        throw UnsupportedOperationException("aucune donnée ici")

    override fun update(
        uri: Uri,
        values: ContentValues?,
        selection: String?,
        selectionArgs: Array<out String>?,
    ): Int = throw UnsupportedOperationException("aucune donnée ici")

    companion object {
        /** The authority `feature/setup`'s manifest declares. */
        fun authority(context: Context): String = "${context.packageName}.setup-migration"
    }
}
