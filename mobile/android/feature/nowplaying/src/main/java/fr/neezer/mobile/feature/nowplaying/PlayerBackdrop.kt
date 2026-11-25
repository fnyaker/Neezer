package fr.neezer.mobile.feature.nowplaying

import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.BlurredEdgeTreatment
import androidx.compose.ui.draw.blur
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.Sillon

/**
 * The sleeve under the player: a blurred, scaled copy of the current cover, dimmed
 * by the gradient below it (R3, the SPA's liquid-glass ground).
 *
 * **Why a 120 dp copy and not the full-screen one.** `Modifier.blur` renders its
 * content into a separate graphics layer, so its cost is the layer's pixel AREA
 * (S6). The full-screen layer that shipped in MB11 cost the player's first frame
 * p95 **775 ms** on the emulator's SwiftShader (MQ2, 2026-11-24) and was removed.
 * A 120 dp square is ~1/25th of that area, and it is scaled up to cover the screen
 * (`graphicsLayer { scale }`): the result is the same soft field for a texture
 * pipeline that is compiled once. Below API 31 `blur` is ignored (S6), so the
 * fallback is exactly the old picture: the sleeve at 900 dp.
 *
 * **`CoverImage(url, 120.dp)` costs one extra decode and ZERO network.**
 * `Core::cover_path` is keyed by the URL only (`nz-core/src/core/covers.rs`), so
 * the file is a cache hit; the size only picks the Coil entry. Do not "optimise"
 * this into a second fetch.
 *
 * The cover is drawn with `RectangleShape`: the image is scaled up by 3–8×, and
 * the design system's rounded sleeve corners would scale with it into four
 * visible notches on a backdrop that must be seamless.
 */
@Composable
internal fun PlayerBackdrop(cover: String?, seed: String?) {
    if (cover == null) return
    val blur = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
    BoxWithConstraints(Modifier.fillMaxSize()) {
        if (blur) {
            val scale = (maxOf(maxWidth, maxHeight) / 120.dp).coerceAtLeast(1f)
            Box(
                Modifier
                    .size(120.dp)
                    .align(Alignment.Center)
                    // Scale OUTSIDE the blur: `Modifier.blur` renders into a layer
                    // bounded by the node's 120 dp size and clips to it, so a scale
                    // applied inside that layer is cropped back to 120 dp (measured
                    // on the P1 capture: a small dark rectangle behind the title
                    // instead of a full-screen field). The outer layer has no clip.
                    .graphicsLayer {
                        scaleX = scale
                        scaleY = scale
                    }
                    .blur(radius = 12.dp, edgeTreatment = BlurredEdgeTreatment.Rectangle),
            ) {
                CoverImage(url = cover, sizeDp = 120.dp, seed = seed, shape = RectangleShape)
            }
        } else {
            CoverImage(url = cover, sizeDp = 900.dp, seed = seed)
        }
        Box(
            Modifier
                .fillMaxSize()
                .background(
                    Brush.verticalGradient(
                        listOf(
                            Sillon.colors.bg.copy(alpha = 0.72f),
                            Sillon.colors.bg.copy(alpha = 0.94f),
                        ),
                    ),
                ),
        )
    }
}
