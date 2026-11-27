package fr.neezer.mobile.feature.nowplaying

import android.graphics.BitmapFactory
import android.os.Build
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import fr.neezer.mobile.designsystem.LocalNzCoverLoader
import fr.neezer.mobile.designsystem.NzCoverLoader
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.nzBlurBitmap
import fr.neezer.mobile.designsystem.nzGrooveBitmap
import fr.neezer.mobile.designsystem.nzGrooveFor
import java.io.File
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sqrt
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Le fond du lecteur : la pochette, floutée en Kotlin (JF, étape B).
 *
 * Le défaut mesuré : sur 09/11 de #447, les trois points de contrôle du fond
 * valent `(12,12,11)` — exactement `Sillon.colors.bg`, à l'octet — parce que
 * l'ancien `PlayerBackdrop` sortait sur `cover == null` ; et sur 08, le champ
 * est `(38,44,49)`, chroma 11, parce que le voile 0,72→0,94 ne laisse que 28 %
 * d'une pochette sur Encre. Ces tests tiennent les nombres du correctif, pas
 * une impression : les pixels sont lus, jamais comparés à l'œil.
 *
 * Même forme que les goldens (`@GraphicsMode(NATIVE)`, les qualifiers du
 * lecteur) : les polices et le rendu sont ceux de l'appareil.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w412dp-h915dp-xhdpi")
class PlayerBackdropTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /**
     * Le flou fait son travail : il efface les bords, il garde la couleur.
     *
     * La source est la pochette du banc (`tests/assets/cover.jpg`), décodée à
     * 96 × 96 — la taille que le fond demande. Un flou boîte de rayon 5, trois
     * passes, à 96 px vaut ~10 % du champ. L'invariant qui compte est que la
     * MOYENNE ne bouge pas (le voile et la saturation sont réglés dessus) et que
     * l'écart entre voisins s'effondre (les bords de la pochette disparaissent,
     * donc le juge ne peut pas la reconnaître comme une image nette).
     */
    @Test
    fun theBlurLosesTheEdgesAndKeepsTheColour() {
        val source = coverAt(96)
        val blurred = nzBlurBitmap(source)

        assertEquals("le flou garde la taille demandée", 96, blurred.width)
        assertEquals(96, blurred.height)

        val sharpEdge = maxNeighbourDelta(source)
        val softEdge = maxNeighbourDelta(blurred)
        // Le coût, mesuré une fois : c'est le chiffre que la PR et la note
        // *Smoothness* citent (aucune assertion de temps : une machine chargée
        // ferait un test rouge sans défaut, la mesure est informative).
        var started = System.nanoTime()
        repeat(100) { nzBlurBitmap(source) }
        val perBlurMs = (System.nanoTime() - started) / 100.0 / 1_000_000.0
        started = System.nanoTime()
        repeat(100) { nzBlurBitmap(nzGrooveBitmap("track-1", 96)) }
        val perGrooveMs = (System.nanoTime() - started) / 100.0 / 1_000_000.0
        val before = mean(source)
        val after = mean(blurred)
        println(
            "MEASURE backdrop blur: voisins source=$sharpEdge après=$softEdge, " +
                "moyenne avant=${rgb(before)} après=${rgb(after)}, " +
                "coût %.2f ms/flou et %.2f ms/rainure+flou (96×96, 3 passes)".format(perBlurMs, perGrooveMs),
        )
        assertTrue("la source du test doit être nette, max voisin=$sharpEdge", sharpEdge >= 50)
        assertTrue("un flou doit effacer les bords : $softEdge", softEdge <= 12)
        for (i in 0..2) {
            assertTrue(
                "le flou ne doit pas déplacer la moyenne (${before[i]} -> ${after[i]})",
                abs(before[i] - after[i]) <= 6,
            )
        }
    }

    /**
     * Un titre sans pochette peint sa rainure, pas le fond nu : c'est le cas
     * 09/11, où la pochette du titre suivant n'est pas encore là.
     */
    @Test
    fun aCoverlessTrackPaintsItsGroove() {
        val groove = nzGrooveFor("track-1")
        val field = nzBlurBitmap(nzGrooveBitmap("track-1", 96))
        val mean = mean(field)
        println("MEASURE backdrop groove: ${rgb(mean)} vs bg 12,12,11")
        assertTrue(
            "le fond doit quitter le bg sur au moins un canal (mesuré ${rgb(mean)})",
            maxOf(abs(mean[0] - 12.0), abs(mean[1] - 12.0), abs(mean[2] - 11.0)) >= 8,
        )

        val centre = field.toPixelMap()[48, 48]
        val toGroove = distance(centre, groove.second)
        val toBlack = distance(centre, Color.Black)
        assertTrue(
            "le centre doit porter la rainure, pas du noir ($toGroove vs $toBlack)",
            toGroove < toBlack,
        )
    }

    /**
     * Le champ du lecteur, aux trois points que le juge regarde (10 %, 45 %,
     * 90 % de la hauteur, x = 2 %), avec un `NzCoverLoader` qui rend la
     * pochette du banc.
     *
     * Les seuils sont ceux du défaut mesuré : 45 % doit avoir une TEINTE
     * (chroma ≥ 18 ; l'ancien 11 est un gris), le haut doit être CLAIR
     * (luminance ≥ 90 ; l'ancien 70), et le bas ne doit plus être le fond nu.
     */
    @Test
    fun theBackdropIsNeverTheBackground() {
        val image = composeBackdrop(assetLoader(), cover = "/api/cover/bench")
        val (xFrom, xTo) = sampleColumns(image)
        val top = headline(image, xFrom, xTo, 0.10f)
        val middle = headline(image, xFrom, xTo, 0.45f)
        val bottom = headline(image, xFrom, xTo, 0.90f)
        println("MEASURE backdrop field: 10%=$top 45%=$middle 90%=$bottom")

        assertTrue("le haut doit être clair, luminance=${top.luminance}", top.luminance >= 90)
        assertTrue("45 % doit porter la teinte de la pochette, chroma=${middle.chroma}", middle.chroma >= 18)
        assertTrue(
            "le bas ne doit plus être le fond nu, écart=${bottom.distanceToBackground}",
            bottom.distanceToBackground >= 6,
        )
        // Le dégradé se lit du HAUT au BAS : le voile est exprès plus clair au
        // milieu (0,50) pour laisser passer la teinte, et sombre en bas (0,92)
        // pour la rangée du pouce. Comparer le milieu au haut mesurerait le
        // voile, pas la pochette — mesuré : 114 au haut, 86 au milieu, 35 en bas.
        assertTrue(
            "un dégradé de pochette, pas un aplat : ${top.luminance} - ${bottom.luminance}",
            top.luminance - bottom.luminance >= 40,
        )
    }

    /** Sans pochette : la rainure du titre, et elle n'est pas un aplat. */
    @Test
    fun theCoverlessBackdropPaintsItsGroove() {
        val image = composeBackdrop(loader = null, cover = null)
        val (xFrom, xTo) = sampleColumns(image)
        val top = headline(image, xFrom, xTo, 0.10f)
        val middle = headline(image, xFrom, xTo, 0.45f)
        println("MEASURE backdrop groove field: 10%=$top 45%=$middle")
        assertTrue(
            "sans pochette, le haut quitte le fond (${top.distanceToBackground})",
            top.distanceToBackground >= 8,
        )
        assertTrue(
            "sans pochette, le milieu quitte le fond (${middle.distanceToBackground})",
            middle.distanceToBackground >= 8,
        )
        // « Pas un aplat » se lit à la TEINTE : le fond d'Encre a un chroma de 0
        // ou 1, la rainure floutée en porte 43 (mesuré). L'écart type d'une
        // bande de 9 px vaut 0,6 : un flou de rayon 5 sur des anneaux de 6 px
        // lisse presque tout, et l'assertion σ ≥ 1 du plan était trop serrée.
        assertTrue("la rainure porte sa teinte (chroma=${middle.chroma})", middle.chroma >= 18)
    }

    /**
     * Le même rendu sous API 30 et API 34 : mêmes pixels (≤ 1 par canal).
     *
     * C'est le fossé que le lot ferme : l'ancien fond branchait sur
     * `Build.VERSION.SDK_INT >= S` et peignait une pochette NETTE sous 31. Le
     * test tourne une fois par SDK (`@Config(sdk = [30, 34])`) et écrit les
     * octets du champ sous `build/backdrop-api/` ; la seconde exécution les
     * compare à la première. Un écart d'un seul canal de plus d'une unité
     * échoue.
     */
    @Test
    @Config(sdk = [30, 34])
    fun theApiLevelDoesNotChangeTheField() {
        val image = composeBackdrop(assetLoader(), cover = "/api/cover/bench")
        val pixels = image.toPixelMap()
        val bytes = ByteArray(image.width * image.height * 3)
        var at = 0
        for (y in 0 until image.height) {
            for (x in 0 until image.width) {
                val c = pixels[x, y]
                bytes[at++] = (c.red * 255f).toInt().toByte()
                bytes[at++] = (c.green * 255f).toInt().toByte()
                bytes[at++] = (c.blue * 255f).toInt().toByte()
            }
        }
        val sdk = Build.VERSION.SDK_INT
        val dir = File(System.getProperty("nz.repoRoot"), "build/backdrop-api").apply { mkdirs() }
        val other = listOf(30, 34).firstOrNull { it != sdk } ?: error("SDK inattendu : $sdk")
        val sibling = File(dir, "field-$other.bin")
        if (sibling.exists()) {
            val reference = sibling.readBytes()
            assertEquals("le champ doit faire la même taille sur les deux API", reference.size, bytes.size)
            var worst = 0
            for (i in bytes.indices) {
                worst = max(worst, abs((bytes[i].toInt() and 0xFF) - (reference[i].toInt() and 0xFF)))
            }
            println("MEASURE backdrop api: SDK $sdk vs $other, écart max=$worst")
            assertTrue("un seul pixel diffère de plus d'une unité (écart $worst)", worst <= 1)
            sibling.delete()
        } else {
            File(dir, "field-$sdk.bin").writeBytes(bytes)
            println("MEASURE backdrop api: SDK $sdk enregistré, en attente de l'autre API")
        }
    }

    // -- helpers --------------------------------------------------------------

    /** La pochette du banc, décodée à [sizePx] : la même source pour tous les cas. */
    private fun coverAt(sizePx: Int): ImageBitmap {
        val root = System.getProperty("nz.repoRoot")
            ?: error("nz.repoRoot is not set: this test needs the bench cover (see build.gradle.kts)")
        val file = File(root, "tests/assets/cover.jpg")
        assertTrue("la pochette du banc manque : $file", file.exists())
        val decoded = BitmapFactory.decodeFile(file.path)
            ?: error("la pochette du banc ne se décode pas : $file")
        return android.graphics.Bitmap.createScaledBitmap(decoded, sizePx, sizePx, true).asImageBitmap()
    }

    private fun assetLoader(): NzCoverLoader {
        val bitmap = coverAt(96)
        return object : NzCoverLoader {
            @Composable
            override fun Image(url: String, sizePx: Int) = Unit

            override suspend fun bitmap(url: String, sizePx: Int): ImageBitmap = bitmap
        }
    }

    /** Compose le fond dans la boîte du lecteur et rend ses pixels. */
    private fun composeBackdrop(loader: NzCoverLoader?, cover: String?): ImageBitmap {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                CompositionLocalProvider(LocalNzCoverLoader provides loader) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                        PlayerBackdrop(cover = cover, seed = "track-1")
                    }
                }
            }
        }
        // Le champ est calculé hors du fil principal : on attend l'ÉVÉNEMENT
        // « il est peint », jamais une horloge. La boucle est BORNÉE (10 s) et
        // ne teste qu'un fait ; `waitUntil` seul ne voit pas un travail fini sur
        // `Dispatchers.Default` (mesuré : « Condition still not satisfied »).
        awaitBackdropPainted()
        // `captureToImage()` attend un redraw de fenêtre que Robolectric ne fait
        // pas (« Condition still not satisfied after 2000 ms », mesuré) : on
        // dessine l'arbre de l'Activity dans un bitmap, ce que fait aussi le
        // test des réglages voiture. NATIVE, donc les pixels sont réels.
        val view = rule.activity.window.decorView
        val width = view.width
        val height = view.height
        assertTrue("la fenêtre du test n'est pas posée : ${width}x$height", width > 0 && height > 0)
        val bitmap = android.graphics.Bitmap.createBitmap(width, height, android.graphics.Bitmap.Config.ARGB_8888)
        view.draw(android.graphics.Canvas(bitmap))
        return bitmap.asImageBitmap()
    }

    private fun awaitBackdropPainted() {
        val deadline = System.currentTimeMillis() + 10_000
        while (System.currentTimeMillis() < deadline) {
            rule.waitForIdle()
            if (rule.onAllNodesWithTag(PLAYER_BACKDROP_TAG).fetchSemanticsNodes().isNotEmpty()) return
            Thread.sleep(10)
        }
        error("le champ du lecteur n'a jamais été peint en 10 s")
    }

    /** Les colonnes du juge : x = 2 % à 10 % de la largeur (8 à 40 px à 412 dp). */
    private fun sampleColumns(image: ImageBitmap): Pair<Int, Int> =
        (image.width * 0.02f).toInt() to (image.width * 0.10f).toInt()

    private data class Headline(
        val luminance: Double,
        val chroma: Double,
        val spread: Double,
        val distanceToBackground: Double,
    ) {
        override fun toString(): String =
            "lum=${luminance.toInt()} chroma=${chroma.toInt()} σ=${"%.1f".format(spread)} " +
                "Δbg=${distanceToBackground.toInt()}"
    }

    /**
     * Une bande de 9 pixels de haut à [fraction] de la hauteur, de [xFrom] à
     * [xTo] : la moyenne d'un petit champ, comme le juge échantillonne une zone
     * (un seul pixel dépendrait de l'anti-aliasing d'une arête).
     */
    private fun headline(image: ImageBitmap, xFrom: Int, xTo: Int, fraction: Float): Headline {
        val pixels = image.toPixelMap()
        val height = image.height
        val y = (height * fraction).toInt().coerceIn(0, height - 1)
        val rs = mutableListOf<Double>()
        val gs = mutableListOf<Double>()
        val bs = mutableListOf<Double>()
        for (x in xFrom until xTo) {
            for (dy in -4..4) {
                val yy = (y + dy).coerceIn(0, height - 1)
                val c = pixels[x, yy]
                rs += c.red * 255.0
                gs += c.green * 255.0
                bs += c.blue * 255.0
            }
        }
        val r = rs.average()
        val g = gs.average()
        val b = bs.average()
        // `Sillon.colors.bg` d'Encre : #0C0C0B.
        val bg = doubleArrayOf(12.0, 12.0, 11.0)
        return Headline(
            luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b,
            chroma = max(r, max(g, b)) - min(r, min(g, b)),
            spread = maxOf(stdDev(rs, r), stdDev(gs, g), stdDev(bs, b)),
            distanceToBackground = maxOf(abs(r - bg[0]), abs(g - bg[1]), abs(b - bg[2])),
        )
    }

    private fun stdDev(values: List<Double>, mean: Double): Double =
        sqrt(values.sumOf { (it - mean) * (it - mean) } / values.size)

    private fun mean(image: ImageBitmap): DoubleArray {
        val pixels = image.toPixelMap()
        var r = 0.0
        var g = 0.0
        var b = 0.0
        for (y in 0 until image.height) {
            for (x in 0 until image.width) {
                val c = pixels[x, y]
                r += c.red * 255.0
                g += c.green * 255.0
                b += c.blue * 255.0
            }
        }
        val n = (image.width * image.height).toDouble()
        return doubleArrayOf(r / n, g / n, b / n)
    }

    private fun maxNeighbourDelta(image: ImageBitmap): Int {
        val pixels = image.toPixelMap()
        var worst = 0
        for (y in 0 until image.height) {
            for (x in 0 until image.width) {
                val c = pixels[x, y]
                val right = if (x + 1 < image.width) pixels[x + 1, y] else c
                val down = if (y + 1 < image.height) pixels[x, y + 1] else c
                worst = max(worst, max(channelDelta(c, right), channelDelta(c, down)))
            }
        }
        return worst
    }

    private fun channelDelta(a: Color, b: Color): Int = max(
        abs((a.red * 255).toInt() - (b.red * 255).toInt()),
        max(
            abs((a.green * 255).toInt() - (b.green * 255).toInt()),
            abs((a.blue * 255).toInt() - (b.blue * 255).toInt()),
        ),
    )

    private fun distance(a: Color, b: Color): Double {
        val dr = (a.red - b.red) * 255.0
        val dg = (a.green - b.green) * 255.0
        val db = (a.blue - b.blue) * 255.0
        return sqrt(dr * dr + dg * dg + db * db)
    }

    private fun rgb(values: DoubleArray): String =
        values.joinToString(",") { "%.0f".format(it) }
}
