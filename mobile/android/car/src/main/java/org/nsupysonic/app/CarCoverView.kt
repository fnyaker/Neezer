package org.nsupysonic.app

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.Rect
import android.graphics.RectF
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.AttributeSet
import android.util.LruCache
import android.view.View
import fr.neezer.mobile.bridge.NzBridge
import java.net.Inet6Address
import java.net.InetAddress
import java.net.URI
import kotlin.math.max
import kotlin.math.min
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * A record sleeve for the car. It now shows the real cover when one is
 * available, and keeps the eight Sillon groove pairs — deterministic from the
 * id, exactly like the SPA's `Cover` fallback — as the placeholder while the
 * image loads and as the final answer when it never does.
 *
 * A cover is decoration: it must never delay the title, the buttons or the
 * sound. The view paints its grooves in the same frame it is given a URL, asks
 * [CarCovers] off the main thread, and swaps the bitmap in when it arrives. A
 * late answer for a previous track is dropped by generation.
 */
class CarCoverView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
) : View(context, attrs) {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val imagePaint = Paint(Paint.ANTI_ALIAS_FLAG or Paint.FILTER_BITMAP_FLAG)
    private val clip = Path()
    private val bounds = RectF()
    private val source = Rect()

    private var bitmap: Bitmap? = null
    private var request: CarCovers.Request? = null
    private var generation = 0
    private var loadedUrl = ""

    var seed: String = ""
        set(v) {
            field = v
            invalidate()
        }

    var sizeDp: Int = 72
        set(v) {
            field = v
            requestLayout()
            invalidate()
        }

    var round: Boolean = false
        set(v) {
            field = v
            invalidate()
        }

    /**
     * The track or playlist cover URL. Blank keeps the grooves only; a URL
     * paints the grooves at once and the image when it arrives. No network or
     * decode ever runs on the main thread.
     */
    var coverUrl: String = ""
        set(v) {
            val clean = v.trim()
            if (clean == field) return
            field = clean
            show(clean)
        }

    private fun show(url: String) {
        generation++
        request?.cancel()
        request = null
        bitmap = null
        // The shim resolves the page's artwork to an absolute URL, but a
        // stored playlist cover is a server path: resolve it once, here, so
        // every caller can hand over whatever the server answered.
        val target = if (url.startsWith("/")) CarCovers.resolve(CarServer.origin(), url) else url
        loadedUrl = target
        invalidate()
        if (target.isEmpty()) return
        val cached = CarCovers.peek(target)
        if (cached != null) {
            bitmap = cached
            invalidate()
            return
        }
        if (!isAttachedToWindow) return
        val want = generation
        request = CarCovers.load(context, target, targetPx()) { result ->
            if (want != generation) return@load
            bitmap = result
            invalidate()
        }
    }

    private fun targetPx(): Int = max(48, CarStyle.dp(context, sizeDp.toFloat()))

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val px = CarStyle.dp(context, sizeDp.toFloat())
        setMeasuredDimension(px, px)
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        if (loadedUrl.isNotEmpty() && bitmap == null) show(loadedUrl)
    }

    override fun onDetachedFromWindow() {
        generation++
        request?.cancel()
        request = null
        bitmap = null
        super.onDetachedFromWindow()
    }

    override fun onDraw(canvas: Canvas) {
        val w = width.toFloat()
        val h = height.toFloat()
        if (w <= 0f || h <= 0f) return
        val radius = if (round) min(w, h) / 2f else CarStyle.dp(context, 12f).toFloat()
        bounds.set(0f, 0f, w, h)
        clip.reset()
        clip.addRoundRect(bounds, radius, radius, Path.Direction.CW)
        canvas.save()
        canvas.clipPath(clip)
        val image = bitmap
        if (image != null && !image.isRecycled) {
            centerCrop(image)
            canvas.drawBitmap(image, source, bounds, imagePaint)
        } else {
            grooves(canvas, w, h)
        }
        canvas.restore()
    }

    private fun centerCrop(image: Bitmap) {
        val bw = image.width
        val bh = image.height
        if (bw <= 0 || bh <= 0) return
        val scale = max(width / bw.toFloat(), height / bh.toFloat())
        val sw = (width / scale).toInt().coerceIn(1, bw)
        val sh = (height / scale).toInt().coerceIn(1, bh)
        val left = ((bw - sw) / 2).coerceAtLeast(0)
        val top = ((bh - sh) / 2).coerceAtLeast(0)
        source.set(left, top, left + sw, top + sh)
    }

    private fun grooves(canvas: Canvas, w: Float, h: Float) {
        val pair = GROOVES[(hash(seed) % GROOVES.size + GROOVES.size) % GROOVES.size]
        paint.style = Paint.Style.FILL
        paint.color = Color.parseColor(pair[0])
        canvas.drawRect(bounds, paint)

        paint.style = Paint.Style.STROKE
        paint.strokeWidth = max(1f, CarStyle.dp(context, 1f).toFloat())
        val ringStep = max(1f, CarStyle.dp(context, 5f).toFloat())
        paint.color = Color.parseColor(pair[1])
        val cx = w * 0.68f
        val cy = h * 0.32f
        val maxR = max(w, h) * 1.1f
        var r = ringStep
        while (r < maxR) {
            canvas.drawCircle(cx, cy, r, paint)
            r += ringStep
        }
    }

    private fun hash(seed: String): Int {
        var h = -0x7ee3623b // 0x811c9dc5
        for (i in seed.indices) {
            h = h xor seed[i].code
            h *= 16777619
        }
        return h
    }

    companion object {
        private val GROOVES = arrayOf(
            arrayOf("#2b2b29", "#3b3b37"),
            arrayOf("#7a1d12", "#9b2a1b"),
            arrayOf("#14304a", "#1c4466"),
            arrayOf("#4a2a62", "#5e3a7b"),
            arrayOf("#1f4a3a", "#2a634e"),
            arrayOf("#6b5a14", "#8a7420"),
            arrayOf("#5a1f3a", "#7a2c50"),
            arrayOf("#3a3a33", "#55554a"),
        )
    }
}

/**
 * The car's cover loader, on the CORE's cache (MB2's `cover_path`).
 *
 * The old app fetched covers itself: `Net` with the WebView's cookie, a
 * `filesDir/car-covers` directory, its own LRU and its own bound. The new app has
 * one cover path for every screen — Rust fetches (lane, allow-list, 300 MB LRU in
 * `cacheDir`) and hands back a FILE, and Android decodes it — so this object is
 * what is left of that machinery, and it keeps only what is still its own:
 *
 * - **an in-memory LRU** (1/16 of the heap, 3–8 MB) so a repaint in the same
 *   frame costs no decoding — the cover the card draws while a track plays is
 *   asked for a hundred times;
 * - **one transfer per URL**: MINI, FULL and ULTRA ask for the same sleeve and
 *   share one call ([CoverFanout]), so the core is not asked three times;
 * - **the request is cancellable**: a tile scrolled away, a settings page
 *   closed, a track skipped drops the coroutine, which drops Rust's future
 *   before it writes anything (the UniFFI cancellation rule);
 * - **a failed URL is remembered for a minute**, so a repaint never retries it;
 * - **the decode is bounded** by `BitmapFactory.inSampleSize` at the size the
 *   view draws, so a 20 000 px image cannot become heap.
 *
 * `coverAllowed` stays in front of the call as a fast, local filter on a
 * hostile-until-checked URL. It is NOT the authority: `Core::cover_path` applies
 * the same allow-list again (configured origin or `*.dzcdn.net` over HTTPS, no
 * private IP literal, no userinfo), so a mistake here can only cost a refused
 * cover, never a request to the person's own network.
 *
 * Nothing here blocks the caller: [load] returns at once, and the bitmap comes
 * back on the main thread.
 */
internal object CarCovers {
    private const val FAIL_TTL_MS = 60_000L
    private const val MAX_CACHED_URL = 2_048

    /** The decode is done off the caller's thread; the core does the fetching. */
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val main = Handler(Looper.getMainLooper())

    private val memory = object : LruCache<String, Bitmap>(memoryBudget()) {
        override fun sizeOf(key: String, value: Bitmap): Int = value.allocationByteCount
    }

    /** URL -> elapsedRealtime after which it may be tried again. */
    private val failed = object : LruCache<String, Long>(256) {}

    private val inflight = CoverFanout<Bitmap?>()

    /** A request in flight; [cancel] makes a late answer a no-op. */
    class Request internal constructor(private val callback: (Bitmap?) -> Unit) {
        @Volatile private var cancelled = false
        internal var job: Job? = null

        fun cancel() {
            cancelled = true
            job?.cancel()
        }

        internal fun isCancelled(): Boolean = cancelled

        internal fun deliver(bitmap: Bitmap?) {
            if (!cancelled) callback(bitmap)
        }
    }

    /** The bitmap already in memory, if any (a synchronous, cheap peek). */
    fun peek(url: String?): Bitmap? {
        val clean = url?.trim().orEmpty()
        if (clean.isEmpty() || !coverIsHttp(clean)) return null
        return try {
            memory.get(clean)
        } catch (_: Exception) {
            null
        }
    }

    /**
     * Load [url] at roughly [maxPx] pixels. The callback runs on the main
     * thread with null when the image cannot be shown (the view keeps its
     * grooves). Returns null for an invalid URL or an immediate answer, and a
     * cancellable request otherwise.
     */
    fun load(
        context: Context,
        url: String,
        maxPx: Int,
        callback: (Bitmap?) -> Unit,
    ): Request? {
        val clean = url.trim()
        if (clean.length > MAX_CACHED_URL || !coverIsHttp(clean)) {
            post { callback(null) }
            return null
        }
        val hit = try {
            memory.get(clean)
        } catch (_: Exception) {
            null
        }
        if (hit != null) {
            post { callback(hit) }
            return null
        }
        val retryAt = failed.get(clean)
        if (retryAt != null && retryAt > SystemClock.elapsedRealtime()) {
            post { callback(null) }
            return null
        }
        // The fast, local half of the allow-list: a junk URL (another host, a
        // private literal, userinfo) costs no JNI hop. Rust refuses it again.
        if (!coverAllowed(clean) { candidate -> CarServer.sameOrigin(candidate) }) {
            rememberFailure(clean)
            post { callback(null) }
            return null
        }
        val request = Request(callback)
        if (!inflight.join(clean, request::deliver)) return request
        request.job = scope.launch { fetch(clean, maxPx, request) }
        return request
    }

    /** Resolve the server's relative cover path against the configured root. */
    fun resolve(baseUrl: String, raw: String?): String = resolveCoverUrl(baseUrl, raw)

    private fun post(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block() else main.post(Runnable { block() })
    }

    /**
     * One cover, from Rust's cached FILE to a decoded bitmap. Nothing is written
     * to disk here: `cover_path` owns the bytes and their eviction, and the
     * response to a cancelled request is that the future is dropped.
     */
    private suspend fun fetch(url: String, maxPx: Int, request: Request) {
        // A view that detached before its turn (a settings screen closed, a
        // tile scrolled away) must not spend a decode on a cover nobody draws.
        if (request.isCancelled() && inflight.size(url) <= 1) {
            deliver(url, null)
            return
        }
        var bitmap: Bitmap? = null
        try {
            val screens = NzBridge.core?.screens()
            if (screens != null) {
                val path = screens.coverPath(url, maxPx.coerceIn(48, 512).toUInt())
                bitmap = decodeFile(path, maxPx)
            }
        } catch (_: Exception) {
            bitmap = null
        }
        if (bitmap != null) {
            try {
                memory.put(url, bitmap)
                failed.remove(url)
            } catch (_: Exception) {
            }
        } else {
            rememberFailure(url)
        }
        deliver(url, bitmap)
    }

    private fun rememberFailure(url: String) {
        try {
            failed.put(url, SystemClock.elapsedRealtime() + FAIL_TTL_MS)
        } catch (_: Exception) {
        }
    }

    private fun deliver(url: String, bitmap: Bitmap?) {
        val callbacks = inflight.finish(url)
        if (callbacks.isEmpty()) return
        val run = Runnable { for (callback in callbacks) callback(bitmap) }
        if (Looper.myLooper() == Looper.getMainLooper()) run.run() else main.post(run)
    }

    /**
     * The file Rust wrote, decoded at the size the view draws: bounds first, then
     * the smallest power-of-two sample that still covers [maxPx].
     */
    private fun decodeFile(path: String, maxPx: Int): Bitmap? {
        if (path.isBlank()) return null
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        try {
            BitmapFactory.decodeFile(path, bounds)
        } catch (_: Exception) {
            return null
        }
        val w = bounds.outWidth
        val h = bounds.outHeight
        if (w <= 0 || h <= 0) return null
        val target = maxPx.coerceIn(48, 512)
        var sample = 1
        while (w / (sample * 2) >= target || h / (sample * 2) >= target) sample *= 2
        val options = BitmapFactory.Options().apply { inSampleSize = sample }
        return try {
            BitmapFactory.decodeFile(path, options)
        } catch (_: Exception) {
            null
        }
    }

    private fun memoryBudget(): Int =
        (Runtime.getRuntime().maxMemory() / 16L).coerceIn(3L * 1024 * 1024, 8L * 1024 * 1024).toInt()
}

/**
 * One transfer per key while it is in flight; every later caller joins it. The
 * first [join] answers true and owns the work, the rest answer false.
 */
internal class CoverFanout<T> {
    private val waiting = HashMap<String, MutableList<(T) -> Unit>>()

    @Synchronized
    fun join(key: String, callback: (T) -> Unit): Boolean {
        val list = waiting[key]
        if (list != null) {
            list.add(callback)
            return false
        }
        waiting[key] = mutableListOf(callback)
        return true
    }

    @Synchronized
    fun finish(key: String): List<(T) -> Unit> = waiting.remove(key) ?: emptyList()

    /** How many callers are waiting for [key] (a cancelled owner with none
     * else waiting has no reason to open a connection). */
    @Synchronized
    fun size(key: String): Int = waiting[key]?.size ?: 0
}

/**
 * Whether [url] is an absolute http(s) URL this fetcher can parse. Format
 * only: the host policy (configured origin or Deezer CDN, no userinfo, no
 * private IP) is [coverAllowed], applied before every disk read and every hop.
 */
internal fun coverIsHttp(url: String): Boolean {
    if (url.length > 2_048) return false
    return try {
        val uri = URI(url)
        val scheme = uri.scheme?.lowercase()
        (scheme == "http" || scheme == "https") && !uri.host.isNullOrEmpty()
    } catch (_: Exception) {
        false
    }
}

/**
 * A playlist/track cover as a fetchable URL: the catalogue answers a relative
 * `/api/cover/<id>` and the player's page names an absolute one (the CDN or the
 * same-origin route). Anything else — a blob, a javascript: URL — is dropped,
 * and the view keeps its grooves.
 */
internal fun resolveCoverUrl(baseUrl: String, raw: String?): String {
    val text = raw?.trim().orEmpty()
    if (text.isEmpty() || text == "null") return ""
    if (coverIsHttp(text)) return text
    if (!text.startsWith("/")) return ""
    val root = CarPlaylistSource.rootOf(baseUrl) ?: return ""
    return root + text
}

/** The only remote image hosts a cover may reach: Deezer's own CDN, HTTPS. */
private const val DEEZER_CDN_SUFFIX = ".dzcdn.net"

private val IPV4_LITERAL = Regex("""^\d{1,3}(\.\d{1,3}){3}$""")

/**
 * Whether a cover URL may be fetched at all — the car's FAST, local half of the
 * rule. Two sources only:
 *  - the configured server, through [isSameOrigin] (on the device that is
 *    `CarServer.sameOrigin`, the same origin definition as the session, the pin
 *    and every allow-list), whatever scheme that server uses;
 *  - Deezer's image CDN, over HTTPS only.
 *
 * It is not the authority: `nz-core`'s `cover_path` applies the same allow-list
 * again, so this predicate only decides whether the call is even worth making
 * (and it is what makes the rule readable and testable without a core).
 *
 * A URL carrying userinfo is refused BEFORE the same-origin test: a string
 * such as `https://our-host@evil.example/` reads as our host to the eye and
 * resolves to evil.example. Every other host, every IP literal in a private,
 * loopback or link-local range, and cleartext off the configured server are
 * refused.
 */
internal fun coverAllowed(url: String, isSameOrigin: (String) -> Boolean): Boolean {
    if (url.length > 2_048) return false
    val uri = try {
        URI(url)
    } catch (_: Exception) {
        return false
    }
    if (uri.userInfo != null) return false
    if (isSameOrigin(url)) return true
    return coverAllowedRemote(uri)
}

/** The remote half of the rule, without the configured origin. */
internal fun coverAllowedRemote(url: String): Boolean = try {
    coverAllowedRemote(URI(url))
} catch (_: Exception) {
    false
}

private fun coverAllowedRemote(uri: URI): Boolean {
    if (uri.userInfo != null) return false
    if (uri.scheme?.lowercase() != "https") return false
    val host = uri.host?.lowercase()?.trimEnd('.') ?: return false
    if (host.isEmpty()) return false
    if (isLocalOrPrivateHost(host)) return false
    // `*.dzcdn.net` — the apex is not an image host, and `dzcdn.net.evil.com`
    // does not end with the suffix.
    return host.endsWith(DEEZER_CDN_SUFFIX)
}

/**
 * Private/loopback/link-local IP literals (and `localhost`) never pass, even
 * if a future allow-list entry were an address. Names are not resolved here: a
 * hostname that is not allow-listed is refused without a DNS query, and every
 * allowed name is Deezer's own.
 */
private fun isLocalOrPrivateHost(host: String): Boolean {
    val bare = host.removePrefix("[").removeSuffix("]")
    if (bare == "localhost" || bare.endsWith(".localhost")) return true
    val literal = bare.contains(':') || IPV4_LITERAL.matches(bare)
    if (!literal) return false
    val address = try {
        InetAddress.getByName(bare)
    } catch (_: Exception) {
        return true // a would-be literal we cannot parse is refused, not resolved
    }
    if (address.isAnyLocalAddress || address.isLoopbackAddress ||
        address.isLinkLocalAddress || address.isSiteLocalAddress ||
        address.isMulticastAddress
    ) {
        return true
    }
    val bytes = address.address
    if (bytes.size == 4 && isPrivateIpv4(bytes, 0)) return true
    if (address is Inet6Address) {
        // fc00::/7, the IPv6 unique-local range (not covered by isSiteLocal).
        if (bytes.isNotEmpty() && (bytes[0].toInt() and 0xFE) == 0xFC) return true
        if (isIpv4Mapped(bytes)) return isPrivateIpv4(bytes, 12)
    }
    return false
}

private fun isIpv4Mapped(bytes: ByteArray): Boolean {
    if (bytes.size != 16) return false
    for (i in 0 until 10) if (bytes[i].toInt() != 0) return false
    return (bytes[10].toInt() and 0xFF) == 0xFF && (bytes[11].toInt() and 0xFF) == 0xFF
}

private fun isPrivateIpv4(bytes: ByteArray, at: Int): Boolean {
    if (bytes.size < at + 4) return true
    val a = bytes[at].toInt() and 0xFF
    val b = bytes[at + 1].toInt() and 0xFF
    return a == 0 || a == 10 || a == 127 ||
        (a == 100 && b in 64..127) || // CGNAT
        (a == 169 && b == 254) ||
        (a == 172 && b in 16..31) ||
        (a == 192 && b == 168)
}
