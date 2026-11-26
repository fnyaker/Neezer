package fr.neezer.mobile.feature.party

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.withFrameMillis
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.compose.LifecycleResumeEffect
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzSwap
import fr.neezer.mobile.designsystem.NzToasts
import fr.neezer.mobile.designsystem.NzTopBar
import fr.neezer.mobile.designsystem.Sillon
import uniffi.nz_ffi.PartyPhase
import uniffi.nz_ffi.PartyStateData

/**
 * The GUEST's screen (lot MB20): `routes/Party.svelte`, ported.
 *
 * It renders outside the app's own routes on the page — a link someone sent. Here
 * it is the route `NavGraph` already names (`/party/<id>`), and it keeps the
 * page's five faces: the loading card, the landing (who invites you, what is
 * playing, your name, one button), the live view (the room's playhead, the
 * status line, the next track, the volume and *Décalage*), the two dead-link
 * pages and the "you left" page with « Rejoindre à nouveau ».
 *
 * **The playhead is read while the screen draws and never otherwise**: the bar
 * follows `positionNow()` (Rust's extrapolation of the room's line) through a
 * frame loop that only exists while the screen is RESUMED — a backgrounded
 * phone asks for nothing.
 */
@Composable
fun PartyGuestScreen(
    model: PartyGuestModel,
    nav: NzNavigator,
    modifier: Modifier = Modifier,
    /**
     * Whether the playhead is read while the screen draws. `false` for a
     * SCREENSHOT: a still image observes nothing, and a test that waits for idle
     * cannot with a `withFrameMillis` loop writing state every frame (MB11's own
     * measurement) — the bench then paints the position Rust last pushed.
     */
    liveReads: Boolean = true,
) {
    val state by model.state.collectAsStateWithLifecycle()
    val colors = Sillon.colors

    // A model already driven (a session that survived a rotation, or the bench)
    // is not loaded again: only an idle one asks the card.
    LaunchedEffect(model.id) {
        if (model.state.value.phase == PartyPhase.IDLE) model.load()
    }
    // The screen going away stops the PUSH subscription only: a party is the
    // player's business and outlives its screen (the note's rule).
    LifecycleResumeEffect(model) {
        onPauseOrDispose { model.close() }
    }
    LaunchedEffect(state.message) {
        state.message?.let { message ->
            NzToasts.show(message)
            model.consumeMessage()
        }
    }

    Column(modifier = modifier.fillMaxSize().background(colors.bg)) {
        NzTopBar(
            title = PartyCopy.TITLE,
            onBack = { nav.back() },
            actions = {
                if (state.live) {
                    NzIconButton(
                        glyph = NzGlyph.LogOut,
                        contentDescription = PartyCopy.QUIT,
                        onClick = { model.leave() },
                    )
                }
            },
        )
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.l),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(NzSpacing.l),
        ) {
            // The guest's states replace in place (M7): the loading card, a
            // failure, the three dead-link pages, the landing and the live view.
            NzSwap(targetState = partyGuestPhase(state), label = "soiree") { phase ->
                when (phase) {
                    PARTY_LOADING -> Loading()
                    PARTY_FAILED -> LoadFailed(
                        cause = state.failure ?: PartyCopy.MISSING,
                        onRetry = { model.retry() },
                    )
                    PARTY_MISSING -> Dead(
                        glyph = NzGlyph.Party,
                        title = "Cette soirée n'existe plus",
                        body = "Le lien a expiré, ou l'hôte l'a arrêtée. Demandez-lui un nouveau lien.",
                    )
                    PARTY_ENDED -> Dead(
                        glyph = NzGlyph.Party,
                        title = "La soirée est terminée",
                        body = "L'hôte l'a arrêtée. Merci d'avoir écouté.",
                    )
                    PARTY_LEFT -> Dead(
                        glyph = NzGlyph.Party,
                        title = "Vous avez quitté la soirée",
                        body = null,
                        action = {
                            NzButton(
                                text = PartyCopy.REJOIN,
                                onClick = { model.rejoin() },
                                glyph = NzGlyph.Party,
                            )
                        },
                    )
                    PARTY_LANDING -> Landing(model, state.data)
                    else -> Live(model, state.data, state.volume, state.trimMs, state.showTrim, liveReads)
                }
            }
        }
    }
}

/** The guest's phases, as the [NzSwap] target. */
internal const val PARTY_LOADING = "chargement"
internal const val PARTY_FAILED = "echec"
internal const val PARTY_MISSING = "absente"
internal const val PARTY_ENDED = "finie"
internal const val PARTY_LEFT = "quittee"
internal const val PARTY_LANDING = "accueil"
internal const val PARTY_LIVE = "en-direct"

internal fun partyGuestPhase(state: PartyGuestModel.State): String = when {
    state.loading && state.data == null -> PARTY_LOADING
    state.phase == PartyPhase.FAILED -> PARTY_FAILED
    state.phase == PartyPhase.MISSING -> PARTY_MISSING
    state.phase == PartyPhase.ENDED -> PARTY_ENDED
    state.phase == PartyPhase.LEFT -> PARTY_LEFT
    state.phase == PartyPhase.LANDING -> PARTY_LANDING
    else -> PARTY_LIVE
}

@Composable
private fun Loading() {
    Box(Modifier.fillMaxWidth().padding(NzSpacing.xl), contentAlignment = Alignment.Center) {
        Text(
            "Chargement…",
            style = Sillon.type.bodySmall,
            color = Sillon.colors.dim,
        )
    }
}

@Composable
private fun Dead(
    glyph: NzGlyph,
    title: String,
    body: String?,
    action: (@Composable () -> Unit)? = null,
) {
    val colors = Sillon.colors
    Column(
        modifier = Modifier.fillMaxWidth().padding(top = NzSpacing.xxl),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Box(
            modifier = Modifier.size(64.dp).clip(RoundedCornerShape(20.dp)).background(colors.ac),
            contentAlignment = Alignment.Center,
        ) {
            Icon(glyph = glyph, size = 30.dp, tint = colors.onac)
        }
        Text(
            title,
            style = Sillon.type.title,
            color = colors.tx,
            textAlign = TextAlign.Center,
        )
        if (body != null) {
            Text(
                body,
                style = Sillon.type.bodySmall,
                color = colors.dim,
                textAlign = TextAlign.Center,
            )
        }
        action?.invoke()
    }
}

/** The landing card: who invites you, what plays, your name, « Rejoindre ». */
@Composable
private fun Landing(model: PartyGuestModel, data: PartyStateData?) {
    val colors = Sillon.colors
    val state by model.state.collectAsStateWithLifecycle()
    val track = data?.track
    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Art(track?.cover, track?.id, if (state.stale) 200.dp else 240.dp)
        Text(
            data?.inviteLine ?: PartyCopy.TITLE,
            style = Sillon.type.bodySmall,
            color = colors.dim,
            textAlign = TextAlign.Center,
        )
        Text(
            data?.titleLine ?: "Rien en lecture pour l'instant",
            style = Sillon.type.title,
            color = colors.tx,
            textAlign = TextAlign.Center,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
        track?.artist?.takeIf { it.isNotBlank() }?.let { artist ->
            Text(artist, style = Sillon.type.bodySmall, color = colors.dim, textAlign = TextAlign.Center)
        }
        if (state.stale) {
            // Rule 3: the copy is shown, and it SAYS it is one.
            Text(
                "Hors ligne · version mémorisée",
                style = Sillon.type.labelSmall,
                color = colors.dim,
            )
        }
        PartyField(
            value = state.name,
            onValueChange = { model.setName(it) },
            label = PartyCopy.NAME_LABEL,
            placeholder = PartyCopy.NAME_PLACEHOLDER,
            onGo = { model.join() },
        )
        NzButton(
            text = if (state.joining) "Connexion…" else PartyCopy.JOIN,
            onClick = { model.join() },
            glyph = NzGlyph.Party,
            enabled = !state.joining,
            modifier = Modifier.fillMaxWidth(),
        )
        Text(
            PartyCopy.JOIN_HINT,
            style = Sillon.type.labelSmall,
            color = colors.dim,
            textAlign = TextAlign.Center,
        )
    }
}

/** The live view: the room's playhead, the line, the next track, the controls. */
@Composable
private fun Live(
    model: PartyGuestModel,
    data: PartyStateData?,
    volume: Double,
    trimMs: Int,
    showTrim: Boolean,
    liveReads: Boolean,
) {
    val colors = Sillon.colors
    // The playhead, read only while this composable draws: `withFrameMillis` is
    // the screen's own beat and disappears with it (no timer keeps running).
    val shown: Double = if (liveReads) {
        val hot by produceState(initialValue = model.positionNow(), model) {
            while (true) {
                withFrameMillis { value = model.positionNow() }
            }
        }
        hot
    } else {
        data?.positionS ?: 0.0
    }
    val duration = data?.durationS ?: 0.0
    val progress = if (duration > 0.0) ((shown / duration) * 100.0).coerceIn(0.0, 100.0) else 0.0

    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Art(data?.track?.cover, data?.track?.id, 220.dp)
        Text(
            data?.liveLine ?: "En direct",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        Text(
            data?.titleLine ?: "",
            style = Sillon.type.title,
            color = colors.tx,
            textAlign = TextAlign.Center,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
        data?.track?.artist?.takeIf { it.isNotBlank() }?.let { artist ->
            Text(artist, style = Sillon.type.bodySmall, color = colors.dim)
        }

        // The bar: a 4 dp track, the accent's gradient, never animated (M6: a
        // position is not a transition).
        Box(
            Modifier
                .fillMaxWidth()
                .height(5.dp)
                .clip(RoundedCornerShape(3.dp))
                .background(colors.s2),
        ) {
            Box(
                Modifier
                    .fillMaxWidth((progress / 100.0).toFloat())
                    .height(5.dp)
                    .background(colors.acMark),
            )
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text(nzClock(shown), style = Sillon.type.mono, color = colors.dim)
            Text(nzClock(duration), style = Sillon.type.mono, color = colors.dim)
        }

        // The status line: Rust's sentence, in its own tone.
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Text(
                data?.status.orEmpty(),
                style = Sillon.type.labelSmall,
                color = when (data?.tone) {
                    "ok" -> PartyTones.ok(colors.isDark)
                    "warn" -> PartyTones.amber(colors.isDark)
                    "error" -> colors.hrt
                    else -> colors.dim
                },
            )
        }
        data?.nextLine?.let { next ->
            Text(
                text = PartyCopy.NEXT.uppercase() + "  " + next,
                style = Sillon.type.labelSmall,
                color = colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }

        // Volume and the *Décalage* chip, as the page draws them.
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            Icon(glyph = NzGlyph.Volume, size = 18.dp, tint = colors.dim)
            PartyVolume(
                value = volume,
                onChange = { model.setVolume(it) },
                modifier = Modifier.weight(1f),
            )
            NzButton(
                text = if (trimMs != 0) {
                    "${PartyCopy.TRIM} ${if (trimMs > 0) "+" else ""}$trimMs ms"
                } else {
                    PartyCopy.TRIM
                },
                onClick = { model.toggleTrim() },
                kind = NzButtonKind.Ghost,
                glyph = NzGlyph.Timer,
            )
        }
        if (showTrim) TrimCard(model, data, trimMs)
        Spacer(Modifier.height(NzSpacing.s))
        NzButton(
            text = PartyCopy.QUIT,
            onClick = { model.leave() },
            kind = NzButtonKind.Secondary,
            glyph = NzGlyph.LogOut,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

@Composable
private fun TrimCard(model: PartyGuestModel, data: PartyStateData?, trimMs: Int) {
    val colors = Sillon.colors
    NzCard(modifier = Modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Text(PartyCopy.TRIM_LABEL, style = Sillon.type.label, color = colors.tx)
            Text(PartyCopy.TRIM_HELP, style = Sillon.type.labelSmall, color = colors.dim)
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
            ) {
                NzIconButton(
                    glyph = NzGlyph.Minus,
                    contentDescription = "Retarder cet appareil",
                    onClick = { model.nudge(-10) },
                )
                Text(
                    "${trimMs} ms",
                    style = Sillon.type.titleSmall,
                    color = colors.tx,
                    modifier = Modifier.width(72.dp),
                    textAlign = TextAlign.Center,
                )
                NzIconButton(
                    glyph = NzGlyph.Plus,
                    contentDescription = "Avancer cet appareil",
                    onClick = { model.nudge(10) },
                )
            }
            // What the OS itself reports, and the one shortcut the page offers
            // when it reports nothing worth counting (under 100 ms).
            data?.osLatencyMs?.let { latency ->
                Text(
                    PartyCopy.osLatency(latency),
                    style = Sillon.type.labelSmall,
                    color = colors.dim,
                )
            }
            if ((data?.osLatencyMs ?: 0L) < 100L) {
                NzButton(
                    text = PartyCopy.BLUETOOTH,
                    onClick = { model.nudge(150 - trimMs) },
                    kind = NzButtonKind.Ghost,
                )
            }
        }
    }
}

/** The sleeve, or the party's own placeholder when the host published none. */
@Composable
private fun Art(cover: String?, seed: String?, size: androidx.compose.ui.unit.Dp) {
    val colors = Sillon.colors
    if (cover != null) {
        CoverImage(url = cover, sizeDp = size, seed = seed)
    } else {
        Box(
            modifier = Modifier
                .size(size)
                .clip(RoundedCornerShape(NzRadius.card))
                .background(colors.s1),
            contentAlignment = Alignment.Center,
        ) {
            Icon(glyph = NzGlyph.Music, size = 48.dp, tint = colors.dim)
        }
    }
}

/** `m:ss`, the page's `fmtDuration` (`lib/format.js`). */
fun nzClock(seconds: Double): String {
    if (!seconds.isFinite() || seconds <= 0.0) return "0:00"
    val whole = seconds.toLong()
    val minutes = whole / 60
    val rest = whole % 60
    return "$minutes:${rest.toString().padStart(2, '0')}"
}

/**
 * The name field: a label over a Sillon pill, entered with the keyboard's « Go »
 * — `BasicTextField` and not a Material field, because a default outline is what
 * rule 5 forbids (`LibraryBits.kt#NzSearchField`'s lesson, same shape).
 */
@Composable
private fun PartyField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    placeholder: String,
    onGo: () -> Unit,
) {
    val colors = Sillon.colors
    Column(
        modifier = Modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        Text(label, style = Sillon.type.labelSmall, color = colors.dim)
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(48.dp)
                .clip(RoundedCornerShape(NzRadius.field))
                .background(colors.s1)
                .border(1.dp, colors.ln, RoundedCornerShape(NzRadius.field))
                .padding(horizontal = NzSpacing.m),
            contentAlignment = Alignment.CenterStart,
        ) {
            if (value.isEmpty()) {
                Text(placeholder, style = Sillon.type.body, color = colors.dim)
            }
            BasicTextField(
                value = value,
                onValueChange = onValueChange,
                singleLine = true,
                textStyle = Sillon.type.body.copy(color = colors.tx),
                cursorBrush = SolidColor(colors.acText),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Go),
                keyboardActions = KeyboardActions(onGo = { onGo() }),
                modifier = Modifier
                    .fillMaxWidth()
                    .semantics { contentDescription = label },
            )
        }
    }
}

/**
 * The party's volume: a horizontal track with a thumb, dragged or tapped. A
 * `Slider` is a Material widget on a Sillon surface; this is the SPA's
 * `input[type=range]` drawn with the theme's own pair.
 */
@Composable
private fun PartyVolume(
    value: Double,
    onChange: (Double) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val density = LocalDensity.current
    var widthPx by remember { mutableFloatStateOf(0f) }
    val fraction = value.coerceIn(0.0, 1.0).toFloat()
    Box(
        modifier = modifier
            .height(48.dp)
            .semantics { contentDescription = PartyCopy.VOLUME }
            .onSizeChanged { widthPx = it.width.toFloat() }
            .pointerInput(Unit) {
                detectHorizontalDragGestures { change, _ ->
                    if (widthPx > 0f) {
                        val next = (change.position.x / widthPx).coerceIn(0f, 1f)
                        onChange(next.toDouble())
                    }
                }
            },
        contentAlignment = Alignment.CenterStart,
    ) {
        Box(
            Modifier
                .fillMaxWidth()
                .height(4.dp)
                .clip(RoundedCornerShape(2.dp))
                .background(colors.s2),
        )
        Box(
            Modifier
                .fillMaxWidth(fraction)
                .height(4.dp)
                .clip(RoundedCornerShape(2.dp))
                .background(colors.acMark),
        )
        val offset = with(density) { (widthPx * fraction).toDp() - 10.dp }
        Box(
            Modifier
                .offset(x = offset.coerceAtLeast(0.dp))
                .size(20.dp)
                .clip(RoundedCornerShape(NzRadius.pill))
                .background(colors.tx),
        )
    }
}
