package fr.neezer.mobile.feature.nowplaying

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzNavigator
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
import uniffi.nz_ffi.AppLink
import uniffi.nz_ffi.NowPlaying

/**
 * « À propos » — what lives under the full-screen player
 * (`NowPlayingMore.svelte`), for music.
 *
 * The sections are [AboutModel]'s: the credits of the title, the titles that
 * sound like it, and the two doors the SPA puts beside them (the listen party and
 * the remote control — those two are MB20's and MB19's components, called here
 * through their stubs).
 *
 * A section that cannot load is simply absent; the similar titles are the one
 * that says so with « Réessayer » (its list is the panel's body, so an empty
 * panel with no sentence would look like a title with nothing to say).
 */
@Composable
fun AboutSheet(
    about: AboutModel?,
    player: PlayerPort,
    nav: NzNavigator,
    track: NowPlaying?,
    onClose: () -> Unit,
) {
    val colors = Sillon.colors
    val key = track?.key
    val seed = track?.key

    if (about != null && key != null) {
        LaunchedEffect(key) { about.load(key, seed ?: key) }
    }
    val state = about?.state?.collectAsState()?.value ?: AboutModel.State(loading = false)

    NzSheet(visible = true, onDismiss = onClose) {
        Row(
            Modifier.fillMaxWidth().heightIn(min = NzMinTouchTarget),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text("À PROPOS", style = Sillon.type.labelMono, color = colors.dim)
                Text(
                    text = track?.title ?: "",
                    style = Sillon.type.titleSmall,
                    color = colors.tx,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = track?.album ?: track?.artists ?: "",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            NzIconButton(
                glyph = NzGlyph.Close,
                contentDescription = "Fermer",
                onClick = onClose,
            )
        }

        LazyColumn(Modifier.fillMaxWidth().heightIn(max = 460.dp)) {
            if (state.hasCredits) {
                item { SectionTitle("Crédits") }
                val credits = state.credits
                if (credits != null) {
                    for (group in credits.credits) {
                        item(key = "role:${group.role}") {
                            Column(Modifier.fillMaxWidth().padding(vertical = NzSpacing.xxs)) {
                                Text(group.label, style = Sillon.type.labelMono, color = colors.dim)
                                for (person in group.people) {
                                    Row(
                                        Modifier
                                            .fillMaxWidth()
                                            .heightIn(min = 36.dp)
                                            .clickable(
                                                role = Role.Button,
                                                enabled = person.artistId != null || (person.credited ?: 0u) >= 2u,
                                            ) {
                                                val artist = person.artistId
                                                if (artist != null) {
                                                    onClose()
                                                    nav.go(AppLink.Artist(artist))
                                                } else if ((person.credited ?: 0u) >= 2u) {
                                                    onClose()
                                                    nav.go(AppLink.Credits(person.name))
                                                }
                                            }
                                            .padding(vertical = NzSpacing.xxs),
                                        verticalAlignment = Alignment.CenterVertically,
                                    ) {
                                        Text(
                                            text = person.name,
                                            style = Sillon.type.body,
                                            color = if (person.artistId != null || (person.credited ?: 0u) >= 2u) {
                                                colors.tx
                                            } else {
                                                colors.dim
                                            },
                                            maxLines = 1,
                                            overflow = TextOverflow.Ellipsis,
                                            modifier = Modifier.weight(1f),
                                        )
                                        if (person.artistId != null || (person.credited ?: 0u) >= 2u) {
                                            Icon(
                                                glyph = NzGlyph.ChevronRight,
                                                size = 16.dp,
                                                tint = colors.dim,
                                                contentDescription = null,
                                            )
                                        }
                                    }
                                }
                            }
                        }
                    }
                    val info = credits.info
                    val facts = listOfNotNull(
                        info.released?.let { "Sortie · $it" },
                        info.label?.let { "Label · $it" },
                        info.isrc?.let { "ISRC · $it" },
                        info.copyright,
                    )
                    if (facts.isNotEmpty()) {
                        item(key = "facts") {
                            Column(Modifier.padding(top = NzSpacing.s)) {
                                for (fact in facts) {
                                    Text(fact, style = Sillon.type.bodySmall, color = colors.dim)
                                }
                            }
                        }
                    }
                }
            }

            val artist = track?.artistId
            if (artist != null) {
                item(key = "artist") {
                    SectionTitle("L'artiste")
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .heightIn(min = NzMinTouchTarget)
                            .clickable(role = Role.Button) {
                                onClose()
                                nav.go(AppLink.Artist(artist))
                            }
                            .padding(vertical = NzSpacing.xs),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                    ) {
                        Text(
                            text = track.artists,
                            style = Sillon.type.body,
                            color = colors.tx,
                            modifier = Modifier.weight(1f),
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Icon(glyph = NzGlyph.ChevronRight, size = 18.dp, tint = colors.dim, contentDescription = null)
                    }
                }
            }

            item(key = "similar") { SectionTitle("Titres similaires") }
            if (state.similarFailed) {
                item(key = "similar-failed") {
                    Row(
                        Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                    ) {
                        Text(
                            text = "Impossible de charger les titres similaires.",
                            style = Sillon.type.bodySmall,
                            color = colors.dim,
                            modifier = Modifier.weight(1f),
                        )
                        NzButton(
                            text = "Réessayer",
                            onClick = { key?.let { about?.retry(it, seed ?: it) } },
                            kind = NzButtonKind.Ghost,
                        )
                    }
                }
            } else if (state.similar.isEmpty() && !state.loading) {
                item(key = "similar-empty") {
                    Text(
                        text = "Aucun titre similaire pour l'instant.",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
            } else {
                items(state.similar, key = { row -> "similar:${row.key}" }) { row ->
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .heightIn(min = 56.dp)
                            .clip(RoundedCornerShape(NzRadius.field))
                            .clickable(role = Role.Button) {
                                player.command(
                                    uniffi.nz_ffi.PlayerCommand.Jump(
                                        index = row.position,
                                        key = row.key,
                                    ),
                                )
                            }
                            .padding(horizontal = NzSpacing.xs, vertical = NzSpacing.xxs),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                    ) {
                        CoverImage(url = row.cover, sizeDp = 40.dp, seed = row.id)
                        Column(Modifier.weight(1f)) {
                            Text(
                                row.title,
                                style = Sillon.type.body,
                                color = colors.tx,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            Text(
                                row.artists,
                                style = Sillon.type.bodySmall,
                                color = colors.dim,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                }
                if (state.similar.isNotEmpty()) item(key = "similar-actions") {
                    Row(
                        Modifier.fillMaxWidth().padding(top = NzSpacing.s),
                        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                    ) {
                        NzButton(
                            text = "Tout lire",
                            onClick = { key?.let { about?.playAll(player, it) } },
                            kind = NzButtonKind.Secondary,
                            enabled = state.similar.isNotEmpty(),
                        )
                        NzButton(
                            text = "Ajouter à la file",
                            onClick = { about?.queueNext(player) },
                            kind = NzButtonKind.Ghost,
                            enabled = state.similar.isNotEmpty(),
                        )
                    }
                }
            }

        }
    }
}

/** A section's heading, in the player's own small capitals. */
@Composable
private fun SectionTitle(text: String) {
    Text(
        text = text.uppercase(),
        style = Sillon.type.labelMono,
        color = Sillon.colors.dim,
        modifier = Modifier.padding(top = NzSpacing.m, bottom = NzSpacing.xs),
    )
}

/**
 * The wide layout's right-hand column: the queue and « À propos » as two tabs, the
 * way `DesktopNowPlaying.svelte` puts them beside the sleeve.
 */
@Composable
internal fun SideColumn(
    state: NowPlayingModel.State,
    model: NowPlayingModel,
    nav: NzNavigator,
    about: AboutModel?,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    var queueTab by remember { mutableStateOf(true) }
    Column(modifier.fillMaxWidth().padding(NzSpacing.l)) {
        Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            TabChip("File d'attente", selected = queueTab) { queueTab = true }
            TabChip("À propos", selected = !queueTab) { queueTab = false }
        }
        if (queueTab) {
            QueueEditor(
                state = state,
                model = model,
                nav = nav,
                modifier = Modifier.padding(top = NzSpacing.s),
            )
        } else {
            val key = state.current?.key
            if (about != null && key != null) {
                LaunchedEffect(key) { about.load(key, key) }
            }
            val panel = about?.state?.collectAsState()?.value
            Column(Modifier.padding(top = NzSpacing.s)) {
                if (panel == null) {
                    Text("Rien à afficher.", style = Sillon.type.body, color = colors.dim)
                } else {
                    Text(
                        text = "Crédits · ${panel.credits?.credits?.size ?: 0} rôles",
                        style = Sillon.type.labelMono,
                        color = colors.dim,
                    )
                }
            }
        }
    }
}

@Composable
private fun TabChip(label: String, selected: Boolean, onClick: () -> Unit) {
    val colors = Sillon.colors
    Box(
        Modifier
            .heightIn(min = NzMinTouchTarget)
            .clip(RoundedCornerShape(NzRadius.chip))
            .background(if (selected) colors.ac else colors.s1)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = NzSpacing.m),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = label,
            style = Sillon.type.label,
            color = if (selected) colors.onac else colors.dim,
        )
    }
}
