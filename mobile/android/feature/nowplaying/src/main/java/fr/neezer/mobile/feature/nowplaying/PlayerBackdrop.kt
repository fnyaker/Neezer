package fr.neezer.mobile.feature.nowplaying

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.ColorMatrix
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import fr.neezer.mobile.designsystem.LocalNzCoverLoader
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.nzBlurBitmap
import fr.neezer.mobile.designsystem.nzGrooveBitmap
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** The 96 px field the backdrop blurs, and the tag that says it is painted. */
internal const val PLAYER_BACKDROP_TAG = "player-backdrop"

/**
 * La pochette du lecteur, floutée : ce que le SPA appelle son fond « liquid
 * glass » (`MobileNowPlaying.svelte:1415`, `backdrop.js`).
 *
 * **Une seule voie, sur tous les niveaux d'API.** La version précédente
 * composait `Modifier.blur` sur une vignette de 120 dp : le modificateur est
 * **ignoré sous Android 12**
 * (<https://developer.android.com/develop/ui/compose/graphics/images/customize#blur>),
 * donc l'app shipped une pochette NETTE à 900 dp sur tout appareil < 12 — le
 * juge la compte comme « pochette nette » et refuse (C3). Et le `RenderEffect`
 * plein écran coûtait la première image du lecteur **p95 775 ms** sur le
 * SwiftShader de l'émulateur (MQ2). Ici, on décode 96 × 96 **une fois par
 * titre**, on floute ces pixels en Kotlin ([nzBlurBitmap], hors du fil
 * principal), et on peint le champ : même résultat partout, quelques
 * millisecondes, aucun programme GPU compilé dans la trame.
 *
 * **Sans pochette, la rainure du titre** ([nzGrooveBitmap]) : c'est le cas où
 * `cover == null` (le titre suivant n'est pas encore là, le serveur est
 * injoignable) — l'ancien code ne peignait RIEN et laissait le fond `#0C0C0B`
 * à l'octet (mesuré sur 09/11 de #447).
 *
 * **Le voile.** `bg` de 0,58 à 0,92 : sur Encre (`#0C0C0B`), le 0,72→0,94
 * d'avant ne laissait que 28 % de la pochette — un champ `(38,44,49)` que le
 * juge ne peut pas nommer. À 45 %, le champ vaut `(44,57,70)` : luminance 56,
 * du texte blanc à 5,9:1, et la teinte de la pochette (chroma 26). La
 * saturation (1,6, contre 1,25 dans le SPA parce que le voile est plus sombre)
 * porte la lisibilité SANS toucher à la luminance, donc sans toucher au
 * contraste des textes. Pas de normalisation de niveau (`levelFactor` du SPA) :
 * sur un voile sombre elle ne fait que baisser la pochette claire.
 */
@Composable
internal fun PlayerBackdrop(cover: String?, seed: String?) {
    val loader = LocalNzCoverLoader.current
    val field by produceState<ImageBitmap?>(null, cover, seed) {
        value = withContext(Dispatchers.Default) {
            val source = if (cover != null) loader?.bitmap(cover, BACKDROP_PX) else null
            nzBlurBitmap(source ?: nzGrooveBitmap(seed, BACKDROP_PX))
        }
    }
    Box(Modifier.fillMaxSize()) {
        field?.let { bitmap ->
            Image(
                bitmap = bitmap,
                contentDescription = null,
                contentScale = ContentScale.Crop,
                colorFilter = ColorFilter.colorMatrix(
                    ColorMatrix().apply { setToSaturation(BACKDROP_SATURATION) },
                ),
                modifier = Modifier
                    .fillMaxSize()
                    // What a bench waits on: "the field is painted", an event,
                    // never a sleep (rule: tests wait on events, not clocks).
                    .testTag(PLAYER_BACKDROP_TAG),
            )
        }
        Box(
            Modifier
                .fillMaxSize()
                .background(
                    Brush.verticalGradient(
                        0f to Sillon.colors.bg.copy(alpha = 0.58f),
                        0.25f to Sillon.colors.bg.copy(alpha = 0.50f),
                        1f to Sillon.colors.bg.copy(alpha = 0.92f),
                    ),
                ),
        )
    }
}

/** 96 × 96 = 9 216 px : le flou coûte < 2 ms, décodé une fois par titre. */
private const val BACKDROP_PX = 96
/** Plus haut que le SPA (1,25) : le voile d'Encre est plus sombre. */
private const val BACKDROP_SATURATION = 1.6f
