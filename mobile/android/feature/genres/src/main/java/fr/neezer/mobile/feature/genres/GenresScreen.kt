package fr.neezer.mobile.feature.genres

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.ListWindow
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzTopBar
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.rememberListWindow
import kotlinx.coroutines.flow.collect
import uniffi.nz_ffi.GenreCandidate
import uniffi.nz_ffi.TrackRow
import uniffi.nz_ffi.GenreRankedTag
import uniffi.nz_ffi.GenreTagRow

/**
 * Le studio des genres (lot MB18) : quatre onglets, la même page que la SPA.
 *
 * L'écran ne décide rien : il peint [GenresModel.State] et lui passe les gestes.
 * La copie est celle de [GenresCopy] ; les icônes passent par [Icon] (aucun
 * emoji) ; les listes longues passent par le fenêtrage du design system.
 */
@Composable
fun GenresEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    val service = remember { AppGenres.service() }
    if (service == null) {
        EmptyState(
            title = GenresCopy.TITLE,
            body = GenresCopy.CORE_MISSING,
            modifier = modifier.fillMaxSize(),
            glyph = NzGlyph.Tag,
        )
        return
    }
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    val model = remember(service) { GenresModel(service, scope) }
    LaunchedEffect(service) {
        GenreYielding.install(service)
        model.open()
    }
    GenresPage(nav = nav, model = model, modifier = modifier)
}

/**
 * Le studio lui-même, sans la coque : c'est aussi ce que les captures rendent
 * (une capture d'une maquette n'aurait pas attrapé une mise en page cassée).
 */
@Composable
fun GenresPage(nav: NzNavigator?, model: GenresModel, modifier: Modifier = Modifier) {
    val state = model.state
    Column(modifier = modifier.fillMaxSize().background(Sillon.colors.bg)) {
        NzTopBar(title = GenresCopy.TITLE, onBack = nav?.let { back -> { back.back() } })
        // A scrollable row of chips, not `NzSegmented`: four French tab labels on
        // 360 dp made every one of them wrap ("Étiquet / age", "Biblioth / èque") —
        // a pill that breaks its own label is a bug (rule 5, measured).
        androidx.compose.foundation.lazy.LazyRow(
            modifier = Modifier
                .fillMaxWidth()
                .padding(vertical = NzSpacing.s),
            contentPadding = androidx.compose.foundation.layout.PaddingValues(
                horizontal = NzSpacing.pageX,
            ),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        ) {
            items(count = GenresCopy.TABS.size) { index ->
                val tab = GenresCopy.TABS[index]
                NzChip(
                    text = tab,
                    onClick = { model.selectTab(tab) },
                    selected = state.tab == tab,
                )
            }
        }
        Box(modifier = Modifier.fillMaxSize()) {
            when (state.tab) {
                GenresCopy.TAB_LIBRARY -> LibraryTab(model, state)
                GenresCopy.TAB_TREE -> TreeTab(model, state)
                GenresCopy.TAB_MODEL -> ModelTab(model, state)
                else -> TagTab(model, state)
            }
        }
    }
    // The sheets live above the page, whatever the tab.
    when (val sheet = state.sheet) {
        is GenresModel.Sheet.Tag -> GenreTagSheetPage(model = model, state = state, sheet = sheet)
        is GenresModel.Sheet.Bulk -> GenreBulkSheetPage(model = model, state = state, sheet = sheet)
        null -> Unit
    }
}

// --- *Étiquetage* ---------------------------------------------------------------

@Composable
private fun TagTab(model: GenresModel, state: GenresModel.State) {
    val page = state.candidates
    val studio = state.studio
    when {
        // A failure with nothing to draw says why, and offers « Réessayer ».
        page.failure != null && page.data == null -> LoadFailed(
            cause = page.failure,
            onRetry = { model.loadCandidates() },
            modifier = Modifier.padding(NzSpacing.pageX),
        )
        studio.failure != null && studio.data == null -> LoadFailed(
            cause = studio.failure,
            onRetry = { model.open() },
            modifier = Modifier.padding(NzSpacing.pageX),
        )
        page.data != null && page.data.rows.isEmpty() -> EmptyState(
            title = GenresCopy.NO_CANDIDATES,
            body = GenresCopy.NO_CANDIDATES_BODY,
            glyph = NzGlyph.Check,
            actionLabel = GenresCopy.RETRY,
            onAction = { model.loadCandidates() },
            modifier = Modifier.padding(NzSpacing.pageX),
        )
        else -> Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.s),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            SortRow(model, state)
            val candidate = state.current
            if (candidate != null) {
                CandidateCard(candidate, state)
                PicksCard(model, state)
                Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                    NzButton(
                        text = GenresCopy.BACK,
                        onClick = { model.back() },
                        kind = NzButtonKind.Secondary,
                        glyph = NzGlyph.ChevronLeft,
                        enabled = state.cursor > 0 && !state.busy,
                    )
                    NzButton(
                        text = GenresCopy.SKIP,
                        onClick = { model.skip() },
                        kind = NzButtonKind.Ghost,
                        glyph = NzGlyph.ChevronRight,
                        enabled = !state.busy,
                    )
                    if (state.last != null) {
                        NzButton(
                            text = GenresCopy.UNDO,
                            onClick = { model.undo() },
                            kind = NzButtonKind.Secondary,
                            glyph = NzGlyph.Refresh,
                            enabled = !state.busy,
                        )
                    }
                }
                if (page.stale) StaleNote()
                Text(
                    text = GenresCopy.labelledTracks(state.labelled) + " · " +
                        GenresCopy.countLabel(page.data?.rows?.size ?: 0, "candidat", "candidats"),
                    style = Sillon.type.labelSmall,
                    color = Sillon.colors.dim,
                )
            }
        }
    }
}

@Composable
private fun SortRow(model: GenresModel, state: GenresModel.State) {
    Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        NzChip(
            text = GenresCopy.SORT_PLAYS,
            onClick = { model.setSort(GenresModel.SORT_PLAYS) },
            selected = state.sort == GenresModel.SORT_PLAYS,
        )
        NzChip(
            text = GenresCopy.SORT_ACTIVE,
            onClick = { model.setSort(GenresModel.SORT_ACTIVE) },
            selected = state.sort == GenresModel.SORT_ACTIVE,
        )
    }
}

@Composable
private fun CandidateCard(candidate: GenreCandidate, state: GenresModel.State) {
    NzCard(modifier = Modifier.fillMaxWidth().testTag("genre-candidate")) {
        Column(
            modifier = Modifier.padding(NzSpacing.l),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        ) {
            Text(
                text = candidate.title,
                style = Sillon.type.titleSmall,
                color = Sillon.colors.tx,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = listOf(candidate.artist, candidate.album)
                    .filter { it.isNotBlank() }
                    .joinToString(" · "),
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            val facts = buildList {
                add(clock(candidate.durationS.toInt()))
                if (candidate.playCount > 0u) {
                    add(GenresCopy.countLabel(candidate.playCount.toInt(), "écoute", "écoutes"))
                }
                candidate.bpm?.let { add("${it.toInt()} BPM") }
                candidate.predicted?.let { add("le modèle dit $it") }
            }
            Text(
                text = facts.joinToString(" · "),
                style = Sillon.type.labelSmall,
                color = Sillon.colors.dim,
            )
            if (state.query.isNotEmpty()) {
                Text(
                    text = "« ${state.query} »",
                    style = Sillon.type.label,
                    color = Sillon.colors.acText,
                )
            }
        }
    }
}

@Composable
private fun PicksCard(model: GenresModel, state: GenresModel.State) {
    NzCard(modifier = Modifier.fillMaxWidth().testTag("genre-picks")) {
        Column(
            modifier = Modifier.padding(NzSpacing.m),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Text(
                text = if (state.query.isEmpty()) GenresCopy.RANKED_HINT else GenresCopy.SEARCH_HINT,
                style = Sillon.type.labelSmall,
                color = Sillon.colors.dim,
            )
            val rows = if (state.query.isEmpty()) state.picks else state.ranked.take(24)
            rows.forEach { tag ->
                PickRow(tag = tag, enabled = !state.busy) { model.label(tag) }
            }
            GenreSearchField(
                query = state.query,
                onQuery = { model.setQuery(it) },
                hint = GenresCopy.SEARCH_HINT,
            )
        }
    }
}

@Composable
internal fun PickRow(tag: GenreRankedTag, enabled: Boolean, onClick: () -> Unit) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .sizeIn(minHeight = 48.dp)
            .clip(RoundedCornerShape(12.dp))
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        // The genre's own colour, when it has one: a 6 dp dot, never a filled row.
        val dot = tag.color?.let { parseColor(it) } ?: colors.ln
        Box(Modifier.sizeIn(minWidth = 6.dp, minHeight = 6.dp).clip(RoundedCornerShape(3.dp)).background(dot))
        Column(modifier = Modifier.weight(1f)) {
            Text(tag.name, style = Sillon.type.body, color = colors.tx, maxLines = 1)
            val detail = buildList {
                if (tag.count > 0u) add(GenresCopy.countLabel(tag.count.toInt(), "titre", "titres"))
            }
            if (detail.isNotEmpty()) {
                Text(
                    text = detail.joinToString(" · "),
                    style = Sillon.type.labelSmall,
                    color = colors.dim,
                    maxLines = 1,
                )
            }
        }
        if (tag.score != null) {
            Text(text = percent(tag.score!!), style = Sillon.type.label, color = colors.acText)
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun GenreSearchField(query: String, onQuery: (String) -> Unit, hint: String) {
    OutlinedTextField(
        value = query,
        onValueChange = onQuery,
        singleLine = true,
        label = { Text(hint, style = Sillon.type.labelSmall) },
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
        modifier = Modifier
            .fillMaxWidth()
            .testTag("genre-search"),
    )
}

@Composable
private fun StaleNote() {
    Text(
        text = GenresCopy.OFFLINE_COPY,
        style = Sillon.type.labelSmall,
        color = Sillon.colors.dim,
    )
}

// --- *Bibliothèque* --------------------------------------------------------------

@Composable
private fun LibraryTab(model: GenresModel, state: GenresModel.State) {
    val studio = state.studio
    if (studio.failure != null && studio.data == null) {
        LoadFailed(
            cause = studio.failure,
            onRetry = { model.open() },
            modifier = Modifier.padding(NzSpacing.pageX),
        )
        return
    }
    var newName by remember { mutableStateOf("") }
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.s),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Text(GenresCopy.LIBRARY_TITLE, style = Sillon.type.titleSmall, color = Sillon.colors.tx)
        Text(GenresCopy.LIBRARY_BODY, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
        Row(
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            OutlinedTextField(
                value = newName,
                onValueChange = { newName = it },
                singleLine = true,
                label = { Text("Nom du genre", style = Sillon.type.labelSmall) },
                modifier = Modifier.weight(1f).testTag("genre-new-tag"),
            )
            NzButton(
                text = "Ajouter",
                onClick = {
                    model.createTag(newName)
                    newName = ""
                },
                kind = NzButtonKind.Secondary,
                glyph = NzGlyph.Plus,
                enabled = !state.busy && newName.isNotBlank(),
            )
        }
        NzButton(
            text = GenresCopy.DEFAULTS,
            onClick = { model.restoreDefaults() },
            kind = NzButtonKind.Ghost,
            glyph = NzGlyph.Refresh,
            enabled = !state.busy,
        )
        NzCard(modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.padding(NzSpacing.m), verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                Text(GenresCopy.EMBED, style = Sillon.type.label, color = Sillon.colors.tx)
                Text(GenresCopy.EMBED_BODY, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
                val embed = state.model.data?.embed
                if (embed != null) {
                    val label = buildString {
                        append(if (embed.running) GenresCopy.EMBED_RUNNING else GenresCopy.EMBED_DONE)
                        append(" · ")
                        append(
                            GenresCopy.measured(embed.done.toInt(), embed.total.toInt(), embed.failed.toInt()),
                        )
                    }
                    Text(label, style = Sillon.type.labelSmall, color = Sillon.colors.dim)
                    if (embed.causes.isNotEmpty()) {
                        Text(
                            text = GenresCopy.EMBED_FAILURES + " : " + embed.causes
                                .joinToString(" · ") { "${it.reason} (${it.count})" },
                            style = Sillon.type.labelSmall,
                            color = Sillon.colors.dim,
                        )
                    }
                }
                NzButton(
                    text = GenresCopy.EMBED,
                    onClick = { model.startEmbed(false) },
                    kind = NzButtonKind.Primary,
                    glyph = NzGlyph.Activity,
                    enabled = !state.busy && state.model.data?.embed?.running != true,
                )
                NzButton(
                    text = GenresCopy.EMBED_FORCE,
                    onClick = { model.startEmbed(true) },
                    kind = NzButtonKind.Ghost,
                    enabled = !state.busy && state.model.data?.embed?.running != true,
                )
            }
        }
        ExtractorCard(model, state)
        Text(
            text = GenresCopy.countLabel(state.tags.size, "genre", "genres") + " · " +
                GenresCopy.labelledTracks(state.labelled),
            style = Sillon.type.labelSmall,
            color = Sillon.colors.dim,
        )
        state.tags.forEach { tag ->
            TagRow(tag = tag, enabled = !state.busy, model = model)
        }
    }
}

@Composable
private fun TagRow(tag: GenreTagRow, enabled: Boolean, model: GenresModel) {
    var renaming by remember { mutableStateOf(false) }
    var name by remember(tag.name) { mutableStateOf(tag.name) }
    val colors = Sillon.colors
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(colors.s1)
            .padding(NzSpacing.m),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        if (renaming) {
            Row(
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    singleLine = true,
                    modifier = Modifier.weight(1f),
                )
                NzButton(
                    text = "Enregistrer",
                    onClick = {
                        model.editTag(tag.id.toLong(), name = name)
                        renaming = false
                    },
                    kind = NzButtonKind.Primary,
                    enabled = enabled && name.isNotBlank(),
                )
            }
        } else {
            // The name on its own line: two labelled buttons squeezed it to
            // "Hardcor" (measured on the 360 dp render), and a truncated genre
            // name is the one thing this tab is about.
            Text(tag.name, style = Sillon.type.body, color = colors.tx, maxLines = 1)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = GenresCopy.countLabel(tag.count.toInt(), "titre", "titres") +
                        (tag.family?.let { " · famille $it" } ?: ""),
                    style = Sillon.type.labelSmall,
                    color = colors.dim,
                    modifier = Modifier.weight(1f),
                )
                fr.neezer.mobile.designsystem.NzIconButton(
                    glyph = NzGlyph.Edit,
                    contentDescription = GenresCopy.EDIT_TAG + " " + tag.name,
                    onClick = { renaming = true },
                    enabled = enabled,
                )
                fr.neezer.mobile.designsystem.NzIconButton(
                    glyph = NzGlyph.Trash,
                    contentDescription = GenresCopy.DELETE_TAG + " " + tag.name,
                    onClick = { model.deleteTag(tag.id.toLong()) },
                    enabled = enabled,
                )
            }
        }
    }
}

@Composable
private fun ExtractorCard(model: GenresModel, state: GenresModel.State) {
    val extractor = state.studio.data?.extractor ?: return
    NzCard(modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(NzSpacing.m), verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            Text(GenresCopy.EXTRACTOR, style = Sillon.type.label, color = Sillon.colors.tx)
            Text(
                text = when {
                    extractor.available -> "Installé · ${extractor.dim} dimensions"
                    extractor.reason != null -> extractor.reason!!
                    else -> "Absent : la mesure des empreintes est faite par le serveur."
                },
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
            )
            extractor.sessionError?.let {
                Text(it, style = Sillon.type.labelSmall, color = Sillon.colors.dim)
            }
            state.extractorTest?.let { test ->
                val verdict = when {
                    test.running -> GenresCopy.EXTRACTOR_CHECKING
                    test.ok == true -> GenresCopy.EXTRACTOR_OK
                    else -> test.error ?: GenresCopy.EXTRACTOR_BROKEN
                }
                Text(verdict, style = Sillon.type.labelSmall, color = Sillon.colors.dim)
            }
            NzButton(
                text = GenresCopy.EXTRACTOR_TEST,
                onClick = { model.testExtractor() },
                kind = NzButtonKind.Secondary,
                enabled = extractor.available && !state.busy,
            )
        }
    }
}

// --- *Arbre* --------------------------------------------------------------------

@Composable
private fun TreeTab(model: GenresModel, state: GenresModel.State) {
    val tree = state.tree
    if (tree.failure != null && tree.data == null) {
        LoadFailed(
            cause = tree.failure,
            onRetry = { model.loadTree() },
            modifier = Modifier.padding(NzSpacing.pageX),
        )
        return
    }
    Column(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier.padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.s),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Text(GenresCopy.TREE_TITLE, style = Sillon.type.titleSmall, color = Sillon.colors.tx)
            Text(GenresCopy.TREE_BODY, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
            GenreSearchField(
                query = state.treeQuery,
                onQuery = { model.setTreeQuery(it) },
                hint = GenresCopy.TREE_SEARCH,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                NzButton(
                    text = GenresCopy.TREE_UNDO,
                    onClick = { model.treeUndo() },
                    kind = NzButtonKind.Secondary,
                    glyph = NzGlyph.Refresh,
                    enabled = tree.data?.canUndo == true && !state.busy,
                )
                NzButton(
                    text = GenresCopy.TREE_RESET,
                    onClick = { model.treeReset() },
                    kind = NzButtonKind.Ghost,
                    enabled = tree.data?.defaultTree == false && !state.busy,
                )
            }
            tree.data?.let { data ->
                Text(
                    text = "${data.nodes} ${GenresCopy.TREE_NODES} · ${data.edges} ${GenresCopy.TREE_EDGES}" +
                        if (data.defaultTree) " · " + GenresCopy.TREE_DEFAULT else "",
                    style = Sillon.type.labelSmall,
                    color = Sillon.colors.dim,
                )
            }
        }
        TreeList(model = model, state = state)
    }
}

/**
 * Les lignes de l'arbre, fenêtrées par le design system (`rememberListWindow` sur
 * le `ListHandle` du noyau, une source locale quand un test en fournit une).
 *
 * L'acceptation est mesurée ICI : un arbre de 1 000 nœuds se fait fenêtrer, donc
 * seules les lignes visibles existent.
 */
@Composable
internal fun TreeList(model: GenresModel, state: GenresModel.State) {
    val rows = state.treeRows
    if (rows == null || rows.len() == 0) {
        EmptyState(
            title = GenresCopy.TREE_TITLE,
            body = GenresCopy.NO_CANDIDATES_BODY,
            glyph = NzGlyph.Sort,
            modifier = Modifier.padding(NzSpacing.pageX),
        )
        return
    }
    val window = rememberTreeWindow(rows)
    val listState = rememberLazyListState()
    LaunchedEffect(window, listState) {
        snapshotFlow { listState.firstVisibleItemIndex to listState.isScrollInProgress }
            .collect { (first, scrolling) -> window.onVisible(first, first + VISIBLE_ROWS, scrolling) }
    }
    LazyColumn(state = listState, modifier = Modifier.fillMaxSize().testTag("genre-tree")) {
        items(count = window.len, key = { index -> "tree-row-$index" }) { index ->
            val row = window.rowAt(index)
            if (row == null) {
                TreeRowPlaceholder()
            } else {
                TreeRow(row = row, model = model, enabled = !state.busy)
            }
        }
    }
}

private const val VISIBLE_ROWS = 14

@Composable
private fun rememberTreeWindow(rows: TreeRows): ListWindow {
    val handle = rows.handle
    return if (handle != null) {
        rememberListWindow(handle)
    } else {
        val source = remember(rows) {
            object : fr.neezer.mobile.designsystem.NzRowSource {
                override fun len(): Int = rows.len()

                override fun window(start: Int, count: Int): List<TrackRow> = rows.window(start, count)
            }
        }
        remember(source) { ListWindow(source, 64) }
    }
}

@Composable
private fun TreeRowPlaceholder() {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .height(44.dp)
            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.xs)
            .clip(RoundedCornerShape(8.dp))
            .background(Sillon.colors.s1),
    )
}

@Composable
private fun TreeRow(row: TrackRow, model: GenresModel, enabled: Boolean) {
    val colors = Sillon.colors
    val branch = row.source == "branch" || row.source == "root"
    var menu by remember { mutableStateOf(false) }
    Column {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .sizeIn(minHeight = 48.dp)
                .background(colors.bg)
                .clickable(enabled = branch, role = Role.Button) { model.toggleRow(row.key) }
                .padding(
                    start = NzSpacing.pageX + (row.position.toInt() * 14).dp,
                    end = NzSpacing.pageX,
                    top = NzSpacing.xs,
                    bottom = NzSpacing.xs,
                ),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            if (branch) {
                Icon(
                    glyph = if (row.downloaded) NzGlyph.ChevronDown else NzGlyph.ChevronRight,
                    size = 14.dp,
                    tint = colors.dim,
                    contentDescription = null,
                )
            } else {
                Box(Modifier.sizeIn(minWidth = 14.dp, minHeight = 14.dp))
            }
            Column(Modifier.weight(1f)) {
                Text(row.title, style = Sillon.type.body, color = colors.tx, maxLines = 1)
                val detail = buildList {
                    if (row.artists.isNotEmpty()) add("sous ${row.artists}")
                    if (row.durationS > 0u) {
                        add(GenresCopy.countLabel(row.durationS.toInt(), "sous-genre", "sous-genres"))
                    }
                }
                if (detail.isNotEmpty()) {
                    Text(
                        text = detail.joinToString(" · "),
                        style = Sillon.type.labelSmall,
                        color = colors.dim,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            fr.neezer.mobile.designsystem.NzIconButton(
                glyph = NzGlyph.More,
                contentDescription = "Actions sur " + row.title,
                onClick = { menu = !menu },
                enabled = enabled,
            )
        }
        if (menu) {
            Row(
                modifier = Modifier.padding(start = NzSpacing.pageX, bottom = NzSpacing.xs),
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            ) {
                if (row.artists.isNotEmpty()) {
                    NzButton(
                        text = GenresCopy.TREE_CUT,
                        onClick = {
                            model.treeCut(row.id, row.artists)
                            menu = false
                        },
                        kind = NzButtonKind.Ghost,
                        enabled = enabled,
                    )
                }
                NzButton(
                    text = GenresCopy.TREE_CREATE,
                    onClick = {
                        model.createTag(row.id)
                        menu = false
                    },
                    kind = NzButtonKind.Ghost,
                    enabled = enabled,
                )
            }
        }
    }
}

// --- *Modèle* -------------------------------------------------------------------

@Composable
private fun ModelTab(model: GenresModel, state: GenresModel.State) {
    val modelTab = state.model
    if (modelTab.failure != null && modelTab.data == null) {
        LoadFailed(
            cause = modelTab.failure,
            onRetry = { model.loadModel() },
            modifier = Modifier.padding(NzSpacing.pageX),
        )
        return
    }
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.s),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        ModelStateCard(model, state)
        ReadinessCard(state)
        TrainingCard(model, state)
        state.training.head?.let { head ->
            HealthCard(head.healthLevel, head.healthRemarks)
            PerClassCard(head.perClass.map { Triple(it.label, it.recall, it.examples.toInt()) })
            if (head.confusions.isNotEmpty()) {
                ConfusionsCard(head.confusions.map { Triple(it.from, it.to, it.count.toInt()) })
            }
            if (head.cleanup.isNotEmpty()) {
                CleanupCard(head.cleanup)
            }
        }
        MeasureCard(model, state)
    }
}

@Composable
private fun SectionTitle(text: String, body: String? = null) {
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        Text(text, style = Sillon.type.titleSmall, color = Sillon.colors.tx)
        if (body != null) {
            Text(body, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
        }
    }
}

@Composable
private fun ModelStateCard(model: GenresModel, state: GenresModel.State) {
    val active = state.studio.data?.model
    NzCard(modifier = Modifier.fillMaxWidth().testTag("genre-model-state")) {
        Column(Modifier.padding(NzSpacing.m), verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            SectionTitle(GenresCopy.MODEL_TITLE)
            if (active == null) {
                Text(GenresCopy.MODEL_NONE, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
            } else {
                Text(
                    text = GenresCopy.classes(active.labels.size) + " · " + active.kind +
                        " · " + (active.balanced?.let { GenresCopy.balanced((it * 100).toInt()) } ?: ""),
                    style = Sillon.type.bodySmall,
                    color = Sillon.colors.tx,
                )
                Text(
                    text = if (active.usable) GenresCopy.MODEL_READY else GenresCopy.MODEL_UNUSABLE,
                    style = Sillon.type.labelSmall,
                    color = Sillon.colors.dim,
                )
                Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                    NzButton(
                        text = GenresCopy.MODEL_DISABLE,
                        onClick = { model.disableHead() },
                        kind = NzButtonKind.Ghost,
                        enabled = !state.busy,
                    )
                    NzButton(
                        text = GenresCopy.MODEL_RELABEL,
                        onClick = { model.relabelNow() },
                        kind = NzButtonKind.Secondary,
                        glyph = NzGlyph.Refresh,
                        enabled = !state.busy && !state.relabelRunning,
                    )
                }
            }
            val relabel = state.model.data?.relabel
            if (relabel != null) {
                Text(
                    text = if (relabel.running) {
                        GenresCopy.MODEL_RELABEL_RUNNING + " · " +
                            GenresCopy.relabelRunning(relabel.scanned.toInt(), relabel.total.toInt())
                    } else {
                        GenresCopy.MODEL_RELABEL_DONE + " · " +
                            GenresCopy.relabelDone(relabel.scanned.toInt(), relabel.changed.toInt())
                    },
                    style = Sillon.type.labelSmall,
                    color = Sillon.colors.dim,
                )
            }
        }
    }
}

@Composable
private fun ReadinessCard(state: GenresModel.State) {
    NzCard(modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(NzSpacing.m), verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            SectionTitle(GenresCopy.READY_TITLE, GenresCopy.READY_BODY)
            val ready = state.readiness.take(12)
            ready.forEach { (name, count) ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = name,
                        style = Sillon.type.labelSmall,
                        color = Sillon.colors.tx,
                        modifier = Modifier.weight(1f),
                        maxLines = 1,
                    )
                    // A bar, not a number: the five-title floor is visible.
                    Box(
                        modifier = Modifier
                            .height(6.dp)
                            .sizeIn(minWidth = (count * 6).coerceAtMost(120).dp)
                            .clip(RoundedCornerShape(3.dp))
                            .background(
                                if (count >= GenresModel.MIN_EXAMPLES) Sillon.colors.ac else Sillon.colors.s2,
                            ),
                    )
                    Text(
                        text = " $count",
                        style = Sillon.type.labelSmall,
                        color = Sillon.colors.dim,
                    )
                }
            }
        }
    }
}

@Composable
private fun TrainingCard(model: GenresModel, state: GenresModel.State) {
    NzCard(modifier = Modifier.fillMaxWidth().testTag("genre-training")) {
        Column(Modifier.padding(NzSpacing.m), verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            SectionTitle(GenresCopy.TRAIN_MODES, GenresCopy.modeDescription(state.mode))
            listOf(
                listOf("auto" to GenresCopy.MODE_AUTO, "linear" to GenresCopy.MODE_LINEAR),
                listOf("deep" to GenresCopy.MODE_DEEP, "deep2" to GenresCopy.MODE_DEEP2),
            ).forEach { row ->
                Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                    row.forEach { (key, label) ->
                        NzChip(
                            text = label,
                            onClick = { model.setMode(key) },
                            selected = state.mode == key,
                            enabled = !state.training.running,
                        )
                    }
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                NzSwitchCompat(
                    checked = state.selfTrain,
                    onChange = { model.setSelfTrain(it) },
                    label = GenresCopy.SELF_TRAIN,
                )
            }
            Text(GenresCopy.SELF_TRAIN_BODY, style = Sillon.type.labelSmall, color = Sillon.colors.dim)
            val training = state.training
            if (training.running) {
                Text(
                    text = training.stage + " · " + percent(training.fraction),
                    style = Sillon.type.label,
                    color = Sillon.colors.tx,
                )
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(4.dp)
                        .clip(RoundedCornerShape(2.dp))
                        .background(Sillon.colors.s2),
                ) {
                    Box(
                        modifier = Modifier
                            .fillMaxWidth(training.fraction.coerceIn(0.0, 1.0).toFloat())
                            .height(4.dp)
                            .background(Sillon.colors.ac),
                    )
                }
                NzButton(
                    text = GenresCopy.TRAIN_STOP,
                    onClick = { model.stopTraining() },
                    kind = NzButtonKind.Ghost,
                )
            } else {
                NzButton(
                    text = if (training.head != null) GenresCopy.TRAIN_AGAIN else GenresCopy.TRAIN,
                    onClick = { model.train() },
                    kind = NzButtonKind.Primary,
                    glyph = NzGlyph.Activity,
                    enabled = state.trainable && !state.busy,
                )
                if (!state.trainable) {
                    Text(GenresCopy.TRAIN_NEED, style = Sillon.type.labelSmall, color = Sillon.colors.dim)
                }
                training.cause?.let {
                    Text(it, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
                }
                val head = training.head
                if (head != null) {
                    Text(
                        text = GenresCopy.balanced((head.balanced * 100).toInt()) +
                            " · " + GenresCopy.examples(head.examples.toInt()) +
                            " · " + GenresCopy.classes(head.classes.toInt()),
                        style = Sillon.type.bodySmall,
                        color = Sillon.colors.tx,
                    )
                    NzButton(
                        text = GenresCopy.MODEL_SEND,
                        onClick = { model.sendHead() },
                        kind = NzButtonKind.Primary,
                        glyph = NzGlyph.Upload,
                        enabled = !state.busy,
                    )
                }
            }
        }
    }
}

@Composable
private fun NzSwitchCompat(checked: Boolean, onChange: (Boolean) -> Unit, label: String) {
    fr.neezer.mobile.designsystem.NzSwitch(
        checked = checked,
        onCheckedChange = onChange,
        label = label,
    )
}

@Composable
private fun HealthCard(level: String, remarks: List<String>) {
    val colors = Sillon.colors
    NzCard(modifier = Modifier.fillMaxWidth().testTag("genre-health")) {
        Column(Modifier.padding(NzSpacing.m), verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            SectionTitle(GenresCopy.HEALTH)
            Text(
                text = when (level) {
                    "good" -> "Fiable"
                    "watch" -> "Utilisable, à surveiller"
                    else -> "Peu fiable : ne pas servir"
                },
                style = Sillon.type.label,
                color = if (level == "good") colors.acText else colors.tx,
            )
            remarks.forEach { remark ->
                Text(remark, style = Sillon.type.bodySmall, color = colors.dim)
            }
        }
    }
}

@Composable
private fun PerClassCard(rows: List<Triple<String, Double, Int>>) {
    NzCard(modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(NzSpacing.m), verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            SectionTitle(GenresCopy.PER_CLASS)
            rows.forEach { (label, recall, examples) ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(label, style = Sillon.type.labelSmall, color = Sillon.colors.tx, modifier = Modifier.weight(1f))
                    Text(
                        text = "${percent(recall)} · ${GenresCopy.examples(examples)}",
                        style = Sillon.type.labelSmall,
                        color = Sillon.colors.dim,
                    )
                }
            }
        }
    }
}

@Composable
private fun ConfusionsCard(rows: List<Triple<String, String, Int>>) {
    NzCard(modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(NzSpacing.m), verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            SectionTitle(GenresCopy.CONFUSIONS)
            rows.forEach { (from, to, count) ->
                Text(
                    text = "$from → $to · $count",
                    style = Sillon.type.labelSmall,
                    color = Sillon.colors.dim,
                )
            }
        }
    }
}

@Composable
private fun CleanupCard(sentences: List<String>) {
    NzCard(modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(NzSpacing.m), verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            SectionTitle(GenresCopy.CLEANUP)
            sentences.forEach { sentence ->
                Text(sentence, style = Sillon.type.labelSmall, color = Sillon.colors.dim)
            }
        }
    }
}

@Composable
private fun MeasureCard(model: GenresModel, state: GenresModel.State) {
    NzCard(modifier = Modifier.fillMaxWidth().testTag("genre-measure")) {
        Column(Modifier.padding(NzSpacing.m), verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            SectionTitle(GenresCopy.MEASURE, GenresCopy.MEASURE_BODY)
            val construction = state.model.data?.construction
            if (construction != null) {
                Text(
                    text = "${GenresCopy.CONSTRUCTION} : " +
                        "${construction.measured}/${construction.labelled} · " +
                        "${construction.missing} " + GenresCopy.BULK_NO_VECTOR,
                    style = Sillon.type.labelSmall,
                    color = Sillon.colors.dim,
                )
            }
            val measure = state.measure
            if (measure != null) {
                Text(
                    text = GenresCopy.measured(measure.done.toInt(), measure.total.toInt(), measure.failed.toInt()) +
                        if (measure.current.isNotEmpty()) " · " + measure.current else "",
                    style = Sillon.type.labelSmall,
                    color = Sillon.colors.dim,
                )
                if (measure.causes.isNotEmpty()) {
                    Text(
                        text = measure.causes.joinToString(" · ") { "${it.reason} (${it.count})" },
                        style = Sillon.type.labelSmall,
                        color = Sillon.colors.dim,
                    )
                }
                measure.stopped?.let {
                    Text(it, style = Sillon.type.labelSmall, color = Sillon.colors.dim)
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                NzButton(
                    text = GenresCopy.MEASURE,
                    onClick = { model.startMeasure() },
                    kind = NzButtonKind.Secondary,
                    glyph = NzGlyph.Activity,
                    enabled = state.measure?.running != true,
                )
                if (state.measure?.running == true) {
                    NzButton(
                        text = GenresCopy.MEASURE_STOP,
                        onClick = { model.stopMeasure() },
                        kind = NzButtonKind.Ghost,
                    )
                }
            }
        }
    }
}

// --- the tag sheet of one track --------------------------------------------------

@Composable
private fun GenreTagSheetPage(
    model: GenresModel,
    state: GenresModel.State,
    sheet: GenresModel.Sheet.Tag,
) {
    fr.neezer.mobile.designsystem.NzSheet(visible = true, onDismiss = { model.closeSheet() }) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(NzSpacing.l),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Text(GenresCopy.SHEET_TITLE, style = Sillon.type.titleSmall, color = Sillon.colors.tx)
            Text(sheet.title, style = Sillon.type.body, color = Sillon.colors.dim, maxLines = 1)
            GenreSearchField(
                query = state.sheetQuery,
                onQuery = { model.setSheetQuery(it) },
                hint = GenresCopy.SEARCH_HINT,
            )
            state.sheetRanked.take(40).forEach { tag ->
                val selected = state.sheetTagId == tag.id.toLong()
                NzChip(
                    text = if (selected) "✓ ${tag.name}" else tag.name,
                    onClick = { model.sheetLabel(if (selected) null else tag.id.toLong()) },
                    selected = selected,
                )
            }
            if (state.sheetTagId != null) {
                NzButton(
                    text = GenresCopy.SHEET_REMOVE,
                    onClick = { model.sheetLabel(null) },
                    kind = NzButtonKind.Ghost,
                    glyph = NzGlyph.Trash,
                )
            }
        }
    }
}

// --- the bulk sheet ---------------------------------------------------------------

@Composable
private fun GenreBulkSheetPage(
    model: GenresModel,
    state: GenresModel.State,
    sheet: GenresModel.Sheet.Bulk,
) {
    fr.neezer.mobile.designsystem.NzSheet(visible = true, onDismiss = { model.closeBulk() }) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(NzSpacing.l),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Text(GenresCopy.BULK_TITLE, style = Sillon.type.titleSmall, color = Sillon.colors.tx)
            Text(
                text = sheet.tag + " · " + sheet.scopeLabel,
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
            )
            Text(GenresCopy.BULK_REVIEW, style = Sillon.type.labelSmall, color = Sillon.colors.dim)
            val preview = state.bulk
            when {
                preview.failure != null -> LoadFailed(
                    cause = preview.failure,
                    onRetry = { model.closeBulk() },
                )
                preview.data == null -> Text(GenresCopy.LOADING, style = Sillon.type.labelSmall, color = Sillon.colors.dim)
                preview.data.rows.isEmpty() -> Text(
                    GenresCopy.BULK_EMPTY,
                    style = Sillon.type.bodySmall,
                    color = Sillon.colors.dim,
                )
                else -> {
                    val counts = preview.data.counts
                    Text(
                        text = listOfNotNull(
                            GenresCopy.countLabel(counts.tracks.toInt(), "titre", "titres"),
                            if (counts.otherTag > 0u) "${counts.otherTag} ${GenresCopy.BULK_OTHER}" else null,
                            if (counts.model > 0u) "${counts.model} ${GenresCopy.BULK_MODEL}" else null,
                            if (counts.same > 0u) "${counts.same} ${GenresCopy.BULK_SAME}" else null,
                            if (counts.noVector > 0u) "${counts.noVector} ${GenresCopy.BULK_NO_VECTOR}" else null,
                            if (counts.unjudged > 0u) "${counts.unjudged} ${GenresCopy.BULK_UNJUDGED}" else null,
                            if (counts.missing > 0u) "${counts.missing} ${GenresCopy.BULK_MISSING}" else null,
                        ).joinToString(" · "),
                        style = Sillon.type.labelSmall,
                        color = Sillon.colors.dim,
                    )
                    preview.data.rows.forEach { row ->
                        val ticked = row.reference in state.bulkTicked
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .sizeIn(minHeight = 48.dp)
                                .clickable(role = Role.Checkbox) { model.toggleBulk(row.reference) }
                                .padding(vertical = NzSpacing.xs),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                        ) {
                            // A box, ticked or empty: a "+" for "not chosen yet"
                            // said the opposite of what the row means.
                            Box(
                                modifier = Modifier
                                    .sizeIn(minWidth = 20.dp, minHeight = 20.dp)
                                    .clip(RoundedCornerShape(4.dp))
                                    .background(if (ticked) Sillon.colors.ac else Color.Transparent)
                                    .then(
                                        if (ticked) {
                                            Modifier
                                        } else {
                                            Modifier.border(1.dp, Sillon.colors.ln, RoundedCornerShape(4.dp))
                                        },
                                    ),
                                contentAlignment = Alignment.Center,
                            ) {
                                if (ticked) {
                                    Icon(
                                        glyph = NzGlyph.Check,
                                        size = 14.dp,
                                        tint = Sillon.colors.onac,
                                        contentDescription = "Coché",
                                    )
                                }
                            }
                            Column(Modifier.weight(1f)) {
                                Text(row.title, style = Sillon.type.body, color = Sillon.colors.tx, maxLines = 1)
                                Text(
                                    text = buildList {
                                        add(row.artist)
                                        row.current?.let { add("${GenresCopy.SHEET_WEARS} $it") }
                                        row.opinion?.let { add("${GenresCopy.BULK_MODEL} : $it") }
                                    }.filter { it.isNotBlank() }.joinToString(" · "),
                                    style = Sillon.type.labelSmall,
                                    color = Sillon.colors.dim,
                                    maxLines = 1,
                                )
                            }
                        }
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                        NzButton(
                            text = GenresCopy.BULK_APPLY + " (${state.bulkTicked.size})",
                            onClick = { model.applyBulk() },
                            kind = NzButtonKind.Primary,
                            enabled = state.bulkTicked.isNotEmpty() && !state.bulkApplying,
                        )
                        NzButton(
                            text = "Fermer",
                            onClick = { model.closeBulk() },
                            kind = NzButtonKind.Ghost,
                        )
                    }
                }
            }
        }
    }
}

// --- small helpers ---------------------------------------------------------------

/** `0.5234` → `52 %` : the one place a score becomes a word. */
internal fun percent(value: Double): String = "${(value.coerceIn(0.0, 1.0) * 100).toInt()} %"

/** `212` → `3:32`. */
internal fun clock(seconds: Int): String {
    if (seconds <= 0) return "0:00"
    val minutes = seconds / 60
    return "$minutes:${(seconds % 60).toString().padStart(2, '0')}"
}

/** `#rrggbb` → a Color, or the hairline when the server sends something else. */
internal fun parseColor(text: String): Color? {
    if (text.length != 7 || !text.startsWith("#")) return null
    return text.drop(1).toLongOrNull(16)?.let { value ->
        Color(0xFF000000 or value)
    }
}
