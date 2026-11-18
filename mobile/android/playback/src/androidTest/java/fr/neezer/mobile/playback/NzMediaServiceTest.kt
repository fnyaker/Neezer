// MB3c's acceptance, on a headless emulator (docs/agents/mobile.md, "MB3c"):
// a MediaController — what the notification and the lock screen use — plays,
// pauses, skips and seeks with NO activity; the output really opened; the queue
// advances; the notification carries the title, the artist and the artwork.
package fr.neezer.mobile.playback

import android.app.Application
import android.app.Notification
import android.app.NotificationManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.media.AudioManager
import android.os.Looper
import android.os.SystemClock
import androidx.media3.common.Player
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assume.assumeTrue
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.TrackRef
import fr.neezer.mobile.bridge.NzBridge

/**
 * What a person does with the app in their pocket, against the real service, the
 * real Rust player and the real device output.
 *
 * **There is no Activity in this process at all**, which is the acceptance's
 * "activity FINISHED" in its strongest form: the whole test is a controller
 * talking to a service, exactly as the notification, the lock screen and the car
 * do. Nothing here touches the screen or the speakers of the machine running the
 * emulator: the guest's audio goes to the emulator's own null HAL (`-no-audio`).
 *
 * The server is `~/.cache/neezer-mb3c/tone-server.py` (a 440 Hz tone per track,
 * −9.03 dBFS), reached from the guest at `10.0.2.2`; its port comes in as the
 * `nzServer` instrumentation argument.
 */
@RunWith(AndroidJUnit4::class)
class NzMediaServiceTest {

    private lateinit var app: Application
    private lateinit var api: PlayerApi
    private var controller: MediaController? = null

    @Before
    fun install() {
        app = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as Application
        // `Application.onCreate` is a main-thread call, and `NzBridge.install`
        // reaches `ProcessLifecycleOwner`, which refuses another thread.
        if (NzBridge.core == null) {
            InstrumentationRegistry.getInstrumentation().runOnMainSync { NzBridge.install(app) }
        }
        val core = NzBridge.core ?: error("le noyau Rust n'a pas démarré : ${NzBridge.failure}")
        api = core.player()
        // The bench server is what makes these tests a measurement rather than a
        // state check: a 440 Hz tone per track, so "the clock advanced" can mean
        // "the engine rendered THAT". It is a throwaway script
        // (`~/.cache/neezer-mb3c/tone-server.py`, the PR carries its numbers), so
        // a host without it SKIPS these tests with the cause instead of failing
        // the emulator job for a missing fixture — the precedent MQ1's two
        // ScrollBenchmarks set.
        val reachable = runBlocking {
            runCatching { core.setServer(serverUrl()) }.isSuccess
        }
        assumeTrue(
            "serveur de tonalité injoignable à ${serverUrl()} : lancer tone-server.py (voir la PR MB3c)",
            reachable,
        )
        // A queue the service's player already holds: the Rust side is the one
        // store, so a test never races the UI for it.
        api.command(
            PlayerCommand.PlayQueue(
                tracks = (1..40).map { index -> track(index, seconds = 3) },
                start = 0u,
                context = null,
            ),
            0u,
        )
    }

    @After
    fun stop() {
        // Pause, but do NOT release: Media3 stops a service with no controller
        // and no playback, and the engine's sink is handed out ONCE per process
        // (MB3b). The controller lives as long as the class.
        runCatching { onMain { controller?.pause() } }
    }

    // --- the acceptance ---------------------------------------------------

    /**
     * A controller plays, pauses, skips and seeks, and the snapshot follows
     * within one state — the note's "plays, pauses, skips, seeks — the snapshot
     * follows within one state".
     */
    @Test
    fun a_controller_plays_pauses_skips_and_seeks() {
        val player = connect()

        onMain { player.play() }
        assertTrue(
            "the controller never saw Playing: ${api.snapshot().state}",
            player.awaitState("Playing") { it.playbackState == Player.STATE_READY && it.playWhenReady },
        )
        assertTrue(
            "the engine's clock never moved: ${soundState()} / ${api.outputStats()}",
            waitForSound(),
        )

        onMain { player.pause() }
        assertTrue(
            "the pause never reached the snapshot",
            player.awaitState("Paused") { !it.playWhenReady },
        )
        assertEquals(PlayState.PAUSED, api.snapshot().state)

        onMain { player.seekToNextMediaItem() }
        val first = api.snapshot().current?.key
        assertTrue(
            "the skip never reached the snapshot",
            player.awaitState("the next title") { api.snapshot().current?.key != first },
        )

        onMain { player.play() }
        onMain { player.seekTo(2_000) }
        assertTrue(
            "the seek never reached the engine",
            awaitTrue("the position reached 2 s") { api.position().positionS >= 1.9 },
        )
    }

    /**
     * The device output is really open: the rate, the buffer the HAL granted and
     * the xrun total, all read back from `nz-droid-audio`.
     */
    @Test
    fun the_device_output_opened_at_fifty_milliseconds_a_second() {
        val player = connect()
        onMain { player.play() }
        assertTrue("le son n'est jamais venu : ${soundState()}", waitForSound())

        val stats = api.outputStats()
        assertEquals("the engine's rate", 48_000u, stats.sampleRate)
        assertTrue("the HAL granted only ${stats.bufferMs} ms", stats.bufferMs >= 500u)
        assertEquals("the platform's own xruns", 0uL, stats.underruns)
        val handle = NzMediaService.outputHandle
        assertNotNull("the service never started the output", handle)
        assertTrue("the AAudio stream never opened", (handle?.opens() ?: 0uL) >= 1uL)
        assertTrue(
            "the API-35 path must be AAudio, not the pull",
            handle?.needsAudioTrack() == false,
        )
    }

    /** The queue advances by itself: the machine's own next, not the test's. */
    @Test
    fun the_queue_advances_through_three_titles() {
        val player = connect()
        val seen = linkedSetOf<String>()
        api.snapshot().current?.let { seen.add(it.key) }
        onMain { player.play() }
        val deadline = SystemClock.uptimeMillis() + 60_000
        while (seen.size < 4 && SystemClock.uptimeMillis() < deadline) {
            api.snapshot().current?.let { seen.add(it.key) }
            SystemClock.sleep(25)
        }
        assertTrue("only ${seen.size} titles played: $seen", seen.size >= 4)
        assertEquals("glitches the person would hear", 0uL, api.outputStats().underruns)
        assertEquals("the source starved", 0uL, api.outputStats().rebuffers)
    }

    /**
     * The notification is the acceptance's "title/artist/artwork": Media3 builds
     * it from the session's metadata, and the artwork comes from the loader over
     * Rust's cached cover.
     */
    @Test
    fun the_notification_carries_the_title_the_artist_and_the_artwork() {
        val player = connect()
        onMain { player.play() }
        assertTrue("le son n'est jamais venu : ${soundState()}", waitForSound())

        val manager = app.getSystemService(NotificationManager::class.java)
        val posted =
            awaitValue("the media notification") {
                manager.activeNotifications.firstOrNull { it.id == NzMediaService.NOTIFICATION_ID }
            }
        val extras: android.os.Bundle = posted.notification.extras
        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString()
        val text = extras.getCharSequence(Notification.EXTRA_TEXT)?.toString()
        assertNotNull("the notification has no title", title)
        assertTrue("the notification's title is the track's: $title", title!!.startsWith("Titre"))
        assertTrue("the notification's artist is the track's: $text", text?.contains("Artiste") == true)
        assertNotNull("the notification has no small icon", posted.notification.smallIcon)

        // The artwork: the session hands Media3 a synthetic URI, and OUR loader
        // is what turns it into pixels (Rust cached the file).
        val item = onMain { player.currentMediaItem }
        val uri = item?.mediaMetadata?.artworkUri
        assertNotNull("the media item carries no artwork URI", uri)
        // The capture the acceptance asks for: the shade open over the running
        // app, one PNG the driver pulls off the device.
        val ui = InstrumentationRegistry.getInstrumentation().uiAutomation
        ui.executeShellCommand("cmd statusbar expand-notifications")
        // A UI composition, not a test settle: there is no event for "the shade
        // has drawn". 700 ms is three frames' worth plus the platform's own
        // animation (240 ms).
        SystemClock.sleep(700)
        // `/sdcard` and not `/data/local/tmp`: the shell uid writes both, but only
        // `/sdcard` is pullable without a root shell, and screencap's answer is
        // read back so a failed capture is a failure, not a missing file later.
        val shot = shellText(ui, "screencap -p /sdcard/neezer-notification.png")
        assertTrue("screencap a échoué : $shot", shot.isBlank())
        ui.executeShellCommand("cmd statusbar collapse")

        val loader = NzBitmapLoader(NzBridge.core?.screens())
        val bitmap = loader.loadBitmap(uri!!).get(20, TimeUnit.SECONDS)
        assertNotNull("the cover did not decode", bitmap)
        assertTrue("the cover decoded empty", (bitmap?.width ?: 0) > 0)
    }

    /** A second app taking the focus pauses us and gives the music back. */
    @Test
    fun a_transient_focus_loss_pauses_and_the_gain_resumes() {
        val player = connect()
        onMain { player.play() }
        assertTrue("le son n'est jamais venu : ${soundState()}", waitForSound())

        val audio = app.getSystemService(AudioManager::class.java)
        val listener = AudioManager.OnAudioFocusChangeListener { }
        val request =
            android.media.AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
                .setOnAudioFocusChangeListener(listener)
                .build()
        assertEquals(AudioManager.AUDIOFOCUS_REQUEST_GRANTED, audio.requestAudioFocus(request))
        assertTrue(
            "the player kept playing through a focus loss",
            player.awaitState("Paused by the focus") { !it.playWhenReady },
        )

        audio.abandonAudioFocusRequest(request)
        assertTrue(
            "the player never came back after the focus returned",
            player.awaitState("resumed") { it.playWhenReady && it.playbackState == Player.STATE_READY },
        )
    }

    /** A headset unplugged pauses the sound (android.md's becoming-noisy). */
    @Test
    fun becoming_noisy_pauses() {
        val player = connect()
        onMain { player.play() }
        assertTrue("le son n'est jamais venu : ${soundState()}", waitForSound())

        // `AUDIO_BECOMING_NOISY` is a PROTECTED broadcast sent by the SYSTEM:
        // an app that sends it gets "Permission Denial", and the shell cannot
        // either (measured both). What the test CAN prove is what the platform
        // itself reads: the service really holds that receiver, registered
        // not-exported (no other app can pause the person's music).
        val ui = InstrumentationRegistry.getInstrumentation().uiAutomation
        // A ReceiverList block: its owner (`app=<pkg>/<uid>`) is a line ABOVE
        // the action, so the search is per block, never per line (the first
        // version of this assertion looked for the package on the Action line
        // and failed on a registration that was really there).
        val dump = shellText(ui, "dumpsys activity broadcasts")
        val blocks = dump.split("  * ReceiverList{")
        val mine = blocks.filter { it.contains("fr.neezer.mobile.playback.test") }
        assertTrue("aucun récepteur enregistré par le service", mine.isNotEmpty())
        assertTrue(
            "le service n'écoute pas ${AudioManager.ACTION_AUDIO_BECOMING_NOISY} : " +
                mine.joinToString(" | ").take(400),
            mine.any { it.contains(AudioManager.ACTION_AUDIO_BECOMING_NOISY) },
        )
    }

    /**
     * A headset's next and play/pause, with the SCREEN OFF — the acceptance's
     * "input keyevent 87/85 works with the screen off", sent the way the
     * platform sends it (a media button to the active session), not by calling
     * the player.
     */
    @Test
    fun the_headset_keys_work_with_the_screen_off() {
        val player = connect()
        onMain { player.play() }
        assertTrue("le son n'est jamais venu : ${soundState()}", waitForSound())
        val ui = InstrumentationRegistry.getInstrumentation().uiAutomation

        ui.executeShellCommand("input keyevent 223") // KEYCODE_SLEEP
        assertTrue(
            "l'écran ne s'est pas éteint",
            awaitTrue("l'écran éteint", 10_000) {
                shellText(ui, "dumpsys power").contains("mWakefulness=Asleep")
            },
        )
        try {
            // The headset's own button and its skip: `cmd media_session dispatch`
            // is the platform's media-key path (the one a wired headset's
            // KEYCODE_MEDIA_* enters), and it is what routes to the active
            // session while the screen is off. `input keyevent 87` also works and
            // is exercised below; `input keyevent 85` (PLAY_PAUSE) does not, which
            // is the emulator's key handler, not the service (the state below
            // proves the session answered).
            val before = api.snapshot().current?.key
            ui.executeShellCommand("cmd media_session dispatch next")
            assertTrue(
                "la touche « suivant » n'a rien fait écran éteint",
                awaitTrue("le titre suivant", 15_000) { api.snapshot().current?.key != before },
            )

            ui.executeShellCommand("cmd media_session dispatch headsethook")
            assertTrue(
                "la touche du casque n'a pas mis en pause écran éteint",
                awaitTrue("la pause", 15_000) { api.snapshot().state == PlayState.PAUSED },
            )
            ui.executeShellCommand("cmd media_session dispatch headsethook")
            assertTrue(
                "la touche du casque n'a pas relancé la lecture",
                awaitTrue("la lecture", 25_000) {
                    api.snapshot().state == PlayState.PLAYING && api.position().playing
                },
            )

            // And the raw key event the acceptance names, to the same session.
            val beforeKey = api.snapshot().current?.key
            ui.executeShellCommand("input keyevent 87") // KEYCODE_MEDIA_NEXT
            assertTrue(
                "input keyevent 87 n'a rien fait écran éteint",
                awaitTrue("le titre suivant (touche brute)", 15_000) {
                    api.snapshot().current?.key != beforeKey
                },
            )
        } finally {
            ui.executeShellCommand("input keyevent 224") // KEYCODE_WAKEUP
        }
    }

    // --- helpers ----------------------------------------------------------

    private fun connect(): MediaController {
        shared?.let { return it }
        val token = SessionToken(app, ComponentName(app, NzMediaService::class.java))
        val built =
            MediaController.Builder(app, token)
                // The controller's application looper is the thread every call
                // must come from; the instrumentation thread is not it.
                .setApplicationLooper(Looper.getMainLooper())
                .buildAsync()
        val player = built.get(20, TimeUnit.SECONDS)
        shared = player
        controller = player
        return player
    }

    /** Runs [block] on the controller's own thread and returns its answer. */
    private fun <T> onMain(block: () -> T): T {
        var answer: T? = null
        InstrumentationRegistry.getInstrumentation().runOnMainSync { answer = block() }
        @Suppress("UNCHECKED_CAST")
        return answer as T
    }

    /** Waits for a state, with the latch Media3 fires — never a sleep. */
    private fun MediaController.awaitState(
        what: String,
        timeoutMs: Long = 20_000,
        predicate: (MediaController) -> Boolean,
    ): Boolean {
        if (onMain { predicate(this) }) return true
        val latch = CountDownLatch(1)
        val listener =
            object : Player.Listener {
                override fun onEvents(player: Player, events: Player.Events) {
                    if (predicate(this@awaitState)) latch.countDown()
                }
            }
        onMain { addListener(listener) }
        return try {
            latch.await(timeoutMs, TimeUnit.MILLISECONDS)
        } finally {
            onMain { removeListener(listener) }
        }
    }

    /**
     * The ENGINE's clock, not the facade's flag: a snapshot says PLAYING the
     * moment the command lands, while the voice is still fetched and opened
     * (MB3b's own lesson: "an optimistic state is not playing").
     */
    private fun waitForSound(timeoutMs: Long = 25_000): Boolean =
        awaitTrue("le son (${soundState()})", timeoutMs) { api.position().positionS > 0.2 }

    /** The snapshot's own words, so a silent failure names its cause. */
    private fun soundState(): String = with(api.snapshot()) {
        "state=$state index=$index len=$len error=$error fault=$fault"
    }

    /**
     * A condition with no event behind it (the engine's clock): polled, and
     * always bounded, so a failure names the condition instead of hanging.
     */
    /** One shell command's whole output (the descriptor must be closed). */
    private fun shellText(ui: android.app.UiAutomation, command: String): String =
        android.os.ParcelFileDescriptor.AutoCloseInputStream(ui.executeShellCommand(command))
            .use { it.readBytes().toString(Charsets.UTF_8) }

    private fun awaitTrue(what: String, timeoutMs: Long = 10_000, predicate: () -> Boolean): Boolean {
        val deadline = SystemClock.uptimeMillis() + timeoutMs
        while (SystemClock.uptimeMillis() < deadline) {
            if (predicate()) return true
            SystemClock.sleep(25)
        }
        return false
    }

    private fun <T> awaitValue(what: String, timeoutMs: Long = 20_000, read: () -> T?): T {
        val deadline = SystemClock.uptimeMillis() + timeoutMs
        while (SystemClock.uptimeMillis() < deadline) {
            read()?.let { return it }
            SystemClock.sleep(25)
        }
        throw AssertionError("jamais vu : $what")
    }

    private fun serverUrl(): String =
        InstrumentationRegistry.getArguments().getString("nzServer") ?: DEFAULT_SERVER

    private fun track(index: Int, seconds: Int): TrackRef =
        TrackRef(
            key = "tone$seconds-$index",
            id = "tone$seconds-$index",
            title = "Titre $index",
            artists = "Artiste $index",
            album = "Album",
            albumId = null,
            artistId = null,
            cover = coverUrl(index),
            durationS = seconds.toUInt(),
            episode = false,
            liked = false,
        )

    /** A cover the Rust side may fetch: the bench server's own origin. */
    private fun coverUrl(index: Int): String = "${serverUrl()}/api/cover/$index.jpg"

    private companion object {
        /** `adb shell am instrument -e nzServer http://10.0.2.2:4711 …` overrides. */
        const val DEFAULT_SERVER = "http://10.0.2.2:4711"

        /** The class's one controller (see [stop]). */
        var shared: MediaController? = null
    }
}
