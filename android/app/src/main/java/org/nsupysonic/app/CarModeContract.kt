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
     * Launch something the server already knows. [kind] is `flow`, `playlist`,
     * `album`, `artist`, `mix` or `track`; [payload] is the JSON of a track when
     * the page needs the whole object. The service turns this into the SPA's
     * existing `__nsCarPlay` hook; it does not invent a playback path.
     */
    fun launch(kind: String, id: String, title: String = "", payload: String? = null)

    fun openSettings()
    fun toast(message: String)
}
