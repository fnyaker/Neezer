package fr.neezer.mobile.feature.home

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.Avatar
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSkeletonKind
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.Skeleton
import fr.neezer.mobile.designsystem.TrackRowView
import fr.neezer.mobile.designsystem.nzDuration
import fr.neezer.mobile.designsystem.nzGrooveFor
import java.time.LocalDate
import java.time.format.TextStyle
import java.util.Locale
import uniffi.nz_ffi.FlowStateData
import uniffi.nz_ffi.HomeData
import uniffi.nz_ffi.TrackRow

/**
 * Everything the home can do, as one value.
 *
 * The screen is a pure function of [HomeModel.State] and these callbacks, so the
 * screenshot test draws the REAL screen from a fixture with no core, no player
 * and no network.
 */
data class HomeActions(
    /** « Lancer le Flow » (the hero's button and the picker's). */
    val launchFlow: () -> Unit = {},
    /** Opens or closes the hero's discovery panel. */
    val tune: (Boolean) -> Unit = {},
    /** A notch of the hero's row. */
    val notch: (Double?) -> Unit = {},
    /** A stop of the hero's discovery panel. */
    val discovery: (Double) -> Unit = {},
    /** A style chip (« Tout » clears them). */
    val pickStyle: (String) -> Unit = {},
    val clearStyles: () -> Unit = {},
    /** A sleeve of « Nouveautés », by the index of its first title. */
    val playRelease: (Int) -> Unit = {},
    val playAllNew: () -> Unit = {},
    /** « Tout afficher » de la carte Nouveautés (`routes/Home.svelte:252` → `#/releases`). */
    val openReleases: () -> Unit = {},
    /** An artist portrait. */
    val openArtist: (String) -> Unit = {},
    /** A « Pour toi » row: play it, or queue it next (the swipe's gesture). */
    val playReco: (TrackRow) -> Unit = {},
    val queueReco: (TrackRow) -> Unit = {},
    /** The row's ⋯ — MB10's ONE context menu. */
    val more: (TrackRow) -> Unit = {},
    val moreSuggestions: () -> Unit = {},
    /** A « Mix d'ambiance ». */
    val playMood: (String) -> Unit = {},
    /** The Récap card's door. */
    val openRecap: (Int) -> Unit = {},
    /** « Reprendre » opens the player; the idle card opens Explorer. */
    val openPlayer: () -> Unit = {},
    val openSearch: () -> Unit = {},
    val retry: () -> Unit = {},
    /** The Flow's controls. */
    val flowPanel: (Boolean) -> Unit = {},
    val moreLikeThis: () -> Unit = {},
    val lessLikeThis: () -> Unit = {},
    val nudge: (Int) -> Unit = {},
    val resetFlow: () -> Unit = {},
)

/**
 * Accueil — lot MB5, `routes/Home.svelte` ported to Compose.
 *
 * The order is the SPA's, and so is what each block says: the hero asks ONE
 * question (« Quelle énergie ce matin ? ») with the six notches and « Lancer le
 * Flow », « Reprendre » answers « what was I listening to? », then the styles the
 * Flow rests on, the new titles, the followed artists, « Pour toi », the Récap
 * door and the « Mixes d'ambiance » that close the page. Nothing waits on the
 * network to show something: the cache paints, the answer replaces it.
 *
 * Accessibility: every control is a real target of at least [NzMinTouchTarget],
 * every glyph carries its sentence, the notches are a radio group, and no
 * information is carried by colour alone (`webapp.md` §10).
 */
@Composable
fun HomeScreen(
    state: HomeModel.State,
    actions: HomeActions,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            // The screen's own marker, for the smoke (`mobile/tools/ci-emulator.sh`)
            // and for TalkBack: present in EVERY state — hero, skeletons, a
            // failed load — and independent of the clock. `HomeHero`'s greeting
            // names the weekday and the person, so it can never be a stable
            // needle; this can (`Home.svelte`'s masthead, "Quelle énergie ce
            // matin ?", is fixed copy but reads like a time of day).
            .semantics { contentDescription = HOME_MARKER }
            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.l),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xl),
    ) {
        // The hero is drawn in EVERY state, like `Home.svelte` (HomeHero sits
        // above the sections, never inside them): the day's one decision is the
        // page, and a section that failed is a block below it, not a screen that
        // replaces it.
        HomeHero(state, actions)
        if (state.tuneOpen) TunePanel(state, actions)

        val data = state.data
        when (val screen = state.screen) {
            is fr.neezer.mobile.bridge.ScreenState.Failed -> {
                LoadFailed(
                    cause = screen.cause,
                    onRetry = { actions.retry() },
                    title = "L'accueil n'a pas pu se charger",
                )
                StylePicker(state, actions)
                FlowControls(state, actions)
            }
            else -> {
                if (data == null) {
                    // First paint with nothing yet: a shelf and a list, never an
                    // empty page (Skeleton.svelte's rule).
                    Skeleton(kind = NzSkeletonKind.Shelf, count = 6)
                    Skeleton(kind = NzSkeletonKind.List, count = 6)
                } else {
                    if (state.stale) StaleNote()
                    ResumePanel(data, actions)
                    StylePicker(state, actions)
                    val releases = data.releases
                    if (releases != null && releases.tracks.isNotEmpty()) {
                        ReleasesShelf(releases, actions)
                    }
                    if (data.artists.isNotEmpty()) ArtistsRow(data, actions)
                    if (data.reco.isNotEmpty()) ForYou(data, state, actions)
                    data.recap?.let { RecapPanel(it, actions) }
                    if (data.moods.isNotEmpty()) MoodsRow(data, state, actions)
                    FlowControls(state, actions)
                    if (data.isHomeEmpty()) EmptyHome(state, actions)
                }
            }
        }
    }
}

/** `Home.svelte#empty`/`#allFailed`, read from what the answer carries. */
private fun HomeData.isHomeEmpty(): Boolean =
    releases?.tracks.isNullOrEmpty() && artists.isEmpty() && reco.isEmpty() && moods.isEmpty()

// -- the hero -----------------------------------------------------------------

@Composable
private fun HomeHero(state: HomeModel.State, actions: HomeActions) {
    val colors = Sillon.colors
    val hero = state.data?.hero
    val name = hero?.name.orEmpty().trim()
    val weekday = remember {
        LocalDate.now().dayOfWeek.getDisplayName(TextStyle.FULL, Locale.FRENCH)
    }
    val hello = if (name.isEmpty()) "Bonjour · $weekday" else "Bonjour, $name · $weekday"
    BoxWithConstraints(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(NzRadius.card))
            .background(colors.ac),
    ) {
        val wide = maxWidth >= 600.dp
        Canvas(Modifier.matchParentSize()) {
            // The sleeve's grooves, in the hero's own ink (DESIGN §6's rings).
            val center = Offset(size.width * 1.15f, size.height * 0.05f)
            for (radius in listOf(0.30f, 0.46f, 0.62f, 0.78f, 0.94f)) {
                drawCircle(
                    color = colors.onac.copy(alpha = 0.22f),
                    radius = size.minDimension * radius,
                    center = center,
                    style = Stroke(width = 2.dp.toPx()),
                )
            }
        }
        Column(
            Modifier.padding(if (wide) NzSpacing.xxl else NzSpacing.l),
            verticalArrangement = Arrangement.spacedBy(if (wide) NzSpacing.l else NzSpacing.m),
        ) {
            Text(
                text = hello.uppercase(Locale.FRENCH),
                style = Sillon.type.mono,
                color = colors.onac,
            )
            Text(
                text = "Quelle énergie ce matin ?",
                style = if (wide) Sillon.type.display else Sillon.type.displaySmall,
                color = colors.onac,
            )
            EnergyNotches(hero?.energySteps.orEmpty(), state.notch, actions.notch, wide)
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                HeroFlowButton(state.starting, actions.launchFlow)
                TuneChip(state, actions)
            }
            Text(
                text = "Le Flow suit ton écoute, au niveau choisi.",
                style = Sillon.type.bodySmall,
                color = colors.onac,
            )
        }
    }
}

/** The six notches, on the accent: an outline, and the chosen one filled. */
@Composable
private fun EnergyNotches(
    steps: List<uniffi.nz_ffi.EnergyStepData>,
    selected: Double?,
    onPick: (Double?) -> Unit,
    wide: Boolean,
) {
    if (steps.isEmpty()) return
    val columns = if (wide) steps.size else 3
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        steps.chunked(columns).forEach { line ->
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            ) {
                for (step in line) {
                    Notch(
                        label = step.name,
                        hint = step.hint,
                        chosen = selected == step.value,
                        onClick = { onPick(step.value) },
                        modifier = Modifier.weight(1f),
                    )
                }
                // A short last line keeps its cells the same width as the rest.
                repeat(columns - line.size) { Spacer(Modifier.weight(1f)) }
            }
        }
    }
}

@Composable
private fun Notch(
    label: String,
    hint: String,
    chosen: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.chip)
    Row(
        modifier = modifier
            .heightIn(min = NzMinTouchTarget)
            .clip(shape)
            .then(
                if (chosen) Modifier.background(colors.onac) else Modifier.border(1.5.dp, colors.onac, shape),
            )
            .clickable(role = Role.RadioButton, onClick = onClick)
            .semantics {
                selected = chosen
                role = Role.RadioButton
                contentDescription = "Énergie $label : $hint"
            }
            .padding(horizontal = NzSpacing.s),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs, Alignment.CenterHorizontally),
    ) {
        if (chosen) Icon(glyph = NzGlyph.Check, size = 13.dp, tint = colors.ac)
        Text(
            text = label,
            style = Sillon.type.label,
            color = if (chosen) colors.ac else colors.onac,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** « Lancer le Flow », on the accent: an ink pill with the accent as its word. */
@Composable
private fun HeroFlowButton(starting: Boolean, onClick: () -> Unit) {
    val colors = Sillon.colors
    Row(
        Modifier
            .clip(RoundedCornerShape(NzRadius.pill))
            .background(colors.onac)
            .clickable(enabled = !starting, role = Role.Button, onClick = onClick)
            .padding(horizontal = NzSpacing.xl, vertical = NzSpacing.m)
            .semantics { contentDescription = if (starting) "Le Flow démarre" else "Lancer le Flow" },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Icon(
            glyph = if (starting) NzGlyph.Refresh else NzGlyph.Play,
            size = 15.dp,
            tint = colors.ac,
        )
        Text(
            text = if (starting) "Démarrage…" else "Lancer le Flow",
            style = Sillon.type.label,
            color = colors.ac,
        )
    }
}

/** The discovery chip: it names the breadth before being opened. */
@Composable
private fun TuneChip(state: HomeModel.State, actions: HomeActions) {
    val colors = Sillon.colors
    Row(
        Modifier
            .sizeIn(minHeight = NzMinTouchTarget)
            .clip(RoundedCornerShape(NzRadius.chip))
            .border(1.dp, colors.onac, RoundedCornerShape(NzRadius.chip))
            .clickable(role = Role.Button) { actions.tune(!state.tuneOpen) }
            .padding(horizontal = NzSpacing.m),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        Icon(glyph = NzGlyph.Sliders, size = 13.dp, tint = colors.onac)
        Text(
            text = state.flow?.let { flowName(it) } ?: state.data?.discovery?.name.orEmpty().ifEmpty { "Découverte" },
            style = Sillon.type.label,
            color = colors.onac,
            maxLines = 1,
        )
    }
}

/** The four discovery stops, under the hero — the slider's own value set. */
@Composable
private fun TunePanel(state: HomeModel.State, actions: HomeActions) {
    val colors = Sillon.colors
    NzCard(Modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Text("Découverte", style = Sillon.type.titleSmall, color = colors.tx)
            Text(
                "Jusqu'où le Flow et « Pour toi » s'écartent de ce que tu écoutes déjà.",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                for (step in state.discoverySteps) {
                    NzChip(
                        text = step.name,
                        selected = kotlin.math.abs(state.level - step.value) < 0.13,
                        onClick = { actions.discovery(step.value) },
                        modifier = Modifier.weight(1f),
                    )
                }
            }
            val hint = state.data?.discovery?.hint.orEmpty()
            if (hint.isNotEmpty()) {
                Text(hint, style = Sillon.type.bodySmall, color = colors.dim)
            }
        }
    }
}

// -- « Reprendre » ------------------------------------------------------------

@Composable
private fun ResumePanel(data: HomeData, actions: HomeActions) {
    val colors = Sillon.colors
    NzCard(Modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Text("REPRENDRE", style = Sillon.type.labelMono, color = colors.dim)
            val resume = data.resume
            if (resume == null) {
                Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                    Text("Rien en cours de lecture.", style = Sillon.type.titleSmall, color = colors.tx)
                    Text(
                        "Choisis une énergie et lance le Flow, ou ouvre un titre depuis Explorer.",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                    NzButton(
                        text = "Explorer",
                        onClick = { actions.openSearch() },
                        kind = NzButtonKind.Ghost,
                        glyph = NzGlyph.Search,
                    )
                }
            } else {
                Row(
                    Modifier.fillMaxWidth().clickable(role = Role.Button) { actions.openPlayer() },
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    CoverImage(url = resume.cover, sizeDp = 76.dp, seed = resume.key)
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
                        Text(
                            resume.title,
                            style = Sillon.type.titleSmall,
                            color = colors.tx,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Text(
                            resume.artists,
                            style = Sillon.type.bodySmall,
                            color = colors.dim,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        if (resume.durationS > 0u) {
                            Text(
                                "${nzDuration(resume.positionS.toInt())} · ${nzDuration(resume.durationS.toInt())}",
                                style = Sillon.type.mono,
                                color = colors.dim,
                            )
                        }
                    }
                    Icon(
                        glyph = NzGlyph.Maximize,
                        size = 16.dp,
                        tint = colors.dim,
                        contentDescription = "Ouvrir le lecteur",
                    )
                }
                if (resume.durationS > 0u) {
                    val pct = (resume.positionS / resume.durationS.toDouble()).coerceIn(0.0, 1.0).toFloat()
                    Box(
                        Modifier
                            .fillMaxWidth()
                            .height(3.dp)
                            .clip(RoundedCornerShape(NzRadius.chip))
                            .background(colors.ln),
                    ) {
                        Box(
                            Modifier
                                .fillMaxWidth(pct)
                                .height(3.dp)
                                .background(colors.ac),
                        )
                    }
                }
            }
        }
    }
}

// -- « Par style » ------------------------------------------------------------

@Composable
private fun StylePicker(state: HomeModel.State, actions: HomeActions) {
    val colors = Sillon.colors
    val families = remember(state.styles) { styleFamilies(state.styles) }
    if (families.isEmpty()) return
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Text("Par style", style = Sillon.type.title, color = colors.tx)
            Text("filtre Nouveautés et Pour toi", style = Sillon.type.bodySmall, color = colors.dim)
        }
        Row(
            Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        ) {
            NzChip(
                text = "Tout",
                selected = state.chosen.isEmpty(),
                glyph = if (state.chosen.isEmpty()) NzGlyph.Check else null,
                onClick = { actions.clearStyles() },
            )
            for (family in families) {
                val on = state.chosen.contains(family.name)
                NzChip(
                    text = family.name,
                    selected = on,
                    glyph = if (on) NzGlyph.Check else null,
                    onClick = { actions.pickStyle(family.name) },
                )
            }
        }
        if (state.chosen.isNotEmpty()) {
            Row(
                Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            ) {
                Text("Flow autour de", style = Sillon.type.bodySmall, color = colors.dim)
                for (name in state.chosen) {
                    Row(
                        Modifier
                            .sizeIn(minHeight = 36.dp)
                            .clip(RoundedCornerShape(NzRadius.chip))
                            .background(colors.s2)
                            .clickable(role = Role.Button) { actions.pickStyle(name) }
                            .padding(horizontal = NzSpacing.s),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
                    ) {
                        Text(name, style = Sillon.type.labelSmall, color = colors.tx)
                        Icon(
                            glyph = NzGlyph.Close,
                            size = 11.dp,
                            tint = colors.dim,
                            contentDescription = "Retirer $name",
                        )
                    }
                }
            }
        }
    }
}

// -- « Nouveautés » -----------------------------------------------------------

@Composable
private fun ReleasesShelf(releases: uniffi.nz_ffi.ReleasesCardData, actions: HomeActions) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Row(
                Modifier.weight(1f),
                verticalAlignment = Alignment.Bottom,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Text("Nouveautés", style = Sillon.type.title, color = colors.tx)
                Text(
                    if (releases.count > 1u) "${releases.count} titres" else "${releases.count} titre",
                    style = Sillon.type.mono,
                    color = colors.dim,
                )
                if (releases.unseen > 0u) {
                    Box(
                        Modifier
                            .size(20.dp)
                            .clip(CircleShape)
                            .background(colors.ac),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            releases.unseen.toString(),
                            style = Sillon.type.labelSmall,
                            color = colors.onac,
                        )
                    }
                }
            }
            Column(
                horizontalAlignment = Alignment.End,
                verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
            ) {
                NzIconButton(
                    glyph = NzGlyph.Play,
                    contentDescription = "Tout lire",
                    onClick = { actions.playAllNew() },
                )
                NzButton(
                    text = "Tout afficher",
                    onClick = { actions.openReleases() },
                    kind = NzButtonKind.Ghost,
                )
            }
        }
        BoxWithConstraints {
            val columns = when {
                maxWidth < 440.dp -> 2
                maxWidth < 640.dp -> 3
                else -> 4
            }
            val tile = (maxWidth - NzSpacing.m * (columns - 1)) / columns
            Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
                releases.releases.chunked(columns).forEach { line ->
                    Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
                        for (card in line) {
                            ReleaseTile(
                                card = card,
                                size = tile,
                                onClick = { actions.playRelease(card.trackIndex.toInt()) },
                                modifier = Modifier.weight(1f),
                            )
                        }
                        repeat(columns - line.size) { Spacer(Modifier.weight(1f)) }
                    }
                }
            }
        }
    }
}

@Composable
private fun ReleaseTile(
    card: uniffi.nz_ffi.ReleaseCardData,
    size: Dp,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Column(
        modifier
            .clip(RoundedCornerShape(NzRadius.card))
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = "Lire ${card.title}, ${card.artist}" },
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        CoverImage(
            url = card.cover,
            sizeDp = size,
            seed = card.key,
            modifier = Modifier.width(size),
        )
        Text(
            card.title,
            style = Sillon.type.label,
            color = colors.tx,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            card.artist,
            style = Sillon.type.bodySmall,
            color = colors.dim,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

// -- « Tes artistes » ---------------------------------------------------------

@Composable
private fun ArtistsRow(data: HomeData, actions: HomeActions) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        Text("Tes artistes", style = Sillon.type.title, color = colors.tx)
        Row(
            Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            for (artist in data.artists) {
                Column(
                    Modifier
                        .width(96.dp)
                        .clip(RoundedCornerShape(NzRadius.card))
                        .clickable(role = Role.Button) { actions.openArtist(artist.id) }
                        .padding(NzSpacing.xs),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
                ) {
                    Avatar(name = artist.name, sizeDp = 76.dp, url = artist.picture)
                    Text(
                        artist.name,
                        style = Sillon.type.labelSmall,
                        color = colors.tx,
                        textAlign = TextAlign.Center,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }
}

// -- « Pour toi » -------------------------------------------------------------

@Composable
private fun ForYou(data: HomeData, state: HomeModel.State, actions: HomeActions) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text("Pour toi", style = Sillon.type.title, color = colors.tx, modifier = Modifier.weight(1f))
            NzButton(
                text = "Autres suggestions",
                onClick = { actions.moreSuggestions() },
                kind = NzButtonKind.Ghost,
                glyph = NzGlyph.Refresh,
                enabled = !state.recoBusy,
            )
        }
        Column(Modifier.fillMaxWidth()) {
            for (row in data.reco.take(RECO_ROWS)) {
                TrackRowView(
                    row = row,
                    playing = false,
                    onClick = { actions.playReco(row) },
                    onMore = { actions.more(row) },
                    onSwipeQueue = { actions.queueReco(row) },
                )
            }
        }
    }
}

// -- the Récap door -----------------------------------------------------------

@Composable
private fun RecapPanel(card: uniffi.nz_ffi.RecapCardData, actions: HomeActions) {
    val colors = Sillon.colors
    NzCard(Modifier.fillMaxWidth(), onClick = { actions.openRecap(card.year) }) {
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                Modifier
                    .size(62.dp)
                    .clip(RoundedCornerShape(14.dp))
                    .background(colors.s2)
                    .padding(3.dp),
            ) {
                Column(Modifier.fillMaxSize()) {
                    Row(Modifier.fillMaxWidth().weight(1f), horizontalArrangement = Arrangement.spacedBy(3.dp)) {
                        for (n in 0..1) GrooveTile("recap-${card.year}-$n", Modifier.weight(1f).fillMaxHeight())
                    }
                    Spacer(Modifier.height(3.dp))
                    Row(Modifier.fillMaxWidth().weight(1f), horizontalArrangement = Arrangement.spacedBy(3.dp)) {
                        for (n in 2..3) GrooveTile("recap-${card.year}-$n", Modifier.weight(1f).fillMaxHeight())
                    }
                }
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
                Text("RÉCAP", style = Sillon.type.labelMono, color = colors.dim)
                Text(
                    "Ton année ${card.year}${if (card.partial) ", en cours" else ""}",
                    style = Sillon.type.titleSmall,
                    color = colors.tx,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    "${card.listens} écoute${if (card.listens > 1u) "s" else ""} · ${card.minutes} minutes",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Icon(
                glyph = NzGlyph.ChevronRight,
                size = 16.dp,
                tint = colors.dim,
                contentDescription = "Voir mon récap ${card.year}",
            )
        }
    }
}

@Composable
private fun GrooveTile(seed: String, modifier: Modifier = Modifier) {
    // `sillon.js#grooveBackground`: the pair as a gradient, so four small tiles
    // read as a sleeve mosaic instead of four flat squares.
    val (a, b) = nzGrooveFor(seed)
    Box(
        modifier
            .clip(RoundedCornerShape(4.dp))
            .background(
                androidx.compose.ui.graphics.Brush.linearGradient(listOf(a, b)),
            ),
    )
}

// -- « Mixes d'ambiance » -----------------------------------------------------

@Composable
private fun MoodsRow(data: HomeData, state: HomeModel.State, actions: HomeActions) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Text("Mixes d'ambiance", style = Sillon.type.title, color = colors.tx)
            Text(
                if (data.moods.size > 1) "${data.moods.size} ambiances" else "${data.moods.size} ambiance",
                style = Sillon.type.mono,
                color = colors.dim,
            )
        }
        BoxWithConstraints {
            val columns = if (maxWidth < 440.dp) 2 else 3
            val tile = (maxWidth - NzSpacing.m * (columns - 1)) / columns
            Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
                data.moods.chunked(columns).forEach { line ->
                    Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
                        for (mood in line) {
                            MoodTile(
                                mood = mood,
                                size = tile,
                                busy = state.moodBusy == mood.id,
                                onClick = { actions.playMood(mood.id) },
                                modifier = Modifier.weight(1f),
                            )
                        }
                        repeat(columns - line.size) { Spacer(Modifier.weight(1f)) }
                    }
                }
            }
        }
    }
}

@Composable
private fun MoodTile(
    mood: uniffi.nz_ffi.MoodCardData,
    size: Dp,
    busy: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val label = if (busy) {
        "Préparation du mix ${mood.name}"
    } else {
        "Lancer le mix ${mood.name} : ${mood.subtitle}, ${mood.tracks} titres"
    }
    Column(
        modifier
            .clip(RoundedCornerShape(NzRadius.card))
            .clickable(role = Role.Button, onClick = { if (!busy) onClick() })
            .semantics {
                contentDescription = label
                if (busy) stateDescription = "En préparation"
            },
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        CoverImage(
            url = mood.cover,
            sizeDp = size,
            seed = mood.id,
            modifier = Modifier.width(size),
        )
        Text(
            mood.name,
            style = Sillon.type.label,
            color = colors.tx,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            mood.subtitle,
            style = Sillon.type.bodySmall,
            color = colors.dim,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                if (mood.tracks > 1u) "${mood.tracks} titres" else "${mood.tracks} titre",
                style = Sillon.type.mono,
                color = colors.dim,
                modifier = Modifier.weight(1f),
            )
            Box(
                Modifier
                    .size(28.dp)
                    .clip(CircleShape)
                    .background(if (busy) colors.s2 else colors.btn),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    glyph = if (busy) NzGlyph.Refresh else NzGlyph.Play,
                    size = 15.dp,
                    tint = if (busy) colors.dim else colors.onbtn,
                    contentDescription = null,
                )
            }
        }
    }
}

// -- the Flow's controls ------------------------------------------------------

@Composable
private fun FlowControls(state: HomeModel.State, actions: HomeActions) {
    val flow = state.flow ?: return
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        if (flow.live) {
            // The row answers to its OWN width (`FlowControls.svelte`'s container
            // query): a phone puts the two feedback buttons and the energy on a
            // second line rather than letting the steppers fall off the edge.
            BoxWithConstraints(Modifier.fillMaxWidth()) {
                val narrow = maxWidth < 400.dp
                if (narrow) {
                    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                        Row(
                            Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            FlowAimDoor(flow, actions, Modifier.weight(1f))
                            NzIconButton(
                                glyph = NzGlyph.Minus,
                                contentDescription = "Moins comme ça",
                                onClick = actions.lessLikeThis,
                            )
                            NzIconButton(
                                glyph = NzGlyph.Plus,
                                contentDescription = "Plus comme ça",
                                onClick = actions.moreLikeThis,
                            )
                        }
                        EnergyStepper(flow, actions, Modifier.fillMaxWidth())
                    }
                } else {
                    Row(
                        Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        FlowAimDoor(flow, actions, Modifier.weight(1f))
                        NzChip(text = "Moins", glyph = NzGlyph.Minus, onClick = actions.lessLikeThis)
                        NzChip(text = "Plus", glyph = NzGlyph.Plus, onClick = actions.moreLikeThis)
                        EnergyStepper(flow, actions)
                    }
                }
            }
        }
        if (flow.waiting) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(NzRadius.cover))
                    .background(colors.s2)
                    .padding(NzSpacing.m),
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Icon(glyph = NzGlyph.Sparkles, size = 16.dp, tint = colors.acText)
                Text(
                    "Le titre suivant est choisi ${PICK_AHEAD_S} s avant la fin, d'après ce que vous écoutez.",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
            }
        }
    }
    // The picker: the same body in a sheet (the SPA's FlowPicker in a dialog).
    NzSheet(visible = state.flowPanel, onDismiss = { actions.flowPanel(false) }) {
        FlowPickerBody(state, actions)
    }
}

/** The door onto the picker: it says what the Flow aims at before being opened. */
@Composable
private fun FlowAimDoor(flow: FlowStateData, actions: HomeActions, modifier: Modifier = Modifier) {
    val colors = Sillon.colors
    Row(
        modifier
            .clip(RoundedCornerShape(NzRadius.pill))
            .background(colors.s2)
            .clickable(role = Role.Button) { actions.flowPanel(true) }
            .padding(horizontal = NzSpacing.m, vertical = NzSpacing.s)
            .semantics { contentDescription = "Genres et diversité du Flow — ${flow.aim}" },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        Icon(glyph = NzGlyph.Sliders, size = 15.dp, tint = colors.tx)
        Column(Modifier.weight(1f, fill = false)) {
            Text("Genres & diversité", style = Sillon.type.label, color = colors.tx, maxLines = 1)
            Text(
                flow.aimShort,
                style = Sillon.type.bodySmall,
                color = colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (flow.styles.isNotEmpty()) {
            Box(
                Modifier.size(18.dp).clip(CircleShape).background(colors.ac),
                contentAlignment = Alignment.Center,
            ) {
                Text(flow.styles.size.toString(), style = Sillon.type.labelSmall, color = colors.onac)
            }
        }
    }
}

/** One notch down or up, and the notch's name between them. */
@Composable
private fun EnergyStepper(flow: FlowStateData, actions: HomeActions, modifier: Modifier = Modifier) {
    val colors = Sillon.colors
    Row(
        modifier,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs, Alignment.CenterHorizontally),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        NzIconButton(
            glyph = NzGlyph.ChevronDown,
            contentDescription = "Plus calme",
            onClick = { actions.nudge(-1) },
            enabled = (flow.energy ?: 0.5) > 0.1,
        )
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Text("ÉNERGIE", style = Sillon.type.labelSmall, color = colors.dim)
            Text(flow.energyName, style = Sillon.type.label, color = colors.tx, maxLines = 1)
        }
        NzIconButton(
            glyph = NzGlyph.ChevronUp,
            contentDescription = "Plus énergique",
            onClick = { actions.nudge(1) },
            enabled = (flow.energy ?: 0.5) < 0.9,
        )
    }
}

@Composable
private fun FlowPickerBody(state: HomeModel.State, actions: HomeActions) {
    val colors = Sillon.colors
    val families = remember(state.styles) { styleFamilies(state.styles) }
    Column(
        Modifier.fillMaxWidth().padding(NzSpacing.l),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Text("Orienter le Flow", style = Sillon.type.title, color = colors.tx)
        Text(
            "Les genres sur lesquels il s'appuie, et jusqu'où il peut s'en écarter.",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        Text(
            if (state.chosen.isEmpty()) "Genres · tous les styles"
            else "Genres · ${state.chosen.size} sur ${HomeModel.STYLES_MAX}",
            style = Sillon.type.labelMono,
            color = colors.dim,
        )
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            families.chunked(2).forEach { line ->
                Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                    for (family in line) {
                        val on = state.chosen.contains(family.name)
                        NzChip(
                            text = family.name,
                            selected = on,
                            glyph = if (on) NzGlyph.Check else null,
                            onClick = { actions.pickStyle(family.name) },
                            modifier = Modifier.weight(1f),
                        )
                    }
                    if (line.size == 1) Spacer(Modifier.weight(1f))
                }
            }
        }
        Text(
            "Diversité · ${state.flow?.aimShort?.substringAfter("· ") ?: ""}",
            style = Sillon.type.labelMono,
            color = colors.dim,
        )
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            state.flowSteps.chunked(3).forEach { line ->
                Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                    for (step in line) {
                        NzChip(
                            text = step.name,
                            selected = kotlin.math.abs(state.level - step.value) < 0.13,
                            onClick = { actions.discovery(step.value) },
                            modifier = Modifier.weight(1f),
                        )
                    }
                    repeat(3 - line.size) { Spacer(Modifier.weight(1f)) }
                }
            }
        }
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            NzButton(
                text = "Réinitialiser",
                onClick = { actions.resetFlow() },
                kind = NzButtonKind.Ghost,
                enabled = state.chosen.isNotEmpty() || kotlin.math.abs(state.level - HomeModel.DEFAULT_LEVEL) > 0.01,
            )
            Spacer(Modifier.weight(1f))
            NzButton(
                text = if (state.launching) "Démarrage…" else "Lancer le Flow",
                onClick = { actions.launchFlow() },
                glyph = if (state.launching) NzGlyph.Refresh else NzGlyph.Play,
                enabled = !state.launching,
            )
        }
    }
}

// -- the two ends of the page -------------------------------------------------

/** « Hors ligne · version mémorisée » (the screen standard's second row). */
@Composable
private fun StaleNote() {
    val colors = Sillon.colors
    Row(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(NzRadius.cover))
            .background(colors.s2)
            .padding(horizontal = NzSpacing.m, vertical = NzSpacing.s)
            .semantics { liveRegion = LiveRegionMode.Polite },
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(glyph = NzGlyph.CloudOff, size = 15.dp, tint = colors.dim)
        Text(
            "Hors ligne · version mémorisée",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
    }
}

@Composable
private fun EmptyHome(state: HomeModel.State, actions: HomeActions) {
    val failures = state.data?.failures ?: 0u
    if (failures >= 3u) {
        EmptyState(
            title = "L'accueil n'a pas pu se charger",
            body = "Le serveur n'a pas répondu. Ta musique reste sur l'appareil : la bibliothèque " +
                "et tes téléchargements s'ouvrent normalement.",
            actionLabel = "Réessayer",
            onAction = { actions.retry() },
            glyph = NzGlyph.Server,
        )
    } else {
        EmptyState(
            title = "Ta bibliothèque est vide pour l'instant",
            body = "Cherche un titre ou un artiste et lance la lecture. Ajoute des artistes à tes " +
                "favoris pour voir ici leurs nouveautés.",
            actionLabel = "Explorer",
            onAction = { actions.openSearch() },
            glyph = NzGlyph.Compass,
        )
    }
}

// -- small helpers ------------------------------------------------------------

/**
 * What the home is called, for the headless smoke and for a screen reader: one
 * fixed string, never a greeting. It is a CONTRACT with
 * `mobile/tools/ci-emulator.sh`, which taps the Accueil tab and then waits for
 * this node, and with every later lot that changes what the home draws: keep it
 * on the root.
 */
const val HOME_MARKER = "Accueil Neezer"

/** `flow.js#PICK_AHEAD`, in the sentence `FlowUpNext` writes. */
private const val PICK_AHEAD_S = 20

/** `Home.svelte#RECO_ROWS`: six rows, whatever the answer holds. */
private const val RECO_ROWS = 6


/** The breadth's name from the Flow's own short aim ("2 genres · Voisin"). */
private fun flowName(flow: FlowStateData): String = flow.aimShort.substringAfter("· ", flow.aimShort)
