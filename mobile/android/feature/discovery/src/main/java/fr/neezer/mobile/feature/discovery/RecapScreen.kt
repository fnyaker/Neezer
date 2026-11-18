package fr.neezer.mobile.feature.discovery

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import kotlinx.coroutines.delay
import uniffi.nz_ffi.RecapSlideData

/** What the Récap's controls do. */
data class RecapActions(
    val retry: () -> Unit = {},
    val chooseYear: (Int) -> Unit = {},
    val tap: (Boolean) -> Unit = {},
    val hold: (Boolean) -> Unit = {},
    val restart: () -> Unit = {},
    val share: (Boolean) -> Unit = {},
    val playTop: () -> Unit = {},
    val nearby: () -> Unit = {},
    val openHistory: () -> Unit = {},
)

/**
 * Récap (lot MB14), `routes/Recap.svelte` ported.
 *
 * One card at a time: the year chips, the deck, then — on a wide screen — the
 * "en un coup d'œil" column. A phone keeps the chips above the deck and the
 * share button in the heading, exactly as the page's two breakpoints do. The
 * seven seconds per card is the screen's own `LaunchedEffect`: the model has no
 * timer, so a test drives `tap()` and nothing waits on the clock.
 */
@Composable
fun RecapScreen(
    state: RecapModel.State,
    actions: RecapActions,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val type = Sillon.type
    val data = state.data
    CompositionLocalProvider(LocalContentColor provides colors.tx) {

    // `RecapStory.svelte#arm`: seven seconds per card, suspended while a finger
    // holds, while the share sheet is open and on the last card.
    LaunchedEffect(state.index, state.holding, state.sharing, data?.year) {
        val slides = data?.slides.orEmpty()
        if (slides.isEmpty() || state.holding || state.sharing) return@LaunchedEffect
        if (state.index >= slides.lastIndex) return@LaunchedEffect
        delay(RecapModel.AUTO_MS)
        actions.tap(true)
    }

    Column(
        modifier = modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.s),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        when {
            state.noYears -> EmptyState(
                title = DiscoveryCopy.RÉCAP_NO_YEARS,
                body = "Écoute quelques titres et reviens : ton année se raconte ici.",
                glyph = NzGlyph.Music,
            )

            state.failure is RecapModel.Failure.Invalid -> EmptyState(
                title = DiscoveryCopy.RÉCAP_INVALID_TITLE,
                body = DiscoveryCopy.RÉCAP_INVALID_BODY,
                glyph = NzGlyph.Alert,
            )

            state.failure is RecapModel.Failure.Offline -> EmptyState(
                title = DiscoveryCopy.RÉCAP_OFFLINE_TITLE,
                body = "Ton récap est privé et n'est pas gardé sur l'appareil : il faut le serveur pour le lire.",
                glyph = NzGlyph.CloudOff,
                actionLabel = "Réessayer",
                onAction = actions.retry,
            )

            state.failure is RecapModel.Failure.Cause -> LoadFailed(
                cause = (state.failure as RecapModel.Failure.Cause).message,
                onRetry = actions.retry,
                title = "Récap indisponible",
            )

            data == null -> RecapSkeleton()

            data.empty -> EmptyPanel(state = state, actions = actions, data = data)

            else -> {
                if (state.chips.isNotEmpty()) {
                    YearChips(state, actions)
                }
                Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                    if (state.loading) {
                        Text("Chargement…", style = type.label, color = colors.dim)
                    }
                }
                val slide = state.slide
                if (slide != null) {
                    SlideCard(
                        slide = slide,
                        index = state.index,
                        total = data.slides.size,
                        onTap = actions.tap,
                        onHold = actions.hold,
                    )
                }
                Glance(data)
                NzButton(
                    text = "Partager",
                    glyph = NzGlyph.Share,
                    kind = NzButtonKind.Ghost,
                    onClick = { actions.share(true) },
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
        Spacer(Modifier.height(NzSpacing.xxl))
    }

    if (state.sharing && data != null) {
        RecapShareSheet(data = data, onClose = { actions.share(false) })
    }
    }
}

/** The year selector: one chip per year that holds an event (`yearChips`). */
@Composable
private fun YearChips(state: RecapModel.State, actions: RecapActions) {
    LazyRow(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        items(state.chips, key = { it.year }) { chip ->
            NzChip(
                text = chip.label,
                selected = chip.year == state.activeYear,
                onClick = { actions.chooseYear(chip.year) },
            )
        }
    }
}

/**
 * One card: the kicker, its glyph, the title, then the layout its id names.
 * Tapping the right or the left half turns the card; holding pauses the story
 * (`RecapStory.svelte`'s two zones).
 */
@Composable
private fun SlideCard(
    slide: RecapSlideData,
    index: Int,
    total: Int,
    onTap: (Boolean) -> Unit,
    onHold: (Boolean) -> Unit,
) {
    val colors = Sillon.colors
    val type = Sillon.type
    var width by remember { mutableStateOf(0) }
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 420.dp)
            .clip(RoundedCornerShape(NzRadius.card))
            .background(colors.s1)
            .pointerInput(index) {
                detectTapGestures(
                    onPress = {
                        onHold(true)
                        tryAwaitRelease()
                        onHold(false)
                    },
                    onTap = { offset ->
                        onTap(offset.x >= size.width / 2f)
                    },
                )
            }
            .semantics {
                contentDescription = "${index + 1} sur $total. ${slide.title}"
            },
    ) {
        Column(
            modifier = Modifier.fillMaxSize().padding(NzSpacing.l),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(slide.kicker, style = type.labelMono, color = colors.dim)
                Spacer(Modifier.weight(1f))
                Icon(glyphFor(slide.glyph), size = 17.dp, tint = colors.dim)
            }
            Text(
                slide.title,
                style = if (slide.id == "minutes") type.display else type.displaySmall,
                modifier = Modifier.semantics { heading() },
            )
            when (slide.id) {
                "year" -> {
                    if (slide.lede.isNotEmpty()) {
                        Text(slide.lede, style = type.body, color = colors.dim)
                    }
                    if (slide.mosaic.isNotEmpty()) {
                        Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
                            slide.mosaic.take(4).forEachIndexed { at, cover ->
                                CoverImage(url = cover, sizeDp = if (slide.mosaic.size == 1) 120.dp else 84.dp, seed = "recap-$at")
                            }
                        }
                    }
                    Text(slide.footnote, style = type.label, color = colors.dim)
                }

                "minutes" -> {
                    Text(slide.lede, style = type.body, color = colors.dim)
                    MonthBars(slide)
                    Text(slide.text, style = type.labelMono, color = colors.dim)
                    if (slide.footnote.isNotEmpty()) {
                        Text(slide.footnote, style = type.body)
                    }
                }

                "genres" -> {
                    Text(slide.lede, style = type.body, color = colors.dim)
                    Text(slide.text, style = type.label, color = colors.dim)
                    RankRows(slide, bar = true)
                }

                "streak" -> {
                    if (slide.text.isNotEmpty()) Text(slide.text, style = type.body)
                    if (slide.footnote.isNotEmpty()) Text(slide.footnote, style = type.body, color = colors.dim)
                }

                "firstlast" -> RankRows(slide, bar = false, labelled = true)

                "share" -> {
                    PosterPreview(slide.poster)
                }

                else -> RankRows(slide, bar = false)
            }
            Spacer(Modifier.weight(1f))
            // The progress bar: the card's place, drawn not announced.
            Row(horizontalArrangement = Arrangement.spacedBy(3.dp)) {
                repeat(total) { at ->
                    Box(
                        Modifier
                            .weight(1f)
                            .height(3.dp)
                            .clip(RoundedCornerShape(999.dp))
                            .background(if (at <= index) colors.acText else colors.s2),
                    )
                }
            }
        }
    }
}

/**
 * `RecapSlide.svelte#monthBars`: twelve bars normalised to the busiest (100 %),
 * in a chart area of its own so the labels always sit UNDER the bars — a first
 * capture drew them inside the columns.
 */
@Composable
private fun MonthBars(slide: RecapSlideData) {
    val colors = Sillon.colors
    val type = Sillon.type
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
        Row(
            modifier = Modifier.fillMaxWidth().height(84.dp),
            verticalAlignment = Alignment.Bottom,
            horizontalArrangement = Arrangement.spacedBy(3.dp),
        ) {
            slide.bars.forEach { bar ->
                val fill = if (bar.pct.toInt() > 0) maxOf(4, bar.pct.toInt()) else 2
                Box(
                    Modifier
                        .weight(1f)
                        .height(84.dp * (fill / 100f))
                        .background(colors.acText),
                )
            }
        }
        // No per-bar label: the SPA's chart names the busiest month in words
        // (`chart-foot`), and "juin"/"juil." both cut to "jui" at 360 dp.
    }
}

/** The ranks of a top: the sleeve, the name, the count; a share bar for styles. */
@Composable
private fun RankRows(slide: RecapSlideData, bar: Boolean, labelled: Boolean = false) {
    val colors = Sillon.colors
    val type = Sillon.type
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        slide.rows.forEach { row ->
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                Text("${row.rank}", style = type.mono, color = colors.dim, modifier = Modifier.width(16.dp))
                if (!bar) {
                    CoverImage(url = row.cover, sizeDp = 40.dp, seed = row.key)
                }
                Column(Modifier.weight(1f)) {
                    if (labelled && row.rank == 2u) {
                        Text("Ton dernier titre", style = type.labelMono, color = colors.dim)
                    }
                    Text(row.title, style = type.body, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    if (row.subtitle.isNotEmpty()) {
                        Text(row.subtitle, style = type.label, color = colors.dim, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    if (bar) {
                        Box(
                            Modifier
                                .fillMaxWidth()
                                .padding(top = 4.dp)
                                .height(3.dp)
                                .clip(RoundedCornerShape(999.dp))
                                .background(colors.s2),
                        ) {
                            Box(
                                Modifier
                                    .fillMaxWidth((row.share.coerceIn(0.0, 1.0)).toFloat())
                                    .height(3.dp)
                                    .background(colors.acText),
                            )
                        }
                    }
                }
                Text(row.value, style = type.mono, color = colors.dim, maxLines = 1)
            }
        }
    }
}

/** The share card's poster, as the SPA previews it (`share-preview`). */
@Composable
private fun PosterPreview(lines: List<String>) {
    val colors = Sillon.colors
    val type = Sillon.type
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(NzRadius.cover))
            .background(colors.bg)
            .padding(NzSpacing.l),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        Text("Récap", style = type.labelMono, color = colors.dim)
        lines.forEachIndexed { at, line ->
            Text(
                line,
                style = if (at == 1) type.title else type.body,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Text("neezer", style = type.labelMono, color = colors.dim)
    }
}

/** "En un coup d'œil": minutes, listens, the artist, the style, the big day. */
@Composable
private fun Glance(data: RecapDeck) {
    val colors = Sillon.colors
    val type = Sillon.type
    NzCard(modifier = Modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            Text("EN UN COUP D'ŒIL", style = type.labelMono, color = colors.dim)
            GlanceRow("Minutes", data.minutesLabel)
            GlanceRow("Écoutes", data.listensLabel)
            GlanceRow("Artiste", data.topArtist.ifEmpty { "—" })
            GlanceRow("Style", data.topGenre.ifEmpty { "—" })
            GlanceRow("Plus grosse journée", data.biggestDay.ifEmpty { "—" })
        }
    }
}

@Composable
private fun GlanceRow(label: String, value: String) {
    val colors = Sillon.colors
    val type = Sillon.type
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, style = type.bodySmall, color = colors.dim)
        Spacer(Modifier.weight(1f))
        Text(value, style = type.label, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** The year with no story: the SPA's own three sentences, with "Voir 2025". */
@Composable
private fun EmptyPanel(state: RecapModel.State, actions: RecapActions, data: RecapDeck) {
    EmptyState(
        title = data.emptyTitle,
        body = data.emptyText,
        glyph = when (data.emptyReason) {
            "skips-only" -> NzGlyph.Next
            "thin" -> NzGlyph.Activity
            else -> NzGlyph.Timer
        },
        actionLabel = data.nearbyYear?.let { "Voir $it" },
        onAction = if (data.nearbyYear != null) actions.nearby else null,
    )
    if (state.chips.isNotEmpty()) {
        YearChips(state, actions)
    }
}

@Composable
private fun RecapSkeleton() {
    val colors = Sillon.colors
    Box(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 420.dp)
            .clip(RoundedCornerShape(NzRadius.card))
            .background(colors.s1),
    ) {
        Text(
            "Chargement de ton récap…",
            style = Sillon.type.body,
            color = colors.dim,
            textAlign = TextAlign.Center,
            modifier = Modifier.align(Alignment.Center),
        )
    }
}

/** `RecapSlide.svelte#ICONS`' names, as the design system's glyphs. */
private fun glyphFor(name: String): NzGlyph = when (name) {
    "sparkles" -> NzGlyph.Sparkles
    "timer" -> NzGlyph.Timer
    "user" -> NzGlyph.User
    "disc" -> NzGlyph.Disc
    "tag" -> NzGlyph.Tag
    "activity" -> NzGlyph.Activity
    "share" -> NzGlyph.Share
    else -> NzGlyph.Music
}
