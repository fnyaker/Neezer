package fr.neezer.mobile.feature.podcasts

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSegmented
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import uniffi.nz_ffi.ChapterData
import uniffi.nz_ffi.ChaptersData
import uniffi.nz_ffi.EpisodeData
import uniffi.nz_ffi.EpisodeMarkerData
import uniffi.nz_ffi.TranscriptPageData
import uniffi.nz_ffi.TranscriptSegmentData

/**
 * The episode panel — the SPA's *À propos* for a podcast (`PodcastEpisodePanel`,
 * `ChapterList`, `ChapterSheet`, `ChapterTicks`, `TranscriptPanel`).
 *
 * Two segments and nothing else: **Chapitres** and **Transcription**, then the
 * description and the informations. The chapters show the chapter playing, the
 * one after it and their ticks; the transcript shows the lines with the playing
 * one highlighted and followed, page by page (200 lines, the SPA's page), a
 * search inside the episode, and — for an administrator — the server's
 * start/stop/delete.
 *
 * A transcript the device already holds costs **0 requests** while listening
 * (`transcriptcache.js`), and a search typed offline searches the pages held.
 */
@Composable
fun EpisodePanelSheet(
    episode: EpisodeData?,
    service: PodcastService?,
    chapters: ChaptersData?,
    chaptersBusy: Boolean,
    chaptersFailure: String?,
    markers: List<EpisodeMarkerData>,
    isCurrent: Boolean,
    positionS: Double,
    admin: Boolean,
    onLoadChapters: () -> Unit,
    onSeek: (Double) -> Unit,
    onResume: () -> Unit,
    onAddMarker: (Double) -> Unit,
    onRemoveMarker: (EpisodeMarkerData) -> Unit,
    onStartTranscript: () -> Unit,
    onStopTranscript: () -> Unit,
    onRestartTranscript: (String) -> Unit,
    onDeleteTranscript: () -> Unit,
    onOpenShow: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    NzSheet(visible = true, onDismiss = onDismiss) {
        if (episode == null) {
            Box(Modifier.padding(NzSpacing.xl)) {
                Text(
                    text = "Cet épisode n'est pas chargé.",
                    style = Sillon.type.body,
                    color = Sillon.colors.dim,
                )
            }
            return@NzSheet
        }
        val scope = rememberCoroutineScope()
        val model = remember(episode.id, service, scope) {
            service?.let { TranscriptModel(it, scope) }
        }
        LaunchedEffect(model, episode) { model?.open(episode.id, episode.transcript?.status) }

        EpisodePanelBody(
            episode = episode,
            model = model,
            chapters = chapters,
            chaptersBusy = chaptersBusy,
            chaptersFailure = chaptersFailure,
            markers = markers,
            isCurrent = isCurrent,
            positionS = positionS,
            admin = admin,
            onLoadChapters = onLoadChapters,
            onSeek = onSeek,
            onResume = onResume,
            onAddMarker = onAddMarker,
            onRemoveMarker = onRemoveMarker,
            onStartTranscript = onStartTranscript,
            onStopTranscript = onStopTranscript,
            onRestartTranscript = onRestartTranscript,
            onDeleteTranscript = onDeleteTranscript,
            onOpenShow = onOpenShow,
        )
    }
}

@Composable
private fun EpisodePanelBody(
    episode: EpisodeData,
    model: TranscriptModel?,
    chapters: ChaptersData?,
    chaptersBusy: Boolean,
    chaptersFailure: String?,
    markers: List<EpisodeMarkerData>,
    isCurrent: Boolean,
    positionS: Double,
    admin: Boolean,
    onLoadChapters: () -> Unit,
    onSeek: (Double) -> Unit,
    onResume: () -> Unit,
    onAddMarker: (Double) -> Unit,
    onRemoveMarker: (EpisodeMarkerData) -> Unit,
    onStartTranscript: () -> Unit,
    onStopTranscript: () -> Unit,
    onRestartTranscript: (String) -> Unit,
    onDeleteTranscript: () -> Unit,
    onOpenShow: (String) -> Unit,
) {
    val colors = Sillon.colors
    val transcriptState = model?.state?.collectAsState()?.value
    val canTranscript = admin || episode.transcript != null
    val hasChapters = episode.chapterCount > 0u || (chapters?.chapters?.isNotEmpty() == true)
    var segment = androidx.compose.runtime.remember(episode.id) {
        androidx.compose.runtime.mutableIntStateOf(if (hasChapters || !canTranscript) 0 else 1)
    }
    LaunchedEffect(episode.id, chapters?.chapters?.size) { onLoadChapters() }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .heightIn(max = 640.dp)
            .verticalScroll(rememberScrollState())
            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.l),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Text(
            text = episode.title,
            style = Sillon.type.titleSmall,
            color = colors.tx,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
        if (hasChapters && canTranscript) {
            NzSegmented(
                options = listOf("Chapitres", "Transcription"),
                selectedIndex = segment.intValue,
                onSelect = { segment.intValue = it },
                modifier = Modifier.fillMaxWidth(),
            )
        }

        if (segment.intValue == 0) {
            ChapterBlock(
                chapters = chapters,
                busy = chaptersBusy,
                failure = chaptersFailure,
                isCurrent = isCurrent,
                positionS = positionS,
                onSeek = onSeek,
                onRetry = onLoadChapters,
            )
        } else if (canTranscript) {
            TranscriptBlock(
                episode = episode,
                model = model,
                state = transcriptState,
                admin = admin,
                isCurrent = isCurrent,
                positionS = positionS,
                onSeek = onSeek,
                onStart = onStartTranscript,
                onStop = onStopTranscript,
                onRestart = onRestartTranscript,
                onDelete = onDeleteTranscript,
            )
        }

        if (markers.isNotEmpty()) {
            Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                Text(text = "Marqueurs", style = Sillon.type.label, color = colors.tx)
                markers.forEach { marker ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Row(
                            modifier = Modifier
                                .weight(1f)
                                .heightIn(min = NzMinTouchTarget)
                                .clickable(role = Role.Button) { onSeek(marker.seconds.toDouble()) },
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
                        ) {
                            Icon(glyph = NzGlyph.BookmarkFilled, size = 13.dp, tint = colors.btn)
                            Text(marker.timeLabel, style = Sillon.type.labelMono, color = colors.tx)
                            if (marker.label.isNotBlank()) {
                                Text(marker.label, style = Sillon.type.bodySmall, color = colors.dim)
                            }
                        }
                        NzGlyph3(
                            glyph = NzGlyph.Close,
                            label = "Supprimer le marqueur",
                            onClick = { onRemoveMarker(marker) },
                        )
                    }
                }
            }
        }

        if (episode.description.isNotBlank()) {
            Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                Text(text = "Description", style = Sillon.type.label, color = colors.tx)
                Text(episode.description, style = Sillon.type.bodySmall, color = colors.dim)
            }
        }

        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            Text(text = "Informations", style = Sillon.type.label, color = colors.tx)
            InfoRow("Podcast", episode.channelTitle, onClick = { onOpenShow(episode.channelId) })
            if (episode.published > 0) InfoRow("Publié le", episode.publishedLabel)
            InfoRow("Durée", episode.durationLabel)
            if (chapters != null && chapters.chapters.isNotEmpty()) {
                InfoRow("Chapitres", chapters.chapters.size.toString())
            }
            InfoRow("Archivé", if (episode.archived) "Oui" else "Non")
            if (isCurrent) {
                NzButton(
                    text = "Reprendre ici (${PodcastsCopy.duration(positionS.toInt())})",
                    onClick = { onAddMarker(positionS) },
                    kind = NzButtonKind.Ghost,
                    glyph = NzGlyph.BookmarkPlus,
                )
            } else {
                NzButton(
                    text = "Lire cet épisode",
                    onClick = onResume,
                    kind = NzButtonKind.Ghost,
                    glyph = NzGlyph.Play,
                )
            }
        }
        Spacer(Modifier.height(NzSpacing.pageY))
    }
}

@Composable
private fun InfoRow(label: String, value: String, onClick: (() -> Unit)? = null) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .then(
                if (onClick != null) {
                    Modifier
                        .heightIn(min = NzMinTouchTarget)
                        .clickable(role = Role.Button, onClick = onClick)
                } else {
                    Modifier.padding(vertical = NzSpacing.xs)
                },
            ),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = label,
            style = Sillon.type.bodySmall,
            color = colors.dim,
            modifier = Modifier.weight(1f),
        )
        Text(
            text = value,
            style = Sillon.type.body,
            color = if (onClick != null) colors.btn else colors.tx,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        if (onClick != null) {
            Icon(glyph = NzGlyph.ChevronRight, size = 14.dp, tint = colors.btn)
        }
    }
}

@Composable
private fun NzGlyph3(glyph: NzGlyph, label: String, onClick: () -> Unit) {
    Box(
        modifier = Modifier
            .size(NzMinTouchTarget)
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(glyph = glyph, size = 14.dp, tint = Sillon.colors.dim)
    }
}

/** The chapters of an episode: the playing one, the next, the list, the ticks. */
@Composable
private fun ChapterBlock(
    chapters: ChaptersData?,
    busy: Boolean,
    failure: String?,
    isCurrent: Boolean,
    positionS: Double,
    onSeek: (Double) -> Unit,
    onRetry: () -> Unit,
) {
    val colors = Sillon.colors
    when {
        busy && chapters == null -> Text(
            text = "Chargement des chapitres…",
            style = Sillon.type.bodySmall,
            color = colors.dim,
            modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
        )
        failure != null && chapters == null -> Column {
            Text(PodcastsCopy.CHAPTERS_FAILED, style = Sillon.type.bodySmall, color = colors.dim)
            NzButton(text = "Réessayer", onClick = onRetry, kind = NzButtonKind.Ghost, glyph = NzGlyph.Refresh)
        }
        chapters == null -> Text(
            text = "Chargement des chapitres…",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        !chapters.archived -> Text(
            text = PodcastsCopy.CHAPTERS_UNARCHIVED,
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        chapters.chapters.isEmpty() -> Text(
            text = PodcastsCopy.NO_CHAPTER,
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        else -> {
            val cards = chapters.chapters
            val current = if (isCurrent) chapterAt(cards, positionS) else null
            val ahead = if (isCurrent && current == null) nextChapter(cards, positionS) else null
            if (current != null || ahead != null) {
                ChapterCardView(
                    cards = cards,
                    index = (current ?: ahead)!!.first,
                    ahead = current == null,
                    remainingS = if (current == null) (cards[ahead!!.first].start.toDouble() - positionS) else 0.0,
                    isCurrent = isCurrent,
                    positionS = positionS,
                    durationS = chapters.durationS.toInt(),
                    onSeek = onSeek,
                )
            }
            ChapterList(cards = cards, currentIndex = current?.first ?: -1, onSeek = onSeek)
            ChapterTicks(
                count = cards.size,
                currentIndex = current?.first ?: -1,
                durationS = chapters.durationS.toInt(),
            )
        }
    }
}

/** The card above the list: « Chapitre 3 sur 12 », its progress, prev/next. */
@Composable
private fun ChapterCardView(
    cards: List<ChapterData>,
    index: Int,
    ahead: Boolean,
    remainingS: Double,
    isCurrent: Boolean,
    positionS: Double,
    durationS: Int,
    onSeek: (Double) -> Unit,
) {
    val colors = Sillon.colors
    val chapter = cards.getOrNull(index) ?: return
    val end = if (chapter.end != null) chapter.end!!.toInt() else {
        cards.getOrNull(index + 1)?.start?.toInt() ?: durationS
    }
    val length = (end - chapter.start.toInt()).coerceAtLeast(0)
    val elapsed = if (isCurrent && !ahead) {
        (positionS - chapter.start.toDouble()).coerceIn(0.0, length.toDouble())
    } else {
        0.0
    }
    NzCard(Modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = if (ahead) {
                        "Prochain chapitre"
                    } else if (cards.size == 1) {
                        "Chapitre 1"
                    } else {
                        "Chapitre ${index + 1} sur ${cards.size}"
                    },
                    style = Sillon.type.labelMono,
                    color = colors.dim,
                    modifier = Modifier.weight(1f),
                )
                SeekerButton(NzGlyph.Prev, "Chapitre précédent", enabled = index > 0) {
                    onSeek(cards[index - 1].start.toDouble())
                }
                SeekerButton(
                    NzGlyph.Next,
                    "Chapitre suivant",
                    enabled = index + 1 < cards.size,
                ) {
                    onSeek(cards[index + 1].start.toDouble())
                }
            }
            Text(
                text = chapter.label.ifBlank { "Chapitre ${index + 1}" },
                style = Sillon.type.label,
                color = colors.tx,
            )
            when {
                ahead -> Text(
                    text = "dans ${PodcastsCopy.duration(remainingS.toInt())}",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
                length > 0 -> {
                    Box(
                        Modifier
                            .fillMaxWidth()
                            .height(3.dp)
                            .background(colors.s2, RoundedCornerShape(2.dp)),
                    ) {
                        Box(
                            Modifier
                                .fillMaxWidth((elapsed / length).toFloat().coerceIn(0f, 1f))
                                .height(3.dp)
                                .background(colors.btn, RoundedCornerShape(2.dp)),
                        )
                    }
                    Text(
                        text = "${PodcastsCopy.duration(elapsed.toInt())} / " +
                            PodcastsCopy.duration(length),
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
            }
        }
    }
}

@Composable
private fun SeekerButton(glyph: NzGlyph, label: String, enabled: Boolean, onClick: () -> Unit) {
    Box(
        modifier = Modifier
            .size(NzMinTouchTarget)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            glyph = glyph,
            size = 18.dp,
            tint = if (enabled) Sillon.colors.tx else Sillon.colors.dim,
        )
    }
}

/** Every chapter as a row, the playing one marked (`aria-current`). */
@Composable
fun ChapterList(
    cards: List<ChapterData>,
    currentIndex: Int,
    onSeek: (Double) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Column(modifier.fillMaxWidth()) {
        cards.forEachIndexed { index, chapter ->
            val current = index == currentIndex
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 60.dp)
                    .background(
                        if (current) colors.s1 else colors.bg,
                        RoundedCornerShape(NzRadius.field),
                    )
                    .clickable(role = Role.Button) { onSeek(chapter.start.toDouble()) }
                    .semantics {
                        contentDescription =
                            "${chapter.label}, ${PodcastsCopy.duration(chapter.start.toInt())}" +
                                if (current) ", en cours" else ""
                    }
                    .padding(horizontal = NzSpacing.s),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = PodcastsCopy.duration(chapter.start.toInt()),
                    style = Sillon.type.labelMono,
                    color = if (current) colors.btn else colors.dim,
                )
                Text(
                    text = chapter.label,
                    style = Sillon.type.body,
                    color = if (current) colors.tx else colors.dim,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier
                        .weight(1f)
                        .padding(start = NzSpacing.m),
                )
                if (current) {
                    Icon(glyph = NzGlyph.Play, size = 14.dp, tint = colors.btn)
                }
            }
        }
    }
}

/**
 * The chapter ticks above a bar (`ChapterTicks`): one tick per chapter, drawn
 * only when they are readable — `(width − thumb) / N ≥ 7 px` — so 100 chapters
 * on a phone draw nothing rather than a grey smear.
 */
@Composable
fun ChapterTicks(
    count: Int,
    currentIndex: Int,
    durationS: Int,
    modifier: Modifier = Modifier,
) {
    if (count < 2 || durationS <= 0) return
    val colors = Sillon.colors
    Canvas(
        modifier = modifier
            .fillMaxWidth()
            .height(8.dp)
            .semantics { contentDescription = "$count chapitres" },
    ) {
        val step = size.width / count
        if (step < 7.dp.toPx()) return@Canvas
        for (index in 0 until count) {
            val x = step * index + step / 2
            drawRect(
                color = if (index == currentIndex) colors.btn else colors.ln,
                topLeft = Offset(x, 0f),
                size = Size(1.5.dp.toPx(), size.height),
            )
        }
    }
}

/** The chapters inside a row (4 rows on a phone, then « Voir les N chapitres »). */
@Composable
internal fun InlineChapters(
    chapters: ChaptersData?,
    busy: Boolean,
    failure: String?,
    currentSeconds: Double,
    isCurrent: Boolean,
    onSeek: (Double) -> Unit,
    onOpenAll: () -> Unit,
) {
    val colors = Sillon.colors
    when {
        busy && chapters == null -> Text(
            text = "Chargement des chapitres…",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        failure != null && chapters == null -> Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                text = if (failure.isBlank()) PodcastsCopy.CHAPTERS_FAILED else failure,
                style = Sillon.type.bodySmall,
                color = colors.dim,
                modifier = Modifier.weight(1f),
            )
            Text(
                text = "Réessayer",
                style = Sillon.type.label,
                color = colors.btn,
                modifier = Modifier
                    .heightIn(min = NzMinTouchTarget)
                    .clickable(role = Role.Button, onClick = onOpenAll),
            )
        }
        chapters == null -> Unit
        !chapters.archived -> Text(
            text = PodcastsCopy.CHAPTERS_UNARCHIVED,
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        chapters.chapters.isEmpty() -> Text(
            text = PodcastsCopy.NO_CHAPTER,
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        else -> {
            val visible = chapters.chapters.take(4)
            val current = if (isCurrent) chapterAt(chapters.chapters, currentSeconds)?.first ?: -1 else -1
            ChapterList(cards = visible, currentIndex = current, onSeek = onSeek)
            if (chapters.chapters.size > visible.size) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(min = NzMinTouchTarget)
                        .clickable(role = Role.Button, onClick = onOpenAll),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "Voir les ${chapters.chapters.size} chapitres",
                        style = Sillon.type.label,
                        color = colors.btn,
                    )
                    Icon(glyph = NzGlyph.ChevronRight, size = 15.dp, tint = colors.btn)
                }
            }
        }
    }
}

/** The index of the chapter playing at `seconds` (`chapterAt`). */
internal fun chapterAt(cards: List<ChapterData>, seconds: Double): Pair<Int, ChapterData>? {
    if (cards.isEmpty()) return null
    var found: Int? = null
    cards.forEachIndexed { index, chapter ->
        if (chapter.start.toDouble() <= seconds) found = index
    }
    val index = found ?: return null
    val chapter = cards[index]
    val end = chapter.end?.toDouble() ?: return index to chapter
    return if (seconds < end) index to chapter else null
}

/** The chapter after `seconds` (`nextChapter`). */
internal fun nextChapter(cards: List<ChapterData>, seconds: Double): Pair<Int, ChapterData>? {
    val index = cards.indexOfFirst { it.start.toDouble() > seconds }
    return if (index < 0) null else index to cards[index]
}

// -- the transcript ------------------------------------------------------------

/**
 * The transcript panel's logic: pages of 200 lines, the page that covers a
 * moment, a search inside the episode, and the local search when the link is
 * down (`TranscriptPanel.svelte` + `transcriptcache.js`, ported).
 *
 * The panel READS; the server's start/stop/delete stay the show model's
 * (they change the episode's state, which the show list draws too).
 */
class TranscriptModel(
    private val service: PodcastService,
    private val scope: CoroutineScope,
    private val pageSize: Int = 200,
) {
    data class State(
        val episodeId: String = "",
        val status: String = "none",
        val segments: List<TranscriptSegmentData> = emptyList(),
        val total: Int = 0,
        val offset: Int = 0,
        val loading: Boolean = false,
        val failure: String? = null,
        val fromCache: Boolean = false,
        val query: String = "",
        val hits: List<TranscriptSegmentData> = emptyList(),
        val searching: Boolean = false,
        val searchFailure: String? = null,
        val searchLocal: Boolean = false,
        val page: Int = 1,
        val pages: Int = 1,
        val version: Long = 0,
    ) {
        val hasText: Boolean get() = segments.isNotEmpty()
        val searching2: Boolean get() = query.trim().isNotEmpty()
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var job: Job? = null

    /** Opens an episode: the copy first (0 requests), then the page it covers. */
    fun open(episodeId: String, knownStatus: String?) {
        if (_state.value.episodeId == episodeId) return
        job?.cancel()
        _state.value = State(episodeId = episodeId, status = knownStatus ?: "none", loading = true)
        job = scope.launch {
            val cached = runCatching { service.cachedTranscript(episodeId, pageSize, 0, 0.0) }
                .getOrNull()
            if (cached != null) {
                apply(cached, cached = true)
            }
            if (_state.value.status == "queued") {
                _state.update { it.copy(loading = false, version = it.version + 1) }
                return@launch
            }
            // The copy was ALREADY applied above: asking for it twice would be
            // two disk reads for one page (`transcriptcache.js` reads it once).
            load(offset = 0, at = null, skipCache = true)
        }
    }

    /** One page, by offset or by the moment it must cover. */
    fun load(offset: Int, at: Double?, skipCache: Boolean = false) {
        val episodeId = _state.value.episodeId
        if (episodeId.isEmpty()) return
        job?.cancel()
        _state.update { it.copy(loading = true, failure = null, version = it.version + 1) }
        job = scope.launch {
            val cached = if (skipCache) {
                null
            } else {
                runCatching {
                    service.cachedTranscript(episodeId, pageSize, offset, at)
                }.getOrNull()
            }
            if (cached != null) apply(cached, cached = true)
            try {
                val page = service.transcript(episodeId, pageSize, offset, at, null)
                apply(page, cached = false)
            } catch (error: Throwable) {
                _state.update {
                    it.copy(
                        loading = false,
                        failure = if (it.hasText) null else fr.neezer.mobile.bridge.NzErrors.text(error),
                        fromCache = it.hasText,
                        version = it.version + 1,
                    )
                }
            }
        }
    }

    /** The next page of lines (`loadPage({ offset: next })`). */
    fun next() {
        val state = _state.value
        val next = state.offset + state.segments.size
        if (next >= state.total) return
        load(offset = next, at = null)
    }

    /** The previous page. */
    fun previous() {
        val state = _state.value
        val previous = (state.offset - pageSize).coerceAtLeast(0)
        load(offset = previous, at = null)
    }

    /** A search inside the episode: the server, or the pages held when offline. */
    fun search(query: String) {
        val term = query.trim()
        _state.update { it.copy(query = query, version = it.version + 1) }
        if (term.length < PodcastsModel.MIN_CHARACTERS) {
            _state.update {
                it.copy(hits = emptyList(), searchFailure = null, searching = false, version = it.version + 1)
            }
            return
        }
        val episodeId = _state.value.episodeId
        scope.launch {
            _state.update { it.copy(searching = true, searchFailure = null, version = it.version + 1) }
            try {
                val page = service.transcript(episodeId, 100, 0, null, term)
                _state.update {
                    it.copy(
                        hits = page.segments,
                        searching = false,
                        searchLocal = false,
                        searchFailure = null,
                        version = it.version + 1,
                    )
                }
            } catch (error: Throwable) {
                val local = runCatching { service.localSearch(episodeId, term, 100) }
                    .getOrDefault(emptyList())
                _state.update {
                    it.copy(
                        hits = local,
                        searching = false,
                        searchLocal = local.isNotEmpty(),
                        searchFailure = if (local.isEmpty()) {
                            fr.neezer.mobile.bridge.NzErrors.text(error)
                        } else {
                            null
                        },
                        version = it.version + 1,
                    )
                }
            }
        }
    }

    fun clearSearch() {
        _state.update {
            it.copy(query = "", hits = emptyList(), searchFailure = null, searching = false, version = it.version + 1)
        }
    }

    /** The panel's `poll`: the status only (`limit=1`). */
    fun pollStatus() {
        val episodeId = _state.value.episodeId
        if (episodeId.isEmpty()) return
        scope.launch {
            val page = runCatching {
                service.transcript(episodeId, 1, 0, null, null)
            }.getOrNull() ?: return@launch
            val hadText = _state.value.hasText
            _state.update { it.copy(status = page.status, version = it.version + 1) }
            if (page.status == "ready" && !hadText) load(offset = 0, at = null)
        }
    }

    /** The panel's 10 s poll, armed only while the job is active. */
    fun watch() {
        job = scope.launch {
            while (true) {
                val status = _state.value.status
                if (status != "queued" && status != "running" && status != "leased") break
                delay(TRANSCRIPT_POLL_MS)
                pollStatus()
            }
        }
    }

    fun stop() {
        job?.cancel()
    }

    private fun apply(page: TranscriptPageData, cached: Boolean) {
        _state.update {
            it.copy(
                status = page.status,
                segments = page.segments,
                total = page.total.toInt(),
                offset = page.offset.toInt(),
                page = page.page.toInt(),
                pages = page.pages.toInt(),
                loading = false,
                failure = null,
                fromCache = cached,
                version = it.version + 1,
            )
        }
    }

    companion object {
        /** `TranscriptPanel`'s `POLL_MS`. */
        const val TRANSCRIPT_POLL_MS = 10_000L
    }
}

/** The transcript itself: the states, the lines, the search, the pages. */
@Composable
private fun TranscriptBlock(
    episode: EpisodeData,
    model: TranscriptModel?,
    state: TranscriptModel.State?,
    admin: Boolean,
    isCurrent: Boolean,
    positionS: Double,
    onSeek: (Double) -> Unit,
    onStart: () -> Unit,
    onStop: () -> Unit,
    onRestart: (String) -> Unit,
    onDelete: () -> Unit,
) {
    val colors = Sillon.colors
    if (model == null || state == null) {
        Text(
            text = "Transcription indisponible.",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        return
    }
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        when {
            state.failure != null && !state.hasText -> {
                Text("Transcription indisponible.", style = Sillon.type.label, color = colors.tx)
                Text(state.failure, style = Sillon.type.bodySmall, color = colors.dim)
                NzButton(
                    text = "Réessayer",
                    onClick = { model.load(offset = 0, at = null) },
                    kind = NzButtonKind.Ghost,
                    glyph = NzGlyph.Refresh,
                )
            }
            state.status == "none" -> {
                if (!episode.archived) {
                    Text(
                        text = "L'épisode doit d'abord être archivé sur le serveur.",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                } else if (!admin) {
                    Text(
                        text = "Aucune transcription pour cet épisode.",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                } else {
                    Text("Transcrire cet épisode", style = Sillon.type.label, color = colors.tx)
                    Text(
                        text = PodcastsCopy.TRANSCRIPT_LOCAL,
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                    Text(
                        text = PodcastsCopy.TRANSCRIPT_SLOW,
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                    NzButton(
                        text = "Transcrire cet épisode",
                        onClick = onStart,
                        glyph = NzGlyph.Transcript,
                    )
                }
            }
            state.status == "queued" -> {
                Text("En attente de transcription…", style = Sillon.type.label, color = colors.tx)
                Text(
                    text = "La file avance ; le serveur continue si vous fermez cet écran.",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
                if (admin) {
                    NzButton(text = "Arrêter", onClick = onStop, kind = NzButtonKind.Ghost)
                }
            }
            state.status == "running" || state.status == "leased" -> {
                Text("Transcription en cours…", style = Sillon.type.label, color = colors.tx)
                if (admin) {
                    NzButton(text = "Arrêter", onClick = onStop, kind = NzButtonKind.Ghost)
                }
            }
            state.status == "error" -> {
                Text("La transcription a échoué.", style = Sillon.type.label, color = colors.tx)
                if (admin) {
                    NzButton(
                        text = "Relancer",
                        onClick = { onRestart("auto") },
                        kind = NzButtonKind.Ghost,
                        glyph = NzGlyph.Refresh,
                    )
                }
            }
        }

        if (state.hasText || state.searching2) {
            TranscriptSearchField(state, model)
            if (state.searching2) {
                when {
                    state.searching -> Text(
                        text = "Recherche…",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                        modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
                    )
                    state.hits.isEmpty() -> Text(
                        text = "Aucun passage.",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                    else -> {
                        if (state.searchLocal) {
                            Text(
                                text = "Recherche dans les pages mémorisées.",
                                style = Sillon.type.bodySmall,
                                color = colors.dim,
                            )
                        }
                        state.hits.forEach { hit ->
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .heightIn(min = NzMinTouchTarget)
                                    .clickable(role = Role.Button) { onSeek(hit.start) },
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Text(
                                    text = PodcastsCopy.duration(hit.start.toInt()),
                                    style = Sillon.type.labelMono,
                                    color = colors.btn,
                                )
                                Text(
                                    text = hit.text,
                                    style = Sillon.type.bodySmall,
                                    color = colors.tx,
                                    maxLines = 3,
                                    overflow = TextOverflow.Ellipsis,
                                    modifier = Modifier.padding(start = NzSpacing.s),
                                )
                            }
                        }
                    }
                }
            } else if (state.hasText) {
                val activeIndex = if (isCurrent) indexAt(state.segments, positionS) else -1
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(max = 360.dp)
                        .verticalScroll(rememberScrollState()),
                ) {
                    state.segments.forEachIndexed { index, segment ->
                        TranscriptLine(
                            segment = segment,
                            active = index == activeIndex,
                            onClick = { onSeek(segment.start) },
                        )
                    }
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    NzButton(
                        text = "Précédent",
                        onClick = { model.previous() },
                        kind = NzButtonKind.Ghost,
                        enabled = state.offset > 0,
                    )
                    Text(
                        text = "page ${state.page} sur ${state.pages}",
                        style = Sillon.type.labelMono,
                        color = colors.dim,
                    )
                    NzButton(
                        text = "Suivant",
                        onClick = { model.next() },
                        kind = NzButtonKind.Ghost,
                        enabled = state.offset + state.segments.size < state.total,
                    )
                }
                if (state.fromCache) {
                    Text(
                        text = PodcastsCopy.OFFLINE_COPY,
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
                if (admin) {
                    NzButton(
                        text = "Supprimer la transcription",
                        onClick = onDelete,
                        kind = NzButtonKind.Ghost,
                        glyph = NzGlyph.Trash,
                    )
                }
            }
        }
    }
}

@Composable
private fun TranscriptSearchField(state: TranscriptModel.State, model: TranscriptModel) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.chip)
    androidx.compose.foundation.text.BasicTextField(
        value = state.query,
        onValueChange = model::search,
        singleLine = true,
        textStyle = Sillon.type.body.copy(color = colors.tx),
        cursorBrush = androidx.compose.ui.graphics.SolidColor(colors.acText),
        modifier = Modifier
            .fillMaxWidth()
            .height(NzMinTouchTarget)
            .semantics { contentDescription = "Rechercher dans la transcription" },
        decorationBox = { inner ->
            Row(
                modifier = Modifier
                    .fillMaxSize()
                    .background(colors.s1, shape)
                    .padding(horizontal = NzSpacing.m),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Icon(glyph = NzGlyph.Search, size = 15.dp, tint = colors.dim)
                Box(Modifier.weight(1f)) {
                    if (state.query.isEmpty()) {
                        Text(
                            text = "Rechercher dans cet épisode…",
                            style = Sillon.type.bodySmall,
                            color = colors.dim,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    inner()
                }
                if (state.query.isNotEmpty()) {
                    NzGlyph3(NzGlyph.Close, "Effacer la recherche") { model.clearSearch() }
                }
            }
        },
    )
}

@Composable
private fun TranscriptLine(segment: TranscriptSegmentData, active: Boolean, onClick: () -> Unit) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(if (active) colors.s1 else colors.bg)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(vertical = NzSpacing.xs, horizontal = NzSpacing.xs),
        verticalAlignment = Alignment.Top,
    ) {
        Text(
            text = PodcastsCopy.duration(segment.start.toInt()),
            style = Sillon.type.labelMono,
            color = if (active) colors.btn else colors.dim,
        )
        Text(
            text = segment.text,
            style = Sillon.type.body,
            color = if (active) colors.tx else colors.dim,
            modifier = Modifier.padding(start = NzSpacing.s),
        )
    }
}

/** The index of the line playing at `seconds` (`segmentAt`). */
internal fun indexAt(segments: List<TranscriptSegmentData>, seconds: Double): Int {
    var found = -1
    segments.forEachIndexed { index, segment ->
        if (segment.start <= seconds) found = index
    }
    return found
}
