package org.nsupysonic.app

import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.text.TextUtils
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.webkit.CookieManager
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.util.concurrent.Executors
import kotlin.math.max

/**
 * The car overlay's playlist panel: "Mon Flow" then the pinned playlists, six
 * 84 dp tiles to a page, two columns, pagination, and a dashed "pin more" tile
 * that opens the settings.
 *
 * The list itself is the SERVER's (`/api/me/playlists`), fetched natively with
 * the WebView's own session cookie through [Net] — the same origin rule, pin
 * and redirect discipline as every other native fetch. The pinned order and
 * titles live in [CarPrefs], so the panel still paints the last known tiles
 * with no network, no session or no server: it says which of those it is, and
 * never invents a playlist.
 *
 * Nothing here plays anything: a tap is [CarHost.launch], the only door to the
 * SPA's own `__nsCarPlay`, and the player remains the page's.
 *
 * [server] is the app's [Prefs] (host, pin); [prefs] is the car's own
 * [CarPrefs]. The three-argument overloads let the service pass the object it
 * has at hand without guessing: the car's preferences or the server's.
 */
class CarPlaylistsView @JvmOverloads constructor(
    context: Context,
    private val host: CarHost,
    private val server: Prefs = Prefs(context),
    private val prefs: CarPrefs = host.prefs,
) : LinearLayout(context) {

    constructor(context: Context, host: CarHost, carPrefs: CarPrefs) :
        this(context, host, Prefs(context), carPrefs)

    private enum class Load { IDLE, LOADING, READY, NO_SESSION, FAILED }

    private inner class Cell(
        val id: String,
        val title: String,
        val count: Int,
        val flow: Boolean,
    ) {
        val countLabel: String
            get() = when {
                flow -> "selon ton écoute"
                count <= 0 -> "playlist"
                count == 1 -> "1 titre"
                else -> "$count titres"
            }

        val coverSeed: String get() = if (flow) "flow" else id
    }

    private val main = Handler(Looper.getMainLooper())
    private val grid = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
    private lateinit var subtitle: TextView
    private lateinit var pageLabel: TextView
    private lateinit var previous: FrameLayout
    private lateinit var next: FrameLayout

    private var state = Load.IDLE
    private var items: List<CarPlaylistSource.Item> = emptyList()
    private var page = 0
    private var loadSeq = 0
    private var lastRefreshAt = 0L
    private var lastSuccessAt = 0L

    private val subColor = 0xFFD5D2C8.toInt()
    private val white10 = 0x1AFFFFFF
    private val white12 = 0x1FFFFFFF
    private val white22 = 0x38FFFFFF
    private val white35 = 0x59FFFFFF

    init {
        orientation = LinearLayout.VERTICAL
        clipToPadding = false
        setPadding(d(12), d(12), d(12), d(14))
        addView(buildHeader(), LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        addView(
            grid,
            LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply {
                topMargin = d(10)
            }
        )
        render()
    }

    // -- public contract -----------------------------------------------------

    /**
     * Repaint from the pins already known, then refresh the server list.
     * Called when the panel opens; a second call within 400 ms (attach and
     * visibility often both fire) is coalesced.
     */
    fun refresh() = refresh(false)

    /** Repaint the current pins/pages without a network round-trip. */
    fun render() {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            post { render() }
            return
        }
        val cells = buildCells()
        val pages = pageCount(cells.size)
        page = page.coerceIn(0, pages - 1)
        pageLabel.text = "${page + 1}/$pages"
        pageLabel.contentDescription = "Page ${page + 1} sur $pages"
        val many = pages > 1
        previous.isEnabled = many
        next.isEnabled = many
        previous.alpha = if (many) 1f else 0.35f
        next.alpha = if (many) 1f else 0.35f
        updateSubtitle()

        val slice = cells.drop(page * PAGE_SIZE).take(PAGE_SIZE)
        val shown = mutableListOf<Cell?>()
        shown.addAll(slice)
        if (shown.size < PAGE_SIZE) shown.add(null) // the dashed "pin more" tile
        grid.removeAllViews()
        var row = 0
        while (row * 2 < shown.size) {
            val line = LinearLayout(context).apply { orientation = LinearLayout.HORIZONTAL }
            for (column in 0..1) {
                val at = row * 2 + column
                val cell = shown.getOrNull(at)
                val lp = LinearLayout.LayoutParams(0, d(84), 1f).apply {
                    rightMargin = if (column == 0) d(8) else 0
                }
                if (cell != null) line.addView(tile(cell), lp)
                else if (at < shown.size) line.addView(ghostTile(), lp)
                else line.addView(View(context), lp)
            }
            grid.addView(
                line,
                LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, d(84)).apply {
                    if (row > 0) topMargin = d(8)
                }
            )
            row++
        }
    }

    // -- header --------------------------------------------------------------

    private fun buildHeader(): View {
        val header = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }

        val titles = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
        val title = CarStyle.text(context, "Playlists", 24f, CarStyle.TX, display = true).apply {
            typeface = Typeface.create(CarType.display(context), Typeface.BOLD)
            gravity = Gravity.START or Gravity.CENTER_VERTICAL
        }
        titles.addView(title, wrap())

        val sub = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        val glyph = CarIconView(context).apply {
            name = "pin"
            sizeDp = 11
            color = subColor
            filled = true
        }
        sub.addView(glyph)
        subtitle = CarStyle.monoText(context, "épinglées", 11f, subColor).apply {
            setPadding(d(4), 0, 0, 0)
            maxLines = 2
        }
        sub.addView(subtitle, wrap())
        titles.addView(sub, wrap().apply { topMargin = d(2) })
        header.addView(titles, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))

        previous = roundButton(56, white12, Color.TRANSPARENT, "chevronLeft", "Page précédente") {
            turn(-1)
        }
        header.addView(previous, circleParams(56))

        pageLabel = CarStyle.monoText(context, "1/1", 14f, subColor).apply {
            gravity = Gravity.CENTER
        }
        header.addView(
            pageLabel,
            LinearLayout.LayoutParams(d(36), ViewGroup.LayoutParams.WRAP_CONTENT).apply { leftMargin = d(8) }
        )

        next = roundButton(56, white12, Color.TRANSPARENT, "chevronRight", "Page suivante") {
            turn(1)
        }
        header.addView(next, circleParams(56))

        val close = roundButton(56, Color.TRANSPARENT, white35, "close", "Fermer") {
            host.setScreen(CarScreen.PLAYER)
        }
        header.addView(close, circleParams(56))
        return header
    }

    private fun updateSubtitle() {
        val (text, color, retry) = when (state) {
            Load.NO_SESSION -> Triple("session absente", CarStyle.DIM, false)
            Load.FAILED -> Triple("hors ligne · réessayer", CarStyle.WARN, true)
            else -> Triple("épinglées", subColor, false)
        }
        subtitle.text = text
        subtitle.setTextColor(color)
        subtitle.isClickable = retry
        subtitle.isFocusable = retry
        subtitle.foreground = if (retry) ripple(6f) else null
        subtitle.setOnClickListener(if (retry) View.OnClickListener { refresh(true) } else null)
        subtitle.contentDescription =
            if (retry) "Serveur injoignable, toucher pour réessayer" else null
    }

    // -- tiles ---------------------------------------------------------------

    private fun buildCells(): List<Cell> {
        val byId = items.associateBy { it.id }
        val cells = mutableListOf(Cell("", "Mon Flow", 0, true))
        for (pin in prefs.pins) {
            val fresh = byId[pin.id]
            cells.add(
                Cell(
                    pin.id,
                    fresh?.title?.takeIf { it.isNotBlank() } ?: pin.title.ifBlank { "Playlist" },
                    fresh?.nbTracks ?: pin.nbTracks,
                    false,
                )
            )
        }
        return cells
    }

    private fun pageCount(size: Int): Int = max(1, (size + PAGE_SIZE - 1) / PAGE_SIZE)

    private fun tile(cell: Cell): View {
        val accent = cell.flow
        val box = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(d(10), 0, d(10), 0)
            background = CarStyle.roundRect(
                context,
                if (accent) CarStyle.AC else white10,
                if (accent) Color.TRANSPARENT else white22,
                18f,
                if (accent) 0f else 1f,
            )
            foreground = ripple(18f, if (accent) 0x33000000 else 0x33FFFFFF)
            isClickable = true
            isFocusable = true
            contentDescription = "${cell.title}, ${cell.countLabel}"
            setOnClickListener { pick(cell) }
        }

        val cover = CarCoverView(context).apply {
            seed = cell.coverSeed
            sizeDp = 52
        }
        box.addView(cover, LinearLayout.LayoutParams(d(52), d(52)))

        val column = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
        val name = CarStyle.text(
            context,
            cell.title,
            18f,
            if (accent) CarStyle.ONAC else CarStyle.TX,
        ).apply {
            typeface = Typeface.create(CarType.body(context), Typeface.BOLD)
            gravity = Gravity.START
            maxLines = 2
            ellipsize = TextUtils.TruncateAt.END
            setLineSpacing(0f, 1.08f)
        }
        column.addView(name, wrap())
        val count = CarStyle.monoText(
            context,
            cell.countLabel,
            12f,
            if (accent) CarStyle.ONAC else CarStyle.TX,
        ).apply {
            alpha = 0.8f
        }
        column.addView(count, wrap().apply { topMargin = d(1) })
        box.addView(
            column,
            LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { leftMargin = d(10) }
        )
        return box
    }

    private fun ghostTile(): View {
        val box = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(d(10), 0, d(10), 0)
            background = dashed(white35, 18f, 1.5f)
            foreground = ripple(18f)
            isClickable = true
            isFocusable = true
            contentDescription = "Épingle-en d'autres, dans les réglages"
            setOnClickListener { host.openSettings() }
        }
        val placeholder = View(context).apply { background = dashed(0x80FFFFFF.toInt(), 12f, 1.5f) }
        box.addView(placeholder, LinearLayout.LayoutParams(d(52), d(52)))

        val column = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
        val name = CarStyle.text(context, "Épingle-en d'autres", 18f, subColor).apply {
            typeface = Typeface.create(CarType.body(context), Typeface.BOLD)
            gravity = Gravity.START
            maxLines = 2
            ellipsize = TextUtils.TruncateAt.END
        }
        column.addView(name, wrap())
        column.addView(
            CarStyle.monoText(context, "dans les réglages", 12f, CarStyle.DIM),
            wrap().apply { topMargin = d(1) }
        )
        box.addView(
            column,
            LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { leftMargin = d(10) }
        )
        return box
    }

    private fun pick(cell: Cell) {
        // With no session every request answers 401: saying so beats a launch
        // that dies in the API and a toast claiming a playlist started.
        if (state == Load.NO_SESSION) {
            host.toast("Connecte-toi dans l'app pour lancer une playlist")
            return
        }
        // One toast, and only after the page said it started: CarHost.launch
        // changes the screen and announces "Lecture : X". A failed launch
        // leaves the panel where it is.
        if (cell.flow) {
            host.launch("flow", "", "Mon Flow")
        } else {
            host.launch("playlist", cell.id, cell.title)
        }
    }

    private fun turn(delta: Int) {
        val pages = pageCount(prefs.pins.size + 1)
        if (pages <= 1) return
        page = ((page + delta) % pages + pages) % pages
        render()
    }

    // -- the server list -----------------------------------------------------

    private fun refresh(force: Boolean) {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            post { refresh(force) }
            return
        }
        val now = SystemClock.elapsedRealtime()
        if (!force && now - lastRefreshAt < 400L) return
        // A list read a minute ago is still true enough, and the tiles are
        // repainted from the pins below: opening the panel twice does not cost
        // two requests (the route counts every playlist's tracks).
        if (!force && state == Load.READY && lastSuccessAt != 0L && now - lastSuccessAt < 60_000L) {
            render()
            return
        }
        // The overlay is detached and recreated each time the settings screen
        // or a size change drops it: this instance's fields start empty, but
        // the process-wide read may still be fresh. Reuse it instead of
        // asking /api/me/playlists on every attach.
        if (!force && state == Load.IDLE) {
            val cached = CarPlaylistSource.fresh(now)
            if (cached != null) {
                items = cached
                state = Load.READY
                lastSuccessAt = now
                render()
                return
            }
        }
        lastRefreshAt = now
        val seq = ++loadSeq

        val url = CarPlaylistSource.endpoint(server)
        if (url == null) {
            state = Load.NO_SESSION
            render()
            return
        }
        val cookie = CarPlaylistSource.cookie(url)
        if (cookie.isNullOrEmpty()) {
            state = Load.NO_SESSION
            render()
            return
        }

        state = Load.LOADING
        updateSubtitle()
        CarPlaylistSource.IO.execute {
            var expired = false
            val fetched = try {
                CarPlaylistSource.fetch(server, url, cookie)
            } catch (_: CarPlaylistSource.SessionExpired) {
                expired = true
                null
            } catch (_: Exception) {
                null
            }
            main.post {
                if (seq != loadSeq || !isAttachedToWindow) return@post
                if (expired) {
                    CarPlaylistSource.forget()
                    state = Load.NO_SESSION
                } else if (fetched != null) {
                    items = fetched
                    CarPlaylistSource.remember(fetched)
                    remember(fetched)
                    lastSuccessAt = SystemClock.elapsedRealtime()
                    state = Load.READY
                } else {
                    // Keep what we already had: a refresh failure does not
                    // erase a list that was read a minute ago.
                    state = Load.FAILED
                }
                render()
            }
        }
    }

    /** Keep the pinned titles/counts current after a successful list read. */
    private fun remember(list: List<CarPlaylistSource.Item>) {
        val byId = list.associateBy { it.id }
        var changed = false
        val next = prefs.pins.map { pin ->
            val fresh = byId[pin.id] ?: return@map pin
            if (fresh.title == pin.title && fresh.nbTracks == pin.nbTracks) pin
            else {
                changed = true
                PinnedPlaylist(pin.id, fresh.title, fresh.nbTracks)
            }
        }
        if (changed) prefs.pins = next
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        refresh()
    }

    override fun onVisibilityChanged(changedView: View, visibility: Int) {
        super.onVisibilityChanged(changedView, visibility)
        if (changedView === this && visibility == View.VISIBLE && isAttachedToWindow) refresh()
    }

    override fun onDetachedFromWindow() {
        loadSeq++ // an answer in flight is about a panel that is gone
        lastRefreshAt = 0L
        super.onDetachedFromWindow()
    }

    // -- small factories -----------------------------------------------------

    private fun roundButton(
        sizeDp: Int,
        fill: Int,
        stroke: Int,
        icon: String,
        description: String,
        onClick: () -> Unit,
    ): FrameLayout {
        val box = FrameLayout(context).apply {
            background = CarStyle.roundRect(
                context,
                fill,
                stroke,
                sizeDp / 2f,
                if (stroke != Color.TRANSPARENT) 1.5f else 0f,
            )
            foreground = ripple(sizeDp / 2f)
            isClickable = true
            isFocusable = true
            contentDescription = description
            setOnClickListener { onClick() }
        }
        val glyph = CarIconView(context).apply {
            name = icon
            this.sizeDp = if (sizeDp >= 56) 24 else 22
            color = CarStyle.TX
        }
        box.addView(
            glyph,
            FrameLayout.LayoutParams(d(glyph.sizeDp), d(glyph.sizeDp), Gravity.CENTER)
        )
        return box
    }

    private fun circleParams(sizeDp: Int): LinearLayout.LayoutParams =
        LinearLayout.LayoutParams(d(sizeDp), d(sizeDp)).apply { leftMargin = d(8) }

    private fun wrap(): LinearLayout.LayoutParams =
        LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT)

    private fun d(value: Int): Int = CarStyle.dp(context, value.toFloat())
    private fun d(value: Float): Int = CarStyle.dp(context, value)

    private fun ripple(radiusDp: Float, color: Int = 0x33FFFFFF): RippleDrawable {
        val mask = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = d(radiusDp).toFloat()
            setColor(Color.WHITE)
        }
        return RippleDrawable(ColorStateList.valueOf(color), null, mask)
    }

    private fun dashed(stroke: Int, radiusDp: Float, widthDp: Float): GradientDrawable =
        GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = d(radiusDp).toFloat()
            setColor(Color.TRANSPARENT)
            setStroke(d(widthDp), stroke, d(5f).toFloat(), d(4f).toFloat())
        }

    companion object {
        private const val PAGE_SIZE = 6
    }
}

/**
 * The one reader of `/api/me/playlists` for the car (the overlay and the
 * settings screen). It is the server's own route: no new endpoint, no Deezer
 * call, and the session cookie only ever goes to the configured origin because
 * [Net.get] owns that rule.
 */
internal object CarPlaylistSource {
    const val PATH = "/api/me/playlists"
    private const val MAX_BYTES = 1024 * 1024
    private const val MAX_ITEMS = 500

    data class Item(val id: String, val title: String, val nbTracks: Int)

    /** How long a successful read serves every panel without a new request. */
    private const val CACHE_MS = 60_000L
    private var cached: List<Item>? = null
    private var cachedAt = 0L

    /** The last good list if it is younger than a minute, else null. */
    @Synchronized
    fun fresh(now: Long): List<Item>? {
        val list = cached ?: return null
        return if (now - cachedAt in 0 until CACHE_MS) list else null
    }

    /** Keep a successful read for the next panel instance (attach/reattach). */
    @Synchronized
    fun remember(items: List<Item>) {
        cached = items
        cachedAt = SystemClock.elapsedRealtime()
    }

    /** The session is gone: never serve that list to a reattached panel. */
    @Synchronized
    fun forget() {
        cached = null
        cachedAt = 0L
    }

    /** The cookie was absent, expired or refused: the page must log in again. */
    class SessionExpired : Exception()

    /** One daemon thread for both screens; a hung read never blocks the UI. */
    val IO = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "car-playlists").apply { isDaemon = true }
    }

    /** The configured server's list URL, or null when there is no server. */
    fun endpoint(prefs: Prefs): String? {
        val base = prefs.baseUrl().trimEnd('/')
        if (base.isEmpty()) return null
        val url = base + PATH
        return if (prefs.isServerUrl(url)) url else null
    }

    /** The WebView's own session cookie — never read for another origin. */
    fun cookie(url: String): String? = try {
        CookieManager.getInstance().getCookie(url)
    } catch (_: Exception) {
        null
    }

    fun fetch(prefs: Prefs, url: String, cookie: String): List<Item> {
        val conn = try {
            Net.get(prefs, url, cookie, connectTimeout = 8000, readTimeout = 8000)
        } catch (e: java.io.IOException) {
            // Net's one non-2xx answer worth distinguishing: the API says 401
            // when the session cookie is gone or revoked (neezer/webui
            // login_required), which is "log in", not "the server is down".
            if (e.message == "HTTP 401") throw SessionExpired()
            throw e
        }
        val text = try {
            conn.inputStream.use { input ->
                val out = ByteArrayOutputStream()
                val buffer = ByteArray(16 * 1024)
                var total = 0
                while (true) {
                    val read = input.read(buffer)
                    if (read < 0) break
                    total += read
                    if (total > MAX_BYTES) throw java.io.IOException("playlist list too large")
                    out.write(buffer, 0, read)
                }
                String(out.toByteArray(), Charsets.UTF_8)
            }
        } finally {
            conn.disconnect()
        }

        val array: JSONArray = try {
            JSONObject(text).optJSONArray("playlists") ?: JSONArray()
        } catch (_: Exception) {
            throw java.io.IOException("not a playlist list")
        }
        val out = ArrayList<Item>(array.length().coerceAtMost(MAX_ITEMS))
        val seen = HashSet<String>()
        for (i in 0 until array.length()) {
            if (out.size >= MAX_ITEMS) break
            val o = array.optJSONObject(i) ?: continue
            val id = o.optString("id").trim()
            if (id.isEmpty() || !seen.add(id)) continue
            val title = o.optString("title").trim().take(200).ifEmpty { "Playlist" }
            val count = o.optInt("nb_tracks", 0).coerceAtLeast(0)
            out.add(Item(id, title, count))
        }
        return out
    }
}
