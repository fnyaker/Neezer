package fr.neezer.mobile.feature.remote

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.Sillon

/**
 * The way into remote control, next to the listen party wherever the player is
 * (lot MB19).
 *
 * `components/RemoteButton.svelte`, ported: a glyph, and — while somebody drives
 * this player — the count, on the accent. Its label is the one a screen reader
 * reads: « Contrôle à distance », or « Contrôle à distance — 1 appareil
 * connecté » (the SPA's own wording).
 *
 * The sheet opens as a `Dialog`, not as an in-layout pane: the button lives in the
 * player bar of ANOTHER lot (`MB11`'s now playing), whose file this lot may not
 * edit, and a sheet that only painted inside a 48 dp button would be invisible.
 * The dialog is transparent and undecorated, so what is seen is `NzSheet`'s own
 * scrim and pane.
 *
 * @param model the screen's model, when it has one (a bench, a test); the button
 *   builds its own otherwise, which is what a player bar gets.
 */
@Composable
fun RemoteButton(
    modifier: Modifier = Modifier,
    size: Dp = 18.dp,
    model: RemoteModel? = null,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val owned = model ?: remember(scope) { RemoteModel(NzRemoteService, scope) }
    var open by remember { mutableStateOf(false) }
    val state by owned.state.collectAsState()
    val live = state.status?.controllers?.size ?: 0
    val label = if (live > 0) {
        "Contrôle à distance — $live ${if (live > 1) "appareils connectés" else "appareil connecté"}"
    } else {
        "Contrôle à distance"
    }
    val colors = Sillon.colors

    Row(
        modifier = modifier
            .size(NzMinTouchTarget)
            .background(
                if (live > 0) colors.ac else androidx.compose.ui.graphics.Color.Transparent,
                RoundedCornerShape(NzRadius.pill),
            )
            .clickable(role = Role.Button) {
                open = true
                owned.open()
            }
            .padding(4.dp)
            .semantics { contentDescription = label },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(5.dp),
    ) {
        Icon(
            NzGlyph.Remote,
            size = size,
            tint = if (live > 0) colors.onac else colors.dim,
            contentDescription = label,
        )
        if (live > 0) {
            Text(
                text = "$live",
                style = Sillon.type.labelSmall,
                color = colors.onac,
            )
        }
    }

    if (open) {
        Dialog(
            onDismissRequest = {
                open = false
                owned.close()
            },
            properties = DialogProperties(usePlatformDefaultWidth = false),
        ) {
            Box(Modifier.fillMaxSize()) {
                RemoteSheet(
                    visible = true,
                    model = owned,
                    onDismiss = {
                        open = false
                        owned.close()
                    },
                    onShare = { text -> context.shareText(text) },
                    onCopy = { text -> context.copyText(text) },
                    selfName = "Ce téléphone",
                )
            }
        }
    }
}

/** The system share sheet — the SPA's `navigator.share`, on Android. */
internal fun Context.shareText(text: String) {
    val send = Intent(Intent.ACTION_SEND).apply {
        type = "text/plain"
        putExtra(Intent.EXTRA_TEXT, text)
    }
    startActivity(Intent.createChooser(send, "Partager le lien").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
}

/** The clipboard, with the SPA's own toast-worthy outcome. */
internal fun Context.copyText(text: String) {
    val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager ?: return
    clipboard.setPrimaryClip(ClipData.newPlainText("Contrôle à distance", text))
    fr.neezer.mobile.designsystem.NzToasts.show("Lien copié")
}
