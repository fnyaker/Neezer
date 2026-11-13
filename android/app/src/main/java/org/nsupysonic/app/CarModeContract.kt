package org.nsupysonic.app

/** Which of the three car sizes is on screen (DESIGN §2 of the car prototype). */
enum class CarSize { ULTRA, MINI, FULL }

/** The secondary panels of the overlay. */
enum class CarScreen { PLAYER, PLAYLISTS, SEARCH }

/**
 * What the overlay needs to know about the current title. A plain copy of the
 * fields the page already publishes, so no car view depends on PlayerService.
 */
data class CarNow(
    val title: String = "",
    val artist: String = "",
    val album: String = "",
    val cover: String = "",
    val playing: Boolean = false,
    val position: Double = 0.0,
    val duration: Double = 0.0,
    /** null when the native side does not know (the page owns the favourites). */
    val favorite: Boolean? = null,
)

/**
 * Everything the overlay's visibility depends on, in one value the service
 * receives from [CarTrigger] on the main thread.
 *
 * [gpsPackage] is the configured navigation app currently in front (null when
 * none), so the service can restore that app's own position; [decided] says a
 * foreground answer exists — before it, a manual start must not be torn down
 * while the seed query is still running.
 */
data class DriveState(
    val visible: Boolean = false,
    val moving: Boolean = true,
    val speedKmh: Double = 0.0,
    val gpsPackage: String? = null,
    val gpsInFront: Boolean = false,
    val bluetoothSink: Boolean = false,
    val decided: Boolean = false,
)

/**
 * The ONE visibility rule, pure so the JVM tests can hold it.
 *
 * GPS comes FIRST: while the GPS trigger is usable (switched on and the usage
 * access granted), the overlay exists only when a configured navigation app is
 * actually in front. Bluetooth no longer keeps it over YouTube or the launcher
 * — that OR was why a connected car radio left the card everywhere.
 *
 * Two deliberate doors remain, both explicit in the settings:
 * - the GPS trigger switched OFF: Bluetooth alone decides (documented mode, the
 *   platform cannot tell a car radio from a headset);
 * - the GPS trigger ON but usage access missing: it can never see a foreground
 *   app, so Bluetooth is the only signal left. The settings screen keeps the
 *   access row visible, and [CarModeService.autoStartAllowed] refuses to
 *   auto-start a GPS-only mode in that state.
 *
 * `manual` is NOT part of this formula: a manual start shows the card once, the
 * rule then hides it as soon as no navigation app is in front.
 */
object CarVisibility {
    fun wanted(
        gpsTrigger: Boolean,
        bluetoothTrigger: Boolean,
        gpsUsable: Boolean,
        gpsInFront: Boolean,
        bluetoothSink: Boolean,
    ): Boolean = if (gpsTrigger && gpsUsable) {
        gpsInFront
    } else {
        bluetoothTrigger && bluetoothSink
    }

    /**
     * Attach exactly once: the overlay is wanted, nothing suppresses it (the
     * settings screen, a deliberate close) and it is not already attached.
     * ACTION_REFRESH goes through this, so returning from the car settings can
     * never stack a second window.
     */
    fun shouldAttach(
        visible: Boolean,
        suppressed: Boolean,
        dismissed: Boolean,
        attached: Boolean,
    ): Boolean = visible && !suppressed && !dismissed && !attached

    /**
     * The GPS answer is known and says no: the card goes, whatever `manual`
     * said. Before the first answer the card is left alone (a manual start is
     * waiting for the seed query).
     */
    fun shouldDetach(decided: Boolean, visible: Boolean, attached: Boolean): Boolean =
        decided && !visible && attached
}

/**
 * The only door between the car views and the player. The service implements
 * it with the existing PlayerService/MediaSession; the views never touch audio,
 * the WebView or the server themselves.
 */
interface CarHost {
    val prefs: CarPrefs

    fun now(): CarNow?
    fun isDriving(): Boolean
    fun speedKmh(): Double
    fun isPassenger(): Boolean
    fun setPassenger(value: Boolean)

    fun togglePlay()
    fun next()
    fun previous()
    fun seekTo(seconds: Double)

    fun toggleFavorite()

    fun setSize(size: CarSize)
    fun setScreen(screen: CarScreen)
    fun closeOverlay()

    /**
     * Live drag of the card window, in pixels since the last call. The service
     * clamps to the display's safe area (status bar, navigation bar, cutout) and
     * updates the window itself; [endOverlayMove] persists the result for the
     * current navigation app and size.
     */
    fun moveOverlayBy(dxPx: Int, dyPx: Int)
    fun endOverlayMove()

    /**
     * Launch something the server already knows. [kind] is `flow`, `playlist`,
     * `album`, `artist`, `mix` or `track`; [payload] is the JSON of a track when
     * the page needs the whole object. The service turns this into the SPA's
     * existing `__nsCarPlay` hook; it does not invent a playback path.
     */
    fun launch(kind: String, id: String, title: String = "", payload: String? = null)

    fun openSettings()
    fun toast(message: String)
}
