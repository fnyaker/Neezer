package fr.neezer.mobile.feature.nowplaying

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon

/**
 * The words, as a VIEW of their own — `Lyrics.svelte`, the phone's lyrics sheet.
 *
 * **A view, not a gesture.** The SPA's comment is the rule: the phone's lyrics are
 * not hidden behind a swipe nobody discovers — its own header names the track and
 * the artist, its close is a 44 dp button, and the only scroller is the list of
 * lines. The card's door is a NAMED action at the bottom (« Créer une carte »),
 * with the reason it cannot be taken printed beside it rather than a disabled
 * button that explains nothing.
 *
 * The active line follows the position being HEARD, and the list follows the line
 * (centred a little above the middle, so the next lines are visible). The first
 * jump on open is instant; the later ones are smooth — `scrollBehavior()`'s rule
 * in the SPA, and the eco/reduced guard is the design system's motion.
 */
@Composable
fun LyricsSheet(
    lyrics: LyricsModel?,
    state: NowPlayingModel.State,
    onClose: () -> Unit,
    onCard: () -> Unit,
) {
    val colors = Sillon.colors
    NzSheet(visible = true, onDismiss = onClose) {
        Row(
            Modifier.fillMaxWidth().heightIn(min = NzMinTouchTarget),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text("PAROLES", style = Sillon.type.labelMono, color = colors.dim)
                Text(
                    text = state.current?.title ?: "",
                    style = Sillon.type.titleSmall,
                    color = colors.tx,
                    maxLines = 1,
                )
                Text(
                    text = state.current?.artists ?: "",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    maxLines = 1,
                )
            }
            NzIconButton(
                glyph = NzGlyph.Close,
                contentDescription = "Fermer les paroles",
                onClick = onClose,
            )
        }
        val model = lyrics
        if (model == null) {
            Unavailable()
            return@NzSheet
        }
        val lyricsState by model.state.collectAsState()
        Column(Modifier.fillMaxWidth().heightIn(max = 460.dp)) {
            LyricsView(lyricsState, Modifier.weight(1f))
            Row(
                Modifier.fillMaxWidth().padding(top = NzSpacing.s),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                NzButton(
                    text = "Créer une carte",
                    onClick = onCard,
                    kind = NzButtonKind.Secondary,
                    enabled = lyricsState.canCreate,
                )
                val reason = when {
                    lyricsState.canCreate -> ""
                    lyricsState.none -> ""
                    lyricsState.lyrics == null -> "Chargement des paroles…"
                    else -> "Paroles indisponibles pour ce titre."
                }
                if (reason.isNotBlank()) {
                    Text(reason, style = Sillon.type.bodySmall, color = colors.dim)
                }
            }
            if (lyricsState.notice.isNotBlank()) {
                Text(lyricsState.notice, style = Sillon.type.bodySmall, color = colors.acText)
            }
        }
    }
}

/**
 * The lines themselves, with the active one on the accent.
 *
 * Three states, and no fourth: loading (« Chargement des paroles… »), the words,
 * and the reason there are none (`LoadFailed` with « Réessayer » — a failure, and
 * « Paroles indisponibles pour ce titre » — an answer).
 */
@Composable
internal fun LyricsView(state: LyricsModel.State, modifier: Modifier = Modifier) {
    val colors = Sillon.colors
    when {
        state.screen is ScreenState.Failed -> {
            val failed = state.screen as ScreenState.Failed
            LoadFailed(cause = failed.cause, onRetry = failed.retry, modifier = modifier)
        }
        state.none -> {
            Box(modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                Text(
                    text = "Paroles indisponibles pour ce titre.",
                    style = Sillon.type.body,
                    color = colors.dim,
                    modifier = Modifier.padding(NzSpacing.l),
                )
            }
        }
        state.lyrics == null -> {
            Box(modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                Text(
                    text = "Chargement des paroles…",
                    style = Sillon.type.body,
                    color = colors.dim,
                    modifier = Modifier.padding(NzSpacing.l),
                )
            }
        }
        else -> {
            val lyrics = state.lyrics ?: return
            val listState = rememberLazyListState()
            // Follow the line being heard: centred a little above the middle, an
            // instant jump on the first follow and a smooth one after (the SPA's
            // `followScroll` + `scrollBehavior()`).
            LaunchedEffect(state.activeLine) {
                if (state.activeLine >= 0) {
                    listState.animateScrollToItem((state.activeLine - 2).coerceAtLeast(0))
                }
            }
            if (lyrics.synced.isNotEmpty()) {
                LazyColumn(state = listState, modifier = modifier.fillMaxWidth()) {
                    itemsIndexed(lyrics.synced) { index, line ->
                        val active = index == state.activeLine
                        Text(
                            text = line.text.ifBlank { "• • •" },
                            style = if (active) Sillon.type.titleSmall else Sillon.type.body,
                            color = if (active) colors.acText else colors.tx,
                            modifier = Modifier
                                .fillMaxWidth()
                                .clip(RoundedCornerShape(NzRadius.field))
                                .padding(vertical = NzSpacing.xs, horizontal = NzSpacing.xs),
                        )
                    }
                }
            } else {
                Column(modifier.verticalScroll(rememberScrollState())) {
                    Text(
                        text = lyrics.plain.ifBlank { "Paroles indisponibles pour ce titre." },
                        style = Sillon.type.body,
                        color = colors.tx,
                        modifier = Modifier.padding(horizontal = NzSpacing.xs),
                    )
                }
            }
        }
    }
}

/** A process with no core: the words cannot even be asked for. */
@Composable
private fun Unavailable() {
    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        Text(
            text = "Les paroles ne sont pas disponibles : le noyau de l'application n'a pas démarré.",
            style = Sillon.type.body,
            color = Sillon.colors.dim,
            modifier = Modifier.padding(NzSpacing.l),
        )
    }
}
