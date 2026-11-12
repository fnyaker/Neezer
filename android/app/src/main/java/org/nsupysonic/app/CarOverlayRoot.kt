package org.nsupysonic.app

import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView

/**
 * The overlay's own view tree: the player, the pinned playlists and the search
 * keyboard, plus the one toast the car mode shows. It owns no playback and no
 * network: everything goes through [CarHost] (see CarModeContract.kt).
 *
 * The window itself belongs to CarModeService (type, position, alpha, blur).
 * This root only paints the rounded card and swaps the panel, so a size or a
 * screen change is a visibility change here, never a new window.
 *
 * Sizes: FULL and MINI show the player in its rounded card. ULTRA is the lone
 * 76 dp button the prototype puts at the bottom right, outside the card — the
 * card background is dropped and the player view's own END/BOTTOM gravity is
 * left to place it.
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
    private val toast = CarStyle.text(context, "", 18f, CarStyle.ONAC).apply {
        gravity = Gravity.CENTER
        setTypeface(CarType.body(context), Typeface.BOLD)
        setPadding(dp(18), dp(14), dp(18), dp(14))
        background = CarStyle.roundRect(context, CarStyle.AC, radiusDp = 18f)
        visibility = View.GONE
        isClickable = false
        isFocusable = false
        accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
        maxLines = 4
        ellipsize = android.text.TextUtils.TruncateAt.END
    }

    private val main = Handler(Looper.getMainLooper())
    private val hideToast = Runnable { toast.visibility = View.GONE }

    private var size = CarSize.FULL
    private var screen = CarScreen.PLAYER

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

        column.addView(
            toast,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT
            ).apply { bottomMargin = dp(10) }
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

        val inset = if (ultra) 0 else dp(8)
        setPadding(inset, 0, inset, 0)

        (column.layoutParams as? LayoutParams)?.let { lp ->
            lp.width = if (ultra) LayoutParams.WRAP_CONTENT else LayoutParams.MATCH_PARENT
            column.layoutParams = lp
        }
        (card.layoutParams as? LinearLayout.LayoutParams)?.let { lp ->
            lp.width = if (ultra) LinearLayout.LayoutParams.WRAP_CONTENT
            else LinearLayout.LayoutParams.MATCH_PARENT
            card.layoutParams = lp
        }
        // CarPlayerView measures itself 76 dp square in ULTRA and expects its
        // parent to place it; the prototype puts it 14 dp from the right and
        // 104 dp from the bottom, outside the rounded card.
        (player.layoutParams as? FrameLayout.LayoutParams)?.let { lp ->
            if (ultra) {
                lp.width = FrameLayout.LayoutParams.WRAP_CONTENT
                lp.height = FrameLayout.LayoutParams.WRAP_CONTENT
                lp.gravity = Gravity.END or Gravity.BOTTOM
                lp.marginEnd = dp(14)
                lp.bottomMargin = dp(104)
            } else {
                lp.width = FrameLayout.LayoutParams.MATCH_PARENT
                lp.height = FrameLayout.LayoutParams.WRAP_CONTENT
                lp.gravity = Gravity.TOP or Gravity.START
                lp.marginEnd = 0
                lp.bottomMargin = 0
            }
            player.layoutParams = lp
        }

        card.background = if (ultra) null else cardBackground()
        card.clipToOutline = !ultra
        applyVisibility()
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

    /** The prototype's large citron pill: 2.2 s, then it takes no more room. */
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
        toast.text = message
        toast.visibility = View.VISIBLE
        main.removeCallbacks(hideToast)
        main.postDelayed(hideToast, TOAST_MS)
    }

    fun hideToastNow() {
        main.removeCallbacks(hideToast)
        toast.visibility = View.GONE
    }

    /**
     * Re-read the card style after the settings screen changed the opacity or
     * the palette. The window alpha is CarModeService's (Android 12's touch
     * rule is checked on the window, not on a drawable), so only the card is
     * repainted here.
     */
    fun refreshStyle() {
        card.background = if (size == CarSize.ULTRA) null else cardBackground()
    }

    private fun applyVisibility() {
        val ultra = size == CarSize.ULTRA
        player.visibility = if (ultra || screen == CarScreen.PLAYER) View.VISIBLE else View.GONE
        playlists.visibility =
            if (!ultra && screen == CarScreen.PLAYLISTS) View.VISIBLE else View.GONE
        search.visibility = if (!ultra && screen == CarScreen.SEARCH) View.VISIBLE else View.GONE
    }

    private fun cardBackground(): GradientDrawable = CarStyle.roundRect(
        context,
        Color.rgb(14, 14, 12),
        Color.argb(46, 255, 255, 255),
        radiusDp = 26f,
        strokeDp = 1f
    )

    private fun dp(value: Int): Int = CarStyle.dp(context, value.toFloat())

    override fun onDetachedFromWindow() {
        main.removeCallbacks(hideToast)
        super.onDetachedFromWindow()
    }

    private companion object {
        const val TOAST_MS = 2_200L
    }
}
