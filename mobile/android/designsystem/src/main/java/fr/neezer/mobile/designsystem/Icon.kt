package fr.neezer.mobile.designsystem

import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon as MaterialIcon
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * One Sillon glyph, drawn from the paths of `webapp/src/components/Icon.svelte`
 * ([NzGlyph], generated) and tinted with the theme. Never an emoji, never the
 * Material icon font: a glyph the SPA does not have does not exist here either
 * (rule 5).
 *
 * @param contentDescription the label a screen reader reads. An icon that only
 *   decorates a written label passes `null` (it is `aria-hidden`).
 */
@Composable
fun Icon(
    glyph: NzGlyph,
    modifier: Modifier = Modifier,
    size: Dp = 20.dp,
    tint: Color? = null,
    contentDescription: String? = null,
) {
    MaterialIcon(
        imageVector = nzVector(glyph),
        contentDescription = contentDescription,
        modifier = modifier.size(size),
        tint = tint ?: Sillon.colors.tx,
    )
}
