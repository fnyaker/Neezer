package fr.neezer.mobile.designsystem

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/**
 * One destination of the phone's tab bar: the glyph a person aims at and the word
 * under it. The word is also the [contentDescription] (an item is never carried by
 * its picture alone).
 */
data class NzTabItem(val glyph: NzGlyph, val label: String)

/**
 * The phone's bottom bar — `MobileNav.svelte`: five items, a glyph over a word,
 * an accent pill behind the item the person is on (Sillon's `--ac`, the SPA's
 * 46 × 26 pill), `NzSpacing.tabBar` tall, and nothing else.
 *
 * Why it is not Material's `NavigationBar`: that one draws its own 80 dp surface,
 * its own indicator and its own ripple, three things the SPA does not have. Rule 5
 * is the look of the app a senior team shipped, and the SPA is that look.
 *
 * Layout facts a test holds (the SPA's own numbers, `MobileNav.svelte:78-140`):
 *  * the row is exactly 64 dp (`NzSpacing.tabBar`) plus the system navigation-bar
 *    inset, which the bar pads for — edge-to-edge is on (`MainActivity`);
 *  * each item is `weight(1f)`, so five items share a 360 dp bar as 72 dp each;
 *  * the pill is 46 × 26 with `NzRadius.chip`, a press paints
 *    `nzMix(ac, s2, 0.7f)` on the instant token, the selected one `--ac`;
 *  * the glyph is 22 dp, `--onac` inside the pill and `--dim` outside;
 *  * at `fontScale > 1.3` the word is dropped and the glyph stays — the
 *    contentDescription still carries the word, so TalkBack never loses it.
 *
 * What it does NOT do: navigate. Taps leave through [onPick] with an index, and
 * the shell decides what an index means (a page, or the Réglages door).
 */
@Composable
fun NzTabBar(
    items: List<NzTabItem>,
    selected: Int,
    onPick: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val showLabels = LocalDensity.current.fontScale <= 1.3f
    Column(modifier.fillMaxWidth().navigationBarsPadding()) {
        // The SPA's `border-top: 1px solid var(--ln)`.
        Box(Modifier.fillMaxWidth().height(1.dp).background(colors.ln))
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .height(NzSpacing.tabBar)
                .background(colors.s1),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            items.forEachIndexed { index, item ->
                NzTab(
                    item = item,
                    selected = index == selected,
                    showLabel = showLabels,
                    onClick = { onPick(index) },
                )
            }
        }
    }
}

@Composable
private fun RowScope.NzTab(
    item: NzTabItem,
    selected: Boolean,
    showLabel: Boolean,
    onClick: () -> Unit,
) {
    val colors = Sillon.colors
    val motion = Sillon.motion
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    // The press answers on the PILL, never by shrinking the item: a label that
    // jumps under the finger is the SPA's own rejected variant (`MobileNav.svelte`
    // « A press is answered on the pill »). `--dur-instant` is the token, so eco
    // and reduced motion land at once.
    val pill by animateColorAsState(
        targetValue = when {
            pressed -> nzMix(colors.ac, colors.s2, 0.7f)
            selected -> colors.ac
            else -> Color.Transparent
        },
        animationSpec = motion.tween(NzDur.Instant, NzEase.Standard),
        label = "nz-tab-pill",
    )
    val ink = if (selected) colors.tx else colors.dim
    Column(
        modifier = Modifier
            .weight(1f)
            .fillMaxHeight()
            .clickable(
                interactionSource = interaction,
                indication = null,
                role = Role.Tab,
                onClick = onClick,
            )
            .semantics {
                this.selected = selected
                contentDescription = item.label
                role = Role.Tab
            },
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(3.dp, Alignment.CenterVertically),
    ) {
        Box(
            modifier = Modifier
                .size(width = 46.dp, height = 26.dp)
                .clip(RoundedCornerShape(NzRadius.chip))
                .background(pill),
            contentAlignment = Alignment.Center,
        ) {
            // The glyph is decoration: the item's contentDescription carries the
            // word, so a screen reader reads one label and not two.
            Icon(
                glyph = item.glyph,
                size = 22.dp,
                tint = if (selected) colors.onac else colors.dim,
                contentDescription = null,
            )
        }
        if (showLabel) {
            Text(
                text = item.label,
                style = Sillon.type.tab,
                color = ink,
                maxLines = 1,
                overflow = TextOverflow.Clip,
            )
        }
    }
}
