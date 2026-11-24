package fr.neezer.mobile.feature.viz

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSegmented
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzToasts
import fr.neezer.mobile.designsystem.Sillon
import kotlinx.coroutines.Dispatchers

/**
 * Visualiseur — the full-screen scene, `webapp/src/routes/Viz.svelte`'s phone
 * counterpart (lot MB21).
 *
 * The SPA's route is the PROJECTOR page: the animation full-bleed, a caption card
 * that fades in with the music, and a small bar of controls that hides itself. On
 * a phone there is one screen and the person is holding it, so the controls stay
 * where a thumb reaches and the card sits above them — the same three things, at
 * a phone's size.
 *
 * # What it does NOT have, and why
 *
 * - **The world picker and the quality tiers**: the 50 worlds are MB22's
 *   (`WorldPicker`, `lib/viz/quality.js`), and this lot's scenes are the bars and
 *   the scope, whose quality is the frame's own shape. The strip is
 *   `VizSurface`'s `setShape`.
 * - **The full-screen button**: `toggleFullscreen` exists in a browser because a
 *   browser has chrome. The app's route already is the screen; the system bars are
 *   the shell's business (MB0's `MainActivity`).
 * - **The style readout** (`style.label · BPM`): it is the smart engine's verdict,
 *   which needs the rhythm ladder — MB22's worlds. The bars and the scope draw the
 *   spectrum, and this lot runs no tempo detector.
 *
 * # The states a person can land in
 *
 * Playing: the scene, the card, the controls. Paused: the scene fades out under a
 * quiet notice (`routes/Viz.svelte`'s `.notice.quiet`). Nothing loaded: the same
 * notice, saying to start a title. Eco: no animation at all, and the screen says
 * which switch did it — never an empty box (rule 3).
 */
@Composable
fun VizEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    val scope = rememberCoroutineScope()
    val service = VizCore.service()
    val model = androidx.compose.runtime.remember(service) {
        service?.let {
            VizModel(
                service = it,
                player = NzVizPlayer,
                covers = VizCore.covers(),
                scope = scope,
                io = Dispatchers.IO,
                toast = { text -> NzToasts.show(text) },
            )
        }
    }
    if (model == null) {
        // The native library is not loaded: say so with the cause rather than a
        // black screen (rule 3, and Aide carries the same sentence).
        VizUnavailable(nav, modifier)
        return
    }
    val state by model.state.collectAsState()
    // The screen's first paint: the settings from the device's store, and the
    // track on the card. The player is read here and on a track change, never per
    // frame (the visualiser draws the sound, not the clock).
    LaunchedEffect(model) { model.refresh() }
    // L'avertissement photosensibilité, une fois par appareil : le SPA le pose
    // devant le niveau « Débridé » (`VisualizerSettings.svelte:83`), le téléphone
    // devant SON visualiseur, qui est ce qui fait clignoter l'écran.
    var ack by remember(model) { mutableStateOf<Boolean?>(null) }
    LaunchedEffect(model) { ack = service?.flashAck() != null }
    when (ack) {
        // La lecture de la clé n'a pas encore répondu : le fond, pas un écran
        // qui aurait l'air de fonctionner.
        null -> {
            Box(Modifier.fillMaxSize().background(Sillon.colors.bg))
            return
        }
        false -> if (!state.eco) {
            Box(Modifier.fillMaxSize().background(Sillon.colors.bg))
            FlashWarningSheet(
                visible = true,
                subtitle = VizCopy.FLASH_VIZ_SUBTITLE,
                body = VizCopy.FLASH_VIZ_BODY,
                // L'éco ne clignote pas et l'écran le dit déjà (`VizNotice(ECO_*)`).
                reduced = false,
                onAccept = {
                    scope.launch {
                        service?.ackFlash()
                        ack = true
                    }
                },
                // Refuser referme l'écran : il n'y a pas de visualiseur derrière.
                onDismiss = { nav.back() },
            )
            return
        }
        true -> Unit
    }
    VizScreen(state = state, model = model, modifier = modifier, service = service)
}

@Composable
internal fun VizScreen(
    state: VizModel.State,
    model: VizModel,
    modifier: Modifier,
    service: VizService?,
) {
    val colours = Sillon.colors
    // NO TOP BAR OF ITS OWN: the shell's masthead already names the route
    // (MB16's measured rule — a screen that repeats its title shows it twice, and
    // the emulator capture did). The exits are the shell's: the bottom bar and the
    // system's back.
    Column(
        modifier = modifier
            .fillMaxSize()
            .background(colours.bg),
    ) {
        Box(Modifier.weight(1f).fillMaxWidth()) {
            if (state.eco) {
                VizNotice(VizCopy.ECO_TITLE, VizCopy.ECO_BODY)
            } else {
                if (service != null) {
                    VizSurface(
                        mode = state.mode,
                        modifier = Modifier.fillMaxSize(),
                        service = service,
                        intensity = state.intensity,
                        scopeOrientation = state.orientation,
                        scopeColour = state.scopeColour,
                        fps = state.fps,
                        playing = state.playing,
                        layout = "full",
                    )
                }
                if (!state.playing) {
                    VizNotice(
                        title = if (state.loaded) VizCopy.PAUSED_TITLE else VizCopy.IDLE_TITLE,
                        body = if (state.loaded) VizCopy.PAUSED_BODY else VizCopy.IDLE_BODY,
                        quiet = true,
                    )
                }
            }
        }
        // The card is BELOW the scene and not over it: an overlaid caption hid a
        // quarter of the oscilloscope's second lane (the first capture showed the
        // trace running under the title). The SPA can overlay because its projector
        // screen is a whole laptop display; a phone's scene is small enough that the
        // caption is part of the page.
        NowPlayingCard(
            title = state.title,
            artists = state.artists,
            cover = state.cover,
            modifier = Modifier.padding(horizontal = NzSpacing.l, vertical = NzSpacing.s),
        )
        VizControls(state = state, model = model)
    }
}

/** The caption card: the sleeve, the title, the artist — `routes/Viz.svelte#card`. */
@Composable
private fun NowPlayingCard(
    title: String,
    artists: String,
    cover: String?,
    modifier: Modifier = Modifier,
) {
    if (title.isBlank()) return
    val colours = Sillon.colors
    Row(
        modifier = modifier
            .clip(RoundedCornerShape(NzRadius.card))
            .background(colours.glassBgStrong)
            .padding(NzSpacing.s),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CoverImage(url = cover, sizeDp = 48.dp)
        Spacer(Modifier.width(NzSpacing.m))
        Column {
            Text(
                text = title.ifBlank { VizCopy.CARD_UNKNOWN },
                style = Sillon.type.titleSmall,
                color = colours.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = artists,
                style = Sillon.type.bodySmall,
                color = colours.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

/**
 * The controls: the scene, the palette, and — for the scope — its two knobs, over
 * the eco switch. `FlowRow`s, because six pills are wider than a phone at a 200 %
 * font scale (MB16's measured lesson).
 */
@Composable
private fun VizControls(state: VizModel.State, model: VizModel) {
    val colours = Sillon.colors
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(colours.glassBgStrong)
            .padding(horizontal = NzSpacing.l, vertical = NzSpacing.m),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            val selected = VizCopy.MODES.indexOfFirst { it.id == state.settings?.chosenMode }
                .coerceAtLeast(0)
            // A segmented control divides its width between its labels, so at a
            // 200 % font scale « Oscilloscope » broke mid-word (« Oscillos / cope »,
            // the first capture). Above 130 % the same two choices become chips,
            // which wrap BETWEEN words — MB16's measured rule.
            if (LocalDensity.current.fontScale > 1.3f) {
                FlowRow(
                    modifier = Modifier.weight(1f),
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                ) {
                    VizCopy.MODES.forEachIndexed { index, scene ->
                        NzChip(
                            text = scene.label,
                            onClick = { model.setMode(scene.id) },
                            selected = index == selected,
                        )
                    }
                }
            } else {
                NzSegmented(
                    options = VizCopy.MODES.map { it.label },
                    selectedIndex = selected,
                    onSelect = { model.setMode(VizCopy.MODES[it].id) },
                    modifier = Modifier.weight(1f),
                )
            }
            Spacer(Modifier.width(NzSpacing.m))
            EcoToggle(
                on = state.eco,
                onToggle = { model.toggleEco() },
            )
        }
        FlowRow(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            for (palette in VizCopy.PALETTES) {
                NzChip(
                    text = palette.label,
                    onClick = { model.setPalette(palette.id) },
                    selected = state.palette == palette.id,
                )
            }
        }
        if (state.settings?.chosenMode == "scope") {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                for (choice in VizCopy.ORIENTATIONS) {
                    NzChip(
                        text = choice.label,
                        onClick = { model.setScopeOrientation(choice.id) },
                        selected = state.orientation == choice.id,
                    )
                }
                for (choice in VizCopy.SCOPE_COLOURS) {
                    NzChip(
                        text = choice.label,
                        onClick = { model.setScopeColour(choice.id) },
                        selected = state.scopeColour == choice.id,
                    )
                }
            }
        }
    }
}

@Composable
private fun VizNotice(title: String, body: String, quiet: Boolean = false) {
    val colours = Sillon.colors
    Column(
        modifier = Modifier.fillMaxSize().padding(NzSpacing.xxl),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = title,
            style = Sillon.type.title,
            color = if (quiet) colours.dim else colours.tx,
        )
        Spacer(Modifier.height(NzSpacing.s))
        Text(
            text = body,
            style = Sillon.type.body,
            color = colours.dim,
        )
    }
}

/** The native library could not be built: the cause, and nothing to retry. */
@Composable
private fun VizUnavailable(nav: NzNavigator, modifier: Modifier) {
    val colours = Sillon.colors
    Column(modifier.fillMaxSize().background(colours.bg)) {
        Box(Modifier.fillMaxSize().padding(NzSpacing.pageX), contentAlignment = Alignment.Center) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(NzGlyph.Alert, contentDescription = null, tint = colours.dim)
                Spacer(Modifier.width(NzSpacing.s))
                Text(
                    text = "Le visualiseur n'est pas disponible sur cet appareil.",
                    style = Sillon.type.body,
                    color = colours.tx,
                )
            }
        }
    }
}

/** The eco switch's own place: the player's footer cluster, and this screen's bar. */
@Composable
internal fun EcoToggle(on: Boolean, onToggle: () -> Unit, modifier: Modifier = Modifier) {
    fr.neezer.mobile.designsystem.NzIconButton(
        glyph = NzGlyph.Leaf,
        contentDescription = VizCopy.ECO_LABEL,
        onClick = onToggle,
        modifier = modifier,
        selected = on,
        tint = if (on) Sillon.colors.acText else Sillon.colors.dim,
    )
}
