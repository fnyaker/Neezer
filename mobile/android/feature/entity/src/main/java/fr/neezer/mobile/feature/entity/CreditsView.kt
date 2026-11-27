package fr.neezer.mobile.feature.entity

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSkeletonKind
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzSwap
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.Skeleton
import uniffi.nz_ffi.TrackRow

/**
 * Who wrote, composed and produced one track — `components/TrackCredits.svelte`,
 * ported: one row per role in a sleeve's order, then the release line.
 *
 * A name is a link only when it leads somewhere (`#href`): the artist page when
 * the library holds an artist of that name, else the person's own credits page
 * once they are credited on another title too. A name with neither is plain text,
 * and plain text is not made to look like a link.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun TrackCreditsView(
    credits: TrackCreditsView,
    onArtist: (String) -> Unit,
    onPerson: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Column(modifier = modifier.fillMaxWidth()) {
        credits.groups.forEach { group ->
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(vertical = NzSpacing.s),
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
            ) {
                Text(
                    text = group.label,
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    modifier = Modifier.width(96.dp),
                )
                FlowRow(
                    modifier = Modifier.weight(1f),
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
                    verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
                ) {
                    group.people.forEachIndexed { index, person ->
                        if (index > 0) {
                            Text(text = ",", style = Sillon.type.label, color = colors.dim)
                        }
                        val opensArtist = person.opensArtist
                        val target = when {
                            opensArtist -> person.artistId
                            person.opensCredits -> person.name
                            else -> null
                        }
                        Text(
                            text = person.name,
                            style = Sillon.type.label,
                            color = if (target != null) colors.acText else colors.tx,
                            textDecoration = if (target != null) TextDecoration.Underline else null,
                            modifier = Modifier
                                .clip(RoundedCornerShape(NzRadius.field))
                                .then(
                                    if (target == null) {
                                        Modifier
                                    } else {
                                        Modifier
                                            .clickable(role = Role.Button) {
                                                if (opensArtist) {
                                                    onArtist(target)
                                                } else {
                                                    onPerson(person.name)
                                                }
                                            }
                                            .semantics {
                                                contentDescription =
                                                    if (opensArtist) {
                                                        "${person.name}, ouvrir sa fiche"
                                                    } else {
                                                        "${person.name}, crédité sur " +
                                                            "${person.credited} titres"
                                                    }
                                            }
                                            .padding(vertical = NzSpacing.s)
                                    },
                                ),
                        )
                    }
                }
            }
            Box(Modifier.fillMaxWidth().height(1.dp).background(colors.ln))
        }
        val release = credits.releaseLine
        if (release.isNotEmpty() || credits.isrc != null) {
            Column(Modifier.padding(top = NzSpacing.s)) {
                if (release.isNotEmpty()) {
                    Text(
                        text = release.joinToString(" · "),
                        style = Sillon.type.labelSmall,
                        color = colors.dim,
                    )
                }
                credits.isrc?.let { isrc ->
                    Text(
                        text = "ISRC $isrc",
                        style = Sillon.type.mono,
                        color = colors.dim,
                        modifier = Modifier.padding(top = NzSpacing.xxs),
                    )
                }
            }
        }
    }
}

/**
 * « Crédits du titre »: the sheet every track menu opens (lot MB7 owns the
 * component, MB10's menu opens it, MB11's panel shows the same view).
 *
 * The SAME answer as the full-screen player's panel (one request, kept on the
 * device, painted from the copy first), the track's sleeve and title in the
 * header, and the close control a thumb reaches. A failure is a sentence with
 * « Réessayer », never a spinner.
 *
 * @param state the sheet's model state ([TrackCreditsModel]).
 * @param track the title to draw: the caller keeps the LAST one while the sheet
 *   slides out, so the exit has something to animate.
 * @param visible whether the sheet is open.
 */
@Composable
fun TrackCreditsSheet(
    state: TrackCreditsModel.State,
    track: TrackRow?,
    visible: Boolean,
    onDismiss: () -> Unit,
    onRetry: () -> Unit,
    onArtist: (String) -> Unit,
    onPerson: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (track == null) return
    val colors = Sillon.colors
    NzSheet(visible = visible, onDismiss = onDismiss, modifier = modifier) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            CoverImage(url = track.cover, sizeDp = 48.dp, seed = track.id)
            Column(Modifier.weight(1f)) {
                Text(
                    text = track.title,
                    style = Sillon.type.titleSmall,
                    color = colors.tx,
                    maxLines = 1,
                )
                Text(
                    text = track.artists,
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    maxLines = 1,
                )
            }
            NzIconButton(
                glyph = NzGlyph.Close,
                contentDescription = "Fermer les crédits",
                onClick = onDismiss,
            )
        }
        Text(text = "CRÉDITS", style = Sillon.type.labelMono, color = colors.dim)
        // The sheet's four answers are one phase replaced in place (M7). THIS is
        // an `NzSwap` site and not a `nzPaneEnter` one: the panel is a Column
        // inside `NzSheet`, not the items of a `LazyColumn`.
        NzSwap(targetState = creditsPhaseOf(state), label = "credits-phase") { phase ->
            when (phase) {
                CreditsPhase.Content -> {
                    val credits = state.data
                    if (credits != null) {
                        TrackCreditsView(
                            credits = credits,
                            onArtist = onArtist,
                            onPerson = onPerson,
                        )
                    }
                }
                CreditsPhase.Empty -> MutedNote("Aucun crédit connu pour ce titre.")
                CreditsPhase.Failed -> {
                    val failed = state.screen as? ScreenState.Failed
                    if (failed != null) {
                        LoadFailed(
                            cause = failed.cause,
                            onRetry = onRetry,
                            title = "Les crédits n'ont pas pu être chargés",
                        )
                    }
                }
                CreditsPhase.Loading -> Skeleton(kind = NzSkeletonKind.List, count = 4)
            }
        }
    }
}

/** What the credits sheet is drawing: its four answers (a phase, not a duration). */
private enum class CreditsPhase { Content, Empty, Failed, Loading }

private fun creditsPhaseOf(state: TrackCreditsModel.State): CreditsPhase {
    val data = state.data
    return when {
        data != null && !data.empty -> CreditsPhase.Content
        data != null -> CreditsPhase.Empty
        state.screen is ScreenState.Failed -> CreditsPhase.Failed
        else -> CreditsPhase.Loading
    }
}

/** One role chip of a person's credits page, with its count. */
@Composable
internal fun RoleChip(
    label: String,
    count: Int,
    selected: Boolean,
    onClick: () -> Unit,
) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .clip(RoundedCornerShape(NzRadius.pill))
            .background(if (selected) colors.tx else colors.s2)
            .clickable(role = Role.Tab, onClick = onClick)
            .padding(horizontal = NzSpacing.m, vertical = NzSpacing.s)
            .semantics {
                role = Role.Tab
                contentDescription = "$label, $count titres"
            },
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = label,
            style = Sillon.type.label,
            color = if (selected) colors.bg else colors.tx,
        )
        Text(
            text = count.toString(),
            style = Sillon.type.mono,
            color = if (selected) colors.bg else colors.dim,
        )
    }
}

/** The dim sentence of a credits page whose list is empty, with its drawing. */
@Composable
internal fun CreditsEmptyNote(text: String, modifier: Modifier = Modifier) {
    val colors = Sillon.colors
    Row(
        modifier = modifier.fillMaxWidth().padding(vertical = NzSpacing.xl),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(glyph = NzGlyph.Users, size = 24.dp, tint = colors.dim)
        Text(text = text, style = Sillon.type.bodySmall, color = colors.dim)
    }
}
