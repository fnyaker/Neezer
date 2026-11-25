package fr.neezer.mobile.feature.nowplaying

import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.PagerDefaults
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.Sillon
import kotlin.math.abs
import uniffi.nz_ffi.TrackRow

/**
 * The sleeve, its neighbours peeking: a swipe changes track, as in the SPA.
 *
 * The square is the SMALLER of 80 % of the width and 48 % of the room the block
 * has (the SPA's `min(78vw, 42vh)`), so a 360 × 640 phone shrinks the cover
 * instead of pushing the transport off the screen, and a 600 dp window does not
 * grow a sleeve taller than the column it sits in.
 *
 * **The strip is the queue, one page per track** (P1). The pager used to be a
 * fixed 3 pages keyed by POSITION, and a settle rewrote it to page 1
 * (`scrollToPage`): the page the person had just swiped to was destroyed and
 * rebuilt as the centre, and `neighboursOf()` asked three new sleeves on every
 * track change. Now:
 *
 *  * the page count is the queue's length and the page IS the queue index; the
 *    strip composes only the pages it shows — the current one and the
 *    `pageSpacing` slivers of its neighbours (the SPA's peek), which is why a
 *    `beyondViewportPageCount` of 1 is NOT used here: it composes a fourth,
 *    never-seen sleeve on every change (measured: **+2 decodes per change**
 *    instead of +1, `PlayerCoverReuseTest`). The peek is the precompose;
 *  * each page is keyed by the track's own key, so an advance reuses the page it
 *    swiped to instead of recomposing it (Coil's model is a data class for the
 *    same reason: an equal request is not restarted, `Covers.kt`);
 *  * **one effect per direction**: the player's index moves the pager (animate
 *    for a one-step change, jump for a seek), and a settled page asks the player
 *    to jump. A swipe never rewrites the scroll position afterwards — a gesture
 *    keeps the frame it got from the finger (plan §6 rule 4);
 *  * `CoverImage(url, sizeDp = size)` draws at the slot's own size (M12): the
 *    old `420.dp` forced a 420 dp square inside a ≈288 dp slot and the `Box`
 *    cropped it at the top-start, so the sleeve's optical centre was off.
 */
@Composable
internal fun CoverStage(state: NowPlayingModel.State, model: NowPlayingModel, wide: Boolean) {
    val colors = Sillon.colors
    val pages = maxOf(1, state.len)
    val pager = rememberPagerState(initialPage = state.index.coerceIn(0, pages - 1)) { pages }
    val rows = state.queue

    fun rowAt(page: Int): TrackRow? = rows?.window(page, 1)?.firstOrNull()

    /** The queue key a page plays, when the queue can still answer for it. */
    fun trackKeyAt(page: Int): String? =
        if (page == state.index) state.current?.key else rowAt(page)?.key

    // A pager requires unique keys: the queue may hold the same track twice, so
    // the track key is qualified by its queue position (stable across an advance
    // — the advance moves the playhead, not the queue).
    fun pageKey(page: Int): String = "${trackKeyAt(page) ?: "page-$page"}@$page"

    // The player's index moves the pager. A one-step change (next/prev) animates;
    // anything else (a seek from the queue pane) lands at once.
    LaunchedEffect(state.index, state.len) {
        val target = state.index
        if (target !in 0 until pages || pager.currentPage == target) return@LaunchedEffect
        if (abs(target - pager.currentPage) == 1) {
            pager.animateScrollToPage(target)
        } else {
            pager.scrollToPage(target)
        }
    }

    // A settled page that is not the playhead is a swipe: play THAT entry. It
    // never scrolls the strip afterwards — the finger's position stays.
    LaunchedEffect(pager.settledPage) {
        val target = pager.settledPage
        if (state.len <= 0 || target == state.index || target !in 0 until pages) return@LaunchedEffect
        val key = trackKeyAt(target) ?: return@LaunchedEffect
        model.jump(target, key)
    }

    BoxWithConstraints(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        val fraction = if (wide) 0.62f else 0.80f
        val size = minOf(maxWidth * fraction, maxHeight * 0.48f)
        HorizontalPager(
            state = pager,
            // Only what the strip shows: the current sleeve and the two peeking
            // slivers. `beyondViewportPageCount = 1` composes one more never-seen
            // sleeve on each side (measured: +2 decodes per track change).
            beyondViewportPageCount = 0,
            modifier = Modifier.fillMaxWidth().height(size),
            pageSpacing = 14.dp,
            contentPadding = PaddingValues(horizontal = ((maxWidth - size) / 2).coerceAtLeast(0.dp)),
            key = { page -> pageKey(page) },
            flingBehavior = PagerDefaults.flingBehavior(
                state = pager,
                snapAnimationSpec = spring(
                    dampingRatio = Spring.DampingRatioNoBouncy,
                    stiffness = Spring.StiffnessMediumLow,
                ),
                snapPositionalThreshold = 0.28f,
            ),
        ) { page ->
            val current = page == state.index
            val row = if (current) null else rowAt(page)
            val cover = if (current) state.current?.cover else row?.cover
            val seed = if (current) state.current?.id else row?.id
            Box(
                Modifier
                    .fillMaxHeight()
                    .aspectRatio(1f)
                    .testTag("player-cover-slot")
                    .clip(RoundedCornerShape(NzRadius.cover))
                    .background(colors.s1),
                contentAlignment = Alignment.Center,
            ) {
                CoverImage(
                    url = cover,
                    // The slot's own size: `size` is the pager's height, and the
                    // page is square (M12).
                    sizeDp = size,
                    seed = seed,
                    modifier = Modifier.testTag("player-cover"),
                )
            }
        }
    }
}
