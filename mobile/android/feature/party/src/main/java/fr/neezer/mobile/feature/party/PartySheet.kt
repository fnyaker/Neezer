package fr.neezer.mobile.feature.party

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzToasts
import fr.neezer.mobile.designsystem.Sillon
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import uniffi.nz_ffi.PartyStateData

/**
 * The HOST's sheet (lot MB20): `components/PartySheet.svelte`, ported to Sillon.
 *
 * Two faces, as the SPA has: the three arguments and « Démarrer la soirée » when
 * this device hosts nothing, then the link (a QR for the room, « Copier », the
 * system share sheet), who is listening with each guest's own sync figure, this
 * device's clock, and « Terminer la soirée » — which arms on the first press so
 * a phone in a pocket cannot end a party by accident.
 *
 * Everything it paints comes from Rust (`PartyStateData`), including the status
 * sentence: the screen decides nothing about the party.
 */
@Composable
fun PartySheet(
    model: PartyModel,
    modifier: Modifier = Modifier,
    /** The track to draw on the intro card: the player's own row. */
    current: PartyStateData? = null,
) {
    val state by model.state.collectAsStateWithLifecycle()
    val colors = Sillon.colors

    // A toast is raised by the model as a one-line message; the sheet hands it to
    // the app's ONE toast queue and the model forgets it.
    LaunchedEffect(state.message) {
        state.message?.let { message ->
            NzToasts.show(message)
            model.consumeMessage()
        }
    }

    NzSheet(visible = state.open, onDismiss = { model.close() }, modifier = modifier) {
        // --- header ---------------------------------------------------------
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            Box(
                modifier = Modifier
                    .size(44.dp)
                    .clip(RoundedCornerShape(14.dp))
                    .background(if (state.hosting) colors.ac else colors.s2),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    glyph = NzGlyph.Party,
                    size = 22.dp,
                    tint = if (state.hosting) colors.onac else colors.tx,
                )
            }
            Column(Modifier.weight(1f)) {
                Text(PartyCopy.TITLE, style = Sillon.type.title, color = colors.tx)
                Text(
                    text = if (state.hosting) {
                        state.data?.audience ?: PartyCopy.SUBTITLE
                    } else {
                        PartyCopy.SUBTITLE
                    },
                    style = Sillon.type.labelSmall,
                    color = colors.dim,
                )
            }
            NzIconButton(
                glyph = NzGlyph.Close,
                contentDescription = PartyCopy.CLOSE,
                onClick = { model.close() },
            )
        }

        if (state.hosting) {
            HostBody(model, state.data)
        } else {
            IntroBody(model, current)
        }
    }
}

@Composable
private fun IntroBody(model: PartyModel, current: PartyStateData?) {
    val colors = Sillon.colors
    val state by model.state.collectAsStateWithLifecycle()
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
        Point(NzGlyph.Link, PartyCopy.POINT_LINK)
        Point(NzGlyph.Users, PartyCopy.POINT_ROOM)
        Point(NzGlyph.Music, PartyCopy.POINT_HAND)
        current?.track?.let { track ->
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(NzRadius.card))
                    .background(colors.s1)
                    .padding(NzSpacing.m),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
            ) {
                CoverImage(url = track.cover, sizeDp = 44.dp, seed = track.id)
                Column(Modifier.weight(1f)) {
                    Text(
                        track.title,
                        style = Sillon.type.label,
                        color = colors.tx,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        track.artist,
                        style = Sillon.type.labelSmall,
                        color = colors.dim,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
        NzButton(
            text = if (state.busy) PartyCopy.STARTING else PartyCopy.START,
            onClick = { model.start() },
            glyph = NzGlyph.Party,
            enabled = !state.busy,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

@Composable
private fun HostBody(model: PartyModel, data: PartyStateData?) {
    val colors = Sillon.colors
    val state by model.state.collectAsStateWithLifecycle()
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
        val link = data?.link.orEmpty()
        if (data != null && data.role is uniffi.nz_ffi.PartyRole.Host) {
            val host = data.role as uniffi.nz_ffi.PartyRole.Host
            if (host.lost) Note(PartyCopy.LOST, warn = true)
        }

        // --- the link: a QR for the room, the share sheet for everyone else.
        // Below 480 dp the card is a COLUMN (the SPA's `max-width: 640px` rule):
        // side by side, two 95 dp buttons truncated « Partager » to « P… » on the
        // first 360 dp capture.
        BoxWithConstraints(Modifier.fillMaxWidth()) {
            val narrow = maxWidth < 480.dp
            val card = Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(NzRadius.card))
                .background(colors.s1)
                .padding(NzSpacing.m)
            val qr = @Composable { PartyQr(link = link, size = if (narrow) 148.dp else 120.dp) }
            val column = @Composable {
                Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                    Text(PartyCopy.SCAN, style = Sillon.type.labelSmall, color = colors.dim)
                    Text(
                        text = link.removePrefix("https://").removePrefix("http://"),
                        style = Sillon.type.mono,
                        color = colors.tx,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                        NzButton(
                            text = PartyCopy.SHARE,
                            onClick = { model.shareLink() },
                            glyph = NzGlyph.Share,
                            modifier = Modifier.weight(1f),
                        )
                        NzButton(
                            text = PartyCopy.COPY,
                            onClick = { model.copyLink() },
                            kind = NzButtonKind.Secondary,
                            glyph = NzGlyph.Copy,
                            modifier = Modifier.weight(1f),
                        )
                    }
                }
            }
            if (narrow) {
                Column(
                    modifier = card,
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
                ) {
                    qr()
                    column()
                }
            } else {
                Row(
                    modifier = card,
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    qr()
                    Box(Modifier.weight(1f)) { column() }
                }
            }
        }

        // --- who is listening, with each guest's own figure.
        Text(
            PartyCopy.PEOPLE.uppercase(),
            style = Sillon.type.labelSmall,
            color = colors.dim,
            modifier = Modifier.padding(top = NzSpacing.xs),
        )
        val members = data?.members.orEmpty()
        if (members.isEmpty()) {
            Text(PartyCopy.NOBODY, style = Sillon.type.labelSmall, color = colors.dim)
        } else {
            Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
                members.take(NzPartyRows).forEach { member ->
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clip(RoundedCornerShape(NzRadius.field))
                            .padding(vertical = NzSpacing.xs, horizontal = NzSpacing.xs),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
                    ) {
                        Box(
                            modifier = Modifier
                                .size(32.dp)
                                .clip(RoundedCornerShape(NzRadius.pill))
                                .background(colors.ac),
                            contentAlignment = Alignment.Center,
                        ) {
                            Text(member.initial, style = Sillon.type.label, color = colors.onac)
                        }
                        Text(
                            member.name,
                            style = Sillon.type.label,
                            color = colors.tx,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f),
                        )
                        Text(
                            member.status,
                            style = Sillon.type.labelSmall,
                            color = when {
                                member.synced -> PartyTones.ok(colors.isDark)
                                member.waiting -> PartyTones.amber(colors.isDark)
                                else -> colors.dim
                            },
                        )
                    }
                }
                if (members.size > NzPartyRows) {
                    Text(
                        "+ ${members.size - NzPartyRows}",
                        style = Sillon.type.labelSmall,
                        color = colors.dim,
                    )
                }
            }
        }

        // --- this device's clock, the number the room is locked to.
        Text(data?.status.orEmpty(), style = Sillon.type.labelSmall, color = colors.dim)

        NzButton(
            text = if (state.confirmEnd) PartyCopy.END_CONFIRM else PartyCopy.END,
            onClick = { model.end() },
            kind = NzButtonKind.Ghost,
            glyph = NzGlyph.LogOut,
            enabled = !state.busy,
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(NzSpacing.xs))
    }
}

/** How many members the sheet lists before a `+ n`: the pane must stay scrollable-free. */
const val NzPartyRows = 8

@Composable
private fun Point(glyph: NzGlyph, text: String) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        verticalAlignment = Alignment.Top,
    ) {
        Icon(glyph = glyph, size = 20.dp, tint = colors.acText)
        Text(text, style = Sillon.type.body, color = colors.tx)
    }
}

@Composable
private fun Note(text: String, warn: Boolean) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(NzRadius.field))
            .background(colors.s2)
            .padding(NzSpacing.m),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        verticalAlignment = Alignment.Top,
    ) {
        Icon(
            glyph = if (warn) NzGlyph.Alert else NzGlyph.Info,
            size = 18.dp,
            tint = if (warn) PartyTones.amber(colors.isDark) else colors.dim,
        )
        Text(
            text,
            style = Sillon.type.labelSmall,
            color = if (warn) PartyTones.amber(colors.isDark) else colors.tx,
        )
    }
}

/** The party's two signal colours, mixed for the theme (never two hard-coded hexes). */
object PartyTones {
    /** `#3ddc97` on Encre, darkened for Papier so it stays legible (4.5:1). */
    fun ok(dark: Boolean): androidx.compose.ui.graphics.Color =
        if (dark) androidx.compose.ui.graphics.Color(0xFF3DDC97) else androidx.compose.ui.graphics.Color(0xFF12805A)

    /** `#ffc857` on Encre, darkened for Papier. */
    fun amber(dark: Boolean): androidx.compose.ui.graphics.Color =
        if (dark) androidx.compose.ui.graphics.Color(0xFFFFC857) else androidx.compose.ui.graphics.Color(0xFF8A5A00)
}
