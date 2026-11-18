package fr.neezer.mobile.feature.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.feature.admin.AdminSections
import uniffi.nz_ffi.AppLink

/**
 * Réglages, the person's own sections (lot MB16) and, behind the administrator's
 * own pages, the server's (MB17).
 *
 * The SPA's two levels, ported (`routes/Settings.svelte`): the LIST of categories
 * is the phone's screen, a category is another one, and back returns to the list.
 * At 900 dp the list becomes a 200 dp rail beside the open category, exactly as
 * the SPA's `wide` grid does. The filter narrows the MENU — no route, no request,
 * no state beyond its field — and matching is accent- and case-blind, which is
 * what a thumb types.
 *
 * **The extension point a later lot uses** (`docs/agents/mobile.md`, MB16's
 * section): [SETTING_CATEGORIES] and [ADMIN_CATEGORIES] are data, and
 * [SettingsSections]/[AdminSections] dispatch on ids. LN14 (« Appareils et
 * amis ») adds ONE entry to that table plus one branch here; MB17 replaces the
 * `AdminSections()` stub with its own module. No other line of this screen moves.
 */
@Composable
fun SettingsEntry(
    section: String?,
    nav: NzNavigator,
    modifier: Modifier = Modifier,
    isAdmin: Boolean = false,
    // The seam a JVM test uses (`SettingsScreenshotTest`): the app passes
    // nothing and gets the core's own service; a fake renders the same screen.
    service: SettingsService? = AppSettings.service(),
) {
    val scope = rememberCoroutineScope()
    val model = remember(service) { service?.let { SettingsModel(it, scope) } }

    // The section comes from the ROUTE (a tap navigates, back pops): one source
    // of truth, so a deep link and a tap land on the same page.
    LaunchedEffect(section) {
        model?.start()
        model?.open(section)
    }

    val colors = Sillon.colors
    BoxWithConstraints(
        modifier = modifier
            .fillMaxSize()
            .background(colors.bg),
    ) {
        val wide = maxWidth >= 900.dp
        val state = model?.state
        if (model == null || state == null) {
            // The native core could not be built: Aide still says which build
            // this is, and why it cannot read anything (rule 3).
            CoreUnavailable(modifier = Modifier.fillMaxSize())
            return@BoxWithConstraints
        }

        val go: (String) -> Unit = { id -> nav.go(AppLink.Settings(section = id)) }
        if (wide) {
            Row(Modifier.fillMaxSize()) {
                Box(Modifier.width(200.dp).fillMaxHeight()) {
                    SettingsMenu(
                        model = model,
                        state = state,
                        selected = section,
                        onOpen = go,
                        showHints = false,
                        modifier = Modifier.verticalScroll(rememberScrollState()).padding(NzSpacing.l),
                    )
                }
                Box(Modifier.weight(1f).fillMaxHeight()) {
                    SettingsPane(
                        model = model,
                        state = state,
                        section = section,
                        onBack = null,
                        modifier = Modifier.verticalScroll(rememberScrollState()),
                    )
                }
            }
        } else if (section == null) {
            SettingsMenu(
                model = model,
                state = state,
                selected = null,
                onOpen = go,
                showHints = true,
                modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()),
            )
        } else {
            SettingsPane(
                model = model,
                state = state,
                section = section,
                onBack = { nav.back() },
                modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()),
            )
        }
    }
}

/** The list: the title, the filter, then the groups of categories. */
@Composable
private fun SettingsMenu(
    model: SettingsModel,
    state: SettingsModel.State,
    selected: String?,
    onOpen: (String) -> Unit,
    showHints: Boolean,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        // No second « Réglages »: the shell's top bar already names the route,
        // and the emulator capture showed the two headings stacked.
        Spacer(Modifier.height(NzSpacing.m))
        SettingsFilter(
            value = state.query,
            onValue = model::setQuery,
            modifier = Modifier.padding(horizontal = NzSpacing.pageX),
        )
        val groups = state.categories
        if (groups.isEmpty()) {
            Text(
                text = "Aucun réglage ne porte ce nom.",
                style = Sillon.type.bodySmall,
                color = colors.dim,
                modifier = Modifier.padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.l),
            )
        }
        groups.forEach { (group, items) ->
            Text(
                text = group.label.uppercase(),
                style = Sillon.type.labelMono,
                color = colors.dim,
                modifier = Modifier.padding(start = NzSpacing.pageX, top = NzSpacing.l, bottom = NzSpacing.xs),
            )
            items.forEach { category ->
                CategoryRow(
                    category = category,
                    selected = selected == category.id,
                    hint = showHints,
                    onClick = { onOpen(category.id) },
                )
            }
        }
        Spacer(Modifier.height(NzSpacing.pageY))
    }
}

/** `Settings.svelte`'s `.cat` row: an icon chip, the name, and what it holds. */
@Composable
private fun CategoryRow(
    category: SettingCategory,
    selected: Boolean,
    hint: Boolean,
    onClick: () -> Unit,
) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.field)
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = NzSpacing.m)
            .height(NzMinTouchTarget + 12.dp)
            // A selected row is the accent SURFACE with its own ink, never a
            // lemon label on cream (MB1's rule: a label never inherits a colour).
            .then(
                if (selected) Modifier.background(colors.ac, shape) else Modifier,
            )
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = NzSpacing.m),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Box(
            modifier = Modifier
                .size(38.dp)
                .background(
                    if (selected) colors.onac.copy(alpha = 0f) else nzChipTint(),
                    RoundedCornerShape(NzRadius.field),
                ),
            contentAlignment = Alignment.Center,
        ) {
            Icon(glyph = category.glyph, size = 19.dp, tint = if (selected) colors.onac else colors.btn)
        }
        Column(Modifier.weight(1f)) {
            Text(
                text = category.label,
                style = Sillon.type.label,
                color = if (selected) colors.onac else colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (hint && !selected) {
                Text(
                    text = category.hint,
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        if (hint) {
            Icon(
                glyph = NzGlyph.ChevronRight,
                size = 18.dp,
                tint = if (selected) colors.onac else colors.dim,
            )
        }
    }
}

/** The icon chip's 16 % accent tint (`Settings.svelte`'s `.cat-ico`). */
@Composable
private fun nzChipTint() = fr.neezer.mobile.designsystem.nzMix(
    Sillon.colors.btn.copy(alpha = 0.16f),
    Sillon.colors.bg,
    1f,
)

/** "Chercher un réglage…" — a pill field, Sillon's own colours, 48 dp. */
@Composable
private fun SettingsFilter(
    value: String,
    onValue: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.chip)
    BasicTextField(
        value = value,
        onValueChange = onValue,
        singleLine = true,
        textStyle = Sillon.type.body.copy(color = colors.tx),
        cursorBrush = SolidColor(colors.acText),
        modifier = modifier
            .fillMaxWidth()
            .height(NzMinTouchTarget)
            .semantics { contentDescription = "Chercher un réglage" },
        decorationBox = { inner ->
            Row(
                modifier = Modifier
                    .fillMaxSize()
                    .background(colors.s1, shape)
                    .border(1.dp, colors.ln, shape)
                    .padding(horizontal = NzSpacing.m),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Icon(glyph = NzGlyph.Search, size = 16.dp, tint = colors.dim)
                Box(Modifier.weight(1f)) {
                    if (value.isEmpty()) {
                        Text(
                            text = "Chercher un réglage…",
                            style = Sillon.type.body,
                            color = colors.dim,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    inner()
                }
                if (value.isNotEmpty()) {
                    NzIconButton(
                        glyph = NzGlyph.Close,
                        contentDescription = "Effacer la recherche",
                        onClick = { onValue("") },
                        size = 14.dp,
                    )
                }
            }
        },
    )
}

/** One category's page: its back button, its title, its cards. */
@Composable
private fun SettingsPane(
    model: SettingsModel,
    state: SettingsModel.State,
    section: String?,
    onBack: (() -> Unit)?,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Column(
        modifier = modifier.fillMaxWidth().padding(bottom = NzSpacing.pageY),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(
                    start = if (onBack != null) NzSpacing.s else NzSpacing.pageX,
                    end = NzSpacing.pageX,
                    top = NzSpacing.l,
                    bottom = NzSpacing.xs,
                ),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            if (onBack != null) {
                NzIconButton(
                    glyph = NzGlyph.ChevronLeft,
                    contentDescription = "Retour aux réglages",
                    onClick = onBack,
                )
            }
            Text(
                text = categoryOf(section)?.label ?: "Réglages",
                style = Sillon.type.titleSmall,
                color = colors.tx,
            )
        }
        Box(Modifier.padding(horizontal = NzSpacing.pageX)) {
            Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
                when (section) {
                    "playback" -> PlaybackSection(model, state)
                    "appearance" -> AppearanceSection(model, state)
                    "viz" -> VizSection(model, state)
                    "downloads" -> DownloadsSection(model, state)
                    "discovery" -> DiscoverySection(model, state)
                    // LN14's own screen, hosted (MB16's extension point).
                    "lan" -> fr.neezer.mobile.feature.lan.LanSettingsSection()
                    "sources" -> SourcesSection(model, state)
                    "account" -> AccountSection(model, state)
                    "help" -> HelpSection(model, state)
                    // MB17's two pages: its own module, its own composable. This
                    // line is the whole seam (the extension point the note names),
                    // and it hands the page ITS OWN id: `AdminSections()` with no
                    // argument drew the same page for both categories (MB17's
                    // PR declares the one-word edit as outside its file list).
                    "users", "server" -> AdminSections(section)
                    else -> Text(
                        text = "Cette page arrive avec son lot.",
                        style = Sillon.type.body,
                        color = colors.dim,
                    )
                }
            }
        }
    }
    // Nothing here fixes a text height: at a 200 % font scale the labels grow and
    // the cards follow them, which is what the acceptance's scale check reads.
}

/**
 * One section, over a model a test built — the real dispatch, without the route.
 * The screenshots use it; the app never does (the route is the app's).
 */
@Composable
internal fun SettingsSection(
    section: String,
    model: SettingsModel,
    modifier: Modifier = Modifier,
) {
    Box(modifier.fillMaxWidth()) {
        Column(
            verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
            modifier = Modifier.padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.l),
        ) {
            when (section) {
                "playback" -> PlaybackSection(model, model.state)
                "appearance" -> AppearanceSection(model, model.state)
                "viz" -> VizSection(model, model.state)
                "downloads" -> DownloadsSection(model, model.state)
                "discovery" -> DiscoverySection(model, model.state)
                "lan" -> fr.neezer.mobile.feature.lan.LanSettingsSection()
                "sources" -> SourcesSection(model, model.state)
                "account" -> AccountSection(model, model.state)
                "help" -> HelpSection(model, model.state)
                else -> Text(
                    text = categoryOf(section)?.label ?: "Réglages",
                    style = Sillon.type.titleSmall,
                    color = Sillon.colors.tx,
                )
            }
        }
    }
}

/** A card with a title, a subtitle and its content (the SPA's `.card`). */
@Composable
internal fun SettingsCard(
    title: String,
    subtitle: String? = null,
    content: @Composable () -> Unit,
) {
    val colors = Sillon.colors
    NzCard(Modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Text(text = title, style = Sillon.type.label, color = colors.tx)
            if (subtitle != null) {
                Text(text = subtitle, style = Sillon.type.bodySmall, color = colors.dim)
            }
            content()
        }
    }
}

/** « Hors ligne · version mémorisée » — what a card says over its kept copy. */
@Composable
internal fun StaleNote() {
    Text(
        text = "Hors ligne · version mémorisée",
        style = Sillon.type.bodySmall,
        color = Sillon.colors.dim,
    )
}

/** The build could not load the native core: Aide still names what it knows. */
@Composable
private fun CoreUnavailable(modifier: Modifier = Modifier) {
    val colors = Sillon.colors
    Column(
        modifier = modifier.padding(NzSpacing.pageX),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Text(text = "Réglages", style = Sillon.type.title, color = colors.tx)
        Text(
            text = "Le noyau de l'application n'a pas pu être chargé : ces réglages sont indisponibles.",
            style = Sillon.type.body,
            color = colors.dim,
        )
        Text(
            text = fr.neezer.mobile.bridge.NzBridge.failure ?: "cause inconnue",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
    }
}

/**
 * The wrapper `MainActivity` reads: everything it draws is inside [NzTheme] with
 * what Réglages saved, so a theme, an accent or the eco switch applies to the
 * WHOLE app — the home included — without a restart.
 *
 * It is the one seam between this module and `:app` (see `NzAppearance`), and
 * `SettingsThemeSwitchTest` proves a switch repaints another feature's screen.
 */
@Composable
fun NzAppearanceTheme(content: @Composable () -> Unit) {
    val appearance by NzAppearance.current
    NzTheme(
        kind = appearance.theme,
        accent = appearance.accent,
        coverAccent = appearance.sleeve,
        motion = appearance.motion,
        content = content,
    )
}
