package fr.neezer.mobile.feature.remote

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import kotlinx.coroutines.delay
import uniffi.nz_ffi.RemoteCommand
import uniffi.nz_ffi.RemotePhase

/**
 * Le contrôle distant: THIS phone driving another player (lot MB19).
 *
 * The SPA rebinds its whole player store; the phone deliberately does not — the
 * shell's transport is one action path (`PlaybackConnection`, the Media3 session,
 * the notification) that a feature lot may not replace, and a loan must never
 * leave the phone's own queue reachable by a stray tap. The desktop made the same
 * choice for the same reason (`desktop.md` D15a: "a loan's transport lives in the
 * Contrôle distant screen, with the owner's queue"). So the loan is a SCREEN, and
 * this file is it:
 *
 * - [`RemoteBanner`] says whose player this is, whether it answers, and the way
 *   out — always the first thing on the screen, as the SPA's pill is always on
 *   its app;
 * - the transport is the owner's player's: play/pause, previous/next, a seek bar,
 *   shuffle and repeat (`commands.js#OPS`, cut to what MB19's screen offers);
 * - under it, the owner's queue, windowed through `ListHandle` + the design
 *   system's own window (4 000 rows never cross whole), and a tap on a row JUMPs
 *   the owner's player there.
 *
 * The position is read from `RemoteModel.refreshControl`'s beat, four times a
 * second while the screen is visible — the SPA's `tick`, and nothing while it is
 * off screen (the effect is tied to composition).
 */
@Composable
fun RemoteControl(
    model: RemoteModel,
    onLeave: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val state by model.state.collectAsState()

    // The owner's player, read while this screen is visible and nowhere else.
    LaunchedEffect(model) {
        while (true) {
            model.refreshControl()
            delay(RemoteModel.CONTROL_MS)
        }
    }
    RemoteControlContent(model = model, state = state, onLeave = onLeave, modifier = modifier)
}

/**
 * The same screen over a state handed in: what the screenshot test renders (a
 * `ListHandle` cannot be built on the JVM), and what the refresh loop above feeds
 * on a device.
 */
@Composable
internal fun RemoteControlContent(
    model: RemoteModel,
    state: RemoteModel.State,
    onLeave: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Box(
        modifier = modifier.fillMaxSize().background(colors.bg),
        contentAlignment = Alignment.TopCenter,
    ) {
    Column(
        modifier = Modifier
            .fillMaxHeight()
            .widthIn(max = 560.dp)
            .padding(horizontal = NzSpacing.m),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        val status = state.status
        RemoteBanner(
            status = status,
            grant = state.grant ?: status?.grant,
            onLeave = {
                model.leave()
                onLeave()
            },
        )
        OwnerTransport(model = model, state = state)
        OwnerQueue(model = model, state = state, modifier = Modifier.weight(1f))
    }
    }
}

/**
 * « Lecteur de Léa · En direct · Complet » and « Quitter » — `RemoteBanner.svelte`
 * as a card rather than a floating pill (the screen IS the loan here, so a pill
 * floating over it would hide the transport).
 *
 * `ended` is the full-screen notice: the grant is gone, and the card offers the
 * account the server handed back.
 */
@Composable
fun RemoteBanner(
    status: RemoteStatusFi?,
    grant: RemoteGrantFi?,
    onLeave: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val phase = status?.phase ?: RemotePhase.CONNECTING
    val (dot, label) = when {
        phase == RemotePhase.RECONNECTING -> colors.dim to "Reconnexion…"
        phase == RemotePhase.CONNECTING -> colors.dim to "Connexion…"
        status?.online != true -> colors.dim to "Lecteur hors ligne"
        else -> colors.ac to "En direct"
    }
    val owner = grant?.owner?.ifBlank { "…" } ?: "…"
    val level = grant?.levelLabel ?: ""
    NzCard(modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(NzSpacing.m)
                .semantics {
                    contentDescription =
                        "Lecteur de $owner, $label${if (level.isNotEmpty()) ", $level" else ""}"
                },
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Box(
                Modifier
                    .size(10.dp)
                    .background(dot, RoundedCornerShape(NzRadius.pill)),
            )
            Icon(NzGlyph.Remote, size = 16.dp, tint = colors.ac)
            Column(Modifier.weight(1f)) {
                Text(
                    text = "Lecteur de $owner",
                    style = Sillon.type.label,
                    color = colors.tx,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = if (level.isNotEmpty()) "$label · $level" else label,
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            NzButton(
                text = "Quitter",
                kind = NzButtonKind.Ghost,
                glyph = NzGlyph.LogOut,
                onClick = onLeave,
            )
        }
    }
}

/**
 * The grant is over (a cut, an expiry, the owner logged out): the full-screen
 * notice `RemoteBanner.svelte` becomes, with the account to go back to when the
 * server named one.
 */
@Composable
fun RemoteEnded(restored: String?, onBack: () -> Unit, modifier: Modifier = Modifier) {
    val colors = Sillon.colors
    Column(
        modifier = modifier.fillMaxSize().padding(NzSpacing.xl),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(
            Modifier
                .size(64.dp)
                .background(colors.s2, RoundedCornerShape(NzRadius.card)),
            contentAlignment = Alignment.Center,
        ) {
            Icon(NzGlyph.Remote, size = 26.dp, tint = colors.tx)
        }
        Text("Le contrôle est terminé", style = Sillon.type.title, color = colors.tx)
        Text(
            text = if (restored.isNullOrBlank()) {
                "Ce lien a été coupé, ou il a expiré. Rien n'a été gardé sur cet appareil."
            } else {
                "Ce lien a été coupé, ou il a expiré. Rien n'a été gardé sur cet appareil. " +
                    "Votre compte, $restored, est de retour."
            },
            style = Sillon.type.body,
            color = colors.dim,
        )
        NzButton(
            text = if (restored.isNullOrBlank()) "Ouvrir l'app" else "Revenir à mon compte",
            glyph = if (restored.isNullOrBlank()) NzGlyph.Home else NzGlyph.User,
            onClick = onBack,
        )
    }
}

@Composable
private fun OwnerTransport(model: RemoteModel, state: RemoteModel.State) {
    val colors = Sillon.colors
    val now = state.control
    NzCard(Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(NzSpacing.m),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            val track = now?.track
            Text(
                text = track?.title ?: "Rien ne joue chez ${state.grant?.owner ?: "ce lecteur"}",
                style = Sillon.type.label,
                color = colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = track?.artists.orEmpty(),
                style = Sillon.type.bodySmall,
                color = colors.dim,
                maxLines = 1,
            )
            ProgressLine(
                position = now?.positionS ?: 0.0,
                duration = now?.durationS ?: 0.0,
                onSeek = { model.send(RemoteCommand.Seek(it)) },
            )
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s, Alignment.CenterHorizontally),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                TransportButton(NzGlyph.Shuffle, "Mélanger la file du lecteur", now?.shuffle == true) {
                    model.send(RemoteCommand.Shuffle)
                }
                TransportButton(NzGlyph.Prev, "Titre précédent chez le lecteur", false) {
                    model.send(RemoteCommand.Prev)
                }
                TransportButton(
                    glyph = if (now?.playing == true) NzGlyph.Pause else NzGlyph.Play,
                    label = if (now?.playing == true) "Mettre en pause le lecteur" else "Lire chez le lecteur",
                    on = true,
                ) {
                    model.send(if (now?.playing == true) RemoteCommand.Pause else RemoteCommand.Play)
                }
                TransportButton(NzGlyph.Next, "Titre suivant chez le lecteur", false) {
                    model.send(RemoteCommand.Next)
                }
                TransportButton(NzGlyph.Repeat, "Répéter chez le lecteur", now?.repeat != "off") {
                    model.send(RemoteCommand.Repeat)
                }
            }
        }
    }
}

@Composable
private fun TransportButton(
    glyph: NzGlyph,
    label: String,
    on: Boolean,
    onClick: () -> Unit,
) {
    val colors = Sillon.colors
    Box(
        modifier = Modifier
            .size(NzMinTouchTarget)
            .background(
                if (on) colors.ac else colors.s2,
                RoundedCornerShape(NzRadius.pill),
            )
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(glyph, size = 20.dp, tint = if (on) colors.onac else colors.tx, contentDescription = label)
    }
}

/**
 * The seek bar: a tap or a drag picks a position and sends ONE seek — the core
 * coalesces a burst of them into one request (`controller.js`'s 150 ms).
 *
 * It works on the DRAG, not on release: a person dragging the loan's bar wants
 * the owner's player to follow, which is what the SPA's `seekTo` does.
 */
@Composable
private fun ProgressLine(position: Double, duration: Double, onSeek: (Double) -> Unit) {
    val colors = Sillon.colors
    val fraction = if (duration > 0) (position / duration).coerceIn(0.0, 1.0) else 0.0
    var width by androidx.compose.runtime.remember { androidx.compose.runtime.mutableStateOf(1f) }
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = NzMinTouchTarget)
                .onSizeChanged { width = it.width.toFloat().coerceAtLeast(1f) }
                .pointerInput(duration) {
                    detectTapGestures { offset ->
                        if (duration > 0) {
                            onSeek((offset.x / width).coerceIn(0f, 1f) * duration)
                        }
                    }
                }
                .semantics {
                    contentDescription =
                        "Position ${formatTime(position)} sur ${formatTime(duration)}, toucher pour avancer"
                },
            contentAlignment = Alignment.CenterStart,
        ) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .heightIn(min = 4.dp)
                    .background(colors.s2, RoundedCornerShape(NzRadius.pill)),
            )
            Box(
                Modifier
                    .fillMaxWidth(fraction.toFloat())
                    .heightIn(min = 4.dp)
                    .background(colors.acMark, RoundedCornerShape(NzRadius.pill)),
            )
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text(formatTime(position), style = Sillon.type.mono, color = colors.dim)
            Text(formatTime(duration), style = Sillon.type.mono, color = colors.dim)
        }
    }
}

/** `m:ss`, the player's own spelling (`PlayerBar`). */
fun formatTime(seconds: Double): String {
    if (seconds.isNaN() || seconds <= 0) return "0:00"
    val total = seconds.toLong()
    return "${total / 60}:${(total % 60).toString().padStart(2, '0')}"
}

/** How many rows a phone shows at once (the design system's own `VisibleRows`). */
private const val QUEUE_VISIBLE = 10

@Composable
private fun OwnerQueue(
    model: RemoteModel,
    state: RemoteModel.State,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val rows = state.rows
    if (rows == null) {
        Text(
            "Lecture de la file du lecteur…",
            style = Sillon.type.bodySmall,
            color = colors.dim,
            modifier = modifier.padding(top = NzSpacing.s),
        )
        return
    }
    val window = remember(rows.source) {
        fr.neezer.mobile.designsystem.ListWindow(rows.source)
    }
    val listState = androidx.compose.foundation.lazy.rememberLazyListState()
    LaunchedEffect(window, listState) {
        snapshotFlow { listState.firstVisibleItemIndex to listState.isScrollInProgress }
            .collect { (first, scrolling) ->
                window.onVisible(first, first + QUEUE_VISIBLE, scrolling)
            }
    }
    LazyColumn(
        state = listState,
        modifier = modifier,
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        items(count = window.len) { index ->
            val row = window.rowAt(index)
            OwnerQueueRow(
                index = index,
                title = row?.title ?: "…",
                artists = row?.artists.orEmpty(),
                duration = (row?.durationS ?: 0u).toInt(),
                playing = state.control != null && (state.control!!.index == index),
                onClick = { model.send(RemoteCommand.Jump(index.toUInt())) },
            )
        }
    }
}

@Composable
private fun OwnerQueueRow(
    index: Int,
    title: String,
    artists: String,
    duration: Int,
    playing: Boolean,
    onClick: () -> Unit,
) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = NzMinTouchTarget)
            .background(
                if (playing) colors.s2 else colors.bg,
                RoundedCornerShape(NzRadius.chip),
            )
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xs)
            .semantics { contentDescription = "Aller à $title" },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Text(
            text = "${index + 1}",
            style = Sillon.type.mono,
            color = if (playing) colors.acText else colors.dim,
        )
        Column(Modifier.weight(1f)) {
            Text(
                text = title,
                style = Sillon.type.body,
                color = if (playing) colors.acText else colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (artists.isNotBlank()) {
                Text(artists, style = Sillon.type.bodySmall, color = colors.dim, maxLines = 1)
            }
        }
        Text(formatTime(duration.toDouble()), style = Sillon.type.mono, color = colors.dim)
    }
}
