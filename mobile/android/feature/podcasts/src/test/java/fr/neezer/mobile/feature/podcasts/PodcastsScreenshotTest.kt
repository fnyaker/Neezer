package fr.neezer.mobile.feature.podcasts

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.hasScrollToNodeAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.unit.Density
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.feature.offline.DownloadsModel
import fr.neezer.mobile.feature.offline.OfflineHost
import fr.neezer.mobile.feature.offline.OfflinePort
import uniffi.nz_ffi.DownloadEntry
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.NoHandle
import uniffi.nz_ffi.OfflineListener
import uniffi.nz_ffi.OfflineState
import uniffi.nz_ffi.OfflineSettings
import uniffi.nz_ffi.SetProgress
import uniffi.nz_ffi.StorageReport
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.After
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The podcast screens, rendered: Encre and Papier at 360 dp and 600 dp, plus the
 * 200 % font scale the acceptance asks about — committed under
 * `docs/agents/screenshots/mobile/MB13/`.
 *
 * The renderer is Robolectric's NATIVE graphics backend (Skia on the host): real
 * layout, real text with the three real fonts, real pixels, on the JVM — no
 * emulator, no window on the person's display (worker rule 7). The same recipe
 * MB1 measured and MB5/MB7/MB10/MB15 hold.
 */
abstract class PodcastsScreenshotBase(private val widthDp: Int) {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set: the screenshot test writes the lot's images"),
        )

    @After
    fun forget() {
        OfflineHost.reset()
    }

    private fun capture(
        name: String,
        theme: NzThemeKind,
        fontScale: Float = 1f,
        content: @Composable () -> Unit,
    ) {
        // MB15's button is ON the show page: its model is installed over a fake
        // port, exactly as MB15's own render does, so the header draws the real
        // control in its real state.
        OfflineHost.install(
            DownloadsModel(
                port = EmptyOfflinePort(),
                scope = CoroutineScope(Dispatchers.Unconfined),
                io = Dispatchers.Unconfined,
                freeSpace = { 8L * 1024 * 1024 * 1024 },
                toast = { _, _, _ -> },
                causeText = { "erreur" },
            ),
        )
        rule.setContent {
            NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale)) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) { content() }
                }
            }
        }
        // The screens are static once painted; advancing past the motion tokens
        // makes the IMAGE the settled state (an event, not a sleep).
        rule.mainClock.advanceTimeBy(1_000)
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        val bitmap = cropToContent(full, theme)
        val file = File(outputDir, name)
        file.parentFile?.mkdirs()
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
    }

    /** The decor is taller than the content: crop the empty ground away. */
    private fun cropToContent(full: Bitmap, theme: NzThemeKind): Bitmap {
        val ground = Sillon.theme(theme).bg.toArgb()
        var last = full.height - 1
        for (y in full.height - 1 downTo 0) {
            var content = false
            for (x in 0 until full.width step 4) {
                if (full.getPixel(x, y) != ground) {
                    content = true
                    break
                }
            }
            if (content) {
                last = y
                break
            }
        }
        val height = (last + NzSpacing.l.value.toInt()).coerceIn(1, full.height)
        return Bitmap.createBitmap(full, 0, 0, full.width, height)
    }

    /** The subscriptions: two shows, one card with an archive badge. */
    private fun gridState(): PodcastsModel.State = PodcastsModel.State(
        shows = listOf(
            show("show-1", "Les Histoires", deezer = "9400001", episodes = 2_000, autoArchive = true),
            show("show-2", "Météo du soir", episodes = 12),
        ),
        loading = false,
    )

    private fun podcastsActions(): PodcastsActions = PodcastsActions(
        retry = {},
        setQuery = {},
        retrySearch = {},
        submitLink = {},
        subscribe = {},
        openShow = {},
        consumeMessage = {},
    )

    @Test
    fun subscriptions_encre() = capture("podcasts-encre-${widthDp}dp.png", NzThemeKind.Encre) {
        PodcastsScreen(gridState(), podcastsActions(), isAdmin = true)
    }

    @Test
    fun subscriptions_papier() = capture("podcasts-papier-${widthDp}dp.png", NzThemeKind.Papier) {
        PodcastsScreen(gridState(), podcastsActions(), isAdmin = true)
    }

    @Test
    fun subscriptions_200_percent() = capture(
        "podcasts-encre-${widthDp}dp-200.png",
        NzThemeKind.Encre,
        fontScale = 2f,
    ) {
        PodcastsScreen(gridState(), podcastsActions(), isAdmin = true)
    }

    @Test
    fun subscriptions_empty() = capture("podcasts-vide-encre-${widthDp}dp.png", NzThemeKind.Encre) {
        PodcastsScreen(
            PodcastsModel.State(loading = false),
            podcastsActions(),
            isAdmin = false,
        )
    }

    @Test
    fun subscriptions_failure() = capture("podcasts-echec-encre-${widthDp}dp.png", NzThemeKind.Encre) {
        PodcastsScreen(
            PodcastsModel.State(loading = false, failure = "Le serveur n'a pas répondu."),
            podcastsActions(),
            isAdmin = true,
        )
    }

    /** The show page: header, archive switch, transcription card, rows. */
    private fun showState(): ShowModel.State {
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

    private fun showActions(): ShowActions = ShowActions(
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

    @Test
    fun show_encre() = capture("podcast-encre-${widthDp}dp.png", NzThemeKind.Encre) {
        ShowScreen(
            state = showState(),
            actions = showActions(),
            isAdmin = true,
            currentKey = "ep-1",
            playing = false,
            positionS = 1_200.0,
        )
    }

    @Test
    fun show_papier() = capture("podcast-papier-${widthDp}dp.png", NzThemeKind.Papier) {
        ShowScreen(
            state = showState(),
            actions = showActions(),
            isAdmin = true,
            currentKey = "ep-1",
            playing = false,
            positionS = 1_200.0,
        )
    }

    @Test
    fun show_200_percent() = capture(
        "podcast-encre-${widthDp}dp-200.png",
        NzThemeKind.Encre,
        fontScale = 2f,
    ) {
        ShowScreen(
            state = showState(),
            actions = showActions(),
            isAdmin = true,
            currentKey = "ep-1",
            playing = false,
            positionS = 1_200.0,
        )
    }

    /** The episode panel: chapters, the card, the list and the ticks. */
    @Test
    fun panel_chapters() = capture("panneau-chapitres-encre-${widthDp}dp.png", NzThemeKind.Encre) {
        EpisodePanelSheet(
            episode = episode("ep-0", chapters = 3, transcript = transcriptInfo("ready", 1.0)),
            service = FakePodcastService(),
            chapters = chaptersOf("ep-0"),
            chaptersBusy = false,
            chaptersFailure = null,
            markers = listOf(marker("m-1", "ep-0", 600_000, "La maison")),
            isCurrent = true,
            positionS = 1_000.0,
            admin = true,
            onLoadChapters = {},
            onSeek = {},
            onResume = {},
            onAddMarker = {},
            onRemoveMarker = {},
            onStartTranscript = {},
            onStopTranscript = {},
            onRestartTranscript = {},
            onDeleteTranscript = {},
            onOpenShow = {},
            onDismiss = {},
        )
    }

    /**
     * **TalkBack**: every control of the two screens carries a French name, and
     * the ones that say something the pixels cannot (a play button's episode, a
     * marker's moment, the search field's purpose) carry it in their label.
     */
    @Test
    fun every_control_is_named_for_a_screen_reader() {
        OfflineHost.install(
            DownloadsModel(
                port = EmptyOfflinePort(),
                scope = CoroutineScope(Dispatchers.Unconfined),
                io = Dispatchers.Unconfined,
                freeSpace = { 0L },
                toast = { _, _, _ -> },
                causeText = { "erreur" },
            ),
        )
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                ShowScreen(
                    state = showState(),
                    actions = showActions(),
                    isAdmin = true,
                    currentKey = "ep-1",
                    playing = false,
                    positionS = 1_200.0,
                )
            }
        }
        rule.waitForIdle()
        rule.onNodeWithText("Lire").assertExists()
        // The rows below the fold are SCROLLED to first: a lazy list does not
        // compose what nobody has looked at, and the check is about what the
        // person can reach, not about what a 360 dp viewport happens to hold.
        fun scrollTo(text: String) {
            rule.onNode(hasScrollToNodeAction()).performScrollToNode(hasText(text, substring = true))
            rule.waitForIdle()
        }
        // The switch and its row both carry the sentence: the control IS named.
        scrollTo("Archiver automatiquement")
        rule.onAllNodesWithText("Archiver automatiquement les nouveaux épisodes")
            .onFirst()
            .assertExists()
        scrollTo("Transcription des épisodes")
        rule.onNodeWithText("Transcription des épisodes").assertExists()
        scrollTo("Rechercher dans les épisodes")
        rule.onNodeWithContentDescription("Rechercher dans les épisodes").assertExists()
        scrollTo("Épisode 0")
        rule.onNodeWithContentDescription("Lire Épisode 0 · Un récit plus long que la ligne").assertExists()
        rule.onNodeWithContentDescription("Afficher les chapitres").assertExists()
        scrollTo("Épisode 2")
        rule.onNodeWithContentDescription("Supprimer le marqueur").assertExists()
    }
}

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h1800dp-xhdpi")
class PodcastsScreenshotPhone : PodcastsScreenshotBase(360)

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w600dp-h2000dp-xhdpi")
class PodcastsScreenshotWide : PodcastsScreenshotBase(600)

/** A downloads port that is always empty: the render only needs a state. */
private class EmptyOfflinePort : OfflinePort {
    private val state = OfflineState(
        version = 0uL,
        state = "Rien à télécharger",
        paused = false,
        sets = emptyList(),
        settings = OfflineSettings(
            quality = "opus320",
            offlineOnly = false,
            playCacheCap = 2_147_483_648L,
        ),
        storage = StorageReport(
            downloadsBytes = 0uL,
            cacheBytes = 0uL,
            coversBytes = 0uL,
            tracks = 0u,
            freeBytes = 8_000_000_000uL,
        ),
        note = null,
    )

    override suspend fun state(): OfflineState = state

    override suspend fun addSet(
        context: String,
        quality: String,
        title: String,
        artist: String,
    ): SetProgress = SetProgress(
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
