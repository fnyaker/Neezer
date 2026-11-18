package fr.neezer.mobile.feature.admin

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
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
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.cancel
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

/**
 * Les deux pages d'administration, rendues : Encre et Papier à 360 dp (le
 * téléphone) et 600 dp (la colonne de lecture de la tablette), plus l'échelle
 * 200 % que l'acceptation demande, sous
 * `docs/agents/screenshots/mobile/MB17/`.
 *
 * Le rendu est le backend NATIF de Robolectric (Skia sur l'hôte) : vraie mise en
 * page, vrai texte avec les trois polices, vrais pixels, sur la JVM — aucun
 * émulateur, aucune fenêtre sur l'écran de la personne (règle 7). La recette que
 * MB1 a mesurée et que MB5/MB6/MB7/MB16 réutilisent.
 *
 * Ce qui est dessiné est la VRAIE page ([AdminPage]) sur [FakeAdminService] : une
 * capture d'une maquette n'aurait pas attrapé les défauts de mise en page.
 */
abstract class AdminScreenshotBase(private val width: Int) {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    protected val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set: the screenshot test writes the lot's images"),
        )

    protected fun capture(
        section: String,
        theme: NzThemeKind,
        fontScale: Float = 1f,
        service: FakeAdminService = FakeAdminService(),
        name: String = "mb17-%s-%s-%ddp%s.png".format(
            section,
            theme.name.lowercase(),
            width,
            if (fontScale > 1f) "-font${(fontScale * 100).toInt()}" else "",
        ),
        open: ((AdminModel) -> Unit)? = null,
    ) {
        val scope = CoroutineScope(Dispatchers.Main)
        val model = AdminModel(service, scope, io = Dispatchers.Main)
        model.open(section)
        // The page's own reads land on the main looper: drain it BEFORE a test
        // drives the screen, or it would open a sheet on an empty table.
        shadowOf(Looper.getMainLooper()).idle()
        open?.invoke(model)
        rule.setContent {
            NzTheme(kind = theme, accent = null, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale)) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                        // Réglages wraps a category in its own scroll; the capture
                        // reproduces that, so the page is drawn the way it is met.
                        Box(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                            // Réglages pads a category by `pageX`; without it the
                            // capture would show pills touching the screen's edge.
                            Box(
                                Modifier.padding(
                                    start = fr.neezer.mobile.designsystem.NzSpacing.pageX,
                                    end = fr.neezer.mobile.designsystem.NzSpacing.pageX,
                                    top = fr.neezer.mobile.designsystem.NzSpacing.l,
                                ),
                            ) {
                                AdminPage(section = section, model = model)
                            }
                        }
                    }
                }
            }
        }
        // The model's flows run on the main looper Robolectric owns: let them land
        // (an EVENT, never a duration — `idle()` drains the queue).
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()

        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        val bitmap = cropToContent(full, theme)
        outputDir.mkdirs()
        val file = File(outputDir, name)
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
        scope.cancel()
    }

    /** A block rendered on its own (a folded-away panel opened for the capture). */
    protected fun captureContent(
        theme: NzThemeKind,
        name: String,
        fontScale: Float = 1f,
        content: @androidx.compose.runtime.Composable () -> Unit,
    ) {
        rule.setContent {
            NzTheme(kind = theme, accent = null, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale)) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                        Box(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                            Box(
                                Modifier.padding(
                                    start = fr.neezer.mobile.designsystem.NzSpacing.pageX,
                                    end = fr.neezer.mobile.designsystem.NzSpacing.pageX,
                                    top = fr.neezer.mobile.designsystem.NzSpacing.l,
                                ),
                            ) { content() }
                        }
                    }
                }
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        val bitmap = cropToContent(full, theme)
        outputDir.mkdirs()
        val file = File(outputDir, name)
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
    }

    /** The window is taller than the page: the tail is the ground and nothing else. */
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

    // --- *Utilisateurs* -------------------------------------------------------

    @Test
    fun usersInEncre() = capture(section = AdminModel.SECTION_USERS, theme = NzThemeKind.Encre)

    @Test
    fun usersInPapier() = capture(section = AdminModel.SECTION_USERS, theme = NzThemeKind.Papier)

    @Test
    fun usersAtTwoHundredPercent() = capture(
        section = AdminModel.SECTION_USERS,
        theme = NzThemeKind.Papier,
        fontScale = 2f,
        name = "mb17-users-papier-%ddp-font200.png".format(width),
    )

    /** A failed table: the cause and « Réessayer », never a spinner. */
    @Test
    fun aFailedTableNamesItsCause() = capture(
        section = AdminModel.SECTION_USERS,
        theme = NzThemeKind.Encre,
        service = FakeAdminService().apply {
            failures["users"] = IllegalStateException("Le serveur ne répond pas")
        },
        name = "mb17-users-failure-encre-%ddp.png".format(width),
    )

    /** A member reached the page: no request, and the sentence that says why. */
    @Test
    fun aMemberSeesNoAdminEntry() = capture(
        section = AdminModel.SECTION_SERVER,
        theme = NzThemeKind.Encre,
        service = FakeAdminService(admin = false),
        name = "mb17-member-encre-%ddp.png".format(width),
    )

    /** The account sheet: the facts, the two switches and the danger zone. */
    @Test
    fun theAccountSheet() = capture(
        section = AdminModel.SECTION_USERS,
        theme = NzThemeKind.Encre,
        open = { model ->
            model.openUser(model.state.rows.users().first { it.id == "u-2" })
        },
        name = "mb17-account-sheet-encre-%ddp.png".format(width),
    )

    /** The invite sheet, on the step that shows the link. */
    @Test
    fun theInviteLink() = capture(
        section = AdminModel.SECTION_USERS,
        theme = NzThemeKind.Papier,
        open = { model ->
            model.openInvite()
            model.createInvite("Léa", AdminCopy.INVITE_DEFAULT_TTL)
        },
        name = "mb17-invite-papier-%ddp.png".format(width),
    )

    /**
     * A job's failures, OPENED: the ranked causes, the rows and what the
     * server's sample left out (rule 2). The block is folded on the page — a
     * healthy run must not shout — so it is captured on its own, open.
     */
    @Test
    fun aJobsFailures() = captureContent(
        theme = NzThemeKind.Encre,
        name = "mb17-failures-encre-%ddp.png".format(width),
    ) {
        JobFailuresBlock(
            failures = AdminFixtures.failures(),
            label = "analyse",
            onCopy = {},
            startOpen = true,
        )
    }

    // --- *Serveur* ------------------------------------------------------------

    @Test
    fun serverInEncre() = capture(section = AdminModel.SECTION_SERVER, theme = NzThemeKind.Encre)

    @Test
    fun serverInPapier() = capture(section = AdminModel.SECTION_SERVER, theme = NzThemeKind.Papier)

    @Test
    fun serverAtTwoHundredPercent() = capture(
        section = AdminModel.SECTION_SERVER,
        theme = NzThemeKind.Papier,
        fontScale = 2f,
        name = "mb17-server-papier-%ddp-font200.png".format(width),
    )
}

/** 360 dp: the phone the app is designed for first. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h2400dp-xhdpi")
class AdminScreenshotPhoneTest : AdminScreenshotBase(width = 360)

/** 600 dp: the tablet shell's reading column. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w600dp-h1600dp-xhdpi")
class AdminScreenshotTabletTest : AdminScreenshotBase(width = 600)
