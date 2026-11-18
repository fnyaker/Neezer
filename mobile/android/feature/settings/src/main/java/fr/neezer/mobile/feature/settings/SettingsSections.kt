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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
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
import fr.neezer.mobile.designsystem.NzSwitch
import fr.neezer.mobile.designsystem.NzThemeKind
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
        subtitle = "Le flux du titre en cours ; le suivant suit au démarrage.",
    ) {
        ChoicePills(
            choices = QUALITY_CHOICES.map { it.second },
            selected = QUALITY_CHOICES.indexOfFirst { it.first == settings.quality }.coerceAtLeast(0),
            onSelect = { index -> model.setPlaybackQuality(QUALITY_CHOICES[index].first) },
        )
    }
    // MB12's own module, its own ten faders and its meter: hosted, never copied.
    AudioFxSections()
}

/** « Apparence » : the three themes, the person's accent, Pochette's sleeve. */
@Composable
internal fun AppearanceSection(model: SettingsModel, state: SettingsModel.State) {
    val settings = state.settings ?: return
    SettingsCard(
        title = "Ambiance",
        subtitle = "Le thème est retenu sur cet appareil et s'applique tout de suite : rien n'est rechargé, rien n'est renvoyé au serveur.",
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            THEME_CHOICES.forEach { (id, label, hint) ->
                ThemeCard(
                    id = id,
                    label = label,
                    hint = hint,
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
private fun ThemeCard(id: String, label: String, hint: String, selected: Boolean, onClick: () -> Unit) {
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
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(label, style = Sillon.type.label, color = colors.tx)
                if (selected) {
                    Spacer(Modifier.width(NzSpacing.s))
                    Icon(NzGlyph.Check, size = 13.dp, tint = colors.acText)
                    Text("Choisi", style = Sillon.type.labelMono, color = colors.acText)
                }
            }
            Text(hint, style = Sillon.type.bodySmall, color = colors.dim, maxLines = 2, overflow = TextOverflow.Ellipsis)
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
        subtitle = "Une couleur à vous pour les boutons, le cœur et les sélections. Elle s'applique à Encre et Papier ; Pochette garde la couleur de la pochette qui joue.",
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
        Text(
            text = "Pochette suit la pochette qui joue : l'accent reprend sa couleur dominante et change à chaque titre. Une pochette sans couleur franche laisse l'orange de repli.",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
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

/** « Animations » : the eco switch and the knobs of the visualiser. */
@Composable
internal fun VizSection(model: SettingsModel, state: SettingsModel.State) {
    val settings = state.settings ?: return
    SettingsCard(
        title = "Mode éco",
        subtitle = "Coupe toutes les animations et le fond animé. La lecture et les indicateurs d'état restent.",
    ) {
        NzSwitch(checked = settings.eco, onCheckedChange = model::setEco, label = "Mode éco")
        Text(
            text = if (settings.eco) "Aucune transition : tout paraît d'un coup." else "Les transitions suivent les réglages du système.",
            style = Sillon.type.bodySmall,
            color = Sillon.colors.dim,
        )
    }
    SettingsCard(title = "Visualiseur", subtitle = "Ce qui s'anime pendant la lecture.") {
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
            Text(
                text = "Le tempo détecté module les barres et la couleur.",
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
            )
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
        subtitle = "Jusqu'où le Flow et les suggestions « Pour vous » s'éloignent de ce que vous écoutez. Le réglage vous suit sur tous vos appareils.",
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
        Text(step.hint, style = Sillon.type.bodySmall, color = colors.dim)
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
        subtitle = "Ils ne reviennent plus dans le Flow, les radios et les mixes. Vous pouvez toujours les chercher et les écouter vous-même.",
    ) {
        when {
            section.failure != null && section.data == null -> LoadFailed(
                cause = section.failure,
                onRetry = { model.retry("discovery") },
            )
            section.data == null -> Text("Chargement…", style = Sillon.type.bodySmall, color = colors.dim)
            else -> {
                if (section.stale) StaleNote()
                val handle = section.data
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
                        body = "Touchez « Je n'aime pas » dans le lecteur ou le menu d'un titre.",
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
    SettingsCard(
        title = "Diagnostic",
        subtitle = "Un rapport à joindre à un signalement : les versions, le serveur, l'appareil, ce que l'app a demandé au réseau et les réglages en force. Ni mot de passe, ni jeton, ni cookie.",
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
