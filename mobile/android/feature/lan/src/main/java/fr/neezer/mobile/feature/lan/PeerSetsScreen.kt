package fr.neezer.mobile.feature.lan

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import uniffi.nz_ffi.PeerSetFfi

/**
 * « Sur vos appareils » (lot LN14): the sets your connected devices expose
 * (playlists, albums, shows), multi-selectable, and « Télécharger ».
 *
 * The list comes from the node's in-memory replica — a peer's `Sets` pages are
 * replicated when its connection authenticates (LN11) — so opening the tab costs
 * **0 requests** and paints in one frame. « Télécharger » queues ONE ordinary
 * `nz-offline` job per ticked set: progress and causes then live on the
 * downloads screen like any download (rule 2), and each track still yields to
 * what the person is listening to (rule 1, the Bulk lane).
 *
 * Drawn from one body in two places: the offline page's tab (MB15's screen, the
 * entry LN14 adds) and the route `Routes.PeerSets`.
 */
@Composable
fun PeerSetsSection(
    modifier: Modifier = Modifier,
    model: PeerSetsModel? = null,
) {
    val built = rememberPeerSetsModel(model)
    val state by built.state.collectAsState()
    LaunchedEffect(built) { built.reload() }
    LaunchedEffect(built) {
        // A peer connecting (or leaving) changes the replica; the node says so.
        LanFeed.version.collect { built.reload() }
    }

    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        if (state.sets.isEmpty()) {
            EmptyState(
                title = "Rien à prendre pour l'instant",
                body = peerSetsEmptyText(state.peers),
                glyph = NzGlyph.Download,
            )
            return@Column
        }
        state.groups.forEach { (head, rows) ->
            PeerGroupHeader(
                head = head,
                selectedCount = rows.count { PeerSetsModel.keyOf(it) in state.selected },
                onTogglePeer = { built.togglePeer(head.peer) },
            )
            rows.forEach { set ->
                val key = PeerSetsModel.keyOf(set)
                PeerSetRow(
                    set = set,
                    checked = key in state.selected,
                    onToggle = { built.toggle(key) },
                )
            }
        }
        state.note?.let { sentence ->
            Text(text = sentence, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
        }
        state.notice?.let { cause -> CauseLine(cause = cause, onDismiss = built::dismissNotice) }
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Text(
                text = when (val count = state.selected.size) {
                    0 -> "Choisissez une ou plusieurs listes"
                    1 -> "1 liste choisie"
                    else -> "$count listes choisies"
                },
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
                modifier = Modifier.weight(1f),
            )
            NzButton(
                text = "Télécharger",
                onClick = built::download,
                kind = NzButtonKind.Primary,
                glyph = NzGlyph.Download,
                enabled = state.canDownload,
                modifier = Modifier.semantics {
                    contentDescription = "Télécharger ${state.selected.size} liste(s) depuis vos appareils"
                },
            )
        }
    }
}

/** The route's screen: the section, in a lazy list of its own. */
@Composable
fun PeerSetsEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    androidx.compose.foundation.layout.Column(
        modifier = modifier.fillMaxSize().background(Sillon.colors.bg),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Text(
            text = "Les listes que vos appareils ont déjà. Les prendre ici les télécharge par le Wi-Fi, " +
                "sans repasser par le serveur.",
            style = Sillon.type.bodySmall,
            color = Sillon.colors.dim,
            modifier = Modifier.padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.l),
        )
        PeerSetsListBody(
            modifier = Modifier.fillMaxSize().padding(horizontal = NzSpacing.pageX),
        )
    }
}

/**
 * The scrolling body, for the route: the same rows as [PeerSetsSection], in a
 * `LazyColumn` so a peer exposing 500 sets never composes them all (rule 4).
 */
@Composable
private fun PeerSetsListBody(modifier: Modifier = Modifier) {
    val built = rememberPeerSetsModel(null)
    val state by built.state.collectAsState()
    LaunchedEffect(built) { built.reload() }
    LaunchedEffect(built) { LanFeed.version.collect { built.reload() } }
    val rows = state.sets
    if (rows.isEmpty()) {
        EmptyState(title = "Rien à prendre pour l'instant", body = peerSetsEmptyText(state.peers), glyph = NzGlyph.Download)
        return
    }
    Column(modifier = modifier) {
        LazyColumn(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            items(rows, key = { PeerSetsModel.keyOf(it) }) { set ->
                val key = PeerSetsModel.keyOf(set)
                PeerSetRow(set = set, checked = key in state.selected, onToggle = { built.toggle(key) })
            }
        }
        Row(
            modifier = Modifier.fillMaxWidth().padding(vertical = NzSpacing.s),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Text(
                text = "${state.selected.size} liste(s) choisie(s)",
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
                modifier = Modifier.weight(1f),
            )
            NzButton(
                text = "Télécharger",
                onClick = built::download,
                glyph = NzGlyph.Download,
                enabled = state.canDownload,
            )
        }
        state.notice?.let { cause -> CauseLine(cause = cause, onDismiss = built::dismissNotice) }
    }
}

@Composable
private fun PeerGroupHeader(head: PeerSetFfi, selectedCount: Int, onTogglePeer: () -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Icon(glyph = relationGlyph(head.own), size = 16.dp, tint = Sillon.colors.dim)
        Text(
            text = "${head.peerName.ifBlank { "Un appareil" }} · ${relationText(head.own)}",
            style = Sillon.type.label,
            color = Sillon.colors.tx,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f),
        )
        NzButton(
            text = if (selectedCount == 0) "Tout choisir" else "Tout retirer",
            onClick = onTogglePeer,
            kind = NzButtonKind.Ghost,
        )
    }
}

@Composable
private fun PeerSetRow(set: PeerSetFfi, checked: Boolean, onToggle: () -> Unit) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.card)
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(64.dp)
            .background(if (checked) colors.s2 else colors.s1, shape)
            .clickable(role = Role.Checkbox, onClick = onToggle)
            .padding(horizontal = NzSpacing.m, vertical = NzSpacing.s)
            .semantics {
                contentDescription = "${set.title}, ${setKindText(set.kind)}, " +
                    setDetailText(set.tracks.toInt(), set.local.toInt(), set.complete) + ", " +
                    completenessText(set.complete) + if (checked) ", choisi" else ""
            },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Icon(
            glyph = if (checked) NzGlyph.Check else NzGlyph.Plus,
            size = 16.dp,
            tint = if (checked) colors.acText else colors.dim,
        )
        Column(Modifier.weight(1f)) {
            Text(
                text = set.title.ifBlank { set.ident },
                style = Sillon.type.body,
                color = colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = "${setKindText(set.kind)} · " +
                    setDetailText(set.tracks.toInt(), set.local.toInt(), set.complete),
                style = Sillon.type.bodySmall,
                color = colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Column(horizontalAlignment = Alignment.End) {
            Text(text = bytesText(set.bytes.toLong()), style = Sillon.type.bodySmall, color = colors.dim)
            Text(
                text = completenessText(set.complete),
                style = Sillon.type.bodySmall,
                color = if (set.complete) colors.tx else colors.dim,
            )
        }
    }
}

@Composable
private fun rememberPeerSetsModel(model: PeerSetsModel?): PeerSetsModel {
    if (model != null) return model
    val scope = rememberCoroutineScope()
    return remember(scope) {
        PeerSetsModel(
            port = LanHost.node ?: FfiLanPort(),
            scope = scope,
            toast = { message -> fr.neezer.mobile.designsystem.NzToasts.show(message) },
        )
    }
}
