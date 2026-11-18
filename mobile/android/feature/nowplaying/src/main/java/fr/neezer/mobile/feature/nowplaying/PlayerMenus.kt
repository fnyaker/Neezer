package fr.neezer.mobile.feature.nowplaying

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.QualityBadge
import fr.neezer.mobile.designsystem.Sillon
import uniffi.nz_ffi.Quality
import uniffi.nz_ffi.SleepState
import uniffi.nz_ffi.SleepTimer

/**
 * The player's three small menus — `SleepButton.svelte`, `QualityMenu.svelte` and
 * `SpeedButton.svelte`, ported.
 *
 * Each is the same shape: a chip in the header, and a list that hangs from it over
 * a glass surface. What differs is where the VALUE lives, and that is the whole
 * point of this file:
 *
 * - **the quality is a setting** (the player's `AudioSettings.quality`) and takes
 *   effect at the next title, which the toast says;
 * - **the sleep timer is the player's deadline** (`PlayerApi.set_sleep`), and the
 *   badge reads the countdown the ENGINE keeps — a local tick would drift off the
 *   fade within a minute;
 * - **a podcast's rate is per SHOW**, remembered in the device's store
 *   (`lib/speed.js`) and applied through the player; it is absent for music,
 *   because nobody wants an album at 1,5×.
 */

/** `SLEEP_CHOICES`: the seven the SPA offers, in minutes. */
internal val SLEEP_CHOICES = listOf(5, 10, 15, 30, 45, 60, 90)

/**
 * A popup list over a Sillon surface: a card at the top-end of its chip, dismissed
 * by a tap anywhere else (the SPA's `svelte:window` click) or by Back.
 */
@Composable
internal fun NzPopupList(
    expanded: Boolean,
    onDismiss: () -> Unit,
    width: Int = 260,
    content: @Composable () -> Unit,
) {
    if (!expanded) return
    val colors = Sillon.colors
    Popup(
        alignment = Alignment.TopEnd,
        onDismissRequest = onDismiss,
        properties = PopupProperties(focusable = true),
    ) {
        Column(
            Modifier
                .widthIn(min = width.dp, max = width.dp)
                .clip(RoundedCornerShape(NzRadius.card))
                .background(colors.s2)
                .padding(vertical = NzSpacing.s)
                .verticalScroll(rememberScrollState()),
        ) {
            content()
        }
    }
}

/** One row of a popup list: a label, an optional hint, a check when chosen. */
@Composable
internal fun MenuRow(
    label: String,
    hint: String? = null,
    selected: Boolean = false,
    onClick: () -> Unit,
) {
    val colors = Sillon.colors
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = NzMinTouchTarget)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = NzSpacing.m, vertical = NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                text = label,
                style = Sillon.type.body,
                color = if (selected) colors.acText else colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (hint != null) {
                Text(text = hint, style = Sillon.type.mono, color = colors.dim, maxLines = 1)
            }
        }
        if (selected) Icon(glyph = NzGlyph.Check, size = 16.dp, tint = colors.acText, contentDescription = null)
    }
}

/** The moon: its menu sets a deadline, ends with the track, or cancels one. */
@Composable
internal fun SleepButton(sleep: SleepState?, model: NowPlayingModel) {
    var open by remember { mutableStateOf(false) }
    val colors = Sillon.colors
    val label = sleepLabel(sleep)
    Box {
        Box(
            Modifier
                .heightIn(min = NzMinTouchTarget)
                .widthIn(min = NzMinTouchTarget)
                .clip(RoundedCornerShape(NzRadius.field))
                .clickable(role = Role.Button) { open = !open }
                .padding(horizontal = NzSpacing.xs),
            contentAlignment = Alignment.Center,
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
            ) {
                Icon(
                    glyph = NzGlyph.Moon,
                    size = 20.dp,
                    tint = if (sleep != null) colors.acText else colors.tx,
                    contentDescription = if (sleep != null) {
                        "Minuteur de sommeil : $label"
                    } else {
                        "Minuteur de sommeil"
                    },
                )
                if (sleep != null) {
                    Text(label, style = Sillon.type.mono, color = colors.acText, maxLines = 1)
                }
            }
        }
        NzPopupList(expanded = open, onDismiss = { open = false }) {
            Text(
                text = "Minuteur de sommeil",
                style = Sillon.type.labelMono,
                color = colors.dim,
                modifier = Modifier.padding(horizontal = NzSpacing.m, vertical = NzSpacing.xs),
            )
            if (sleep != null) {
                MenuRow(label = "Désactiver", hint = label) {
                    model.setSleep(null)
                    open = false
                }
            }
            for (minutes in SLEEP_CHOICES) {
                val chosen = sleep != null && !sleep.atEndOfTrack && sleep.minutes == minutes.toUInt()
                MenuRow(
                    label = minutesName(minutes.toUInt()),
                    selected = chosen,
                ) {
                    model.setSleep(SleepTimer.Minutes(minutes.toUInt()))
                    open = false
                }
            }
            MenuRow(label = "À la fin du titre", selected = sleep?.atEndOfTrack == true) {
                model.setSleep(SleepTimer.EndOfTrack)
                open = false
            }
        }
    }
}

/** The badge and its five rungs — the ones the player can LOAD (desktop's rule). */
@Composable
internal fun QualityButton(quality: Quality, model: NowPlayingModel) {
    var open by remember { mutableStateOf(false) }
    val colors = Sillon.colors
    Box {
        Box(
            Modifier
                .heightIn(min = NzMinTouchTarget)
                .widthIn(min = NzMinTouchTarget)
                .clickable(role = Role.Button) { open = !open },
            contentAlignment = Alignment.Center,
        ) {
            QualityBadge(label = qualityShort(quality))
        }
        NzPopupList(expanded = open, onDismiss = { open = false }) {
            Text(
                text = "Qualité de streaming",
                style = Sillon.type.labelMono,
                color = colors.dim,
                modifier = Modifier.padding(horizontal = NzSpacing.m, vertical = NzSpacing.xs),
            )
            for (rung in PLAYER_QUALITIES) {
                MenuRow(
                    label = qualityLabel(rung),
                    hint = qualityHint(rung),
                    selected = rung == quality,
                ) {
                    model.setQuality(rung)
                    open = false
                }
            }
        }
    }
}

/**
 * The five rungs the player has: `Auto`, FLAC, 320, 128, 64.
 *
 * **Not the SPA's six.** `QualityMenu.svelte` lists OPUS_256 and OPUS_192 too,
 * and the player cannot load them (`nz_audio::Quality` has no 256/192; the desktop
 * made the same call, `desktop.md` *D14a*: "a menu that offered them would save a
 * value the player cannot load"). The server accepts the two, so the day the
 * engine gains them this list grows — in one place.
 */
internal val PLAYER_QUALITIES = listOf(
    Quality.AUTO,
    Quality.FLAC,
    Quality.OPUS320,
    Quality.OPUS128,
    Quality.OPUS64,
)

/** The episode's rate chip; absent for music (the menu only exists for a show). */
@Composable
internal fun SpeedChip(rate: Float, model: NowPlayingModel) {
    var open by remember { mutableStateOf(false) }
    val colors = Sillon.colors
    val choices = remember { NowPlayingCore.service()?.speedChoices().orEmpty() }
    Box {
        Box(
            Modifier
                .heightIn(min = NzMinTouchTarget)
                .widthIn(min = NzMinTouchTarget)
                .clip(RoundedCornerShape(NzRadius.chip))
                .background(if (rate != 1f) colors.s2 else colors.s1)
                .clickable(role = Role.Button) { open = !open }
                .padding(horizontal = NzSpacing.s),
            contentAlignment = Alignment.Center,
        ) {
            Text(speedLabel(rate), style = Sillon.type.label, color = colors.tx)
        }
        NzPopupList(expanded = open, onDismiss = { open = false }) {
            Text(
                text = "Vitesse de lecture",
                style = Sillon.type.labelMono,
                color = colors.dim,
                modifier = Modifier.padding(horizontal = NzSpacing.m, vertical = NzSpacing.xs),
            )
            for (choice in choices) {
                MenuRow(
                    label = if (choice == 1f) "Normale" else speedLabel(choice),
                    selected = choice == rate,
                ) {
                    model.chooseSpeed(choice)
                    open = false
                }
            }
        }
    }
}
