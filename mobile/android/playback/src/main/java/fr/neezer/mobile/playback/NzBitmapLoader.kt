// :playback — the artwork Media3 asks for, read from the file Rust already
// cached (docs/agents/mobile.md, "MB3c": "artwork through a `BitmapLoader` using
// `cover_path`"; "Covers": Rust fetches the encoded file, Android decodes it).
package fr.neezer.mobile.playback

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import androidx.media3.common.MediaMetadata
import androidx.media3.common.util.BitmapLoader
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture
import com.google.common.util.concurrent.MoreExecutors
import com.google.common.util.concurrent.SettableFuture
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import uniffi.nz_ffi.Screens

/**
 * How a cover travels from a queue row to Media3: a synthetic URI, never a
 * network address Media3 could fetch itself.
 *
 * `MediaMetadata.artworkUri` crosses the Binder to the notification, the lock
 * screen and Android Auto, so it must be a value that means the same thing in
 * another process. Ours names the COVER, and [NzBitmapLoader] turns it into the
 * file `Screens.cover_path` caches — the request goes on rule 1's lanes inside
 * Rust, never through Media3's own network stack.
 */
object CoverArt {
    const val SCHEME = "neezer-cover"

    /** The URI a `MediaMetadata` carries, or `null` when the row has no cover. */
    fun uri(cover: String?): Uri? =
        if (cover.isNullOrBlank()) null else Uri.parse("$SCHEME:${Uri.encode(cover)}")

    /** The cover URL behind a [uri], or `null` for anything we did not build. */
    fun urlOf(uri: Uri?): String? {
        if (uri == null || uri.scheme != SCHEME) return null
        val encoded = uri.encodedSchemeSpecificPart ?: return null
        if (encoded.isEmpty()) return null
        return Uri.decode(encoded)
    }
}

/**
 * Decodes the cover file Rust cached, at the size a notification and a lock
 * screen draw (512 px: the large icon is 64 dp at mdpi, ×3 on a dense phone,
 * with room for the car's larger tile).
 *
 * Why a [BitmapLoader] and not Coil: Media3 calls this interface itself for the
 * notification, the lock screen and Auto, from its own threads, and wants a
 * `ListenableFuture`. Why the work runs on `Dispatchers.IO`: `cover_path` may
 * FETCH on first use (a lane, up to a second), and `getState()` must never wait
 * for that.
 */
class NzBitmapLoader(
    private val screens: Screens?,
    private val artworkPx: Int = ARTWORK_PX,
) : BitmapLoader {

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    override fun supportsMimeType(mimeType: String): Boolean = mimeType.startsWith("image/")

    override fun decodeBitmap(data: ByteArray): ListenableFuture<Bitmap> =
        Futures.immediateFuture(BitmapFactory.decodeByteArray(data, 0, data.size, sampling(data.size)))

    override fun loadBitmap(uri: Uri): ListenableFuture<Bitmap> {
        val url = CoverArt.urlOf(uri) ?: return Futures.immediateFuture(null)
        val client = screens ?: return Futures.immediateFuture(null)
        val answer = SettableFuture.create<Bitmap>()
        val job = scope.launch {
            val bitmap = try {
                decodeFile(File(client.coverPath(url, artworkPx.toUInt())))
            } catch (error: Exception) {
                // A cover is never a playback failure: the notification keeps
                // the groove, and the next state asks again (rule 3's "what was
                // fetched once is kept" is on Rust's side, not here).
                null
            }
            answer.set(bitmap)
        }
        // A controller that went away takes its decode with it (the UniFFI
        // future behind `cover_path` is dropped, so a cancelled fetch never
        // reaches the network).
        answer.addListener({ if (answer.isCancelled) job.cancel() }, MoreExecutors.directExecutor())
        return answer
    }

    override fun loadBitmapFromMetadata(metadata: MediaMetadata): ListenableFuture<Bitmap> {
        val uri = metadata.artworkUri ?: return Futures.immediateFuture(null)
        return loadBitmap(uri)
    }

    private fun decodeFile(file: File): Bitmap? {
        if (!file.isFile) return null
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(file.path, bounds)
        if (bounds.outWidth <= 0) return null
        var sample = 1
        while (bounds.outWidth / (sample * 2) >= artworkPx) sample *= 2
        return BitmapFactory.decodeFile(
            file.path,
            BitmapFactory.Options().apply { inSampleSize = sample },
        )
    }

    private fun sampling(bytes: Int): BitmapFactory.Options =
        BitmapFactory.Options().apply {
            // A byte array has no bounds pass; Media3 hands small covers here.
            inSampleSize = if (bytes > 4 * 1024 * 1024) 4 else 1
        }

    companion object {
        /** What a notification, a lock screen and a car tile draw, with margin. */
        const val ARTWORK_PX = 512
    }
}
