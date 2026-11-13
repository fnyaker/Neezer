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

/** One saved place on screen, in dp, from the display's left edge and bottom. */
data class CarSlot(val xDp: Float, val bottomDp: Float)

/** The display's safe insets, in dp (status bar, navigation bar, cutout). */
data class CarInsets(
    val leftDp: Float = 0f,
    val topDp: Float = 0f,
    val rightDp: Float = 0f,
    val bottomDp: Float = 0f,
)

/**
 * The card's geometry, pure so the JVM tests can hold the prototype's numbers
 * and the drag clamp without a device.
 *
 * The overlay's window is EXACTLY the painted surface (see CarModeService):
 * FULL/MINI are `screen - 16 dp` wide at x = 8 dp, ULTRA is 76 × 76 dp. The
 * stored slot is always measured from the START edge, even for ULTRA whose
 * window uses Gravity.END: the service converts it, so rotating the phone
 * keeps the button near the same corner instead of off screen.
 */
object CarPlacement {
    const val SIDE_DP = 8f
    const val CARD_BOTTOM_DP = 92f
    const val ULTRA_SIZE_DP = 76f
    const val ULTRA_END_DP = 14f
    const val ULTRA_BOTTOM_DP = 104f

    fun widthDp(size: CarSize, screenWidthDp: Float): Float =
        if (size == CarSize.ULTRA) ULTRA_SIZE_DP
        else (screenWidthDp - SIDE_DP * 2f).coerceAtLeast(1f)

    /**
     * Where the prototype puts each size on a fresh install: FULL/MINI at
     * x = 8 dp / bottom = 92 dp; ULTRA 76 dp square, 14 dp from the END edge
     * and 104 dp from the bottom.
     */
    fun defaults(size: CarSize, screenWidthDp: Float): CarSlot = if (size == CarSize.ULTRA) {
        CarSlot(screenWidthDp - ULTRA_END_DP - ULTRA_SIZE_DP, ULTRA_BOTTOM_DP)
    } else {
        CarSlot(SIDE_DP, CARD_BOTTOM_DP)
    }

    /**
     * Keep a slot inside the safe area. [windowHeightDp] is the measured card
     * height (0 before the first layout: only the bottom's lower bound and the
     * x bounds are enforced then; the service re-clamps after layout).
     */
    fun clamp(
        slot: CarSlot,
        windowWidthDp: Float,
        windowHeightDp: Float,
        screenWidthDp: Float,
        screenHeightDp: Float,
        insets: CarInsets = CarInsets(),
    ): CarSlot {
        val width = windowWidthDp.coerceAtLeast(1f)
        val height = windowHeightDp.coerceAtLeast(0f)
        val minX = insets.leftDp.coerceAtLeast(0f)
        val maxX = (screenWidthDp - width - insets.rightDp).coerceAtLeast(minX)
        val minBottom = insets.bottomDp.coerceAtLeast(0f)
        val maxBottom =
            (screenHeightDp - height - insets.topDp).coerceAtLeast(minBottom)
        return CarSlot(
            slot.xDp.coerceIn(minX, maxX),
            slot.bottomDp.coerceIn(minBottom, maxBottom),
        )
    }
}

/**
 * The storage key of one (navigation app, size) slot. Preferences keys are
 * flat strings, so the package goes in them verbatim (a package name cannot
 * contain ':') and an empty package means "no navigation app in front" — the
 * manual/Bluetooth case. No JSON, no path, no Android type: the JVM test can
 * round-trip every key and value.
 */
object CarPositions {
    private const val PREFIX = "position:"

    fun key(pkg: String?, size: CarSize): String =
        "$PREFIX${pkg.orEmpty()}:${size.name}"

    fun isPositionKey(key: String): Boolean = key.startsWith(PREFIX)

    /** The (package, size) a stored key names, or null for anything else. */
    fun parse(key: String): Pair<String?, CarSize>? {
        if (!key.startsWith(PREFIX)) return null
        val rest = key.removePrefix(PREFIX)
        val at = rest.lastIndexOf(':')
        if (at < 0) return null
        val size = CarSize.values().firstOrNull { it.name == rest.substring(at + 1) }
            ?: return null
        val pkg = rest.substring(0, at)
        return pkg.ifEmpty { null } to size
    }

    fun packageOf(key: String): String? = parse(key)?.first

    fun encode(slot: CarSlot): String = "${slot.xDp},${slot.bottomDp}"

    fun decode(raw: String?): CarSlot? {
        if (raw == null) return null
        val at = raw.indexOf(',')
        if (at <= 0) return null
        val x = raw.substring(0, at).toFloatOrNull() ?: return null
        val bottom = raw.substring(at + 1).toFloatOrNull() ?: return null
        if (!x.isFinite() || !bottom.isFinite() || x < 0f || bottom < 0f) return null
        return CarSlot(x, bottom)
    }
}

/**
 * Adding or updating one pinned playlist, with the hard cap.
 *
 * The person's own catalogue can be large, and the SPA's pin action is reachable
 * from any page of the same origin: an unbounded list would let that page grow
 * `CarPrefs.pins` (a JSON string rewritten at every pin) without limit. 24 is
 * the ceiling — six pages of four tiles in the overlay, far past what a driver
 * scrolls; a pin that already exists can ALWAYS be refreshed in place, and a
 * new one past the ceiling is refused (the SPA shows "Impossible d'épingler").
 *
 * Pure on purpose: `CarPinsTest` holds the boundary on the JVM.
 */
object CarPins {
    const val MAX_PINS = 24

    /**
     * The list after adding [item], or null when it would exceed [MAX_PINS].
     * An existing id is replaced at its current rank (dedup, order kept).
     */
    fun add(
        current: List<PinnedPlaylist>,
        item: PinnedPlaylist,
    ): List<PinnedPlaylist>? {
        if (item.id.isBlank()) return null
        val at = current.indexOfFirst { it.id == item.id }
        if (at >= 0) {
            val next = current.toMutableList()
            next[at] = item
            return next
        }
        if (current.size >= MAX_PINS) return null
        return current + item
    }
}

/**
 * The car mode's own preferences, in the app's SharedPreferences convention
 * (no DataStore dependency, no server round-trip). Everything is on this
 * device: visibility, opacity, keyboard, triggers, the pinned order and the
 * per-navigation-app position.
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

    /** Pinned playlists, in display order. The Favoris tile is implicit, never stored. */
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

    /**
     * Pin (or refresh) one playlist. The ONE door for the cap: [CarPins.add]
     * refuses a new pin past 24 and always accepts an existing id. Returns
     * whether the list changed, so `carPin` can answer the SPA truthfully.
     */
    fun pin(item: PinnedPlaylist): Boolean {
        val next = CarPins.add(pins, item) ?: return false
        pins = next
        return true
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

    // -- position ------------------------------------------------------------

    /**
     * The saved place of the card for [pkg] in [size], or null when the person
     * never moved it there (the caller then applies [CarPlacement.defaults]).
     */
    fun position(pkg: String?, size: CarSize): CarSlot? {
        val raw = sp.getString(CarPositions.key(pkg, size), null) ?: return null
        return CarPositions.decode(raw)
    }

    /** Save one slot; the service listens to the file and re-applies it live. */
    fun setPosition(pkg: String?, size: CarSize, slot: CarSlot) {
        sp.edit()
            .putString(CarPositions.key(pkg, size), CarPositions.encode(slot))
            .apply()
    }

    /** Forget every size of one navigation app (null = the no-app slot). */
    fun clearPosition(pkg: String?) {
        val edit = sp.edit()
        for (key in sp.all.keys) {
            if (CarPositions.parse(key)?.first == pkg) edit.remove(key)
        }
        edit.apply()
    }

    /** Back to the prototype's place for every app and every size. */
    fun clearPositions() {
        val edit = sp.edit()
        for (key in sp.all.keys) {
            if (CarPositions.isPositionKey(key)) edit.remove(key)
        }
        edit.apply()
    }

    /** The navigation apps that have a saved position (the null bucket is reset by [clearPositions]). */
    fun positionedPackages(): Set<String> = sp.all.keys
        .asSequence()
        .mapNotNull { CarPositions.packageOf(it) }
        .filter { it.isNotBlank() }
        .toSet()

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
