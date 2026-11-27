package fr.neezer.mobile.feature.settings

import android.content.Intent
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
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
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.DevicePrompts
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSegmented
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzSwap
import fr.neezer.mobile.designsystem.NzSwitch
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.NzToasts
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.TrackRowView
import fr.neezer.mobile.designsystem.nzAccent
import fr.neezer.mobile.designsystem.rememberListWindow
import fr.neezer.mobile.feature.audiofx.AudioFxSections
import kotlinx.coroutines.launch

/**
 * The sections only this device's settings feed: Lecture & son (MB12's own
 * composable, hosted here), Apparence, Animations, Téléchargements, Découverte
 * and Assistance.
 *
 * The copy and the controls are `routes/Settings.svelte`'s, ported: the same
 * names, the same order, the same sentences, and a switch where the SPA has one.
 * Nothing here asks the server: every tap writes at once through
 * [SettingsModel]'s optimistic local write, and the shell repaints.
 */

/** « Lecture & son »: MB12 draws the effects; the quality is Réglages' own. */
@Composable
internal fun PlaybackSection(model: SettingsModel, state: SettingsModel.State) {
    val settings = state.settings ?: return
    SettingsCard(
        title = "Qualité demandée au serveur",
    ) {
        ChoicePills(
            choices = QUALITY_CHOICES.map { it.second },
            selected = QUALITY_CHOICES.indexOfFirst { it.first == settings.quality }.coerceAtLeast(0),
            onSelect = { index -> model.setPlaybackQuality(QUALITY_CHOICES[index].first) },
        )
    }
    // MB12's own module, its own ten faders and its meter: hosted, never copied.
    AudioFxSections()

    // Le mode voiture n'avait AUCUNE porte depuis Réglages sur le téléphone : il
    // fallait la tuile des réglages rapides ou le panneau lui-même (audit §9).
    // `CarModeService.openSettings` est la seule porte — elle masque le panneau
    // avant d'ouvrir l'écran — et c'est le moment de demander les notifications,
    // puisque c'est là que le mode se règle (M13).
    val ctx = LocalContext.current
    val notifLauncher = androidx.activity.compose.rememberLauncherForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.RequestPermission(),
    ) { granted ->
        DevicePrompts.noteNotificationsAnswered(ctx)
        if (!granted) NzToasts.show(DevicePrompts.NOTIFICATIONS_LOST)
    }
    SettingsCard(
        title = "Mode voiture",
    ) {
        NzButton(
            text = "Ouvrir les réglages du mode voiture",
            onClick = {
                if (DevicePrompts.shouldAskNotifications(ctx)) {
                    notifLauncher.launch(android.Manifest.permission.POST_NOTIFICATIONS)
                }
                fr.neezer.mobile.car.CarModeService.openSettings(ctx)
            },
            glyph = NzGlyph.Monitor,
            kind = NzButtonKind.Secondary,
        )
    }
}

/** « Apparence » : the three themes, the person's accent, Pochette's sleeve. */
@Composable
internal fun AppearanceSection(model: SettingsModel, state: SettingsModel.State) {
    val settings = state.settings ?: return
    SettingsCard(
        title = "Ambiance",
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            THEME_CHOICES.forEach { (id, label) ->
                ThemeCard(
                    id = id,
                    label = label,
                    selected = settings.theme == id,
                    onClick = { model.setTheme(id) },
                )
            }
        }
    }
    AccentCard(model, settings)
}

/**
 * One theme, drawn as itself: its ground, its card and its accent, with a check
 * when it is the one in force (`AppearanceSettings.svelte`'s swatch).
 */
@Composable
private fun ThemeCard(id: String, label: String, selected: Boolean, onClick: () -> Unit) {
    val colors = Sillon.colors
    val preview = Sillon.theme(themeKindOf(id))
    val shape = RoundedCornerShape(NzRadius.field)
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = NzMinTouchTarget + 20.dp)
            .background(if (selected) colors.s2 else colors.s1, shape)
            .border(if (selected) 2.dp else 1.dp, if (selected) colors.btn else colors.ln, shape)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(NzSpacing.s)
            .semantics { contentDescription = "$label${if (selected) ", choisi" else ""}" },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Box(
            modifier = Modifier
                .size(56.dp)
                .background(preview.bg, RoundedCornerShape(NzRadius.field))
                .border(1.dp, preview.ln, RoundedCornerShape(NzRadius.field)),
            contentAlignment = Alignment.Center,
        ) {
            Box(
                Modifier
                    .width(34.dp)
                    .height(20.dp)
                    .background(preview.s1, RoundedCornerShape(4.dp)),
            ) {
                Box(
                    Modifier
                        .padding(start = 5.dp, top = 7.dp)
                        .width(16.dp)
                        .height(3.dp)
                        .background(preview.tx.copy(alpha = 0.7f), RoundedCornerShape(2.dp)),
                )
            }
            Box(
                Modifier
                    .align(Alignment.BottomEnd)
                    .padding(6.dp)
                    .size(10.dp)
                    .background(preview.ac, CircleShape),
            )
        }
        Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
            Text(label, style = Sillon.type.label, color = colors.tx)
            if (selected) {
                Spacer(Modifier.width(NzSpacing.s))
                Icon(NzGlyph.Check, size = 13.dp, tint = colors.acText)
                Text("Choisi", style = Sillon.type.labelMono, color = colors.acText)
            }
        }
    }
}

/** `AppearanceSettings.svelte`'s custom colour: the field, the correction, the preview. */
@Composable
private fun AccentCard(model: SettingsModel, settings: uniffi.nz_ffi.SettingsData) {
    val colors = Sillon.colors
    val theme = themeKindOf(settings.theme)
    val stored = settings.accent
    // On Pochette the colour is dormant, so the preview shows the theme it WILL
    // apply to rather than pretending it is on (the SPA does the same).
    val previewKind = if (theme == NzThemeKind.Pochette) NzThemeKind.Encre else theme
    val pair = nzAccent(colourOf(stored ?: themeAccentOf(previewKind)) ?: colors.ac, previewKind)
    var hex by remember(stored) { mutableStateOf(stored?.removePrefix("#") ?: "") }
    val active = stored != null && theme != NzThemeKind.Pochette

    SettingsCard(
        title = "Couleur personnalisée",
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Row(
                modifier = Modifier
                    .weight(1f)
                    .height(NzMinTouchTarget)
                    .background(colors.s2, RoundedCornerShape(NzRadius.field))
                    .border(1.dp, colors.ln, RoundedCornerShape(NzRadius.field))
                    .padding(horizontal = NzSpacing.m),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("#", style = Sillon.type.labelMono, color = colors.dim)
                Spacer(Modifier.width(NzSpacing.xs))
                BasicTextField(
                    value = hex,
                    onValueChange = { typed ->
                        val cleaned = typed.removePrefix("#").uppercase().take(6)
                        hex = cleaned
                        // Only a COMPLETE colour applies; anything shorter is not a
                        // colour yet, not an error to shout about while typing.
                        if (cleaned.length == 6) model.setAccent("#$cleaned")
                    },
                    singleLine = true,
                    textStyle = Sillon.type.body.copy(color = colors.tx),
                    cursorBrush = SolidColor(colors.acText),
                    modifier = Modifier
                        .weight(1f)
                        .semantics { contentDescription = "Code hexadécimal de la couleur, sans le dièse" },
                )
            }
            NzButton(
                text = "Réinitialiser",
                onClick = {
                    hex = ""
                    model.setAccent(null)
                },
                kind = NzButtonKind.Secondary,
                glyph = NzGlyph.Refresh,
                enabled = stored != null,
            )
        }
        val correction = when {
            pair.adjusted && previewKind == NzThemeKind.Papier ->
                "Trop claire pour Papier : les textes et icônes d'accent prennent une variante assombrie (4,5:1). Le bouton garde votre couleur, son libellé reste noir ou blanc."
            pair.adjusted ->
                "Trop sombre pour Encre : les textes et icônes d'accent prennent une variante éclaircie (4,5:1). Le bouton garde votre couleur, son libellé reste noir ou blanc."
            stored == null -> "Aucune couleur personnalisée : le thème garde son accent."
            active -> "Contraste vérifié : le texte tient 4,5:1 sur cette couleur."
            else -> "Mémorisée pour Encre et Papier ; Pochette garde la couleur de la pochette."
        }
        Text(text = correction, style = Sillon.type.bodySmall, color = colors.dim)
        // The pair as it will be painted: surface, its own text, and the ink the
        // heart and the marks use. A real rendering is what makes the correction
        // visible (`AppearanceSettings`'s `.preview`).
        Row(
            modifier = Modifier.fillMaxWidth().heightIn(min = NzMinTouchTarget),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            // The button as it will be painted. WITHOUT a custom colour that is
            // the theme's own pair — on Papier a lemon « Lire » would be the
            // highlighter MB1 forbids (`--btn` is ink there); WITH one it is the
            // accent itself, which is exactly what the SPA paints.
            val buttonSurface = if (stored != null && theme != NzThemeKind.Pochette) pair.surface else colors.btn
            val buttonInk = if (stored != null && theme != NzThemeKind.Pochette) pair.on else colors.onbtn
            Row(
                modifier = Modifier
                    .height(NzMinTouchTarget)
                    .background(buttonSurface, RoundedCornerShape(NzRadius.pill))
                    .padding(horizontal = NzSpacing.l),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("Lire", style = Sillon.type.label, color = buttonInk)
            }
            // The ink variant, on the CARD (where a heart and the marks are
            // drawn): a lemon heart on the lemon pill would be invisible, which
            // this lot's first capture showed.
            Icon(NzGlyph.HeartFilled, size = 20.dp, tint = pair.ink)
            Text(text = "#${colourHex(pair.surface)}", style = Sillon.type.labelMono, color = colors.dim)
        }
    }
}

/** `lib/theme.js#themeAccentOf`: the accent a theme paints before a choice. */
private fun themeAccentOf(kind: NzThemeKind): String = when (kind) {
    NzThemeKind.Encre -> "#D2FF3C"
    NzThemeKind.Papier -> "#14130F"
    NzThemeKind.Pochette -> "#FF6B4A"
}

/** `#RRGGBB` of a colour, for the preview's own label. */
private fun colourHex(color: Color): String {
    val value = (color.value shr 32).toInt() and 0xFFFFFF
    return String.format("%06X", value)
}

/**
 * « Animations » : the eco switch, the knobs of the visualiser, and MB21's door to
 * the full-screen scene.
 *
 * The door is the SPA's own (`VisualizerSettings.svelte#openScreen` →
 * `openProjector()`): the settings choose what the scene draws, and one button
 * opens it. `onOpen` is null in a screenshot test, which then shows the card
 * without a control that could do nothing.
 */
@Composable
internal fun VizSection(
    model: SettingsModel,
    state: SettingsModel.State,
    onOpen: (() -> Unit)? = null,
    isAdmin: Boolean = false,
    onGenres: (() -> Unit)? = null,
) {
    val settings = state.settings ?: return
    if (onOpen != null) {
        SettingsCard(
            title = "Écran d'animation",
        ) {
            NzButton(
                text = "Ouvrir le visualiseur",
                onClick = onOpen,
                glyph = NzGlyph.Maximize,
                kind = NzButtonKind.Secondary,
            )
        }
    }
    SettingsCard(
        title = "Mode éco",
    ) {
        NzSwitch(checked = settings.eco, onCheckedChange = model::setEco, label = "Mode éco")
    }
    SettingsCard(title = "Visualiseur") {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
            Text("Forme", style = Sillon.type.label, color = Sillon.colors.tx)
            NzSegmented(
                options = listOf("Barres", "Oscilloscope"),
                selectedIndex = if (settings.vizMode == "scope") 1 else 0,
                onSelect = { model.setVizMode(if (it == 1) "scope" else "bars") },
            )
            Text("Palette", style = Sillon.type.label, color = Sillon.colors.tx)
            ChoicePills(
                choices = listOf("Pochette", "Duo", "Monochrome"),
                selected = when (settings.vizPalette) {
                    "duo" -> 1
                    "mono" -> 2
                    else -> 0
                },
                onSelect = { model.setVizPalette(listOf("cover", "duo", "mono")[it]) },
            )
            Text("Intensité", style = Sillon.type.label, color = Sillon.colors.tx)
            NzLevelSlider(
                value = settings.vizIntensity.toFloat(),
                onValueChange = { model.setVizIntensity(it.toDouble()) },
                label = "Intensité du visualiseur",
            )
            Text("Images par seconde", style = Sillon.type.label, color = Sillon.colors.tx)
            NzSegmented(
                options = listOf("30", "60"),
                selectedIndex = if (settings.vizFps <= 30u) 0 else 1,
                onSelect = { model.setVizFps(if (it == 0) 30 else 60) },
            )
            NzSwitch(checked = settings.vizBeat, onCheckedChange = model::setVizBeat, label = "Détection du tempo")
            if (settings.vizMode == "scope") {
                Text("Orientation", style = Sillon.type.label, color = Sillon.colors.tx)
                NzSegmented(
                    options = listOf("Horizontal", "Vertical"),
                    selectedIndex = if (settings.vizScopeOrientation == "vertical") 1 else 0,
                    onSelect = { model.setScopeOrientation(if (it == 1) "vertical" else "horizontal") },
                )
                Text("Couleur", style = Sillon.type.label, color = Sillon.colors.tx)
                ChoicePills(
                    choices = listOf("Duo", "Monochrome"),
                    selected = if (settings.vizScopeColour == "mono") 1 else 0,
                    onSelect = { model.setScopeColour(if (it == 1) "mono" else "duo") },
                )
            }
        }
    }
    // La porte du studio de genres (`VisualizerSettings.svelte:558-566`'s
    // `{#if $isAdmin && CAN_ADMIN}`): l'échappatoire d'un vocabulaire figé, et
    // RIEN pour un membre. La carte n'appelle rien : `GenresEntry` fait ses
    // propres lectures (MB18).
    if (isAdmin && onGenres != null) {
        SettingsCard(
            title = "Studio de genres",
        ) {
            NzButton(
                text = "Ouvrir",
                onClick = onGenres,
                glyph = NzGlyph.Sliders,
                kind = NzButtonKind.Secondary,
            )
        }
    }
}

/**
 * « Téléchargements »: **MB15's own section, hosted**.
 *
 * `OfflineSettingsSection` owns every pixel and every call inside it (the quality
 * a new download takes, what the device holds, the offline-only switch, the play
 * cache's gauge and caps, and the per-title list on its own page) — MB15's note
 * says so, and this lot composes it exactly as it composes `AudioFxSections()`.
 * Nothing is copied here: a second version of the same cards would drift.
 */
@Composable
internal fun DownloadsSection(@Suppress("UNUSED_PARAMETER") model: SettingsModel, @Suppress("UNUSED_PARAMETER") state: SettingsModel.State) {
    // LN14's « Sur vos appareils » is the offline page's second tab: the tab
    // strip and its content live in MB15's module, the content is LN14's own
    // section (one writer per screen, and no copy of the peers' list here).
    fr.neezer.mobile.feature.offline.OfflineSettingsSection(
        peerSets = { fr.neezer.mobile.feature.lan.PeerSetsSection() },
    )
}

/** « Découverte » : the level, and the titles the person does not like. */
@Composable
internal fun DiscoverySection(model: SettingsModel, state: SettingsModel.State) {
    val colors = Sillon.colors
    val step = discoveryStepOf(state.discoveryLevel)
    SettingsCard(
        title = "Niveau de découverte",
    ) {
        Text(step.name, style = Sillon.type.titleSmall, color = colors.tx)
        Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            DISCOVERY_STEPS.forEach { choice ->
                NzChip(
                    text = choice.name,
                    onClick = { model.setDiscoveryLevel(if (choice.from == 0.0) 0.0 else choice.from + 0.125) },
                    selected = step.name == choice.name,
                )
            }
        }
    }
    DislikesCard(model, state)
}

/**
 * `DislikedTracks.svelte`: the rows, each with « Retirer » (optimistic).
 *
 * The list goes through a `ListHandle` + `rememberListWindow` (rule 4): a person
 * with thousands of them scrolls 64-row pages, and the card's own 380 dp window
 * is what keeps it a card on a page that already scrolls.
 */
@Composable
private fun DislikesCard(model: SettingsModel, state: SettingsModel.State) {
    val colors = Sillon.colors
    val section = state.dislikes
    SettingsCard(
        title = "Titres que vous n'aimez pas",
    ) {
        // The card's phase replaces in place (M7): the cause, the loading line,
        // the empty note or the window.
        NzSwap(targetState = settingsPhase(section), label = "titres-aines") { phase ->
            when (phase) {
                SETTINGS_PHASE_FAILED -> LoadFailed(
                    cause = section.failure.orEmpty(),
                    onRetry = { model.retry("discovery") },
                )
                SETTINGS_PHASE_LOADING -> Text("Chargement…", style = Sillon.type.bodySmall, color = colors.dim)
                else -> {
                    if (section.stale) StaleNote()
                    val handle = section.data ?: return@NzSwap
                    val window = rememberListWindow(handle)
                    val listState = rememberLazyListState()
                    LaunchedEffect(listState, window) {
                        snapshotFlow { listState.firstVisibleItemIndex to listState.isScrollInProgress }
                            .collect { (first, scrolling) ->
                                window.onVisible(first, first + VISIBLE_ROWS, scrolling)
                            }
                    }
                    if (window.len == 0) {
                        EmptyState(
                            title = "Aucun pour l'instant",
                            glyph = NzGlyph.ThumbsDown,
                        )
                    } else {
                        LazyColumn(
                            state = listState,
                            modifier = Modifier.fillMaxWidth().height(DISLIKES_HEIGHT),
                        ) {
                            items(window.len, key = { index -> window.rowAt(index)?.key ?: "ligne-$index" }) { index ->
                                val row = window.rowAt(index)
                                if (row == null) {
                                    SkeletonRow()
                                } else if (row.key in model.hiddenDislikes) {
                                    // Removed optimistically: the row is gone in the
                                    // same frame, and comes back if the server refuses.
                                } else {
                                    TrackRowView(
                                        row = row,
                                        playing = false,
                                        onClick = {},
                                        onMore = { model.undislike(row) },
                                        onSwipeQueue = {},
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/**
 * A settings card's phase, as the [NzSwap] target: its cause, its loading line,
 * its body. One rule for every panel this lot touches.
 */
internal const val SETTINGS_PHASE_FAILED = "echec"
internal const val SETTINGS_PHASE_LOADING = "chargement"
internal const val SETTINGS_PHASE_CONTENT = "contenu"

internal fun <T> settingsPhase(section: SettingsModel.Section<T>): String = when {
    section.failure != null && section.data == null -> SETTINGS_PHASE_FAILED
    section.data == null -> SETTINGS_PHASE_LOADING
    else -> SETTINGS_PHASE_CONTENT
}

/** An unloaded row: the design system's skeleton, never a blank line. */
@Composable
private fun SkeletonRow() {
    Box(
        Modifier
            .fillMaxWidth()
            .height(56.dp)
            .padding(vertical = NzSpacing.xs),
    ) {
        fr.neezer.mobile.designsystem.Skeleton(modifier = Modifier.fillMaxWidth().height(40.dp))
    }
}

/** « Assistance » : the two versions, the report, the share. */
@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
internal fun HelpSection(model: SettingsModel, state: SettingsModel.State) {
    val colors = Sillon.colors
    val versions = state.versions
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    SettingsCard(title = "Versions") {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            Text("Application : ${versions?.app ?: "inconnue"}", style = Sillon.type.body, color = colors.tx)
            Text(
                text = "Noyau (Rust) : ${fr.neezer.mobile.bridge.NzBridge.version ?: "indisponible"}",
                style = Sillon.type.body,
                color = colors.tx,
            )
            Text(
                text = "Serveur : ${versions?.server?.takeIf { it.isNotBlank() } ?: "inconnue"}",
                style = Sillon.type.body,
                color = colors.tx,
            )
        }
    }
    // The native app updates from ITS OWN server channel (`/api/android/native-apk`):
    // a server that predates it answers "mise à jour indisponible", never the
    // WebView APK (`docs/agents/mobile.md`, "Deux apps Android").
    fr.neezer.mobile.feature.setup.UpdateCard()
    // Le dernier plantage (M14). Rien à afficher = rien à dessiner : une carte
    // vide serait un mensonge sur l'état de l'app.
    var crash by remember { mutableStateOf(fr.neezer.mobile.bridge.CrashRecord.read(context)) }
    crash?.let { trace ->
        SettingsCard(
            title = "Dernier incident",
        ) {
            Text(
                text = trace,
                style = Sillon.type.labelMono,
                color = colors.dim,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(160.dp)
                    .background(colors.s2, RoundedCornerShape(NzRadius.field))
                    .padding(NzSpacing.s)
                    .verticalScroll(rememberScrollState()),
            )
            androidx.compose.foundation.layout.FlowRow(
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            ) {
                NzButton(
                    text = "Partager l'incident",
                    onClick = {
                        val send = Intent(Intent.ACTION_SEND).apply {
                            type = "text/plain"
                            putExtra(Intent.EXTRA_TEXT, trace)
                            putExtra(Intent.EXTRA_SUBJECT, "Neezer — incident")
                        }
                        context.startActivity(Intent.createChooser(send, "Partager l'incident"))
                    },
                    glyph = NzGlyph.Share,
                    kind = NzButtonKind.Secondary,
                )
                NzButton(
                    text = "Effacer",
                    onClick = {
                        fr.neezer.mobile.bridge.CrashRecord.clear(context)
                        crash = null
                    },
                    kind = NzButtonKind.Ghost,
                )
            }
        }
    }
    SettingsCard(
        title = "Diagnostic",
    ) {
        NzButton(
            text = "Partager le rapport",
            onClick = {
                scope.launch {
                    val report = model.buildReport() ?: return@launch
                    val send = Intent(Intent.ACTION_SEND).apply {
                        type = "text/plain"
                        putExtra(Intent.EXTRA_TEXT, report)
                        putExtra(Intent.EXTRA_SUBJECT, "Neezer — diagnostic")
                    }
                    context.startActivity(Intent.createChooser(send, "Partager le diagnostic"))
                }
            },
            glyph = NzGlyph.Share,
            kind = NzButtonKind.Secondary,
        )
        val report = state.report
        if (report != null) {
            Text(
                text = report,
                style = Sillon.type.labelMono,
                color = colors.dim,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(180.dp)
                    .background(colors.s2, RoundedCornerShape(NzRadius.field))
                    .padding(NzSpacing.s)
                    .verticalScroll(rememberScrollState()),
            )
        }
    }
}

// -- small shared controls -----------------------------------------------------

/**
 * A row of chips, one selected — the SPA's `.quality`/`.limits` rows.
 *
 * It WRAPS: at a 200 % font scale six chips are wider than a phone, and a plain
 * `Row` cut the last four off the screen (this lot's first capture of
 * *Téléchargements*).
 */
@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
internal fun ChoicePills(choices: List<String>, selected: Int, onSelect: (Int) -> Unit) {
    androidx.compose.foundation.layout.FlowRow(
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        choices.forEachIndexed { index, label ->
            NzChip(text = label, onClick = { onSelect(index) }, selected = index == selected)
        }
    }
}

/**
 * A horizontal level control (0…1), drawn with Sillon's own tokens: a 6 dp
 * track, a 22 dp thumb and a 48 dp finger box — the SPA's `input[type=range]`
 * measured, not Material's default slider (rule 5: no default-looking widget).
 */
@Composable
internal fun NzLevelSlider(
    value: Float,
    onValueChange: (Float) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val fraction = value.coerceIn(0f, 1f)
    BoxWithConstraints(
        modifier = modifier
            .fillMaxWidth()
            .height(NzMinTouchTarget)
            .semantics {
                contentDescription = label
                progressBarRangeInfo = androidx.compose.ui.semantics.ProgressBarRangeInfo(fraction, 0f..1f)
            }
            .pointerInput(Unit) {
                detectHorizontalDragGestures { change, _ ->
                    val width = size.width.toFloat().coerceAtLeast(1f)
                    onValueChange((change.position.x / width).coerceIn(0f, 1f))
                }
            },
    ) {
        val travel = (maxWidth - THUMB).coerceAtLeast(0.dp)
        Box(
            Modifier
                .align(Alignment.CenterStart)
                .fillMaxWidth()
                .height(6.dp)
                .background(colors.s2, RoundedCornerShape(NzRadius.pill)),
        )
        Box(
            Modifier
                .align(Alignment.CenterStart)
                .width(travel * fraction + THUMB)
                .height(6.dp)
                .background(colors.btn, RoundedCornerShape(NzRadius.pill)),
        )
        Box(
            Modifier
                .align(Alignment.CenterStart)
                .padding(start = travel * fraction)
                .size(THUMB)
                .background(colors.onbtn, CircleShape)
                .border(1.dp, colors.ln, CircleShape),
        )
    }
}

/** The level slider's thumb, and the dot its track ends in. */
private val THUMB = 22.dp

/** The rows a dislike card draws at once; the window fetches 64-row pages. */
internal const val VISIBLE_ROWS = 8

/** The card's own scroll window: a list inside a page that already scrolls. */
internal val DISLIKES_HEIGHT = 380.dp
