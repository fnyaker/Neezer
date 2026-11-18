package fr.neezer.mobile.feature.offline

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSegmented
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzSwitch
import fr.neezer.mobile.designsystem.QualityBadge
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.rememberListWindow
import kotlinx.coroutines.launch
import uniffi.nz_ffi.DownloadEntry
import uniffi.nz_ffi.SetProgress

/**
 * Réglages → Téléchargements (lot MB15), `Settings.svelte`'s `downloads` section
 * PORTED: the quality a new download takes, what the device holds, the
 * "only downloaded" switch, the play cache's gauge and caps, and the per-title
 * list on its own page (a 4 000-row list has no business between two cards).
 *
 * **MB16 hosts this**: its `downloads` section composes
 * `OfflineSettingsSection()`, and this file owns every pixel and every call
 * inside it. The section is also where rule 2 is kept for the queue: what is
 * running, how far, and the CAUSE of everything that failed, each with its own
 * « Réessayer ».
 *
 * Nothing here counts as a request: the state comes from the local store and the
 * worker's own publications, so opening the section on a warm device costs 0
 * requests (the SPA's own number for this section).
 */
@Composable
fun OfflineSettingsSection(
    modifier: Modifier = Modifier,
    /**
     * The offline page's SECOND tab, when the caller has one (LN14's « Sur vos
     * appareils »: the sets the connected devices expose). The tab strip and its
     * state live here — this module owns the page — and the CONTENT is the
     * caller's composable, so the peers' list is written once, in its own lot.
     * A caller that passes nothing (a JVM test, a build without LN14) draws the
     * downloads only, exactly as before.
     */
    peerSets: (@Composable () -> Unit)? = null,
) {
    val model = remember { OfflineHost.model() }
    val state by model.state.collectAsState()
    LaunchedEffect(model) { model.start() }

    var downloadsOpen by rememberSaveable { mutableStateOf(false) }
    // 0 = « Téléchargements », 1 = the caller's second tab.
    var tab by rememberSaveable { mutableStateOf(0) }

    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(NzSpacing.l)) {
        if (peerSets != null && !downloadsOpen) {
            NzSegmented(
                options = listOf("Téléchargements", "Sur vos appareils"),
                selectedIndex = tab,
                onSelect = { tab = it },
            )
        }
        when {
            state.failure != null && state.version == 0uL && tab == 0 -> LoadFailed(
                cause = state.failure!!,
                onRetry = model::refresh,
                title = "Téléchargements indisponibles",
            )

            downloadsOpen -> DownloadsPage(
                model = model,
                onBack = { downloadsOpen = false },
            )

            tab == 1 && peerSets != null -> peerSets()

            else -> {
                QueueCard(model, state)
                QualityCard(model, state)
                StorageCard(model, state)
                PlayCacheCard(model, state)
                DownloadsRow(state) { downloadsOpen = true }
            }
        }
    }
}

/** The queue itself: what runs, how far, and every cause (rule 2). */
@Composable
private fun QueueCard(model: DownloadsModel, state: DownloadsModel.State) {
    val active = state.sets.filter { it.running || it.failed > 0u }
    if (active.isEmpty()) return
    NzCard(modifier = Modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
            // The worker's sentence on its own line, the controls under it: at
            // 200 % a sentence beside a button broke mid-word (measured on the
            // device: « Téléchargemen / t en cours »).
            Text(
                text = state.worker.ifEmpty { "File de téléchargement" },
                style = Sillon.type.titleSmall,
                color = Sillon.colors.tx,
            )
            NzButton(
                text = if (state.paused) "Reprendre" else "Pause",
                onClick = { if (state.paused) model.resume() else model.pause() },
                kind = NzButtonKind.Ghost,
                glyph = if (state.paused) NzGlyph.Play else NzGlyph.Pause,
            )
            active.forEach { set -> SetRowView(model, set) }
            state.note?.let { note ->
                Text(note, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
            }
            if (state.sets.any { it.failed > 0u }) {
                NzButton(
                    text = "Tout réessayer",
                    onClick = model::retryAll,
                    kind = NzButtonKind.Secondary,
                    glyph = NzGlyph.Refresh,
                )
            }
        }
    }
}

/** One set: its title, `n / m`, a rail, and its failures as sentences. */
@Composable
private fun SetRowView(model: DownloadsModel, set: SetProgress) {
    val colors = Sillon.colors
    val done = set.done.toInt()
    val total = set.total.toInt()
    val progress = if (total > 0) done.toFloat() / total else 0f
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .semantics {
                contentDescription = if (set.failed > 0u) {
                    "${set.title}, $done sur $total, ${set.failed} échec(s)"
                } else {
                    "${set.title}, $done titre(s) sur $total"
                }
            },
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        // The title, then the counts on their own line: side by side they
        // touched at 200 % (« Nuit194 / 200 ») and an ellipsis ate the title.
        Text(
            text = set.title.ifEmpty { set.context },
            style = Sillon.type.bodySmall,
            color = colors.tx,
        )
        Text(
            text = if (set.failed > 0u) {
                "$done / $total · ${set.failed} échec(s)"
            } else {
                "$done / $total"
            },
            style = Sillon.type.mono,
            color = colors.dim,
        )
        // The rail: a 6 dp well with the fill on it, and enough air above it
        // that it is read as a progress bar and never as an underline of the
        // title (measured on Papier, where the well is the page's own cream).
        Box(
            Modifier
                .fillMaxWidth()
                .height(6.dp)
                .clip(RoundedCornerShape(3.dp))
                .background(colors.s2),
        ) {
            Box(
                Modifier
                    .fillMaxWidth(progress.coerceIn(0f, 1f))
                    .height(6.dp)
                    .background(if (set.failed > 0u) colors.hrt else colors.acMark),
            )
        }
        // The CAUSE of each failure, one French sentence per title, with its own
        // 「 Réessayer 」 — never a count (rule 2).
        set.failures.take(4).forEach { cause ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = cause,
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                NzIconButton(
                    glyph = NzGlyph.Refresh,
                    contentDescription = "Réessayer : $cause",
                    onClick = { model.retry(set.context) },
                )
            }
        }
        if (set.failures.size > 4) {
            Text(
                text = "… et ${set.failures.size - 4} autre(s) échec(s)",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
        }
    }
}

/** `offline.quality`: the rung a new download takes. */
@Composable
private fun QualityCard(model: DownloadsModel, state: DownloadsModel.State) {
    val current = state.settings?.quality ?: "OPUS_320"
    NzCard(modifier = Modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Text("Qualité de téléchargement par défaut", style = Sillon.type.titleSmall, color = Sillon.colors.tx)
            Text(
                "Utilisée pour les nouveaux téléchargements.",
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
            )
            DownloadsModel.QUALITIES.forEach { (rung, hint) ->
                ChoiceRow(
                    label = DownloadsModel.qualityLabel(rung),
                    hint = hint,
                    selected = current == rung,
                    onSelect = { model.setQuality(rung) },
                )
            }
        }
    }
}

/** A radio-like row: the whole row is the target, the tick says which. */
@Composable
private fun ChoiceRow(label: String, hint: String, selected: Boolean, onSelect: () -> Unit) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(NzRadius.field))
            .clickable(role = Role.RadioButton, onClick = onSelect)
            .semantics { role = Role.RadioButton }
            // `heightIn`, never a fixed 48 dp: at 200 % a rung's hint needs the
            // second line, and a `height` clipped it under the next row
            // (measured on the 200 % capture).
            .heightIn(min = NzMinTouchTarget)
            .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(label, style = Sillon.type.label, color = colors.tx)
            Text(hint, style = Sillon.type.bodySmall, color = colors.dim)
        }
        if (selected) Icon(NzGlyph.Check, size = 18.dp, tint = colors.acMark)
    }
}

/** `Settings.svelte`'s « Stockage des téléchargements » card. */
@Composable
private fun StorageCard(model: DownloadsModel, state: DownloadsModel.State) {
    val colors = Sillon.colors
    val storage = state.storage
    val onlyDownloaded = state.settings?.offlineOnly ?: true
    NzCard(modifier = Modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Text("Stockage des téléchargements", style = Sillon.type.titleSmall, color = colors.tx)
            Text(
                "Vos téléchargements sont permanents : ils restent disponibles hors ligne " +
                    "jusqu'à ce que vous les retiriez vous-même. Ce n'est pas un cache — rien " +
                    "n'est supprimé automatiquement.",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
            // The size on its line, the count on the next: a row of three
            // texts squeezed the count to one character per line at 200 %
            // (measured), and a figure nobody can read is worse than a line.
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = DownloadsModel.bytesLabel((storage?.downloadsBytes ?: 0uL).toLong()),
                    style = Sillon.type.titleSmall,
                    color = colors.tx,
                )
                Spacer(Modifier.width(NzSpacing.s))
                Text(
                    "utilisés sur cet appareil",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    modifier = Modifier.weight(1f),
                )
            }
            Text(
                text = "${storage?.tracks ?: 0u} titre(s)",
                style = Sillon.type.mono,
                color = colors.dim,
            )
            if (state.freeBytes > 0) {
                Text(
                    "Espace libre : ${DownloadsModel.bytesLabel(state.freeBytes)}",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(
                        "Hors ligne : ne lire que les titres téléchargés",
                        style = Sillon.type.label,
                        color = colors.tx,
                    )
                    Text(
                        "Lancer une playlist hors ligne ne met en file que ce qui est sur " +
                            "l'appareil, sans sauter les manquants.",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
                // No `label` on the switch: its own label would sit beside the
                // knob and take the whole row's width, squeezing the sentence
                // on the left to one letter per line (measured at 360 dp). The
                // sentence IS the label here, and TalkBack reads the pair.
                NzSwitch(
                    checked = onlyDownloaded,
                    onCheckedChange = { model.setOfflineOnly(it) },
                    modifier = Modifier.semantics {
                        contentDescription = "Ne lire que les titres téléchargés"
                    },
                )
            }
        }
    }
}

/** `lib/playcache.js`'s card: the prefetch cache, its cap and « Vider ». */
@Composable
private fun PlayCacheCard(model: DownloadsModel, state: DownloadsModel.State) {
    val colors = Sillon.colors
    val cap = state.settings?.playCacheCap ?: (1024L * 1024 * 1024)
    val used = state.playCacheBytes
    val share = if (cap > 0) (used.toFloat() / cap).coerceIn(0f, 1f) else 0f
    NzCard(modifier = Modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Cache de lecture", style = Sillon.type.titleSmall, color = colors.tx)
                Spacer(Modifier.weight(1f))
                if (used > 0) {
                    NzButton(
                        text = "Vider",
                        onClick = model::clearPlayCache,
                        kind = NzButtonKind.Ghost,
                        glyph = NzGlyph.Trash,
                    )
                }
            }
            Text(
                "Pendant la lecture, les titres à venir sont préchargés ici (audio et " +
                    "pochette) : une coupure réseau ne l'interrompt pas. C'est un cache — les " +
                    "plus anciens partent au-delà de la limite.",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
            Box(
                Modifier
                    .fillMaxWidth()
                    .height(4.dp)
                    .clip(RoundedCornerShape(2.dp))
                    .background(colors.s2),
            ) {
                Box(
                    Modifier
                        .fillMaxWidth(share)
                        .height(4.dp)
                        .background(if (share > 0.9f) colors.hrt else colors.acMark),
                )
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(DownloadsModel.bytesLabel(used), style = Sillon.type.mono, color = colors.tx)
                Spacer(Modifier.weight(1f))
                Text(
                    "limite ${DownloadsModel.bytesLabel(cap)}",
                    style = Sillon.type.mono,
                    color = colors.dim,
                )
            }
            Row(
                modifier = Modifier.horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            ) {
                DownloadsModel.CACHE_CAPS.forEach { (bytes, label) ->
                    ChoiceChip(
                        label = label,
                        selected = cap == bytes,
                        onClick = { model.setPlayCacheCap(bytes) },
                    )
                }
            }
        }
    }
}

@Composable
private fun ChoiceChip(label: String, selected: Boolean, onClick: () -> Unit) {
    val colors = Sillon.colors
    Box(
        modifier = Modifier
            .clip(RoundedCornerShape(NzRadius.chip))
            .background(if (selected) colors.ac else colors.s2)
            .clickable(role = Role.RadioButton, onClick = onClick)
            .height(NzMinTouchTarget)
            .padding(horizontal = NzSpacing.m),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = label,
            style = Sillon.type.label,
            color = if (selected) colors.onac else colors.tx,
        )
    }
}

/** The row that opens the per-title list, with what is in it (the SPA's `subnav`). */
@Composable
private fun DownloadsRow(state: DownloadsModel.State, onOpen: () -> Unit) {
    val colors = Sillon.colors
    val count = state.storage?.tracks ?: 0u
    NzCard(modifier = Modifier.fillMaxWidth(), onClick = onOpen) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("Titres téléchargés", style = Sillon.type.label, color = colors.tx)
                Text(
                    text = if (count == 0u) {
                        "Aucun titre sur cet appareil"
                    } else {
                        "$count titre(s) · ${DownloadsModel.bytesLabel((state.storage?.downloadsBytes ?: 0uL).toLong())}"
                    },
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
            }
            Icon(NzGlyph.ChevronRight, size = 20.dp, tint = colors.dim)
        }
    }
}

/**
 * « Titres téléchargés » — the SPA's own page, windowed (rule 4): a 4 000-download
 * device pages 64 rows at a time through the shared `ListWindow`, keys on every
 * item, and asks the store for the quality and the size of the rows it SHOWS.
 */
@Composable
internal fun DownloadsPage(model: DownloadsModel, onBack: () -> Unit) {
    val colors = Sillon.colors
    val state by model.state.collectAsState()
    LaunchedEffect(Unit) { model.openDownloads() }
    val handle = state.handle

    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            NzIconButton(
                glyph = NzGlyph.ChevronLeft,
                contentDescription = "Retour aux réglages",
                onClick = onBack,
            )
            Spacer(Modifier.width(NzSpacing.xs))
            Text(
                "Titres téléchargés",
                style = Sillon.type.titleSmall,
                color = colors.tx,
                modifier = Modifier.weight(1f),
            )
            Text(
                "${state.storage?.tracks ?: 0u} · ${DownloadsModel.bytesLabel((state.storage?.downloadsBytes ?: 0uL).toLong())}",
                style = Sillon.type.mono,
                color = colors.dim,
            )
        }

        when {
            state.failure != null && handle == null -> LoadFailed(
                cause = state.failure!!,
                onRetry = model::openDownloads,
                title = "Liste indisponible",
            )

            handle == null || handle.len().toInt() == 0 -> EmptyState(
                title = "Aucun titre téléchargé",
                body = "Utilisez le bouton de téléchargement sur un titre, un album ou une " +
                    "playlist pour les rendre disponibles hors ligne.",
                glyph = NzGlyph.Download,
            )

            else -> DownloadsList(model, state, handle)
        }
    }
}

@Composable
private fun DownloadsList(model: DownloadsModel, state: DownloadsModel.State, handle: uniffi.nz_ffi.ListHandle) {
    val window = rememberListWindow(handle)
    val listState = rememberLazyListState()
    val facts = state.facts

    LaunchedEffect(window, model) {
        snapshotFlow { listState.firstVisibleItemIndex to listState.isScrollInProgress }
            .collect { (first, scrolling) ->
                window.onVisible(first, first + VisibleRows, scrolling)
                val keys = (first until (first + VisibleRows)).mapNotNull { index ->
                    window.rowAt(index)?.key
                }
                model.loadFacts(keys)
            }
    }

    LazyColumn(state = listState, modifier = Modifier.fillMaxWidth()) {
        items(window.len) { index ->
            val row = window.rowAt(index)
            if (row == null) {
                SkeletonRow()
            } else {
                DownloadRowView(
                    row = row,
                    fact = facts[row.key],
                    onRemove = { model.removeDownload(row.key) },
                )
            }
        }
    }
}

/** Rows a phone shows at once: the window's page hint, never the whole list. */
private const val VisibleRows = 12

@Composable
private fun SkeletonRow() {
    Box(
        Modifier
            .fillMaxWidth()
            .height(56.dp)
            .padding(vertical = NzSpacing.xs),
    ) {
        Box(
            Modifier
                .fillMaxWidth()
                .height(48.dp)
                .clip(RoundedCornerShape(NzRadius.cover))
                .background(Sillon.colors.s1),
        )
    }
}

@Composable
private fun DownloadRowView(
    row: uniffi.nz_ffi.TrackRow,
    fact: DownloadEntry?,
    onRemove: () -> Unit,
) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(56.dp)
            .semantics {
                contentDescription = buildString {
                    append(row.title)
                    if (row.artists.isNotEmpty()) append(", ${row.artists}")
                    val quality = fact?.quality
                    if (!quality.isNullOrEmpty()) append(", ${DownloadsModel.qualityLabel(quality)}")
                    val bytes = (fact?.bytes ?: 0uL).toLong()
                    if (bytes > 0) append(", ${DownloadsModel.bytesLabel(bytes)}")
                }
            },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CoverImage(url = row.cover, sizeDp = 40.dp, seed = row.key)
        Spacer(Modifier.width(NzSpacing.m))
        Column(Modifier.weight(1f)) {
            Text(
                text = row.title,
                style = Sillon.type.label,
                color = colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = row.artists,
                style = Sillon.type.bodySmall,
                color = colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        fact?.quality?.takeIf { it.isNotEmpty() }?.let { quality ->
            QualityBadge(DownloadsModel.qualityLabel(quality))
            Spacer(Modifier.width(NzSpacing.s))
        }
        val factBytes = (fact?.bytes ?: 0uL).toLong()
        if (factBytes > 0) {
            Text(
                text = DownloadsModel.bytesLabel(factBytes),
                style = Sillon.type.mono,
                color = colors.dim,
            )
            Spacer(Modifier.width(NzSpacing.s))
        }
        NzIconButton(
            glyph = NzGlyph.Trash,
            contentDescription = "Retirer « ${row.title} »",
            onClick = onRemove,
        )
    }
}
