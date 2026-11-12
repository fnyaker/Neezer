package org.nsupysonic.app

import android.content.Context
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import kotlin.math.roundToInt

/**
 * The overlay's own view tree: the player, the pinned playlists and the search
 * keyboard, plus the one toast the car mode shows. It owns no playback and no
 * network: everything goes through [CarHost] (see CarModeContract.kt).
 *
 * The card window itself belongs to CarModeService (type, position, size,
 * blur). This root paints the rounded card and swaps the panel, so a size or a
 * screen change is a visibility change here.
 *
 * Sizes: FULL and MINI show the player in its rounded card, which fills the
 * window exactly (the service gives it screen − 16 dp). ULTRA is the lone
 * 76 dp button the prototype puts at the bottom right; the service gives the
 * window exactly 76 × 76 dp, so there is no transparent band around the disc.
 *
 * The toast is NOT inside that window: the prototype floats it at top 206 dp,
 * and a strip added inside a bottom-anchored window would either move the card
 * or intercept the GPS app's touches. It gets its own non-touchable overlay
 * window instead.
 */
class CarOverlayRoot(
    context: Context,
    private val host: CarHost,
    private val carPrefs: CarPrefs,
) : FrameLayout(context) {

    private val player = CarPlayerView(context, host)
    private val playlists = CarPlaylistsView(context, host, carPrefs)
    private val search = CarSearchView(context, host, carPrefs)

    private val column = LinearLayout(context)
    private val card = FrameLayout(context)
    private val windowManager =
        context.getSystemService(Context.WINDOW_SERVICE) as WindowManager

    /**
     * The prototype's toast: citron pill, 18 sp bold, left/right 16 dp, top
     * 206 dp. It lives in its own non-touchable window so it can sit high on
     * the screen without widening the card window or eating GPS taps.
     */
    private val toast = CarStyle.text(context, "", 18f, CarStyle.ONAC).apply {
        gravity = Gravity.CENTER
        setTypeface(CarType.body(context), Typeface.BOLD)
        setPadding(dp(18), dp(14), dp(18), dp(14))
        background = CarStyle.roundRect(context, CarStyle.AC, radiusDp = 18f)
        isClickable = false
        isFocusable = false
        accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
        maxLines = 4
        ellipsize = android.text.TextUtils.TruncateAt.END
    }

    private val main = Handler(Looper.getMainLooper())
    private val hideToast = Runnable { hideToastNow() }

    private var size = CarSize.FULL
    private var screen = CarScreen.PLAYER
    private var cardOpacity = DEFAULT_CARD_OPACITY
    private var toastAttached = false

    init {
        setClipChildren(false)
        setClipToPadding(false)
        setBackgroundColor(Color.TRANSPARENT)

        column.orientation = LinearLayout.VERTICAL
        column.gravity = Gravity.BOTTOM
        column.clipChildren = false
        column.clipToPadding = false
        addView(
            column,
            LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT)
        )

        card.clipChildren = false
        card.clipToPadding = false
        column.addView(
            card,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT
            )
        )

        val full = FrameLayout.LayoutParams(
            FrameLayout.LayoutParams.MATCH_PARENT,
            FrameLayout.LayoutParams.WRAP_CONTENT
        )
        card.addView(player, full)
        card.addView(
            playlists,
            FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.WRAP_CONTENT
            )
        )
        card.addView(
            search,
            FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.WRAP_CONTENT
            )
        )

        // The three panels are siblings; only the one the screen names may be
        // laid out, so a GONE view never keeps its own height in the window.
        setSize(CarSize.FULL)
        setScreen(CarScreen.PLAYER)
    }

    /**
     * Forward one state snapshot. [now] is what the page last published (null
     * before the first publish); [driving] and [passenger] are the trigger's
     * view, which the search keyboard reads through CarHost.
     */
    fun render(now: CarNow?, driving: Boolean, speedKmh: Double, passenger: Boolean) {
        player.render(now)
        player.setDriving(driving)
        search.setDriving(driving)
    }

    fun setSize(size: CarSize) {
        this.size = size
        val ultra = size == CarSize.ULTRA
        player.setSize(size)

        // The window is already exactly the card (or the 76 dp disc), so the
        // content fills it: no side padding, no transparent band inside the
        // window that would swallow a touch meant for the navigation app.
        setPadding(0, 0, 0, 0)
        (column.layoutParams as? LayoutParams)?.let { lp ->
            lp.width = LayoutParams.MATCH_PARENT
            column.layoutParams = lp
        }
        (card.layoutParams as? LinearLayout.LayoutParams)?.let { lp ->
            lp.width = LinearLayout.LayoutParams.MATCH_PARENT
            card.layoutParams = lp
        }
        (player.layoutParams as? FrameLayout.LayoutParams)?.let { lp ->
            if (ultra) {
                // The service already places the window at 14/104 with a
                // 76 × 76 dp size; the disc fills it.
                lp.width = FrameLayout.LayoutParams.MATCH_PARENT
                lp.height = FrameLayout.LayoutParams.MATCH_PARENT
                lp.gravity = Gravity.CENTER
            } else {
                lp.width = FrameLayout.LayoutParams.MATCH_PARENT
                lp.height = FrameLayout.LayoutParams.WRAP_CONTENT
                lp.gravity = Gravity.TOP or Gravity.START
            }
            lp.marginEnd = 0
            lp.bottomMargin = 0
            lp.topMargin = 0
            player.layoutParams = lp
        }

        card.background = if (ultra) null else cardBackground()
        card.clipToOutline = !ultra
        applyVisibility()
    }

    /**
     * The person's 40–80 % setting fills the CARD, not the window: text, the
     * white levels and the lemon play button stay at 100 %, exactly like the
     * prototype's `rgba(14,14,12,glass)`. Clamped here too because the value
     * also comes from prefs changed on another screen.
     */
    fun setCardOpacity(percent: Int) {
        val next = percent.coerceIn(MIN_CARD_OPACITY, MAX_CARD_OPACITY)
        if (next == cardOpacity) return
        cardOpacity = next
        card.background = if (size == CarSize.ULTRA) null else cardBackground()
    }

    fun setScreen(screen: CarScreen) {
        val changed = this.screen != screen
        this.screen = screen
        when (screen) {
            CarScreen.PLAYLISTS -> playlists.refresh()
            CarScreen.SEARCH -> if (changed) search.reset()
            CarScreen.PLAYER -> Unit
        }
        applyVisibility()
    }

    /** The prototype's large citron pill: 2.2 s at top 206 dp, in its own window. */
    fun showToast(text: String) {
        val message = text.trim().take(200)
        if (message.isEmpty()) {
            hideToastNow()
            return
        }
        if (Looper.myLooper() != Looper.getMainLooper()) {
            post { showToast(message) }
            return
        }
        if (!isAttachedToWindow) {
            systemToast(message)
            return
        }
        toast.text = message
        if (toastAttached) {
            try {
                windowManager.updateViewLayout(toast, toastParams())
            } catch (_: Exception) {
                // The window manager dropped it (permission revoked): fall
                // back rather than leaving a toast that never appears.
                toastAttached = false
                systemToast(message)
            }
        } else {
            try {
                windowManager.addView(toast, toastParams())
                toastAttached = true
            } catch (_: Exception) {
                systemToast(message)
                return
            }
        }
        main.removeCallbacks(hideToast)
        main.postDelayed(hideToast, TOAST_MS)
    }

    fun hideToastNow() {
        main.removeCallbacks(hideToast)
        if (!toastAttached) return
        toastAttached = false
        try {
            windowManager.removeViewImmediate(toast)
        } catch (_: Exception) {
        }
    }

    /**
     * Re-read the card style after the settings screen changed the opacity or
     * the palette. The window itself stays at alpha 1: only the card's fill
     * carries the 40–80 % setting (CarModeService.buildParams explains why the
     * Android 12 touch cap does not apply to a window that handles its own
     * touches).
     */
    fun refreshStyle() {
        setCardOpacity((carPrefs.overlayAlpha * 100f).roundToInt())
        card.background = if (size == CarSize.ULTRA) null else cardBackground()
        if (toastAttached) {
            try {
                windowManager.updateViewLayout(toast, toastParams())
            } catch (_: Exception) {
                hideToastNow()
            }
        }
    }

    private fun applyVisibility() {
        val ultra = size == CarSize.ULTRA
        player.visibility = if (ultra || screen == CarScreen.PLAYER) View.VISIBLE else View.GONE
        playlists.visibility =
            if (!ultra && screen == CarScreen.PLAYLISTS) View.VISIBLE else View.GONE
        search.visibility = if (!ultra && screen == CarScreen.SEARCH) View.VISIBLE else View.GONE
    }

    private fun cardBackground(): GradientDrawable {
        // Direct mapping: 40–80 % is the card's own opacity, as the prototype's
        // rgba(14,14,12,glass). Rounding is the nearest 8-bit alpha.
        val alpha = (cardOpacity * 255f / 100f).roundToInt().coerceIn(0, 255)
        return CarStyle.roundRect(
            context,
            Color.argb(alpha, 14, 14, 12),
            Color.argb(46, 255, 255, 255),
            radiusDp = 26f,
            strokeDp = 1f
        )
    }

    /**
     * The toast window: 16 dp in from each side, top 206 dp, no touch at all.
     * NOT_TOUCHABLE is the point — a status pill must never eat a map tap.
     */
    private fun toastParams(): WindowManager.LayoutParams {
        val width = (screenWidthPx() - dp(32)).coerceAtLeast(dp(1))
        return WindowManager.LayoutParams(
            width,
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE or
                WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
            PixelFormat.TRANSLUCENT
        ).apply {
            gravity = Gravity.TOP or Gravity.START
            x = dp(16)
            y = dp(206)
            // This window passes touches through (NOT_TOUCHABLE), so Android
            // 12's untrusted-touch rule applies: above 0.8 the map under the
            // pill would stop receiving taps for 2.2 s. 0.8 is the highest
            // allowed value and keeps the citron pill readable.
            alpha = 0.8f
        }
    }

    private fun screenWidthPx(): Int {
        if (android.os.Build.VERSION.SDK_INT >= 30) {
            return windowManager.currentWindowMetrics.bounds.width()
        }
        val point = android.graphics.Point()
        @Suppress("DEPRECATION")
        windowManager.defaultDisplay.getRealSize(point)
        return point.x
    }

    private fun systemToast(message: String) {
        try {
            Toast.makeText(context, message, Toast.LENGTH_SHORT).show()
        } catch (_: Exception) {
        }
    }

    private fun dp(value: Int): Int = CarStyle.dp(context, value.toFloat())

    override fun onDetachedFromWindow() {
        hideToastNow()
        super.onDetachedFromWindow()
    }

    private companion object {
        const val TOAST_MS = 2_200L
        const val MIN_CARD_OPACITY = 40
        const val MAX_CARD_OPACITY = 80
        const val DEFAULT_CARD_OPACITY = 62
    }
}
