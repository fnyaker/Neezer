// :playback — the app's media service (docs/agents/mobile.md, "MB3c"): Media3
// owns the system surfaces, the player stays in Rust.
package fr.neezer.mobile.playback

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.util.Log
import androidx.annotation.OptIn
import androidx.media3.common.MediaItem
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.DefaultMediaNotificationProvider
import androidx.media3.session.LibraryResult
import androidx.media3.session.MediaLibraryService
import androidx.media3.session.MediaSession
import androidx.media3.session.SessionError
import com.google.common.collect.ImmutableList
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture
import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.OutputHandle
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerSnapshot
import uniffi.nz_ffi.nzErrorText
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.playback.auto.AutoLibrary

/**
 * The one foreground service while music plays.
 *
 * What it owns, and why each piece is here and not in the player:
 *
 * - **the [NzPlayer] facade and its [MediaLibrarySession]** — the notification,
 *   the lock screen, headset buttons, Bluetooth AVRCP and Android Auto come from
 *   Media3 for free once a session is over a player;
 * - **the device output** ([PlayerApi.startOutput], MB3c's wiring of MB3a's
 *   `nz-droid-audio`): without it the engine's clock never advances, so this is
 *   what makes "Lecture" a sound rather than a flag;
 * - **audio focus** ([FocusController]) — one request, ducking in the engine;
 * - **the wake and Wi-Fi locks** ([PlaybackLocks]) — held only while the state
 *   needs them (rule 1: a background reader must not keep the radio awake);
 * - **the screen state and becoming-noisy** ([PlaybackReceivers]);
 * - **the battery-exemption flag** ([BatteryExemption]) — written at play,
 *   cleared at a clean pause, offered once.
 *
 * Nothing here decides what plays next: every command goes to
 * `PlayerApi.command`, and every field of the state comes back from its
 * snapshot.
 */
@OptIn(UnstableApi::class)
class NzMediaService : MediaLibraryService() {

    private var api: PlayerApi? = null
    private var nzPlayer: NzPlayer? = null
    private var session: MediaLibrarySession? = null
    private var output: OutputHandle? = null
    private var focus: FocusController? = null
    private var locks: PlaybackLocks? = null
    private var receivers: PlaybackReceivers? = null
    private var battery: BatteryExemption? = null

    override fun onCreate() {
        super.onCreate()
        setMediaNotificationProvider(notificationProvider())
        val exemption = BatteryExemption(SharedPreferencesFlags(this))
        // Before the player exists: a flag left behind by a killed process is
        // the ONE reason the app ever offers the exemption.
        if (exemption.processStarted()) {
            Log.i(TAG, "la dernière lecture a été interrompue par le système")
        }
        battery = exemption
        if (buildPlayer()) return
        Log.w(TAG, "noyau indisponible : aucune session média")
    }

    /** The session Media3, the notification and the car ask for. */
    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaLibrarySession? =
        session

    /**
     * A swiped-away task is not a stop: the engine keeps playing behind a locked
     * screen and the notification is the only door out. Media3's own answer
     * already keeps a player that is playing; nothing is overridden.
     */
    override fun onTaskRemoved(rootIntent: Intent?) {
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        receivers?.stop()
        receivers = null
        focus?.abandon()
        focus = null
        locks?.release()
        locks = null
        // The device output is NOT stopped here: it belongs to the PROCESS (the
        // engine's sink is handed out once, and Android recreates a media service
        // whenever it likes). A recreated service adopts the same stream through
        // `startOutput`'s idempotence; stopping it here would leave the queue
        // playing into nothing. Neither the Kotlin pull on API 26: it is the same
        // session, held below by the companion.
        output = null
        session?.release()
        session = null
        // A destroyed service is a clean stop (a system kill never calls this).
        battery?.noteCleanPause()
        battery = null
        nzPlayer = null
        api = null
        super.onDestroy()
    }

    // --- building ---------------------------------------------------------

    private fun buildPlayer(): Boolean {
        val core = NzBridge.core ?: return false
        val playerApi = core.player()
        startOutput(playerApi)

        val player = NzPlayer(playerApi)
        player.onSnapshot = ::follow
        nzPlayer = player
        api = playerApi

        val built =
            MediaLibrarySession.Builder(this, player, LibraryCallback())
                // Covers: Rust caches the file, Android decodes it (MB2's rule).
                .setBitmapLoader(NzBitmapLoader(core.screens()))
                .setSessionActivity(launchIntent())
                .build()
        session = built

        // MB25: the car's browse tree, installed once the session exists (it is
        // what lets a node say "j'ai plus" while a head unit looks at it). The
        // tree itself is Rust's; `AutoLibrary` is the Media3 glue.
        AutoLibrary.register(core.screens(), playerApi, built)

        focus = FocusController(this, wasPlaying = ::playing) { action -> applyFocus(action) }
        locks = PlaybackLocks(this)
        receivers =
            PlaybackReceivers(
                this,
                onScreen = { on -> playerApi.setScreenOn(on) },
                onBecomingNoisy = ::pauseForNoise,
            ).also { it.start() }
        return true
    }

    /**
     * The device output, ONCE (the engine's sink is handed out once). The
     * failure is kept for Aide: a phone whose HAL refused would otherwise show
     * "Lecture" over silence with no cause anywhere (rule 3).
     */
    private fun startOutput(playerApi: PlayerApi) {
        val handle =
            try {
                playerApi.startOutput(Build.VERSION.SDK_INT.toUInt())
            } catch (error: NzException) {
                lastOutputFailure = nzErrorText(error)
                Log.w(TAG, "sortie audio : $lastOutputFailure")
                return
            }
        output = handle
        outputHandle = handle
        if (handle.needsAudioTrack()) {
            // API 26: AAudio can crash after close on Android 8.0, so one Kotlin
            // thread pulls into an AudioTrack (MB3a). ONE pull per process, like
            // the session it drains: a recreated service keeps the running one.
            if (pull == null) {
                pull =
                    AudioTrackPull(
                        handle.handle(),
                        handle.sampleRate().toInt(),
                        handle.bufferMs().toInt(),
                    ) { cause ->
                        lastOutputFailure = cause
                        Log.w(TAG, "AudioTrack : $cause")
                    }.also { it.start(this) }
            }
        }
    }

    private fun notificationProvider(): DefaultMediaNotificationProvider {
        val provider =
            DefaultMediaNotificationProvider.Builder(this)
                .setChannelId(CHANNEL_ID)
                .setChannelName(R.string.nz_playback_channel)
                .setNotificationId(NOTIFICATION_ID)
                .build()
        provider.setSmallIcon(smallIcon())
        return provider
    }

    /**
     * `ic_stat_neezer` lives in `:app` (lot LG1): a library module cannot read
     * another module's `R`, so the drawable is resolved by NAME from the merged
     * APK. `playback/src/main/res/raw/keep.xml` keeps the resource shrinker from
     * removing what only a name points at.
     */
    private fun smallIcon(): Int {
        val resolved = resources.getIdentifier("ic_stat_neezer", "drawable", packageName)
        if (resolved != 0) return resolved
        Log.i(TAG, "ic_stat_neezer introuvable: icône de repli")
        return android.R.drawable.ic_media_play
    }

    private fun launchIntent(): PendingIntent {
        val intent =
            packageManager.getLaunchIntentForPackage(packageName)
                ?: Intent(Intent.ACTION_MAIN).setPackage(packageName)
        return PendingIntent.getActivity(
            this,
            0,
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
    }

    // --- following the Rust state -----------------------------------------

    /** Called on the main looper, once per coalesced snapshot. */
    private fun follow(snapshot: PlayerSnapshot) {
        val playing = snapshot.state == PlayState.PLAYING || snapshot.state == PlayState.BUFFERING
        if (playing) {
            // The flag is what proves a system kill (it never calls onDestroy).
            battery?.notePlaying()
            focus?.request()
        } else {
            if (snapshot.state != PlayState.BUFFERING) battery?.noteCleanPause()
            if (focus?.pausedForFocus != true) focus?.abandon()
        }
        locks?.apply(snapshot.state, snapshot.streaming)
        nzPlayer?.transientFocusLoss = focus?.pausedForFocus == true
    }

    private fun playing(): Boolean {
        val state = nzPlayer?.currentSnapshot()?.state ?: return false
        return state == PlayState.PLAYING || state == PlayState.BUFFERING
    }

    private fun applyFocus(action: FocusAction) {
        val player = nzPlayer ?: return
        action.play?.let { play -> if (play) player.play() else player.pause() }
        action.ducked?.let { ducked -> api?.setDucked(ducked) }
    }

    private fun pauseForNoise() {
        nzPlayer?.pause()
        focus?.abandon()
    }

    /**
     * The browse tree, delegated to [LibraryTrees] (MB25 registers the real one;
     * today the root is the only node).
     */
    private inner class LibraryCallback : MediaLibrarySession.Callback {
        override fun onGetLibraryRoot(
            session: MediaLibrarySession,
            browser: MediaSession.ControllerInfo,
            params: LibraryParams?,
        ): ListenableFuture<LibraryResult<MediaItem>> =
            Futures.immediateFuture(LibraryResult.ofItem(LibraryTrees.current.root(), params))

        override fun onGetChildren(
            session: MediaLibrarySession,
            browser: MediaSession.ControllerInfo,
            parentId: String,
            page: Int,
            pageSize: Int,
            params: LibraryParams?,
        ): ListenableFuture<LibraryResult<ImmutableList<MediaItem>>> =
            Futures.immediateFuture(
                LibraryResult.ofItemList(
                    LibraryTrees.current.children(parentId, page, pageSize),
                    params,
                ),
            )

        override fun onGetItem(
            session: MediaLibrarySession,
            browser: MediaSession.ControllerInfo,
            mediaId: String,
        ): ListenableFuture<LibraryResult<MediaItem>> {
            val item = LibraryTrees.current.item(mediaId)
            return if (item == null) {
                Futures.immediateFuture(LibraryResult.ofError(SessionError.ERROR_BAD_VALUE))
            } else {
                Futures.immediateFuture(LibraryResult.ofItem(item, null))
            }
        }

        override fun onSearch(
            session: MediaLibrarySession,
            browser: MediaSession.ControllerInfo,
            query: String,
            params: LibraryParams?,
        ): ListenableFuture<LibraryResult<Void>> =
            Futures.immediateFuture(LibraryResult.ofError(SessionError.ERROR_NOT_SUPPORTED))

        /**
         * A head unit plays by media id: the platform's `playFromMediaId` becomes
         * `setMediaItems([one item])` here. Three cases, and only the first two
         * are MB25's:
         *
         * 1. **A NODE of the car's tree** (Favoris, a playlist, Mon Flow,
         *    Récents, Téléchargements): [AutoLibrary.play] queues what the node
         *    showed — or asks the player for its own entity — and answers an
         *    EMPTY list, which `NzPlayer.handleSetMediaItems` ignores because
         *    the queue is already the right one.
         * 2. **The tree's own titles** (`neezer:track:<key>`): they already carry
         *    everything `NzPlayer.trackOf` needs (id, album, duration), so they
         *    pass through untouched — Media3's default `onAddMediaItems` refuses
         *    a `MediaItem` without a `LocalConfiguration`, and every row of this
         *    app's queue is of that kind (`androidx.media3.session.MediaSession`
         *    `Callback.onAddMediaItems`: `UnsupportedOperationException`).
         * 3. **Anything else** — a queue row handed back by MB24's car overlay, a
         *    controller's own item — keeps Media3's default path exactly as it
         *    was; the notification, the lock screen and Bluetooth never come here
         *    at all (they drive the queue the player already holds).
         */
        override fun onSetMediaItems(
            session: MediaSession,
            browser: MediaSession.ControllerInfo,
            mediaItems: MutableList<MediaItem>,
            startIndex: Int,
            startPositionMs: Long,
        ): ListenableFuture<MediaSession.MediaItemsWithStartPosition> {
            val node = mediaItems.singleOrNull()?.mediaId
            if (node != null && AutoLibrary.isContainer(node)) {
                return AutoLibrary.play(node)
            }
            if (mediaItems.isNotEmpty() && mediaItems.all { AutoLibrary.isTreeItem(it.mediaId) }) {
                // `setMediaItem` arrives with `C.INDEX_UNSET` (-1): the one item
                // it named IS the first one.
                val start = startIndex.coerceIn(0, mediaItems.size - 1)
                return Futures.immediateFuture(
                    MediaSession.MediaItemsWithStartPosition(mediaItems, start, startPositionMs),
                )
            }
            return super.onSetMediaItems(session, browser, mediaItems, startIndex, startPositionMs)
        }

        override fun onGetSearchResult(
            session: MediaLibrarySession,
            browser: MediaSession.ControllerInfo,
            query: String,
            page: Int,
            pageSize: Int,
            params: LibraryParams?,
        ): ListenableFuture<LibraryResult<ImmutableList<MediaItem>>> =
            Futures.immediateFuture(LibraryResult.ofError(SessionError.ERROR_NOT_SUPPORTED))
    }

    companion object {
        private const val TAG = "NzMediaService"

        /** The notification channel; the app has no other media channel. */
        const val CHANNEL_ID = "neezer.lecture"

        /** One notification, updated in place (the old app's id stayed too). */
        const val NOTIFICATION_ID = 0x4E5A

        /**
         * The device's French cause when the output could not open, for Aide
         * (`null` while it works). Process-wide: Réglages reads it without
         * binding the service.
         */
        @Volatile
        var lastOutputFailure: String? = null
            private set

        /**
         * The device output the service started, for the diagnostic screen and
         * the acceptance (`opens()`, `performance_mode()`, the buffer the HAL
         * granted). `null` until the service has run, and again after it stops.
         */
        @Volatile
        var outputHandle: OutputHandle? = null
            private set

        /**
         * The API-26 pull, one per PROCESS: the session it drains is the engine's
         * sink, handed out once, and a service Android recreates adopts it.
         */
        @Volatile
        private var pull: AudioTrackPull? = null

        /** The flag store the offer reads, shared with the shell (MB16/MB-SWAP). */
        fun flags(context: Context): FlagStore = SharedPreferencesFlags(context)

        /** A kill was detected and the person was never asked: show the offer. */
        fun pendingBatteryOffer(context: Context): Boolean =
            BatteryExemption(flags(context)).shouldOffer()

        /** The dialog was shown: never again (whatever the answer). */
        fun markBatteryOfferShown(context: Context) {
            BatteryExemption(flags(context)).markOffered()
        }

        /**
         * The system's own exemption screen for this app, or `null` when it is
         * already exempt (a second dialog would be a lie).
         */
        fun batteryExemptionIntent(context: Context): Intent? {
            val power = context.getSystemService(android.os.PowerManager::class.java) ?: return null
            if (power.isIgnoringBatteryOptimizations(context.packageName)) return null
            return Intent(
                Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS,
                Uri.parse("package:${context.packageName}"),
            )
        }
    }
}

/**
 * The device's booleans: `commit()` and not `apply()`.
 *
 * The flag this stores is "we were playing when the process died", so losing the
 * write to a background flush is exactly the case it exists for. It is written
 * on TRANSITIONS only (a play, a pause), never per state, so the disk cost is a
 * few writes an hour.
 */
class SharedPreferencesFlags(context: Context) : FlagStore {
    private val prefs: SharedPreferences =
        context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    override fun getBoolean(key: String): Boolean = prefs.getBoolean(key, false)

    override fun putBoolean(key: String, value: Boolean) {
        prefs.edit().putBoolean(key, value).commit()
    }

    private companion object {
        const val FILE = "neezer-playback"
    }
}
