package fr.neezer.mobile.feature.discovery

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon

/**
 * The search box the SPA draws (`ReleaseShelf.svelte`'s `.searchbox`,
 * `ExplorerStyles.svelte`'s `.allsearch`): a token surface with a glyph, no
 * Material field — a default-looking control is what rule 5 forbids, and a
 * text field's own underline would be the only one on the screen.
 */
@Composable
fun SearchField(
    query: String,
    onQuery: (String) -> Unit,
    modifier: Modifier = Modifier,
    placeholder: String = "Rechercher…",
    label: String = placeholder,
) {
    val colors = Sillon.colors
    val type = Sillon.type
    Row(
        modifier = modifier
            .fillMaxWidth()
            .heightIn(min = NzMinTouchTarget)
            .clip(RoundedCornerShape(NzRadius.field))
            .background(colors.s1)
            .border(1.dp, colors.ln, RoundedCornerShape(NzRadius.field))
            .padding(horizontal = NzSpacing.m)
            .semantics { contentDescription = label },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Icon(NzGlyph.Search, size = 16.dp, tint = colors.dim)
        Box(Modifier.weight(1f)) {
            if (query.isEmpty()) {
                Text(placeholder, style = type.body, color = colors.dim)
            }
            BasicTextField(
                value = query,
                onValueChange = onQuery,
                singleLine = true,
                textStyle = type.body.copy(color = colors.tx),
                cursorBrush = androidx.compose.ui.graphics.SolidColor(colors.acText),
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}
