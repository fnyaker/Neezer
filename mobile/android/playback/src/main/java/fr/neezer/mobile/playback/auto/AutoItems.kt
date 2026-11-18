// :playback — one row of the Auto tree as the Media3 object the session, the
// notification and the head unit exchange.
//
// The conversion exists for one reason: a `MediaItem` crosses a Binder, so what
// the player needs to rebuild its queue must travel INSIDE it. `NzPlayer` reads
// exactly these extras back (`NzPlayer.trackOf`), which is what makes a title
// tapped in a car play through the same `PlayQueue` as a title tapped on the
// phone — no second play path, no node id mistaken for a track key.
package fr.neezer.mobile.playback.auto

import android.os.Bundle
import androidx.annotation.OptIn
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.util.UnstableApi
import fr.neezer.mobile.playback.CoverArt
import fr.neezer.mobile.playback.NzPlayer
import uniffi.nz_ffi.AutoItem
import uniffi.nz_ffi.AutoKind

@OptIn(UnstableApi::class)
object AutoItems {
    /** The root a controller connects to: `Neezer`, browsable, never played. */
    fun root(): MediaItem =
        MediaItem.Builder()
            .setMediaId(AutoNode.ROOT)
            .setMediaMetadata(
                MediaMetadata.Builder()
                    .setTitle(AutoNode.ROOT_TITLE)
                    .setIsBrowsable(true)
                    .setIsPlayable(false)
                    .build(),
            )
            .build()

    /**
     * One Rust row, as Media3 sees it: the title, the line under it, the cover
     * (`CoverArt`'s synthetic URI, decoded by `NzBitmapLoader` from the file
     * Rust cached) and — for a title — the extras the player needs.
     */
    fun mediaItem(item: AutoItem): MediaItem {
        val metadata =
            MediaMetadata.Builder()
                .setTitle(item.title)
                .setArtist(item.subtitle)
                .setAlbumTitle(item.album.takeIf { it.isNotEmpty() })
                .setIsBrowsable(item.browsable)
                .setIsPlayable(item.playable)
                .setArtworkUri(CoverArt.uri(item.cover))
                .setExtras(extrasOf(item))
                .build()
        return MediaItem.Builder()
            .setMediaId(item.mediaId)
            .setMediaMetadata(metadata)
            .build()
    }

    /**
     * What a title carries so the queue can be rebuilt: its id, its album and
     * its duration — the fields `NzPlayer.trackOf` reads, and nothing else (a
     * row that is not a title carries an empty bundle).
     */
    private fun extrasOf(item: AutoItem): Bundle =
        Bundle().apply {
            if (item.kind != AutoKind.TRACK) return@apply
            putString(NzPlayer.EXTRA_ID, item.id)
            item.albumId?.let { putString(NzPlayer.EXTRA_ALBUM_ID, it) }
            putInt(NzPlayer.EXTRA_DURATION_S, item.durationS.toInt())
            putBoolean(NzPlayer.EXTRA_LIKED, item.liked)
            putBoolean(NzPlayer.EXTRA_EPISODE, false)
        }
}
