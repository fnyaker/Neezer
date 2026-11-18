package fr.neezer.mobile.feature.discovery

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.nzGrooveFor
import kotlinx.coroutines.delay
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.StationRowData

/**
 * Explorer's ten parent styles, as a block the Search screen places (lot MB14):
 * a style tile opens its genre station, its sheet runs one style's discovery.
 *
 * This is `components/ExplorerStyles.svelte` ported: the tiles are the styles
 * the person LISTENS to (never a leaf with no way in), the disc plays the
 * server's own genre radio, the sheet is one POST then a poll every two seconds,
 * and "Toutes les stations" is one `/flow/styles` for the whole tree. The
 * screen paints the kept styles first, so a warm open costs nothing.
 */
@Composable
fun ExplorerStylesSection(modifier: Modifier = Modifier) {
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val service = remember(screens) { screens?.let(::NzDiscoveryService) }
    val player = remember { DiscoveryPlayer.port() }
    val scope = rememberCoroutineScope()
    val model = remember(service, player, scope) {
        service?.let { ExplorerModel(it, player, scope) }
    }

    if (model == null) {
        Column(modifier.fillMaxWidth()) {
            Text(
                "La découverte par style n'est pas disponible : le noyau de l'application manque.",
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
                modifier = Modifier.padding(NzSpacing.m),
            )
        }
        return
    }

    LaunchedEffect(model) { model.load() }
    val state by model.state.collectAsState()

    // The poll and the busy countdown are the SCREEN's timers, never a core's
    // (docs/agents/mobile.md, "Hot reads"): a panel nobody looks at asks nothing.
    LaunchedEffect(state.phase, state.polls, state.picked?.name, state.level) {
        if (state.phase == ExplorerModel.Phase.Pending) {
            delay(state.pollMs)
            model.poll()
        }
    }
    LaunchedEffect(state.phase, state.retryLeft) {
        if (state.phase == ExplorerModel.Phase.Busy && state.retryLeft > 0) {
            delay(1_000)
            model.tick()
        }
    }

    ExplorerPreview(
        state = state,
        onOpen = model::chooseStyle,
        onStation = { name -> model.startStation(name) },
        onAllStations = model::openStations,
        onRetry = model::retry,
        modifier = modifier,
    )

    if (state.picked != null) {
        DiscoverSheet(model = model, state = state)
    }
    if (state.allOpen) {
        StationsSheet(model = model, state = state)
    }
}

/**
 * The section as a pure function of [state]: what a screenshot renders and what
 * [ExplorerStylesSection] shows, so the pictures prove the REAL layout with no
 * core behind it.
 */
@Composable
fun ExplorerPreview(
    state: ExplorerModel.State,
    onOpen: (uniffi.nz_ffi.StyleTileData) -> Unit,
    onStation: (String) -> Unit,
    onAllStations: () -> Unit,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
        CompositionLocalProvider(LocalContentColor provides Sillon.colors.tx) {
        ExplorerHead(hasTiles = state.tiles.isNotEmpty(), onAllStations = onAllStations)
        when (val screen = state.screen) {
            is ScreenState.Loading -> TilesSkeleton()
            is ScreenState.Failed -> LoadFailed(
                cause = screen.cause,
                onRetry = onRetry,
                title = "Impossible de charger vos styles",
            )

            is ScreenState.Ready -> {
                if (state.tiles.isEmpty()) {
                    Text(
                        if (screen.data.source == "none") {
                            "Votre historique d'écoute est encore vide : les styles que vous écoutez le plus apparaîtront ici. « Toutes les stations » lance un genre sans historique."
                        } else {
                            "Aucun style analysé pour l'instant : « Toutes les stations » en propose un."
                        },
                        style = Sillon.type.bodySmall,
                        color = Sillon.colors.dim,
                    )
                } else {
                    TileGrid(
                        state = state,
                        onOpen = onOpen,
                        onStation = onStation,
                    )
                    if (state.stale) {
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                            Text(
                                "Liste enregistrée sur cet appareil — l'actualisation a échoué.",
                                style = Sillon.type.bodySmall,
                                color = Sillon.colors.dim,
                            )
                            NzButton(text = "Réessayer", kind = NzButtonKind.Ghost, onClick = onRetry)
                        }
                    }
                }
            }
        }
        }
    }
}

@Composable
private fun ExplorerHead(hasTiles: Boolean, onAllStations: () -> Unit) {
    val colors = Sillon.colors
    // The page's `.head` wraps: a title and its lead, then the door on a line of
    // its own when there is no room — side by side, a 360 dp screen broke
    // « Parcourir par style » across two lines (the first capture).
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        Text(
            DiscoveryCopy.EXPLORER_TITLE,
            style = Sillon.type.titleSmall,
            modifier = Modifier.semantics { heading() },
        )
        if (hasTiles) {
            Text(DiscoveryCopy.EXPLORER_LEAD, style = Sillon.type.bodySmall, color = colors.dim)
        }
        NzButton(
            text = DiscoveryCopy.ALL_STATIONS,
            glyph = NzGlyph.Radio,
            kind = NzButtonKind.Ghost,
            onClick = onAllStations,
        )
    }
}

/**
 * A tile is a CARD with two actions, the page's own layout: the whole surface
 * opens the sheet, the 44 dp disc above it starts the station (`a button may not
 * hold another button`, hence the flat layers).
 */
@Composable
private fun TileGrid(
    state: ExplorerModel.State,
    onOpen: (uniffi.nz_ffi.StyleTileData) -> Unit,
    onStation: (String) -> Unit,
) {
    val colors = Sillon.colors
    val type = Sillon.type
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        state.tiles.chunked(2).forEach { pair ->
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                pair.forEach { tile ->
                    val (top, bottom) = nzGrooveFor(tile.name)
                    Box(
                        modifier = Modifier
                            .weight(1f)
                            .height(112.dp)
                            .clip(RoundedCornerShape(18.dp))
                            .background(colors.s1),
                    ) {
                        Box(
                            Modifier
                                .fillMaxSize()
                                .padding(bottom = 54.dp)
                                .background(top),
                        )
                        Box(Modifier.fillMaxSize().padding(bottom = 54.dp)) {
                            Box(
                                Modifier
                                    .size(96.dp)
                                    .align(Alignment.Center)
                                    .background(bottom, CircleShape),
                            )
                        }
                        Column(
                            modifier = Modifier.align(Alignment.BottomStart).padding(NzSpacing.s),
                        ) {
                            Text(tile.name, style = type.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(tile.countLabel, style = type.mono, color = colors.dim)
                            Box(
                                Modifier
                                    .fillMaxWidth(tile.barPct.coerceIn(0u, 100u).toFloat() / 100f)
                                    .height(2.dp)
                                    .background(colors.acText),
                            )
                        }
                        Box(
                            Modifier
                                .matchParentSize()
                                .clip(RoundedCornerShape(18.dp))
                                .semantics {
                                    contentDescription = "Découvrir ${tile.name} — ${tile.countLabel}"
                                }
                                .clickable(
                                    role = androidx.compose.ui.semantics.Role.Button,
                                    onClick = { onOpen(tile) },
                                ),
                        )
                        Box(
                            modifier = Modifier
                                .align(Alignment.BottomEnd)
                                .padding(end = 6.dp, bottom = 58.dp)
                                .size(44.dp)
                                .clip(CircleShape)
                                .background(colors.btn)
                                .clickable(
                                    role = androidx.compose.ui.semantics.Role.Button,
                                    onClick = { onStation(tile.name) },
                                ),
                            contentAlignment = Alignment.Center,
                        ) {
                            if (state.stationBusy == tile.name) {
                                Text("…", style = type.label, color = colors.onbtn)
                            } else {
                                Icon(NzGlyph.Play, size = 18.dp, tint = colors.onbtn, contentDescription = "Lancer la station ${tile.name}")
                            }
                        }
                    }
                }
                if (pair.size == 1) {
                    Box(Modifier.weight(1f))
                }
            }
        }
    }
}

/** The discovery sheet: the breadth slider, « Découvrir », and the five answers. */
@Composable
private fun DiscoverSheet(model: ExplorerModel, state: ExplorerModel.State) {
    val colors = Sillon.colors
    val type = Sillon.type
    val tile = state.picked ?: return
    NzSheet(visible = true, onDismiss = model::close) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(NzSpacing.l),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text("DÉCOUVERTE PAR STYLE", style = type.labelMono, color = colors.dim)
                    Text(tile.name, style = type.titleSmall, modifier = Modifier.semantics { heading() })
                    Text(
                        "${tile.countLabel} dans votre historique · playlists et titres vérifiés chez Deezer",
                        style = type.bodySmall,
                        color = colors.dim,
                    )
                }
                NzIconButton(glyph = NzGlyph.Close, contentDescription = "Fermer", onClick = model::close)
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Niveau de découverte", style = type.label)
                Text(" · ", style = type.label, color = colors.dim)
                Text(state.aim?.name ?: "Équilibré", style = type.label)
            }
            LazyRow(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                items(state.levels, key = { it.level }) { notch ->
                    NzButton(
                        text = notch.name,
                        kind = if (notch.level == state.level.toUInt()) NzButtonKind.Primary else NzButtonKind.Ghost,
                        onClick = { model.setLevel(notch.level.toInt()) },
                    )
                }
            }
            Text(state.aim?.hint.orEmpty(), style = type.bodySmall, color = colors.dim)
            NzButton(
                text = if (state.phase == ExplorerModel.Phase.Pending) "Recherche…" else "Découvrir",
                glyph = NzGlyph.Search,
                onClick = model::discover,
                enabled = state.phase != ExplorerModel.Phase.Pending && state.phase != ExplorerModel.Phase.Busy,
                modifier = Modifier.fillMaxWidth(),
            )
            when (state.phase) {
                ExplorerModel.Phase.Pending -> Text(
                    "Recherche de playlists Deezer pour ${tile.name}… La vérification des titres prend quelques secondes.",
                    style = type.bodySmall,
                    color = colors.dim,
                )

                ExplorerModel.Phase.Busy -> {
                    Text(state.cause.ifEmpty { "Le serveur est occupé." }, style = type.body)
                    Text(state.busyWait, style = type.label, color = colors.dim)
                    NzButton(text = "Réessayer", kind = NzButtonKind.Ghost, onClick = model::retryDiscover)
                }

                ExplorerModel.Phase.Error -> {
                    Text(state.cause, style = type.body)
                    NzButton(text = "Réessayer", kind = NzButtonKind.Ghost, onClick = model::retryDiscover)
                }

                ExplorerModel.Phase.Empty -> {
                    Text(
                        "Aucune playlist Deezer ne correspond à ${tile.name} à ce niveau.",
                        style = type.body,
                    )
                    if (state.level < 5) {
                        NzButton(
                            text = "Essayer un niveau plus large",
                            kind = NzButtonKind.Ghost,
                            onClick = model::widen,
                        )
                    } else {
                        Text("Essayez un autre style.", style = type.bodySmall, color = colors.dim)
                    }
                }

                ExplorerModel.Phase.Ready -> LazyColumn(
                    modifier = Modifier.heightIn(max = 320.dp),
                    contentPadding = PaddingValues(vertical = NzSpacing.xs),
                    verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
                ) {
                    items(state.playlists, key = { it.id }) { playlist ->
                        PlaylistRow(playlist, onPlay = { model.playPlaylist(playlist) })
                    }
                }

                ExplorerModel.Phase.Idle -> Text(DiscoveryCopy.STYLE_HINT, style = type.bodySmall, color = colors.dim)
            }
        }
    }
}

@Composable
private fun PlaylistRow(playlist: PlaylistTracks, onPlay: () -> Unit) {
    val colors = Sillon.colors
    val type = Sillon.type
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        CoverImage(url = playlist.cover, sizeDp = 52.dp, seed = playlist.id)
        Column(Modifier.weight(1f)) {
            Text(playlist.title, style = type.body, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(playlist.countLabel, style = type.label, color = colors.dim, maxLines = 1)
        }
        NzButton(
            text = "Lire",
            glyph = NzGlyph.Play,
            onClick = onPlay,
            enabled = playlist.trackCount > 0,
        )
    }
}

/** "Toutes les stations": the kept list first, one `/flow/styles`, then a search. */
@Composable
private fun StationsSheet(model: ExplorerModel, state: ExplorerModel.State) {
    val colors = Sillon.colors
    val type = Sillon.type
    NzSheet(visible = true, onDismiss = model::closeStations) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(NzSpacing.l),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text("STATIONS", style = type.labelMono, color = colors.dim)
                    Text(
                        DiscoveryCopy.ALL_STATIONS,
                        style = type.titleSmall,
                        modifier = Modifier.semantics { heading() },
                    )
                    Text(
                        "Un genre, ses sous-genres et votre écoute." +
                            if (state.allRows is ScreenState.Ready) {
                                " ${state.allRows.data.size} styles"
                            } else {
                                ""
                            },
                        style = type.bodySmall,
                        color = colors.dim,
                    )
                }
                NzIconButton(glyph = NzGlyph.Close, contentDescription = "Fermer", onClick = model::closeStations)
            }
            SearchField(
                query = state.allQuery,
                onQuery = model::searchStations,
                placeholder = "Chercher un style",
            )
            when (val rows = state.allRows) {
                is ScreenState.Loading -> Text("Chargement des stations…", style = type.bodySmall, color = colors.dim)
                is ScreenState.Failed -> {
                    Text("Impossible de charger les stations (${rows.cause}).", style = type.body)
                    NzButton(text = "Réessayer", kind = NzButtonKind.Ghost, onClick = model::loadStations)
                }

                is ScreenState.Ready -> {
                    val shown: List<StationRowData> = state.shownStations
                    if (shown.isEmpty()) {
                        Text(
                            if (state.allQuery.isBlank()) {
                                "Aucun style analysé ne correspond à votre bibliothèque."
                            } else {
                                "Aucun style analysé ne correspond à cette recherche."
                            },
                            style = type.body,
                        )
                    } else {
                        LazyColumn(
                            modifier = Modifier.heightIn(max = 360.dp),
                            verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
                        ) {
                            items(shown, key = { it.name }) { row ->
                                StationRowView(
                                    row = row,
                                    busy = state.stationBusy == row.name,
                                    onPlay = { model.startStation(row.name, fromSheet = true) },
                                )
                            }
                        }
                    }
                    if (rows.stale) {
                        Text(
                            "Liste gardée sur cet appareil — l'actualisation a échoué.",
                            style = type.bodySmall,
                            color = colors.dim,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun StationRowView(row: StationRowData, busy: Boolean, onPlay: () -> Unit) {
    val colors = Sillon.colors
    val type = Sillon.type
    Row(
        modifier = Modifier.fillMaxWidth().heightIn(min = NzMinTouchTarget),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Column(Modifier.weight(1f)) {
            Text(row.name, style = type.body, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                (row.parent?.let { "$it · " } ?: "") + row.countLabel,
                style = type.label,
                color = colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        NzButton(
            text = if (busy) "…" else "Station",
            glyph = NzGlyph.Play,
            onClick = onPlay,
            enabled = !busy,
        )
    }
}

@Composable
private fun TilesSkeleton() {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        repeat(3) {
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                repeat(2) {
                    Box(
                        Modifier
                            .weight(1f)
                            .height(112.dp)
                            .clip(RoundedCornerShape(18.dp))
                            .background(colors.s1),
                    )
                }
            }
        }
    }
}
