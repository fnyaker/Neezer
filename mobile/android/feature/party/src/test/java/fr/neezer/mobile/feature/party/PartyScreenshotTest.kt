package fr.neezer.mobile.feature.party

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
import androidx.compose.ui.unit.Density
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.nz_ffi.AppLink

/**
 * The two party screens, rendered: the sheet (its three arguments, and the live
 * side with the link, the QR, the guests and the clock), and the guest's landing
 * card and live view — in Encre and Papier at 360 dp and 600 dp plus the 200 %
 * font scale, committed under `docs/agents/screenshots/mobile/MB20/`.
 *
 * The renderer is Robolectric's NATIVE graphics backend (Skia on the host): real
 * layout, real text with the design system's faces, real pixels, on the JVM — no
 * emulator, no window on the person's display (worker rule 7).
 *
 * **A screenshot is a test, not a decoration** (MB1's lesson): these are the only
 * images that prove the QR is on a white ground, that « Démarrer la soirée » did
 * not slide off a 360 dp phone, that the status line's green is legible on Papier
 * and that the chip row still reads at 200 %.
 */
abstract class PartyScreenshotBase(private val width: Int) {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set: the screenshot test writes the lot's images"),
        )

    protected val navigator = object : NzNavigator {
        override fun go(link: AppLink) = Unit
        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    protected fun capture(
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
        // A sheet slides in and a page fades: the IMAGE is the settled state, and
        // the test advances the main clock past the tokens (an EVENT, not a
        // sleep) and reads a pixel only then.
        rule.mainClock.advanceTimeBy(1_200)
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

    private fun suffix(fontScale: Float) = if (fontScale > 1f) "-font${(fontScale * 100).toInt()}" else ""

    protected fun name(theme: NzThemeKind, screen: String, fontScale: Float = 1f) =
        "mb20-%s-%ddp-%s%s.png".format(theme.name.lowercase(), width, screen, suffix(fontScale))

    // -- the fakes ------------------------------------------------------------

    /** A party this device hosts, with two guests and a locked clock. */
    protected fun hostingModel(): PartyModel {
        val service = FakePartyService()
        val model = PartyModel(
            service = service,
            scope = CoroutineScope(Dispatchers.Unconfined),
            share = object : PartyShare {
                override fun copy(text: String) = true
                override fun share(title: String, text: String, url: String) = true
            },
            trimDebounceMs = 0,
            errorText = ::testErrorText,
        )
        model.open()
        service.push(
            hostState(
                members = listOf(
                    partyMember(name = "Salon"),
                    partyMember(name = "Cuisine", status = "Chargement…", synced = false, waiting = true),
                ),
                audience = "2 personnes à l'écoute",
                status = "Horloge de cet appareil calée à ±0.4 ms · aller-retour 8 ms",
                clockSpreadMs = 0.4,
            ),
        )
        return model
    }

    /** A party that has not started: the three arguments. */
    protected fun introModel(): PartyModel {
        val service = FakePartyService()
        val model = PartyModel(
            service = service,
            scope = CoroutineScope(Dispatchers.Unconfined),
            share = object : PartyShare {
                override fun copy(text: String) = true
                override fun share(title: String, text: String, url: String) = true
            },
            errorText = ::testErrorText,
        )
        model.open()
        return model
    }

    /** The guest, on the landing card of a party somebody else hosts. */
    protected fun guestModel(stale: Boolean = false): PartyGuestModel {
        val service = FakePartyService(
            card = partyState(
                members = listOf(partyMember(name = "Salon")),
                inviteLine = "Camille vous invite à écouter avec 1 autre personne",
                listeners = 1u,
            ),
            cached = if (stale) partyState() else null,
        )
        val model = PartyGuestModel(
            service = service,
            scope = CoroutineScope(Dispatchers.Unconfined),
            id = "p1",
            trimDebounceMs = 0,
            errorText = ::testErrorText,
        )
        model.load()
        return model
    }

    /** The guest, listening: two tracks in, a locked clock, a next track. */
    protected fun liveModel(): PartyGuestModel {
        val service = FakePartyService()
        service.position = 42.5
        val model = PartyGuestModel(
            service = service,
            scope = CoroutineScope(Dispatchers.Unconfined),
            id = "p1",
            trimDebounceMs = 0,
            errorText = ::testErrorText,
        )
        model.load()
        model.join()
        service.push(
            partyState(
                next = partyTrack(id = "3000002", title = "Suivant", artist = "Kaya Project"),
                nextLine = "Suivant — Kaya Project",
                liveLine = "En direct avec Camille · 3 à l'écoute",
                listeners = 3u,
                osLatencyMs = 42L,
            ),
        )
        model.toggleTrim()
        return model
    }
}

/** 360 dp: the phone the plan's numbers are taken on. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class PartyPhoneScreenshotTest : PartyScreenshotBase(360) {
    @Test
    fun sheetIntroEncre() {
        val model = introModel()
        capture(name(NzThemeKind.Encre, "feuille-intro"), NzThemeKind.Encre) {
            PartySheet(model = model, current = partyState())
        }
    }

    @Test
    fun sheetIntroPapier() {
        val model = introModel()
        capture(name(NzThemeKind.Papier, "feuille-intro"), NzThemeKind.Papier) {
            PartySheet(model = model, current = partyState())
        }
    }

    @Test
    fun sheetLiveEncre() {
        val model = hostingModel()
        capture(name(NzThemeKind.Encre, "feuille-direct"), NzThemeKind.Encre) {
            PartySheet(model = model)
        }
    }

    @Test
    fun sheetLivePapier() {
        val model = hostingModel()
        capture(name(NzThemeKind.Papier, "feuille-direct"), NzThemeKind.Papier) {
            PartySheet(model = model)
        }
    }

    @Test
    fun sheetAtTwoHundredPercent() {
        val model = hostingModel()
        capture(name(NzThemeKind.Encre, "feuille-direct", 2f), NzThemeKind.Encre, fontScale = 2f) {
            PartySheet(model = model)
        }
    }

    @Test
    fun guestLandingEncre() {
        val model = guestModel()
        capture(name(NzThemeKind.Encre, "invite-accueil"), NzThemeKind.Encre) {
            PartyGuestScreen(model = model, nav = navigator, liveReads = false)
        }
    }

    @Test
    fun guestLandingPapier() {
        val model = guestModel(stale = true)
        capture(name(NzThemeKind.Papier, "invite-accueil"), NzThemeKind.Papier) {
            PartyGuestScreen(model = model, nav = navigator, liveReads = false)
        }
    }

    @Test
    fun guestLiveEncre() {
        val model = liveModel()
        capture(name(NzThemeKind.Encre, "invite-direct"), NzThemeKind.Encre) {
            PartyGuestScreen(model = model, nav = navigator, liveReads = false)
        }
    }

    @Test
    fun guestLivePapier() {
        val model = liveModel()
        capture(name(NzThemeKind.Papier, "invite-direct"), NzThemeKind.Papier) {
            PartyGuestScreen(model = model, nav = navigator, liveReads = false)
        }
    }

    @Test
    fun guestLiveAtTwoHundredPercent() {
        val model = liveModel()
        capture(name(NzThemeKind.Encre, "invite-direct", 2f), NzThemeKind.Encre, fontScale = 2f) {
            PartyGuestScreen(model = model, nav = navigator, liveReads = false)
        }
    }

    @Test
    fun guestMissingEncre() {
        val service = FakePartyService(card = null)
        val model = PartyGuestModel(
            service = service,
            scope = CoroutineScope(Dispatchers.Unconfined),
            id = "gone",
            trimDebounceMs = 0,
            errorText = ::testErrorText,
        )
        model.load()
        capture(name(NzThemeKind.Encre, "invite-absente"), NzThemeKind.Encre) {
            PartyGuestScreen(model = model, nav = navigator, liveReads = false)
        }
    }
}

/** 600 dp: the phone column, one step from the wide shell. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w600dp-h900dp-xhdpi")
class PartyWideScreenshotTest : PartyScreenshotBase(600) {
    @Test
    fun sheetWideEncre() {
        val model = hostingModel()
        capture(name(NzThemeKind.Encre, "feuille-direct"), NzThemeKind.Encre) {
            PartySheet(model = model)
        }
    }

    @Test
    fun sheetWidePapier() {
        val model = hostingModel()
        capture(name(NzThemeKind.Papier, "feuille-direct"), NzThemeKind.Papier) {
            PartySheet(model = model)
        }
    }

    @Test
    fun guestWideEncre() {
        val model = liveModel()
        capture(name(NzThemeKind.Encre, "invite-direct"), NzThemeKind.Encre) {
            PartyGuestScreen(model = model, nav = navigator, liveReads = false)
        }
    }

    @Test
    fun guestWidePapier() {
        val model = guestModel()
        capture(name(NzThemeKind.Papier, "invite-accueil"), NzThemeKind.Papier) {
            PartyGuestScreen(model = model, nav = navigator, liveReads = false)
        }
    }
}
