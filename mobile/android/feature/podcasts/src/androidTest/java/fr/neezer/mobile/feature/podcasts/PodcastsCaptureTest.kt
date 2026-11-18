package fr.neezer.mobile.feature.podcasts

import android.graphics.Bitmap
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.Density
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.After
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.nz_ffi.DownloadEntry
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.NoHandle
import uniffi.nz_ffi.OfflineListener
import uniffi.nz_ffi.OfflineSettings
import uniffi.nz_ffi.OfflineState
import uniffi.nz_ffi.SetProgress
import uniffi.nz_ffi.StorageReport
import fr.neezer.mobile.feature.offline.DownloadsModel
import fr.neezer.mobile.feature.offline.OfflineHost
import fr.neezer.mobile.feature.offline.OfflinePort

/**
 * The acceptance's **real-screen capture**: the podcast grid and the show page,
 * rendered by the REAL Compose runtime on the headless emulator, written into
 * the test app's own files, from where the lot pulls them
 * (`adb -s emulator-<port> pull`).
 *
 * It is an instrumented test, not a debug activity in `:app`: a library module
 * may not open a door into the app's manifest for a screenshot, and the test APK
 * already has the one activity Compose needs. Nothing is drawn on the person's
 * display (`mobile/tools/emu.sh start` is `-no-window`), and the bitmap is read
 * from the node the test itself built.
 *
 * The state it paints is a fake port's (the device has no session), so what this
 * proves is what the JVM render cannot: the real fonts at the device's real
 * density, TalkBack's tree, and the 200 % scale.
 */
@RunWith(AndroidJUnit4::class)
class PodcastsCaptureTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    @After
    fun forget() {
        OfflineHost.reset()
    }

    private fun capture(name: String, theme: NzThemeKind, fontScale: Float = 1f, content: @androidx.compose.runtime.Composable () -> Unit) {
        OfflineHost.install(
            DownloadsModel(
                port = DeviceDownloads(),
                scope = CoroutineScope(Dispatchers.Main.immediate),
                io = Dispatchers.Main.immediate,
                freeSpace = { 8L * 1024 * 1024 * 1024 },
                toast = { _, _, _ -> },
                causeText = { "erreur" },
            ),
        )
        val windowGround = Sillon.theme(theme).bg.toArgb()
        rule.activity.runOnUiThread {
            rule.activity.window.setBackgroundDrawable(
                android.graphics.drawable.ColorDrawable(windowGround),
            )
        }
        rule.setContent {
            val density = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale)) {
                NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                    Box(
                        Modifier
                            .fillMaxSize()
                            .background(Sillon.colors.bg)
                            .testTag("capture"),
                    ) {
                        content()
                    }
                }
            }
        }
        rule.waitForIdle()
        val full = rule.onNodeWithTag("capture").captureToImage().asAndroidBitmap()
        val dir = File(
            InstrumentationRegistry.getInstrumentation().targetContext.filesDir,
            "mb13-screenshots",
        ).apply { mkdirs() }
        val file = File(dir, name)
        file.outputStream().use { full.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${full.width}x${full.height} ${file.absolutePath} ${file.length()} bytes")
        check(full.width > 100 && full.height > 100) { "the screen painted nothing" }
    }

    @Test
    fun the_podcast_grid_renders_on_the_device() {
        capture("podcasts-encre.png", NzThemeKind.Encre) {
            PodcastsScreen(
                state = PodcastsModel.State(
                    shows = listOf(
                        show("show-1", "Les Histoires", deezer = "9400001", episodes = 2_000, autoArchive = true),
                        show("show-2", "Météo du soir", episodes = 12),
                    ),
                    loading = false,
                ),
                actions = podcastsActions(),
                isAdmin = true,
            )
        }
        rule.onNodeWithTag("capture").assertExists()
    }

    @Test
    fun the_show_page_renders_on_the_device() {
        capture("podcast-encre.png", NzThemeKind.Encre) { ShowScreen(deviceState(), actions(), true, null, false, 0.0) }
        rule.onNodeWithTag("capture").assertExists()
    }

    @Test
    fun the_show_page_renders_in_papier_at_200_percent() {
        capture("podcast-papier-200.png", NzThemeKind.Papier, fontScale = 2f) {
            ShowScreen(deviceState(), actions(), true, null, false, 0.0)
        }
        rule.onNodeWithTag("capture").assertExists()
    }

    private fun podcastsActions(): PodcastsActions = PodcastsActions(
        retry = {},
        setQuery = {},
        retrySearch = {},
        submitLink = {},
        subscribe = {},
        openShow = {},
        consumeMessage = {},
    )

    private fun actions(): ShowActions = ShowActions(
        retry = {},
        onVisible = { _, _, _ -> },
        loadMore = {},
        play = {},
        playAll = {},
        playAt = { _, _ -> },
        toggleAuto = {},
        retryTranscribe = {},
        toggleTranscribe = {},
        setLanguage = {},
        setBacklog = {},
        setBatchCount = {},
        startBatch = {},
        startOne = {},
        stopOne = {},
        setSearchQuery = {},
        retrySearch = {},
        clearSearch = {},
        toggleChapters = {},
        openSheet = {},
        addMarker = { _, _ -> },
        removeMarker = {},
        openEpisode = {},
        consumeMessage = {},
    )

    /** The show page's state on a device: the same fixtures the JVM test uses. */
    private fun deviceState(): ShowModel.State {
        val episodes = (0 until 6).map { index ->
            episode(
                "ep-$index",
                title = "Épisode $index · Un récit plus long que la ligne",
                chapters = if (index == 0) 3 else 0,
                transcript = when (index) {
                    0 -> transcriptInfo("ready", 1.0)
                    1 -> transcriptInfo("running", 0.42)
                    2 -> transcriptInfo("queued")
                    else -> null
                },
                position = if (index == 1) resumeAt(1_200) else null,
                markers = if (index == 2) listOf(marker("m-1", "ep-2", 600_000, "La maison")) else emptyList(),
            )
        }
        return ShowModel.State(
            id = "show-1",
            show = show("show-1", "Les Histoires", episodes = 2_000, autoArchive = true),
            loading = false,
            loaded = episodes.size,
            next = "offset:6",
            cards = episodes.withIndex().associate { it.index to it.value },
            transcribeSettings = settings(),
            batchCount = 5,
        )
    }

    /** The downloads' port on a device: an empty queue, which is the real one. */
    private class DeviceDownloads : OfflinePort {
        private val state = OfflineState(
            version = 0uL,
            state = "Rien à télécharger",
            paused = false,
            sets = emptyList(),
            settings = OfflineSettings(quality = "opus320", offlineOnly = false, playCacheCap = 0L),
            storage = StorageReport(
                downloadsBytes = 0uL,
                cacheBytes = 0uL,
                coversBytes = 0uL,
                tracks = 0u,
                freeBytes = 0uL,
            ),
            note = null,
        )

        override suspend fun state(): OfflineState = state

        override suspend fun addSet(context: String, quality: String, title: String, artist: String): SetProgress =
            SetProgress(
                context = context,
                title = title,
                kind = context.substringBefore(':'),
                id = context.substringAfter(':', ""),
                done = 0u,
                total = 0u,
                failed = 0u,
                running = false,
                paused = false,
                failures = emptyList(),
            )

        override suspend fun removeSet(kind: String, id: String) = Unit

        override suspend fun removeDownload(key: String) = Unit

        override suspend fun retry(key: String) = Unit

        override suspend fun retryAll() = Unit

        override suspend fun pause() = Unit

        override suspend fun resume() = Unit

        override suspend fun setQuality(quality: String) = Unit

        override suspend fun setOfflineOnly(onlyDownloaded: Boolean) = Unit

        override suspend fun setPlayCacheCap(bytes: Long) = Unit

        override suspend fun clearPlayCache() = Unit

        override suspend fun downloads(): ListHandle = ListHandle(NoHandle)

        override suspend fun facts(keys: List<String>): List<DownloadEntry> = emptyList()

        override suspend fun playCacheBytes(): Long = 0L

        override suspend fun subscribe(listener: OfflineListener) = Unit
    }
}
