package fr.neezer.mobile.feature.entity

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzSkeletonKind
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.Skeleton
import fr.neezer.mobile.designsystem.nzPaneEnter
import fr.neezer.mobile.designsystem.rememberPaneEnter
import uniffi.nz_ffi.TrackRow

/** The screen's marker, in every state (see [ARTIST_MARKER]). */
const val CREDITS_MARKER = "Crédits"

/** S2's geometry test: the band (it cancels the page's top margin) and the list. */
const val CREDITS_BAND_TAG = "credits-band"
const val CREDITS_LIST_TAG = "credits-list"

/**
 * What the list is drawing — the pane's key (D2), from the same fields its
 * branches test.
 */
private enum class CreditsListPhase { Loading, Failed, Empty, Content }

private fun creditsListPhase(state: PersonModel.State): CreditsListPhase {
    val page = state.data
    return when {
        state.screen is ScreenState.Failed -> CreditsListPhase.Failed
        page == null -> CreditsListPhase.Loading
        page.count == 0 -> CreditsListPhase.Empty
        else -> CreditsListPhase.Content
    }
}

/** Everything a person's credits page can do. */
data class CreditsActions(
    val retry: () -> Unit = {},
    /** A role chip: the list already in hand, narrowed. */
    val role: (String) -> Unit = {},
    val playAll: () -> Unit = {},
    val playFrom: (TrackRow) -> Unit = {},
    val queueNext: (TrackRow) -> Unit = {},
    val more: (TrackRow) -> Unit = {},
)

/**
 * Crédits — lot MB7, `routes/Credits.svelte` and `components/PersonCredits.svelte`
 * ported to Compose.
 *
 * What one person wrote, composed or produced among the library's titles — not
 * what they performed (that is their artist page). The round plaque, the name,
 * « N titres de la bibliothèque », then « Tout lire » and one chip per role; the
 * chips NARROW the list already on the device, so choosing one costs no request.
 *
 * ONE request opens the page (`/credits/person?name=`), and the list is windowed:
 * a songwriter credited on four hundred titles costs the rows on screen.
 */
@Composable
fun CreditsScreen(
    state: PersonModel.State,
    actions: CreditsActions,
    modifier: Modifier = Modifier,
    /** The playing track's key, when the shell knows it (MB11 feeds it). */
    playingKey: String? = null,
) {
    val colors = Sillon.colors
    // The list's phases are one pane (D2): its key is the phase, so a skeleton →
    // list, a failure and an empty page enter while the band stays put (the band
    // is drawn in every state, so it is not part of what changes).
    val pane = rememberPaneEnter(creditsListPhase(state))
    val enter = Modifier.nzPaneEnter(pane)
    // Read in the composition: `items` runs in the LazyColumn's scope, where a
    // `CompositionLocal` cannot be read.
    val specs = Sillon.motion.itemSpecs()
    BoxWithConstraints(
        modifier
            .fillMaxSize()
            .semantics { contentDescription = CREDITS_MARKER },
    ) {
        val wide = maxWidth >= 600.dp
        val listState = rememberLazyListState()
        val window = state.tracks?.let { source -> remember(source, state.name) { windowFor(source) } }
        window?.let { ReportWindow(listState, it, offset = 2) }

        LazyColumn(
            state = listState,
            modifier = Modifier.fillMaxSize().background(colors.bg).testTag(CREDITS_LIST_TAG),
            contentPadding = PaddingValues(bottom = NzSpacing.pageBottom),
        ) {
            item(key = "band") {
                CreditsBand(state, wide)
                if (state.stale) {
                    EntityStaleNote(Modifier.padding(horizontal = NzSpacing.pageX))
                }
            }
            val failed = state.screen as? ScreenState.Failed
            if (failed != null) {
                item(key = "failed") {
                    LoadFailed(
                        cause = failed.cause,
                        onRetry = actions.retry,
                        title = "Cette page de crédits n'a pas pu se charger",
                        modifier = enter.padding(horizontal = NzSpacing.pageX),
                    )
                }
                return@LazyColumn
            }
            val page = state.data
            if (page == null) {
                item(key = "skeleton") {
                    Skeleton(
                        kind = NzSkeletonKind.List,
                        count = 6,
                        modifier = enter.padding(horizontal = NzSpacing.pageX),
                    )
                }
                return@LazyColumn
            }
            if (page.count == 0) {
                item(key = "empty") {
                    CreditsEmptyNote(
                        emptyPersonSentence(page.name),
                        enter.padding(horizontal = NzSpacing.pageX),
                    )
                }
                return@LazyColumn
            }
            val pad = enter.padding(horizontal = NzSpacing.pageX)
            item(key = "bar") {
                PersonCreditsBar(
                    page = page,
                    role = state.role,
                    count = state.tracks?.len() ?: 0,
                    onRole = actions.role,
                    onPlayAll = actions.playAll,
                    modifier = pad.padding(vertical = NzSpacing.s),
                )
            }
            val count = state.tracks?.len() ?: 0
            // The key is the TRACK's identity, not its rank: an indexed key makes
            // every row below a removal a new item, so `animateItem` has nothing to
            // move (measured in X5). A person's credits list holds each title once,
            // so it is unique; a page that has not landed draws `pending-<index>`.
            items(count = count, key = { index -> window?.rowAt(index)?.key ?: "pending-$index" }) { index ->
                val row = window?.rowAt(index)
                if (row == null) {
                    Skeleton(
                        kind = NzSkeletonKind.List,
                        count = 1,
                        modifier = enter.animateItem(null, specs.placement, null).then(
                            Modifier.padding(horizontal = NzSpacing.pageX),
                        ),
                    )
                } else {
                    EntityTrackRow(
                        row = row,
                        playing = playingKey == row.key,
                        onClick = { actions.playFrom(row) },
                        onMore = { actions.more(row) },
                        onSwipeQueue = { actions.queueNext(row) },
                        modifier = enter.animateItem(specs.fadeIn, specs.placement, specs.fadeOut)
                            .then(Modifier.padding(horizontal = NzSpacing.pageX)),
                    )
                }
            }
        }
    }
}

/** The band: the round plaque, « CRÉDITS », the name and the title count. */
@Composable
private fun CreditsBand(state: PersonModel.State, wide: Boolean) {
    val colors = Sillon.colors
    val page = state.data
    val name = page?.name?.takeIf { it.isNotBlank() } ?: state.name
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .testTag(CREDITS_BAND_TAG)
            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.l),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            modifier = Modifier
                .size(if (wide) 168.dp else 120.dp)
                .clip(CircleShape)
                .background(colors.s1),
            contentAlignment = Alignment.Center,
        ) {
            Icon(glyph = NzGlyph.Users, size = if (wide) 64.dp else 48.dp, tint = colors.dim)
        }
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        ) {
            FactsLine("Crédits")
            Text(
                text = name,
                style = if (wide) Sillon.type.display else Sillon.type.displaySmall,
                color = colors.tx,
                maxLines = 3,
            )
            val count = page?.count ?: 0
            if (count > 0) {
                MutedNote("$count titre" + (if (count > 1) "s" else "") + " de la bibliothèque")
            }
        }
    }
}
