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
import uniffi.nz_ffi.TrackRow

/** The screen's marker, in every state (see [ARTIST_MARKER]). */
const val CREDITS_MARKER = "Crédits"

/** How many rows clear the shell's mini player and tab bar. */
private val SHELL_CLEARANCE = 128.dp

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
            modifier = Modifier.fillMaxSize().background(colors.bg),
            contentPadding = PaddingValues(bottom = SHELL_CLEARANCE),
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
                        modifier = Modifier.padding(horizontal = NzSpacing.pageX),
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
                        modifier = Modifier.padding(horizontal = NzSpacing.pageX),
                    )
                }
                return@LazyColumn
            }
            if (page.count == 0) {
                item(key = "empty") {
                    CreditsEmptyNote(
                        emptyPersonSentence(page.name),
                        Modifier.padding(horizontal = NzSpacing.pageX),
                    )
                }
                return@LazyColumn
            }
            val pad = Modifier.padding(horizontal = NzSpacing.pageX)
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
            items(count = count, key = { index -> "credit-row-$index" }) { index ->
                val row = window?.rowAt(index)
                if (row == null) {
                    Skeleton(kind = NzSkeletonKind.List, count = 1, modifier = pad)
                } else {
                    EntityTrackRow(
                        row = row,
                        playing = playingKey == row.key,
                        onClick = { actions.playFrom(row) },
                        onMore = { actions.more(row) },
                        onSwipeQueue = { actions.queueNext(row) },
                        modifier = pad,
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
