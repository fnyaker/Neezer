// :playback — `NzPlayer`, the Media3 facade over the Rust player
// (docs/agents/mobile.md, "MB3c": "`NzPlayer : SimpleBasePlayer(main looper)`:
// `getState()` built from `PlayerSnapshot` … the playlist exposed as a WINDOW
// (previous 1 + current + next 100 …), the position through a `PositionSupplier`
// reading `PlayerApi.position()` (no tick), artwork through a `BitmapLoader`";
// "the facade holds no logic").
package fr.neezer.mobile.playback

import android.os.Bundle
import androidx.annotation.OptIn
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackParameters
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.common.SimpleBasePlayer
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture
import java.util.concurrent.atomic.AtomicBoolean
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.PlayerListener
import uniffi.nz_ffi.PlayerSnapshot
import uniffi.nz_ffi.Repeat
import uniffi.nz_ffi.TrackRef
import uniffi.nz_ffi.TrackRow

/**
 * The player Media3 sees, and the ONLY thing it sees: every field of a state
 * comes from `PlayerApi`'s snapshot, every command goes back to
 * `PlayerApi.command`, and nothing here decides what plays next.
 *
 * Three rules make it cheap, and each is a number from the note:
 *
 * - **`getState()` never calls Rust's locks.** `SimpleBasePlayer` asks for the
 *   whole state on every interaction (a seek, a volume change, the notification
 *   updating), so the snapshot is pushed in by [`PlayerListener`] and cached in a
 *   volatile field; only the POSITION is read live, and it is `PlayerApi
 *   .position()` — atomics, no lock (MB3b: "`position()` reads atomics (the
 *   engine's own clock…) and no lock at all").
 * - **The playlist is a window of 102 rows** ([PlaybackWindow]), rebuilt only
 *   when the current title leaves it: 4 000 `MediaItemData` per state would cost
 *   a frame each time Media3 rebuilds its timeline.
 * - **The queue is copied off the main thread.** `onQueueChanged` fires on a Rust
 *   thread, which is where `PlayerApi.queue()` (`ListHandle`) is taken; the main
 *   looper only fetches the 102-row page.
 *
 * The main looper is not a taste: `SimpleBasePlayer` on any other looper crashed
 * the service when a controller connected (Media3 issue, MB3c "Pitfalls").
 */
@OptIn(UnstableApi::class)
class NzPlayer(private val api: PlayerApi) : SimpleBasePlayer(Looper.getMainLooper()), PlayerListener {

    /** The service's hook: locks, focus and the battery flag follow each state. */
    var onSnapshot: ((PlayerSnapshot) -> Unit)? = null

    /** The focus is lost for a while: Media3 draws a suppressed player, not a pause. */
    @Volatile
    var transientFocusLoss: Boolean = false

    private val main = Handler(Looper.getMainLooper())
    private val posted = AtomicBoolean(false)

    @Volatile
    private var snapshot: PlayerSnapshot = api.snapshot()

    /** Taken on the Rust thread that fired `onQueueChanged`, swapped in on main. */
    @Volatile
    private var freshQueue: uniffi.nz_ffi.ListHandle? = null

    private var queue: uniffi.nz_ffi.ListHandle? = api.queue()
    private var items: List<MediaItemData> = emptyList()
    private var keys: List<String> = emptyList()
    private var slice: PlaybackWindow.Slice = PlaybackWindow.Slice.EMPTY
    private var windowLen: Int = -1
    private var windowIndex: Int = 0

    init {
        api.subscribe(this)
    }

    // --- the state Media3 reads ------------------------------------------

    override fun getState(): State {
        val snap = snapshot
        ensureWindow(snap)
        val builder =
            State.Builder()
                .setAvailableCommands(COMMANDS)
                .setPlayWhenReady(
                    snap.state == PlayState.PLAYING || snap.state == PlayState.BUFFERING,
                    if (snap.state == PlayState.ENDED) {
                        Player.PLAY_WHEN_READY_CHANGE_REASON_END_OF_MEDIA_ITEM
                    } else {
                        Player.PLAY_WHEN_READY_CHANGE_REASON_USER_REQUEST
                    },
                )
                .setPlaybackState(playbackStateOf(snap.state))
                .setPlaybackSuppressionReason(
                    if (transientFocusLoss) {
                        Player.PLAYBACK_SUPPRESSION_REASON_TRANSIENT_AUDIO_FOCUS_LOSS
                    } else {
                        Player.PLAYBACK_SUPPRESSION_REASON_NONE
                    },
                )
                .setRepeatMode(repeatOf(snap.repeat))
                .setShuffleModeEnabled(snap.shuffle)
                .setIsLoading(snap.state == PlayState.BUFFERING)
                .setVolume(snap.volume.coerceIn(0f, 1f))
                .setAudioAttributes(AUDIO_ATTRIBUTES)
                .setSeekBackIncrementMs(SEEK_BACK_MS)
                .setSeekForwardIncrementMs(SEEK_FORWARD_MS)
                .setPlaylist(items)
        if (items.isNotEmpty()) {
            builder
                .setCurrentMediaItemIndex(windowIndex)
                .setContentPositionMs(PositionSupplier { positionMs() })
                .setContentBufferedPositionMs(PositionSupplier { bufferedMs() })
        }
        return builder.build()
    }

    // --- what Rust pushes ------------------------------------------------

    override fun onSnapshot(snapshot: PlayerSnapshot) {
        this.snapshot = snapshot
        invalidate()
    }

    override fun onQueueChanged(version: ULong) {
        // A fresh handle per change (MB3b's `queue()`): the copy of the rows
        // happens HERE, on the Rust thread, so a 4 000-row queue never costs the
        // main looper a frame.
        freshQueue = api.queue()
        invalidate()
    }

    override fun onFault(cause: String) {
        // The snapshot carries the cause (`fault`); a screen shows it.
        invalidate()
    }

    /**
     * One posted invalidation, whatever the burst: a skip that opens a stream
     * emits three snapshots (buffering, started, position) and Media3 only needs
     * to ask once.
     */
    private fun invalidate() {
        if (!posted.compareAndSet(false, true)) return
        main.post {
            posted.set(false)
            invalidateState()
            onSnapshot?.invoke(snapshot)
        }
    }

    // --- the window ------------------------------------------------------

    private fun ensureWindow(snap: PlayerSnapshot) {
        val fresh = freshQueue
        if (fresh != null && fresh !== queue) {
            queue = fresh
            freshQueue = null
            windowLen = -1
        }
        val len = snap.len.toInt()
        val index = snap.index?.toInt()
        if (
            items.isNotEmpty() &&
                windowLen == len &&
                index != null &&
                PlaybackWindow.stillShows(slice, len, index)
        ) {
            windowIndex = slice.windowOf(index)
            return
        }
        val next = PlaybackWindow.slice(len, index)
        val rows =
            if (next.count == 0) {
                emptyList()
            } else {
                queue?.window(next.start.toUInt(), next.count.toUInt()).orEmpty()
            }
        items = rows.mapIndexed { offset, row -> mediaItemData(row, next.start + offset) }
        keys = rows.map { row -> row.key }
        slice = next.copy(count = rows.size)
        windowLen = len
        windowIndex = index?.let { slice.windowOf(it) }?.coerceAtLeast(0) ?: 0
    }

    private fun mediaItemData(row: TrackRow, realIndex: Int): MediaItemData {
        val metadata = metadataOf(row)
        val item = MediaItem.Builder().setMediaId(row.key).setMediaMetadata(metadata).build()
        return MediaItemData.Builder("$realIndex:${row.key}")
            .setMediaItem(item)
            .setMediaMetadata(metadata)
            .setDurationUs(if (row.durationS == 0u) C.TIME_UNSET else row.durationS.toLong() * 1_000_000L)
            .setIsSeekable(true)
            .setIsDynamic(false)
            .build()
    }

    // --- the commands Media3 sends ---------------------------------------

    override fun handleSetPlayWhenReady(playWhenReady: Boolean): ListenableFuture<*> {
        send(if (playWhenReady) PlayerCommand.Play else PlayerCommand.Pause)
        return Futures.immediateVoidFuture()
    }

    override fun handlePrepare(): ListenableFuture<*> {
        send(PlayerCommand.Play)
        return Futures.immediateVoidFuture()
    }

    override fun handleStop(): ListenableFuture<*> {
        send(PlayerCommand.Pause)
        return Futures.immediateVoidFuture()
    }

    override fun handleRelease(): ListenableFuture<*> {
        // The service owns the player's lifetime: Media3 releasing its *session*
        // must not stop the engine (the notification is not the player).
        return Futures.immediateVoidFuture()
    }

    override fun handleSeek(
        mediaItemIndex: Int,
        positionMs: Long,
        seekCommand: Int,
    ): ListenableFuture<*> {
        val snap = snapshot
        val current = snap.index?.toInt() ?: return Futures.immediateVoidFuture()
        val target = slice.realOf(mediaItemIndex)
        val ms = if (positionMs == C.TIME_UNSET) 0L else positionMs
        when {
            // Same title: a plain seek.
            target == current && ms >= 0 -> send(PlayerCommand.Seek(ms / 1000.0))
            // The neighbours: the MACHINE's next/previous, so shuffle, repeat
            // and the fault policy are the app's and not Media3's.
            target == current + 1 && ms == 0L -> send(PlayerCommand.Next)
            target == current - 1 && ms == 0L -> send(PlayerCommand.Prev)
            else -> {
                val key = keys.getOrNull(mediaItemIndex) ?: return Futures.immediateVoidFuture()
                send(PlayerCommand.Jump(target.coerceAtLeast(0).toUInt(), key))
                if (ms > 0) send(PlayerCommand.Seek(ms / 1000.0))
            }
        }
        return Futures.immediateVoidFuture()
    }

    override fun handleSetMediaItems(
        mediaItems: MutableList<MediaItem>,
        startIndex: Int,
        startPositionMs: Long,
    ): ListenableFuture<*> {
        val tracks = mediaItems.map(::trackOf)
        if (tracks.isEmpty()) return Futures.immediateVoidFuture()
        send(
            PlayerCommand.PlayQueue(
                tracks = tracks,
                start = startIndex.coerceIn(0, tracks.size - 1).toUInt(),
                context = null,
            ),
        )
        if (startPositionMs > 0) send(PlayerCommand.Seek(startPositionMs / 1000.0))
        return Futures.immediateVoidFuture()
    }

    override fun handleAddMediaItems(
        index: Int,
        mediaItems: MutableList<MediaItem>,
    ): ListenableFuture<*> {
        if (mediaItems.isNotEmpty()) {
            // `PlayerCommand` has no POSITIONAL add: the queue appends, which is
            // what a controller's "add to queue" means (a screen's "play next"
            // goes through `PlayerApi` directly, MB10).
            send(PlayerCommand.AddToQueue(mediaItems.map(::trackOf)))
        }
        return Futures.immediateVoidFuture()
    }

    override fun handleMoveMediaItems(
        from: Int,
        to: Int,
        count: Int,
    ): ListenableFuture<*> {
        for ((source, target) in QueueEdit.moveRange(from, to, count)) {
            send(PlayerCommand.Move(source.toUInt(), target.toUInt()))
        }
        return Futures.immediateVoidFuture()
    }

    override fun handleRemoveMediaItems(from: Int, to: Int): ListenableFuture<*> {
        for (index in QueueEdit.removeRange(from, to)) {
            val key = keys.getOrNull(index) ?: continue
            send(PlayerCommand.Remove(slice.realOf(index).toUInt(), key))
        }
        return Futures.immediateVoidFuture()
    }

    override fun handleReplaceMediaItems(
        from: Int,
        to: Int,
        mediaItems: MutableList<MediaItem>,
    ): ListenableFuture<*> {
        val removed = QueueEdit.removeRange(from, to).size
        handleRemoveMediaItems(from, to)
        if (mediaItems.isEmpty()) return Futures.immediateVoidFuture()
        send(PlayerCommand.AddToQueue(mediaItems.map(::trackOf)))
        // The appended titles land at the end of the queue; move each into the
        // slot the range occupied (the queue length after the removal is what
        // the snapshot said, minus what this call removed).
        val len = snapshot.len.toInt() - removed
        mediaItems.indices.forEach { offset ->
            val appended = len + offset
            val target = slice.realOf(from) + offset
            send(PlayerCommand.Move(appended.toUInt(), target.coerceAtLeast(0).toUInt()))
        }
        return Futures.immediateVoidFuture()
    }

    override fun handleSetRepeatMode(repeatMode: Int): ListenableFuture<*> {
        val target = repeatOfInt(repeatMode)
        repeat(QueueEdit.repeatCycles(snapshot.repeat, target)) { send(PlayerCommand.CycleRepeat) }
        return Futures.immediateVoidFuture()
    }

    override fun handleSetShuffleModeEnabled(shuffleModeEnabled: Boolean): ListenableFuture<*> {
        if (shuffleModeEnabled != snapshot.shuffle) send(PlayerCommand.SetShuffle(shuffleModeEnabled))
        return Futures.immediateVoidFuture()
    }

    override fun handleSetVolume(volume: Float): ListenableFuture<*> {
        send(PlayerCommand.SetVolume(volume.coerceIn(0f, 1f)))
        return Futures.immediateVoidFuture()
    }

    @Suppress("DEPRECATION") // Media3 deprecates the setter, not the handler a controller still calls.
    override fun handleSetPlaybackParameters(playbackParameters: PlaybackParameters): ListenableFuture<*> {
        val audio = api.audio()
        if (audio.speed != playbackParameters.speed) {
            api.setAudio(audio.copy(speed = playbackParameters.speed))
        }
        return Futures.immediateVoidFuture()
    }

    // --- helpers ---------------------------------------------------------

    /** Reads the snapshot the facade is showing (the service's state loop). */
    fun currentSnapshot(): PlayerSnapshot = snapshot

    private fun send(command: PlayerCommand) {
        // The tap instant travels with the command: `output_stats()` reports
        // tap → first audible frame from it (MB3b's acceptance, MB3c's proof
        // that a controller's play reaches the sound).
        api.command(command, System.nanoTime().toULong())
    }

    private fun positionMs(): Long {
        val position = api.position()
        return (position.positionS * 1000.0).toLong().coerceAtLeast(0)
    }

    private fun bufferedMs(): Long {
        val position = api.position()
        return (position.bufferedS * 1000.0).toLong().coerceAtLeast(positionMs())
    }

    private fun trackOf(item: MediaItem): TrackRef {
        val metadata = item.mediaMetadata
        val extras = metadata.extras
        return TrackRef(
            key = item.mediaId,
            id = extras?.getString(EXTRA_ID) ?: item.mediaId,
            title = metadata.title?.toString() ?: item.mediaId,
            artists = metadata.artist?.toString().orEmpty(),
            album = metadata.albumTitle?.toString().orEmpty(),
            albumId = extras?.getString(EXTRA_ALBUM_ID),
            artistId = extras?.getString(EXTRA_ARTIST_ID),
            cover = CoverArt.urlOf(metadata.artworkUri),
            durationS = (extras?.getInt(EXTRA_DURATION_S) ?: 0).coerceAtLeast(0).toUInt(),
            episode = extras?.getBoolean(EXTRA_EPISODE) ?: false,
            liked = extras?.getBoolean(EXTRA_LIKED) ?: false,
        )
    }

    private fun metadataOf(row: TrackRow): MediaMetadata =
        MediaMetadata.Builder()
            .setTitle(row.title)
            .setArtist(row.artists)
            .setAlbumTitle(row.album)
            .setIsBrowsable(false)
            .setIsPlayable(true)
            .setArtworkUri(CoverArt.uri(row.cover))
            .setExtras(
                Bundle().apply {
                    putString(EXTRA_ID, row.id)
                    putString(EXTRA_ALBUM_ID, row.albumId)
                    putInt(EXTRA_DURATION_S, row.durationS.toInt())
                    putBoolean(EXTRA_LIKED, row.liked)
                },
            )
            .build()

    private fun playbackStateOf(state: PlayState): Int =
        when (state) {
            PlayState.PLAYING -> Player.STATE_READY
            PlayState.BUFFERING -> Player.STATE_BUFFERING
            PlayState.PAUSED -> Player.STATE_READY
            PlayState.ENDED -> Player.STATE_ENDED
            PlayState.STOPPED -> Player.STATE_IDLE
        }

    private fun repeatOf(repeat: Repeat): Int =
        when (repeat) {
            Repeat.OFF -> Player.REPEAT_MODE_OFF
            Repeat.ALL -> Player.REPEAT_MODE_ALL
            Repeat.ONE -> Player.REPEAT_MODE_ONE
        }

    private fun repeatOfInt(repeatMode: Int): Repeat =
        when (repeatMode) {
            Player.REPEAT_MODE_ALL -> Repeat.ALL
            Player.REPEAT_MODE_ONE -> Repeat.ONE
            else -> Repeat.OFF
        }

    companion object {
        private const val TAG = "NzPlayer"

        /** A notification's back/forward steps (the SPA's own buttons). */
        const val SEEK_BACK_MS = 15_000L
        const val SEEK_FORWARD_MS = 30_000L

               // The extras a controller hands back: what `PlayerCommand.PlayQueue`
        // needs to rebuild a `TrackRef` (`nz-core` never invents a row).
        internal const val EXTRA_ID = "nz.id"
        internal const val EXTRA_ALBUM_ID = "nz.albumId"
        internal const val EXTRA_ARTIST_ID = "nz.artistId"
        internal const val EXTRA_DURATION_S = "nz.durationS"
        internal const val EXTRA_EPISODE = "nz.episode"
        internal const val EXTRA_LIKED = "nz.liked"

        private val AUDIO_ATTRIBUTES: AudioAttributes =
            AudioAttributes.Builder()
                .setUsage(C.USAGE_MEDIA)
                .setContentType(C.AUDIO_CONTENT_TYPE_MUSIC)
                .build()

        /**
         * What the facade really answers. Only what is handled is advertised:
         * Media3 offers a controller exactly these, and a command we advertised
         * but did not implement is a `UnsupportedOperationException` on the
         * notification's own button.
         */
        val COMMANDS: Player.Commands =
            Player.Commands.Builder()
                .addAll(
                    Player.COMMAND_PLAY_PAUSE,
                    Player.COMMAND_PREPARE,
                    Player.COMMAND_STOP,
                    Player.COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM,
                    Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM,
                    Player.COMMAND_SEEK_TO_NEXT,
                    Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM,
                    Player.COMMAND_SEEK_TO_PREVIOUS,
                    Player.COMMAND_SEEK_TO_DEFAULT_POSITION,
                    Player.COMMAND_SEEK_TO_MEDIA_ITEM,
                    Player.COMMAND_GET_CURRENT_MEDIA_ITEM,
                    Player.COMMAND_GET_TIMELINE,
                    Player.COMMAND_GET_METADATA,
                    Player.COMMAND_SET_MEDIA_ITEM,
                    Player.COMMAND_CHANGE_MEDIA_ITEMS,
                    Player.COMMAND_SET_REPEAT_MODE,
                    Player.COMMAND_SET_SHUFFLE_MODE,
                    Player.COMMAND_SET_VOLUME,
                    Player.COMMAND_GET_VOLUME,
                    Player.COMMAND_GET_AUDIO_ATTRIBUTES,
                    Player.COMMAND_SET_SPEED_AND_PITCH,
                    Player.COMMAND_RELEASE,
                )
                .build()
    }
}
