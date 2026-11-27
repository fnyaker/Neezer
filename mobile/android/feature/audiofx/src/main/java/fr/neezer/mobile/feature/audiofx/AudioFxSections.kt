package fr.neezer.mobile.feature.audiofx

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.background
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzFader
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzSwap
import fr.neezer.mobile.designsystem.NzSwitch
import fr.neezer.mobile.designsystem.Sillon
import uniffi.nz_ffi.FxState

/** The ten sliders' centres, low to high (`nscore::dsp::EQ_FREQS`). */
private val BAND_HZ = listOf(31, 62, 125, 250, 500, 1_000, 2_000, 4_000, 8_000, 16_000)

/**
 * The audio-effects block Réglages shows (lot MB12): the output and its verdict,
 * mono and balance, the bass lift and the normalization, the ten-band equalizer
 * with its meter and its presets, and the transitions.
 *
 * One composable, called from `:feature:settings`, so the effects have a single
 * definition instead of a copy in each settings section. The behaviour is
 * `webapp/src/components/AudioEffects.svelte`'s, ported: the same cards in the
 * same order, the same French sentences, the same rules — and the meter runs only
 * while this composable is composed.
 *
 * With no core (a host without the native library), the card still draws and says
 * so: it never spins and never lies about a value it does not have.
 */
@Composable
fun AudioFxSections(modifier: Modifier = Modifier) {
    val backend = remember { bridgeAudioFx() }
    if (backend == null) {
        NzCard(modifier.fillMaxWidth()) {
            Text(
                text = "Les effets audio ne sont pas disponibles : le moteur audio de " +
                    "l'application n'a pas pu démarrer.",
                style = Sillon.type.body,
                color = Sillon.colors.dim,
            )
        }
        return
    }
    val model = remember(backend) { AudioFxModel(backend) }
    // The meter is the CARD's: it starts with the composition and is stopped and
    // joined when the card leaves the screen (0 analysis CPU afterwards).
    DisposableEffect(model) {
        model.start()
        model.openMeter()
        onDispose { model.closeMeter() }
    }
    AudioFxPanel(model, modifier)
}

/** The cards themselves, over a model — what the screenshot test renders. */
@Composable
internal fun AudioFxPanel(model: AudioFxModel, modifier: Modifier = Modifier) {
    val state = model.state
    // The page already scrolls (`:feature:settings`' pane): this block only
    // stacks its cards, or a nested scrollable would be measured with an
    // infinite height and crash.
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        // The panel's phase replaces in place (M7): the first read's line, then
        // the five cards.
        //
        // The inner `Column` is not decoration: `AnimatedContent` lays its
        // children in a BOX, so five cards given to it directly are all painted
        // at the same bounds (measured on `06-lecture-et-son`, run 45939410793:
        // five siblings at `[85,860][995,1903]`, `precheck overlap=1 targets=1`
        // — the equaliser's switch read 48 × 22 dp). Every other `NzSwap` call
        // site stacks its rows in a `Column`; this one forgot it in X6.
        NzSwap(
            targetState = if (state == null) AUDIOFX_LOADING else AUDIOFX_CONTENT,
            label = "effets-audio",
        ) { phase ->
            if (phase == AUDIOFX_LOADING) {
                // The first read is `kv` only: it lands in the frame it was asked
                // for, and a card that cannot read says why instead of spinning.
                Text(
                    text = model.failure ?: "Lecture des réglages…",
                    style = Sillon.type.body,
                    color = Sillon.colors.dim,
                )
            } else if (state != null) {
                Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
                    OutputCard(model, state)
                    PanCard(model, state)
                    AudioCard(model, state)
                    EqualizerCard(model, state)
                    TransitionsCard(model, state)
                }
            }
        }
    }
}

/** The panel's phases, as the [NzSwap] target. */
internal const val AUDIOFX_LOADING = "chargement"
internal const val AUDIOFX_CONTENT = "contenu"

// -- the cards -----------------------------------------------------------------

@Composable
private fun OutputCard(model: AudioFxModel, state: FxState) {
    FxCard(FxCopy.OUTPUT_TITLE) {
        FxChoiceRow(
            options = FxCopy.OUTPUT_MODES.map { it.first },
            selected = FxCopy.OUTPUT_MODES.indexOfFirst { it.second == state.output }.coerceAtLeast(0),
            onSelect = { index -> model.setOutput(FxCopy.OUTPUT_MODES[index].second) },
            label = "Sortie audio",
        )
        Spacer(Modifier.height(NzSpacing.s))
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                glyph = if (state.routeText.startsWith("Bluetooth")) NzGlyph.Bluetooth else NzGlyph.Speaker,
                size = 16.dp,
                tint = Sillon.colors.dim,
            )
            Column(Modifier.weight(1f)) {
                Text(state.pathText, style = Sillon.type.label, color = Sillon.colors.tx)
                if (state.whyText.isNotEmpty()) {
                    Text(
                        text = state.whyText,
                        style = Sillon.type.bodySmall,
                        color = Sillon.colors.dim,
                    )
                }
            }
        }
        val retryAt = state.retryAtMs
        if (retryAt != null) {
            Spacer(Modifier.height(NzSpacing.s))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = FxCopy.RETRY_NEXT_TRACK,
                    style = Sillon.type.bodySmall,
                    color = Sillon.colors.dim,
                    modifier = Modifier.weight(1f),
                )
                NzButton(FxCopy.retry(), { model.retryNow() }, kind = NzButtonKind.Secondary)
            }
        }
    }
}

@Composable
private fun PanCard(model: AudioFxModel, state: FxState) {
    FxCard(FxCopy.PAN_TITLE) {
        FxSwitchRow(FxCopy.MONO, state.mono, model::setMono)
        Spacer(Modifier.height(NzSpacing.s))
        // The slider carries no value of its own: the summary line below already
        // says « Stéréo · 30 % à gauche », and at a 200 % font scale the repeated
        // one was truncated to « 30 % à … » anyway (the capture showed it).
        FxSlider(
            value = (state.balance + 1f) / 2f,
            onChange = { fraction -> model.setBalance(fraction * 2f - 1f) },
            label = "Balance gauche-droite",
            valueText = "",
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(FxCopy.LEFT, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
            Spacer(Modifier.weight(1f))
            Text(FxCopy.RIGHT, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
        }
        Spacer(Modifier.height(NzSpacing.s))
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Text(
                text = "${if (state.mono) "Mono" else "Stéréo"} · ${state.panText}",
                style = Sillon.type.label,
                color = Sillon.colors.tx,
                modifier = Modifier.weight(1f),
            )
            NzChip(
                text = FxCopy.CENTRE,
                onClick = { model.resetBalance() },
                selected = state.balance == 0f,
            )
        }
    }
}

@Composable
private fun AudioCard(model: AudioFxModel, state: FxState) {
    FxCard(FxCopy.AUDIO_TITLE) {
        Text(FxCopy.NORMALIZATION, style = Sillon.type.label, color = Sillon.colors.tx)
        Spacer(Modifier.height(NzSpacing.xs))
        FxChoiceRow(
            options = FxCopy.NORM_LEVELS.map { it.first },
            selected = FxCopy.NORM_LEVELS.indexOfFirst { it.second == state.normalization }
                .coerceAtLeast(0),
            onSelect = { index -> model.setNormalization(FxCopy.NORM_LEVELS[index].second) },
            label = FxCopy.NORMALIZATION,
        )
        Spacer(Modifier.height(NzSpacing.m))
        Text(FxCopy.BASS, style = Sillon.type.label, color = Sillon.colors.tx)
        Spacer(Modifier.height(NzSpacing.xs))
        FxSlider(
            value = state.bass,
            onChange = model::setBass,
            label = FxCopy.BASS,
            valueText = FxFormat.percent(state.bass),
        )
    }
}

@Composable
private fun EqualizerCard(model: AudioFxModel, state: FxState) {
    FxCard(FxCopy.EQ_TITLE) {
        FxSwitchRow(FxCopy.EQ_SWITCH, state.eqEnabled, model::setEqEnabled)
        Spacer(Modifier.height(NzSpacing.m))

        PresetsBlock(model, state)
        Spacer(Modifier.height(NzSpacing.m))
        MeterBlock(model, state)
        Spacer(Modifier.height(NzSpacing.m))
        FadersBlock(model, state)
    }
}

@Composable
private fun PresetsBlock(model: AudioFxModel, state: FxState) {
    val builtins = state.presets.filter { it.builtin }
    val mine = state.presets.filter { !it.builtin }
    val selectedId = model.selectedId()
    FxGroupHeader(FxCopy.BUILTINS, builtins.size.toString(), onAdd = null)
    FxBandsWrap {
        builtins.forEach { preset ->
            NzChip(
                text = preset.name,
                onClick = { model.usePreset(preset) },
                selected = state.activePreset == preset.id,
            )
        }
    }
    Spacer(Modifier.height(NzSpacing.s))
    FxGroupHeader(FxCopy.MINE, mine.size.toString()) { model.openNaming() }
    if (mine.isEmpty()) {
        Text(FxCopy.NO_PRESETS, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
    } else {
        FxBandsWrap {
            mine.forEach { preset ->
                NzChip(
                    text = preset.name,
                    onClick = { model.usePreset(preset) },
                    selected = selectedId == preset.id,
                )
            }
        }
    }
    Spacer(Modifier.height(NzSpacing.xs))
    val draftName = model.draftName
    val selection = state.presets.firstOrNull { it.id == selectedId }
    if (draftName != null) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            FxTextField(
                value = draftName,
                onValueChange = model::editName,
                placeholder = FxCopy.PRESET_PLACEHOLDER,
                label = FxCopy.PRESET_NAME,
                modifier = Modifier.weight(1f),
            )
            NzButton(FxCopy.SAVE, { model.confirmSave() }, kind = NzButtonKind.Primary, enabled = draftName.isNotBlank())
            NzButton(FxCopy.CANCEL, { model.cancelNaming() }, kind = NzButtonKind.Ghost)
        }
    } else {
        val label = when {
            selection == null -> ""
            selection.builtin -> "« ${selection.name} » · intégré"
            else -> "« ${selection.name} » · à jour."
        }
        if (label.isNotEmpty()) {
            // Above the buttons, on its own line: beside them it was squeezed
            // into a two-character column (the first capture).
            Text(
                text = label,
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
                modifier = Modifier.fillMaxWidth(),
            )
        }
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (selection?.builtin == true || selection == null) {
                NzButton(FxCopy.SAVE_AS, { model.openNaming() }, kind = NzButtonKind.Secondary)
            } else {
                NzButton(FxCopy.UPDATE, { model.updateSelected() }, kind = NzButtonKind.Secondary)
                NzButton(FxCopy.DELETE, { model.deleteSelected() }, kind = NzButtonKind.Ghost)
            }
        }
    }
}

@Composable
private fun MeterBlock(model: AudioFxModel, state: FxState) {
    val frame = model.meter
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(NzGlyph.Activity, size = 14.dp, tint = Sillon.colors.dim)
        Text(FxCopy.METER_TITLE, style = Sillon.type.label, color = Sillon.colors.tx)
    }
    // The three readings get their own line: beside the title they were squeezed
    // into a 40 dp column and wrapped letter by letter (the first capture).
    when {
        frame == null || !frame.available -> Text(
            text = if (!state.eqEnabled) FxCopy.METER_IDLE else FxCopy.METER_PENDING,
            style = Sillon.type.bodySmall,
            color = Sillon.colors.dim,
        )

        else -> androidx.compose.foundation.layout.FlowRow(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            FxReading(FxCopy.METER_PEAK, FxFormat.db(frame.peakDb))
            FxReading(FxCopy.METER_HEADROOM, FxFormat.db(frame.headroomDb))
            FxReading(FxCopy.METER_LIMITER, FxFormat.db(frame.reductionDb))
        }
    }
    val warning = frame?.warning
    if (warning != null) {
        Spacer(Modifier.height(NzSpacing.s))
        FxWarningBanner(
            tone = warning.tone,
            reduction = FxFormat.db(warning.reductionDb),
            fromChange = warning.fromChange,
            cut = FxFormat.db(warning.cutDb),
            canReduce = !model.busy,
            onReduce = { model.reduceVolume(warning.cutDb) },
        )
    }
}

@Composable
private fun FadersBlock(model: AudioFxModel, state: FxState) {
    val levels = model.meter?.bands.orEmpty()
    // Ten 56 dp columns do not fit across 390 dp: the phone gets two rows of
    // five, the tablet's reading column one row of ten (the SPA's own ≤ 640 px
    // rule, and MB1's 48 dp touch target).
    androidx.compose.foundation.layout.BoxWithConstraints(Modifier.fillMaxWidth()) {
        val perRow = if (maxWidth >= 560.dp) 10 else 5
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
    BAND_HZ.chunked(perRow).forEachIndexed { rowIndex, row ->
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceEvenly,
        ) {
            row.forEachIndexed { column, hz ->
                val index = rowIndex * perRow + column
                val db = state.bands.getOrElse(index) { 0f }
                val level = levels.getOrElse(index) { 0f }
                Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    modifier = Modifier.width(56.dp),
                ) {
                    Text(FxFormat.gain(db), style = Sillon.type.bodySmall, color = Sillon.colors.tx)
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        // The measured level of the band, beside the gain: the
                        // SPA's tint behind the fader, drawn where a phone can
                        // read it.
                        FxLevelBar(level, Modifier.height(120.dp))
                        NzFader(
                            value = (db + 16f) / 32f,
                            onValueChange = { fraction ->
                                model.setBand(index, (fraction * 32f - 16f).coerceIn(-16f, 16f))
                            },
                            height = 120.dp,
                            label = "${FxFormat.frequency(hz)} Hz",
                            enabled = state.eqEnabled,
                        )
                    }
                    Text(FxFormat.frequency(hz), style = Sillon.type.bodySmall, color = Sillon.colors.dim)
                }
            }
        }
    }
            if (!state.eqEnabled) {
                Text(
                    text = "Inactif",
                    style = Sillon.type.bodySmall,
                    color = Sillon.colors.dim,
                )
            }
        }
    }
}

@Composable
private fun TransitionsCard(model: AudioFxModel, state: FxState) {
    FxCard(FxCopy.TRANSITIONS_TITLE) {
        FxSwitchRow(
            FxCopy.CROSSFADE,
            state.crossfadeEnabled,
            { on -> model.setCrossfade(on, state.crossfadeS) },
        )
        FxSlider(
            value = state.crossfadeS / 12f,
            onChange = { fraction -> model.setCrossfade(state.crossfadeEnabled, fraction * 12f) },
            label = FxCopy.CROSSFADE_DURATION,
            valueText = FxFormat.fade(state.crossfadeS),
            enabled = state.crossfadeEnabled,
        )
        Spacer(Modifier.height(NzSpacing.m))
        FxSwitchRow(
            FxCopy.SKIP_FADE,
            state.crossfadeOnSkip,
            { on -> model.setSkipFade(on, state.crossfadeSkipS) },
            enabled = state.crossfadeEnabled,
        )
        FxSlider(
            value = state.crossfadeSkipS / 5f,
            onChange = { fraction -> model.setSkipFade(state.crossfadeOnSkip, fraction * 5f) },
            label = FxCopy.SKIP_DURATION,
            valueText = FxFormat.skipFade(state.crossfadeSkipS),
            enabled = state.crossfadeEnabled && state.crossfadeOnSkip,
        )
        Spacer(Modifier.height(NzSpacing.m))
        FxSwitchRow(FxCopy.TRIM, state.trim, model::setTrim)
        Text(
            text = FxCopy.trimThreshold(state.trimDb),
            style = Sillon.type.bodySmall,
            color = Sillon.colors.dim,
        )
    }
}

// -- the pieces ----------------------------------------------------------------

/** The card's shape: a title, then the controls. */
@Composable
private fun FxCard(title: String, content: @Composable () -> Unit) {
    NzCard(Modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            Text(title, style = Sillon.type.title, color = Sillon.colors.tx)
            Spacer(Modifier.height(NzSpacing.xs))
            content()
        }
    }
}

@Composable
private fun FxGroupHeader(title: String, count: String, onAdd: (() -> Unit)?) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Text(title, style = Sillon.type.label, color = Sillon.colors.tx)
        Text(count, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
        Spacer(Modifier.weight(1f))
        if (onAdd != null) {
            NzChip(FxCopy.NEW_PRESET, onAdd, selected = false, glyph = NzGlyph.Plus)
        }
    }
}

/** Chips wrap, they never squeeze: six preset names are wider than a phone. */
@Composable
private fun FxBandsWrap(content: @Composable () -> Unit) {
    androidx.compose.foundation.layout.FlowRow(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        content()
    }
}

@Composable
private fun FxSwitchRow(
    title: String,
    checked: Boolean,
    onChange: (Boolean) -> Unit,
    enabled: Boolean = true,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Text(
            text = title,
            style = Sillon.type.label,
            color = Sillon.colors.tx,
            modifier = Modifier.weight(1f),
        )
        // The switch's own name for TalkBack, in SEMANTICS and not as a label
        // beside it: `NzSwitch` draws a label inline, and MB16 measured that a
        // label inside a row with a weighted child squeezes the row to a
        // one-letter column.
        NzSwitch(
            checked = checked,
            onCheckedChange = onChange,
            enabled = enabled,
            modifier = Modifier.semantics { contentDescription = title },
        )
    }
}

@Composable
private fun FxChoiceRow(
    options: List<String>,
    selected: Int,
    onSelect: (Int) -> Unit,
    label: String,
) {
    androidx.compose.foundation.layout.FlowRow(
        modifier = Modifier
            .fillMaxWidth()
            .semantics { contentDescription = label },
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        options.forEachIndexed { index, option ->
            NzChip(option, { onSelect(index) }, selected = index == selected)
        }
    }
}

/**
 * A horizontal slider drawn by hand: a 6 dp groove, a 20 dp thumb, the fill
 * inside the groove — the same drawing the SPA's `input[type=range]` gets
 * (`AudioEffects.svelte`), never a Material default.
 */
@Composable
private fun FxSlider(
    value: Float,
    onChange: (Float) -> Unit,
    label: String,
    valueText: String,
    enabled: Boolean = true,
) {
    val colors = Sillon.colors
    val current = rememberUpdatedState(value)
    val emit = rememberUpdatedState(onChange)
    val track = 6.dp
    val thumb = 20.dp
    // `at` is defined inside each gesture scope: a PointerInputScope IS a
    // Density, so the thumb's pixels come from the device's, not a guess.
    fun at(x: Float, width: Float, thumbPx: Float): Float {
        val span = (width - thumbPx).coerceAtLeast(1f)
        return ((x - thumbPx / 2f) / span).coerceIn(0f, 1f)
    }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .sizeIn(minHeight = 48.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Box(
            modifier = Modifier
                .weight(1f)
                .height(48.dp)
                .semantics {
                    contentDescription = label
                    progressBarRangeInfo = ProgressBarRangeInfo(value, 0f..1f)
                }
                .pointerInput(enabled) {
                    if (!enabled) return@pointerInput
                    val thumbPx = thumb.toPx()
                    detectTapGestures { offset ->
                        emit.value(at(offset.x, size.width.toFloat(), thumbPx))
                    }
                }
                .pointerInput(enabled) {
                    if (!enabled) return@pointerInput
                    val thumbPx = thumb.toPx()
                    detectHorizontalDragGestures(
                        onDragStart = { offset ->
                            emit.value(at(offset.x, size.width.toFloat(), thumbPx))
                        },
                    ) { change, _ ->
                        emit.value(at(change.position.x, size.width.toFloat(), thumbPx))
                        change.consume()
                    }
                },
        ) {
            Canvas(Modifier.fillMaxWidth().height(48.dp)) {
                val cy = size.height / 2f
                val trackPx = track.toPx()
                val thumbPx = thumb.toPx()
                val span = (size.width - thumbPx).coerceAtLeast(1f)
                val v = current.value.coerceIn(0f, 1f)
                val x = thumbPx / 2f + span * v
                val alpha = if (enabled) 1f else 0.4f
                // The rail is the HAIRLINE, not a well: `s2` on a glass card is
                // invisible, and a slider at 0 then reads as a lone white knob
                // floating on the card (the emulator capture showed exactly
                // that). The fill stays `--ac-mark`, the value's own colour.
                drawRoundRect(
                    color = colors.dim.copy(alpha = 0.30f * alpha),
                    topLeft = Offset(thumbPx / 2f, cy - trackPx / 2f),
                    size = Size(span, trackPx),
                    cornerRadius = CornerRadius(trackPx / 2f),
                )
                drawRoundRect(
                    color = colors.acMark.copy(alpha = alpha),
                    topLeft = Offset(thumbPx / 2f, cy - trackPx / 2f),
                    size = Size(span * v, trackPx),
                    cornerRadius = CornerRadius(trackPx / 2f),
                )
                drawCircle(
                    color = colors.tx.copy(alpha = alpha),
                    radius = thumbPx / 2f,
                    center = Offset(x, cy),
                )
            }
        }
        Text(
            text = valueText,
            style = Sillon.type.bodySmall,
            color = Sillon.colors.dim,
            maxLines = 1,
            softWrap = false,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.width(104.dp),
        )
    }
}

/**
 * One band's measured level, beside its fader: a 6 dp groove over −60..0 dBFS.
 *
 * The groove is the HAIRLINE, not the card's own well: on Encre, `s2` on a glass
 * card is invisible, and a fader resting at 0 shown as a lone white knob read as
 * a stray dot in the emulator capture (MB1's `NzFader` draws its own track in
 * `s2`). This bar gives the column a visible rail at every value, and the level
 * fills it from the bottom.
 */
@Composable
private fun FxLevelBar(level: Float, modifier: Modifier = Modifier) {
    val colors = Sillon.colors
    Canvas(modifier.width(6.dp)) {
        val radius = CornerRadius(size.width / 2f)
        drawRoundRect(color = colors.ln, size = size, cornerRadius = radius)
        val v = level.coerceIn(0f, 1f)
        val height = size.height * v
        drawRoundRect(
            color = colors.acMark,
            topLeft = Offset(0f, size.height - height),
            size = Size(size.width, height),
            cornerRadius = radius,
        )
    }
}

/** One number with its name, in the meter's header. */
@Composable
private fun FxReading(name: String, value: String) {
    Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(name, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
        Text(value, style = Sillon.type.bodySmall, color = Sillon.colors.tx)
    }
}

/**
 * The warning: what the equalizer is doing, and the one button that answers it.
 * The tone colours it — amber for a warning, the theme's alert red for a clip —
 * never a colour the theme does not own.
 */
@Composable
private fun FxWarningBanner(
    tone: String,
    reduction: String,
    fromChange: Boolean,
    cut: String,
    canReduce: Boolean,
    onReduce: () -> Unit,
) {
    val colors = Sillon.colors
    val clip = tone == "clip"
    // A bare block, not a card inside the card: the equalizer card already is one.
    androidx.compose.foundation.layout.Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(androidx.compose.foundation.shape.RoundedCornerShape(NzRadius.field))
            .background(colors.s2)
            .padding(NzSpacing.s),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            verticalAlignment = Alignment.Top,
        ) {
            Icon(NzGlyph.Alert, size = 18.dp, tint = if (clip) colors.ac else colors.acMark)
            Column(Modifier.weight(1f)) {
                Text(
                    text = if (clip) FxCopy.clip() else FxCopy.warn(),
                    style = Sillon.type.label,
                    color = colors.tx,
                )
                Text(
                    text = FxCopy.limiterPulls(
                        reduction,
                        if (fromChange) FxCopy.moreSinceChange() else "",
                    ),
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
            }
        }
        // The button gets its own line: beside the sentence it halved the text's
        // width and wrapped « le limiteur réduit de 6,4 dB » into three lines.
        NzButton(
            text = FxCopy.reduceVolume(cut),
            onClick = onReduce,
            kind = NzButtonKind.Secondary,
            enabled = canReduce,
        )
    }
}

/** A text field the design system does not have yet: 48 dp, Sillon's own pair. */
@Composable
private fun FxTextField(
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String,
    label: String,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    androidx.compose.material3.TextField(
        value = value,
        onValueChange = onValueChange,
        modifier = modifier
            .height(56.dp)
            .semantics { contentDescription = label },
        singleLine = true,
        placeholder = { Text(placeholder, style = Sillon.type.bodySmall) },
        textStyle = Sillon.type.body,
        colors = androidx.compose.material3.TextFieldDefaults.colors(
            focusedContainerColor = colors.s2,
            unfocusedContainerColor = colors.s2,
            focusedTextColor = colors.tx,
            unfocusedTextColor = colors.tx,
            cursorColor = colors.acMark,
            focusedIndicatorColor = colors.acMark,
            unfocusedIndicatorColor = colors.ln,
        ),
    )
}
