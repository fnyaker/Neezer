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
import android.webkit.CookieManager
import java.io.File
import java.io.IOException
import java.security.MessageDigest
import java.util.concurrent.Executors
import kotlin.math.max
import kotlin.math.min

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
        val target = if (url.startsWith("/")) CarCovers.resolve(Prefs(context).baseUrl(), url) else url
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
 * The car's own cover loader: one daemon thread, an in-memory LRU, a bounded
 * disk cache under `filesDir/car-covers`, and NO new dependency.
 *
 * The rules that keep it a decoration:
 *  - the URL must be http(s); a `blob:` from the page is not fetchable;
 *  - the cookie is read from the WebView's own [CookieManager] and only the
 *    configured origin receives it ([Net] enforces that rule again);
 *  - one request per URL at a time: MINI, FULL and ULTRA ask for the same
 *    track and share one transfer ([CoverFanout]);
 *  - the encoded body is capped ([MAX_ENCODED]) and the decode is bounded by
 *    the requested size (BitmapFactory.inSampleSize), so a hostile 20 000 px
 *    image cannot become heap;
 *  - a failed URL is remembered for a minute, so a repaint does not retry it;
 *  - the disk addition is public only once whole (temp file + rename), and the
 *    directory is trimmed to [DISK_BUDGET] at most once a minute.
 *
 * Nothing here blocks the caller: [load] returns at once, and the bitmap comes
 * back on the main thread.
 */
internal object CarCovers {
    private const val MAX_ENCODED = 4 * 1024 * 1024
    private const val DISK_BUDGET = 24L * 1024 * 1024
    private const val DISK_MAX_FILES = 300
    private const val TRIM_EVERY_MS = 60_000L
    private const val FAIL_TTL_MS = 60_000L
    private const val CONNECT_TIMEOUT = 4_000
    private const val READ_TIMEOUT = 6_000
    private const val MAX_CACHED_URL = 2_048
    const val DIR_NAME = "car-covers"

    private val io = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "car-covers").apply { isDaemon = true }
    }
    private val main = Handler(Looper.getMainLooper())

    private val memory = object : LruCache<String, Bitmap>(memoryBudget()) {
        override fun sizeOf(key: String, value: Bitmap): Int = value.allocationByteCount
    }

    /** URL -> elapsedRealtime after which it may be tried again. */
    private val failed = object : LruCache<String, Long>(256) {}

    private val inflight = CoverFanout<Bitmap?>()
    private var lastTrim = 0L

    /** A request in flight; [cancel] makes a late answer a no-op. */
    class Request internal constructor(private val callback: (Bitmap?) -> Unit) {
        @Volatile private var cancelled = false

        fun cancel() {
            cancelled = true
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
        val request = Request(callback)
        if (!inflight.join(clean, request::deliver)) return request
        val app = context.applicationContext
        io.execute { fetch(app, clean, maxPx, request) }
        return request
    }

    /** Resolve the server's relative cover path against the configured root. */
    fun resolve(baseUrl: String, raw: String?): String = resolveCoverUrl(baseUrl, raw)

    private fun post(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block() else main.post(Runnable { block() })
    }

    private fun fetch(context: Context, url: String, maxPx: Int, request: Request) {
        // A view that detached before its turn (a settings screen closed, a
        // tile scrolled away) must not spend the one cover thread on a network
        // round-trip nobody will draw.
        if (request.isCancelled() && inflight.size(url) <= 1) {
            deliver(url, null)
            return
        }
        var bitmap: Bitmap? = null
        try {
            val dir = File(context.filesDir, DIR_NAME)
            val file = File(dir, coverCacheKey(url) + ".img")
            // Decode BEFORE publishing to disk: a login page or a truncated
            // body is not a cover, and writing it would poison the entry for
            // every later read. A stored file that no longer decodes is
            // dropped and fetched again.
            val stored = readDisk(file)
            bitmap = decode(stored, maxPx)
            if (bitmap == null) {
                if (stored != null) file.delete()
                val bytes = download(context, url)
                if (bytes != null) {
                    bitmap = decode(bytes, maxPx)
                    if (bitmap != null) writeDisk(dir, file, bytes)
                }
            }
            if (bitmap != null) {
                memory.put(url, bitmap)
                failed.remove(url)
                trimDisk(dir)
            }
        } catch (_: Exception) {
            bitmap = null
        }
        if (bitmap == null) {
            try {
                failed.put(url, SystemClock.elapsedRealtime() + FAIL_TTL_MS)
            } catch (_: Exception) {
            }
        }
        deliver(url, bitmap)
    }

    private fun deliver(url: String, bitmap: Bitmap?) {
        val callbacks = inflight.finish(url)
        if (callbacks.isEmpty()) return
        val run = Runnable { for (callback in callbacks) callback(bitmap) }
        if (Looper.myLooper() == Looper.getMainLooper()) run.run() else main.post(run)
    }

    private fun download(context: Context, url: String): ByteArray? {
        val prefs = Prefs(context)
        // Only our own server gets the session cookie; Net checks the origin
        // again on every redirect hop.
        val cookie = if (prefs.isServerUrl(url)) {
            try {
                CookieManager.getInstance().getCookie(url)
            } catch (_: Exception) {
                null
            }
        } else {
            null
        }
        val conn = try {
            Net.get(prefs, url, cookie, CONNECT_TIMEOUT, READ_TIMEOUT)
        } catch (_: IOException) {
            return null
        }
        return try {
            conn.inputStream.use { readCapped(it) }
        } catch (_: Exception) {
            null
        } finally {
            conn.disconnect()
        }
    }

    private fun readCapped(input: java.io.InputStream): ByteArray? {
        val out = java.io.ByteArrayOutputStream()
        val buffer = ByteArray(16 * 1024)
        var total = 0
        while (true) {
            val read = input.read(buffer)
            if (read < 0) break
            total += read
            if (total > MAX_ENCODED) return null
            out.write(buffer, 0, read)
        }
        return out.toByteArray()
    }

    private fun readDisk(file: File): ByteArray? {
        if (!file.isFile) return null
        val size = file.length()
        if (size <= 0L || size > MAX_ENCODED) return null
        return try {
            file.readBytes()
        } catch (_: Exception) {
            null
        }
    }

    private fun writeDisk(dir: File, file: File, bytes: ByteArray) {
        try {
            if (!dir.isDirectory && !dir.mkdirs()) return
            val temp = File.createTempFile("cover", ".tmp", dir)
            try {
                temp.writeBytes(bytes)
                if (temp.renameTo(file)) return
                // Some file systems refuse to replace; remove and retry once.
                file.delete()
                if (!temp.renameTo(file)) temp.delete()
            } catch (_: Exception) {
                temp.delete()
            }
        } catch (_: Exception) {
        }
    }

    private fun trimDisk(dir: File) {
        val now = SystemClock.elapsedRealtime()
        synchronized(this) {
            if (lastTrim != 0L && now - lastTrim in 0 until TRIM_EVERY_MS) return
            lastTrim = now
        }
        try {
            val wall = System.currentTimeMillis()
            val all = dir.listFiles() ?: return
            // A crash between the write and the rename leaves a .tmp: it is
            // not a published cover, and it is the only thing the bound below
            // does not count.
            for (f in all) {
                if (f.isFile && f.name.endsWith(".tmp") &&
                    wall - f.lastModified() > 24L * 60 * 60 * 1000
                ) {
                    f.delete()
                }
            }
            val files = all.filter { f -> f.isFile && f.name.endsWith(".img") }
            val entries = files.mapNotNull { f ->
                CoverDiskEntry(f.name, f.length(), f.lastModified())
            }
            for (name in coverEvictions(entries, DISK_BUDGET, DISK_MAX_FILES)) {
                File(dir, name).delete()
            }
        } catch (_: Exception) {
        }
    }

    private fun decode(bytes: ByteArray?, maxPx: Int): Bitmap? {
        if (bytes == null || bytes.isEmpty()) return null
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        val w = bounds.outWidth
        val h = bounds.outHeight
        if (w <= 0 || h <= 0) return null
        val target = maxPx.coerceIn(48, 512)
        var sample = 1
        while (w / (sample * 2) >= target || h / (sample * 2) >= target) sample *= 2
        val options = BitmapFactory.Options().apply { inSampleSize = sample }
        return try {
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
        } catch (_: Exception) {
            null
        }
    }

    private fun memoryBudget(): Int =
        (Runtime.getRuntime().maxMemory() / 16L).coerceIn(3L * 1024 * 1024, 8L * 1024 * 1024).toInt()
}

/** A disk entry, free of Android types so its eviction is testable on the JVM. */
internal data class CoverDiskEntry(val name: String, val size: Long, val modified: Long)

/**
 * Which cached files to delete so the directory stays under [budget] bytes and
 * [maxFiles] entries: the oldest first, never the one just written.
 */
internal fun coverEvictions(
    entries: List<CoverDiskEntry>,
    budget: Long,
    maxFiles: Int,
): List<String> {
    val ordered = entries.sortedWith(compareBy({ it.modified }, { it.name }))
    var total = entries.sumOf { it.size }
    var count = entries.size
    val evicted = ArrayList<String>()
    for (entry in ordered) {
        if (total <= budget && count <= maxFiles) break
        evicted.add(entry.name)
        total -= entry.size
        count--
    }
    return evicted
}

/** The disk name of a URL: a hash, never a path built from the URL. */
internal fun coverCacheKey(url: String): String {
    val digest = MessageDigest.getInstance("SHA-256").digest(url.toByteArray(Charsets.UTF_8))
    val hex = StringBuilder(64)
    for (byte in digest) hex.append(String.format(java.util.Locale.ROOT, "%02x", byte.toInt() and 0xFF))
    return hex.toString()
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

/** Whether [url] is an absolute http(s) URL the native fetcher may open. */
internal fun coverIsHttp(url: String): Boolean {
    if (url.length > 2_048) return false
    return try {
        val uri = java.net.URI(url)
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
