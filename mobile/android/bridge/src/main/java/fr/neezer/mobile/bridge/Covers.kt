package fr.neezer.mobile.bridge

import android.content.Context
import android.util.Log
import coil3.ImageLoader
import coil3.SingletonImageLoader
import coil3.decode.DataSource
import coil3.decode.ImageSource
import coil3.fetch.FetchResult
import coil3.fetch.Fetcher
import coil3.fetch.SourceFetchResult
import coil3.key.Keyer
import coil3.request.Options
import okio.FileSystem
import okio.Path.Companion.toPath
import uniffi.nz_ffi.Screens

/**
 * Covers, from Rust's file to Compose's pixels (docs/agents/mobile.md, *Caches*).
 *
 * **Rust fetches and caches the encoded file; Android decodes it.** Copying RGBA
 * across JNI would cost more than the platform decoder, so the boundary hands out
 * a PATH and Coil does what it is good at: downsampling to the view's size,
 * hardware bitmaps, one decode per screenful.
 *
 * Three rules this file keeps:
 *
 * - **Coil's own caches are OFF.** The file is already on disk, written and
 *   LRU-managed by `nz-cache` (300 MB); a second disk cache would double the
 *   bytes and the eviction, and Coil's network cache has nothing to fetch — our
 *   [Fetcher] is the only network path, and it goes through the core's lanes.
 * - **A cover is keyed by URL AND drawn size**, so the same sleeve in a 36 dp row
 *   and on a 512 dp page are two entries, and the file Rust returns is not
 *   re-decoded at the wrong size.
 * - **A refused URL never reaches the network.** `cover_path` enforces the CDN
 *   allow-list; this file cannot bypass it because it has no other path.
 */
class NzCoverRequest(
    /** The URL a row carries: our `/api/cover/<id>` or a `*.dzcdn.net` address. */
    val url: String,
    /** The size the view draws it at, in pixels. */
    val sizePx: Int,
)

/** The Coil cache key: the URL and the size it is drawn at. */
class NzCoverKeyer : Keyer<NzCoverRequest> {
    override fun key(data: NzCoverRequest, options: Options): String =
        "nz-cover:${data.sizePx}:${data.url}"
}

/** Asks the core for the cached file and hands Coil its path. */
class NzCoverFetcher(
    private val request: NzCoverRequest,
    private val screens: Screens,
) : Fetcher {
    override suspend fun fetch(): FetchResult {
        // `cover_path` is a suspend FFI call on Rust's runtime: a scrolled-away
        // cover cancels this coroutine, and Rust's future is dropped before it
        // writes anything.
        val path = screens.coverPath(request.url, request.sizePx.toUInt())
        return SourceFetchResult(
            // The file Rust wrote, on the app's own filesystem: `FileSystem.SYSTEM`
            // is okio's name for it (the remote path never names a server file).
            source = ImageSource(path.toPath(), FileSystem.SYSTEM),
            // The server keeps the sleeve's own format (JPEG, PNG, WebP), so
            // nothing is claimed about it: Coil sniffs the bytes.
            mimeType = null,
            // It came from this device's disk (possibly fetched a moment ago by
            // Rust). Coil's cache policies are off anyway.
            dataSource = DataSource.DISK,
        )
    }

    class Factory(private val screens: Screens) : Fetcher.Factory<NzCoverRequest> {
        override fun create(
            data: NzCoverRequest,
            options: Options,
            imageLoader: ImageLoader,
        ): Fetcher = NzCoverFetcher(data, screens)
    }
}

/**
 * Installs the one [ImageLoader] the app uses, at `Application.onCreate`, after
 * the core exists.
 *
 * `diskCache(null)` is the rule above; the MEMORY cache stays (it is what keeps
 * a screenful of decoded bitmaps off the decoder), and hardware bitmaps are
 * Coil's default on API >= 26 — which is this app's floor.
 */
object NeezerImages {
    private const val TAG = "NeezerImages"

    @Volatile
    private var installed = false

    fun install(context: Context, screens: Screens) {
        if (installed) return
        installed = true
        val appContext = context.applicationContext
        SingletonImageLoader.setSafe {
            ImageLoader.Builder(appContext)
                .components {
                    add(NzCoverKeyer())
                    add(NzCoverFetcher.Factory(screens))
                }
                // Rust already owns the file and its 300 MB LRU.
                .diskCache(null)
                .build()
        }
        Log.i(TAG, "cover loader installed (Rust files, platform decode)")
    }

    /** For tests and for the diagnostic screen. */
    val ready: Boolean
        get() = installed
}
