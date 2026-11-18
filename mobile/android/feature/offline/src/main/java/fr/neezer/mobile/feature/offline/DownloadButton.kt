package fr.neezer.mobile.feature.offline

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzGlyph
import uniffi.nz_ffi.SetProgress

/**
 * The download action every list and every entity page shows (lot MB15):
 * « Télécharger » for a whole playlist, album or show, and the queue's own state
 * afterwards.
 *
 * `lib/offline.js` + the SPA's `DownloadButton` PORTED, in five states — the
 * person always knows where their tap landed (rule 2):
 *
 * | state | the button says | a tap does |
 * |---|---|---|
 * | nothing queued | « Télécharger » | queues the set (optimistic) |
 * | queued or running | « 12 / 200 » (disabled) | — (it is already working) |
 * | every title here | « Téléchargé » (disabled) | — (the bin is in Réglages) |
 * | some titles failed | « Réessayer » | retries the failures |
 * | the core said no | « Télécharger » | tries again |
 *
 * A failure's CAUSE is never hidden in a disabled button: it is toasted when it
 * happens (`OfflineListener::on_failure`) and listed in Réglages →
 * Téléchargements, one French sentence per title.
 *
 * `context` is `kind:id` (`playlist:42`, `album:7`, `show:3`, `track:3135556`).
 */
@Composable
fun DownloadButton(
    context: String,
    modifier: Modifier = Modifier,
    label: String = "Télécharger",
    /** The row's own labels, for a single title (`track:`): the phone has no
     *  route that turns a track id back into a row. */
    title: String = "",
    artist: String = "",
    quality: String = "",
) {
    val model = remember { OfflineHost.model() }
    val state by model.state.collectAsState()
    LaunchedEffect(model) { model.start() }

    val set = state.set(context)
    val pending = state.isPending(context)
    val running = pending || (set != null && set.running)
    val failed = set?.failed?.toInt() ?: 0
    val done = set?.done?.toInt() ?: 0
    val total = set?.total?.toInt() ?: 0

    when {
        running -> NzButton(
            text = if (total > 0) "$done / $total" else "En file",
            onClick = {},
            modifier = modifier.semantics {
                contentDescription =
                    if (total > 0) "Téléchargement en cours, $done titre(s) sur $total" else "Téléchargement en file"
            },
            kind = NzButtonKind.Ghost,
            glyph = NzGlyph.Download,
            enabled = false,
        )

        set != null && failed > 0 -> NzButton(
            text = "Réessayer",
            onClick = { model.retry(set.context) },
            modifier = modifier.semantics {
                contentDescription = "Réessayer le téléchargement : ${causeOf(set)}"
            },
            kind = NzButtonKind.Secondary,
            glyph = NzGlyph.Refresh,
        )

        set != null && total > 0 && done >= total -> NzButton(
            text = "Téléchargé",
            onClick = {},
            modifier = modifier.semantics {
                contentDescription = "Téléchargé : ${set.title.ifEmpty { context }} est sur cet appareil"
            },
            kind = NzButtonKind.Ghost,
            glyph = NzGlyph.Downloaded,
            enabled = false,
        )

        else -> NzButton(
            text = label,
            onClick = { model.add(context, quality, title, artist) },
            modifier = modifier,
            kind = NzButtonKind.Secondary,
            glyph = NzGlyph.Download,
        )
    }
}

/** The first cause, for a TalkBack label: the button shows one line, the cause
 *  is what a person needs to hear. */
private fun causeOf(set: SetProgress): String =
    set.failures.firstOrNull()?.substringAfter(" — ") ?: "une erreur est survenue"
