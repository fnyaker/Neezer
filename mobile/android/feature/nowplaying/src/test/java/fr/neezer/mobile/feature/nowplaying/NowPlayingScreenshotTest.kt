package fr.neezer.mobile.feature.nowplaying

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.statusBars
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.assertWidthIsAtLeast
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.advanceEventTime
import androidx.compose.ui.test.down
import androidx.compose.ui.test.moveBy
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.feature.nowplaying.NowPlayingState.Pane
import java.io.File
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The player, rendered: the full-screen sheet (phone thumb column and the wide
 * reading columns), the queue editor, the words (with a line and with none) and
 * « À propos », in Encre and Papier at 360 dp and 600 dp plus the 200 % font
 * scale — committed under `docs/agents/screenshots/mobile/MB11/`.
 *
 * The renderer is Robolectric's NATIVE graphics backend (Skia on the host): real
 * layout, real text with the design system's faces, real pixels, on the JVM — no
 * emulator, no window on the person's display (worker rule 7).
 *
 * **A screenshot is a test, not a decoration** (MB1's lesson): these images are
 * the only thing that proves a badge did not inherit the wrong colour, a title did
 * not clip at 200 %, or the thumb row still fits five 44 dp targets on a 360 dp
 * phone.
 */
abstract class NowPlayingScreenshotBase(protected val width: Int) {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set: the screenshot test writes the lot's images"),
        )

    /**
     * P1's own images. The base's `nz.screenshots` property is MB11's directory
     * (its build.gradle.kts owns it); this lot writes next to it from the
     * repository root the same build script already hands the test, so no shared
     * build file is edited (`docs/agents/mobile.md`, worker rule 3).
     */
    private val p1OutputDir: File
        get() = File(
            File(
                System.getProperty("nz.repoRoot")
                    ?: error("nz.repoRoot is not set: the screenshot test needs the repository root"),
            ),
            "docs/agents/screenshots/mobile/P1",
        )

    /** P2's own images: the sheet held under the finger. */
    private val p2OutputDir: File
        get() = File(
            File(
                System.getProperty("nz.repoRoot")
                    ?: error("nz.repoRoot is not set: the screenshot test needs the repository root"),
            ),
            "docs/agents/screenshots/mobile/P2",
        )

    protected val navigator = object : NzNavigator {
        override fun go(link: uniffi.nz_ffi.AppLink) = Unit
        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    protected fun capture(
        name: String,
        theme: NzThemeKind,
        fontScale: Float = 1f,
        content: @Composable () -> Unit,
    ) = captureTo(outputDir, name, theme, fontScale, content)

    /** One of P1's blurred-backdrop images. */
    protected fun captureP1(
        name: String,
        theme: NzThemeKind,
        fontScale: Float = 1f,
        content: @Composable () -> Unit,
    ) = captureTo(p1OutputDir, name, theme, fontScale, content)

    /**
     * P2's image: the sheet, held at [held] of its height with the finger still
     * down. The whole point of the lot is that this frame exists at all — an
     * `AnimatedVisibility` could not paint it (the sheet was not movable), and the
     * exit always restarted at 0.
     */
    protected fun captureDraggedSheet(name: String, theme: NzThemeKind, held: Float = 0.25f) {
        NowPlayingState.hide()
        NowPlayingState.show()
        val model = model()
        // Read outside the composition (lint's `StateFlow.value` rule).
        val state = model.state.value
        rule.setContent {
            NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                // The page the sheet covers, as the shell draws it: what a dismissal
                // uncovers is the app's ink, never the window's default white.
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                PlayerSheetFrame(
                    opened = NowPlayingState.open,
                    onDismiss = { NowPlayingState.hide() },
                ) {
                    Box(Modifier.fillMaxSize().testTag(P2_TAG)) {
                        PlayerSurface(
                            state = state,
                            model = model,
                            nav = navigator,
                            lyrics = lyrics(),
                            about = null,
                            onClose = {},
                            liveReads = false,
                        )
                    }
                }
                }
            }
        }
        rule.mainClock.advanceTimeBy(1_200)
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
        rule.onNodeWithTag(P2_TAG).performTouchInput {
            down(Offset(centerX, top + 40.dp.toPx()))
            advanceEventTime(400)
            moveBy(Offset(0f, height * held))
        }
        rule.waitForIdle()
        writeBitmap(p2OutputDir, name, theme)
    }

    private fun captureTo(
        dir: File,
        name: String,
        theme: NzThemeKind,
        fontScale: Float = 1f,
        content: @Composable () -> Unit,
    ) {
        rule.setContent {
            NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale)) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) { content() }
                }
            }
        }
        // The sheets slide in and the scenes fade: the IMAGE must be the settled
        // state, so the test advances the main clock past the tokens (an EVENT,
        // not a sleep) and reads a pixel only then.
        rule.mainClock.advanceTimeBy(1_200)
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
        writeBitmap(dir, name, theme)
    }

    /** The window as it is now, cropped and written to [dir]/[name]. */
    private fun writeBitmap(dir: File, name: String, theme: NzThemeKind) {
        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        val bitmap = cropToContent(full, theme)
        val file = File(dir, name)
        file.parentFile?.mkdirs()
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
    }

    /** The window is taller than the page: crop the ground away (MB5's recipe). */
    private fun cropToContent(full: Bitmap, theme: NzThemeKind): Bitmap {
        val background = Sillon.theme(theme).bg.toArgb()
        var last = full.height - 1
        val row = IntArray(full.width)
        while (last > 0) {
            full.getPixels(row, 0, full.width, 0, last, full.width, 1)
            if (row.any { it != background }) break
            last--
        }
        val height = (last + 24).coerceIn(1, full.height)
        if (height <= 24) return full
        return Bitmap.createBitmap(full, 0, 0, full.width, height)
    }

    protected companion object {
        /** P2's sheet node: `Modifier.offset` moves the content, so the tag is on it. */
        const val P2_TAG = "p2-sheet"
    }

    private fun suffix(fontScale: Float) = if (fontScale > 1f) "-font${(fontScale * 100).toInt()}" else ""

    protected fun name(theme: NzThemeKind, screen: String, fontScale: Float = 1f) =
        "mb11-%s-%ddp-%s%s.png".format(theme.name.lowercase(), width, screen, suffix(fontScale))

    // -- the fakes ------------------------------------------------------------

    protected fun model(
        playing: Boolean = true,
        episode: Boolean = false,
        rows: Int = 4_000,
        sleep: uniffi.nz_ffi.SleepState? = null,
    ): NowPlayingModel {
        val port = FakePlayerPort(
            snapshot = FakePlayerPort.snapshot(
                state = if (playing) uniffi.nz_ffi.PlayState.PLAYING else uniffi.nz_ffi.PlayState.PAUSED,
                current = FakePlayerPort.track(episode = episode, albumId = if (episode) "show-7" else "501"),
                index = 1u,
                len = rows.toUInt(),
            ).copy(sleep = sleep),
            queueRows = FakePlayerPort.queueOf(rows),
        )
        val service = FakeService(cachedLyrics = FakeService.lyrics())
        val model = NowPlayingModel(
            player = port,
            service = service,
            scope = kotlinx.coroutines.CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
        )
        model.refresh()
        return model
    }

    protected fun lyrics(loaded: Boolean = true): LyricsModel {
        val service = if (loaded) FakeService(cachedLyrics = FakeService.lyrics()) else FakeService(
            cachedLyrics = FakeService.emptyLyrics(),
            fresh = FakeService.emptyLyrics(),
        )
        val model = LyricsModel(
            service = service,
            scope = kotlinx.coroutines.CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
        )
        model.open("9001", "Fauve", "Le chant des sirènes")
        if (loaded) model.onPosition(25.0)
        return model
    }

    // -- the images -----------------------------------------------------------

    private fun player(theme: NzThemeKind, fontScale: Float = 1f, playing: Boolean = true) {
        val model = model(playing = playing, sleep = uniffi.nz_ffi.SleepState(30u, false))
        val lyricsModel = lyrics()
        // The state is read OUTSIDE the composition (lint's `StateFlow.value`
        // rule): a screenshot is a still, and it is taken from the model the fakes
        // were driven into, not from a live flow.
        val state = model.state.value
        capture(name(theme, "lecteur", fontScale), theme, fontScale) {
            PlayerSurface(
                state = state,
                model = model,
                nav = navigator,
                lyrics = lyricsModel,
                about = null,
                onClose = {},
                liveReads = false,
            )
        }
    }

    @Test
    fun playerEncrePhone() = player(NzThemeKind.Encre)

    @Test
    fun playerPapierPhone() = player(NzThemeKind.Papier, playing = false)

    @Test
    fun playerAtTwoHundredPercent() = player(NzThemeKind.Encre, fontScale = 2f)

    @Test
    fun playerPapierWide() {
        val model = model()
        val state = model.state.value
        capture(name(NzThemeKind.Papier, "lecteur-large"), NzThemeKind.Papier) {
        PlayerSurface(
            state = state,
            model = model,
            nav = navigator,
            lyrics = lyrics(),
            about = null,
            onClose = {},
            liveReads = false,
        )
        }
    }

    @Test
    fun queueEncre() = capture(name(NzThemeKind.Encre, "file"), NzThemeKind.Encre) {
        val model = model()
        QueueSheet(model = model, nav = navigator, lyrics = null, onClose = {})
    }

    @Test
    fun queuePapierWide() = capture(name(NzThemeKind.Papier, "file"), NzThemeKind.Papier) {
        val model = model()
        QueueSheet(model = model, nav = navigator, lyrics = null, onClose = {})
    }

    @Test
    fun lyricsEncre() {
        val model = model()
        val state = model.state.value
        capture(name(NzThemeKind.Encre, "paroles"), NzThemeKind.Encre) {
            LyricsSheet(lyrics = lyrics(), state = state, onClose = {}, onCard = {})
        }
    }

    @Test
    fun lyricsPapierNone() {
        val model = model()
        val state = model.state.value
        capture(name(NzThemeKind.Papier, "paroles-sans"), NzThemeKind.Papier) {
            LyricsSheet(lyrics = lyrics(loaded = false), state = state, onClose = {}, onCard = {})
        }
    }

    @Test
    fun aboutEncre() = capture(name(NzThemeKind.Encre, "a-propos"), NzThemeKind.Encre) {
        val model = model()
        val about = AboutModel(
            service = FakeService(),
            scope = kotlinx.coroutines.CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
        )
        AboutSheet(
            about = about,
            player = FakePlayerPort(),
            nav = navigator,
            track = FakePlayerPort.track(),
            onClose = {},
        )
    }

    /** The card's own sheet, with a bitmap already drawn (the painter's own test). */
    @Test
    fun cardEncre() {
        val lyricsModel = lyrics()
        val lyricsState = lyricsModel.state.value
        val card = CardSheetState()
        card.bitmap = Bitmap.createBitmap(1080, 1350, Bitmap.Config.ARGB_8888).apply {
            val canvas = Canvas(this)
            canvas.drawColor(android.graphics.Color.rgb(18, 20, 26))
        }
        capture(name(NzThemeKind.Encre, "carte"), NzThemeKind.Encre) {
            LyricCardSheet(
                state = lyricsState,
                model = lyricsModel,
                card = card,
                track = FakePlayerPort.track(),
                onClose = {},
            )
        }
    }

    /** The player's own sheet host, from the state the shell toggles. */
    @Test
    fun thePaneOrderOpensOneAtATime() {
        // The SPA's rule: opening one sheet closes the other.
        NowPlayingState.show()
        NowPlayingState.showPane(Pane.Queue)
        assert(NowPlayingState.pane == Pane.Queue)
        NowPlayingState.showPane(Pane.Lyrics)
        assert(NowPlayingState.pane == Pane.Lyrics)
        NowPlayingState.openQueue()
        assert(NowPlayingState.pane == Pane.Queue)
        NowPlayingState.hide()
        assert(!NowPlayingState.open && NowPlayingState.pane == Pane.None)
    }

    /**
     * The two doors of M5: « Soirée d'écoute » and « Contrôle à distance », in the
     * thumb row between « + » and « Paroles » (`MobileNowPlaying.svelte` §9).
     *
     * It proves the row FITS at every width of the class and at both font scales
     * the plan names: every door keeps a ≥ 48 dp touch target, the five doors stay
     * on ONE line inside the page's usable width, and « À propos » — which left the
     * row in this lot — sits below it, centred. Run against `dcb78607` it fails
     * first on « Soirée d'écoute » (**32 dp**: the slot was wrapped in `Door`'s
     * 8 dp pads), then on « Paroles » (no contentDescription); the row of six with
     * the chevron inside it is the ~20 dp last target MB11's capture measured.
     *
     * That the app PASSES the two slots is `NowPlayingHost`'s two lines, and the
     * emulator smoke is what opens the real player.
     */
    @Test
    fun thePlayerRowHasRoomForTheTwoDoors() {
        val model = model()
        // The state is read OUTSIDE the composition (lint's `StateFlow.value` rule).
        val state = model.state.value
        val fontScale = mutableStateOf(1f)
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale.value)) {
                    PlayerSurface(
                        state = state,
                        model = model,
                        nav = navigator,
                        lyrics = null,
                        about = null,
                        onClose = {},
                        party = { NzIconButton(NzGlyph.Party, "Soirée d'écoute", {}) },
                        remote = { NzIconButton(NzGlyph.Remote, "Contrôle à distance", {}) },
                        liveReads = false,
                    )
                }
            }
        }
        for (scale in listOf(1f, 2f)) {
            fontScale.value = scale
            rule.mainClock.advanceTimeBy(1_200)
            shadowOf(Looper.getMainLooper()).idle()
            rule.waitForIdle()
            assertTheThumbRowFits(scale)
        }
    }

    /**
     * The row's own geometry, read from the semantics tree: five doors, one line,
     * 48 dp each, inside the page's usable width — and the peek below them.
     */
    private fun assertTheThumbRowFits(fontScale: Float) {
        val doors = listOf(
            "Ajouter à une playlist",
            "Soirée d'écoute",
            "Contrôle à distance",
            "Paroles",
            "File d'attente",
        )
        val boxes = doors.map { description ->
            val node = rule.onNodeWithContentDescription(description).assertIsDisplayed()
            val bounds = node.getUnclippedBoundsInRoot()
            println(
                "MEASURE m5 door: width=${width}dp fontScale=$fontScale « $description » " +
                    "${bounds.right - bounds.left}×${bounds.bottom - bounds.top} at x=${bounds.left.value}",
            )
            node.assertWidthIsAtLeast(48.dp).assertHeightIsAtLeast(48.dp)
            bounds
        }
        val left = boxes.minOf { it.left.value }
        val right = boxes.maxOf { it.right.value }
        val top = boxes.minOf { it.top.value }
        val bottom = boxes.maxOf { it.bottom.value }
        // 360 dp keeps the phone's `pageX` (20 dp); the wide shells keep `xl`.
        val usable = width - 2 * if (width >= 640) 24 else 20
        println(
            "MEASURE m5 row: width=${width}dp fontScale=$fontScale " +
                "row=${right - left}dp usable=${usable}dp top=${top}dp bottom=${bottom}dp",
        )
        assertTrue(
            "the five doors span ${right - left} dp of the $usable dp usable at $fontScale×",
            right - left <= usable + 0.5f,
        )
        assertTrue(
            "the row is ONE line at $fontScale× (tops ${boxes.map { it.top.value }}, " +
                "bottoms ${boxes.map { it.bottom.value }})",
            boxes.all { it.top.value - top <= 0.5f && bottom - it.bottom.value <= 0.5f },
        )
        // Side by side, in order, no overlap: a wrapped Row would stack them.
        boxes.zipWithNext { a, b ->
            assertTrue("each door is right of the previous one at $fontScale×", b.left.value >= a.right.value)
        }
        val about = rule.onNodeWithContentDescription("À propos du titre")
            .assertIsDisplayed()
            .assertWidthIsAtLeast(48.dp)
            .assertHeightIsAtLeast(48.dp)
            .getUnclippedBoundsInRoot()
        println(
            "MEASURE m5 peek: width=${width}dp fontScale=$fontScale " +
                "${about.right - about.left}×${about.bottom - about.top} at x=${about.left.value} " +
                "rowBottom=${bottom}dp",
        )
        assertTrue(
            "« À propos » sits UNDER the row at $fontScale× (about.top=${about.top.value}, row.bottom=$bottom)",
            about.top.value >= bottom - 0.5f,
        )
        // Centred on the column the row lives in.
        val rowCentre = (left + right) / 2
        val aboutCentre = (about.left.value + about.right.value) / 2
        assertTrue(
            "« À propos » is centred under the row at $fontScale× ($aboutCentre vs $rowCentre)",
            kotlin.math.abs(aboutCentre - rowCentre) <= 1f,
        )
    }
    /**
     * P1's geometry: the sleeve draws at its slot's size, the slot is square, and
     * the header sits inside the status bar (M5, M12).
     *
     * The centring test is what holds M12: the old stage forced a `420.dp` sleeve
     * inside a ≈288 dp slot and the `Box` cropped it at the top-start, so the drawn
     * rect was the slot's rect plus 132 dp and its optical centre was off. Asserting
     * the drawn rect against the slot's (within 1 dp) is the number; a screenshot
     * shows the symptom but cannot say by how much.
     *
     * The inset half is the platform's own value: on Robolectric it is 0, so this
     * assertion holds trivially on the JVM and bites on the emulator journey, which
     * is where the real inset exists.
     */
    @Test
    fun p1TheSleeveFillsItsSlot() {
        val model = model()
        val state = model.state.value
        var insetTopDp = 0f
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                insetTopDp = WindowInsets.statusBars.getTop(density) / density.density
                PlayerSurface(
                    state = state,
                    model = model,
                    nav = navigator,
                    lyrics = null,
                    about = null,
                    onClose = {},
                    liveReads = false,
                )
            }
        }
        rule.mainClock.advanceTimeBy(1_200)
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()

        val slots = rule.onAllNodesWithTag("player-cover-slot", useUnmergedTree = true)
        val covers = rule.onAllNodesWithTag("player-cover", useUnmergedTree = true)
        val slotCount = slots.fetchSemanticsNodes().size
        val coverCount = covers.fetchSemanticsNodes().size
        println(
            "MEASURE p1 sleeve: width=${width}dp slots=$slotCount sleeves=$coverCount " +
                "insetTop=${insetTopDp}dp",
        )
        assertTrue("the stage composed its pages ($slotCount slots)", slotCount >= 3)
        val slot = slots[1].getUnclippedBoundsInRoot()
        val slotW = slot.right - slot.left
        val slotH = slot.bottom - slot.top
        assertEquals("the slot is square", slotW.value, slotH.value, 1f)
        for (index in 0 until coverCount) {
            val cover = covers[index].getUnclippedBoundsInRoot()
            assertEquals(
                "sleeve $index draws the slot's width",
                slotW.value,
                (cover.right - cover.left).value,
                1f,
            )
            assertEquals(
                "sleeve $index draws the slot's height",
                slotH.value,
                (cover.bottom - cover.top).value,
                1f,
            )
        }

        val header = rule.onNodeWithContentDescription("Réduire le lecteur").getUnclippedBoundsInRoot()
        assertTrue(
            "the header starts below the status bar inset " +
                "(top=${header.top.value}dp, inset=${insetTopDp}dp)",
            header.top.value >= insetTopDp + 3.9f,
        )
    }
}

/** 360 dp: the phone the plan's worst case names. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class NowPlayingPhoneScreenshotTest : NowPlayingScreenshotBase(360) {
    /** P2: the sheet held at a quarter of its height, the finger still down. */
    @Test
    fun p2DraggedSheetPhone() =
        captureDraggedSheet("p2-glissement-360dp.png", NzThemeKind.Encre)

    /** P1: the blurred backdrop under the player (phone, Encre). */
    @Test
    fun p1BlurredBackdropEncre() {
        val model = model()
        // The state is read OUTSIDE the composition (lint's `StateFlow.value`
        // rule): a screenshot is a still, not a live flow.
        val state = model.state.value
        captureP1("p1-flou-360dp.png", NzThemeKind.Encre) {
            PlayerSurface(
                state = state,
                model = model,
                nav = navigator,
                lyrics = lyrics(),
                about = null,
                onClose = {},
                liveReads = false,
            )
        }
    }

    /** P1: the same ground on Papier. */
    @Test
    fun p1BlurredBackdropPapier() {
        val model = model(playing = false)
        // Outside the composition: see `p1BlurredBackdropEncre`.
        val state = model.state.value
        captureP1("p1-flou-papier-360dp.png", NzThemeKind.Papier) {
            PlayerSurface(
                state = state,
                model = model,
                nav = navigator,
                lyrics = null,
                about = null,
                onClose = {},
                liveReads = false,
            )
        }
    }
}

/** 600 dp: the phone column, one step from the wide shell. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w600dp-h900dp-xhdpi")
class NowPlayingWideScreenshotTest : NowPlayingScreenshotBase(600) {
    /** P2: the same drag at 600 dp, where the column is still the phone's. */
    @Test
    fun p2DraggedSheetWide() =
        captureDraggedSheet("p2-glissement-600dp.png", NzThemeKind.Encre)

    /** P1: the blurred backdrop at 600 dp. */
    @Test
    fun p1BlurredBackdropWide() {
        val model = model()
        // Outside the composition: see `p1BlurredBackdropEncre`.
        val state = model.state.value
        captureP1("p1-flou-600dp.png", NzThemeKind.Encre) {
            PlayerSurface(
                state = state,
                model = model,
                nav = navigator,
                lyrics = lyrics(),
                about = null,
                onClose = {},
                liveReads = false,
            )
        }
    }
}

/**
 * 1280 dp: the tablet shell the plan names ("rail + reading columns, as
 * `DesktopNowPlaying`") — the words on the left, the sleeve in the middle, the
 * queue and « À propos » on the right (the SPA's own 900/1100 dp breakpoints).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w1280dp-h800dp-mdpi")
class NowPlayingTabletScreenshotTest : NowPlayingScreenshotBase(1280) {
    @Test
    fun tabletEncre() {
        val model = model()
        val state = model.state.value
        val lyricsModel = lyrics()
        capture(name(NzThemeKind.Encre, "lecteur-tablette"), NzThemeKind.Encre) {
            PlayerSurface(
                state = state,
                model = model,
                nav = navigator,
                lyrics = lyricsModel,
                about = null,
                onClose = {},
                liveReads = false,
            )
        }
    }

    @Test
    fun tabletQueue() = capture(name(NzThemeKind.Encre, "file"), NzThemeKind.Encre) {
        val model = model()
        QueueSheet(model = model, nav = navigator, lyrics = null, onClose = {})
    }
}
