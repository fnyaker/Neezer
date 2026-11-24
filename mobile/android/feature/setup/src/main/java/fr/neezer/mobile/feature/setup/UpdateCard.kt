package fr.neezer.mobile.feature.setup

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.os.Handler
import android.os.Looper
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon

/**
 * « Mise à jour » in Réglages → Assistance (lot MB4's updater, wired for the
 * coexistence release).
 *
 * The app updates itself from ITS OWN channel — `/api/android/native-apk`, via
 * `Core::fetch_update` — and the server check happens before a byte moves. A
 * server that predates the native channel gets the sentence the release owes:
 * « Mise à jour indisponible », never the WebView app's `neezer.apk` (different
 * package, different key; the installer would refuse it after 10 MB).
 *
 * One tap = one attempt (rule 3): there is no polling, no retry by itself, and
 * every outcome is a sentence. The installer's dialog belongs to Android; the
 * card says what happened before and after it.
 */
@Composable
fun UpdateCard(modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val activity = context.findActivity()
    var event by remember { mutableStateOf<UpdateEvent?>(null) }
    val main = remember { Handler(Looper.getMainLooper()) }

    val updater = remember(activity) {
        activity?.let { host ->
            Updater(
                activity = host,
                fetch = { destination ->
                    NzBridge.core?.screens()?.fetchUpdate(destination)
                        ?: error("Le noyau de l'application n'est pas disponible")
                },
                onEvent = { next -> main.post { event = next } },
            )
        }
    }
    DisposableEffect(updater) {
        onDispose { updater?.shutdown() }
    }

    NzCard(modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            Text("Mise à jour", style = Sillon.type.titleSmall)
            Text(
                "L'application se met à jour depuis ce serveur, sur son propre canal. " +
                    "Android demande confirmation avant d'installer.",
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
            )
            NzButton(
                text = "Vérifier les mises à jour",
                onClick = { updater?.start() },
                kind = NzButtonKind.Secondary,
                glyph = NzGlyph.Refresh,
                enabled = updater != null && event !is UpdateEvent.Download,
            )
            when (val current = event) {
                is UpdateEvent.Download -> Text(
                    "Téléchargement… ${(current.progress * 100).toInt()} %",
                    style = Sillon.type.bodySmall,
                )
                is UpdateEvent.Install -> Text(
                    "L'installateur d'Android est ouvert.",
                    style = Sillon.type.bodySmall,
                )
                is UpdateEvent.Permission -> Text(
                    "Autorisez les applications inconnues pour Neezer (native), puis revenez ici.",
                    style = Sillon.type.bodySmall,
                )
                is UpdateEvent.UpToDate -> Text(
                    "Vous avez déjà la dernière version.",
                    style = Sillon.type.bodySmall,
                )
                is UpdateEvent.Error -> Text(
                    updateCauseText(current.reason),
                    style = Sillon.type.bodySmall,
                    color = Sillon.colors.hrt,
                )
                null -> Unit
            }
        }
    }
}

/** The hosting `Activity`, through any `ContextWrapper` Compose may hand us. */
private fun Context.findActivity(): Activity? {
    var cursor: Context = this
    while (cursor is ContextWrapper) {
        if (cursor is Activity) return cursor
        cursor = cursor.baseContext
    }
    return cursor as? Activity
}
