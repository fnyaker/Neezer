package fr.neezer.mobile.feature.podcasts

import android.os.Looper
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.animation.core.TweenSpec
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import fr.neezer.mobile.designsystem.NzDur
import fr.neezer.mobile.designsystem.NzMotion
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.inPlaceSpecs
import fr.neezer.mobile.feature.offline.DownloadsModel
import fr.neezer.mobile.feature.offline.OfflineHost
import fr.neezer.mobile.feature.offline.OfflinePort
import fr.neezer.mobile.testkit.ScreenGoldenRule
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.nz_ffi.DownloadEntry
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.NoHandle
import uniffi.nz_ffi.OfflineListener
import uniffi.nz_ffi.OfflineState
import uniffi.nz_ffi.OfflineSettings
import uniffi.nz_ffi.SetProgress
import uniffi.nz_ffi.StorageReport

/**
 * The podcast subscriptions, rendered and VERIFIED at 360 dp (plan T3's matrix,
 * one representative state).
 *
 * T3 replaced MB13's write-only captures with this golden. The real
 * [PodcastsScreen] is drawn over a two-show grid, with MB15's download button
 * installed on a fake port so the header control is in its real state (worker
 * rule 7: no emulator, no window).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h900dp-xhdpi")
class PodcastsGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    @After
    fun forget() {
        OfflineHost.reset()
    }

    @Test
    fun subscriptionsAt360() {
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
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    PodcastsScreen(gridState(), podcastsActions(), isAdmin = true)
                }
            }
        }
        rule.composeRule.mainClock.advanceTimeBy(1_000)
        shadowOf(Looper.getMainLooper()).idle()
        rule.composeRule.waitForIdle()
        rule.capture("podcasts-abonnements-360dp-f1.0.png")
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

    /**
     * X3: the subscriptions' phase replaced in place. At eco the skeleton is out
     * of the tree in the SAME frame the grid is drawn (M7 + M13); a re-emit of the
     * same phase adds no node (M6). The curve is X0's `inPlaceSpecs` (its exact
     * spec; `TransitionTest` explains why a running animation is unreadable).
     */
    @Test
    fun aPhaseChangeReplacesInTheSameFrameAtEco() {
        var state by mutableStateOf(PodcastsModel.State())
        var tick by mutableIntStateOf(0)
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    tick
                    PodcastsScreen(state = state, actions = podcastsActions(), isAdmin = false)
                }
            }
        }
        rule.composeRule.onNodeWithTag(PODCASTS_LOADING_TAG).assertExists()
        rule.composeRule.onNodeWithText("Les Histoires").assertDoesNotExist()

        state = gridState()
        rule.composeRule.waitForIdle()
        rule.composeRule.onNodeWithTag(PODCASTS_LOADING_TAG).assertDoesNotExist()
        rule.composeRule.onNodeWithText("Les Histoires").assertExists()

        tick++
        rule.composeRule.waitForIdle()
        rule.composeRule.onAllNodesWithText("Les Histoires").assertCountEquals(1)

        val full = inPlaceSpecs(NzMotion(NzMotionLevel.Full))
        assertEquals(NzDur.Fast, (full.fade as TweenSpec<*>).durationMillis)
        assertEquals(NzDur.Fast, (full.move as TweenSpec<*>).durationMillis)
        assertNull(full.exit)
        assertEquals(0, (inPlaceSpecs(NzMotion(NzMotionLevel.Off)).fade as TweenSpec<*>).durationMillis)
    }
}

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
