package org.nsupysonic.app

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Outline
import android.graphics.Paint
import android.graphics.Path
import android.graphics.Typeface
import android.graphics.drawable.InsetDrawable
import android.text.TextUtils
import android.view.Gravity
import android.view.HapticFeedbackConstants
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.ViewOutlineProvider
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import kotlin.math.abs
import kotlin.math.min
import kotlin.math.roundToInt

/**
 * The car overlay's player, in the prototype's three sizes (Main.dc.html,
 * lines 55-159 and the gesture block at 213-375):
 *
 *  - FULL: 72 dp sleeve, 26 sp Bricolage title, 17 sp artist, 48 dp reduce,
 *    heart 64, previous 72, 92 dp lemon play, next 72, Playlists / Recherche
 *    64 dp in two columns.
 *  - MINI: 48 dp sleeve, 18/14 sp, 56 dp play and expand.
 *  - ULTRA: one 76 dp round button (lemon 2 dp ring, sleeve, half-black scrim,
 *    play/pause), left/right to skip, up to FULL, long to MINI.
 *
 * Gestures are the prototype's own detector: long press at 550 ms, a swipe at
 * 40 dp or more, a tap under 14 dp, otherwise nothing. A cancelled touch, a
 * second finger, the view being hidden and a size change all drop the pending
 * long press; a tap goes through performClick so TalkBack and the click sound
 * keep working. [CarPrefs.invertSwipe] swaps the ULTRA's left/right actions.
 *
 * Everything the view knows comes from [render]; every action leaves through
 * [CarHost]. It owns no playback state, no audio and no network.
 *
 * Placement: add it with MATCH_PARENT width and WRAP_CONTENT height for
 * FULL/MINI. In ULTRA it measures itself 76 x 76 dp, so the overlay should put
 * it at gravity END|BOTTOM (the prototype's right 14 / bottom 104) and let the
 * window shrink to it: a transparent full-width strip would eat the GPS app's
 * touches. Call the public methods on the main thread.
 */
@SuppressLint("ClickableViewAccessibility")
class CarPlayerView(context: Context, private val host: CarHost) : FrameLayout(context) {

    private companion object {
        /** The prototype's `setTimeout(..., 550)`. */
        const val LONG_PRESS_MS = 550L
        /** A horizontal (or upward) drag of this many dp is a swipe. */
        const val SWIPE_DP = 40f
        /** Below this many dp in both axes, the release is a tap. */
        const val TAP_DP = 14f
        const val ULTRA_DP = 76
        /** The visual floor for the five FULL controls when the row is tight. */
        const val MIN_TARGET_DP = 56
        const val PRESS_ALPHA = 0.86f
        const val DEFAULT_ULTRA_DESCRIPTION =
            "Sillon : toucher pour lire, glisser à gauche pour le titre précédent, " +
                "à droite pour le titre suivant, vers le haut pour développer, " +
                "appui long pour réduire en barre."
    }

    private var size: CarSize = CarSize.FULL
    private var driving = false
    private var ready = false

    // The last values drawn, so a 250 ms position tick (the state carries a
    // position we do not show) does not restyle the whole overlay four times
    // a second.
    private var shownTitle: String? = "Aucun titre"
    private var shownArtist: String? = ""
    private var shownSeed: String? = ""
    private var shownPlaying = false
    private var shownFavorite = false
    private var shownUltraDescription = DEFAULT_ULTRA_DESCRIPTION

    private val gestures = mutableListOf<Gesture>()

    // ---------------------------------------------------------------- MINI

    private val miniCover = CarCoverView(context).apply {
        sizeDp = 48
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
    }

    private val miniTitle = label("Aucun titre", 18f, CarStyle.TX, bold = true)

    private val miniArtist = label("", 14f, CarStyle.DIM)

    private val miniPlayIcon = icon("play", 22, CarStyle.ONAC)

    private val miniPlay = iconButton(
        sizeDp = 56,
        fill = CarStyle.AC,
        stroke = Color.TRANSPARENT,
        strokeDp = 0f,
        icon = miniPlayIcon,
        description = "Lire",
        onClick = { host.togglePlay(); confirm() },
    )

    private val miniExpand = iconButton(
        sizeDp = 56,
        fill = Color.TRANSPARENT,
        stroke = veil(CarStyle.TX, 0.35f),
        strokeDp = 1.5f,
        icon = icon("chevronUp", 22, CarStyle.TX),
        description = "Développer (appui long : bouton seul)",
    )

    private val miniPanel = LinearLayout(context).apply {
        orientation = HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        setPadding(dp(12f), dp(10f), dp(12f), dp(10f))

        val textColumn = LinearLayout(context).apply {
            orientation = VERTICAL
            gravity = Gravity.CENTER_VERTICAL
        }
        textColumn.addView(miniTitle, LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT))
        textColumn.addView(
            miniArtist,
            LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT).apply { topMargin = dp(1f) },
        )

        addView(miniCover, LinearLayout.LayoutParams(dp(48f), dp(48f)))
        addView(
            textColumn,
            LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f).apply {
                marginStart = dp(12f)
                marginEnd = dp(12f)
            },
        )
        addView(miniPlay, LinearLayout.LayoutParams(dp(56f), dp(56f)))
        addView(
            miniExpand,
            LinearLayout.LayoutParams(dp(56f), dp(56f)).apply { marginStart = dp(12f) },
        )
    }

    // ---------------------------------------------------------------- FULL

    private val fullCover = CarCoverView(context).apply {
        sizeDp = 72
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
    }

    private val fullTitle = label("Aucun titre", 26f, CarStyle.TX, display = true, bold = true).apply {
        letterSpacing = -0.02f
    }

    private val fullArtist = label("", 17f, CarStyle.DIM)

    private val fullReduce = iconButton(
        sizeDp = 48,
        fill = veil(CarStyle.TX, 0.10f),
        stroke = Color.TRANSPARENT,
        strokeDp = 0f,
        icon = icon("chevronDown", 22, CarStyle.TX),
        description = "Réduire (appui long : bouton seul)",
    )

    private val fullHeader = LinearLayout(context).apply {
        orientation = HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL

        val textColumn = LinearLayout(context).apply {
            orientation = VERTICAL
            gravity = Gravity.CENTER_VERTICAL
        }
        textColumn.addView(fullTitle, LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT))
        textColumn.addView(
            fullArtist,
            LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT).apply { topMargin = dp(2f) },
        )

        addView(fullCover, LinearLayout.LayoutParams(dp(72f), dp(72f)))
        addView(
            textColumn,
            LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f).apply {
                marginStart = dp(14f)
                marginEnd = dp(14f)
            },
        )
        addView(fullReduce, LinearLayout.LayoutParams(dp(48f), dp(48f)))
    }

    private val heartIcon = icon("heart", 30, CarStyle.TX)

    private val fullHeart = iconButton(
        sizeDp = 64,
        fill = Color.TRANSPARENT,
        stroke = Color.TRANSPARENT,
        strokeDp = 0f,
        icon = heartIcon,
        description = "Ajouter aux favoris",
        onClick = { host.toggleFavorite(); confirm() },
    )

    private val fullPrev = iconButton(
        sizeDp = 72,
        fill = veil(CarStyle.TX, 0.12f),
        stroke = Color.TRANSPARENT,
        strokeDp = 0f,
        icon = SkipGlyph(context, false, 30),
        description = "Titre précédent",
        onClick = { host.previous(); confirm() },
    )

    private val fullPlayIcon = icon("play", 36, CarStyle.ONAC)

    private val fullPlay = iconButton(
        sizeDp = 92,
        fill = CarStyle.AC,
        stroke = Color.TRANSPARENT,
        strokeDp = 0f,
        icon = fullPlayIcon,
        description = "Lire",
        onClick = { host.togglePlay(); confirm() },
    )

    private val fullNext = iconButton(
        sizeDp = 72,
        fill = veil(CarStyle.TX, 0.12f),
        stroke = Color.TRANSPARENT,
        strokeDp = 0f,
        icon = SkipGlyph(context, true, 30),
        description = "Titre suivant",
        onClick = { host.next(); confirm() },
    )

    // The prototype balances the heart with an empty 64 px span so the play
    // button stays centred.
    private val fullSpacer = View(context).apply {
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
    }

    private val fullControls = SpaceBetweenRow(context, MIN_TARGET_DP).apply {
        setPadding(dp(6f), 0, dp(6f), 0)
        addView(fullHeart, ViewGroup.LayoutParams(dp(64f), dp(64f)))
        addView(fullPrev, ViewGroup.LayoutParams(dp(72f), dp(72f)))
        addView(fullPlay, ViewGroup.LayoutParams(dp(92f), dp(92f)))
        addView(fullNext, ViewGroup.LayoutParams(dp(72f), dp(72f)))
        addView(fullSpacer, ViewGroup.LayoutParams(dp(64f), dp(64f)))
    }

    private val playlistsButton = wideButton(
        fill = veil(CarStyle.TX, 0.08f),
        stroke = veil(CarStyle.TX, 0.35f),
        contentColor = CarStyle.TX,
        iconName = "playlist",
        title = "Playlists",
        onClick = { host.setScreen(CarScreen.PLAYLISTS) },
    )

    private val searchButton = wideButton(
        fill = CarStyle.AC,
        stroke = Color.TRANSPARENT,
        contentColor = CarStyle.ONAC,
        iconName = "search",
        title = "Recherche",
        onClick = { host.setScreen(CarScreen.SEARCH) },
    )

    private val fullScreens = LinearLayout(context).apply {
        orientation = HORIZONTAL
        addView(playlistsButton, LinearLayout.LayoutParams(0, dp(64f), 1f))
        addView(
            searchButton,
            LinearLayout.LayoutParams(0, dp(64f), 1f).apply { marginStart = dp(10f) },
        )
    }

    private val fullPanel = LinearLayout(context).apply {
        orientation = VERTICAL
        setPadding(dp(14f), dp(14f), dp(14f), dp(16f))
        addView(fullHeader, LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT))
        addView(
            fullControls,
            LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT).apply { topMargin = dp(14f) },
        )
        addView(
            fullScreens,
            LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT).apply { topMargin = dp(14f) },
        )
    }

    // --------------------------------------------------------------- ULTRA

    private val ultraCover = CarCoverView(context).apply {
        sizeDp = ULTRA_DP
        round = true
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
    }

    private val ultraScrim = View(context).apply {
        setBackgroundColor(veil(CarStyle.BG, 0.50f))
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
    }

    private val ultraIcon = icon("play", 30, CarStyle.TX)

    private val ultraBorder = View(context).apply {
        // A 2 dp ring, inset by 1 dp so the stroke is not clipped by the
        // circular outline: the prototype's 2 px #D2FF3C border.
        background = InsetDrawable(
            CarStyle.roundRect(context, Color.TRANSPARENT, CarStyle.AC, 999f, 2f),
            dp(1f),
        )
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
    }

    private val ultraPanel = FrameLayout(context).apply {
        isClickable = true
        isFocusable = true
        contentDescription = DEFAULT_ULTRA_DESCRIPTION
        outlineProvider = object : ViewOutlineProvider() {
            override fun getOutline(view: View, outline: Outline) {
                outline.setOval(0, 0, view.width, view.height)
            }
        }
        clipToOutline = true
        elevation = dp(4f).toFloat()

        addView(ultraCover, FrameLayout.LayoutParams(dp(ULTRA_DP.toFloat()), dp(ULTRA_DP.toFloat())))
        addView(ultraScrim, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))
        addView(
            ultraIcon,
            FrameLayout.LayoutParams(WRAP_CONTENT, WRAP_CONTENT, Gravity.CENTER),
        )
        addView(ultraBorder, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))
    }

    init {
        addView(miniPanel, FrameLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT))
        addView(fullPanel, FrameLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT))
        addView(
            ultraPanel,
            FrameLayout.LayoutParams(
                dp(ULTRA_DP.toFloat()),
                dp(ULTRA_DP.toFloat()),
                Gravity.END or Gravity.BOTTOM,
            ),
        )

        miniPanel.visibility = GONE
        fullPanel.visibility = VISIBLE
        ultraPanel.visibility = GONE

        attachGesture(
            miniExpand,
            tap = { openFull() },
            long = { requestSize(CarSize.ULTRA) },
        )
        attachGesture(
            fullReduce,
            tap = { requestSize(CarSize.MINI) },
            long = { requestSize(CarSize.ULTRA) },
        )
        attachGesture(
            ultraPanel,
            tap = { host.togglePlay(); confirm() },
            long = { requestSize(CarSize.MINI) },
            left = { swipeLeft() },
            right = { swipeRight() },
            up = { openFull() },
        )

        ready = true
    }

    // ------------------------------------------------------------ contract

    /**
     * Draws [now], or the empty state when the service has nothing yet. Only
     * the fields the car shows are touched: the position is deliberately not
     * drawn (the prototype has no progress bar), so ticks cost a comparison.
     */
    fun render(now: CarNow?) {
        val n = now ?: CarNow()

        val title = n.title.ifBlank { "Aucun titre" }
        if (title != shownTitle) {
            shownTitle = title
            miniTitle.text = title
            fullTitle.text = title
        }
        if (n.artist != shownArtist) {
            shownArtist = n.artist
            miniArtist.text = n.artist
            fullArtist.text = n.artist
        }

        // CarCoverView draws Sillon's grooves, not a CDN image. The seed is the
        // track's identity (album/title/artist) rather than the cover URL, so
        // MINI, FULL and ULTRA show the same sleeve even if the URL carries a
        // size.
        val seed = listOf(n.album, n.title, n.artist)
            .filter { it.isNotBlank() }
            .joinToString("\u0000")
        if (seed != shownSeed) {
            shownSeed = seed
            miniCover.seed = seed
            fullCover.seed = seed
            ultraCover.seed = seed
        }

        if (n.playing != shownPlaying) {
            shownPlaying = n.playing
            val glyph = if (n.playing) "pause" else "play"
            miniPlayIcon.name = glyph
            fullPlayIcon.name = glyph
            ultraIcon.name = glyph
            val description = if (n.playing) "Mettre en pause" else "Lire"
            miniPlay.contentDescription = description
            fullPlay.contentDescription = description
        }

        val favorite = n.favorite == true
        if (favorite != shownFavorite) {
            shownFavorite = favorite
            heartIcon.filled = favorite
            heartIcon.color = if (favorite) CarStyle.AC else CarStyle.TX
            fullHeart.isSelected = favorite
            fullHeart.contentDescription =
                if (favorite) "Retirer des favoris" else "Ajouter aux favoris"
        }

        updateUltraDescription()
    }

    /** Shows one of the three sizes. The parent re-measures; ULTRA is 76 dp square. */
    fun setSize(value: CarSize) {
        if (size == value) return
        size = value
        cancelGestures()
        miniPanel.visibility = if (value == CarSize.MINI) VISIBLE else GONE
        fullPanel.visibility = if (value == CarSize.FULL) VISIBLE else GONE
        ultraPanel.visibility = if (value == CarSize.ULTRA) VISIBLE else GONE
        requestLayout()
        invalidate()
    }

    fun currentSize(): CarSize = size

    /**
     * The car is moving. Gestures in flight are dropped (a speed change in the
     * middle of a long press is not a command), and transport actions get a
     * light haptic tick so the driver learns the tap registered without
     * looking — the prototype's screens stay exactly as they are.
     */
    fun setDriving(value: Boolean) {
        if (driving == value) return
        driving = value
        cancelGestures()
    }

    // --------------------------------------------------------- measure/layout

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        if (size == CarSize.ULTRA) {
            // Ignore a MATCH_PARENT spec: the round button is 76 dp, not the
            // width of the overlay. The parent positions it (END|BOTTOM).
            val side = dp(ULTRA_DP.toFloat())
            ultraPanel.measure(
                MeasureSpec.makeMeasureSpec(side, MeasureSpec.EXACTLY),
                MeasureSpec.makeMeasureSpec(side, MeasureSpec.EXACTLY),
            )
            setMeasuredDimension(side, side)
            return
        }
        super.onMeasure(widthMeasureSpec, heightMeasureSpec)
    }

    // -------------------------------------------------------------- gestures

    private fun openFull() {
        host.setScreen(CarScreen.PLAYER)
        requestSize(CarSize.FULL)
    }

    private fun requestSize(value: CarSize) {
        // Apply locally first so the overlay answers within the frame, then
        // tell the service; a service that echoes setSize() back is a no-op.
        setSize(value)
        host.setSize(value)
    }

    private fun swipeLeft() {
        if (host.prefs.invertSwipe) skip(forward = true) else skip(forward = false)
    }

    private fun swipeRight() {
        if (host.prefs.invertSwipe) skip(forward = false) else skip(forward = true)
    }

    private fun skip(forward: Boolean) {
        confirm()
        if (forward) host.next() else host.previous()
    }

    private fun confirm() {
        if (driving) performHapticFeedback(HapticFeedbackConstants.KEYBOARD_TAP)
    }

    private fun attachGesture(
        view: View,
        tap: (() -> Unit)? = null,
        long: (() -> Unit)? = null,
        left: (() -> Unit)? = null,
        right: (() -> Unit)? = null,
        up: (() -> Unit)? = null,
    ) {
        tap?.let { action -> view.setOnClickListener { action() } }
        long?.let { action ->
            view.setOnLongClickListener {
                // TalkBack's double-tap-and-hold reaches this path; a physical
                // long press goes through the Gesture timer below, which calls
                // performLongClick().
                it.performHapticFeedback(HapticFeedbackConstants.LONG_PRESS)
                action()
                true
            }
        }
        val gesture = Gesture(view, long != null, left, right, up)
        gestures += gesture
        view.setOnTouchListener(gesture)
    }

    private fun cancelGestures() {
        for (gesture in gestures) gesture.cancel()
        // A tap-only button hidden in the middle of a press may never get its
        // ACTION_UP; do not leave it faded.
        miniPlay.alpha = 1f
        fullHeart.alpha = 1f
        fullPrev.alpha = 1f
        fullPlay.alpha = 1f
        fullNext.alpha = 1f
        playlistsButton.alpha = 1f
        searchButton.alpha = 1f
    }

    override fun onVisibilityChanged(changedView: View, visibility: Int) {
        super.onVisibilityChanged(changedView, visibility)
        if (ready && visibility != VISIBLE) cancelGestures()
    }

    override fun onWindowVisibilityChanged(visibility: Int) {
        super.onWindowVisibilityChanged(visibility)
        if (ready && visibility != VISIBLE) cancelGestures()
    }

    override fun onDetachedFromWindow() {
        super.onDetachedFromWindow()
        if (ready) cancelGestures()
    }

    /**
     * The prototype's detector, with Android's lifecycle folded in: one action
     * at most per touch, timers removed on UP/CANCEL/hide/detach, and a tap
     * routed through performClick for accessibility and the click sound.
     */
    private inner class Gesture(
        private val target: View,
        private val hasLong: Boolean,
        private val left: (() -> Unit)?,
        private val right: (() -> Unit)?,
        private val up: (() -> Unit)?,
    ) : View.OnTouchListener {
        private var downX = 0f
        private var downY = 0f
        private var touchActive = false
        private var tracking = false
        private var fired = false

        private val longPress = Runnable {
            if (tracking && !fired) {
                fired = true
                target.performLongClick()
            }
        }

        override fun onTouch(v: View, event: MotionEvent): Boolean {
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    touchActive = true
                    tracking = true
                    fired = false
                    downX = event.x
                    downY = event.y
                    target.alpha = PRESS_ALPHA
                    target.removeCallbacks(longPress)
                    if (hasLong) target.postDelayed(longPress, LONG_PRESS_MS)
                    return true
                }

                MotionEvent.ACTION_POINTER_DOWN -> {
                    if (!touchActive) return false
                    // A second finger is not the one-finger gesture the
                    // prototype defines; drop it rather than guess. The rest
                    // of the stream is consumed so no click leaks out.
                    cancel()
                    return true
                }

                MotionEvent.ACTION_MOVE -> {
                    if (!touchActive) return false
                    if (event.pointerCount > 1) cancel()
                    // The prototype does not cancel the long press when the
                    // finger moves, so neither do we; fired decides the rest.
                    return true
                }

                MotionEvent.ACTION_UP -> {
                    if (!touchActive) return false
                    touchActive = false
                    target.removeCallbacks(longPress)
                    target.alpha = 1f
                    if (!tracking) {
                        fired = false
                        return true
                    }
                    tracking = false
                    if (fired) {
                        fired = false
                        return true
                    }
                    val dx = event.x - downX
                    val dy = event.y - downY
                    val swipe = dp(SWIPE_DP).toFloat()
                    val tap = dp(TAP_DP).toFloat()
                    when {
                        abs(dx) >= swipe && abs(dx) > abs(dy) ->
                            if (dx < 0) left?.invoke() else right?.invoke()

                        dy <= -swipe -> up?.invoke()
                        abs(dx) < tap && abs(dy) < tap -> target.performClick()
                    }
                    return true
                }

                MotionEvent.ACTION_CANCEL -> {
                    touchActive = false
                    cancel()
                    return true
                }
            }
            return false
        }

        fun cancel() {
            tracking = false
            fired = false
            target.removeCallbacks(longPress)
            target.alpha = 1f
        }
    }

    // -------------------------------------------------------------- factories

    private fun label(
        value: String,
        sizeSp: Float,
        color: Int,
        display: Boolean = false,
        bold: Boolean = false,
    ): TextView = CarStyle.text(context, value, sizeSp, color, display).apply {
        maxLines = 1
        ellipsize = TextUtils.TruncateAt.END
        if (bold) typeface = Typeface.create(typeface, Typeface.BOLD)
    }

    private fun icon(name: String, sizeDp: Int, color: Int): CarIconView =
        CarIconView(context).apply {
            this.name = name
            this.sizeDp = sizeDp
            this.color = color
            importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
        }

    private fun iconButton(
        sizeDp: Int,
        fill: Int,
        stroke: Int,
        strokeDp: Float,
        icon: View,
        description: String,
        onClick: (() -> Unit)? = null,
    ): FrameLayout = FrameLayout(context).apply {
        background = CarStyle.roundRect(context, fill, stroke, sizeDp / 2f, strokeDp)
        minimumWidth = dp(sizeDp.toFloat())
        minimumHeight = dp(sizeDp.toFloat())
        isClickable = true
        isFocusable = true
        contentDescription = description
        addView(icon, FrameLayout.LayoutParams(WRAP_CONTENT, WRAP_CONTENT, Gravity.CENTER))
        onClick?.let { action ->
            setOnClickListener { action() }
            pressFeedback(this)
        }
    }

    private fun wideButton(
        fill: Int,
        stroke: Int,
        contentColor: Int,
        iconName: String,
        title: String,
        onClick: () -> Unit,
    ): FrameLayout {
        val button = FrameLayout(context).apply {
            background = CarStyle.roundRect(context, fill, stroke, 20f, 1.5f)
            minimumHeight = dp(64f)
            isClickable = true
            isFocusable = true
            contentDescription = title
            setOnClickListener { onClick() }
        }
        val row = LinearLayout(context).apply {
            orientation = HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
        }
        val glyph = icon(iconName, 24, contentColor)
        val text = label(title, 19f, contentColor, bold = true)
        row.addView(
            glyph,
            LinearLayout.LayoutParams(WRAP_CONTENT, WRAP_CONTENT).apply { marginEnd = dp(10f) },
        )
        row.addView(text, LinearLayout.LayoutParams(WRAP_CONTENT, WRAP_CONTENT))
        button.addView(row, FrameLayout.LayoutParams(WRAP_CONTENT, WRAP_CONTENT, Gravity.CENTER))
        pressFeedback(button)
        return button
    }

    /**
     * The prototype's press state is not drawn in the mockup; Android needs
     * one. The listener returns false so the normal click (sound, TalkBack)
     * still runs.
     */
    private fun pressFeedback(view: View) {
        view.setOnTouchListener { target, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> target.alpha = PRESS_ALPHA
                MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> target.alpha = 1f
            }
            false
        }
    }

    private fun dp(value: Float): Int = CarStyle.dp(context, value)

    private fun veil(color: Int, alpha: Float): Int {
        val a = (alpha.coerceIn(0f, 1f) * 255f + 0.5f).toInt()
        return Color.argb(a, Color.red(color), Color.green(color), Color.blue(color))
    }

    private fun updateUltraDescription() {
        val action = if (shownPlaying) "mettre en pause" else "lire"
        val inverted = host.prefs.invertSwipe
        val leftTitle = if (inverted) "le titre suivant" else "le titre précédent"
        val rightTitle = if (inverted) "le titre précédent" else "le titre suivant"
        val description =
            "Sillon : toucher pour $action, glisser à gauche pour $leftTitle, " +
                "à droite pour $rightTitle, vers le haut pour développer, " +
                "appui long pour réduire en barre."
        if (description != shownUltraDescription) {
            shownUltraDescription = description
            ultraPanel.contentDescription = description
        }
    }

    /**
     * A horizontal row of fixed-size controls that behaves like the prototype's
     * flex row: space-between while the controls fit, proportional shrink when
     * they do not, and a 56 dp floor unless the overlay is narrower than
     * five targets (then it scales evenly — there is no room left to protect).
     */
    private class SpaceBetweenRow(
        context: Context,
        private val minTargetDp: Int,
    ) : ViewGroup(context) {

        override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
            val children = visibleChildren()
            if (children.isEmpty()) {
                setMeasuredDimension(
                    resolveSize(paddingLeft + paddingRight, widthMeasureSpec),
                    resolveSize(paddingTop + paddingBottom, heightMeasureSpec),
                )
                return
            }

            val desired = IntArray(children.size) { index ->
                children[index].layoutParams.width.coerceAtLeast(0)
            }
            val available = when (MeasureSpec.getMode(widthMeasureSpec)) {
                MeasureSpec.UNSPECIFIED -> desired.sum()
                else -> (MeasureSpec.getSize(widthMeasureSpec) - paddingLeft - paddingRight)
                    .coerceAtLeast(0)
            }
            val widths = fit(desired, available)

            var rowHeight = 0
            for (index in children.indices) {
                val height = children[index].layoutParams.height.coerceAtLeast(0)
                children[index].measure(
                    MeasureSpec.makeMeasureSpec(
                        widths[index].roundToInt().coerceAtLeast(0),
                        MeasureSpec.EXACTLY,
                    ),
                    MeasureSpec.makeMeasureSpec(height, MeasureSpec.EXACTLY),
                )
                rowHeight = maxOf(rowHeight, height)
            }

            setMeasuredDimension(
                resolveSize(available + paddingLeft + paddingRight, widthMeasureSpec),
                resolveSize(rowHeight + paddingTop + paddingBottom, heightMeasureSpec),
            )
        }

        override fun onLayout(changed: Boolean, l: Int, t: Int, r: Int, b: Int) {
            val children = visibleChildren()
            if (children.isEmpty()) return
            var total = 0f
            for (child in children) total += child.measuredWidth
            val gap = if (children.size > 1) {
                ((width - paddingLeft - paddingRight - total) / (children.size - 1))
                    .coerceAtLeast(0f)
            } else {
                0f
            }
            val innerHeight = height - paddingTop - paddingBottom
            var x = paddingLeft.toFloat()
            for (child in children) {
                val childHeight = child.measuredHeight
                val top = paddingTop + (innerHeight - childHeight) / 2
                val left = x.roundToInt()
                child.layout(left, top, left + child.measuredWidth, top + childHeight)
                x += child.measuredWidth + gap
            }
        }

        override fun generateDefaultLayoutParams(): ViewGroup.LayoutParams =
            ViewGroup.LayoutParams(WRAP_CONTENT, WRAP_CONTENT)

        private fun visibleChildren(): List<View> {
            val out = ArrayList<View>(childCount)
            for (index in 0 until childCount) {
                val child = getChildAt(index)
                if (child.visibility != GONE) out.add(child)
            }
            return out
        }

        private fun fit(desired: IntArray, available: Int): FloatArray {
            val widths = FloatArray(desired.size) { desired[it].toFloat() }
            val total = desired.sum()
            if (total <= available || total == 0) return widths

            val minTargetPx = CarStyle.dp(context, minTargetDp.toFloat())
            if (available < desired.size * minTargetPx) {
                val scale = available.toFloat() / total.toFloat()
                for (index in desired.indices) widths[index] = desired[index].toFloat() * scale
                return widths
            }
            val minTarget = minTargetPx.toFloat()

            val locked = BooleanArray(desired.size)
            var free = available.toFloat()
            var remainingDesired = total.toFloat()
            var unlocked = desired.size
            var changed = true

            while (changed && unlocked > 0) {
                changed = false
                if (remainingDesired <= 0f) break
                val scale = free / remainingDesired
                for (index in desired.indices) {
                    if (locked[index]) continue
                    val candidate = desired[index].toFloat() * scale
                    if (candidate < minTarget) {
                        widths[index] = minTarget
                        locked[index] = true
                        free -= minTarget
                        remainingDesired -= desired[index].toFloat()
                        unlocked--
                        changed = true
                    }
                }
                if (!changed) {
                    for (index in desired.indices) {
                        if (!locked[index]) widths[index] = desired[index].toFloat() * scale
                    }
                }
            }
            return widths
        }
    }

    /**
     * CarIconView carries the play/pause, heart, chevrons, search and playlist
     * glyphs but not the skip ones, and this file owns no other icon. These are
     * the prototype's own two filled paths (M6 5h2v14H6zM20 5v14L9 12z and
     * M16 5h2v14h-2zM4 5v14l11-7z), drawn in the current palette.
     */
    private class SkipGlyph(
        context: Context,
        private val forward: Boolean,
        private val sizeDp: Int,
    ) : View(context) {
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.FILL
        }
        private val path = Path()

        init {
            importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
        }

        override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
            val side = CarStyle.dp(context, sizeDp.toFloat())
            setMeasuredDimension(side, side)
        }

        override fun onDraw(canvas: Canvas) {
            val side = min(width, height).toFloat()
            if (side <= 0f) return
            canvas.save()
            canvas.scale(side / 24f, side / 24f)
            paint.color = CarStyle.TX
            path.reset()
            if (forward) {
                path.moveTo(16f, 5f)
                path.lineTo(18f, 5f)
                path.lineTo(18f, 19f)
                path.lineTo(16f, 19f)
                path.close()
                path.moveTo(4f, 5f)
                path.lineTo(15f, 12f)
                path.lineTo(4f, 19f)
                path.close()
            } else {
                path.moveTo(6f, 5f)
                path.lineTo(8f, 5f)
                path.lineTo(8f, 19f)
                path.lineTo(6f, 19f)
                path.close()
                path.moveTo(20f, 5f)
                path.lineTo(9f, 12f)
                path.lineTo(20f, 19f)
                path.close()
            }
            canvas.drawPath(path, paint)
            canvas.restore()
        }
    }
}
