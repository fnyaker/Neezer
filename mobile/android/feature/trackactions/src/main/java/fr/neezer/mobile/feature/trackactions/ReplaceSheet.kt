package fr.neezer.mobile.feature.trackactions

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.nzDuration
import uniffi.nz_ffi.ReplaceCandidate

/** The label the device test waits for. */
const val REPLACE_MARKER = "Remplacer le titre"

/** The width under which a candidate row drops its duration (the SPA's 560 px). */
private const val NARROW_WIDTH = 560

/**
 * Give a dead title a stand-in (`ReplaceSheet.svelte`, PORTED).
 *
 * A provider title can stop being playable at any time, and it stays in the
 * playlists it was in. The sheet finds the closest matches, lets the person
 * choose, and the swap happens EVERYWHERE on the server (one job), so it survives
 * closing the app.
 *
 * What is deliberately not here (each needs a component this lot does not own):
 * the audition (an audio path belongs to the player's lot — a silent button would
 * be a bug), « Importer mon fichier » (MB8's upload flow), and the fallback
 * search's source names (Réglages → Sources is MB16's).
 */
@Composable
fun ReplaceSheet(
    state: TrackActionsModel.State,
    onSearch: (String) -> Unit,
    onFallbacks: () -> Unit,
    onCommit: (ReplaceCandidate) -> Unit,
    onDelete: () -> Unit,
    onRetry: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val replace = state.replace ?: return
    val colors = Sillon.colors
    NzSheet(visible = !state.closed, onDismiss = onDismiss, modifier = modifier) {
        Column(
            modifier = Modifier
                .widthIn(max = 620.dp)
                .fillMaxWidth()
                .align(Alignment.CenterHorizontally)
                .semantics { contentDescription = REPLACE_MARKER },
            verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            Row(verticalAlignment = Alignment.Top) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(text = REPLACE_MARKER, style = Sillon.type.titleSmall, color = colors.tx)
                    Text(
                        text = "« ${state.row.title} » n'est plus lisible. Choisissez un " +
                            "remplaçant : il prendra sa place partout où il apparaît.",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                        maxLines = 3,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                NzIconButton(glyph = NzGlyph.Close, contentDescription = "Fermer", onClick = onDismiss)
            }

            // The search box sits outside the scroller: it stays in reach whatever
            // has been scrolled (the SPA's own layout decision).
            ReplaceSearch(state = state, onSearch = onSearch)

            Box(modifier = Modifier.fillMaxWidth().heightIn(min = 120.dp, max = 340.dp)) {
                when {
                    replace.loading -> ReplaceState("Recherche de titres proches…")
                    replace.error != null -> ReplaceState(
                        text = "Impossible de chercher un remplaçant : ${replace.error}",
                        action = "Réessayer",
                        onAction = onRetry,
                    )
                    replace.candidates.isEmpty() -> ReplaceState(
                        text = "Aucun titre proche trouvé" +
                            (if (replace.query.isNotBlank()) " pour « ${replace.query} »" else "") +
                            ". Essayez une autre recherche.",
                    )
                    else -> Column(
                        // The sheet holds at most thirty candidates, so a plain
                        // column that scrolls IS the window (and it wraps its
                        // content: no empty tail under three rows).
                        modifier = Modifier
                            .fillMaxWidth()
                            .verticalScroll(rememberScrollState()),
                        verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
                    ) {
                        replace.candidates.forEach { candidate ->
                            key(candidate.key) {
                                CandidateRow(
                                    candidate = candidate,
                                    busy = replace.busy,
                                    onCommit = { onCommit(candidate) },
                                )
                            }
                        }
                    }
                }
            }

            // The fallback sources: asked when the person presses for it, and
            // reported when the server reached for one on its own.
            when {
                replace.fallbacksLoading -> ReplaceState("Recherche sur les sources de secours…")
                replace.fallbacksFailed -> ReplaceState(
                    "Les sources de secours n'ont pas répondu. Réessayez plus tard.",
                )
                replace.fallbackAuto != null -> ReplaceState(
                    "Rien sur la source principale : les titres les plus proches de " +
                        "${replace.fallbackAuto} ont été ajoutés, archivés à la première écoute.",
                )
                !replace.fallbacksAsked -> NzButton(
                    text = "Chercher aussi ailleurs",
                    kind = NzButtonKind.Secondary,
                    glyph = NzGlyph.Search,
                    onClick = onFallbacks,
                )
            }

            // The third answer, set apart by a rule rather than by shouting: it is
            // legitimate, it is just the one you cannot undo.
            Box(modifier = Modifier.fillMaxWidth().height(1.dp).background(colors.ln))
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
            ) {
                Box(
                    modifier = Modifier
                        .border(1.dp, colors.hrt, RoundedCornerShape(NzRadius.pill))
                        .padding(NzSpacing.xxs),
                ) {
                    NzButton(
                        text = if (replace.confirming) {
                            "Confirmer la suppression"
                        } else {
                            "Supprimer ce titre"
                        },
                        kind = NzButtonKind.Ghost,
                        glyph = NzGlyph.Trash,
                        enabled = !replace.busy,
                        onClick = onDelete,
                    )
                }
            }
            Text(
                text = if (replace.confirming) {
                    "Le titre disparaîtra de vos playlists et de vos favoris. Le serveur " +
                        "vérifie d'abord qu'il est bien introuvable des deux côtés."
                } else {
                    "Ce titre n'existe plus ni sur le fournisseur ni sur le disque."
                },
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
        }
    }
}

@Composable
private fun ReplaceSearch(
    state: TrackActionsModel.State,
    onSearch: (String) -> Unit,
) {
    val colors = Sillon.colors
    val replace = state.replace ?: return
    var text by remember(replace.searched) { mutableStateOf(replace.searched) }
    val loading = replace.loading
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(colors.s2, RoundedCornerShape(NzRadius.chip))
            .sizeIn(minHeight = NzMinTouchTarget)
            .padding(start = NzSpacing.m, end = NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Icon(glyph = NzGlyph.Search, size = 16.dp, tint = colors.dim, contentDescription = null)
        BasicTextField(
            value = text,
            onValueChange = { text = it },
            singleLine = true,
            textStyle = Sillon.type.body.copy(color = colors.tx),
            cursorBrush = SolidColor(colors.acText),
            modifier = Modifier
                .weight(1f)
                .semantics { contentDescription = "Chercher un autre remplaçant" },
            decorationBox = { inner ->
                if (text.isEmpty()) {
                    Text(
                        text = "Artiste, titre, album…",
                        style = Sillon.type.body,
                        color = colors.dim,
                    )
                }
                inner()
            },
        )
        NzButton(
            text = "Chercher",
            kind = NzButtonKind.Ghost,
            enabled = !loading && text.isNotBlank(),
            onClick = { onSearch(text) },
        )
    }
}

@Composable
private fun CandidateRow(
    candidate: ReplaceCandidate,
    busy: Boolean,
    onCommit: () -> Unit,
) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .sizeIn(minHeight = NzMinTouchTarget + 8.dp)
            .padding(horizontal = NzSpacing.xs, vertical = NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        CoverImage(url = candidate.cover, sizeDp = 40.dp, seed = candidate.id)
        Column(modifier = Modifier.weight(1f)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            ) {
                // The source mark: a file already here, a fallback's own id.
                if (candidate.local) {
                    Icon(
                        glyph = NzGlyph.CloudOff,
                        size = 12.dp,
                        tint = colors.dim,
                        contentDescription = "Fichier local",
                    )
                } else if (candidate.source != "deezer") {
                    Icon(
                        glyph = NzGlyph.Youtube,
                        size = 12.dp,
                        tint = colors.dim,
                        contentDescription = candidate.source,
                    )
                }
                Text(
                    text = candidate.title,
                    style = Sillon.type.body,
                    color = colors.tx,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
            }
            Text(
                text = listOf(candidate.artists, candidate.album)
                    .filter { it.isNotBlank() }
                    .joinToString(" · "),
                style = Sillon.type.bodySmall,
                color = colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        // The duration is the first thing to go when space is tight — the title,
        // the artist and the two actions all matter more (`ReplaceSheet.svelte`'s
        // own `@media (max-width: 560px)` rule, ported).
        if (LocalConfiguration.current.screenWidthDp >= NARROW_WIDTH) {
            Text(
                text = nzDuration(candidate.durationS.toInt()),
                style = Sillon.type.mono,
                color = colors.dim,
            )
        }
        NzButton(
            text = "Remplacer",
            kind = NzButtonKind.Primary,
            enabled = !busy,
            onClick = onCommit,
        )
    }
}

@Composable
private fun ReplaceState(
    text: String,
    action: String? = null,
    onAction: (() -> Unit)? = null,
) {
    val colors = Sillon.colors
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = NzSpacing.l, horizontal = NzSpacing.s)
            .semantics { contentDescription = text },
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Text(text = text, style = Sillon.type.bodySmall, color = colors.dim)
        if (action != null && onAction != null) {
            NzButton(text = action, kind = NzButtonKind.Ghost, onClick = onAction)
        }
    }
}
