package org.nsupysonic.app

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

/** One playlist pinned by the person, in the order the car shows them. */
data class PinnedPlaylist(val id: String, val title: String, val nbTracks: Int = 0) {
    fun toJson(): JSONObject = JSONObject()
        .put("id", id)
        .put("title", title)
        .put("n", nbTracks)

    companion object {
        fun fromJson(o: JSONObject): PinnedPlaylist? {
            val id = o.optString("id").takeIf { it.isNotBlank() } ?: return null
            return PinnedPlaylist(id, o.optString("title").take(200), o.optInt("n", 0))
        }
    }
}

/**
 * The car mode's own preferences, in the app's SharedPreferences convention
 * (no DataStore dependency, no server round-trip). Everything is on this
 * device: visibility, opacity, keyboard, triggers and the pinned order.
 *
 * The opacity is CLAMPED here, at the only door: Android 12+ stops delivering
 * touches to an overlay whose background is more than 80 % opaque, and a car
 * UI that cannot be tapped is worse than a translucent one.
 */
class CarPrefs(context: Context) {
    private val sp = context.applicationContext.getSharedPreferences("car", Context.MODE_PRIVATE)

    var overlayAlpha: Float
        get() = sp.getFloat(KEY_ALPHA, 0.62f).coerceIn(0.40f, 0.80f)
        set(v) = sp.edit().putFloat(KEY_ALPHA, v.coerceIn(0.40f, 0.80f)).apply()

    /** Show the overlay when a GPS app comes to the foreground. */
    var gpsTrigger: Boolean
        get() = sp.getBoolean(KEY_GPS, true)
        set(v) = sp.edit().putBoolean(KEY_GPS, v).apply()

    /** Also show it when the car's Bluetooth sink connects. */
    var bluetoothTrigger: Boolean
        get() = sp.getBoolean(KEY_BT, true)
        set(v) = sp.edit().putBoolean(KEY_BT, v).apply()

    /** Packages considered navigation apps. */
    var gpsPackages: Set<String>
        get() {
            val raw = sp.getString(KEY_PACKAGES, null) ?: return DEFAULT_GPS
            return try {
                val a = JSONArray(raw)
                val out = linkedSetOf<String>()
                for (i in 0 until a.length()) {
                    a.optString(i).takeIf { it.isNotBlank() }?.let { out.add(it) }
                }
                out
            } catch (_: Exception) {
                DEFAULT_GPS
            }
        }
        set(v) {
            val a = JSONArray()
            v.filter { it.isNotBlank() }.forEach(a::put)
            sp.edit().putString(KEY_PACKAGES, a.toString()).apply()
        }

    /** auto | azerty | qwerty | qwertz | abc */
    var keyboardLayout: String
        get() = sp.getString(KEY_LAYOUT, "auto")?.takeIf { it in LAYOUTS } ?: "auto"
        set(v) = sp.edit().putString(KEY_LAYOUT, if (v in LAYOUTS) v else "auto").apply()

    /** 2–4 letters while driving; the next letter starts a new query. */
    var maxLetters: Int
        get() = sp.getInt(KEY_LETTERS, 3).coerceIn(2, 4)
        set(v) = sp.edit().putInt(KEY_LETTERS, v.coerceIn(2, 4)).apply()

    /** Invert the ultra-button's left/right gestures (for RTL habits). */
    var invertSwipe: Boolean
        get() = sp.getBoolean(KEY_INVERT, false)
        set(v) = sp.edit().putBoolean(KEY_INVERT, v).apply()

    /** Pinned playlists, in display order. "Mon Flow" is implicit, never stored. */
    var pins: List<PinnedPlaylist>
        get() {
            val raw = sp.getString(KEY_PINS, null) ?: return emptyList()
            return try {
                val a = JSONArray(raw)
                val out = mutableListOf<PinnedPlaylist>()
                for (i in 0 until a.length()) {
                    val item = PinnedPlaylist.fromJson(a.optJSONObject(i) ?: JSONObject())
                    if (item != null) out.add(item)
                }
                out
            } catch (_: Exception) {
                emptyList()
            }
        }
        set(v) {
            val a = JSONArray()
            v.distinctBy { it.id }.forEach { a.put(it.toJson()) }
            sp.edit().putString(KEY_PINS, a.toString()).apply()
        }

    fun isPinned(id: String): Boolean = pins.any { it.id == id }

    fun pin(item: PinnedPlaylist) {
        if (isPinned(item.id)) return
        pins = pins + item
    }

    fun unpin(id: String) {
        pins = pins.filterNot { it.id == id }
    }

    fun movePin(id: String, delta: Int) {
        val list = pins.toMutableList()
        val at = list.indexOfFirst { it.id == id }
        if (at < 0) return
        val to = (at + delta).coerceIn(0, list.lastIndex)
        if (to == at) return
        val item = list.removeAt(at)
        list.add(to, item)
        pins = list
    }

    companion object {
        private const val KEY_ALPHA = "overlayAlpha"
        private const val KEY_GPS = "gpsTrigger"
        private const val KEY_BT = "bluetoothTrigger"
        private const val KEY_PACKAGES = "gpsPackages"
        private const val KEY_LAYOUT = "keyboardLayout"
        private const val KEY_LETTERS = "maxLetters"
        private const val KEY_INVERT = "invertSwipe"
        private const val KEY_PINS = "pins"

        val LAYOUTS = setOf("auto", "azerty", "qwerty", "qwertz", "abc")

        /** Maps, Waze, OsmAnd, Organic Maps, Sygic — the prototype's list. */
        val DEFAULT_GPS: Set<String> = setOf(
            "com.google.android.apps.maps",
            "com.waze",
            "net.osmand",
            "net.osmand.plus",
            "app.organicmaps",
            "com.sygic.aura",
        )
    }
}
