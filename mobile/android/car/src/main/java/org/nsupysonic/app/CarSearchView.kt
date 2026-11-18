package org.nsupysonic.app

import android.animation.ObjectAnimator
import android.animation.ValueAnimator
import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.os.Handler
import android.os.Looper
import android.text.SpannableString
import android.text.Spanned
import android.text.TextUtils
import android.text.style.ReplacementSpan
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.inputmethod.InputMethodManager
import android.view.inputmethod.InputMethodSubtype
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import androidx.core.graphics.ColorUtils
import fr.neezer.mobile.bridge.NzErrors
import java.text.Normalizer
import java.util.Locale
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

/**
 * The car's search panel: a 52 dp field, a drawn keyboard, three results and a
 * French voice button. There is deliberately NO EditText and NO system IME: the
 * overlay window is not focusable over the navigation app, and a soft keyboard
 * covering the map while driving is the wrong answer.
 *
 * States:
 *  - stopped, or passenger: free typing, space enabled, no letter limit;
 *  - driving (setDriving(true) from the speed trigger, passenger false): the
 *    query keeps at most CarPrefs.maxLetters characters and the NEXT letter
 *    starts a new query; the space key is disabled. This is the prototype's
 *    rule, not a truncation of what was already typed.
 *
 * Search is ONE read of the core's own search (`Screens.search`, MB6: one request
 * on the `User` lane, counted under `search`), debounced 700 ms and sent only
 * from two characters — the route fans out to five upstream searches, so a
 * one-letter query is work without an answer. A new keystroke CANCELS the call
 * in flight (the UniFFI future is dropped, Rust's request with it), invalidates
 * the answer of the previous generation, and only the last three hits are drawn.
 * Tapping a row calls the only playback door, [CarHost.launch]; this view never
 * touches audio.
 *
 * Auto layout reads the current InputMethodManager subtype (`fr` -> AZERTY,
 * `de` -> QWERTZ, otherwise QWERTY; fr_CA is QWERTY and fr_CH QWERTZ because
 * those keyboards really are). A fixed layout in CarPrefs always wins.
 */
class CarSearchView(
    context: Context,
    private val host: CarHost,
    private val prefs: CarPrefs = host.prefs,
) : LinearLayout(context) {
    // A 640 dp car screen with the 92 dp bottom margin cannot fit 50 dp keys
    // in the ABC + passenger layout; 42 dp rows still leave a 44 dp target
    // after the touch slop, and keep the whole panel on screen.
    private val compactScreen = context.resources.displayMetrics.run {
        heightPixels / density < 700f
    }
    private val keyHeightDp = if (compactScreen) 42 else 50
    private val suggestionHeightDp = if (compactScreen) 42 else 46
    private val handler = Handler(Looper.getMainLooper())
    // Created on the first search and cancelled with the view: the service builds
    // a new root for every attach, and two attaches must not leave two live
    // scopes (nor a request nobody reads).
    private var searchScope: CoroutineScope? = null

    private var driving = host.isDriving()
    private var query = ""
    private var voiceListening = false
    private var pulse: ObjectAnimator? = null
    private var spaceKey: TextView? = null

    @Volatile private var searchGeneration = 0
    // The call in flight, so the next keystroke or a detach cancels it instead
    // of letting it answer into the void.
    @Volatile private var activeJob: Job? = null
    private var searchRunnable: Runnable? = null

    private lateinit var queryView: TextView
    private lateinit var micButton: FrameLayout
    private lateinit var micIcon: CarIconView
    private lateinit var closeButton: FrameLayout
    private lateinit var modeChip: TextView
    private lateinit var passengerButton: TextView
    private lateinit var suggestions: LinearLayout
    private lateinit var keyboard: LinearLayout

    private val speech = CarSpeech(context.applicationContext) { text -> onSpeechText(text) }

    private val chipTick = object : Runnable {
        override fun run() {
            if (!isShown || !driving) return
            updateMode()
            if (!host.isPassenger()) handler.postDelayed(this, CHIP_TICK_MS)
        }
    }

    init {
        orientation = VERTICAL
        setPadding(dp(12), dp(12), dp(12), dp(12))
        clipToPadding = false
        isFocusable = false
        isFocusableInTouchMode = false
        descendantFocusability = FOCUS_BLOCK_DESCENDANTS
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_YES

        buildHeader()
        buildMode()
        buildSuggestions()
        buildKeyboard()

        speech.onPartial = { part ->
            if (voiceListening && part.isNotBlank()) {
                renderQuery(display = part, asPlaceholder = false)
            }
        }
        speech.onFailure = { message ->
            if (isAttachedToWindow) {
                val wasListening = voiceListening
                if (wasListening) {
                    setListeningUi(false)
                    renderQuery()
                    if (query.isNotBlank()) scheduleSearch(0)
                }
                host.toast(message)
            }
        }

        reset()
    }

    /** Called by the driving trigger; the keyboard and the mode chip follow. */
    fun setDriving(value: Boolean) {
        val changed = driving != value
        driving = value
        updateMode()
        if (changed && !voiceListening && query.isEmpty()) renderQuery()
        scheduleChipTick()
    }

    /** Clear the query and the results, re-read the layout and the driving state. */
    fun reset() {
        interrupt()
        query = ""
        driving = host.isDriving()
        clearSuggestions()
        rebuildKeyboard()
        renderQuery()
        scheduleChipTick()
    }

    // -- layout ---------------------------------------------------------------

    private fun buildHeader() {
        val row = LinearLayout(context).apply {
            orientation = HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }

        val field = LinearLayout(context).apply {
            orientation = HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(16), 0, dp(12), 0)
            background = CarStyle.roundRect(context, withAlpha(CarStyle.TX, 0.10f), radiusDp = 16f)
        }
        queryView = TextView(context).apply {
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 22f)
            typeface = Typeface.create(CarType.body(context), Typeface.BOLD)
            setTextColor(CarStyle.TX)
            gravity = Gravity.START or Gravity.CENTER_VERTICAL
            includeFontPadding = false
            setSingleLine(true)
            maxLines = 1
            ellipsize = TextUtils.TruncateAt.START
            isFocusable = false
            isFocusableInTouchMode = false
            isClickable = false
            isLongClickable = false
            setTextIsSelectable(false)
            setPadding(0, 0, 0, 0)
        }
        field.addView(queryView, LinearLayout.LayoutParams(0, dp(30), 1f))
        row.addView(field, LinearLayout.LayoutParams(0, dp(52), 1f))

        micIcon = CarIconView(context).apply {
            name = "mic"
            sizeDp = 24
            color = CarStyle.ONAC
        }
        micButton = FrameLayout(context).apply {
            contentDescription = "Recherche vocale"
            addView(
                micIcon,
                FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.WRAP_CONTENT,
                    FrameLayout.LayoutParams.WRAP_CONTENT,
                    Gravity.CENTER,
                ),
            )
        }
        applyMicBackground()
        micButton.setOnClickListener { onMic() }
        row.addView(micButton, LinearLayout.LayoutParams(dp(52), dp(52)).apply { marginStart = dp(8) })

        closeButton = FrameLayout(context).apply {
            contentDescription = "Fermer la recherche"
            val icon = CarIconView(context).apply {
                name = "close"
                sizeDp = 24
                color = CarStyle.TX
            }
            addView(
                icon,
                FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.WRAP_CONTENT,
                    FrameLayout.LayoutParams.WRAP_CONTENT,
                    Gravity.CENTER,
                ),
            )
        }
        tappable(
            closeButton,
            CarStyle.pill(context, Color.TRANSPARENT, withAlpha(CarStyle.TX, 0.35f)),
            CarStyle.pill(context, CarStyle.TX),
        )
        closeButton.setOnClickListener { onClose() }
        row.addView(closeButton, LinearLayout.LayoutParams(dp(52), dp(52)).apply { marginStart = dp(8) })

        addView(row, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT))
    }

    private fun buildMode() {
        // The prototype lets the passenger button wrap under the mode chip; a
        // vertical column is the Android equivalent of that flex-wrap, and it
        // keeps the chip's full text readable on a 390 dp phone.
        val column = LinearLayout(context).apply {
            orientation = VERTICAL
            gravity = Gravity.START
        }
        modeChip = CarStyle.monoText(context, "", 12f, CarStyle.TX).apply {
            gravity = Gravity.CENTER
            setPadding(dp(10), 0, dp(10), 0)
            setSingleLine(true)
            maxLines = 1
            ellipsize = TextUtils.TruncateAt.END
            isClickable = false
            isFocusable = false
        }
        column.addView(modeChip, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, dp(30)))

        passengerButton = CarStyle.text(context, "Je suis passager", 13f, CarStyle.TX).apply {
            setTypeface(typeface, Typeface.BOLD)
            gravity = Gravity.CENTER
            setPadding(dp(12), 0, dp(12), 0)
            visibility = GONE
        }
        tappable(
            passengerButton,
            CarStyle.pill(context, Color.TRANSPARENT, withAlpha(CarStyle.TX, 0.40f)),
            CarStyle.pill(context, CarStyle.TX),
        )
        passengerButton.setOnClickListener { onPassenger() }
        column.addView(
            passengerButton,
            LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, dp(30)).apply { topMargin = dp(6) },
        )

        addView(column, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(8) })
    }

    private fun buildSuggestions() {
        suggestions = LinearLayout(context).apply {
            orientation = VERTICAL
            isClickable = false
            isFocusable = false
        }
        // Only the rows actually drawn take room (0–3 × 42/46 dp plus 5 dp
        // gaps), and the whole panel is bottom-anchored in CarOverlayRoot: an
        // answer grows the card upward, it never moves the keyboard under a
        // finger. An empty area is GONE so its 8 dp gap is not reserved either.
        addView(
            suggestions,
            LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
                topMargin = dp(8)
            },
        )
    }

    private fun buildKeyboard() {
        keyboard = LinearLayout(context).apply {
            orientation = VERTICAL
            // Latin key rows stay left-to-right even on an RTL phone.
            layoutDirection = LAYOUT_DIRECTION_LTR
            isFocusable = false
        }
        addView(keyboard, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(8) })
    }

    private fun rebuildKeyboard() {
        val layout = resolveLayout()
        keyboard.removeAllViews()
        spaceKey = null

        val rows = when (layout) {
            "azerty" -> listOf("AZERTYUIOP", "QSDFGHJKLM", "WXCVBN")
            "qwertz" -> listOf("QWERTZUIOP", "ASDFGHJKL", "YXCVBNM")
            "abc" -> listOf("ABCDEFG", "HIJKLMN", "OPQRSTU", "VWXYZ")
            else -> listOf("QWERTYUIOP", "ASDFGHJKL", "ZXCVBNM")
        }

        rows.forEachIndexed { index, letters ->
            val row = LinearLayout(context).apply {
                orientation = HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                if ((layout == "qwerty" || layout == "qwertz") && index == 1) {
                    setPadding(dp(14), 0, dp(14), 0)
                }
            }
            letters.forEachIndexed { i, letter ->
                val key = keyView(letter.toString(), "Lettre $letter", vowel = letter in VOWELS)
                key.setOnClickListener {
                    leaveVoiceForTyping()
                    onLetter(letter)
                }
                row.addView(key, keyParams(1f, if (i == 0) 0 else 5))
            }
            if (index == rows.lastIndex) {
                val delete = keyView("⌫", "Effacer", accent = true)
                delete.setOnClickListener {
                    leaveVoiceForTyping()
                    onBackspace()
                }
                row.addView(delete, keyParams(if (layout == "abc") 2f else 1.5f, 5))
            }
            keyboard.addView(
                row,
                LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(keyHeightDp)).apply {
                    if (index > 0) topMargin = dp(5)
                },
            )
        }

        val space = keyView("espace", "Espace", space = true)
        spaceKey = space
        space.setOnClickListener {
            leaveVoiceForTyping()
            onSpace()
        }
        keyboard.addView(
            space,
            LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(keyHeightDp)).apply { topMargin = dp(5) },
        )
        updateMode()
    }

    private fun keyView(
        label: String,
        description: String,
        vowel: Boolean = false,
        accent: Boolean = false,
        space: Boolean = false,
    ): TextView {
        val fill = when {
            accent -> withAlpha(CarStyle.AC, 0.22f)
            vowel -> withAlpha(CarStyle.TX, 0.20f)
            else -> withAlpha(CarStyle.TX, 0.08f)
        }
        val stroke = if (accent) withAlpha(CarStyle.AC, 0.60f) else withAlpha(CarStyle.TX, 0.22f)
        val key = TextView(context).apply {
            text = label
            gravity = Gravity.CENTER
            includeFontPadding = false
            setTextColor(CarStyle.TX)
            setTextSize(TypedValue.COMPLEX_UNIT_SP, if (space) 16f else 21f)
            typeface = Typeface.create(CarType.body(context), Typeface.BOLD)
            minWidth = 0
            minHeight = 0
            setPadding(0, 0, 0, 0)
            contentDescription = description
            isFocusable = true
        }
        tappable(
            key,
            CarStyle.roundRect(context, fill, stroke, radiusDp = 12f),
            CarStyle.roundRect(context, CarStyle.TX, radiusDp = 12f),
        )
        return key
    }

    private fun keyParams(weight: Float, marginStart: Int = 0): LinearLayout.LayoutParams =
        LinearLayout.LayoutParams(0, dp(keyHeightDp), weight).apply {
            this.marginStart = dp(marginStart)
        }

    // -- state ----------------------------------------------------------------

    private fun isFree(): Boolean = !driving || host.isPassenger()

    private fun updateMode() {
        val passenger = host.isPassenger()
        val free = !driving || passenger
        val label = when {
            driving && !passenger -> {
                val speed = host.speedKmh()
                val kmh =
                    if (speed.isFinite() && speed > 0.5) "${speed.roundToInt()} km/h · " else ""
                "En roulant · ${kmh}${prefs.maxLetters} lettres max"
            }
            passenger -> "Mode passager · clavier complet"
            else -> "À l'arrêt · clavier complet"
        }
        modeChip.text = label
        modeChip.setTextColor(if (free) CarStyle.AC else CarStyle.TX)
        modeChip.background = CarStyle.pill(
            context,
            Color.TRANSPARENT,
            if (free) withAlpha(CarStyle.AC, 0.70f) else withAlpha(CarStyle.TX, 0.40f),
        )
        passengerButton.visibility = if (driving && !passenger) VISIBLE else GONE
        spaceKey?.let { key ->
            key.isEnabled = free
            key.alpha = if (free) 1f else 0.35f
            key.contentDescription = if (free) "Espace" else "Espace indisponible en roulant"
        }
    }

    private fun scheduleChipTick() {
        handler.removeCallbacks(chipTick)
        if (driving && !host.isPassenger() && isShown) {
            handler.postDelayed(chipTick, CHIP_TICK_MS)
        }
    }

    private fun onPassenger() {
        stopVoice()
        host.setPassenger(true)
        updateMode()
        renderQuery()
        scheduleChipTick()
    }

    private fun onClose() {
        reset()
        host.setScreen(CarScreen.PLAYER)
    }

    // -- query ----------------------------------------------------------------

    private fun placeholderText(): String =
        if (isFree()) "Tape ou dis un nom" else "Dis un nom, ou ${prefs.maxLetters} lettres"

    private fun renderQuery(
        display: String = if (query.isEmpty()) placeholderText() else query,
        asPlaceholder: Boolean = query.isEmpty(),
    ) {
        queryView.text = withCaret(display)
        queryView.setTextColor(if (asPlaceholder) CarStyle.SUB else CarStyle.TX)
        queryView.setTypeface(CarType.body(context), if (asPlaceholder) Typeface.NORMAL else Typeface.BOLD)
        queryView.setTextSize(TypedValue.COMPLEX_UNIT_SP, if (asPlaceholder) 18f else 22f)
    }

    private fun onLetter(letter: Char) {
        val max = if (isFree()) Int.MAX_VALUE else prefs.maxLetters
        query = if (query.length < max) query + letter else letter.toString()
        renderQuery()
        scheduleSearch(DEBOUNCE_MS)
    }

    private fun onBackspace() {
        if (query.isEmpty()) {
            renderQuery()
            return
        }
        val before = query.trim()
        query = query.dropLast(1).trimStart()
        renderQuery()
        if (query.trim() != before) scheduleSearch(DEBOUNCE_MS)
    }

    private fun onSpace() {
        if (!isFree()) return
        if (query.isEmpty() || query.endsWith(" ")) return
        query += " "
        renderQuery()
        // The server trims the query, so a trailing space changes no result:
        // no request, and the suggestions for the previous word stay visible.
    }

    private fun withCaret(value: String): CharSequence {
        val text = SpannableString(value + "\u200B")
        text.setSpan(CaretSpan(), text.length - 1, text.length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        return text
    }

    /**
     * The lemon caret the prototype draws after the text. A ReplacementSpan on a
     * zero-width space keeps it in the text flow (after the last glyph, and after
     * the placeholder on an empty field) without making the field an editor.
     */
    private inner class CaretSpan : ReplacementSpan() {
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = CarStyle.AC }
        private val caretWidth = dp(2)
        private val caretHeight = dp(26)

        override fun getSize(
            paint: Paint,
            text: CharSequence,
            start: Int,
            end: Int,
            fm: Paint.FontMetricsInt?,
        ): Int = caretWidth

        override fun draw(
            canvas: Canvas,
            text: CharSequence,
            start: Int,
            end: Int,
            x: Float,
            top: Int,
            y: Int,
            bottom: Int,
            paint: Paint,
        ) {
            val metrics = paint.fontMetrics
            val center = y + (metrics.ascent + metrics.descent) / 2f
            val half = caretHeight / 2f
            canvas.drawRoundRect(
                x,
                center - half,
                x + caretWidth,
                center + half,
                caretWidth / 2f,
                caretWidth / 2f,
                this.paint,
            )
        }
    }

    // -- voice ----------------------------------------------------------------

    private fun onMic() {
        if (voiceListening) {
            speech.stop()
            setListeningUi(false)
            renderQuery()
            if (query.isNotBlank()) scheduleSearch(0)
            return
        }
        if (!CarPermissions.hasMicrophone(context)) {
            host.toast("Micro non autorisé — autorise-le dans les réglages")
            return
        }
        if (!speech.start()) return
        setListeningUi(true)
        clearSuggestions()
        renderQuery(display = "Écoute…", asPlaceholder = true)
    }

    private fun onSpeechText(text: String) {
        if (!isAttachedToWindow) return
        setListeningUi(false)
        val clean = text.trim().replace(WHITESPACE, " ")
        if (clean.isEmpty()) {
            host.toast("Rien entendu")
            renderQuery()
            return
        }
        // A spoken query is NOT truncated to maxLetters: the limit is the rule
        // for the drawn keyboard, and cutting "Angerfist" to three letters would
        // make the voice button useless. The next drawn letter still replaces it.
        query = clean
        renderQuery()
        scheduleSearch(0)
    }

    private fun setListeningUi(on: Boolean) {
        voiceListening = on
        pulse?.cancel()
        pulse = null
        micIcon.alpha = 1f
        applyMicBackground()
        if (on) {
            micButton.contentDescription = "Arrêter l'écoute"
            pulse = ObjectAnimator.ofFloat(micIcon, "alpha", 1f, 0.35f).apply {
                duration = 550L
                repeatMode = ValueAnimator.REVERSE
                repeatCount = ValueAnimator.INFINITE
                start()
            }
        } else {
            micButton.contentDescription = "Recherche vocale"
        }
    }

    private fun applyMicBackground() {
        val fill = if (voiceListening) CarStyle.S2 else CarStyle.AC
        val stroke = if (voiceListening) CarStyle.AC else Color.TRANSPARENT
        tappable(
            micButton,
            CarStyle.pill(context, fill, stroke),
            CarStyle.pill(context, CarStyle.TX),
            rippleColor = if (voiceListening) CarStyle.AC else CarStyle.ONAC,
        )
        micIcon.color = if (voiceListening) CarStyle.AC else CarStyle.ONAC
    }

    private fun stopVoice() {
        speech.stop()
        if (voiceListening) {
            setListeningUi(false)
            renderQuery()
        }
    }

    private fun leaveVoiceForTyping() {
        if (!voiceListening) return
        speech.stop()
        setListeningUi(false)
        renderQuery()
    }

    // -- search ---------------------------------------------------------------

    private fun scheduleSearch(delayMs: Long) {
        searchGeneration++
        // A new keystroke abandons the request already on the wire: the worker
        // fails fast instead of holding a download slot for an answer nobody
        // will read.
        cancelRequest()
        searchRunnable?.let { handler.removeCallbacks(it) }
        searchRunnable = null
        val q = query.trim()
        if (q.length < MIN_QUERY_CHARS) {
            // Empty, or one letter: /api/search fans that out to five upstream
            // searches for nothing. No request, and no spinner that no answer
            // will ever end.
            clearSuggestions()
            return
        }
        val generation = searchGeneration
        val runnable = Runnable { requestSearch(q, generation) }
        searchRunnable = runnable
        handler.postDelayed(runnable, delayMs)
    }

    private fun requestSearch(q: String, generation: Int) {
        if (!isAttachedToWindow) return
        if (generation != searchGeneration || q.length < MIN_QUERY_CHARS) return
        if (!CarServer.configured()) {
            showError("Serveur non configuré", session = false)
            return
        }
        val core = fr.neezer.mobile.bridge.NzBridge.core
        if (core == null) {
            showError("Recherche indisponible", session = false)
            return
        }
        val screens = core.screens()
        showLoading()
        activeJob = searchScope().launch {
            val outcome = try {
                val answer = screens.search(q, uniffi.nz_ffi.SearchScope.LIBRARY, false)
                SearchOutcome.Done(suggestionsOf(answer, q))
            } catch (error: Throwable) {
                SearchOutcome.Failed(messageFor(error), session = isSessionError(error))
            }
            handler.post {
                if (generation == searchGeneration && isAttachedToWindow) {
                    when (outcome) {
                        is SearchOutcome.Done -> showSuggestions(outcome.items)
                        is SearchOutcome.Failed ->
                            showError(outcome.message, outcome.session)
                    }
                }
            }
        }
    }

    /** The view's own scope, recreated if a previous detach cancelled it. */
    private fun searchScope(): CoroutineScope {
        searchScope?.let { if (it.coroutineContext[Job]?.isActive != false) return it }
        return CoroutineScope(
            SupervisorJob() + Dispatchers.IO
        ).also { searchScope = it }
    }

    /**
     * Abandon the call in flight. The UniFFI future is cancelled, which drops
     * Rust's own request before it writes anything (the cancellation rule of the
     * boundary), and its generation check keeps the failure off the screen.
     */
    private fun cancelRequest() {
        val job = activeJob ?: return
        activeJob = null
        runCatching { job.cancel() }
    }

    /** The cause in the person's words, from the core's own error register. */
    private fun messageFor(error: Throwable): String {
        if (error is uniffi.nz_ffi.NzException) {
            return when (error) {
                is uniffi.nz_ffi.NzException.Offline -> "Hors ligne"
                is uniffi.nz_ffi.NzException.Timeout -> "Délai dépassé"
                is uniffi.nz_ffi.NzException.Session -> "Session expirée"
                is uniffi.nz_ffi.NzException.Certificate -> "Certificat refusé"
                is uniffi.nz_ffi.NzException.Server ->
                    if (error.status.toInt() == 404) "Recherche indisponible" else "Serveur en erreur"
                else -> NzErrors.text(error)
            }
        }
        return NzErrors.text(error)
    }

    private fun isSessionError(error: Throwable): Boolean =
        error is uniffi.nz_ffi.NzException.Session

    /**
     * The core's answer, in the shape the panel draws: the SAME priority the old
     * parser gave (a result whose title starts with what was typed first, then
     * the server's own order across artists, playlists, titles and albums), the
     * same three rows, and the same one-line subtitle per kind.
     */
    private fun suggestionsOf(
        answer: uniffi.nz_ffi.SearchResults,
        q: String,
    ): List<Suggestion> {
        val folded = fold(q)
        val candidates = mutableListOf<Candidate>()
        var order = 0

        fun add(kind: String, type: String, id: String, title: String, sub: String, row: uniffi.nz_ffi.TrackRow?) {
            if (title.isBlank() || id.isBlank()) return
            candidates.add(
                Candidate(
                    prefix = fold(title).startsWith(folded),
                    order = order++,
                    suggestion = Suggestion(kind, id, title, sub, type, row),
                )
            )
        }

        for (entity in answer.artists) {
            add("artist", "Artiste", entity.id, entity.title, entity.subtitle, null)
        }
        for (entity in answer.playlists) {
            add("playlist", "Playlist", entity.id, entity.title, entity.subtitle, null)
        }
        for (row in answer.tracks.window(0u, SUGGESTION_SCAN)) {
            add("track", "Titre", row.id, row.title, row.artists, row)
        }
        for (entity in answer.albums) {
            add("album", "Album", entity.id, entity.title, entity.subtitle, null)
        }

        return candidates
            .sortedWith(compareByDescending<Candidate> { it.prefix }.thenBy { it.order })
            .map { it.suggestion }
            .take(MAX_SUGGESTIONS)
    }

    private fun showLoading() {
        clearSuggestions()
        val row = CarStyle.monoText(context, "Recherche…", 12f, CarStyle.DIM).apply {
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(14), 0, dp(14), 0)
            contentDescription = "Recherche en cours"
        }
        suggestions.addView(row, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(suggestionHeightDp)))
        suggestions.visibility = VISIBLE
    }

    private fun showSuggestions(items: List<Suggestion>) {
        clearSuggestions()
        if (items.isEmpty()) {
            val empty = CarStyle.text(context, "Aucun résultat", 15f, CarStyle.DIM).apply {
                setPadding(dp(14), 0, dp(14), 0)
                contentDescription = "Aucun résultat"
            }
            suggestions.addView(empty, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(suggestionHeightDp)))
            suggestions.visibility = VISIBLE
            return
        }
        items.forEachIndexed { index, item ->
            suggestions.addView(suggestionRow(item, index))
        }
        suggestions.visibility = VISIBLE
    }

    private fun suggestionRow(
        item: Suggestion,
        index: Int,
        accent: Int = CarStyle.AC,
        subColor: Int = CarStyle.SUB,
        onTap: (() -> Unit)? = null,
    ): LinearLayout {
        val row = LinearLayout(context).apply {
            orientation = HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(14), 0, dp(14), 0)
            isFocusable = true
            contentDescription = listOf(item.type, item.title, item.sub)
                .filter { it.isNotBlank() }
                .joinToString(", ")
        }
        row.layoutParams = LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(suggestionHeightDp)).apply {
            if (index > 0) topMargin = dp(5)
        }

        val type = CarStyle.monoText(
            context,
            item.type.uppercase(Locale.FRENCH),
            11f,
            accent,
        ).apply {
            gravity = Gravity.CENTER_VERTICAL
            letterSpacing = 0.06f
            maxLines = 1
            ellipsize = TextUtils.TruncateAt.END
            importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
        }
        row.addView(type, LinearLayout.LayoutParams(dp(62), LinearLayout.LayoutParams.WRAP_CONTENT))

        val label = CarStyle.text(context, item.title, 19f, CarStyle.TX).apply {
            setTypeface(typeface, Typeface.BOLD)
            maxLines = 1
            ellipsize = TextUtils.TruncateAt.END
            importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
        }
        row.addView(
            label,
            LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f).apply { marginStart = dp(12) },
        )

        if (item.sub.isNotBlank()) {
            val sub = CarStyle.text(context, item.sub, 13f, subColor).apply {
                maxLines = 1
                ellipsize = TextUtils.TruncateAt.END
                importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
            }
            row.addView(
                sub,
                LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { marginStart = dp(8) },
            )
        }

        tappable(
            row,
            CarStyle.roundRect(context, withAlpha(CarStyle.TX, 0.10f), radiusDp = 14f),
            CarStyle.roundRect(context, CarStyle.TX, radiusDp = 14f),
        )
        row.setOnClickListener {
            val tap = onTap
            if (tap != null) tap() else pick(item)
        }
        return row
    }

    private fun showError(message: String, session: Boolean) {
        clearSuggestions()
        val row = suggestionRow(
            Suggestion(
                kind = "",
                id = "",
                title = message,
                sub = "Réessayer",
                type = if (session) "Session" else "Réseau",
                row = null,
            ),
            index = 0,
            accent = CarStyle.WARN,
            subColor = CarStyle.AC,
            onTap = { scheduleSearch(0) },
        )
        row.contentDescription = "$message. Réessayer"
        suggestions.addView(row)
        suggestions.visibility = VISIBLE
    }

    private fun clearSuggestions() {
        suggestions.removeAllViews()
        // GONE, not INVISIBLE: an empty area takes no height AND no 8 dp gap,
        // so the panel is exactly as tall as what it shows.
        suggestions.visibility = GONE
    }

    private fun pick(item: Suggestion) {
        if (item.id.isBlank()) return
        host.launch(item.kind, item.id, item.title, item.row)
        reset()
        host.setScreen(CarScreen.PLAYER)
    }

    // -- auto layout ----------------------------------------------------------

    private fun resolveLayout(): String {
        val chosen = prefs.keyboardLayout
        if (chosen != "auto") return chosen
        val locale = detectKeyboardLocale() ?: return "qwerty"
        val language = locale.language.lowercase(Locale.ROOT)
        val country = locale.country.uppercase(Locale.ROOT)
        return when (language) {
            "fr" -> when (country) {
                // French Canadian keyboards are QWERTY and Swiss ones QWERTZ;
                // the hexagon's AZERTY is only the common case, not the law.
                "CA" -> "qwerty"
                "CH" -> "qwertz"
                else -> "azerty"
            }
            "de" -> "qwertz"
            else -> "qwerty"
        }
    }

    private fun detectKeyboardLocale(): Locale? {
        val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
            ?: return Locale.getDefault()
        val candidates = mutableListOf<Locale>()
        try {
            imm.currentInputMethodSubtype?.let { subtype ->
                if (subtype.mode != "voice") subtypeLocale(subtype)?.let { candidates.add(it) }
            }
            imm.enabledInputMethodList.forEach { ime ->
                imm.getEnabledInputMethodSubtypeList(ime, true).forEach { subtype ->
                    if (subtype.mode != "voice") {
                        subtypeLocale(subtype)?.let { candidates.add(it) }
                    }
                }
            }
        } catch (_: Exception) {
            // An IME can disappear between the two system calls. The phone's
            // locale is a fine floor for a car keyboard.
        }
        return candidates.firstOrNull() ?: Locale.getDefault()
    }

    private fun subtypeLocale(subtype: InputMethodSubtype): Locale? {
        val tag = subtype.languageTag
        if (tag.isNotBlank()) {
            val fromTag = Locale.forLanguageTag(tag)
            if (fromTag.language.isNotBlank() && fromTag.language != "und") return fromTag
        }
        // Older/custom IMEs only fill the deprecated locale field ("fr_FR").
        @Suppress("DEPRECATION")
        val raw = subtype.locale
        if (raw.isBlank()) return null
        return Locale.forLanguageTag(raw.replace('_', '-'))
            .takeIf { it.language.isNotBlank() && it.language != "und" }
    }

    // -- lifecycle ------------------------------------------------------------

    /** Stop the microphone and invalidate a search nobody is waiting for. */
    private fun interrupt() {
        stopVoice()
        searchGeneration++
        searchRunnable?.let { handler.removeCallbacks(it) }
        searchRunnable = null
        cancelRequest()
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        driving = host.isDriving()
        updateMode()
        if (!voiceListening) renderQuery()
        scheduleChipTick()
    }

    override fun onDetachedFromWindow() {
        interrupt()
        // This handler is the view's own: nothing it queued (a late result, the
        // speed ticker) may run after the service dropped the window.
        handler.removeCallbacksAndMessages(null)
        searchScope?.cancel()
        searchScope = null
        speech.destroy()
        pulse?.cancel()
        pulse = null
        super.onDetachedFromWindow()
    }

    override fun onVisibilityChanged(changedView: View, visibility: Int) {
        super.onVisibilityChanged(changedView, visibility)
        // isShown() covers this view AND its ancestors: a hidden panel must not
        // keep the microphone open, and the speed ticker stops with it. The
        // search already asked for is left to answer: one small request whose
        // result is still valid when the panel comes back.
        if (isShown) {
            updateMode()
            if (!voiceListening) renderQuery()
            scheduleChipTick()
        } else {
            stopVoice()
        }
    }

    override fun onWindowVisibilityChanged(visibility: Int) {
        super.onWindowVisibilityChanged(visibility)
        if (visibility == VISIBLE) {
            updateMode()
            if (!voiceListening) renderQuery()
            scheduleChipTick()
        } else {
            stopVoice()
        }
    }

    // -- drawing helpers ------------------------------------------------------

    private fun tappable(
        view: View,
        shape: GradientDrawable,
        mask: GradientDrawable,
        rippleColor: Int = withAlpha(CarStyle.TX, 0.16f),
    ) {
        view.background = RippleDrawable(ColorStateList.valueOf(rippleColor), shape, mask)
        view.isClickable = true
    }

    private fun withAlpha(color: Int, fraction: Float): Int =
        ColorUtils.setAlphaComponent(
            color,
            (Color.alpha(color) * fraction).roundToInt().coerceIn(0, 255),
        )

    private fun dp(value: Int): Int = CarStyle.dp(context, value.toFloat())

    private fun fold(value: String): String {
        val decomposed = Normalizer.normalize(value, Normalizer.Form.NFD)
        return decomposed.replace(MARKS, "").lowercase(Locale.ROOT)
    }

    private data class Suggestion(
        val kind: String,
        val id: String,
        val title: String,
        val sub: String,
        val type: String,
        /** The row a `track` tap queues; null for everything else. */
        val row: uniffi.nz_ffi.TrackRow?,
    )

    private data class Candidate(
        val prefix: Boolean,
        val order: Int,
        val suggestion: Suggestion,
    )

    private sealed class SearchOutcome {
        data class Done(val items: List<Suggestion>) : SearchOutcome()
        data class Failed(val message: String, val session: Boolean) : SearchOutcome()
    }

    companion object {
        private const val MAX_SUGGESTIONS = 3
        /** How many titles the panel looks at before it ranks: three are drawn. */
        private const val SUGGESTION_SCAN = 12u
        // /api/search is five upstream searches behind the route (artists,
        // albums, tracks, playlists, podcasts). 700 ms is long enough that a
        // typed name costs one request instead of one per letter, short enough
        // that the pause after a word still feels like the answer is immediate.
        private const val DEBOUNCE_MS = 700L
        // One letter matches half a library and answers nothing worth five
        // round trips; two is the first useful prefix.
        private const val MIN_QUERY_CHARS = 2
        private const val CHIP_TICK_MS = 4000L

        private val VOWELS = setOf('A', 'E', 'I', 'O', 'U', 'Y')
        private val MARKS = Regex("\\p{M}+")
        private val WHITESPACE = Regex("\\s+")
    }
}
