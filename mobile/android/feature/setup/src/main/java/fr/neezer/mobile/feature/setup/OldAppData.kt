package fr.neezer.mobile.feature.setup

import android.content.Context
import android.webkit.CookieManager
import java.io.File
import org.json.JSONObject
import uniffi.nz_ffi.OldDownloadRecord
import uniffi.nz_ffi.OldPrefsRecord
import uniffi.nz_ffi.OldSettingRecord

/**
 * What the OLD app left on this device, read with the platform's own APIs (lot
 * MB4, rule 7).
 *
 * The new app keeps the old one's `applicationId` and signing key, so an update
 * installs OVER it and its data directory survives. Three things are read here
 * and nowhere else:
 *
 * - **`SharedPreferences("settings")`** — the old app's `Prefs`: `host`, `port`,
 *   `verifySsl`, `pinOrigin`, `pinCert`. Android owns that file's format, so
 *   this is a platform read; what the values MEAN is `nz-core`'s.
 * - **`filesDir/offline`** — its downloads: `<sha40>.audio` with a `<sha40>.json`
 *   sidecar naming the track key and the content type. The names are handed to
 *   Rust as NAMES (`OldDownloadRecord.file`): a hostile sidecar can never walk
 *   out of the directory.
 * - **`CookieManager.getCookie(<configured origin>)`** — the session the old
 *   WebView still holds, so the person is not asked to sign in again. Only the
 *   configured origin's cookie is ever asked for.
 *
 * Nothing here writes, deletes or even renames: the migration's own moves happen
 * in Rust, and the old WebView's data directory is left exactly where it is.
 */
internal object OldAppData {
    private const val PREFS = "settings"

    /** The old app's server, or `null` when it never got that far. */
    fun prefs(context: Context): OldPrefsRecord? {
        val shared = runCatching {
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        }.getOrNull() ?: return null
        val host = shared.getString("host", "").orEmpty()
        if (host.isBlank() && shared.getBoolean("configured", false).not()) return null
        return OldPrefsRecord(
            host = host,
            port = shared.getString("port", "").orEmpty(),
            verifySsl = shared.getBoolean("verifySsl", true),
            pinOrigin = shared.getString("pinOrigin", null),
            pinCert = shared.getString("pinCert", null),
        )
    }

    /** Whether the old app ran here at all (its preferences file exists). */
    fun found(context: Context): Boolean =
        File(context.filesDir, "offline").isDirectory ||
            File(context.applicationInfo.dataDir, "shared_prefs/$PREFS.xml").isFile

    /**
     * The old downloads' files, with the track key and content type their
     * sidecars name. `owners` is the SPA's IndexedDB answer (key → account), so
     * a track lands in the right account's list; an unknown key keeps an empty
     * owner, which is what the SPA wrote before accounts existed.
     */
    fun downloads(context: Context, owners: Map<String, String>): List<OldDownloadRecord> {
        val directory = File(context.filesDir, "offline")
        val sidecars = directory.listFiles { file -> file.isFile && file.name.endsWith(".json") }
            ?: return emptyList()
        val out = ArrayList<OldDownloadRecord>(sidecars.size)
        for (sidecar in sidecars) {
            val base = sidecar.name.removeSuffix(".json")
            val audio = File(directory, "$base.audio")
            if (!audio.isFile || audio.length() == 0L) continue
            val record = runCatching { JSONObject(sidecar.readText()) }.getOrNull() ?: continue
            val key = record.optString("id", "").trim()
            if (key.isEmpty()) continue
            out.add(
                OldDownloadRecord(
                    file = audio.name,
                    key = key,
                    owner = owners[key].orEmpty(),
                    contentType = record.optString("type", "audio/flac").ifBlank { "audio/flac" },
                ),
            )
        }
        return out
    }

    /**
     * The session cookie the old WebView holds for [origin], or `null`. The
     * caller passes the CONFIGURED origin: `CookieManager` would happily answer
     * for any host, and a cookie lifted from one must never be handed to
     * another.
     */
    fun cookie(origin: String): String? = runCatching {
        val manager = CookieManager.getInstance()
        manager.flush()
        manager.getCookie(origin)
    }.getOrNull()?.takeIf { it.isNotBlank() }
}

/**
 * The `localStorage` values a page can hold, as the SPA wrote them.
 *
 * The mapping table is `nz-core`'s (`setup::migration::KNOWN_SETTINGS`): this only
 * carries every key it finds, so the table can grow without touching Kotlin —
 * and a key the new app does not know is ignored there, never planted here.
 */
internal fun settingsFrom(json: JSONObject?): List<OldSettingRecord> {
    if (json == null) return emptyList()
    val out = ArrayList<OldSettingRecord>(json.length())
    val keys = json.keys()
    while (keys.hasNext()) {
        val key = keys.next()
        val value = json.optString(key, "")
        if (value.isEmpty()) continue
        out.add(OldSettingRecord(key = key, value = value))
    }
    return out
}
