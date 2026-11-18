package fr.neezer.mobile.feature.onboarding

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.unit.Density
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.nz_ffi.AccountRun
import uniffi.nz_ffi.ImportState
import uniffi.nz_ffi.OnboardingStep
import java.io.File

/**
 * The onboarding and the invitation, rendered: Encre and Papier at 360 dp (the
 * phone) and 600 dp (the tablet's reading column), plus the 200 % font scale the
 * acceptance asks about, committed under
 * `docs/agents/screenshots/mobile/MB5b/`.
 *
 * The renderer is Robolectric's NATIVE graphics backend (Skia on the host): real
 * layout, real text with the three real fonts, real pixels, on the JVM — no
 * emulator, no window on the person's display (worker rule 7). The screens drawn
 * are the REAL [OnboardingScreen] and [InviteScreen] over [OnboardingFixtures].
 */
abstract class OnboardingScreenshotBase(private val width: Int) {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    protected val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set: the screenshot test writes the lot's images"),
        )

    private fun capture(
        theme: NzThemeKind,
        fontScale: Float,
        name: String,
        content: @androidx.compose.runtime.Composable () -> Unit,
    ) {
        rule.setContent {
            NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(
                    LocalDensity provides Density(density.density, fontScale),
                ) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) { content() }
                }
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        val bitmap = cropToContent(full, theme)
        val file = File(outputDir, name)
        file.parentFile?.mkdirs()
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
    }

    /**
     * The window is taller than the page: the tail is the ground and nothing
     * else. Cropping keeps the PNG a picture of the SCREEN rather than of an
     * empty display (MB5's own crop, same reason).
     */
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

    private fun name(theme: NzThemeKind, screen: String, fontScale: Float = 1f) =
        "mb5b-${theme.name.lowercase()}-%ddp-%s%s.png".format(
            width,
            screen,
            if (fontScale > 1f) "-font${(fontScale * 100).toInt()}" else "",
        )

    @Test
    fun artistsEncre() = capture(
        NzThemeKind.Encre,
        1f,
        name(NzThemeKind.Encre, "artistes"),
    ) { OnboardingScreen(artistsState(), OnboardingActions()) }

    @Test
    fun artistsPapier() = capture(
        NzThemeKind.Papier,
        1f,
        name(NzThemeKind.Papier, "artistes"),
    ) { OnboardingScreen(artistsState(), OnboardingActions()) }

    @Test
    fun artistsAtTwoHundredPercent() = capture(
        NzThemeKind.Encre,
        2f,
        name(NzThemeKind.Encre, "artistes", fontScale = 2f),
    ) { OnboardingScreen(artistsState(), OnboardingActions()) }

    @Test
    fun accountEncre() = capture(
        NzThemeKind.Encre,
        1f,
        name(NzThemeKind.Encre, "compte"),
    ) { OnboardingScreen(accountState(), OnboardingActions()) }

    @Test
    fun accountPapier() = capture(
        NzThemeKind.Papier,
        1f,
        name(NzThemeKind.Papier, "compte"),
    ) { OnboardingScreen(accountState(), OnboardingActions()) }

    @Test
    fun inviteEncre() = capture(
        NzThemeKind.Encre,
        1f,
        name(NzThemeKind.Encre, "invitation"),
    ) { InviteScreen(inviteState(), inviteFeedback(), InviteActions()) }

    @Test
    fun invitePapier() = capture(
        NzThemeKind.Papier,
        1f,
        name(NzThemeKind.Papier, "invitation"),
    ) { InviteScreen(inviteState(), inviteFeedback(), InviteActions()) }

    @Test
    fun inviteInvalidEncre() = capture(
        NzThemeKind.Encre,
        1f,
        name(NzThemeKind.Encre, "invitation-invalide"),
    ) {
        InviteScreen(
            inviteState().copy(phase = InviteModel.Phase.Invalid),
            inviteFeedback(),
            InviteActions(),
        )
    }

    companion object {
        fun artistsState(): OnboardingModel.State = OnboardingModel.State(
            loading = false,
            tiles = listOf(
                OnboardingFixtures.artist("101", "Aya Nakamura", genres = listOf("Pop")),
                OnboardingFixtures.artist("102", "Daft Punk", liked = true, genres = listOf("Électro")),
                OnboardingFixtures.artist("103", "Air", genres = listOf("Électro")),
                OnboardingFixtures.artist("104", "Justice", genres = listOf("Électro")),
                OnboardingFixtures.artist("105", "Kavinsky", genres = listOf("Électro")),
                OnboardingFixtures.artist("106", "Phoenix", genres = listOf("Pop")),
            ),
            liked = 1,
            likedIds = setOf("102"),
            feed = OnboardingFixtures.feed(),
        )

        fun accountState(): OnboardingModel.State = OnboardingModel.State(
            step = OnboardingStep.ACCOUNT,
            loading = false,
            options = OnboardingFixtures.options(
                deezer = OnboardingFixtures.provider(
                    canLink = false,
                    linked = true,
                    account = OnboardingFixtures.linkedAccount(
                        running = AccountRun(phase = "import", done = 42u, total = 120u),
                        addedArtists = 42,
                        addedTracks = 118,
                    ),
                ),
            ),
            importJob = OnboardingFixtures.job(
                state = ImportState.DONE,
                total = 3,
                done = 3,
                matched = 1,
                error = "Deezer ne répondait pas : certains titres n'ont pas pu être cherchés",
                items = listOf(
                    OnboardingFixtures.item("Get Lucky"),
                    OnboardingFixtures.item("Inconnu 1", matched = false, artists = "Quelqu'un"),
                    OnboardingFixtures.item("Inconnu 2", matched = false, artists = ""),
                ),
            ),
        )

        fun inviteState(): InviteModel.State = InviteModel.State(
            phase = InviteModel.Phase.Ready,
            label = "Invitation",
            expires = "2026-11-21T18:00:00",
            username = "elodie",
            password = "une phrase de passe simple",
            confirm = "une phrase de passe simple",
            submitted = true,
        )

        fun inviteFeedback(): InviteFeedback = InviteFeedback(
            strength = uniffi.nz_ffi.PasswordStrength.GOOD,
            score = 3,
            missing = 0,
            rules = OnboardingFixtures.rules,
            expiry = InviteModel.expiryLabel("2026-11-21T18:00:00"),
        )
    }
}

/** 360 dp: the phone the app is designed for first. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h2400dp-xhdpi")
class OnboardingScreenshotPhoneTest : OnboardingScreenshotBase(width = 360)

/** 600 dp: the tablet shell's reading column. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w600dp-h2400dp-xhdpi")
class OnboardingScreenshotTabletTest : OnboardingScreenshotBase(width = 600)
